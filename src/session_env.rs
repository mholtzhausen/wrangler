//! Desktop session environment for privileged launches (e.g. `sudo wrangler --tray`).
//!
//! StatusNotifierItem tray icons and the daemon Unix socket live in the invoking
//! user's session (`/run/user/<uid>`). `sudo` runs the process as root with a
//! different (or missing) session bus, so we reattach to the sudo caller's bus.

#[cfg(target_os = "linux")]
use std::process::Command;

/// Reattach to the invoking user's desktop session when launched via `sudo`.
///
/// Must run on the main thread before spawning worker threads or tray/IPC setup.
#[cfg(target_os = "linux")]
pub fn restore_invoking_user_session() -> bool {
    use nix::unistd::{geteuid, Uid, User};

    if !geteuid().is_root() {
        return false;
    }

    let Ok(uid_str) = std::env::var("SUDO_UID") else {
        tracing::warn!(
            "running as root without SUDO_UID; system tray and daemon socket may not \
             appear in your desktop session (launch with sudo from a logged-in user)"
        );
        return false;
    };

    let Ok(uid) = uid_str.parse::<u32>() else {
        tracing::warn!(uid = %uid_str, "invalid SUDO_UID; skipping session restore");
        return false;
    };

    let runtime = format!("/run/user/{uid}");
    if !std::path::Path::new(&runtime).is_dir() {
        tracing::warn!(
            runtime = %runtime,
            "user runtime directory missing; system tray likely unavailable"
        );
        return false;
    }

    // SAFETY: called once at startup before other threads mutate the environment.
    unsafe {
        std::env::set_var("XDG_RUNTIME_DIR", &runtime);
        std::env::set_var("DBUS_SESSION_BUS_ADDRESS", format!("unix:path={runtime}/bus"));
    }

    if let Ok(Some(user)) = User::from_uid(Uid::from_raw(uid)) {
        unsafe {
            std::env::set_var("HOME", user.dir);
        }
        restore_display_from_user(&user.name);
    } else if let Ok(name) = std::env::var("SUDO_USER") {
        restore_display_from_user(&name);
    }

    tracing::info!(
        runtime = %runtime,
        "restored invoking user session for tray and IPC"
    );
    true
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
pub fn restore_invoking_user_session() -> bool {
    false
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
}
