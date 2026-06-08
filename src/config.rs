use serde::{Deserialize, Serialize};
use std::fs;
use std::io;
use std::path::PathBuf;

use crate::policy::GroupingMode;
use crate::runtime::RuntimeSettings;

pub const DEFAULT_APP_CAP: f32 = 40.0;
pub const DEFAULT_PRESSURE_THRESHOLD: f32 = 85.0;
pub const DEFAULT_TOP_OFFENDERS: usize = 1;
pub const DEFAULT_INTERVAL_MS: u64 = 1000;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Config {
    #[serde(default = "default_app_cap", alias = "threshold")]
    pub app_cap: f32,
    #[serde(default = "default_pressure_threshold")]
    pub pressure_threshold: f32,
    #[serde(default = "default_top_offenders")]
    pub top_offenders: usize,
    #[serde(default)]
    pub grouping: GroupingMode,
    #[serde(default)]
    pub protected_apps: Vec<String>,
    #[serde(default = "default_interval_ms")]
    pub interval_ms: u64,
    #[serde(default)]
    pub use_cgroups: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            app_cap: DEFAULT_APP_CAP,
            pressure_threshold: DEFAULT_PRESSURE_THRESHOLD,
            top_offenders: DEFAULT_TOP_OFFENDERS,
            grouping: GroupingMode::default(),
            protected_apps: Vec::new(),
            interval_ms: DEFAULT_INTERVAL_MS,
            use_cgroups: false,
        }
    }
}

fn default_app_cap() -> f32 {
    DEFAULT_APP_CAP
}

fn default_pressure_threshold() -> f32 {
    DEFAULT_PRESSURE_THRESHOLD
}

fn default_top_offenders() -> usize {
    DEFAULT_TOP_OFFENDERS
}

fn default_interval_ms() -> u64 {
    DEFAULT_INTERVAL_MS
}

pub fn clamp_app_cap(value: f32) -> f32 {
    value.clamp(1.0, 100.0)
}

pub fn clamp_pressure_threshold(value: f32) -> f32 {
    value.clamp(0.0, 100.0)
}

pub fn config_path() -> PathBuf {
    if let Ok(dir) = std::env::var("WRANGLER_CONFIG_DIR") {
        return PathBuf::from(dir).join("config.toml");
    }
    if let Ok(dir) = std::env::var("XDG_CONFIG_HOME") {
        return PathBuf::from(dir).join("wrangler").join("config.toml");
    }
    if let Ok(home) = std::env::var("HOME") {
        return PathBuf::from(home)
            .join(".config")
            .join("wrangler")
            .join("config.toml");
    }
    PathBuf::from("config.toml")
}

impl Config {
    pub fn load() -> Self {
        let path = config_path();
        let _ = migrate_legacy_config(&path);
        Self::load_from(&path)
    }

    pub fn load_from(path: &std::path::Path) -> Self {
        let Ok(contents) = fs::read_to_string(path) else {
            return Self::default();
        };
        toml::from_str(&contents).unwrap_or_else(|e| {
            tracing::warn!(path = %path.display(), error = %e, "invalid config; using defaults");
            Self::default()
        })
    }

    pub fn save(&self) -> io::Result<()> {
        self.save_to(&config_path())
    }

    pub fn save_to(&self, path: &std::path::Path) -> io::Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let contents = toml::to_string_pretty(self)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        fs::write(path, contents)
    }

    pub fn from_settings(settings: &RuntimeSettings) -> Self {
        Self {
            app_cap: settings.app_cap,
            pressure_threshold: settings.pressure_threshold,
            top_offenders: settings.top_offenders,
            grouping: settings.grouping,
            protected_apps: settings.protected_apps.clone(),
            interval_ms: settings.interval.as_millis() as u64,
            use_cgroups: settings.cgroups,
        }
    }

    pub fn update_app_cap(app_cap: f32) -> io::Result<()> {
        let path = config_path();
        Self::update_app_cap_at(&path, app_cap)
    }

    pub fn update_app_cap_at(path: &std::path::Path, app_cap: f32) -> io::Result<()> {
        let mut config = Self::load_from(path);
        config.app_cap = clamp_app_cap(app_cap);
        config.save_to(path)
    }
}

/// Rewrite legacy configs that still use `threshold` to the current schema.
pub fn migrate_legacy_config(path: &std::path::Path) -> io::Result<bool> {
    let Ok(contents) = fs::read_to_string(path) else {
        return Ok(false);
    };

    let uses_legacy = contents.contains("threshold") && !contents.contains("app_cap");
    if !uses_legacy {
        return Ok(false);
    }

    let config: Config = toml::from_str(&contents).map_err(|e| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("failed to migrate legacy config: {e}"),
        )
    })?;
    config.save_to(path)?;
    tracing::info!(path = %path.display(), "migrated config from threshold to app_cap schema");
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn temp_config_path() -> PathBuf {
        std::env::temp_dir().join(format!(
            "wrangler-config-{}-{}.toml",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn clamp_app_cap_bounds() {
        assert_eq!(clamp_app_cap(0.0), 1.0);
        assert_eq!(clamp_app_cap(50.0), 50.0);
        assert_eq!(clamp_app_cap(150.0), 100.0);
    }

    #[test]
    fn save_and_load_roundtrip() {
        let path = temp_config_path();
        let config = Config {
            app_cap: 55.0,
            pressure_threshold: 80.0,
            top_offenders: 2,
            grouping: GroupingMode::Name,
            protected_apps: vec!["firefox".into()],
            interval_ms: 500,
            use_cgroups: true,
        };
        config.save_to(&path).unwrap();
        let loaded = Config::load_from(&path);
        assert_eq!(loaded, config);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn update_app_cap_persists() {
        let path = temp_config_path();
        Config::default().save_to(&path).unwrap();
        Config::update_app_cap_at(&path, 42.0).unwrap();
        assert_eq!(Config::load_from(&path).app_cap, 42.0);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn missing_file_returns_defaults() {
        let path = temp_config_path();
        assert_eq!(Config::load_from(&path), Config::default());
    }

    #[test]
    fn legacy_threshold_key_loads_as_app_cap() {
        let path = temp_config_path();
        fs::write(&path, "threshold = 33.0\n").unwrap();
        assert_eq!(Config::load_from(&path).app_cap, 33.0);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn migrate_legacy_config_rewrites_file() {
        let path = temp_config_path();
        fs::write(&path, "threshold = 33.0\ninterval_ms = 750\n").unwrap();
        assert!(migrate_legacy_config(&path).unwrap());
        let migrated = fs::read_to_string(&path).unwrap();
        assert!(migrated.contains("app_cap"));
        assert!(
            !migrated
                .lines()
                .any(|line| line.trim_start().starts_with("threshold"))
        );
        assert_eq!(Config::load_from(&path).app_cap, 33.0);
        let _ = fs::remove_file(path);
    }
}
