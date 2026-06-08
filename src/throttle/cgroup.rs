use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

pub const CGROUP_ROOT: &str = "/sys/fs/cgroup/wrangler";
const UNIFIED_ROOT: &str = "/sys/fs/cgroup";

pub struct CgroupThrottle {
    active: HashMap<u32, Vec<u32>>,
}

impl CgroupThrottle {
    pub fn new() -> Result<Self, String> {
        ensure_wrangler_root()?;
        Ok(Self {
            active: HashMap::new(),
        })
    }

    pub fn start_group(
        &mut self,
        group_key: u32,
        pids: &[u32],
        app_cap: f32,
        num_cores: usize,
    ) -> Result<(), String> {
        ensure_wrangler_root()?;

        let cgroup_path = cgroup_dir(group_key);
        if cgroup_path.exists() {
            release_group_pids(group_key)?;
            let _ = fs::remove_dir(&cgroup_path);
        }

        fs::create_dir_all(&cgroup_path).map_err(|e| format!("create cgroup: {e}"))?;
        write_cpu_max(&cgroup_path, app_cap, num_cores)?;

        let mut attached = Vec::new();
        for pid in pids {
            match attach_pid(&cgroup_path, *pid) {
                Ok(()) => attached.push(*pid),
                Err(e) => tracing::warn!(pid, error = %e, "failed to attach pid to cgroup"),
            }
        }

        if attached.is_empty() {
            let _ = fs::remove_dir(&cgroup_path);
            return Err("no processes attached to cgroup".into());
        }

        self.active.insert(group_key, attached);
        tracing::info!(
            group_key,
            path = %cgroup_path.display(),
            pids = ?self.active.get(&group_key),
            "cgroup throttle started"
        );
        Ok(())
    }

    pub fn sync_group(&mut self, group_key: u32, pids: &[u32]) -> Result<(), String> {
        let Some(attached) = self.active.get_mut(&group_key) else {
            return Err(format!("group {group_key} is not active"));
        };

        let cgroup_path = cgroup_dir(group_key);
        for pid in pids {
            if !attached.contains(pid) {
                attach_pid(&cgroup_path, *pid)?;
                attached.push(*pid);
                tracing::debug!(group_key, pid, "attached new group member to cgroup");
            }
        }

        attached.retain(|pid| pids.contains(pid));
        Ok(())
    }

    pub fn update_group_cap(
        &mut self,
        group_key: u32,
        app_cap: f32,
        num_cores: usize,
    ) -> Result<(), String> {
        if !self.active.contains_key(&group_key) {
            return Ok(());
        }
        write_cpu_max(&cgroup_dir(group_key), app_cap, num_cores)
    }

    pub fn update_all_caps(&mut self, app_cap: f32, num_cores: usize) -> Result<(), String> {
        let keys: Vec<u32> = self.active.keys().copied().collect();
        for key in keys {
            self.update_group_cap(key, app_cap, num_cores)?;
        }
        Ok(())
    }

    pub fn stop_group(&mut self, group_key: u32) {
        self.active.remove(&group_key);
        if let Err(e) = release_group_pids(group_key) {
            tracing::warn!(group_key, error = %e, "failed to release cgroup pids");
        }
        let path = cgroup_dir(group_key);
        if path.exists() {
            if let Err(e) = fs::remove_dir(&path) {
                tracing::warn!(group_key, error = %e, "failed to remove cgroup directory");
            }
        }
        tracing::info!(group_key, "cgroup throttle stopped");
    }

    pub fn stop_all(&mut self) {
        let keys: Vec<u32> = self.active.keys().copied().collect();
        for key in keys {
            self.stop_group(key);
        }
        let root = Path::new(CGROUP_ROOT);
        if root.exists() {
            let _ = fs::remove_dir(root);
        }
    }

    pub fn active_groups(&self) -> Vec<u32> {
        self.active.keys().copied().collect()
    }
}

pub fn cgroup_dir(group_key: u32) -> PathBuf {
    Path::new(CGROUP_ROOT).join(format!("group-{group_key}"))
}

pub fn compute_group_cpu_max(app_cap: f32, num_cores: usize) -> String {
    let quota = ((app_cap / 100.0) * num_cores as f32 * 100_000.0).round() as u64;
    let quota = quota.max(1);
    format!("{quota} 100000")
}

fn write_cpu_max(cgroup_path: &Path, app_cap: f32, num_cores: usize) -> Result<(), String> {
    let cpu_max = compute_group_cpu_max(app_cap, num_cores);
    fs::write(cgroup_path.join("cpu.max"), cpu_max)
        .map_err(|e| format!("write cpu.max: {e}"))
}

fn attach_pid(cgroup_path: &Path, pid: u32) -> Result<(), String> {
    let mut procs = fs::OpenOptions::new()
        .write(true)
        .open(cgroup_path.join("cgroup.procs"))
        .map_err(|e| format!("open cgroup.procs: {e}"))?;

    writeln!(procs, "{pid}").map_err(|e| format!("write cgroup.procs: {e}"))?;
    Ok(())
}

fn release_group_pids(group_key: u32) -> Result<(), String> {
    let group_path = cgroup_dir(group_key);
    if !group_path.exists() {
        return Ok(());
    }

    let procs = fs::read_to_string(group_path.join("cgroup.procs"))
        .map_err(|e| format!("read cgroup.procs: {e}"))?;
    let root_procs = Path::new(UNIFIED_ROOT).join("cgroup.procs");
    let mut out = fs::OpenOptions::new()
        .write(true)
        .open(&root_procs)
        .map_err(|e| format!("open root cgroup.procs: {e}"))?;

    for line in procs.lines() {
        let pid = line.trim();
        if pid.is_empty() {
            continue;
        }
        writeln!(out, "{pid}").map_err(|e| format!("migrate pid {pid} to root: {e}"))?;
    }

    Ok(())
}

fn ensure_wrangler_root() -> Result<(), String> {
    let unified = Path::new(UNIFIED_ROOT);
    if !unified.join("cgroup.controllers").exists() {
        return Err("cgroup v2 unified hierarchy not available".into());
    }

    enable_cpu_delegation(unified)?;

    let root = Path::new(CGROUP_ROOT);
    if !root.exists() {
        fs::create_dir(root).map_err(|e| format!("create wrangler cgroup root: {e}"))?;
    }

    enable_cpu_delegation(root)?;
    Ok(())
}

fn enable_cpu_delegation(cgroup_path: &Path) -> Result<(), String> {
    let subtree = cgroup_path.join("cgroup.subtree_control");
    if !subtree.exists() {
        return Ok(());
    }

    let current = fs::read_to_string(&subtree).unwrap_or_default();
    if current.split_whitespace().any(|token| token == "cpu") {
        return Ok(());
    }

    fs::OpenOptions::new()
        .append(true)
        .open(&subtree)
        .and_then(|mut file| writeln!(file, "+cpu"))
        .map_err(|e| format!("enable cpu controller under {}: {e}", cgroup_path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn group_cpu_max_scales_with_machine_share() {
        assert_eq!(compute_group_cpu_max(40.0, 8), "320000 100000");
        assert_eq!(compute_group_cpu_max(100.0, 1), "100000 100000");
    }

    #[test]
    fn cgroup_dir_uses_group_prefix() {
        assert_eq!(
            cgroup_dir(1234),
            PathBuf::from("/sys/fs/cgroup/wrangler/group-1234")
        );
    }
}
