use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

const CGROUP_ROOT: &str = "/sys/fs/cgroup/wrangler";

pub struct CgroupThrottle {
    active: Vec<u32>,
}

impl CgroupThrottle {
    pub fn new() -> Self {
        Self {
            active: Vec::new(),
        }
    }

    pub fn start(&mut self, pid: u32, cpu_usage: f32, threshold: f32) -> Result<(), String> {
        if self.active.contains(&pid) {
            return Ok(());
        }

        let cgroup_path = cgroup_dir(pid);
        fs::create_dir_all(&cgroup_path)
            .map_err(|e| format!("create cgroup: {e}"))?;

        let cpu_max = compute_cpu_max(cpu_usage, threshold);
        fs::write(cgroup_path.join("cpu.max"), cpu_max)
            .map_err(|e| format!("write cpu.max: {e}"))?;

        let mut procs = fs::OpenOptions::new()
            .write(true)
            .open(cgroup_path.join("cgroup.procs"))
            .map_err(|e| format!("open cgroup.procs: {e}"))?;

        writeln!(procs, "{pid}").map_err(|e| format!("write cgroup.procs: {e}"))?;

        self.active.push(pid);
        Ok(())
    }

    pub fn stop(&mut self, pid: u32) {
        if let Some(pos) = self.active.iter().position(|p| *p == pid) {
            self.active.remove(pos);
        }
        let path = cgroup_dir(pid);
        let _ = fs::remove_dir(&path);
    }

    pub fn stop_all(&mut self) {
        let pids: Vec<u32> = self.active.clone();
        for pid in pids {
            self.stop(pid);
        }
    }
}

fn cgroup_dir(pid: u32) -> PathBuf {
    Path::new(CGROUP_ROOT).join(pid.to_string())
}

fn compute_cpu_max(cpu_usage: f32, threshold: f32) -> String {
    let target = if cpu_usage > 0.0 {
        (threshold / cpu_usage).clamp(0.05, 1.0)
    } else {
        0.5
    };
    let quota = (target * 100_000.0) as u64;
    format!("{quota} 100000")
}
