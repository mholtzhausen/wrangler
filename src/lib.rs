pub mod app;
pub mod cli;
pub mod config;
pub mod daemon;
pub mod dashboard;
pub mod event;
pub mod instance;
pub mod ipc;
pub mod monitor;
pub mod policy;
pub mod runtime;
pub mod session_env;
#[cfg(target_os = "linux")]
pub mod service;
pub mod throttle;
pub mod ui;

#[cfg(target_os = "linux")]
pub mod tray;

use clap::Parser;
use cli::{Cli, Commands};
use crossterm::{
    event::DisableMouseCapture,
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
    event::{KeyboardEnhancementFlags, PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags},
};
use event::HubCommand;
use ipc::AttachSession;
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use runtime::spawn_core;
use std::io;
use std::sync::Arc;
use tokio::sync::{mpsc, watch, Mutex};
use ui::{run_ui, UiAction, UiConfig};

pub async fn run() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();
    session_env::restore_invoking_user_session();

    let cli = Cli::parse();

    #[cfg(target_os = "linux")]
    if let Some(Commands::Service(service_args)) = &cli.command {
        return service::run(&service_args.command);
    }

    #[cfg(not(target_os = "linux"))]
    if cli.command.is_some() {
        return Err("service commands are only supported on Linux".into());
    }

    if cli.status {
        return print_daemon_status().await;
    }

    let settings = cli.resolve();

    if cli.daemon_mode() {
        if ipc::daemon_running().await {
            tracing::error!("wrangler daemon is already running");
            std::process::exit(1);
        }
        if cli.should_detach_from_terminal() {
            config::Config::from_settings(&settings).save()?;
            daemon::spawn_detached_child()?;
        }
        let _lock = instance::acquire_daemon_lock()?;
        config::Config::from_settings(&settings).save()?;
        return daemon::run_from_cli(&cli, settings).await;
    }

    if cli.attach || ipc::daemon_running().await {
        if !cli.attach {
            tracing::info!("daemon detected; attaching dashboard");
        }
        return run_attach_tui().await;
    }

    config::Config::from_settings(&settings).save()?;
    let core = spawn_core(&settings);
    run_standalone_tui(core).await
}

async fn print_daemon_status() -> Result<(), Box<dyn std::error::Error>> {
    let session = AttachSession::connect()
        .await
        .map_err(|e| format!("failed to connect to wrangler daemon: {e}"))?;
    let state = session.state_rx().borrow().clone();
    println!("{}", serde_json::to_string_pretty(&state)?);
    session.detach().await?;
    Ok(())
}

async fn run_standalone_tui(core: runtime::CoreRuntime) -> Result<(), Box<dyn std::error::Error>> {
    let hub = core.hub;

    let (ui_action_tx, mut ui_action_rx) = mpsc::channel::<UiAction>(16);
    let hub_for_ui = hub.command_tx.clone();
    tokio::spawn(async move {
        while let Some(action) = ui_action_rx.recv().await {
            match action {
                UiAction::SetAppCap(t) => {
                    let _ = hub_for_ui.send(HubCommand::SetAppCap(t)).await;
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
                UiAction::SetAppCap(t) => {
                    let _ = active.set_app_cap(t).await;
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
    let _ = execute!(
        stdout,
        PushKeyboardEnhancementFlags(
            KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
                | KeyboardEnhancementFlags::REPORT_EVENT_TYPES
        )
    );
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
    execute!(
        io::stdout(),
        PopKeyboardEnhancementFlags,
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;

    Ok(())
}
