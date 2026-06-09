#[cfg(target_os = "linux")]
use crate::app::AppState;
#[cfg(target_os = "linux")]
use crate::tray_indicator::{dot_icon, PressureColor, PressureTracker};
#[cfg(target_os = "linux")]
use ksni::menu::StandardItem;
#[cfg(target_os = "linux")]
use ksni::{Category, Handle, Status, ToolTip, Tray, TrayMethods};
#[cfg(target_os = "linux")]
use std::sync::mpsc;
#[cfg(target_os = "linux")]
use std::time::Duration;
#[cfg(target_os = "linux")]
use tokio::sync::mpsc as async_mpsc;
#[cfg(target_os = "linux")]
use tokio::sync::watch;
#[cfg(target_os = "linux")]
use tokio::task::JoinHandle;
#[cfg(target_os = "linux")]
use tokio::time::MissedTickBehavior;

#[cfg(target_os = "linux")]
pub enum TrayAction {
    OpenDashboard,
    Quit,
}

#[cfg(target_os = "linux")]
pub(crate) struct WranglerTray {
    action_tx: mpsc::Sender<TrayAction>,
    pressure_color: PressureColor,
    pressure_avg: f32,
    icon_pixmap: Vec<ksni::Icon>,
    tooltip_title: String,
    tooltip_body: String,
}

#[cfg(target_os = "linux")]
impl WranglerTray {
    fn new(action_tx: mpsc::Sender<TrayAction>) -> Self {
        let mut tray = Self {
            action_tx,
            pressure_color: PressureColor::Green,
            pressure_avg: 0.0,
            icon_pixmap: Vec::new(),
            tooltip_title: String::new(),
            tooltip_body: String::new(),
        };
        tray.set_pressure(PressureColor::Green, 0.0);
        tray
    }

    fn set_pressure(&mut self, color: PressureColor, avg_cpu: f32) {
        self.pressure_color = color;
        self.pressure_avg = avg_cpu;
        self.icon_pixmap = vec![dot_icon(color)];
        self.tooltip_title = format!("Wrangler · {:.0}% CPU", avg_cpu);
        self.tooltip_body = format!(
            "Machine pressure: {} (5s avg {:.0}%)",
            color.label(),
            avg_cpu
        );
    }
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
        self.tooltip_title.clone()
    }

    fn status(&self) -> Status {
        Status::Active
    }

    fn icon_name(&self) -> String {
        String::new()
    }

    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        self.icon_pixmap.clone()
    }

    fn tool_tip(&self) -> ToolTip {
        ToolTip {
            icon_name: String::new(),
            icon_pixmap: self.icon_pixmap.clone(),
            title: self.tooltip_title.clone(),
            description: self.tooltip_body.clone(),
        }
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
pub(crate) async fn spawn_with_state(
    action_tx: mpsc::Sender<TrayAction>,
    state_rx: watch::Receiver<AppState>,
) -> Result<(Handle<WranglerTray>, JoinHandle<()>), ksni::Error> {
    let tray = WranglerTray::new(action_tx);
    let handle = tray
        .assume_sni_available(true)
        .spawn()
        .await?;
    let pressure_task = spawn_pressure_watcher(handle.clone(), state_rx);
    Ok((handle, pressure_task))
}

#[cfg(target_os = "linux")]
fn spawn_pressure_watcher(
    handle: Handle<WranglerTray>,
    mut state_rx: watch::Receiver<AppState>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut tracker = PressureTracker::new(Duration::from_secs(5));
        let mut last_color = PressureColor::Green;
        let mut interval = tokio::time::interval(Duration::from_millis(500));
        interval.set_missed_tick_behavior(MissedTickBehavior::Skip);

        tracker.record(state_rx.borrow().global_cpu);

        loop {
            tokio::select! {
                changed = state_rx.changed() => {
                    if changed.is_err() {
                        break;
                    }
                    tracker.record(state_rx.borrow_and_update().global_cpu);
                }
                _ = interval.tick() => {}
            }

            let avg = tracker.average();
            let color = PressureColor::from_cpu(avg);
            if color == last_color {
                continue;
            }

            last_color = color;
            if handle
                .update(|tray| tray.set_pressure(color, avg))
                .await
                .is_none()
            {
                break;
            }
        }
    })
}
