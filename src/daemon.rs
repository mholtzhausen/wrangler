use crate::cli::Cli;
use crate::dashboard;
use crate::event::HubCommand;
use crate::runtime::{CoreRuntime, RuntimeSettings};
use tokio::sync::mpsc;

#[cfg(target_os = "linux")]
use crate::tray::{self, TrayAction};

pub async fn run(core: CoreRuntime, cli: &Cli) -> Result<(), Box<dyn std::error::Error>> {
    if cli.tray_enabled() {
        warn_if_tray_unavailable();
    }

    let hub_cmd = core.hub.command_tx.clone();
    let mut shutdown_rx = core.hub.subscribe_shutdown();

    #[cfg(target_os = "linux")]
    let mut tray_context = if cli.tray_enabled() {
        spawn_tray_context().await
    } else {
        TrayContext::disabled()
    };

    loop {
        #[cfg(target_os = "linux")]
        let should_break =
            run_daemon_iteration(&hub_cmd, &mut shutdown_rx, &mut tray_context).await?;

        #[cfg(not(target_os = "linux"))]
        let should_break = run_daemon_iteration(&hub_cmd, &mut shutdown_rx).await?;

        if should_break {
            break;
        }
    }

    #[cfg(target_os = "linux")]
    if let Some(handle) = tray_context.handle.take() {
        handle.shutdown();
    }

    Ok(())
}

#[cfg(target_os = "linux")]
struct TrayContext {
    rx: Option<mpsc::Receiver<TrayAction>>,
    handle: Option<ksni::Handle<crate::tray::WranglerTray>>,
}

#[cfg(target_os = "linux")]
impl TrayContext {
    fn disabled() -> Self {
        Self {
            rx: None,
            handle: None,
        }
    }
}

#[cfg(target_os = "linux")]
async fn spawn_tray_context() -> TrayContext {
    let (tx, rx) = tray::action_channel();
    match tray::spawn(tx).await {
        Ok(handle) => {
            tracing::info!("system tray active");
            TrayContext {
                rx: Some(rx),
                handle: Some(handle),
            }
        }
        Err(e) => {
            tracing::warn!(error = %e, "system tray unavailable; continuing headless");
            TrayContext::disabled()
        }
    }
}

#[cfg(target_os = "linux")]
async fn run_daemon_iteration(
    hub_cmd: &mpsc::Sender<HubCommand>,
    shutdown_rx: &mut tokio::sync::watch::Receiver<bool>,
    tray_context: &mut TrayContext,
) -> Result<bool, Box<dyn std::error::Error>> {
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {
            tracing::info!("received SIGINT, shutting down");
            let _ = hub_cmd.send(HubCommand::Quit).await;
            Ok(true)
        }
        _ = wait_sigterm() => {
            tracing::info!("received SIGTERM, shutting down");
            let _ = hub_cmd.send(HubCommand::Quit).await;
            Ok(true)
        }
        changed = shutdown_rx.changed() => {
            Ok(changed.is_ok() && *shutdown_rx.borrow())
        }
        action = recv_tray_action(tray_context) => {
            if let Some(action) = action {
                handle_tray_action(action, hub_cmd).await;
            }
            Ok(false)
        }
    }
}

#[cfg(not(target_os = "linux"))]
async fn run_daemon_iteration(
    hub_cmd: &mpsc::Sender<HubCommand>,
    shutdown_rx: &mut tokio::sync::watch::Receiver<bool>,
) -> Result<bool, Box<dyn std::error::Error>> {
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {
            tracing::info!("received SIGINT, shutting down");
            let _ = hub_cmd.send(HubCommand::Quit).await;
            Ok(true)
        }
        _ = wait_sigterm() => {
            tracing::info!("received SIGTERM, shutting down");
            let _ = hub_cmd.send(HubCommand::Quit).await;
            Ok(true)
        }
        changed = shutdown_rx.changed() => {
            Ok(changed.is_ok() && *shutdown_rx.borrow())
        }
    }
}

#[cfg(target_os = "linux")]
async fn recv_tray_action(ctx: &mut TrayContext) -> Option<TrayAction> {
    match &mut ctx.rx {
        Some(channel) => channel.recv().await,
        None => std::future::pending().await,
    }
}

#[cfg(target_os = "linux")]
async fn handle_tray_action(action: TrayAction, hub_cmd: &mpsc::Sender<HubCommand>) {
    match action {
        TrayAction::OpenDashboard => dashboard::open_dashboard(),
        TrayAction::Quit => {
            let _ = hub_cmd.send(HubCommand::Quit).await;
        }
    }
}

#[cfg(not(target_os = "linux"))]
fn warn_if_tray_unavailable() {
    tracing::warn!("system tray is only supported on Linux; running headless");
}

#[cfg(target_os = "linux")]
fn warn_if_tray_unavailable() {}

async fn wait_sigterm() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        if let Ok(mut stream) = signal(SignalKind::terminate()) {
            stream.recv().await;
            return;
        }
    }
    std::future::pending::<()>().await;
}

pub async fn run_from_cli(
    cli: &Cli,
    settings: RuntimeSettings,
) -> Result<(), Box<dyn std::error::Error>> {
    let core = crate::runtime::spawn_core(&settings);
    let _ipc =
        crate::ipc::spawn_server(core.hub.command_tx.clone(), core.hub.state_rx.clone()).await?;
    run(core, cli).await
}
