use crate::app::ProcessInfo;
use crate::event::HubCommand;
use std::collections::HashSet;
use std::time::Duration;
use sysinfo::{CpuRefreshKind, ProcessRefreshKind, System};
use tokio::sync::mpsc::Sender;
use tokio::sync::watch;
use tokio::time::sleep;

pub struct MonitorConfig {
    pub interval: Duration,
    pub self_pid: u32,
    pub protected_pids: HashSet<u32>,
}

pub async fn run_monitor_with_threshold(
    hub_tx: Sender<HubCommand>,
    mut threshold_rx: watch::Receiver<f32>,
    config: MonitorConfig,
) {
    let mut sys = System::new_with_specifics(
        sysinfo::RefreshKind::new()
            .with_processes(ProcessRefreshKind::everything())
            .with_cpu(CpuRefreshKind::everything()),
    );

    let mut governed: HashSet<u32> = HashSet::new();

    loop {
        sys.refresh_cpu();
        sys.refresh_processes();

        let threshold = *threshold_rx.borrow_and_update();

        let mut processes: Vec<ProcessInfo> = Vec::new();
        let mut seen_pids: HashSet<u32> = HashSet::new();

        for (pid, process) in sys.processes() {
            let pid_u32 = pid.as_u32();
            if config.protected_pids.contains(&pid_u32) || pid_u32 == config.self_pid {
                continue;
            }

            seen_pids.insert(pid_u32);
            let cpu = process.cpu_usage();
            processes.push(ProcessInfo {
                pid: pid_u32,
                name: process.name().to_string(),
                cpu_usage: cpu,
            });

            if cpu > threshold {
                if !governed.contains(&pid_u32) {
                    governed.insert(pid_u32);
                    let _ = hub_tx
                        .send(HubCommand::StartThrottle {
                            pid: pid_u32,
                            cpu_usage: cpu,
                        })
                        .await;
                }
            } else if governed.contains(&pid_u32) {
                governed.remove(&pid_u32);
                let _ = hub_tx.send(HubCommand::StopThrottle { pid: pid_u32 }).await;
            }
        }

        let exited: Vec<u32> = governed
            .iter()
            .filter(|p| !seen_pids.contains(p))
            .copied()
            .collect();
        for pid in exited {
            governed.remove(&pid);
            let _ = hub_tx.send(HubCommand::StopThrottle { pid }).await;
        }

        processes.sort_by(|a, b| {
            b.cpu_usage
                .partial_cmp(&a.cpu_usage)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        let _ = hub_tx.send(HubCommand::ProcessSnapshot(processes)).await;

        sleep(config.interval).await;
    }
}

pub fn protected_pids() -> HashSet<u32> {
    let mut set = HashSet::new();
    set.insert(std::process::id());
    set.insert(nix::unistd::getppid().as_raw() as u32);
    set
}
