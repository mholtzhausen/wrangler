use serde::{Deserialize, Serialize};
use std::collections::HashSet;

use crate::policy::AppGroup;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProcessInfo {
    pub pid: u32,
    pub parent_pid: Option<u32>,
    pub name: String,
    pub user: String,
    pub group: String,
    pub cpu_usage: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AppGroupInfo {
    pub group_key: u32,
    pub name: String,
    pub user: String,
    pub group: String,
    pub pids: Vec<u32>,
    pub cpu_total: f32,
}

pub fn summarize_identities<'a, I>(values: I) -> String
where
    I: Iterator<Item = &'a str>,
{
    let mut unique: Vec<&str> = values.filter(|value| !value.is_empty()).collect();
    unique.sort_unstable();
    unique.dedup();
    match unique.len() {
        0 => "-".to_string(),
        1 => unique[0].to_string(),
        _ => unique.join(","),
    }
}

pub fn identities_for_pids(pids: &[u32], processes: &[ProcessInfo]) -> (String, String) {
    let members: Vec<&ProcessInfo> = pids
        .iter()
        .filter_map(|pid| processes.iter().find(|process| process.pid == *pid))
        .collect();
    (
        summarize_identities(members.iter().map(|process| process.user.as_str())),
        summarize_identities(members.iter().map(|process| process.group.as_str())),
    )
}

impl From<AppGroup> for AppGroupInfo {
    fn from(group: AppGroup) -> Self {
        Self {
            group_key: group.key,
            name: group.name,
            user: "-".to_string(),
            group: "-".to_string(),
            pids: group.pids,
            cpu_total: group.cpu_total,
        }
    }
}

impl From<&AppGroup> for AppGroupInfo {
    fn from(group: &AppGroup) -> Self {
        Self {
            group_key: group.key,
            name: group.name.clone(),
            user: "-".to_string(),
            group: "-".to_string(),
            pids: group.pids.clone(),
            cpu_total: group.cpu_total,
        }
    }
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
    pub groups: Vec<AppGroupInfo>,
    pub throttled_groups: Vec<ThrottledGroupInfo>,
    pub throttled_pids: HashSet<u32>,
    pub app_cap: f32,
    pub pressure_threshold: f32,
    pub global_cpu: f32,
    pub num_cores: usize,
    pub grouping: String,
    pub protected_apps: Vec<String>,
    pub throttle_backend: String,
    pub throttle_log: Vec<ThrottleLogEntry>,
    pub last_error: Option<String>,
    pub quitting: bool,
}

impl AppState {
    pub fn new(
        app_cap: f32,
        pressure_threshold: f32,
        throttle_backend: impl Into<String>,
        grouping: impl Into<String>,
        protected_apps: Vec<String>,
    ) -> Self {
        Self {
            processes: Vec::new(),
            groups: Vec::new(),
            throttled_groups: Vec::new(),
            throttled_pids: HashSet::new(),
            app_cap,
            pressure_threshold,
            global_cpu: 0.0,
            num_cores: 1,
            grouping: grouping.into(),
            protected_apps,
            throttle_backend: throttle_backend.into(),
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

    pub fn is_group_throttled(&self, group_key: u32) -> bool {
        self.throttled_groups
            .iter()
            .any(|group| group.group_key == group_key)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summarize_identities_collapses_duplicates() {
        assert_eq!(summarize_identities(["alice"].into_iter()), "alice");
        assert_eq!(
            summarize_identities(["bob", "alice", "bob"].into_iter()),
            "alice,bob"
        );
        assert_eq!(summarize_identities([].into_iter()), "-");
    }
}
