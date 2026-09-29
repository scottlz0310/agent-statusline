//! Windows の gix は、システム gitconfig の場所を調べるために `git.exe` を起動する (約 40ms)。
//! `render` は描画のたびに新しいプロセスとして起動されるため、求めたパスをキャッシュして再利用する。

use serde::{Deserialize, Serialize};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

const REFRESH_INTERVAL_SECS: i64 = 86_400; // 24 hours

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct SystemConfigCache {
    resolved_at: i64,
    /// Git が見つからないなどで解決できなかった場合は `None`
    path: Option<PathBuf>,
}

impl SystemConfigCache {
    fn load(path: &Path) -> Option<Self> {
        let content = fs::read_to_string(path).ok()?;
        serde_json::from_str(&content).ok()
    }

    fn save(&self, path: &Path) -> io::Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(self)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        fs::write(path, json)
    }

    fn is_fresh(&self, now: i64) -> bool {
        (0..REFRESH_INTERVAL_SECS).contains(&(now - self.resolved_at))
    }
}

/// gix に渡すシステム gitconfig のパス。Windows 以外は固定パスで、プロセスを起動しない。
pub fn system_config_path() -> Option<PathBuf> {
    let resolve = || gix::path::env::system_config().map(Path::to_path_buf);
    if !cfg!(windows) {
        return resolve();
    }
    let cache_path = cache_file_path();
    let (path, save_error) = resolve_cached(&cache_path, chrono::Utc::now().timestamp(), resolve);
    // 保存できないと描画のたびに `git` が起動するため、原因を追えるよう警告する
    if let Some(err) = save_error {
        eprintln!(
            "[agent-statusline] Failed to save git system config cache to {}: {err}",
            cache_path.display()
        );
    }
    path
}

fn cache_file_path() -> PathBuf {
    dirs::cache_dir()
        .or_else(dirs::data_local_dir)
        .unwrap_or_else(|| PathBuf::from("."))
        .join("agent-statusline")
        .join("git_system_config.json")
}

/// 解決したパスと、キャッシュの保存に失敗した場合のエラーを返す
fn resolve_cached(
    cache_path: &Path,
    now: i64,
    resolve: impl FnOnce() -> Option<PathBuf>,
) -> (Option<PathBuf>, Option<io::Error>) {
    if let Some(cache) = SystemConfigCache::load(cache_path)
        && cache.is_fresh(now)
        && cache.path.as_deref().is_none_or(Path::is_file)
    {
        return (cache.path, None);
    }

    let path = resolve();
    let save_error = SystemConfigCache {
        resolved_at: now,
        path: path.clone(),
    }
    .save(cache_path)
    .err();
    (path, save_error)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    const NOW: i64 = 1_800_000_000;

    #[test]
    fn test_resolve_cached() {
        let dir = tempfile::tempdir().unwrap();
        let existing = dir.path().join("gitconfig");
        fs::write(&existing, "").unwrap();
        let missing = dir.path().join("missing-gitconfig");
        let resolved = dir.path().join("resolved-gitconfig");

        // (名前, キャッシュの内容, 解決処理を呼ぶか, 期待するパス)
        let cases = [
            ("no cache", None, true, Some(resolved.clone())),
            (
                "fresh cache",
                Some((NOW - 60, Some(existing.clone()))),
                false,
                Some(existing.clone()),
            ),
            (
                "fresh unresolved cache",
                Some((NOW - 60, None)),
                false,
                None,
            ),
            (
                "cached file removed",
                Some((NOW - 60, Some(missing))),
                true,
                Some(resolved.clone()),
            ),
            (
                "stale cache",
                Some((NOW - REFRESH_INTERVAL_SECS, Some(existing.clone()))),
                true,
                Some(resolved.clone()),
            ),
            (
                "cache from the future",
                Some((NOW + 60, Some(existing))),
                true,
                Some(resolved.clone()),
            ),
        ];

        for (name, cache, expect_resolve, expected) in cases {
            let cache_path = dir.path().join(format!("{name}.json"));
            if let Some((resolved_at, path)) = cache {
                SystemConfigCache { resolved_at, path }
                    .save(&cache_path)
                    .unwrap();
            }
            let called = Cell::new(false);

            let (path, save_error) = resolve_cached(&cache_path, NOW, || {
                called.set(true);
                Some(resolved.clone())
            });

            assert_eq!(path, expected, "{name}");
            assert!(save_error.is_none(), "{name}: {save_error:?}");
            assert_eq!(called.get(), expect_resolve, "{name}");
            if expect_resolve {
                assert_eq!(
                    SystemConfigCache::load(&cache_path),
                    Some(SystemConfigCache {
                        resolved_at: NOW,
                        path: Some(resolved.clone()),
                    }),
                    "{name}"
                );
            }
        }
    }

    #[test]
    fn test_resolve_cached_reports_save_error_and_resolves_every_time() {
        let dir = tempfile::tempdir().unwrap();
        // 親がファイルのため、キャッシュのディレクトリを作れない
        let blocker = dir.path().join("not-a-directory");
        fs::write(&blocker, "").unwrap();
        let cache_path = blocker.join("git_system_config.json");
        let resolved = dir.path().join("resolved-gitconfig");
        let calls = Cell::new(0);

        for _ in 0..2 {
            let (path, save_error) = resolve_cached(&cache_path, NOW, || {
                calls.set(calls.get() + 1);
                Some(resolved.clone())
            });

            assert_eq!(path, Some(resolved.clone()));
            assert!(save_error.is_some());
        }
        assert_eq!(calls.get(), 2);
    }
}
