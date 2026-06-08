#[cfg(target_os = "linux")]
use ksni::menu::StandardItem;
#[cfg(target_os = "linux")]
use ksni::{Category, Handle, Status, Tray, TrayMethods};
#[cfg(target_os = "linux")]
use std::sync::mpsc;
#[cfg(target_os = "linux")]
use tokio::sync::mpsc as async_mpsc;

#[cfg(target_os = "linux")]
pub enum TrayAction {
    OpenDashboard,
    Quit,
}

#[cfg(target_os = "linux")]
pub(crate) struct WranglerTray {
    action_tx: mpsc::Sender<TrayAction>,
}

/// Bridge tray menu callbacks (sync, may run on tokio workers) to the async daemon loop.
#[cfg(target_os = "linux")]
pub fn action_channel() -> (mpsc::Sender<TrayAction>, async_mpsc::Receiver<TrayAction>) {
    let (std_tx, std_rx) = mpsc::channel();
    let (async_tx, async_rx) = async_mpsc::channel(16);

    tokio::spawn(async move {
        while let Ok(action) = std_rx.recv() {
            if async_tx.send(action).await.is_err() {
                break;
            }
        }
    });

    (std_tx, async_rx)
}

#[cfg(target_os = "linux")]
impl Tray for WranglerTray {
    fn id(&self) -> String {
        "wrangler".into()
    }

    fn category(&self) -> Category {
        Category::SystemServices
    }

    fn title(&self) -> String {
        "Wrangler".into()
    }

    fn status(&self) -> Status {
        Status::Active
    }

    fn icon_name(&self) -> String {
        "utilities-system-monitor".into()
    }

    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        vec![
            StandardItem {
                label: "Open Dashboard".into(),
                activate: Box::new(|tray: &mut Self| {
                    let _ = tray.action_tx.send(TrayAction::OpenDashboard);
                }),
                ..Default::default()
            }
            .into(),
            ksni::MenuItem::Separator,
            StandardItem {
                label: "Quit".into(),
                activate: Box::new(|tray: &mut Self| {
                    let _ = tray.action_tx.send(TrayAction::Quit);
                }),
                ..Default::default()
            }
            .into(),
        ]
    }
}

#[cfg(target_os = "linux")]
pub async fn spawn(
    action_tx: mpsc::Sender<TrayAction>,
) -> Result<Handle<WranglerTray>, ksni::Error> {
    WranglerTray { action_tx }
        .assume_sni_available(true)
        .spawn()
        .await
}
