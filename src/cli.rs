use clap::Parser;
use std::time::Duration;

#[derive(Debug, Parser)]
#[command(name = "wrangler", about = "Process monitor and CPU throttle TUI")]
pub struct Cli {
    /// CPU usage threshold (%) above which processes are throttled
    #[arg(long, default_value_t = 80.0)]
    pub threshold: f32,

    /// Monitor polling interval in milliseconds
    #[arg(long, default_value_t = 1000)]
    pub interval: u64,

    /// Use cgroups v2 cpu.max instead of SIGSTOP/SIGCONT (requires root)
    #[arg(long)]
    pub cgroups: bool,
}

impl Cli {
    pub fn interval_duration(&self) -> Duration {
        Duration::from_millis(self.interval)
    }
}
