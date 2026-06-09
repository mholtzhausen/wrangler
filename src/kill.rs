use crate::binary_install::{privileged_command, use_sudo};
use crate::cli::KillOptions;
use nix::sys::signal::{kill, Signal};
use nix::unistd::{geteuid, getpid, getuid, Pid};
use std::collections::HashSet;
use std::process::Command;
use std::thread;
use std::time::Duration;
use sysinfo::{ProcessRefreshKind, System};

#[cfg(target_os = "linux")]
use crate::service::{SERVICE_NAME, TRAY_SERVICE_NAME};

const GRACE_PERIOD: Duration = Duration::from_millis(500);

pub fn run(opts: &KillOptions) -> Result<(), Box<dyn std::error::Error>> {
    validate_options(opts)?;

    let self_pid = getpid().as_raw();
    let targets = target_uids(opts);

    #[cfg(target_os = "linux")]
    stop_managed_services(opts)?;

    let pids = find_wrangler_pids(&targets, self_pid);
    if pids.is_empty() {
        println!("No wrangler processes found");
        return Ok(());
    }

    let killed = terminate_pids(&pids, opts)?;
    println!("Stopped {killed} wrangler process(es)");
    Ok(())
}

fn validate_options(opts: &KillOptions) -> Result<(), Box<dyn std::error::Error>> {
    if (opts.sudo || opts.all) && !geteuid().is_root() && !sudo_available() {
        return Err("killing root wrangler processes requires a working sudo".into());
    }
    Ok(())
}

fn sudo_available() -> bool {
    Command::new("sudo")
        .args(["-n", "true"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
        || Command::new("which")
            .arg("sudo")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|status| status.success())
            .unwrap_or(false)
}

fn target_uids(opts: &KillOptions) -> HashSet<u32> {
    let mut uids = HashSet::new();
    if opts.all || !opts.sudo {
        uids.insert(getuid().as_raw());
    }
    if opts.all || opts.sudo {
        uids.insert(0);
    }
    uids
}

fn find_wrangler_pids(target_uids: &HashSet<u32>, self_pid: i32) -> Vec<(i32, u32)> {
    let mut sys = System::new_with_specifics(
        sysinfo::RefreshKind::new().with_processes(ProcessRefreshKind::everything()),
    );
    sys.refresh_processes();

    let mut matches = Vec::new();
    for (pid, process) in sys.processes() {
        let pid_raw = pid.as_u32() as i32;
        if pid_raw == self_pid || !is_wrangler_process(process) {
            continue;
        }

        let Some(proc_uid) = process.user_id().map(|uid| **uid) else {
            continue;
        };
        if target_uids.contains(&proc_uid) {
            matches.push((pid_raw, proc_uid));
        }
    }

    matches.sort_unstable_by_key(|(pid, _)| *pid);
    matches
}

fn is_wrangler_process(process: &sysinfo::Process) -> bool {
    if process.name() == "wrangler" {
        return true;
    }

    if process
        .exe()
        .and_then(|path| path.file_name())
        .is_some_and(|name| name == "wrangler")
    {
        return true;
    }

    process.cmd().iter().any(|arg| {
        arg == "wrangler" || arg.ends_with("/wrangler")
    })
}

fn terminate_pids(pids: &[(i32, u32)], opts: &KillOptions) -> Result<usize, Box<dyn std::error::Error>> {
    let mut killed = HashSet::new();

    for &(pid, uid) in pids {
        if signal_pid(pid, Signal::SIGTERM, uid, opts).is_ok() {
            killed.insert(pid);
        }
    }

    thread::sleep(GRACE_PERIOD);

    for &(pid, uid) in &find_wrangler_pids(&target_uids(opts), getpid().as_raw()) {
        if signal_pid(pid, Signal::SIGKILL, uid, opts).is_ok() {
            killed.insert(pid);
        }
    }

    Ok(killed.len())
}

fn signal_pid(pid: i32, signal: Signal, owner_uid: u32, opts: &KillOptions) -> Result<(), nix::Error> {
    let needs_privilege = owner_uid == 0 && !geteuid().is_root();
    if needs_privilege {
        send_signal_privileged(pid, signal, opts)?;
        return Ok(());
    }
    kill(Pid::from_raw(pid), signal)
}

fn send_signal_privileged(
    pid: i32,
    signal: Signal,
    opts: &KillOptions,
) -> Result<(), nix::Error> {
    let flag = match signal {
        Signal::SIGTERM => "-TERM",
        Signal::SIGKILL => "-KILL",
        _ => return Err(nix::Error::EINVAL),
    };

    let use_sudo_flag = opts.sudo || opts.all;
    if !use_sudo_flag {
        return Err(nix::Error::EPERM);
    }

    let status = privileged_command(
        "kill",
        &[flag, &pid.to_string()],
        use_sudo(use_sudo_flag),
    )
    .status()
    .map_err(|_| nix::Error::UnknownErrno)?;

    if status.success() {
        Ok(())
    } else {
        Err(nix::Error::UnknownErrno)
    }
}

#[cfg(target_os = "linux")]
fn stop_managed_services(opts: &KillOptions) -> Result<(), Box<dyn std::error::Error>> {
    let stop_user = !opts.sudo || opts.all;
    let stop_root = opts.sudo || opts.all;

    if stop_user {
        let _ = Command::new("systemctl")
            .args([
                "--user",
                "stop",
                SERVICE_NAME,
                TRAY_SERVICE_NAME,
            ])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    }

    if stop_root {
        let needs_sudo = !geteuid().is_root() && (opts.sudo || opts.all);
        let status = if needs_sudo {
            privileged_command("systemctl", &["stop", SERVICE_NAME], true).status()?
        } else {
            Command::new("systemctl")
                .args(["stop", SERVICE_NAME])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()?
        };
        let _ = status;

        if opts.sudo || opts.all {
            let _ = Command::new("systemctl")
                .args(["--user", "stop", TRAY_SERVICE_NAME])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status();
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_uids_default_is_current_user_only() {
        let opts = KillOptions::default();
        let uids = target_uids(&opts);
        assert_eq!(uids.len(), 1);
        assert!(uids.contains(&getuid().as_raw()));
    }

    #[test]
    fn target_uids_sudo_is_root_only() {
        let opts = KillOptions {
            sudo: true,
            all: false,
        };
        let uids = target_uids(&opts);
        assert_eq!(uids, HashSet::from([0]));
    }

    #[test]
    fn target_uids_all_includes_user_and_root() {
        let opts = KillOptions {
            sudo: false,
            all: true,
        };
        let uids = target_uids(&opts);
        assert_eq!(uids.len(), 2);
        assert!(uids.contains(&getuid().as_raw()));
        assert!(uids.contains(&0));
    }

}
