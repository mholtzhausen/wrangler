use serde::{Deserialize, Serialize};
use std::fs;
use std::io;
use std::path::PathBuf;

use crate::runtime::RuntimeSettings;

pub const DEFAULT_THRESHOLD: f32 = 80.0;
pub const DEFAULT_INTERVAL_MS: u64 = 1000;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Config {
    #[serde(default = "default_threshold")]
    pub threshold: f32,
    #[serde(default = "default_interval_ms")]
    pub interval_ms: u64,
    #[serde(default)]
    pub use_cgroups: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            threshold: DEFAULT_THRESHOLD,
            interval_ms: DEFAULT_INTERVAL_MS,
            use_cgroups: false,
        }
    }
}

fn default_threshold() -> f32 {
    DEFAULT_THRESHOLD
}

fn default_interval_ms() -> u64 {
    DEFAULT_INTERVAL_MS
}

pub fn clamp_threshold(value: f32) -> f32 {
    value.clamp(1.0, 100.0)
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
        Self::load_from(&config_path())
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
            threshold: settings.threshold,
            interval_ms: settings.interval.as_millis() as u64,
            use_cgroups: settings.cgroups,
        }
    }

    pub fn update_threshold(threshold: f32) -> io::Result<()> {
        let path = config_path();
        Self::update_threshold_at(&path, threshold)
    }

    pub fn update_threshold_at(path: &std::path::Path, threshold: f32) -> io::Result<()> {
        let mut config = Self::load_from(path);
        config.threshold = clamp_threshold(threshold);
        config.save_to(path)
    }
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
    fn clamp_threshold_bounds() {
        assert_eq!(clamp_threshold(0.0), 1.0);
        assert_eq!(clamp_threshold(50.0), 50.0);
        assert_eq!(clamp_threshold(150.0), 100.0);
    }

    #[test]
    fn save_and_load_roundtrip() {
        let path = temp_config_path();
        let config = Config {
            threshold: 55.0,
            interval_ms: 500,
            use_cgroups: true,
        };
        config.save_to(&path).unwrap();
        let loaded = Config::load_from(&path);
        assert_eq!(loaded, config);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn update_threshold_persists() {
        let path = temp_config_path();
        Config::default().save_to(&path).unwrap();
        Config::update_threshold_at(&path, 42.0).unwrap();
        assert_eq!(Config::load_from(&path).threshold, 42.0);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn missing_file_returns_defaults() {
        let path = temp_config_path();
        assert_eq!(Config::load_from(&path), Config::default());
    }
}
