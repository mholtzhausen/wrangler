use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

const CGROUP_ROOT: &str = "/sys/fs/cgroup/wrangler";

pub struct CgroupThrottle {
    active: HashMap<u32, Vec<u32>>,
}

impl CgroupThrottle {
    pub fn new() -> Self {
        Self {
            active: HashMap::new(),
        }
    }

    pub fn start_group(
        &mut self,
        group_key: u32,
        pids: &[u32],
        app_cap: f32,
        num_cores: usize,
    ) -> Result<(), String> {
        let cgroup_path = cgroup_dir(group_key);
        fs::create_dir_all(&cgroup_path).map_err(|e| format!("create cgroup: {e}"))?;

        let cpu_max = compute_group_cpu_max(app_cap, num_cores);
        fs::write(cgroup_path.join("cpu.max"), cpu_max)
            .map_err(|e| format!("write cpu.max: {e}"))?;

        let mut attached = Vec::new();
        for pid in pids {
            if self.attach_pid(group_key, *pid).is_ok() {
                attached.push(*pid);
            }
        }

        if attached.is_empty() {
            let _ = fs::remove_dir(&cgroup_path);
            return Err("no processes attached to cgroup".into());
        }

        self.active.insert(group_key, attached);
        Ok(())
    }

    pub fn sync_group(&mut self, group_key: u32, pids: &[u32]) -> Result<(), String> {
        if !self.active.contains_key(&group_key) {
            return Err(format!("group {group_key} is not active"));
        }

        let mut attached = self.active.get(&group_key).cloned().unwrap_or_default();
        for pid in pids {
            if !attached.contains(pid) {
                self.attach_pid(group_key, *pid)?;
                attached.push(*pid);
            }
        }

        attached.retain(|pid| pids.contains(pid));
        self.active.insert(group_key, attached);
        Ok(())
    }

    pub fn stop_group(&mut self, group_key: u32) {
        self.active.remove(&group_key);
        let path = cgroup_dir(group_key);
        let _ = fs::remove_dir(&path);
    }

    pub fn stop_all(&mut self) {
        let keys: Vec<u32> = self.active.keys().copied().collect();
        for key in keys {
            self.stop_group(key);
        }
    }

    fn attach_pid(&self, group_key: u32, pid: u32) -> Result<(), String> {
        let cgroup_path = cgroup_dir(group_key);
        let mut procs = fs::OpenOptions::new()
            .write(true)
            .open(cgroup_path.join("cgroup.procs"))
            .map_err(|e| format!("open cgroup.procs: {e}"))?;

        writeln!(procs, "{pid}").map_err(|e| format!("write cgroup.procs: {e}"))?;
        Ok(())
    }
}

fn cgroup_dir(group_key: u32) -> PathBuf {
    Path::new(CGROUP_ROOT).join(format!("group-{group_key}"))
}

fn compute_group_cpu_max(app_cap: f32, num_cores: usize) -> String {
    let quota = ((app_cap / 100.0) * num_cores as f32 * 100_000.0).round() as u64;
    let quota = quota.max(1);
    format!("{quota} 100000")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn group_cpu_max_scales_with_machine_share() {
        assert_eq!(compute_group_cpu_max(40.0, 8), "320000 100000");
        assert_eq!(compute_group_cpu_max(100.0, 1), "100000 100000");
    }
}
