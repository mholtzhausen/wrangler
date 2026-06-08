use clap::Parser;
use std::time::Duration;

use crate::config::Config;
use crate::runtime::RuntimeSettings;

#[derive(Debug, Parser)]
#[command(name = "wrangler", about = "Process monitor and CPU throttle TUI")]
pub struct Cli {
    /// CPU usage threshold (%) above which processes are throttled
    #[arg(long)]
    pub threshold: Option<f32>,

    /// Monitor polling interval in milliseconds
    #[arg(long)]
    pub interval: Option<u64>,

    /// Use cgroups v2 cpu.max instead of SIGSTOP/SIGCONT (requires root)
    #[arg(long)]
    pub cgroups: bool,

    /// Run in background without TUI (monitor + throttle only)
    #[arg(long)]
    pub daemon: bool,

    /// Disable system tray when running as a daemon
    #[arg(long)]
    pub no_tray: bool,

    /// Attach TUI to a running daemon (used by the tray dashboard launcher)
    #[arg(long)]
    pub attach: bool,

    /// Print daemon state as JSON and exit (requires a running daemon)
    #[arg(long)]
    pub status: bool,
}

impl Cli {
    pub fn resolve(&self) -> RuntimeSettings {
        self.resolve_with(&Config::load())
    }

    pub fn resolve_with(&self, file: &Config) -> RuntimeSettings {
        RuntimeSettings {
            threshold: self
                .threshold
                .map(crate::config::clamp_threshold)
                .unwrap_or(file.threshold),
            interval: Duration::from_millis(self.interval.unwrap_or(file.interval_ms)),
            cgroups: self.cgroups || file.use_cgroups,
        }
    }

    pub fn tray_enabled(&self) -> bool {
        self.daemon && !self.no_tray
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_overrides_config_file() {
        let file = Config {
            threshold: 60.0,
            interval_ms: 2000,
            use_cgroups: false,
        };
        let cli = Cli {
            threshold: Some(45.0),
            interval: Some(750),
            cgroups: false,
            daemon: false,
            no_tray: false,
            attach: false,
            status: false,
        };
        let settings = cli.resolve_with(&file);
        assert_eq!(settings.threshold, 45.0);
        assert_eq!(settings.interval.as_millis(), 750);
    }

    #[test]
    fn config_used_when_cli_omits_flags() {
        let file = Config {
            threshold: 60.0,
            interval_ms: 2000,
            use_cgroups: false,
        };
        let cli = Cli {
            threshold: None,
            interval: None,
            cgroups: false,
            daemon: false,
            no_tray: false,
            attach: false,
            status: false,
        };
        let settings = cli.resolve_with(&file);
        assert_eq!(settings.threshold, 60.0);
        assert_eq!(settings.interval.as_millis(), 2000);
    }
}
