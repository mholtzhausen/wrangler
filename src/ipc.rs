use crate::app::AppState;
use crate::event::HubCommand;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;

const SOCKET_NAME: &str = "wrangler.sock";

pub fn socket_path() -> PathBuf {
    if let Ok(dir) = std::env::var("WRANGLER_RUNTIME_DIR") {
        return PathBuf::from(dir).join(SOCKET_NAME);
    }
    if let Ok(dir) = std::env::var("XDG_RUNTIME_DIR") {
        return PathBuf::from(dir).join(SOCKET_NAME);
    }
    if let Ok(home) = std::env::var("HOME") {
        return PathBuf::from(home)
            .join(".local")
            .join("run")
            .join(SOCKET_NAME);
    }
    PathBuf::from("/tmp").join(format!("wrangler-{}.sock", std::process::id()))
}

pub struct IpcServerGuard {
    path: PathBuf,
    _accept_task: JoinHandle<()>,
}

pub async fn spawn_server(
    hub_tx: mpsc::Sender<HubCommand>,
    state_rx: watch::Receiver<AppState>,
) -> Result<IpcServerGuard, Box<dyn std::error::Error>> {
    let path = socket_path();
    if path.exists() {
        std::fs::remove_file(&path)?;
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let listener = UnixListener::bind(&path)?;
    fix_socket_permissions(&path)?;
    tracing::info!(path = %path.display(), "IPC server listening");

    let accept_task = tokio::spawn(async move {
        loop {
            match listener.accept().await {
                Ok((stream, _)) => {
                    let hub_tx = hub_tx.clone();
                    let state_rx = state_rx.clone();
                    tokio::spawn(async move {
                        if let Err(e) = handle_client(stream, hub_tx, state_rx).await {
                            tracing::debug!(error = %e, "IPC client disconnected");
                        }
                    });
                }
                Err(e) => {
                    tracing::warn!(error = %e, "IPC accept failed");
                    break;
                }
            }
        }
    });

    Ok(IpcServerGuard {
        path,
        _accept_task: accept_task,
    })
}

impl Drop for IpcServerGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum ClientRequest {
    Ping,
    Subscribe,
    SetAppCap { value: f32 },
    #[serde(alias = "set_threshold")]
    SetThreshold { value: f32 },
    SetPressureThreshold { value: f32 },
    Detach,
    Quit,
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMessage {
    Ok,
    Error { message: String },
    State { data: Box<AppState> },
}

fn socket_owner_uid() -> Option<u32> {
    if let Ok(uid) = std::env::var("WRANGLER_SOCKET_UID") {
        if let Ok(parsed) = uid.parse() {
            return Some(parsed);
        }
    }
    let dir = std::env::var("XDG_RUNTIME_DIR")
        .or_else(|_| std::env::var("WRANGLER_RUNTIME_DIR"))
        .ok()?;
    let name = std::path::Path::new(&dir).file_name()?.to_str()?;
    name.parse().ok()
}

#[cfg(unix)]
fn fix_socket_permissions(path: &std::path::Path) -> std::io::Result<()> {
    use nix::unistd::{chown, geteuid, Gid, Uid, User};

    if !geteuid().is_root() {
        return Ok(());
    }

    let Some(uid) = socket_owner_uid() else {
        tracing::warn!("running as root but could not determine socket owner; clients may be unable to connect");
        return Ok(());
    };

    let gid = User::from_uid(Uid::from_raw(uid))
        .ok()
        .flatten()
        .map(|user| user.gid)
        .unwrap_or_else(|| Gid::from_raw(uid));

    chown(path, Some(Uid::from_raw(uid)), Some(gid)).map_err(|errno| {
        std::io::Error::from_raw_os_error(errno as i32)
    })?;

    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o660))?;
    Ok(())
}

#[cfg(not(unix))]
fn fix_socket_permissions(_path: &std::path::Path) -> std::io::Result<()> {
    Ok(())
}

pub async fn daemon_running() -> bool {
    ping().await.is_ok()
}

pub async fn ping() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let mut stream = UnixStream::connect(socket_path()).await?;
    let request = serde_json::to_string(&ClientRequest::Ping)? + "\n";
    stream.write_all(request.as_bytes()).await?;

    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    tokio::time::timeout(
        std::time::Duration::from_secs(1),
        reader.read_line(&mut line),
    )
    .await
    .map_err(|_| "daemon ping timed out")??;

    match serde_json::from_str::<ServerMessage>(line.trim())? {
        ServerMessage::Ok => Ok(()),
        ServerMessage::Error { message } => Err(message.into()),
        other => Err(format!("unexpected ping response: {other:?}").into()),
    }
}

async fn handle_client(
    stream: UnixStream,
    hub_tx: mpsc::Sender<HubCommand>,
    state_rx: watch::Receiver<AppState>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let (read_half, mut write_half) = stream.into_split();
    let (out_tx, mut out_rx) = mpsc::channel::<ServerMessage>(32);

    let writer = tokio::spawn(async move {
        while let Some(msg) = out_rx.recv().await {
            let line = match serde_json::to_string(&msg) {
                Ok(s) => s,
                Err(_) => continue,
            };
            if write_half.write_all(line.as_bytes()).await.is_err() {
                break;
            }
            if write_half.write_all(b"\n").await.is_err() {
                break;
            }
        }
    });

    let mut forward_task: Option<JoinHandle<()>> = None;
    let mut lines = BufReader::new(read_half).lines();

    while let Ok(Some(line)) = lines.next_line().await {
        let req: ClientRequest = serde_json::from_str(&line)?;
        match req {
            ClientRequest::Ping => {
                let _ = send_message(&out_tx, ServerMessage::Ok).await;
            }
            ClientRequest::Subscribe => {
                if forward_task.is_some() {
                    continue;
                }
                let snapshot = state_rx.borrow().clone();
                let _ = send_message(
                    &out_tx,
                    ServerMessage::State {
                        data: Box::new(snapshot),
                    },
                )
                .await;

                let mut rx = state_rx.clone();
                let tx = out_tx.clone();
                forward_task = Some(tokio::spawn(async move {
                    loop {
                        if rx.changed().await.is_err() {
                            break;
                        }
                        let state = rx.borrow().clone();
                        if send_message(
                            &tx,
                            ServerMessage::State {
                                data: Box::new(state),
                            },
                        )
                            .await
                            .is_err()
                        {
                            break;
                        }
                    }
                }));
            }
            ClientRequest::SetAppCap { value } | ClientRequest::SetThreshold { value } => {
                let num_cores = state_rx.borrow().num_cores;
                let clamped = crate::config::clamp_app_cap(value, num_cores);
                let _ = hub_tx.send(HubCommand::SetAppCap(clamped)).await;
                let _ = send_message(&out_tx, ServerMessage::Ok).await;
            }
            ClientRequest::SetPressureThreshold { value } => {
                let clamped = crate::config::clamp_pressure_threshold(value);
                let _ = hub_tx
                    .send(HubCommand::SetPressureThreshold(clamped))
                    .await;
                let _ = send_message(&out_tx, ServerMessage::Ok).await;
            }
            ClientRequest::Detach => break,
            ClientRequest::Quit => {
                let _ = hub_tx.send(HubCommand::Quit).await;
                let _ = send_message(&out_tx, ServerMessage::Ok).await;
            }
        }
    }

    if let Some(task) = forward_task {
        task.abort();
    }
    writer.abort();
    Ok(())
}

async fn send_message(tx: &mpsc::Sender<ServerMessage>, msg: ServerMessage) -> Result<(), ()> {
    tx.send(msg).await.map_err(|_| ())
}

pub struct AttachSession {
    request_tx: mpsc::Sender<ClientRequest>,
    state_tx: watch::Sender<AppState>,
    shutdown_tx: watch::Sender<bool>,
    _reader_task: JoinHandle<()>,
    _writer_task: JoinHandle<()>,
}

impl AttachSession {
    pub async fn connect() -> Result<Self, Box<dyn std::error::Error>> {
        let stream = UnixStream::connect(socket_path()).await?;
        let (read_half, mut write_half) = stream.into_split();
        let (request_tx, mut request_rx) = mpsc::channel::<ClientRequest>(16);
        let (state_tx, _state_rx) = watch::channel(AppState::new(
            80.0,
            crate::config::DEFAULT_PRESSURE_THRESHOLD,
            "signal",
            "tree",
            Vec::new(),
            crate::config::available_cpu_cores(),
        ));
        let (initial_tx, initial_rx) = tokio::sync::oneshot::channel();
        let (shutdown_tx, _) = watch::channel(false);

        let writer_task = tokio::spawn(async move {
            while let Some(req) = request_rx.recv().await {
                let line = match serde_json::to_string(&req) {
                    Ok(s) => s,
                    Err(_) => continue,
                };
                if write_half.write_all(line.as_bytes()).await.is_err() {
                    break;
                }
                if write_half.write_all(b"\n").await.is_err() {
                    break;
                }
            }
        });

        request_tx.send(ClientRequest::Subscribe).await?;

        let reader_state_tx = state_tx.clone();
        let reader_shutdown_tx = shutdown_tx.clone();
        let reader_task = tokio::spawn(async move {
            let mut lines = BufReader::new(read_half).lines();
            let mut initial_tx = Some(initial_tx);

            while let Ok(Some(line)) = lines.next_line().await {
                let msg: ServerMessage = match serde_json::from_str(&line) {
                    Ok(m) => m,
                    Err(_) => continue,
                };
                match msg {
                    ServerMessage::State { data } => {
                        let quitting = data.quitting;
                        let _ = reader_state_tx.send(*data);
                        if let Some(tx) = initial_tx.take() {
                            let _ = tx.send(());
                        }
                        if quitting {
                            let _ = reader_shutdown_tx.send(true);
                        }
                    }
                    ServerMessage::Ok => {}
                    ServerMessage::Error { message } => {
                        tracing::warn!(error = %message, "daemon IPC error");
                    }
                }
            }
            let _ = reader_shutdown_tx.send(true);
        });

        tokio::time::timeout(std::time::Duration::from_secs(2), initial_rx)
            .await
            .map_err(|_| "timed out waiting for daemon state")??;

        Ok(Self {
            request_tx,
            state_tx,
            shutdown_tx,
            _reader_task: reader_task,
            _writer_task: writer_task,
        })
    }

    pub fn state_rx(&self) -> watch::Receiver<AppState> {
        self.state_tx.subscribe()
    }

    pub fn shutdown_rx(&self) -> watch::Receiver<bool> {
        self.shutdown_tx.subscribe()
    }

    pub async fn set_app_cap(&self, value: f32) -> Result<(), Box<dyn std::error::Error>> {
        let num_cores = self.state_tx.borrow().num_cores;
        self.request_tx
            .send(ClientRequest::SetAppCap {
                value: crate::config::clamp_app_cap(value, num_cores),
            })
            .await?;
        Ok(())
    }

    pub async fn set_pressure_threshold(
        &self,
        value: f32,
    ) -> Result<(), Box<dyn std::error::Error>> {
        self.request_tx
            .send(ClientRequest::SetPressureThreshold {
                value: crate::config::clamp_pressure_threshold(value),
            })
            .await?;
        Ok(())
    }

    pub async fn detach(self) -> Result<(), Box<dyn std::error::Error>> {
        let _ = self.request_tx.send(ClientRequest::Detach).await;
        Ok(())
    }

    pub async fn quit_daemon(&self) -> Result<(), Box<dyn std::error::Error>> {
        self.request_tx.send(ClientRequest::Quit).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn socket_owner_uid_parses_runtime_dir() {
        std::env::set_var("XDG_RUNTIME_DIR", "/run/user/1000");
        assert_eq!(socket_owner_uid(), Some(1000));
        std::env::remove_var("XDG_RUNTIME_DIR");
    }

    #[test]
    fn socket_owner_uid_prefers_explicit_override() {
        std::env::set_var("XDG_RUNTIME_DIR", "/run/user/1000");
        std::env::set_var("WRANGLER_SOCKET_UID", "1001");
        assert_eq!(socket_owner_uid(), Some(1001));
        std::env::remove_var("XDG_RUNTIME_DIR");
        std::env::remove_var("WRANGLER_SOCKET_UID");
    }
}

#[cfg(test)]
mod integration_tests {
    use super::*;
    use crate::event::spawn_event_hub;
    use crate::throttle::ThrottleBackend;
    use std::fs;

    fn test_runtime_dir() -> PathBuf {
        std::env::temp_dir().join(format!(
            "wrangler-ipc-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn protocol_serde_roundtrip() {
        let req = ClientRequest::SetAppCap { value: 42.0 };
        let json = serde_json::to_string(&req).unwrap();
        let parsed: ClientRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, req);

        let msg = ServerMessage::Ok;
        let json = serde_json::to_string(&msg).unwrap();
        let parsed: ServerMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, msg);
    }

    #[tokio::test]
    async fn daemon_ping_and_subscribe() {
        let dir = test_runtime_dir();
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        std::env::set_var("WRANGLER_RUNTIME_DIR", &dir);

        let hub = spawn_event_hub(
            70.0,
            85.0,
            crate::policy::GroupingMode::Tree,
            Vec::new(),
            ThrottleBackend::Signal,
        );
        let _server = spawn_server(hub.command_tx.clone(), hub.state_rx.clone())
            .await
            .unwrap();

        ping().await.expect("ping should succeed");

        let session = AttachSession::connect()
            .await
            .expect("subscribe should succeed");
        assert_eq!(session.state_rx().borrow().app_cap, 70.0);
        session.detach().await.unwrap();

        std::env::remove_var("WRANGLER_RUNTIME_DIR");
        let _ = fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn attach_session_exits_when_daemon_quits() {
        let dir = test_runtime_dir();
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        std::env::set_var("WRANGLER_RUNTIME_DIR", &dir);

        let hub = spawn_event_hub(
            70.0,
            85.0,
            crate::policy::GroupingMode::Tree,
            Vec::new(),
            ThrottleBackend::Signal,
        );
        let _server = spawn_server(hub.command_tx.clone(), hub.state_rx.clone())
            .await
            .unwrap();

        let session = AttachSession::connect()
            .await
            .expect("subscribe should succeed");
        let mut shutdown_rx = session.shutdown_rx();

        hub.command_tx
            .send(crate::event::HubCommand::Quit)
            .await
            .unwrap();

        tokio::time::timeout(std::time::Duration::from_secs(2), shutdown_rx.changed())
            .await
            .expect("shutdown should be signaled")
            .expect("shutdown channel should stay open");
        assert!(*shutdown_rx.borrow_and_update());

        session.detach().await.unwrap();

        std::env::remove_var("WRANGLER_RUNTIME_DIR");
        let _ = fs::remove_dir_all(&dir);
    }
}
