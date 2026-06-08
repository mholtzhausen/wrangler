mod cgroup;
mod signal;

use tokio_util::sync::CancellationToken;

pub enum ThrottleBackend {
    Signal,
    Cgroup(cgroup::CgroupThrottle),
}

impl ThrottleBackend {
    pub fn from_cli(use_cgroups: bool) -> Self {
        if use_cgroups {
            ThrottleBackend::Cgroup(cgroup::CgroupThrottle::new())
        } else {
            ThrottleBackend::Signal
        }
    }

    pub fn is_cgroup(&self) -> bool {
        matches!(self, ThrottleBackend::Cgroup(_))
    }
}

pub async fn run_signal_governor(
    pid: u32,
    cpu_usage: f32,
    threshold: f32,
    cancel: CancellationToken,
) {
    signal::SignalThrottle::run_governor(pid, cpu_usage, threshold, cancel).await;
}

pub fn resume_signal(pid: u32) {
    signal::SignalThrottle::resume(pid);
}

impl ThrottleBackend {
    pub async fn start_cgroup(
        &mut self,
        pid: u32,
        cpu_usage: f32,
        threshold: f32,
        cancel: CancellationToken,
    ) -> Result<(), String> {
        if let ThrottleBackend::Cgroup(cg) = self {
            cg.start(pid, cpu_usage, threshold)?;
            cancel.cancelled().await;
            cg.stop(pid);
            Ok(())
        } else {
            Err("not a cgroup backend".into())
        }
    }

    pub async fn stop_pid(&mut self, pid: u32) {
        match self {
            ThrottleBackend::Signal => resume_signal(pid),
            ThrottleBackend::Cgroup(cg) => cg.stop(pid),
        }
    }

    pub async fn stop_all(&mut self) {
        if let ThrottleBackend::Cgroup(cg) = self {
            cg.stop_all();
        }
    }
}
