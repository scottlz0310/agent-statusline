use serde::{Deserialize, Serialize};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

pub const UPDATE_CHECK_INTERVAL_SECS: u64 = 86_400; // 24 hours

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UpdateCache {
    pub last_checked_at: i64,
    pub latest_version: String,
    pub has_update: bool,
}

impl UpdateCache {
    pub fn load(path: &Path) -> Option<Self> {
        let content = fs::read_to_string(path).ok()?;
        serde_json::from_str(&content).ok()
    }

    pub fn save(&self, path: &Path) -> io::Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(self)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        fs::write(path, json)
    }

    #[must_use]
    pub fn is_fresh(&self, now: i64) -> bool {
        let diff = now - self.last_checked_at;
        diff >= 0 && diff.cast_unsigned() < UPDATE_CHECK_INTERVAL_SECS
    }
}

#[must_use]
pub fn get_cache_file_path() -> PathBuf {
    dirs::cache_dir()
        .or_else(dirs::data_local_dir)
        .unwrap_or_else(|| PathBuf::from("."))
        .join("agent-statusline")
        .join("update_check.json")
}

/// Zero-latency background update check for `render` execution.
/// Checks the timestamp of the cache file (< 0ms).
/// Only if 24 hours have elapsed, spawns a detached background process to query GitHub Releases API.
pub fn check_update_background_if_needed() {
    let cache_path = get_cache_file_path();
    let now = chrono::Utc::now().timestamp();

    if let Some(cache) = UpdateCache::load(&cache_path)
        && cache.is_fresh(now)
    {
        return;
    }

    // Spawn detached background process
    if let Ok(current_exe) = std::env::current_exe() {
        let _ = std::process::Command::new(current_exe)
            .arg("update")
            .arg("--background")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_update_cache_roundtrip_and_freshness() {
        let dir = tempfile::tempdir().unwrap();
        let cache_path = dir.path().join("update_check.json");

        let now = 1_700_000_000;
        let cache = UpdateCache {
            last_checked_at: now,
            latest_version: "v0.2.0".to_string(),
            has_update: true,
        };

        cache.save(&cache_path).unwrap();
        let loaded = UpdateCache::load(&cache_path).unwrap();
        assert_eq!(loaded, cache);

        // Within 24 hours
        assert!(loaded.is_fresh(now + 3600));
        assert!(loaded.is_fresh(now + 86399));

        // Expired after 24 hours
        assert!(!loaded.is_fresh(now + 86400));
        assert!(!loaded.is_fresh(now + 100000));
    }
}
