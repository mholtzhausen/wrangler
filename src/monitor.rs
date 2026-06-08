use crate::app::{identities_for_pids, AppGroupInfo, ProcessInfo};
use crate::event::HubCommand;
use crate::policy::{
    build_groups, effective_protected_apps, evaluate, group_by_key, GroupingMode,
    PolicySettings, RawProcess, ThrottleDecision,
};
use std::collections::{HashMap, HashSet};
use std::time::Duration;
use nix::unistd::{Group, Gid, User, Uid};
use sysinfo::{CpuRefreshKind, ProcessRefreshKind, System};
use tokio::sync::mpsc::Sender;
use tokio::sync::watch;
use tokio::time::sleep;

pub struct MonitorConfig {
    pub interval: Duration,
    pub self_pid: u32,
    pub protected_pids: HashSet<u32>,
    pub protected_apps: Vec<String>,
    pub grouping: GroupingMode,
    pub policy: PolicySettings,
}

pub async fn run_monitor(
    hub_tx: Sender<HubCommand>,
    mut policy_rx: watch::Receiver<PolicySettings>,
    config: MonitorConfig,
) {
    let mut sys = System::new_with_specifics(
        sysinfo::RefreshKind::new()
            .with_processes(ProcessRefreshKind::everything())
            .with_cpu(CpuRefreshKind::everything()),
    );

    let mut throttled_groups: HashSet<u32> = HashSet::new();
    let protected_apps = effective_protected_apps(&config.protected_apps);

    loop {
        sys.refresh_cpu();
        sys.refresh_processes();

        let policy = policy_rx.borrow_and_update().clone();
        let num_cores = sys.cpus().len().max(1);
        let global_cpu = global_cpu_usage(&sys);

        let mut processes: Vec<ProcessInfo> = Vec::new();
        let mut raw: Vec<RawProcess> = Vec::new();
        let mut seen_pids: HashSet<u32> = HashSet::new();
        let mut identity_cache: HashMap<(u32, u32), (String, String)> = HashMap::new();

        for (pid, process) in sys.processes() {
            let pid_u32 = pid.as_u32();
            if config.protected_pids.contains(&pid_u32) || pid_u32 == config.self_pid {
                continue;
            }

            seen_pids.insert(pid_u32);
            let parent_pid = process.parent().map(|p| p.as_u32());
            let cpu = process.cpu_usage();
            let name = process.name().to_string();
            let (user, group) = resolve_identity(
                process.user_id(),
                process.group_id(),
                &mut identity_cache,
            );

            raw.push(RawProcess {
                pid: pid_u32,
                parent_pid,
                name: name.clone(),
                user: user.clone(),
                group: group.clone(),
                cpu_usage: cpu,
            });
            processes.push(ProcessInfo {
                pid: pid_u32,
                parent_pid,
                name,
                user,
                group,
                cpu_usage: cpu,
            });
        }

        processes.sort_by(|a, b| {
            b.cpu_usage
                .partial_cmp(&a.cpu_usage)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        let groups = build_groups(
            &raw,
            &config.protected_pids,
            &protected_apps,
            policy.grouping,
        );
        let decision = evaluate(
            &groups,
            global_cpu,
            num_cores,
            &policy,
            &throttled_groups,
        );

        apply_decision(
            &hub_tx,
            &groups,
            &decision,
            num_cores,
            global_cpu,
            &mut throttled_groups,
        )
        .await;

        let exited_groups: Vec<u32> = throttled_groups
            .iter()
            .filter(|key| {
                group_by_key(&groups, **key)
                    .map(|group| group.pids.iter().all(|pid| !seen_pids.contains(pid)))
                    .unwrap_or(true)
            })
            .copied()
            .collect();
        for key in exited_groups {
            if let Some(group) = group_by_key(&groups, key) {
                let _ = hub_tx
                    .send(HubCommand::StopThrottleGroup {
                        group_key: key,
                        pids: group.pids.clone(),
                    })
                    .await;
            } else {
                let _ = hub_tx
                    .send(HubCommand::StopThrottleGroup {
                        group_key: key,
                        pids: Vec::new(),
                    })
                    .await;
            }
            throttled_groups.remove(&key);
        }

        let group_infos: Vec<AppGroupInfo> = groups
            .iter()
            .map(|group| {
                let (user, group_name) = identities_for_pids(&group.pids, &processes);
                AppGroupInfo {
                    group_key: group.key,
                    name: group.name.clone(),
                    user,
                    group: group_name,
                    pids: group.pids.clone(),
                    cpu_total: group.cpu_total,
                }
            })
            .collect();
        let _ = hub_tx
            .send(HubCommand::ProcessSnapshot {
                processes,
                groups: group_infos,
                global_cpu,
                num_cores,
            })
            .await;

        sleep(config.interval).await;
    }
}

async fn apply_decision(
    hub_tx: &Sender<HubCommand>,
    groups: &[crate::policy::AppGroup],
    decision: &ThrottleDecision,
    num_cores: usize,
    global_cpu: f32,
    throttled_groups: &mut HashSet<u32>,
) {
    for key in &decision.to_stop {
        if let Some(group) = group_by_key(groups, *key) {
            let _ = hub_tx
                .send(HubCommand::StopThrottleGroup {
                    group_key: *key,
                    pids: group.pids.clone(),
                })
                .await;
        } else {
            let _ = hub_tx
                .send(HubCommand::StopThrottleGroup {
                    group_key: *key,
                    pids: Vec::new(),
                })
                .await;
        }
        throttled_groups.remove(key);
    }

    for key in &decision.to_start {
        if let Some(group) = group_by_key(groups, *key) {
            let _ = hub_tx
                .send(HubCommand::StartThrottleGroup {
                    group_key: group.key,
                    name: group.name.clone(),
                    pids: group.pids.clone(),
                    cpu_total: group.cpu_total,
                    num_cores,
                    global_cpu,
                })
                .await;
            throttled_groups.insert(*key);
        }
    }

    for key in &decision.to_sync {
        if let Some(group) = group_by_key(groups, *key) {
            let _ = hub_tx
                .send(HubCommand::SyncThrottleGroup {
                    group_key: group.key,
                    pids: group.pids.clone(),
                    cpu_total: group.cpu_total,
                })
                .await;
        }
    }
}

fn resolve_identity(
    uid: Option<&sysinfo::Uid>,
    gid: Option<sysinfo::Gid>,
    cache: &mut HashMap<(u32, u32), (String, String)>,
) -> (String, String) {
    let uid_val = uid.map(|value| **value).unwrap_or(u32::MAX);
    let gid_val = gid.map(|value| *value).unwrap_or(u32::MAX);
    if let Some(hit) = cache.get(&(uid_val, gid_val)) {
        return hit.clone();
    }

    let user = if uid_val == u32::MAX {
        "?".to_string()
    } else if let Ok(Some(account)) = User::from_uid(Uid::from_raw(uid_val)) {
        account.name
    } else {
        uid_val.to_string()
    };

    let group = if gid_val == u32::MAX {
        "?".to_string()
    } else if let Ok(Some(account)) = Group::from_gid(Gid::from_raw(gid_val)) {
        account.name
    } else {
        gid_val.to_string()
    };

    cache.insert((uid_val, gid_val), (user.clone(), group.clone()));
    (user, group)
}

fn global_cpu_usage(sys: &System) -> f32 {
    let cpus = sys.cpus();
    if cpus.is_empty() {
        return 0.0;
    }
    cpus.iter().map(|cpu| cpu.cpu_usage()).sum::<f32>() / cpus.len() as f32
}

pub fn protected_pids() -> HashSet<u32> {
    let mut set = HashSet::new();
    set.insert(std::process::id());
    set.insert(nix::unistd::getppid().as_raw() as u32);
    set
}
