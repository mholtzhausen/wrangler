use serde::{Deserialize, Serialize};
use std::collections::HashSet;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProcessInfo {
    pub pid: u32,
    pub parent_pid: Option<u32>,
    pub name: String,
    pub cpu_usage: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ThrottledGroupInfo {
    pub group_key: u32,
    pub name: String,
    pub pids: Vec<u32>,
    pub cpu_total: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ThrottleLogEntry {
    pub pid: u32,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AppState {
    pub processes: Vec<ProcessInfo>,
    pub throttled_groups: Vec<ThrottledGroupInfo>,
    pub throttled_pids: HashSet<u32>,
    pub app_cap: f32,
    pub pressure_threshold: f32,
    pub global_cpu: f32,
    pub num_cores: usize,
    pub throttle_log: Vec<ThrottleLogEntry>,
    pub last_error: Option<String>,
    pub quitting: bool,
}

impl AppState {
    pub fn new(app_cap: f32, pressure_threshold: f32) -> Self {
        Self {
            processes: Vec::new(),
            throttled_groups: Vec::new(),
            throttled_pids: HashSet::new(),
            app_cap,
            pressure_threshold,
            global_cpu: 0.0,
            num_cores: 1,
            throttle_log: Vec::new(),
            last_error: None,
            quitting: false,
        }
    }

    pub fn sync_throttled_pids(&mut self) {
        self.throttled_pids = self
            .throttled_groups
            .iter()
            .flat_map(|group| group.pids.iter().copied())
            .collect();
    }

    pub fn push_log(&mut self, pid: u32, message: impl Into<String>) {
        self.throttle_log.push(ThrottleLogEntry {
            pid,
            message: message.into(),
        });
        if self.throttle_log.len() > 50 {
            let drain = self.throttle_log.len() - 50;
            self.throttle_log.drain(0..drain);
        }
    }
}
