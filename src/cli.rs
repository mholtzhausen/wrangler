use clap::Parser;
use std::io::IsTerminal;
use std::time::Duration;

use crate::config::Config;
use crate::daemon::is_detached_child;
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

    /// Run in background without TUI (monitor + throttle; tray enabled unless --no-tray)
    #[arg(long, visible_alias = "tray")]
    pub daemon: bool,

    /// Keep the daemon in the foreground (for systemd and debugging)
    #[arg(long)]
    pub foreground: bool,

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

    pub fn daemon_mode(&self) -> bool {
        self.daemon
    }

    pub fn tray_enabled(&self) -> bool {
        self.daemon_mode() && !self.no_tray
    }

    /// Detach from the launching terminal when started interactively (e.g. `wrangler --tray`).
    pub fn should_detach_from_terminal(&self) -> bool {
        self.daemon_mode()
            && !self.foreground
            && !is_detached_child()
            && std::io::stdin().is_terminal()
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
            foreground: false,
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
            foreground: false,
            no_tray: false,
            attach: false,
            status: false,
        };
        let settings = cli.resolve_with(&file);
        assert_eq!(settings.threshold, 60.0);
        assert_eq!(settings.interval.as_millis(), 2000);
    }

    #[test]
    fn tray_alias_enables_daemon_with_tray() {
        let cli = Cli::try_parse_from(["wrangler", "--tray"]).unwrap();
        assert!(cli.daemon_mode());
        assert!(cli.tray_enabled());
        assert!(!cli.no_tray);
    }

    #[test]
    fn foreground_prevents_detach() {
        let cli = Cli::try_parse_from(["wrangler", "--tray", "--foreground"]).unwrap();
        assert!(!cli.should_detach_from_terminal());
    }
}
