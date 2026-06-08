use std::path::PathBuf;
use std::process::{Command, Stdio};

/// Launch the interactive TUI in a new terminal window.
pub fn open_dashboard() {
    let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("wrangler"));

    let terminals: [(&str, &[&str]); 5] = [
        ("alacritty", &["-e"]),
        ("gnome-terminal", &["--"]),
        ("konsole", &["-e"]),
        ("xfce4-terminal", &["-e"]),
        ("xterm", &["-e"]),
    ];

    for (terminal, prefix_args) in terminals {
        if !terminal_available(terminal) {
            continue;
        }

        let mut cmd = Command::new(terminal);
        cmd.args(prefix_args)
            .arg(&exe)
            .arg("--attach")
            .stdin(Stdio::null());

        match cmd.spawn() {
            Ok(_) => {
                tracing::info!(terminal, "opened wrangler dashboard");
                return;
            }
            Err(e) => {
                tracing::warn!(terminal, error = %e, "failed to launch dashboard");
            }
        }
    }

    tracing::error!("no terminal emulator available; run `wrangler` directly in a terminal");
}

fn terminal_available(name: &str) -> bool {
    Command::new("which")
        .arg(name)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}
