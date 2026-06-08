use crate::cli::Cli;
use crate::event::{spawn_event_hub, HubHandles};
use crate::monitor::{protected_pids, run_monitor_with_threshold, MonitorConfig};
use crate::throttle::ThrottleBackend;
use tokio::sync::watch;

pub struct CoreRuntime {
    pub hub: HubHandles,
}

pub fn spawn_core(cli: &Cli) -> CoreRuntime {
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

    CoreRuntime { hub }
}
