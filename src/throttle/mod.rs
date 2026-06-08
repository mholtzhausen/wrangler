mod cgroup;
mod signal;

use tokio_util::sync::CancellationToken;

pub enum ThrottleBackend {
    Signal,
    Cgroup(cgroup::CgroupThrottle),
}

impl ThrottleBackend {
    pub fn resolve(explicit_cgroups: bool) -> Self {
        let is_root = nix::unistd::geteuid().is_root();
        if is_root {
            ThrottleBackend::Cgroup(cgroup::CgroupThrottle::new())
        } else {
            if explicit_cgroups {
                tracing::warn!("cgroups require root; using SIGSTOP/SIGCONT");
            }
            ThrottleBackend::Signal
        }
    }

    pub fn is_cgroup(&self) -> bool {
        matches!(self, ThrottleBackend::Cgroup(_))
    }
}

pub async fn run_signal_governor(
    pid: u32,
    cpu_total: f32,
    machine_budget: f32,
    cancel: CancellationToken,
) {
    signal::SignalThrottle::run_governor(pid, cpu_total, machine_budget, cancel).await;
}

pub fn resume_signal(pid: u32) {
    signal::SignalThrottle::resume(pid);
}

impl ThrottleBackend {
    pub async fn start_group_cgroup(
        &mut self,
        group_key: u32,
        pids: &[u32],
        app_cap: f32,
        num_cores: usize,
        cancel: CancellationToken,
    ) -> Result<(), String> {
        if let ThrottleBackend::Cgroup(cg) = self {
            cg.start_group(group_key, pids, app_cap, num_cores)?;
            cancel.cancelled().await;
            cg.stop_group(group_key);
            Ok(())
        } else {
            Err("not a cgroup backend".into())
        }
    }

    pub async fn sync_group_cgroup(
        &mut self,
        group_key: u32,
        pids: &[u32],
    ) -> Result<(), String> {
        if let ThrottleBackend::Cgroup(cg) = self {
            cg.sync_group(group_key, pids)
        } else {
            Err("not a cgroup backend".into())
        }
    }

    pub async fn stop_group(&mut self, group_key: u32, pids: &[u32]) {
        match self {
            ThrottleBackend::Signal => {
                for pid in pids {
                    resume_signal(*pid);
                }
            }
            ThrottleBackend::Cgroup(cg) => cg.stop_group(group_key),
        }
    }

    pub async fn stop_all(&mut self) {
        if let ThrottleBackend::Cgroup(cg) = self {
            cg.stop_all();
        }
    }
}
