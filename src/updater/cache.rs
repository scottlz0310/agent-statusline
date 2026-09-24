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

    check_update_background_core(&cache_path, now, || {
        if let Ok(current_exe) = std::env::current_exe() {
            let _ = std::process::Command::new(current_exe)
                .arg("update")
                .arg("--background")
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn();
        }
    });
}

/// Core logic for background update check with dependency injection for spawning.
pub fn check_update_background_core<F: FnOnce()>(cache_path: &Path, now: i64, spawn_fn: F) -> bool {
    if let Some(cache) = UpdateCache::load(cache_path)
        && cache.is_fresh(now)
    {
        return false;
    }

    // Immediately record check attempt timestamp to suppress duplicate concurrent spawns
    let current_pkg = env!("CARGO_PKG_VERSION");
    let touch_cache = UpdateCache::load(cache_path).unwrap_or_else(|| UpdateCache {
        last_checked_at: now,
        latest_version: current_pkg.to_string(),
        has_update: false,
    });
    let updated = UpdateCache {
        last_checked_at: now,
        ..touch_cache
    };
    let _ = updated.save(cache_path);

    spawn_fn();
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

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
        assert!(!loaded.is_fresh(now + 86_400));
        assert!(!loaded.is_fresh(now + 100_000));
    }

    #[test]
    fn test_background_check_suppresses_duplicate_spawns() {
        let dir = tempfile::tempdir().unwrap();
        let cache_path = dir.path().join("update_check.json");

        let spawn_count = AtomicUsize::new(0);
        let now = 1_700_000_000;

        // 1st call: cache does not exist, should spawn and touch cache
        let spawned = check_update_background_core(&cache_path, now, || {
            spawn_count.fetch_add(1, Ordering::SeqCst);
        });
        assert!(spawned);
        assert_eq!(spawn_count.load(Ordering::SeqCst), 1);

        // Verify cache file was written with `now`
        let loaded = UpdateCache::load(&cache_path).unwrap();
        assert_eq!(loaded.last_checked_at, now);

        // 2nd call 10 seconds later: cache is fresh, should NOT spawn
        let spawned2 = check_update_background_core(&cache_path, now + 10, || {
            spawn_count.fetch_add(1, Ordering::SeqCst);
        });
        assert!(!spawned2);
        assert_eq!(spawn_count.load(Ordering::SeqCst), 1);

        // 3rd call 24 hours + 1s later: cache expired, should spawn again
        let spawned3 = check_update_background_core(&cache_path, now + 86_401, || {
            spawn_count.fetch_add(1, Ordering::SeqCst);
        });
        assert!(spawned3);
        assert_eq!(spawn_count.load(Ordering::SeqCst), 2);
    }
}
