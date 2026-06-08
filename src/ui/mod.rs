use crate::app::AppState;
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, List, ListItem, Paragraph},
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

    loop {
        if *shutdown_rx.borrow_and_update() || state.quitting {
            break;
        }

        terminal.draw(|f| draw_ui(f, &state, &config))?;

        tokio::select! {
            changed = state_rx.changed() => {
                if changed.is_ok() {
                    state = state_rx.borrow_and_update().clone();
                }
            }
            _ = tokio::time::sleep(Duration::from_millis(50)) => {
                if event::poll(Duration::from_millis(0))? {
                    match event::read()? {
                        Event::Key(key) if key.kind == KeyEventKind::Press => {
                            match key.code {
                                KeyCode::Char('q') | KeyCode::Esc => {
                                    let _ = action_tx.send(UiAction::Quit).await;
                                    break;
                                }
                                KeyCode::Up => {
                                    let new = (state.cpu_threshold + 5.0).min(100.0);
                                    let _ = action_tx.send(UiAction::SetThreshold(new)).await;
                                }
                                KeyCode::Down => {
                                    let new = (state.cpu_threshold - 5.0).max(1.0);
                                    let _ = action_tx.send(UiAction::SetThreshold(new)).await;
                                }
                                _ => {}
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    Ok(())
}

fn draw_ui(f: &mut ratatui::Frame, state: &AppState, config: &UiConfig) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(10),
            Constraint::Length(5),
        ])
        .split(f.area());

    let mode_hint = if config.attached {
        "attached to daemon"
    } else {
        "standalone"
    };

    let header = Paragraph::new(format!(
        "Active Limit Threshold: {:.1}% ({mode_hint}) | [Up/Down] adjust | [q/Esc] quit",
        state.cpu_threshold
    ))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .title(" Daemon Configuration "),
    );
    f.render_widget(header, chunks[0]);

    let items: Vec<ListItem> = state
        .processes
        .iter()
        .take(20)
        .map(|p| {
            let mut style = Style::default();
            if state.throttled_pids.contains(&p.pid) {
                style = style.fg(Color::Red).add_modifier(Modifier::BOLD);
            }
            ListItem::new(format!(
                "PID: {:<6} | Process: {:<25} | CPU: {:.1}%",
                p.pid, p.name, p.cpu_usage
            ))
            .style(style)
        })
        .collect();

    let list = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" System Resource Processes Monitor "),
    );
    f.render_widget(list, chunks[1]);

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
