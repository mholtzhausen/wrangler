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
pub struct GroupBehaviorRecord {
    pub group_key: u32,
    pub name: String,
    pub times_throttled: u32,
    pub peak_cpu: f32,
    pub last_cpu: f32,
    pub throttle_seconds: u64,
    pub currently_throttled: bool,
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
    pub group_behavior: Vec<GroupBehaviorRecord>,
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
            group_behavior: Vec::new(),
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

    pub fn record_throttle_start(&mut self, group_key: u32, name: impl Into<String>, cpu: f32) {
        let name = name.into();
        if let Some(record) = self
            .group_behavior
            .iter_mut()
            .find(|record| record.group_key == group_key)
        {
            record.name = name;
            record.times_throttled = record.times_throttled.saturating_add(1);
            record.peak_cpu = record.peak_cpu.max(cpu);
            record.last_cpu = cpu;
            record.currently_throttled = true;
            return;
        }

        self.group_behavior.push(GroupBehaviorRecord {
            group_key,
            name,
            times_throttled: 1,
            peak_cpu: cpu,
            last_cpu: cpu,
            throttle_seconds: 0,
            currently_throttled: true,
        });
    }

    pub fn record_throttle_stop(&mut self, group_key: u32, elapsed_secs: u64) {
        let Some(record) = self
            .group_behavior
            .iter_mut()
            .find(|record| record.group_key == group_key)
        else {
            return;
        };
        record.currently_throttled = false;
        record.throttle_seconds = record.throttle_seconds.saturating_add(elapsed_secs);
    }

    pub fn refresh_group_behavior_from_snapshot(&mut self) {
        for group in &self.groups {
            let throttled = self.is_group_throttled(group.group_key);
            let Some(record) = self
                .group_behavior
                .iter_mut()
                .find(|record| record.group_key == group.group_key)
            else {
                continue;
            };
            record.name.clone_from(&group.name);
            record.last_cpu = group.cpu_total;
            if throttled {
                record.peak_cpu = record.peak_cpu.max(group.cpu_total);
            }
        }
    }

    pub fn top_bad_actors(&self, limit: usize) -> Vec<&GroupBehaviorRecord> {
        let mut ranked: Vec<&GroupBehaviorRecord> = self.group_behavior.iter().collect();
        ranked.sort_by(|left, right| {
            right
                .times_throttled
                .cmp(&left.times_throttled)
                .then(
                    right
                        .peak_cpu
                        .partial_cmp(&left.peak_cpu)
                        .unwrap_or(std::cmp::Ordering::Equal),
                )
                .then(
                    right
                        .throttle_seconds
                        .cmp(&left.throttle_seconds),
                )
        });
        ranked.truncate(limit);
        ranked
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

    #[test]
    fn top_bad_actors_ranks_by_throttle_count_then_peak_cpu() {
        let mut state = AppState::new(40.0, 85.0, "signal", "tree", Vec::new());
        state.record_throttle_start(1, "chrome", 55.0);
        state.record_throttle_start(2, "firefox", 90.0);
        state.record_throttle_start(2, "firefox", 70.0);
        state.record_throttle_stop(1, 3);

        let top = state.top_bad_actors(2);
        assert_eq!(top.len(), 2);
        assert_eq!(top[0].group_key, 2);
        assert_eq!(top[0].times_throttled, 2);
        assert_eq!(top[1].group_key, 1);
    }
}
