use nix::sys::signal::{self, Signal};
use nix::unistd::Pid;
use std::time::Duration;
use tokio::time::sleep;
use tokio_util::sync::CancellationToken;

const SLICE_MS: u64 = 40;

pub struct SignalThrottle;

impl SignalThrottle {
    pub async fn run_governor(
        pid: u32,
        cpu_total: f32,
        machine_budget: f32,
        cancel: CancellationToken,
    ) {
        let nix_pid = Pid::from_raw(pid as i32);
        let stop_fraction = compute_stop_fraction(cpu_total, machine_budget);

        loop {
            if cancel.is_cancelled() {
                break;
            }

            if !process_exists(pid) {
                break;
            }

            let stop_ms = ((SLICE_MS as f32) * stop_fraction).max(1.0) as u64;
            let run_ms = SLICE_MS.saturating_sub(stop_ms).max(1);

            if stop_fraction > 0.0 {
                let _ = signal::kill(nix_pid, Signal::SIGSTOP);
                tokio::select! {
                    _ = sleep(Duration::from_millis(stop_ms)) => {}
                    _ = cancel.cancelled() => {
                        let _ = signal::kill(nix_pid, Signal::SIGCONT);
                        return;
                    }
                }
            }

            let _ = signal::kill(nix_pid, Signal::SIGCONT);
            tokio::select! {
                _ = sleep(Duration::from_millis(run_ms)) => {}
                _ = cancel.cancelled() => {
                    let _ = signal::kill(nix_pid, Signal::SIGCONT);
                    return;
                }
            }
        }

        let _ = signal::kill(nix_pid, Signal::SIGCONT);
    }

    pub fn resume(pid: u32) {
        let nix_pid = Pid::from_raw(pid as i32);
        let _ = signal::kill(nix_pid, Signal::SIGCONT);
    }
}

fn compute_stop_fraction(cpu_total: f32, machine_budget: f32) -> f32 {
    if cpu_total <= machine_budget {
        return 0.5;
    }
    let excess = (cpu_total - machine_budget) / cpu_total;
    excess.clamp(0.1, 0.9)
}

fn process_exists(pid: u32) -> bool {
    std::path::Path::new(&format!("/proc/{pid}")).exists()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stop_fraction_increases_with_excess_cpu() {
        assert!(compute_stop_fraction(400.0, 320.0) > compute_stop_fraction(330.0, 320.0));
    }
}
