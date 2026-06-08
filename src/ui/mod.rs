use crate::app::AppState;
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
use std::io::{self, Stdout};
use std::time::Duration;
use tokio::sync::{mpsc, watch};

pub enum UiAction {
    SetThreshold(f32),
    Quit,
}

#[derive(Default)]
pub struct UiConfig {
    pub attached: bool,
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

    loop {
        if *shutdown_rx.borrow_and_update() || state.quitting {
            break;
        }

        let mut dirty = false;

        if state_rx.has_changed().unwrap_or(false) {
            state = state_rx.borrow_and_update().clone();
            let size = terminal.size()?;
            clamp_table_offset(
                &mut table_state,
                state.processes.len(),
                table_viewport_rows(Rect::new(0, 0, size.width, size.height)),
            );
            dirty = true;
        }

        let viewport = table_viewport_rows(terminal.size()?.into());
        while let Some(code) = poll_key_event()? {
            if handle_key(
                code,
                &state,
                &mut table_state,
                &action_tx,
                viewport,
            )
            .await?
            {
                return Ok(());
            }
            if is_scroll_key(code) {
                terminal.draw(|f| draw_ui(f, &state, &config, &mut table_state))?;
            } else {
                dirty = true;
            }
        }

        if dirty {
            terminal.draw(|f| draw_ui(f, &state, &config, &mut table_state))?;
            continue;
        }

        tokio::select! {
            changed = state_rx.changed() => {
                if changed.is_ok() {
                    state = state_rx.borrow_and_update().clone();
                    let size = terminal.size()?;
                    clamp_table_offset(
                        &mut table_state,
                        state.processes.len(),
                        table_viewport_rows(Rect::new(0, 0, size.width, size.height)),
                    );
                    terminal.draw(|f| draw_ui(f, &state, &config, &mut table_state))?;
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

async fn handle_key(
    code: KeyCode,
    state: &AppState,
    table_state: &mut TableState,
    action_tx: &mpsc::Sender<UiAction>,
    viewport: usize,
) -> io::Result<bool> {
    match code {
        KeyCode::Char('q') | KeyCode::Esc => {
            let _ = action_tx.send(UiAction::Quit).await;
            Ok(true)
        }
        KeyCode::Char('+') | KeyCode::Char('=') => {
            let new = (state.cpu_threshold + 5.0).min(100.0);
            let _ = action_tx.send(UiAction::SetThreshold(new)).await;
            Ok(false)
        }
        KeyCode::Char('-') | KeyCode::Char('_') => {
            let new = (state.cpu_threshold - 5.0).max(1.0);
            let _ = action_tx.send(UiAction::SetThreshold(new)).await;
            Ok(false)
        }
        KeyCode::Down | KeyCode::Char('j') | KeyCode::PageDown => {
            scroll_table_down(
                table_state,
                state.processes.len(),
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

/// Middle pane height minus block borders and header row.
fn table_viewport_rows(area: Rect) -> usize {
    // header(3) + footer(5) + middle borders/header row
    let middle = area.height.saturating_sub(3 + 5);
    middle.saturating_sub(3).max(1) as usize
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

/// Right-edge track aligned with scrollable data rows (below block border + column header).
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
) {
    let area = f.area();

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(10),
            Constraint::Length(5),
        ])
        .split(area);

    let viewport = chunks[1].height.saturating_sub(3).max(1) as usize;
    clamp_table_offset(table_state, state.processes.len(), viewport);

    let mode_hint = if config.attached {
        "attached to daemon"
    } else {
        "standalone"
    };

    let header = Paragraph::new(format!(
        "Threshold: {:.1}% ({mode_hint}) | [+/-] threshold | [Up/Down] scroll | [q/Esc] quit",
        state.cpu_threshold
    ))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .title(" Daemon Configuration "),
    );
    f.render_widget(header, chunks[0]);

    let total = state.processes.len();
    let offset = table_state.offset();
    let visible_end = (offset + viewport).min(total);
    let table_title = if total == 0 {
        " System Resource Processes Monitor ".to_string()
    } else if total <= viewport {
        format!(" System Resource Processes Monitor ({total}) ")
    } else {
        format!(
            " System Resource Processes Monitor ({}–{} of {total}) ",
            offset + 1,
            visible_end
        )
    };

    let rows: Vec<Row> = state
        .processes
        .iter()
        .map(|p| {
            let mut style = Style::default();
            if state.throttled_pids.contains(&p.pid) {
                style = style.fg(Color::Red).add_modifier(Modifier::BOLD);
            }
            Row::new(vec![
                Cell::from(p.pid.to_string()),
                Cell::from(p.name.as_str()),
                Cell::from(format!("{:.1}%", p.cpu_usage)),
            ])
            .style(style)
        })
        .collect();

    let header_row = Row::new(vec![
        Cell::from("PID").style(Style::default().add_modifier(Modifier::BOLD)),
        Cell::from("Process").style(Style::default().add_modifier(Modifier::BOLD)),
        Cell::from("CPU").style(Style::default().add_modifier(Modifier::BOLD)),
    ]);

    let table = Table::new(
        rows,
        [
            Constraint::Length(10),
            Constraint::Min(16),
            Constraint::Length(8),
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

    if total > viewport {
        let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .begin_symbol(Some("↑"))
            .end_symbol(Some("↓"))
            .track_symbol(Some("│"))
            .thumb_symbol("█")
            .thumb_style(Style::default().fg(Color::Cyan))
            .track_style(Style::default().fg(Color::DarkGray));

        let mut scrollbar_state = ScrollbarState::new(total)
            .position(offset)
            .viewport_content_length(viewport);

        f.render_stateful_widget(
            scrollbar,
            table_scrollbar_area(chunks[1]),
            &mut scrollbar_state,
        );
    }

    let throttled: Vec<String> = state.throttled_pids.iter().map(|p| p.to_string()).collect();
    let recent_log: Vec<String> = state
        .throttle_log
        .iter()
        .rev()
        .take(3)
        .map(|e| format!("PID {}: {}", e.pid, e.message))
        .collect();

    let mut footer_text = format!("Throttled PIDs: [{}]", throttled.join(", "));
    if let Some(err) = &state.last_error {
        footer_text.push_str(&format!(" | Error: {err}"));
    }
    if !recent_log.is_empty() {
        footer_text.push_str(&format!(" | Log: {}", recent_log.join(" | ")));
    }

    let footer = Paragraph::new(footer_text).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" Mitigation Logs "),
    );
    f.render_widget(footer, chunks[2]);
}
