//! Desktop session environment for privileged launches (e.g. `sudo wrangler --tray`).
//!
//! StatusNotifierItem tray icons and the daemon Unix socket live in the invoking
//! user's session (`/run/user/<uid>`). Root cannot host a tray icon on the user
//! session bus, so privileged daemons spawn a `--tray-client` process as the
//! desktop user.

#[cfg(target_os = "linux")]
use std::path::PathBuf;
#[cfg(target_os = "linux")]
use std::process::{Command, Stdio};

/// Desktop session metadata for the invoking (non-root) user.
#[cfg(target_os = "linux")]
#[derive(Debug, Clone)]
pub struct DesktopSession {
    pub uid: u32,
    pub username: String,
    pub home: PathBuf,
    pub xdg_runtime_dir: String,
    pub dbus_address: String,
    pub display: Option<String>,
    pub wayland_display: Option<String>,
}

/// Re-exec wrangler under sudo, preserving the desktop environment.
#[cfg(target_os = "linux")]
pub fn reexec_with_sudo() -> Result<(), Box<dyn std::error::Error>> {
    use nix::unistd::geteuid;

    if geteuid().is_root() {
        return Ok(());
    }

    let exe = std::env::current_exe()?;
    let args: Vec<String> = std::env::args().skip(1).collect();

    let status = Command::new("sudo")
        .arg("-E")
        .arg(&exe)
        .args(&args)
        .status()?;

    std::process::exit(status.code().unwrap_or(1));
}

/// Reattach to the invoking user's desktop session when launched via `sudo`.
///
/// Must run on the main thread before spawning worker threads or IPC setup.
#[cfg(target_os = "linux")]
pub fn restore_invoking_user_session() -> bool {
    use nix::unistd::geteuid;

    if !geteuid().is_root() {
        return false;
    }

    let Some(session) = resolve_desktop_session() else {
        tracing::warn!(
            "running as root without a desktop user session; IPC socket may be unreachable \
             (launch with sudo from a logged-in user, or use wrangler --tray --sudo)"
        );
        return false;
    };

    apply_desktop_session(&session);
    tracing::info!(
        runtime = %session.xdg_runtime_dir,
        user = %session.username,
        "restored invoking user session for IPC"
    );
    true
}

/// Spawn a tray-client process as the desktop user (required when the daemon is root).
#[cfg(target_os = "linux")]
pub fn spawn_tray_client_as_user() -> Result<std::process::Child, Box<dyn std::error::Error>> {
    let session = resolve_desktop_session()
        .ok_or("could not resolve desktop user session for tray client")?;
    let exe = std::env::current_exe()?;

    let mut cmd = Command::new("runuser");
    cmd.args(["-u", &session.username, "--", "env"]);
    cmd.arg(format!("HOME={}", session.home.display()));
    cmd.arg(format!("XDG_RUNTIME_DIR={}", session.xdg_runtime_dir));
    cmd.arg(format!("DBUS_SESSION_BUS_ADDRESS={}", session.dbus_address));
    if let Some(display) = &session.display {
        cmd.arg(format!("DISPLAY={display}"));
    }
    if let Some(wayland) = &session.wayland_display {
        cmd.arg(format!("WAYLAND_DISPLAY={wayland}"));
    }
    cmd.arg(exe);
    cmd.args(["--tray-client", "--foreground"]);
    cmd.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    let child = cmd.spawn()?;
    tracing::info!(
        user = %session.username,
        pid = child.id(),
        "spawned tray client in desktop session"
    );
    Ok(child)
}

#[cfg(target_os = "linux")]
pub fn resolve_desktop_session() -> Option<DesktopSession> {
    use nix::unistd::{geteuid, getuid, User, Uid};

    let (uid, username) = if let Ok(uid_str) = std::env::var("SUDO_UID") {
        let uid: u32 = uid_str.parse().ok()?;
        let name = std::env::var("SUDO_USER").unwrap_or_else(|_| uid.to_string());
        (uid, name)
    } else if geteuid().is_root() {
        return None;
    } else {
        let uid = getuid().as_raw();
        let name = User::from_uid(getuid())
            .ok()
            .flatten()
            .map(|user| user.name)
            .unwrap_or_else(|| uid.to_string());
        (uid, name)
    };

    let xdg_runtime_dir = format!("/run/user/{uid}");
    if !std::path::Path::new(&xdg_runtime_dir).is_dir() {
        return None;
    }

    let home = User::from_uid(Uid::from_raw(uid))
        .ok()
        .flatten()
        .map(|user| user.dir)
        .unwrap_or_else(|| PathBuf::from(format!("/home/{username}")));

    let dbus_address = format!("unix:path={xdg_runtime_dir}/bus");
    let (display, wayland_display) = read_display_env(&username);

    Some(DesktopSession {
        uid,
        username,
        home,
        xdg_runtime_dir,
        dbus_address,
        display,
        wayland_display,
    })
}

#[cfg(target_os = "linux")]
fn apply_desktop_session(session: &DesktopSession) {
    // SAFETY: called once at startup before other threads mutate the environment.
    unsafe {
        std::env::set_var("XDG_RUNTIME_DIR", &session.xdg_runtime_dir);
        std::env::set_var("DBUS_SESSION_BUS_ADDRESS", &session.dbus_address);
        std::env::set_var("WRANGLER_SOCKET_UID", session.uid.to_string());
        std::env::set_var("HOME", &session.home);
    }

    if session.display.is_none() && session.wayland_display.is_none() {
        restore_display_from_user(&session.username);
    } else {
        if let Some(display) = &session.display {
            unsafe {
                std::env::set_var("DISPLAY", display);
            }
        }
        if let Some(wayland) = &session.wayland_display {
            unsafe {
                std::env::set_var("WAYLAND_DISPLAY", wayland);
            }
        }
    }
}

#[cfg(target_os = "linux")]
fn read_display_env(username: &str) -> (Option<String>, Option<String>) {
    let Some(output) = Command::new("runuser")
        .args(["-u", username, "--", "printenv"])
        .output()
        .ok()
        .filter(|out| out.status.success())
    else {
        return (None, None);
    };

    let mut display = None;
    let mut wayland = None;
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        match key {
            "DISPLAY" => display = Some(value.to_string()),
            "WAYLAND_DISPLAY" => wayland = Some(value.to_string()),
            _ => {}
        }
    }
    (display, wayland)
}

#[cfg(target_os = "linux")]
fn restore_display_from_user(username: &str) {
    if std::env::var("DISPLAY").is_ok() || std::env::var("WAYLAND_DISPLAY").is_ok() {
        return;
    }

    let Ok(output) = Command::new("runuser")
        .args(["-u", username, "--", "printenv"])
        .output()
    else {
        return;
    };

    if !output.status.success() {
        return;
    }

    for line in String::from_utf8_lossy(&output.stdout).lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if matches!(
            key,
            "DISPLAY" | "WAYLAND_DISPLAY" | "XDG_CURRENT_DESKTOP" | "DESKTOP_SESSION"
        ) {
            // SAFETY: startup-only, single-threaded.
            unsafe {
                std::env::set_var(key, value);
            }
        }
    }
}

#[cfg(not(target_os = "linux"))]
pub fn reexec_with_sudo() -> Result<(), Box<dyn std::error::Error>> {
    Err("sudo mode is only supported on Linux".into())
}

#[cfg(not(target_os = "linux"))]
pub fn restore_invoking_user_session() -> bool {
    false
}

#[cfg(not(target_os = "linux"))]
pub fn spawn_tray_client_as_user() -> Result<std::process::Child, Box<dyn std::error::Error>> {
    Err("tray client spawn is only supported on Linux".into())
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    #[test]
    fn restore_is_noop_without_root() {
        if !nix::unistd::geteuid().is_root() {
            assert!(!restore_invoking_user_session());
        }
    }

    #[test]
    fn resolve_desktop_session_for_current_user() {
        if nix::unistd::geteuid().is_root() {
            return;
        }
        let session = resolve_desktop_session().expect("desktop session");
        assert_eq!(session.uid, nix::unistd::getuid().as_raw());
        assert!(session.xdg_runtime_dir.starts_with("/run/user/"));
    }
}
