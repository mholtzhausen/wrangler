use crate::app::{AppState, ProcessInfo};
use crate::throttle::{run_signal_governor, ThrottleBackend};
use std::collections::HashMap;
use tokio::sync::{mpsc, watch, Mutex};

pub enum HubCommand {
    ProcessSnapshot(Vec<ProcessInfo>),
    StartThrottle { pid: u32, cpu_usage: f32 },
    StopThrottle { pid: u32 },
    SetThreshold(f32),
    Quit,
}

pub enum ThrottleEvent {
    Started { pid: u32 },
    Stopped { pid: u32 },
    Error { pid: u32, message: String },
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

struct ActiveThrottle {
    cancel: tokio_util::sync::CancellationToken,
}

pub fn spawn_event_hub(initial_threshold: f32, backend: ThrottleBackend) -> HubHandles {
    let (command_tx, mut command_rx) = mpsc::channel::<HubCommand>(256);
    let (throttle_tx, mut throttle_rx) = mpsc::channel::<ThrottleEvent>(64);
    let (state_tx, state_rx) = watch::channel(AppState::new(initial_threshold));
    let (shutdown_tx, _) = watch::channel(false);
    let shutdown_notify = shutdown_tx.clone();

    let use_cgroups = backend.is_cgroup();
    let backend = std::sync::Arc::new(Mutex::new(backend));

    tokio::spawn(async move {
        let mut state = AppState::new(initial_threshold);
        let mut active: HashMap<u32, ActiveThrottle> = HashMap::new();

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
    active: &mut HashMap<u32, ActiveThrottle>,
    state_tx: &watch::Sender<AppState>,
    throttle_tx: &mpsc::Sender<ThrottleEvent>,
    backend: &std::sync::Arc<Mutex<ThrottleBackend>>,
    use_cgroups: bool,
) -> bool {
    match cmd {
        HubCommand::ProcessSnapshot(processes) => {
            state.processes = processes;
            broadcast(state_tx, state);
        }
        HubCommand::StartThrottle { pid, cpu_usage } => {
            if active.contains_key(&pid) {
                return false;
            }
            let target_pct = state.cpu_threshold;
            let cancel = tokio_util::sync::CancellationToken::new();
            let throttle_events = throttle_tx.clone();
            let backend_clone = backend.clone();
            let cancel_clone = cancel.clone();

            if use_cgroups {
                tokio::spawn(async move {
                    let mut guard = backend_clone.lock().await;
                    if let Err(e) = guard
                        .start_cgroup(pid, cpu_usage, target_pct, cancel_clone)
                        .await
                    {
                        let _ = throttle_events
                            .send(ThrottleEvent::Error { pid, message: e })
                            .await;
                    }
                });
            } else {
                tokio::spawn(async move {
                    run_signal_governor(pid, cpu_usage, target_pct, cancel_clone).await;
                });
            }

            active.insert(pid, ActiveThrottle { cancel });
            let _ = throttle_tx.send(ThrottleEvent::Started { pid }).await;
        }
        HubCommand::StopThrottle { pid } => {
            stop_pid(pid, active, throttle_tx, backend, use_cgroups).await;
        }
        HubCommand::SetThreshold(threshold) => {
            state.cpu_threshold = crate::config::clamp_threshold(threshold);
            let _ = crate::config::Config::update_threshold(state.cpu_threshold);
            broadcast(state_tx, state);
        }
        HubCommand::Quit => {
            state.quitting = true;
            broadcast(state_tx, state);
            let pids: Vec<u32> = active.keys().copied().collect();
            for pid in &pids {
                if let Some(entry) = active.remove(pid) {
                    entry.cancel.cancel();
                }
            }
            if use_cgroups {
                let mut guard = backend.lock().await;
                guard.stop_all().await;
            } else {
                for pid in pids {
                    crate::throttle::resume_signal(pid);
                }
            }
            return true;
        }
    }
    false
}

async fn stop_pid(
    pid: u32,
    active: &mut HashMap<u32, ActiveThrottle>,
    throttle_tx: &mpsc::Sender<ThrottleEvent>,
    backend: &std::sync::Arc<Mutex<ThrottleBackend>>,
    use_cgroups: bool,
) {
    if let Some(entry) = active.remove(&pid) {
        entry.cancel.cancel();
        if use_cgroups {
            let mut guard = backend.lock().await;
            guard.stop_pid(pid).await;
        } else {
            crate::throttle::resume_signal(pid);
        }
        let _ = throttle_tx.send(ThrottleEvent::Stopped { pid }).await;
    }
}

fn handle_throttle_event(
    event: ThrottleEvent,
    state: &mut AppState,
    active: &mut HashMap<u32, ActiveThrottle>,
    state_tx: &watch::Sender<AppState>,
) {
    match event {
        ThrottleEvent::Started { pid } => {
            state.throttled_pids.insert(pid);
            state.push_log(pid, "throttle started");
            broadcast(state_tx, state);
        }
        ThrottleEvent::Stopped { pid } => {
            state.throttled_pids.remove(&pid);
            state.push_log(pid, "throttle stopped");
            broadcast(state_tx, state);
        }
        ThrottleEvent::Error { pid, message } => {
            state.last_error = Some(format!("PID {pid}: {message}"));
            state.push_log(pid, format!("error: {message}"));
            active.remove(&pid);
            broadcast(state_tx, state);
        }
    }
}

fn broadcast(state_tx: &watch::Sender<AppState>, state: &AppState) {
    let _ = state_tx.send(state.clone());
}
