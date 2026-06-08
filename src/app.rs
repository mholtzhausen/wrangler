use std::collections::HashSet;

#[derive(Debug, Clone)]
pub struct ProcessInfo {
    pub pid: u32,
    pub name: String,
    pub cpu_usage: f32,
}

#[derive(Debug, Clone)]
pub struct ThrottleLogEntry {
    pub pid: u32,
    pub message: String,
}

#[derive(Debug, Clone)]
pub struct AppState {
    pub processes: Vec<ProcessInfo>,
    pub throttled_pids: HashSet<u32>,
    pub cpu_threshold: f32,
    pub throttle_log: Vec<ThrottleLogEntry>,
    pub last_error: Option<String>,
    pub quitting: bool,
}

impl AppState {
    pub fn new(cpu_threshold: f32) -> Self {
        Self {
            processes: Vec::new(),
            throttled_pids: HashSet::new(),
            cpu_threshold,
            throttle_log: Vec::new(),
            last_error: None,
            quitting: false,
        }
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
