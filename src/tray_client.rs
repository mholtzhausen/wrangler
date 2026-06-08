#[cfg(target_os = "linux")]
use crate::dashboard;
#[cfg(target_os = "linux")]
use crate::ipc::AttachSession;
#[cfg(target_os = "linux")]
use crate::tray::{self, TrayAction};

#[cfg(target_os = "linux")]
pub async fn run() -> Result<(), Box<dyn std::error::Error>> {
    wait_for_daemon().await?;

    let session = AttachSession::connect()
        .await
        .map_err(|error| format!("failed to connect tray client to daemon: {error}"))?;
    let mut state_rx = session.state_rx();

    let (tx, mut rx) = tray::action_channel();
    let handle = tray::spawn(tx)
        .await
        .map_err(|error| format!("system tray unavailable: {error}"))?;
    tracing::info!("tray client connected to daemon");

    loop {
        tokio::select! {
            action = rx.recv() => {
                match action {
                    Some(TrayAction::OpenDashboard) => dashboard::open_dashboard(),
                    Some(TrayAction::Quit) => {
                        session.quit_daemon().await?;
                        break;
                    }
                    None => break,
                }
            }
            changed = state_rx.changed() => {
                if changed.is_ok() && state_rx.borrow_and_update().quitting {
                    tracing::info!("daemon stopped; exiting tray client");
                    break;
                }
            }
        }
    }

    handle.shutdown();
    session.detach().await?;
    Ok(())
}

#[cfg(target_os = "linux")]
async fn wait_for_daemon() -> Result<(), Box<dyn std::error::Error>> {
    const MAX_ATTEMPTS: usize = 30;
    for attempt in 1..=MAX_ATTEMPTS {
        if crate::ipc::daemon_running().await {
            return Ok(());
        }
        if attempt == MAX_ATTEMPTS {
            break;
        }
        tracing::debug!(
            attempt,
            max = MAX_ATTEMPTS,
            "waiting for wrangler daemon IPC socket"
        );
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }
    Err("no wrangler daemon is running; start the daemon first".into())
}
