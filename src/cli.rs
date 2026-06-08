use clap::Parser;
use std::io::IsTerminal;
use std::time::Duration;

use crate::config::Config;
use crate::daemon::is_detached_child;
use crate::policy::GroupingMode;
use crate::runtime::RuntimeSettings;

#[derive(Debug, Parser)]
#[command(name = "wrangler", about = "Process monitor and CPU throttle TUI")]
pub struct Cli {
    /// Max % of machine CPU per app group (throttled when system is under pressure)
    #[arg(long, visible_alias = "threshold")]
    pub app_cap: Option<f32>,

    /// Global CPU % above which throttling may engage (0 = always evaluate)
    #[arg(long)]
    pub pressure_threshold: Option<f32>,

    /// Monitor polling interval in milliseconds
    #[arg(long)]
    pub interval: Option<u64>,

    /// Process grouping strategy: tree (default) or name
    #[arg(long, value_parser = parse_grouping)]
    pub grouping: Option<GroupingMode>,

    /// Request cgroups v2 cpu.max (only effective when running as root)
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
            app_cap: self
                .app_cap
                .map(crate::config::clamp_app_cap)
                .unwrap_or(file.app_cap),
            pressure_threshold: self
                .pressure_threshold
                .map(crate::config::clamp_pressure_threshold)
                .unwrap_or(file.pressure_threshold),
            top_offenders: file.top_offenders,
            grouping: self.grouping.unwrap_or(file.grouping),
            protected_apps: file.protected_apps.clone(),
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

fn parse_grouping(value: &str) -> Result<GroupingMode, String> {
    match value.to_ascii_lowercase().as_str() {
        "tree" => Ok(GroupingMode::Tree),
        "name" => Ok(GroupingMode::Name),
        other => Err(format!("invalid grouping '{other}' (expected tree or name)")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_overrides_config_file() {
        let file = Config {
            app_cap: 60.0,
            pressure_threshold: 85.0,
            top_offenders: 1,
            grouping: GroupingMode::Tree,
            protected_apps: Vec::new(),
            interval_ms: 2000,
            use_cgroups: false,
        };
        let cli = Cli {
            app_cap: Some(45.0),
            pressure_threshold: None,
            interval: Some(750),
            grouping: None,
            cgroups: false,
            daemon: false,
            foreground: false,
            no_tray: false,
            attach: false,
            status: false,
        };
        let settings = cli.resolve_with(&file);
        assert_eq!(settings.app_cap, 45.0);
        assert_eq!(settings.interval.as_millis(), 750);
    }

    #[test]
    fn config_used_when_cli_omits_flags() {
        let file = Config {
            app_cap: 60.0,
            pressure_threshold: 85.0,
            top_offenders: 1,
            grouping: GroupingMode::Tree,
            protected_apps: Vec::new(),
            interval_ms: 2000,
            use_cgroups: false,
        };
        let cli = Cli {
            app_cap: None,
            pressure_threshold: None,
            interval: None,
            grouping: None,
            cgroups: false,
            daemon: false,
            foreground: false,
            no_tray: false,
            attach: false,
            status: false,
        };
        let settings = cli.resolve_with(&file);
        assert_eq!(settings.app_cap, 60.0);
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

    #[test]
    fn threshold_alias_maps_to_app_cap() {
        let cli = Cli::try_parse_from(["wrangler", "--threshold", "55"]).unwrap();
        assert_eq!(cli.app_cap, Some(55.0));
    }

    #[test]
    fn grouping_flag_parses_name_mode() {
        let cli = Cli::try_parse_from(["wrangler", "--grouping", "name"]).unwrap();
        assert_eq!(cli.grouping, Some(GroupingMode::Name));
    }
}
