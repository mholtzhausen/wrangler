use crate::cli::InstallOptions;
use nix::unistd::geteuid;
use std::path::{Path, PathBuf};
use std::process::Command;

pub const DEFAULT_INSTALL_DIR: &str = "/usr/local/bin";
pub const SYSTEM_BIN_PATH: &str = "/usr/local/bin/wrangler";

pub fn install_bin_dir() -> PathBuf {
    std::env::var("WRANGLER_INSTALL_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(DEFAULT_INSTALL_DIR))
}

pub fn install_bin_path() -> PathBuf {
    install_bin_dir().join("wrangler")
}

pub fn install(opts: &InstallOptions) -> Result<(), Box<dyn std::error::Error>> {
    ensure_privileged(opts.sudo, "install")?;

    let exe = std::env::current_exe()?;
    let dest = install_bin_path();
    copy_binary(&exe, &dest, opts.sudo)?;

    println!("Installed {}", dest.display());
    Ok(())
}

pub fn uninstall(opts: &InstallOptions) -> Result<(), Box<dyn std::error::Error>> {
    ensure_privileged(opts.sudo, "uninstall")?;

    let dest = install_bin_path();
    if !dest.exists() {
        println!("No binary at {}", dest.display());
        return Ok(());
    }

    remove_path(&dest, opts.sudo)?;
    println!("Removed {}", dest.display());
    Ok(())
}

pub fn ensure_privileged(requested_sudo: bool, action: &str) -> Result<(), Box<dyn std::error::Error>> {
    if geteuid().is_root() || requested_sudo {
        return Ok(());
    }
    Err(format!(
        "binary {action} requires root privileges; re-run with --sudo"
    )
    .into())
}

pub fn use_sudo(requested: bool) -> bool {
    requested && !geteuid().is_root()
}

pub fn privileged_command(program: &str, args: &[&str], requested_sudo: bool) -> Command {
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

pub fn copy_binary(
    src: &Path,
    dest: &Path,
    requested_sudo: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    if use_sudo(requested_sudo) {
        let status = privileged_command(
            "cp",
            &[
                src.to_string_lossy().as_ref(),
                dest.to_string_lossy().as_ref(),
            ],
            true,
        )
        .status()?;
        if !status.success() {
            return Err(format!("failed to copy binary to {}", dest.display()).into());
        }
        let status = privileged_command("chmod", &["755", dest.to_string_lossy().as_ref()], true)
            .status()?;
        if !status.success() {
            return Err(format!("failed to chmod {}", dest.display()).into());
        }
        return Ok(());
    }

    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::copy(src, dest).map_err(|error| {
        if error.kind() == std::io::ErrorKind::Other
            || error.raw_os_error() == Some(26)
        {
            format!(
                "failed to install {}: {error} (stop running wrangler services first)",
                dest.display()
            )
        } else {
            format!("failed to install {}: {error}", dest.display())
        }
    })?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dest, std::fs::Permissions::from_mode(0o755))?;
    }
    Ok(())
}

pub fn remove_path(path: &Path, requested_sudo: bool) -> Result<(), Box<dyn std::error::Error>> {
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
    fn install_bin_path_defaults_to_system_location() {
        std::env::remove_var("WRANGLER_INSTALL_DIR");
        assert_eq!(install_bin_path(), PathBuf::from(SYSTEM_BIN_PATH));
    }

    #[test]
    fn install_bin_path_honors_env_override() {
        std::env::set_var("WRANGLER_INSTALL_DIR", "/opt/bin");
        assert_eq!(install_bin_path(), PathBuf::from("/opt/bin/wrangler"));
        std::env::remove_var("WRANGLER_INSTALL_DIR");
    }
}
