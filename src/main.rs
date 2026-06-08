mod app;
mod cli;
mod daemon;
mod dashboard;
mod event;
mod ipc;
mod monitor;
mod runtime;
mod throttle;
mod ui;

#[cfg(target_os = "linux")]
mod tray;

use clap::Parser;
use cli::Cli;
use crossterm::{
    event::DisableMouseCapture,
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use event::HubCommand;
use ipc::AttachSession;
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use runtime::spawn_core;
use std::io;
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex, watch};
use ui::{run_ui, UiAction, UiConfig};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();

    let cli = Cli::parse();

    if cli.daemon {
        return daemon::run_from_cli(&cli).await;
    }

    if cli.attach {
        return run_attach_tui().await;
    }

    let core = spawn_core(&cli);
    run_standalone_tui(core).await
}

async fn run_standalone_tui(core: runtime::CoreRuntime) -> Result<(), Box<dyn std::error::Error>> {
    let hub = core.hub;

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

    let shutdown_rx = hub.subscribe_shutdown();
    let hub_cmd = hub.command_tx.clone();

    run_tui_loop(
        hub.state_rx.clone(),
        ui_action_tx,
        shutdown_rx,
        UiConfig::default(),
        async {
            let _ = hub_cmd.send(HubCommand::Quit).await;
        },
    )
    .await
}

async fn run_attach_tui() -> Result<(), Box<dyn std::error::Error>> {
    let session = AttachSession::connect()
        .await
        .map_err(|e| format!("failed to connect to wrangler daemon: {e}"))?;

    let state_rx = session.state_rx();
    let session = Arc::new(Mutex::new(Some(session)));

    let (ui_action_tx, mut ui_action_rx) = mpsc::channel::<UiAction>(16);
    let session_bg = session.clone();
    tokio::spawn(async move {
        while let Some(action) = ui_action_rx.recv().await {
            let mut guard = session_bg.lock().await;
            let Some(active) = guard.as_mut() else {
                break;
            };
            match action {
                UiAction::SetThreshold(t) => {
                    let _ = active.set_threshold(t).await;
                }
                UiAction::Quit => break,
            }
        }
    });

    let shutdown_rx = watch::channel(false).1;

    run_tui_loop(
        state_rx,
        ui_action_tx,
        shutdown_rx,
        UiConfig { attached: true },
        async {},
    )
    .await?;

    let mut guard = session.lock().await;
    if let Some(active) = guard.take() {
        active.detach().await?;
    }

    Ok(())
}

async fn run_tui_loop(
    state_rx: watch::Receiver<app::AppState>,
    ui_action_tx: mpsc::Sender<UiAction>,
    shutdown_rx: watch::Receiver<bool>,
    config: UiConfig,
    on_ctrl_c: impl std::future::Future<Output = ()> + Send,
) -> Result<(), Box<dyn std::error::Error>> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, DisableMouseCapture)?;
    let terminal = Terminal::new(CrosstermBackend::new(stdout))?;

    tokio::select! {
        res = run_ui(state_rx, ui_action_tx.clone(), terminal, shutdown_rx, config) => {
            res?;
        }
        _ = tokio::signal::ctrl_c() => {
            on_ctrl_c.await;
        }
    }

    disable_raw_mode()?;
    execute!(io::stdout(), LeaveAlternateScreen, DisableMouseCapture)?;

    Ok(())
}
