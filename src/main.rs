mod app;
mod cli;
mod event;
mod monitor;
mod throttle;
mod ui;

use clap::Parser;
use cli::Cli;
use crossterm::{
    event::DisableMouseCapture,
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use event::{spawn_event_hub, HubCommand};
use monitor::{protected_pids, run_monitor_with_threshold, MonitorConfig};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use std::io;
use throttle::ThrottleBackend;
use tokio::sync::{mpsc, watch};
use ui::{run_ui, UiAction};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();

    let cli = Cli::parse();
    let backend = ThrottleBackend::from_cli(cli.cgroups);

    let hub = spawn_event_hub(cli.threshold, backend);

    let (threshold_watch_tx, threshold_watch_rx) = watch::channel(cli.threshold);
    {
        let mut state_rx = hub.state_rx.clone();
        tokio::spawn(async move {
            loop {
                if state_rx.changed().await.is_err() {
                    break;
                }
                let t = state_rx.borrow().cpu_threshold;
                let _ = threshold_watch_tx.send(t);
            }
        });
    }

    let monitor_config = MonitorConfig {
        interval: cli.interval_duration(),
        self_pid: std::process::id(),
        protected_pids: protected_pids(),
    };

    let hub_tx_monitor = hub.command_tx.clone();
    tokio::spawn(async move {
        run_monitor_with_threshold(hub_tx_monitor, threshold_watch_rx, monitor_config).await;
    });

    let (ui_action_tx, mut ui_action_rx) = mpsc::channel::<UiAction>(16);
    let hub_for_ui = hub.command_tx.clone();
    tokio::spawn(async move {
        while let Some(action) = ui_action_rx.recv().await {
            match action {
                UiAction::SetThreshold(t) => {
                    let _ = hub_for_ui.send(HubCommand::SetThreshold(t)).await;
                }
                UiAction::Quit => {
                    let _ = hub_for_ui.send(HubCommand::Quit).await;
                }
            }
        }
    });

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, DisableMouseCapture)?;
    let terminal = Terminal::new(CrosstermBackend::new(stdout))?;

    let shutdown_rx = hub.subscribe_shutdown();
    let hub_cmd = hub.command_tx.clone();

    tokio::select! {
        res = run_ui(hub.state_rx.clone(), ui_action_tx, terminal, shutdown_rx) => {
            res?;
        }
        _ = tokio::signal::ctrl_c() => {
            let _ = hub_cmd.send(HubCommand::Quit).await;
        }
    }

    disable_raw_mode()?;
    execute!(io::stdout(), LeaveAlternateScreen, DisableMouseCapture)?;

    Ok(())
}
