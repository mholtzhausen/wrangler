use crate::binary_install::{
    copy_binary, ensure_privileged, privileged_command, remove_path, use_sudo, SYSTEM_BIN_PATH,
};
use crate::cli::{ServiceCommand, ServiceOptions};
use nix::unistd::{geteuid, getuid, User};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

pub const SERVICE_NAME: &str = "wrangler.service";
pub const TRAY_SERVICE_NAME: &str = "wrangler-tray.service";
pub const SYSTEM_UNIT_PATH: &str = "/etc/systemd/system/wrangler.service";

pub fn run(command: &ServiceCommand) -> Result<(), Box<dyn std::error::Error>> {
    match command {
        ServiceCommand::Status(opts) => status(opts),
        ServiceCommand::Install(opts) => install(opts),
        ServiceCommand::Uninstall(opts) => uninstall(opts),
    }
}

fn status(opts: &ServiceOptions) -> Result<(), Box<dyn std::error::Error>> {
    if opts.sudo {
        let output = run_systemctl(&["status", SERVICE_NAME, "--no-pager"], true)?;
        print_systemctl_output(&output)?;
        if let Ok(session) = resolve_target_user_session() {
            println!("---");
            let tray = systemctl_user_for(&session, &["status", TRAY_SERVICE_NAME, "--no-pager"], true);
            if let Ok(tray_output) = tray {
                print_systemctl_output(&tray_output)?;
            }
        }
        if output.status.success() {
            Ok(())
        } else {
            Err(format!(
                "systemctl status exited with {}",
                output.status.code().unwrap_or(-1)
            )
            .into())
        }
    } else {
        let output = run_systemctl_user(&["status", SERVICE_NAME, "--no-pager"])?;
        print_systemctl_output(&output)?;
        if output.status.success() {
            Ok(())
        } else {
            Err(format!(
                "systemctl status exited with {}",
                output.status.code().unwrap_or(-1)
            )
            .into())
        }
    }
}

fn install(opts: &ServiceOptions) -> Result<(), Box<dyn std::error::Error>> {
    if opts.sudo {
        install_system(opts)
    } else {
        install_user(opts)
    }
}

fn uninstall(opts: &ServiceOptions) -> Result<(), Box<dyn std::error::Error>> {
    if opts.sudo {
        uninstall_system(opts)
    } else {
        uninstall_user(opts)
    }
}

fn install_user(_opts: &ServiceOptions) -> Result<(), Box<dyn std::error::Error>> {
    let exe = std::env::current_exe()?;
    let bin_dir = user_local_bin_dir()?;
    let unit_dir = user_systemd_dir()?;
    let bin_path = bin_dir.join("wrangler");
    let unit_path = unit_dir.join(SERVICE_NAME);

    std::fs::create_dir_all(&bin_dir)?;
    std::fs::create_dir_all(&unit_dir)?;
    copy_binary_local(&exe, &bin_path)?;
    std::fs::write(&unit_path, render_user_unit(&bin_path))?;
    run_systemctl_user_or_fail(&["daemon-reload"])?;
    run_systemctl_user_or_fail(&["enable", "--now", SERVICE_NAME])?;

    println!("Installed user service at {}", unit_path.display());
    println!("Binary installed to {}", bin_path.display());
    println!("Tray-enabled daemon started in your desktop session.");
    println!("Check with: wrangler service status");
    Ok(())
}

fn uninstall_user(_opts: &ServiceOptions) -> Result<(), Box<dyn std::error::Error>> {
    let bin_path = user_local_bin_dir()?.join("wrangler");
    let unit_path = user_systemd_dir()?.join(SERVICE_NAME);

    let _ = run_systemctl_user(&["disable", "--now", SERVICE_NAME]);
    let _ = std::fs::remove_file(&unit_path);
    let _ = std::fs::remove_file(&bin_path);
    run_systemctl_user_or_fail(&["daemon-reload"])?;
    let _ = run_systemctl_user(&["reset-failed"]);

    println!("Removed user service {SERVICE_NAME}");
    Ok(())
}

fn install_system(opts: &ServiceOptions) -> Result<(), Box<dyn std::error::Error>> {
    ensure_privileged(opts.sudo, "install")?;

    let exe = std::env::current_exe()?;
    let session = resolve_target_user_session()?;
    let unit = render_system_unit(Path::new(SYSTEM_BIN_PATH), &session);

    copy_binary(&exe, Path::new(SYSTEM_BIN_PATH), opts.sudo)?;
    write_system_unit(&unit, opts.sudo)?;
    install_tray_user_service(&session, Path::new(SYSTEM_BIN_PATH), opts.sudo)?;
    run_systemctl_or_fail(&["daemon-reload"], opts.sudo)?;
    run_systemctl_or_fail(&["enable", "--now", SERVICE_NAME], opts.sudo)?;
    enable_tray_user_service(&session, opts.sudo)?;

    println!("Installed system service at {SYSTEM_UNIT_PATH}");
    println!("Binary installed to {SYSTEM_BIN_PATH}");
    println!(
        "Tray client installed for user {} at ~/.config/systemd/user/{TRAY_SERVICE_NAME}",
        session.username
    );
    println!("Services enabled and started. Check with: wrangler service status --sudo");
    Ok(())
}

fn uninstall_system(opts: &ServiceOptions) -> Result<(), Box<dyn std::error::Error>> {
    ensure_privileged(opts.sudo, "uninstall")?;

    if let Ok(session) = resolve_target_user_session() {
        let _ = disable_tray_user_service(&session, opts.sudo);
        let tray_unit = session
            .home
            .join(".config/systemd/user")
            .join(TRAY_SERVICE_NAME);
        let _ = remove_path_owned(&tray_unit, &session.username, opts.sudo);
    }

    let _ = run_systemctl(&["disable", "--now", SERVICE_NAME], opts.sudo);
    remove_path(Path::new(SYSTEM_UNIT_PATH), opts.sudo)?;
    remove_path(Path::new(SYSTEM_BIN_PATH), opts.sudo)?;
    run_systemctl_or_fail(&["daemon-reload"], opts.sudo)?;
    run_systemctl_or_fail(&["reset-failed"], opts.sudo).ok();

    println!("Removed system service {SERVICE_NAME} and tray client {TRAY_SERVICE_NAME}");
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserSession {
    pub uid: u32,
    pub username: String,
    pub home: PathBuf,
    pub xdg_runtime_dir: String,
    pub dbus_address: String,
    pub display: Option<String>,
    pub wayland_display: Option<String>,
}

pub fn render_user_unit(exec_path: &Path) -> String {
    format!(
        r#"[Unit]
Description=Wrangler process CPU throttle daemon
Documentation=https://github.com/mholtzhausen/wrangler
After=graphical-session.target
PartOf=graphical-session.target

[Service]
Type=simple
ExecStart={} --daemon --foreground
Restart=on-failure
RestartSec=5

[Install]
WantedBy=graphical-session.target
"#,
        exec_path.display()
    )
}

pub fn render_system_unit(exec_path: &Path, session: &UserSession) -> String {
    format!(
        r#"[Unit]
Description=Wrangler process CPU throttle daemon (system)
Documentation=https://github.com/mholtzhausen/wrangler
After=network-online.target
ConditionPathExists={}
ConditionPathExists={}

[Service]
Type=simple
Environment=HOME={}
Environment=XDG_RUNTIME_DIR={}
ExecStart={} --daemon --no-tray --foreground --cgroups
Restart=on-failure
RestartSec=5

[Install]
WantedBy=multi-user.target
"#,
        session.xdg_runtime_dir,
        session.xdg_runtime_dir,
        session.home.display(),
        session.xdg_runtime_dir,
        exec_path.display()
    )
}

pub fn render_tray_user_unit(exec_path: &Path) -> String {
    format!(
        r#"[Unit]
Description=Wrangler system tray (connects to root daemon)
Documentation=https://github.com/mholtzhausen/wrangler
After=graphical-session.target
PartOf=graphical-session.target

[Service]
Type=simple
ExecStart={} --tray-client --foreground
Restart=on-failure
RestartSec=5

[Install]
WantedBy=graphical-session.target
"#,
        exec_path.display()
    )
}

fn resolve_target_user_session() -> Result<UserSession, Box<dyn std::error::Error>> {
    let (uid, username) = if let Ok(uid_str) = std::env::var("SUDO_UID") {
        let uid: u32 = uid_str.parse()?;
        let name = std::env::var("SUDO_USER").unwrap_or_else(|_| uid.to_string());
        (uid, name)
    } else if geteuid().is_root() {
        return Err(
            "system service install needs a desktop user session; run with sudo from your user account"
                .into(),
        );
    } else {
        let uid = getuid().as_raw();
        let name = User::from_uid(getuid())
            .ok()
            .flatten()
            .map(|user| user.name)
            .unwrap_or_else(|| uid.to_string());
        (uid, name)
    };

    let home = User::from_uid(nix::unistd::Uid::from_raw(uid))
        .ok()
        .flatten()
        .map(|user| user.dir)
        .unwrap_or_else(|| PathBuf::from(format!("/home/{username}")));

    let xdg_runtime_dir = format!("/run/user/{uid}");
    let dbus_address = format!("unix:path={xdg_runtime_dir}/bus");
    let (display, wayland_display) = read_display_env(&username);

    Ok(UserSession {
        uid,
        username,
        home,
        xdg_runtime_dir,
        dbus_address,
        display,
        wayland_display,
    })
}

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

fn user_local_bin_dir() -> Result<PathBuf, Box<dyn std::error::Error>> {
    let home = std::env::var("HOME")?;
    Ok(PathBuf::from(home).join(".local").join("bin"))
}

fn user_systemd_dir() -> Result<PathBuf, Box<dyn std::error::Error>> {
    let home = std::env::var("HOME")?;
    Ok(PathBuf::from(home)
        .join(".config")
        .join("systemd")
        .join("user"))
}

fn run_systemctl(args: &[&str], requested_sudo: bool) -> io::Result<Output> {
    privileged_command("systemctl", args, requested_sudo).output()
}

fn run_systemctl_user(args: &[&str]) -> io::Result<Output> {
    Command::new("systemctl").arg("--user").args(args).output()
}

fn run_systemctl_user_for(session: &UserSession, args: &[&str]) -> io::Result<Output> {
    Command::new("runuser")
        .args(["-u", &session.username, "--"])
        .env("XDG_RUNTIME_DIR", &session.xdg_runtime_dir)
        .env("DBUS_SESSION_BUS_ADDRESS", &session.dbus_address)
        .arg("systemctl")
        .arg("--user")
        .args(args)
        .output()
}

fn install_tray_user_service(
    session: &UserSession,
    bin_path: &Path,
    requested_sudo: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let unit_dir = session.home.join(".config/systemd/user");
    let unit_path = unit_dir.join(TRAY_SERVICE_NAME);
    let unit = render_tray_user_unit(bin_path);

    if geteuid().is_root() || requested_sudo {
        let status = privileged_command(
            "runuser",
            &[
                "-u",
                &session.username,
                "--",
                "mkdir",
                "-p",
                unit_dir.to_string_lossy().as_ref(),
            ],
            use_sudo(requested_sudo),
        )
        .status()?;
        if !status.success() {
            return Err(format!("failed to create {}", unit_dir.display()).into());
        }
        write_file_as_user(&session.username, &unit_path, &unit, requested_sudo)?;
        return Ok(());
    }

    std::fs::create_dir_all(&unit_dir)?;
    std::fs::write(&unit_path, unit)?;
    Ok(())
}

fn enable_tray_user_service(
    session: &UserSession,
    requested_sudo: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    systemctl_user_for_or_fail(session, &["daemon-reload"], requested_sudo)?;
    systemctl_user_for_or_fail(session, &["enable", "--now", TRAY_SERVICE_NAME], requested_sudo)?;
    Ok(())
}

fn disable_tray_user_service(session: &UserSession, requested_sudo: bool) -> Result<(), ()> {
    let _ = systemctl_user_for(session, &["disable", "--now", TRAY_SERVICE_NAME], requested_sudo);
    let _ = systemctl_user_for(session, &["daemon-reload"], requested_sudo);
    Ok(())
}

fn systemctl_user_for(
    session: &UserSession,
    args: &[&str],
    requested_sudo: bool,
) -> io::Result<Output> {
    if geteuid().is_root() {
        run_systemctl_user_for(session, args)
    } else if requested_sudo {
        let mut cmd = Command::new("sudo");
        cmd.args([
            "runuser",
            "-u",
            &session.username,
            "--",
            "env",
            &format!("XDG_RUNTIME_DIR={}", session.xdg_runtime_dir),
            &format!("DBUS_SESSION_BUS_ADDRESS={}", session.dbus_address),
            "systemctl",
            "--user",
        ]);
        cmd.args(args);
        cmd.output()
    } else {
        run_systemctl_user(args)
    }
}

fn systemctl_user_for_or_fail(
    session: &UserSession,
    args: &[&str],
    requested_sudo: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let output = systemctl_user_for(session, args, requested_sudo)?;
    systemctl_success(&output, args)
}

fn write_file_as_user(
    username: &str,
    path: &Path,
    contents: &str,
    requested_sudo: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    if use_sudo(requested_sudo) || geteuid().is_root() {
        let mut child = privileged_command(
            "runuser",
            &["-u", username, "--", "tee", path.to_string_lossy().as_ref()],
            use_sudo(requested_sudo),
        )
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
        if let Some(mut stdin) = child.stdin.take() {
            stdin.write_all(contents.as_bytes())?;
        }
        let status = child.wait()?;
        if !status.success() {
            return Err(format!("failed to write {}", path.display()).into());
        }
        return Ok(());
    }
    std::fs::write(path, contents)?;
    Ok(())
}

fn remove_path_owned(
    path: &Path,
    username: &str,
    requested_sudo: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    if !path.exists() {
        return Ok(());
    }
    if geteuid().is_root() || use_sudo(requested_sudo) {
        let status = privileged_command(
            "runuser",
            &["-u", username, "--", "rm", path.to_string_lossy().as_ref()],
            use_sudo(requested_sudo),
        )
        .status()?;
        if !status.success() {
            return Err(format!("failed to remove {}", path.display()).into());
        }
        return Ok(());
    }
    std::fs::remove_file(path)?;
    Ok(())
}

fn run_systemctl_or_fail(
    args: &[&str],
    requested_sudo: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let output = run_systemctl(args, requested_sudo)?;
    systemctl_success(&output, args)
}

fn run_systemctl_user_or_fail(args: &[&str]) -> Result<(), Box<dyn std::error::Error>> {
    let output = run_systemctl_user(args)?;
    systemctl_success(&output, args)
}

fn systemctl_success(output: &Output, args: &[&str]) -> Result<(), Box<dyn std::error::Error>> {
    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "systemctl {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        )
        .into())
    }
}

fn print_systemctl_output(output: &Output) -> Result<(), Box<dyn std::error::Error>> {
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    if !stdout.is_empty() {
        print!("{stdout}");
    }
    if !stderr.is_empty() {
        eprint!("{stderr}");
    }
    Ok(())
}

fn copy_binary_local(src: &Path, dest: &Path) -> Result<(), Box<dyn std::error::Error>> {
    std::fs::copy(src, dest)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dest, std::fs::Permissions::from_mode(0o755))?;
    }
    Ok(())
}

fn write_system_unit(
    content: &str,
    requested_sudo: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    if use_sudo(requested_sudo) {
        let mut child = privileged_command("tee", &[SYSTEM_UNIT_PATH], true)
            .stdin(Stdio::piped())
            .spawn()?;
        if let Some(mut stdin) = child.stdin.take() {
            stdin.write_all(content.as_bytes())?;
        }
        let status = child.wait()?;
        if !status.success() {
            return Err(format!("failed to write {SYSTEM_UNIT_PATH}").into());
        }
        return Ok(());
    }

    std::fs::write(SYSTEM_UNIT_PATH, content)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_session() -> UserSession {
        UserSession {
            uid: 1000,
            username: "alice".into(),
            home: PathBuf::from("/home/alice"),
            xdg_runtime_dir: "/run/user/1000".into(),
            dbus_address: "unix:path=/run/user/1000/bus".into(),
            display: Some(":0".into()),
            wayland_display: None,
        }
    }

    #[test]
    fn user_unit_enables_tray_daemon() {
        let unit = render_user_unit(Path::new("/home/alice/.local/bin/wrangler"));
        assert!(unit.contains("ExecStart=/home/alice/.local/bin/wrangler --daemon --foreground"));
        assert!(!unit.contains("--no-tray"));
        assert!(unit.contains("WantedBy=graphical-session.target"));
    }

    #[test]
    fn system_unit_runs_headless_with_user_runtime_socket() {
        let unit = render_system_unit(Path::new("/usr/local/bin/wrangler"), &sample_session());
        assert!(unit.contains("ExecStart=/usr/local/bin/wrangler --daemon --no-tray --foreground --cgroups"));
        assert!(unit.contains("Environment=XDG_RUNTIME_DIR=/run/user/1000"));
        assert!(unit.contains("Environment=HOME=/home/alice"));
        assert!(unit.contains("ConditionPathExists=/run/user/1000"));
        assert!(!unit.contains("DBUS_SESSION_BUS_ADDRESS"));
    }

    #[test]
    fn tray_user_unit_runs_tray_client() {
        let unit = render_tray_user_unit(Path::new("/usr/local/bin/wrangler"));
        assert!(unit.contains("ExecStart=/usr/local/bin/wrangler --tray-client --foreground"));
        assert!(unit.contains("WantedBy=graphical-session.target"));
    }
}
