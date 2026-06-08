use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

pub const BUILTIN_PROTECTED_APPS: &[&str] = &[
    "wrangler",
    "sshd",
    "systemd",
    "init",
    "pipewire",
    "wireplumber",
    "Xorg",
    "Xwayland",
    "gnome-shell",
    "kwin_wayland",
    "sway",
    "mutter",
    "waybar",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GroupingMode {
    #[default]
    Tree,
    Name,
}

impl GroupingMode {
    pub fn as_str(self) -> &'static str {
        match self {
            GroupingMode::Tree => "tree",
            GroupingMode::Name => "name",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct PolicySettings {
    pub app_cap: f32,
    pub pressure_threshold: f32,
    pub top_offenders: usize,
    pub grouping: GroupingMode,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RawProcess {
    pub pid: u32,
    pub parent_pid: Option<u32>,
    pub name: String,
    pub user: String,
    pub group: String,
    pub cpu_usage: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AppGroup {
    pub key: u32,
    pub name: String,
    pub pids: Vec<u32>,
    pub cpu_total: f32,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ThrottleDecision {
    pub to_start: Vec<u32>,
    pub to_stop: Vec<u32>,
    pub to_sync: Vec<u32>,
}

pub fn effective_protected_apps(configured: &[String]) -> Vec<String> {
    let mut names: Vec<String> = BUILTIN_PROTECTED_APPS
        .iter()
        .map(|name| (*name).to_string())
        .collect();
    for app in configured {
        if !names.iter().any(|n| n.eq_ignore_ascii_case(app)) {
            names.push(app.clone());
        }
    }
    names
}

pub fn is_protected_name(name: &str, protected_apps: &[String]) -> bool {
    protected_apps
        .iter()
        .any(|app| app.eq_ignore_ascii_case(name))
}

/// Machine-wide CPU budget in htop units (100% = one core).
pub fn machine_cpu_budget(app_cap: f32, num_cores: usize) -> f32 {
    (app_cap / 100.0) * num_cores as f32 * 100.0
}

pub fn system_under_pressure(global_cpu: f32, pressure_threshold: f32) -> bool {
    pressure_threshold <= 0.0 || global_cpu >= pressure_threshold
}

pub fn resolve_tree_root(
    pid: u32,
    parent_map: &HashMap<u32, Option<u32>>,
    protected: &HashSet<u32>,
) -> u32 {
    let mut current = pid;
    let mut visited = HashSet::new();

    loop {
        if !visited.insert(current) {
            return current;
        }
        if protected.contains(&current) {
            return current;
        }
        let Some(parent) = parent_map.get(&current).copied().flatten() else {
            return current;
        };
        if parent <= 1 || parent == current {
            return current;
        }
        current = parent;
    }
}

pub fn stable_name_key(name: &str) -> u32 {
    let mut hash: u32 = 5381;
    for byte in name.to_ascii_lowercase().bytes() {
        hash = hash.wrapping_mul(33).wrapping_add(u32::from(byte));
    }
    hash
}

pub fn build_groups(
    processes: &[RawProcess],
    protected_pids: &HashSet<u32>,
    protected_apps: &[String],
    mode: GroupingMode,
) -> Vec<AppGroup> {
    match mode {
        GroupingMode::Tree => build_groups_by_tree(processes, protected_pids, protected_apps),
        GroupingMode::Name => build_groups_by_name(processes, protected_pids, protected_apps),
    }
}

fn build_groups_by_tree(
    processes: &[RawProcess],
    protected_pids: &HashSet<u32>,
    protected_apps: &[String],
) -> Vec<AppGroup> {
    let parent_map: HashMap<u32, Option<u32>> = processes
        .iter()
        .map(|p| (p.pid, p.parent_pid))
        .collect();

    let mut grouped: HashMap<u32, AppGroup> = HashMap::new();

    for process in processes {
        if protected_pids.contains(&process.pid) || is_protected_name(&process.name, protected_apps)
        {
            continue;
        }

        let key = resolve_tree_root(process.pid, &parent_map, protected_pids);
        let entry = grouped.entry(key).or_insert_with(|| AppGroup {
            key,
            name: process.name.clone(),
            pids: Vec::new(),
            cpu_total: 0.0,
        });
        if entry.pids.is_empty() || key == process.pid {
            entry.name = process.name.clone();
        }
        entry.pids.push(process.pid);
        entry.cpu_total += process.cpu_usage;
    }

    finalize_groups(grouped)
}

fn build_groups_by_name(
    processes: &[RawProcess],
    protected_pids: &HashSet<u32>,
    protected_apps: &[String],
) -> Vec<AppGroup> {
    let mut grouped: HashMap<u32, AppGroup> = HashMap::new();

    for process in processes {
        if protected_pids.contains(&process.pid) || is_protected_name(&process.name, protected_apps)
        {
            continue;
        }

        let key = stable_name_key(&process.name);
        let entry = grouped.entry(key).or_insert_with(|| AppGroup {
            key,
            name: process.name.clone(),
            pids: Vec::new(),
            cpu_total: 0.0,
        });
        entry.pids.push(process.pid);
        entry.cpu_total += process.cpu_usage;
    }

    finalize_groups(grouped)
}

fn finalize_groups(grouped: HashMap<u32, AppGroup>) -> Vec<AppGroup> {
    let mut groups: Vec<AppGroup> = grouped.into_values().collect();
    for group in &mut groups {
        group.pids.sort_unstable();
    }
    groups.sort_by(|a, b| {
        b.cpu_total
            .partial_cmp(&a.cpu_total)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    groups
}

pub fn evaluate(
    groups: &[AppGroup],
    global_cpu: f32,
    num_cores: usize,
    settings: &PolicySettings,
    currently_throttled: &HashSet<u32>,
) -> ThrottleDecision {
    let mut decision = ThrottleDecision::default();

    if !system_under_pressure(global_cpu, settings.pressure_threshold) {
        decision.to_stop = currently_throttled.iter().copied().collect();
        return decision;
    }

    let budget = machine_cpu_budget(settings.app_cap, num_cores);
    let top_n = settings.top_offenders.max(1);

    let mut candidates: Vec<u32> = groups
        .iter()
        .filter(|g| g.cpu_total > budget)
        .take(top_n)
        .map(|g| g.key)
        .collect();

    candidates.sort_unstable();

    for key in currently_throttled {
        if !candidates.contains(key) {
            decision.to_stop.push(*key);
        }
    }

    for key in &candidates {
        if currently_throttled.contains(key) {
            decision.to_sync.push(*key);
        } else {
            decision.to_start.push(*key);
        }
    }

    decision
}

pub fn group_by_key(groups: &[AppGroup], key: u32) -> Option<&AppGroup> {
    groups.iter().find(|g| g.key == key)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn proc(pid: u32, parent: Option<u32>, name: &str, cpu: f32) -> RawProcess {
        RawProcess {
            pid,
            parent_pid: parent,
            name: name.to_string(),
            user: "user".to_string(),
            group: "user".to_string(),
            cpu_usage: cpu,
        }
    }

    #[test]
    fn machine_budget_scales_with_cores() {
        assert_eq!(machine_cpu_budget(40.0, 8), 320.0);
        assert_eq!(machine_cpu_budget(100.0, 1), 100.0);
    }

    #[test]
    fn pressure_gate_blocks_when_system_idle() {
        let settings = PolicySettings {
            app_cap: 40.0,
            pressure_threshold: 85.0,
            top_offenders: 1,
            grouping: GroupingMode::Tree,
        };
        let groups = vec![AppGroup {
            key: 10,
            name: "hog".into(),
            pids: vec![10],
            cpu_total: 500.0,
        }];
        let decision = evaluate(&groups, 20.0, 8, &settings, &HashSet::new());
        assert!(decision.to_start.is_empty());

        let active = HashSet::from([10]);
        let decision = evaluate(&groups, 20.0, 8, &settings, &active);
        assert_eq!(decision.to_stop, vec![10]);
    }

    #[test]
    fn groups_sum_process_tree_cpu() {
        let processes = vec![
            proc(100, Some(1), "chrome", 50.0),
            proc(101, Some(100), "chrome", 80.0),
            proc(102, Some(100), "chrome", 70.0),
        ];
        let groups = build_groups(
            &processes,
            &HashSet::new(),
            &[],
            GroupingMode::Tree,
        );
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].key, 100);
        assert_eq!(groups[0].cpu_total, 200.0);
        assert_eq!(groups[0].pids, vec![100, 101, 102]);
    }

    #[test]
    fn name_grouping_merges_same_executable() {
        let processes = vec![
            proc(100, Some(1), "node", 40.0),
            proc(200, Some(1), "node", 60.0),
        ];
        let groups = build_groups(
            &processes,
            &HashSet::new(),
            &[],
            GroupingMode::Name,
        );
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].name, "node");
        assert_eq!(groups[0].cpu_total, 100.0);
        assert_eq!(groups[0].pids, vec![100, 200]);
    }

    #[test]
    fn protected_apps_are_excluded_from_groups() {
        let processes = vec![
            proc(100, Some(1), "sshd", 90.0),
            proc(200, Some(1), "hog", 90.0),
        ];
        let protected = effective_protected_apps(&[]);
        let groups = build_groups(
            &processes,
            &HashSet::new(),
            &protected,
            GroupingMode::Name,
        );
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].name, "hog");
    }

    #[test]
    fn throttles_top_offender_over_budget_when_pressured() {
        let settings = PolicySettings {
            app_cap: 40.0,
            pressure_threshold: 85.0,
            top_offenders: 1,
            grouping: GroupingMode::Tree,
        };
        let groups = vec![
            AppGroup {
                key: 1,
                name: "big".into(),
                pids: vec![1, 2],
                cpu_total: 500.0,
            },
            AppGroup {
                key: 3,
                name: "small".into(),
                pids: vec![3],
                cpu_total: 400.0,
            },
        ];
        let decision = evaluate(&groups, 90.0, 8, &settings, &HashSet::new());
        assert_eq!(decision.to_start, vec![1]);
        assert!(decision.to_stop.is_empty());
    }

    #[test]
    fn syncs_active_group_membership() {
        let settings = PolicySettings {
            app_cap: 40.0,
            pressure_threshold: 85.0,
            top_offenders: 1,
            grouping: GroupingMode::Tree,
        };
        let groups = vec![AppGroup {
            key: 10,
            name: "hog".into(),
            pids: vec![10, 11],
            cpu_total: 500.0,
        }];
        let active = HashSet::from([10]);
        let decision = evaluate(&groups, 90.0, 8, &settings, &active);
        assert!(decision.to_start.is_empty());
        assert_eq!(decision.to_sync, vec![10]);
    }
}
