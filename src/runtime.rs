use std::time::Duration;

use crate::cli::Cli;
use crate::event::{spawn_event_hub, HubHandles};
use crate::monitor::{protected_pids, run_monitor, MonitorConfig};
use crate::policy::{effective_protected_apps, GroupingMode, PolicySettings};
use crate::throttle::ThrottleBackend;
use tokio::sync::watch;

#[derive(Debug, Clone)]
pub struct RuntimeSettings {
    pub app_cap: f32,
    pub pressure_threshold: f32,
    pub top_offenders: usize,
    pub grouping: GroupingMode,
    pub protected_apps: Vec<String>,
    pub interval: Duration,
    pub cgroups: bool,
}

pub struct CoreRuntime {
    pub hub: HubHandles,
}

pub fn spawn_core(settings: &RuntimeSettings) -> CoreRuntime {
    let backend = ThrottleBackend::resolve(settings.cgroups);
    let protected_apps = effective_protected_apps(&settings.protected_apps);
    let hub = spawn_event_hub(
        settings.app_cap,
        settings.pressure_threshold,
        settings.grouping,
        protected_apps.clone(),
        backend,
    );

    let top_offenders = settings.top_offenders;
    let grouping = settings.grouping;
    let (policy_watch_tx, policy_watch_rx) = watch::channel(PolicySettings {
        app_cap: settings.app_cap,
        pressure_threshold: settings.pressure_threshold,
        top_offenders,
        grouping,
    });

    {
        let mut state_rx = hub.state_rx.clone();
        tokio::spawn(async move {
            loop {
                if state_rx.changed().await.is_err() {
                    break;
                }
                let state = state_rx.borrow().clone();
                let _ = policy_watch_tx.send(PolicySettings {
                    app_cap: state.app_cap,
                    pressure_threshold: state.pressure_threshold,
                    top_offenders,
                    grouping,
                });
            }
        });
    }

    let monitor_config = MonitorConfig {
        interval: settings.interval,
        self_pid: std::process::id(),
        protected_pids: protected_pids(),
        protected_apps: settings.protected_apps.clone(),
        grouping: settings.grouping,
        policy: PolicySettings {
            app_cap: settings.app_cap,
            pressure_threshold: settings.pressure_threshold,
            top_offenders: settings.top_offenders,
            grouping: settings.grouping,
        },
    };

    let hub_tx_monitor = hub.command_tx.clone();
    tokio::spawn(async move {
        run_monitor(hub_tx_monitor, policy_watch_rx, monitor_config).await;
    });

    CoreRuntime { hub }
}

#[allow(dead_code)]
pub fn spawn_core_from_cli(cli: &Cli) -> CoreRuntime {
    spawn_core(&cli.resolve())
}
