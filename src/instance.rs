use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::os::unix::io::AsRawFd;
use std::path::PathBuf;

use nix::fcntl::FlockArg;

pub struct DaemonLock {
    _file: File,
}

pub fn lock_path() -> PathBuf {
    if let Ok(dir) = std::env::var("WRANGLER_RUNTIME_DIR") {
        return PathBuf::from(dir).join("wrangler.lock");
    }
    if let Ok(dir) = std::env::var("XDG_RUNTIME_DIR") {
        return PathBuf::from(dir).join("wrangler.lock");
    }
    if let Ok(home) = std::env::var("HOME") {
        return PathBuf::from(home)
            .join(".local")
            .join("run")
            .join("wrangler.lock");
    }
    PathBuf::from("/tmp").join(format!("wrangler-{}.lock", std::process::id()))
}

pub fn acquire_daemon_lock() -> io::Result<DaemonLock> {
    acquire_daemon_lock_at(&lock_path())
}

pub fn acquire_daemon_lock_at(path: &std::path::Path) -> io::Result<DaemonLock> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let mut file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(path)?;

    #[allow(deprecated)]
    nix::fcntl::flock(file.as_raw_fd(), FlockArg::LockExclusiveNonblock).map_err(|errno| {
        io::Error::new(
            io::ErrorKind::WouldBlock,
            format!("another wrangler daemon holds {path:?}: {errno}"),
        )
    })?;

    file.set_len(0)?;
    file.write_all(format!("{}\n", std::process::id()).as_bytes())?;
    file.sync_all()?;

    Ok(DaemonLock { _file: file })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn lock_writes_current_pid() {
        let dir = std::env::temp_dir().join(format!(
            "wrangler-lock-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let lock_file = dir.join("wrangler.lock");

        let _lock = acquire_daemon_lock_at(&lock_file).expect("first lock");
        let contents = fs::read_to_string(&lock_file).expect("lock file");
        assert!(contents.contains(&std::process::id().to_string()));

        let _ = fs::remove_dir_all(&dir);
    }
}
