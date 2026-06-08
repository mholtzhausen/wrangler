use crate::cli::{ServiceCommand, ServiceOptions};
use nix::unistd::geteuid;
use std::io::{self, Write};
use std::path::Path;
use std::process::{Command, Output, Stdio};

pub const SERVICE_NAME: &str = "wrangler.service";
pub const SYSTEM_UNIT_PATH: &str = "/etc/systemd/system/wrangler.service";
pub const INSTALL_BIN_PATH: &str = "/usr/local/bin/wrangler";

pub fn run(command: &ServiceCommand) -> Result<(), Box<dyn std::error::Error>> {
    match command {
        ServiceCommand::Status(opts) => status(opts),
        ServiceCommand::Install(opts) => install(opts),
        ServiceCommand::Uninstall(opts) => uninstall(opts),
    }
}

fn status(opts: &ServiceOptions) -> Result<(), Box<dyn std::error::Error>> {
    let output = run_systemctl(&["status", SERVICE_NAME, "--no-pager"], opts.sudo)?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    if !stdout.is_empty() {
        print!("{stdout}");
    }
    if !stderr.is_empty() {
        eprint!("{stderr}");
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
}

fn install(opts: &ServiceOptions) -> Result<(), Box<dyn std::error::Error>> {
    ensure_privileged(opts.sudo, "install")?;

    let exe = std::env::current_exe()?;
    let unit = render_system_unit(Path::new(INSTALL_BIN_PATH));

    copy_binary(&exe, Path::new(INSTALL_BIN_PATH), opts.sudo)?;
    write_system_unit(&unit, opts.sudo)?;
    run_systemctl_or_fail(&["daemon-reload"], opts.sudo)?;
    run_systemctl_or_fail(&["enable", "--now", SERVICE_NAME], opts.sudo)?;

    println!("Installed system service at {SYSTEM_UNIT_PATH}");
    println!("Binary installed to {INSTALL_BIN_PATH}");
    println!("Service enabled and started. Check with: wrangler service status --sudo");
    Ok(())
}

fn uninstall(opts: &ServiceOptions) -> Result<(), Box<dyn std::error::Error>> {
    ensure_privileged(opts.sudo, "uninstall")?;

    let _ = run_systemctl(&["disable", "--now", SERVICE_NAME], opts.sudo);
    remove_path(Path::new(SYSTEM_UNIT_PATH), opts.sudo)?;
    remove_path(Path::new(INSTALL_BIN_PATH), opts.sudo)?;
    run_systemctl_or_fail(&["daemon-reload"], opts.sudo)?;
    run_systemctl_or_fail(&["reset-failed"], opts.sudo).ok();

    println!("Removed system service {SERVICE_NAME}");
    Ok(())
}

pub fn render_system_unit(exec_path: &Path) -> String {
    format!(
        r#"[Unit]
Description=Wrangler process CPU throttle daemon (system)
Documentation=https://github.com/mholtzhausen/wrangler
After=network.target

[Service]
Type=simple
ExecStart={} --daemon --no-tray --foreground --cgroups
Restart=on-failure
RestartSec=5

[Install]
WantedBy=multi-user.target
"#,
        exec_path.display()
    )
}

fn ensure_privileged(requested_sudo: bool, action: &str) -> Result<(), Box<dyn std::error::Error>> {
    if geteuid().is_root() || requested_sudo {
        return Ok(());
    }
    Err(format!(
        "service {action} requires root privileges; re-run with --sudo"
    )
    .into())
}

fn use_sudo(requested: bool) -> bool {
    requested && !geteuid().is_root()
}

fn privileged_command(program: &str, args: &[&str], requested_sudo: bool) -> Command {
    if use_sudo(requested_sudo) {
        let mut cmd = Command::new("sudo");
        cmd.arg(program).args(args);
        cmd
    } else {
        let mut cmd = Command::new(program);
        cmd.args(args);
        cmd
    }
}

fn run_systemctl(args: &[&str], requested_sudo: bool) -> io::Result<Output> {
    privileged_command("systemctl", args, requested_sudo).output()
}

fn run_systemctl_or_fail(args: &[&str], requested_sudo: bool) -> Result<(), Box<dyn std::error::Error>> {
    let output = run_systemctl(args, requested_sudo)?;
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

fn copy_binary(src: &Path, dest: &Path, requested_sudo: bool) -> Result<(), Box<dyn std::error::Error>> {
    if use_sudo(requested_sudo) {
        let status = privileged_command(
            "cp",
            &[src.to_string_lossy().as_ref(), dest.to_string_lossy().as_ref()],
            true,
        )
        .status()?;
        if !status.success() {
            return Err(format!("failed to copy binary to {}", dest.display()).into());
        }
        let status = privileged_command("chmod", &["755", dest.to_string_lossy().as_ref()], true).status()?;
        if !status.success() {
            return Err(format!("failed to chmod {}", dest.display()).into());
        }
        return Ok(());
    }

    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::copy(src, dest)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dest, std::fs::Permissions::from_mode(0o755))?;
    }
    Ok(())
}

fn write_system_unit(content: &str, requested_sudo: bool) -> Result<(), Box<dyn std::error::Error>> {
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

fn remove_path(path: &Path, requested_sudo: bool) -> Result<(), Box<dyn std::error::Error>> {
    if !path.exists() {
        return Ok(());
    }
    if use_sudo(requested_sudo) {
        let status = privileged_command("rm", &[path.to_string_lossy().as_ref()], true).status()?;
        if !status.success() {
            return Err(format!("failed to remove {}", path.display()).into());
        }
        return Ok(());
    }
    std::fs::remove_file(path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_unit_renders_exec_path() {
        let unit = render_system_unit(Path::new("/usr/local/bin/wrangler"));
        assert!(unit.contains("ExecStart=/usr/local/bin/wrangler --daemon --no-tray --foreground --cgroups"));
        assert!(unit.contains("WantedBy=multi-user.target"));
    }
}
