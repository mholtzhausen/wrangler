use crate::app::{AppGroupInfo, AppState, GroupBehaviorRecord, ProcessInfo};
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    widgets::{
        Block, Borders, Cell, Paragraph, Row, Scrollbar, ScrollbarOrientation, ScrollbarState,
        Table, TableState,
    },
    Terminal,
};
use std::collections::HashSet;
use std::io::{self, Stdout};
use std::time::Duration;
use tokio::sync::{mpsc, watch};

pub enum UiAction {
    SetAppCap(f32),
    Quit,
}

#[derive(Default)]
pub struct UiConfig {
    pub attached: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ViewMode {
    Flat,
    Grouped,
}

enum DisplayRow<'a> {
    GroupHeader {
        group: &'a AppGroupInfo,
        expanded: bool,
        throttled: bool,
    },
    Process {
        process: &'a ProcessInfo,
        indented: bool,
        throttled: bool,
    },
}

pub async fn run_ui(
    mut state_rx: watch::Receiver<AppState>,
    action_tx: mpsc::Sender<UiAction>,
    mut terminal: Terminal<CrosstermBackend<Stdout>>,
    mut shutdown_rx: watch::Receiver<bool>,
    config: UiConfig,
) -> io::Result<()> {
    let mut state = state_rx.borrow_and_update().clone();
    let mut table_state = TableState::default();
    let mut view_mode = ViewMode::Grouped;
    let mut expanded_groups = HashSet::new();

    loop {
        let mut dirty = false;

        if state_rx.has_changed().unwrap_or(false) {
            state = state_rx.borrow_and_update().clone();
            let size = terminal.size()?;
            clamp_table_offset(
                &mut table_state,
                display_row_count(&state, view_mode, &expanded_groups),
                table_viewport_rows(
                    Rect::new(0, 0, size.width, size.height),
                    bad_actors_panel_height(size.height, bad_actors_row_count(&state)),
                ),
            );
            dirty = true;
        }

        if *shutdown_rx.borrow_and_update() || state.quitting {
            break;
        }

        let terminal_size = terminal.size()?;
        let footer_height = bad_actors_panel_height(terminal_size.height, bad_actors_row_count(&state));
        let viewport = table_viewport_rows(
            Rect::new(0, 0, terminal_size.width, terminal_size.height),
            footer_height,
        );
        while let Some(code) = poll_key_event()? {
            if handle_key(
                code,
                &state,
                &mut table_state,
                &mut view_mode,
                &mut expanded_groups,
                &action_tx,
                viewport,
            )
            .await?
            {
                return Ok(());
            }
            if is_scroll_key(code) || is_view_key(code) {
                terminal.draw(|f| {
                    draw_ui(
                        f,
                        &state,
                        &config,
                        &mut table_state,
                        view_mode,
                        &expanded_groups,
                    )
                })?;
            } else {
                dirty = true;
            }
        }

        if dirty {
            terminal.draw(|f| {
                draw_ui(
                    f,
                    &state,
                    &config,
                    &mut table_state,
                    view_mode,
                    &expanded_groups,
                )
            })?;
            continue;
        }

        tokio::select! {
            changed = state_rx.changed() => {
                if changed.is_ok() {
                    state = state_rx.borrow_and_update().clone();
                    if state.quitting || *shutdown_rx.borrow_and_update() {
                        break;
                    }
                    let size = terminal.size()?;
                    clamp_table_offset(
                        &mut table_state,
                        display_row_count(&state, view_mode, &expanded_groups),
                        table_viewport_rows(
                            Rect::new(0, 0, size.width, size.height),
                            bad_actors_panel_height(size.height, bad_actors_row_count(&state)),
                        ),
                    );
                    terminal.draw(|f| draw_ui(
                        f,
                        &state,
                        &config,
                        &mut table_state,
                        view_mode,
                        &expanded_groups,
                    ))?;
                }
            }
            changed = shutdown_rx.changed() => {
                if changed.is_ok() && *shutdown_rx.borrow_and_update() {
                    break;
                }
            }
            _ = tokio::time::sleep(Duration::from_millis(16)) => {}
        }
    }

    Ok(())
}

fn poll_key_event() -> io::Result<Option<KeyCode>> {
    while event::poll(Duration::from_millis(0))? {
        match event::read()? {
            Event::Key(key)
                if matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) =>
            {
                return Ok(Some(key.code));
            }
            _ => {}
        }
    }
    Ok(None)
}

fn is_scroll_key(code: KeyCode) -> bool {
    matches!(
        code,
        KeyCode::Up
            | KeyCode::Down
            | KeyCode::Char('j')
            | KeyCode::Char('k')
            | KeyCode::PageUp
            | KeyCode::PageDown
    )
}

fn is_view_key(code: KeyCode) -> bool {
    matches!(code, KeyCode::Char('g') | KeyCode::Char('o') | KeyCode::Enter)
}

async fn handle_key(
    code: KeyCode,
    state: &AppState,
    table_state: &mut TableState,
    view_mode: &mut ViewMode,
    expanded_groups: &mut HashSet<u32>,
    action_tx: &mpsc::Sender<UiAction>,
    viewport: usize,
) -> io::Result<bool> {
    match code {
        KeyCode::Char('q') | KeyCode::Esc => {
            let _ = action_tx.send(UiAction::Quit).await;
            Ok(true)
        }
        KeyCode::Char('g') => {
            *view_mode = match *view_mode {
                ViewMode::Flat => ViewMode::Grouped,
                ViewMode::Grouped => ViewMode::Flat,
            };
            *table_state.offset_mut() = 0;
            Ok(false)
        }
        KeyCode::Char('o') | KeyCode::Enter if *view_mode == ViewMode::Grouped => {
            toggle_expand_at_offset(state, expanded_groups, table_state.offset());
            Ok(false)
        }
        KeyCode::Char('+') | KeyCode::Char('=') => {
            let new = (state.app_cap + 5.0).min(100.0);
            let _ = action_tx.send(UiAction::SetAppCap(new)).await;
            Ok(false)
        }
        KeyCode::Char('-') | KeyCode::Char('_') => {
            let new = (state.app_cap - 5.0).max(1.0);
            let _ = action_tx.send(UiAction::SetAppCap(new)).await;
            Ok(false)
        }
        KeyCode::Down | KeyCode::Char('j') | KeyCode::PageDown => {
            scroll_table_down(
                table_state,
                display_row_count(state, *view_mode, expanded_groups),
                viewport,
                if matches!(code, KeyCode::PageDown) {
                    viewport
                } else {
                    1
                },
            );
            Ok(false)
        }
        KeyCode::Up | KeyCode::Char('k') | KeyCode::PageUp => {
            scroll_table_up(
                table_state,
                if matches!(code, KeyCode::PageUp) {
                    viewport
                } else {
                    1
                },
            );
            Ok(false)
        }
        _ => Ok(false),
    }
}

fn toggle_expand_at_offset(
    state: &AppState,
    expanded_groups: &mut HashSet<u32>,
    offset: usize,
) {
    let rows = build_display_rows(state, ViewMode::Grouped, expanded_groups);
    if let Some(DisplayRow::GroupHeader { group, .. }) = rows.get(offset) {
        let key = group.group_key;
        if expanded_groups.contains(&key) {
            expanded_groups.remove(&key);
        } else {
            expanded_groups.insert(key);
        }
    }
}

fn display_row_count(state: &AppState, view_mode: ViewMode, expanded: &HashSet<u32>) -> usize {
    build_display_rows(state, view_mode, expanded).len()
}

fn build_display_rows<'a>(
    state: &'a AppState,
    view_mode: ViewMode,
    expanded: &HashSet<u32>,
) -> Vec<DisplayRow<'a>> {
    match view_mode {
        ViewMode::Flat => state
            .processes
            .iter()
            .map(|process| DisplayRow::Process {
                process,
                indented: false,
                throttled: state.throttled_pids.contains(&process.pid),
            })
            .collect(),
        ViewMode::Grouped => {
            let mut rows = Vec::new();
            for group in &state.groups {
                let throttled = state.is_group_throttled(group.group_key);
                let is_expanded = expanded.contains(&group.group_key);
                rows.push(DisplayRow::GroupHeader {
                    group,
                    expanded: is_expanded,
                    throttled,
                });
                if is_expanded {
                    for pid in &group.pids {
                        if let Some(process) = state.processes.iter().find(|p| p.pid == *pid) {
                            rows.push(DisplayRow::Process {
                                process,
                                indented: true,
                                throttled: state.throttled_pids.contains(&process.pid),
                            });
                        }
                    }
                }
            }
            rows
        }
    }
}

/// htop-style CPU (100% = one core) as a share of total machine capacity (0–100%).
fn machine_cpu_share(per_core_percent: f32, num_cores: usize) -> f32 {
    per_core_percent / num_cores.max(1) as f32
}

fn selection_style(base: Style, selected: bool) -> Style {
    if selected {
        base.bg(Color::Indexed(236))
    } else {
        base
    }
}

fn truncate_label(value: &str, max_chars: usize) -> String {
    let char_count = value.chars().count();
    if char_count <= max_chars {
        return value.to_string();
    }
    let keep = max_chars.saturating_sub(1);
    format!(
        "{}…",
        value.chars().take(keep).collect::<String>()
    )
}

fn row_to_cells<'a>(
    row: &'a DisplayRow<'a>,
    row_index: usize,
    selected_index: usize,
    num_cores: usize,
) -> Row<'a> {
    let selected = row_index == selected_index;
    match row {
        DisplayRow::GroupHeader {
            group,
            expanded,
            throttled,
        } => {
            let marker = if *throttled { "●" } else { " " };
            let arrow = if *expanded { "▼" } else { "▶" };
            let mut style = Style::default().add_modifier(Modifier::BOLD);
            if *throttled {
                style = style.fg(Color::Red);
            } else {
                style = style.fg(Color::Cyan);
            }
            let machine = machine_cpu_share(group.cpu_total, num_cores);
            Row::new(vec![
                Cell::from(format!("{marker}{arrow} {}", group.group_key)),
                Cell::from(format!(
                    "{} ({} pids)",
                    group.name,
                    group.pids.len()
                )),
                Cell::from(truncate_label(&group.user, 12)),
                Cell::from(truncate_label(&group.group, 12)),
                Cell::from(format!("{:.1}%", group.cpu_total)),
                Cell::from(format!("{machine:.1}%")),
            ])
            .style(selection_style(style, selected))
        }
        DisplayRow::Process {
            process,
            indented,
            throttled,
        } => {
            let mut style = Style::default();
            if *throttled {
                style = style.fg(Color::Red).add_modifier(Modifier::BOLD);
            }
            let prefix = if *indented { "  └ " } else { "" };
            let machine = machine_cpu_share(process.cpu_usage, num_cores);
            Row::new(vec![
                Cell::from(format!("{prefix}{}", process.pid)),
                Cell::from(process.name.as_str()),
                Cell::from(truncate_label(&process.user, 12)),
                Cell::from(truncate_label(&process.group, 12)),
                Cell::from(format!("{:.1}%", process.cpu_usage)),
                Cell::from(format!("{machine:.1}%")),
            ])
            .style(selection_style(style, selected))
        }
    }
}

/// Middle pane height minus header block and bad-actors panel.
fn table_viewport_rows(area: Rect, footer_height: u16) -> usize {
    let middle = area.height.saturating_sub(3 + footer_height);
    middle.saturating_sub(3).max(1) as usize
}

const BAD_ACTORS_MAX_LINES: usize = 10;

fn bad_actors_row_count(state: &AppState) -> usize {
    state.group_behavior.len().max(1)
}

fn bad_actors_content_lines(window_height: u16, entry_count: usize) -> usize {
    let max_by_window = (window_height as usize / 2).saturating_sub(3);
    let cap = BAD_ACTORS_MAX_LINES.min(max_by_window.max(1));
    entry_count.max(1).min(cap)
}

fn bad_actors_panel_height(window_height: u16, entry_count: usize) -> u16 {
    let content = bad_actors_content_lines(window_height, entry_count);
    (content + 3) as u16
}

fn format_throttle_duration(seconds: u64) -> String {
    if seconds >= 3600 {
        format!("{}h{}m", seconds / 3600, (seconds % 3600) / 60)
    } else if seconds >= 60 {
        format!("{}m{}s", seconds / 60, seconds % 60)
    } else {
        format!("{seconds}s")
    }
}

fn bad_actor_row(record: &GroupBehaviorRecord) -> Row<'static> {
    let status = if record.currently_throttled {
        "active"
    } else {
        "idle"
    };
    Row::new(vec![
        Cell::from(record.name.clone()),
        Cell::from(record.times_throttled.to_string()),
        Cell::from(format!("{:.1}%", record.peak_cpu)),
        Cell::from(format!("{:.1}%", record.last_cpu)),
        Cell::from(format_throttle_duration(record.throttle_seconds)),
        Cell::from(status),
    ])
}

fn clamp_table_offset(state: &mut TableState, row_count: usize, viewport: usize) {
    let max_offset = row_count.saturating_sub(viewport);
    *state.offset_mut() = state.offset().min(max_offset);
}

fn scroll_table_down(state: &mut TableState, row_count: usize, viewport: usize, amount: usize) {
    let max_offset = row_count.saturating_sub(viewport);
    *state.offset_mut() = state
        .offset()
        .saturating_add(amount)
        .min(max_offset);
}

fn scroll_table_up(state: &mut TableState, amount: usize) {
    *state.offset_mut() = state.offset().saturating_sub(amount);
}

fn table_scrollbar_area(table_area: Rect) -> Rect {
    Rect {
        x: table_area.x.saturating_add(table_area.width.saturating_sub(1)),
        y: table_area.y.saturating_add(2),
        width: 1,
        height: table_area.height.saturating_sub(3),
    }
}

fn draw_ui(
    f: &mut ratatui::Frame,
    state: &AppState,
    config: &UiConfig,
    table_state: &mut TableState,
    view_mode: ViewMode,
    expanded_groups: &HashSet<u32>,
) {
    let area = f.area();
    let footer_height = bad_actors_panel_height(area.height, bad_actors_row_count(state));
    let content_lines = bad_actors_content_lines(area.height, bad_actors_row_count(state));

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(10),
            Constraint::Length(footer_height),
        ])
        .split(area);

    let viewport = chunks[1].height.saturating_sub(3).max(1) as usize;
    let display_rows = build_display_rows(state, view_mode, expanded_groups);
    let row_count = display_rows.len();
    clamp_table_offset(table_state, row_count, viewport);

    let mode_hint = if config.attached {
        "attached to daemon"
    } else {
        "standalone"
    };
    let view_label = match view_mode {
        ViewMode::Flat => "flat",
        ViewMode::Grouped => "grouped",
    };

    let machine_budget = crate::policy::machine_cpu_budget(state.app_cap, state.num_cores);
    let header = Paragraph::new(format!(
        "System: {:.0}% | Cap: {:.0}% ({:.0}%/{} cores) | {} | {}:{} ({mode_hint}) | [g] view [o] expand [+/-] cap",
        state.global_cpu,
        state.app_cap,
        machine_budget,
        state.num_cores,
        state.throttle_backend,
        view_label,
        state.grouping,
    ))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .title(" Daemon Configuration "),
    );
    f.render_widget(header, chunks[0]);

    let offset = table_state.offset();
    let visible_end = (offset + viewport).min(row_count);
    let table_title = if row_count == 0 {
        format!(" Processes ({view_label}) ")
    } else if row_count <= viewport {
        format!(" Processes ({view_label}, {row_count}) ")
    } else {
        format!(
            " Processes ({view_label}, {}–{} of {row_count}) ",
            offset + 1,
            visible_end
        )
    };

    let selected_index = table_state.offset();
    let rows: Vec<Row> = display_rows
        .iter()
        .enumerate()
        .map(|(index, row)| row_to_cells(row, index, selected_index, state.num_cores))
        .collect();

    let header_row = Row::new(vec![
        Cell::from("PID/Key").style(Style::default().add_modifier(Modifier::BOLD)),
        Cell::from("Process / Group").style(Style::default().add_modifier(Modifier::BOLD)),
        Cell::from("User").style(Style::default().add_modifier(Modifier::BOLD)),
        Cell::from("Group").style(Style::default().add_modifier(Modifier::BOLD)),
        Cell::from("CPU %").style(Style::default().add_modifier(Modifier::BOLD)),
        Cell::from("Machine %").style(Style::default().add_modifier(Modifier::BOLD)),
    ]);

    let table = Table::new(
        rows,
        [
            Constraint::Length(12),
            Constraint::Min(12),
            Constraint::Length(10),
            Constraint::Length(10),
            Constraint::Length(7),
            Constraint::Length(9),
        ],
    )
    .header(header_row)
    .column_spacing(2)
    .block(
        Block::default()
            .borders(Borders::ALL)
            .title(table_title),
    );

    f.render_stateful_widget(table, chunks[1], table_state);

    if row_count > viewport {
        let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .begin_symbol(Some("↑"))
            .end_symbol(Some("↓"))
            .track_symbol(Some("│"))
            .thumb_symbol("█")
            .thumb_style(Style::default().fg(Color::Cyan))
            .track_style(Style::default().fg(Color::DarkGray));

        let mut scrollbar_state = ScrollbarState::new(row_count)
            .position(offset)
            .viewport_content_length(viewport);

        f.render_stateful_widget(
            scrollbar,
            table_scrollbar_area(chunks[1]),
            &mut scrollbar_state,
        );
    }

    let ranked = state.top_bad_actors(content_lines);
    let rows: Vec<Row> = if ranked.is_empty() {
        vec![Row::new(vec![Cell::from("No throttle events yet")])]
    } else {
        ranked.iter().map(|record| bad_actor_row(record)).collect()
    };

    let header = Row::new(vec![
        Cell::from("Application").style(Style::default().add_modifier(Modifier::BOLD)),
        Cell::from("Times").style(Style::default().add_modifier(Modifier::BOLD)),
        Cell::from("Peak %").style(Style::default().add_modifier(Modifier::BOLD)),
        Cell::from("Last %").style(Style::default().add_modifier(Modifier::BOLD)),
        Cell::from("Throttled").style(Style::default().add_modifier(Modifier::BOLD)),
        Cell::from("Status").style(Style::default().add_modifier(Modifier::BOLD)),
    ]);

    let mut title = format!(
        " Bad Actors (pressure >= {:.0}%) ",
        state.pressure_threshold
    );
    if let Some(err) = &state.last_error {
        title.push_str(&format!("| Error: {err} "));
    }

    let table = Table::new(
        rows,
        [
            Constraint::Min(14),
            Constraint::Length(6),
            Constraint::Length(7),
            Constraint::Length(7),
            Constraint::Length(10),
            Constraint::Length(6),
        ],
    )
    .header(header)
    .column_spacing(1)
    .block(
        Block::default()
            .borders(Borders::ALL)
            .title(title),
    );

    f.render_widget(table, chunks[2]);
}
