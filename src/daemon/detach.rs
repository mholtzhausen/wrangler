use std::io;
use std::process::{Command, Stdio};

/// Re-exec wrangler as a background child so the launching shell regains its prompt.
///
/// Tokio runtimes must not be forked; spawning a fresh process is the safe pattern.
pub fn spawn_detached_child() -> io::Result<()> {
    let exe = std::env::current_exe()?;
    let args: Vec<String> = std::env::args().skip(1).collect();

    Command::new(exe)
        .args(args)
        .env("WRANGLER_DAEMON_CHILD", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;

    std::process::exit(0);
}

pub fn is_detached_child() -> bool {
    std::env::var_os("WRANGLER_DAEMON_CHILD").is_some()
}
