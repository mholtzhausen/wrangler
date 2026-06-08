use crate::app::{AppState, ProcessInfo, ThrottledGroupInfo};
use crate::policy::machine_cpu_budget;
use crate::throttle::{run_signal_governor, ThrottleBackend};
use std::collections::HashMap;
use tokio::sync::{mpsc, watch, Mutex};

pub enum HubCommand {
    ProcessSnapshot {
        processes: Vec<ProcessInfo>,
        global_cpu: f32,
        num_cores: usize,
    },
    StartThrottleGroup {
        group_key: u32,
        name: String,
        pids: Vec<u32>,
        cpu_total: f32,
        num_cores: usize,
        global_cpu: f32,
    },
    SyncThrottleGroup {
        group_key: u32,
        pids: Vec<u32>,
        cpu_total: f32,
    },
    StopThrottleGroup {
        group_key: u32,
        pids: Vec<u32>,
    },
    SetAppCap(f32),
    Quit,
}

pub enum ThrottleEvent {
    Started { group_key: u32 },
    Synced { group_key: u32 },
    Stopped { group_key: u32 },
    Error { group_key: u32, message: String },
}

pub struct HubHandles {
    pub command_tx: mpsc::Sender<HubCommand>,
    pub state_rx: watch::Receiver<AppState>,
    shutdown: watch::Sender<bool>,
}

impl HubHandles {
    pub fn subscribe_shutdown(&self) -> watch::Receiver<bool> {
        self.shutdown.subscribe()
    }
}

struct ActiveGroupThrottle {
    cancel: tokio_util::sync::CancellationToken,
    pids: Vec<u32>,
}

pub fn spawn_event_hub(
    initial_app_cap: f32,
    initial_pressure_threshold: f32,
    backend: ThrottleBackend,
) -> HubHandles {
    let (command_tx, mut command_rx) = mpsc::channel::<HubCommand>(256);
    let (throttle_tx, mut throttle_rx) = mpsc::channel::<ThrottleEvent>(64);
    let backend_label = backend.mode_label().to_string();
    let (state_tx, state_rx) = watch::channel(AppState::new(
        initial_app_cap,
        initial_pressure_threshold,
        backend_label.clone(),
    ));
    let (shutdown_tx, _) = watch::channel(false);
    let shutdown_notify = shutdown_tx.clone();

    let use_cgroups = backend.is_cgroup();
    let backend = std::sync::Arc::new(Mutex::new(backend));

    tokio::spawn(async move {
        let mut state = AppState::new(
            initial_app_cap,
            initial_pressure_threshold,
            backend_label,
        );
        let mut active: HashMap<u32, ActiveGroupThrottle> = HashMap::new();

        loop {
            tokio::select! {
                Some(cmd) = command_rx.recv() => {
                    if handle_command(
                        cmd,
                        &mut state,
                        &mut active,
                        &state_tx,
                        &throttle_tx,
                        &backend,
                        use_cgroups,
                    ).await {
                        let _ = shutdown_notify.send(true);
                        break;
                    }
                }
                Some(event) = throttle_rx.recv() => {
                    handle_throttle_event(event, &mut state, &mut active, &state_tx);
                }
            }
        }
    });

    HubHandles {
        command_tx,
        state_rx,
        shutdown: shutdown_tx,
    }
}

async fn handle_command(
    cmd: HubCommand,
    state: &mut AppState,
    active: &mut HashMap<u32, ActiveGroupThrottle>,
    state_tx: &watch::Sender<AppState>,
    throttle_tx: &mpsc::Sender<ThrottleEvent>,
    backend: &std::sync::Arc<Mutex<ThrottleBackend>>,
    use_cgroups: bool,
) -> bool {
    match cmd {
        HubCommand::ProcessSnapshot {
            processes,
            global_cpu,
            num_cores,
        } => {
            state.processes = processes;
            state.global_cpu = global_cpu;
            state.num_cores = num_cores;
            broadcast(state_tx, state);
        }
        HubCommand::StartThrottleGroup {
            group_key,
            name,
            pids,
            cpu_total,
            num_cores,
            global_cpu: _,
        } => {
            if active.contains_key(&group_key) {
                return false;
            }

            let app_cap = state.app_cap;
            let machine_budget = machine_cpu_budget(app_cap, num_cores);
            let cancel = tokio_util::sync::CancellationToken::new();
            let throttle_events = throttle_tx.clone();
            let backend_clone = backend.clone();
            let cancel_clone = cancel.clone();
            let pids_for_task = pids.clone();

            if use_cgroups {
                tokio::spawn(async move {
                    let start_result = {
                        let mut guard = backend_clone.lock().await;
                        guard.start_group(group_key, &pids_for_task, app_cap, num_cores)
                    };
                    if let Err(e) = start_result {
                        let _ = throttle_events
                            .send(ThrottleEvent::Error {
                                group_key,
                                message: e,
                            })
                            .await;
                        return;
                    }

                    cancel_clone.cancelled().await;

                    let mut guard = backend_clone.lock().await;
                    guard.stop_group(group_key, &pids_for_task).await;
                });
            } else {
                for pid in &pids {
                    let pid = *pid;
                    let cancel_pid = cancel.clone();
                    let backend_clone = backend_clone.clone();
                    tokio::spawn(async move {
                        run_signal_governor(pid, cpu_total, machine_budget, cancel_pid).await;
                        let _ = backend_clone;
                    });
                }
            }

            active.insert(
                group_key,
                ActiveGroupThrottle {
                    cancel,
                    pids: pids.clone(),
                },
            );

            state.throttled_groups.push(ThrottledGroupInfo {
                group_key,
                name,
                pids,
                cpu_total,
            });
            state.sync_throttled_pids();
            let _ = throttle_tx
                .send(ThrottleEvent::Started { group_key })
                .await;
            broadcast(state_tx, state);
        }
        HubCommand::SyncThrottleGroup {
            group_key,
            pids,
            cpu_total,
        } => {
            let Some(entry) = active.get_mut(&group_key) else {
                return false;
            };

            if use_cgroups {
                let backend_clone = backend.clone();
                let pids_clone = pids.clone();
                let throttle_events = throttle_tx.clone();
                tokio::spawn(async move {
                    let result = {
                        let mut guard = backend_clone.lock().await;
                        guard.sync_group(group_key, &pids_clone)
                    };
                    if let Err(e) = result {
                        let _ = throttle_events
                            .send(ThrottleEvent::Error {
                                group_key,
                                message: e,
                            })
                            .await;
                    }
                });
            } else {
                let machine_budget =
                    machine_cpu_budget(state.app_cap, state.num_cores);
                for pid in pids
                    .iter()
                    .filter(|pid| !entry.pids.contains(pid))
                    .copied()
                {
                    let cancel = entry.cancel.clone();
                    tokio::spawn(async move {
                        run_signal_governor(pid, cpu_total, machine_budget, cancel).await;
                    });
                }
            }

            entry.pids = pids.clone();
            if let Some(group) = state
                .throttled_groups
                .iter_mut()
                .find(|group| group.group_key == group_key)
            {
                group.pids = pids;
                group.cpu_total = cpu_total;
            }
            state.sync_throttled_pids();
            let _ = throttle_tx
                .send(ThrottleEvent::Synced { group_key })
                .await;
            broadcast(state_tx, state);
        }
        HubCommand::StopThrottleGroup { group_key, pids } => {
            stop_group(
                group_key,
                &pids,
                active,
                throttle_tx,
                backend,
                use_cgroups,
            )
            .await;
            state
                .throttled_groups
                .retain(|group| group.group_key != group_key);
            state.sync_throttled_pids();
            broadcast(state_tx, state);
        }
        HubCommand::SetAppCap(app_cap) => {
            state.app_cap = crate::config::clamp_app_cap(app_cap);
            let _ = crate::config::Config::update_app_cap(state.app_cap);
            if use_cgroups {
                let mut guard = backend.lock().await;
                if let Err(e) = guard.update_all_caps(state.app_cap, state.num_cores) {
                    state.last_error = Some(format!("update cgroup caps: {e}"));
                }
            }
            broadcast(state_tx, state);
        }
        HubCommand::Quit => {
            state.quitting = true;
            broadcast(state_tx, state);
            let groups: Vec<(u32, Vec<u32>)> = active
                .iter()
                .map(|(key, entry)| (*key, entry.pids.clone()))
                .collect();
            for (group_key, pids) in groups {
                if let Some(entry) = active.remove(&group_key) {
                    entry.cancel.cancel();
                }
                let mut guard = backend.lock().await;
                guard.stop_group(group_key, &pids).await;
            }
            if use_cgroups {
                let mut guard = backend.lock().await;
                guard.stop_all().await;
            }
            return true;
        }
    }
    false
}

async fn stop_group(
    group_key: u32,
    pids: &[u32],
    active: &mut HashMap<u32, ActiveGroupThrottle>,
    throttle_tx: &mpsc::Sender<ThrottleEvent>,
    backend: &std::sync::Arc<Mutex<ThrottleBackend>>,
    use_cgroups: bool,
) {
    if let Some(entry) = active.remove(&group_key) {
        entry.cancel.cancel();
        let stop_pids = if pids.is_empty() {
            entry.pids
        } else {
            pids.to_vec()
        };
        let mut guard = backend.lock().await;
        guard.stop_group(group_key, &stop_pids).await;
        if !use_cgroups {
            drop(guard);
        }
        let _ = throttle_tx
            .send(ThrottleEvent::Stopped { group_key })
            .await;
    }
}

fn handle_throttle_event(
    event: ThrottleEvent,
    state: &mut AppState,
    active: &mut HashMap<u32, ActiveGroupThrottle>,
    state_tx: &watch::Sender<AppState>,
) {
    match event {
        ThrottleEvent::Started { group_key } => {
            state.push_log(group_key, "group throttle started");
            broadcast(state_tx, state);
        }
        ThrottleEvent::Synced { group_key } => {
            state.push_log(group_key, "group membership synced");
            broadcast(state_tx, state);
        }
        ThrottleEvent::Stopped { group_key } => {
            state.push_log(group_key, "group throttle stopped");
            broadcast(state_tx, state);
        }
        ThrottleEvent::Error { group_key, message } => {
            state.last_error = Some(format!("group {group_key}: {message}"));
            state.push_log(group_key, format!("error: {message}"));
            active.remove(&group_key);
            state
                .throttled_groups
                .retain(|group| group.group_key != group_key);
            state.sync_throttled_pids();
            broadcast(state_tx, state);
        }
    }
}

fn broadcast(state_tx: &watch::Sender<AppState>, state: &AppState) {
    let _ = state_tx.send(state.clone());
}
