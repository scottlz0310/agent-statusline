//! Codex 常駐 MCP サーバー向けの軽量キャッシュと非ブロッキング Git ワーカー。
//!
//! Codex の `resources/read` は 1 秒タイムアウトで 1 回でも超過するとポーリングが完全停止する。
//! そのため、設定ファイルとレビューサマリは mtime/サイズ監視でキャッシュし、
//! `gix` による Git 状態取得はバックグラウンドワーカー＋TTL キャッシュで非ブロッキング化する。

use std::collections::{HashMap, HashSet, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime};

use crate::engine::config::{Config, SquirrelNotifierConfig, load_config, resolve_config_path};
use crate::engine::formatter::resolve_review_summary_path;
use crate::model::review_summary::{RepoReviews, ReviewSummary};
use crate::modules::git::{GitQuery, GitStatus, get_git_status};
use crate::modules::review::{load_summary, reviews_for_repo};

/// Git 状態キャッシュの既定 TTL
pub const DEFAULT_GIT_TTL: Duration = Duration::from_secs(3);

/// キャッシュミス時に初回応答で待機する最大時間（Codex 1 秒タイムアウトに対し十分な余裕を確保）
pub const DEFAULT_GIT_INITIAL_WAIT: Duration = Duration::from_millis(80);

/// メモリ無制限増加を防ぐキャッシュ保持リポジトリ上限
const MAX_CACHED_REPOS: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq)]
struct FileStamp {
    path: PathBuf,
    modified: Option<SystemTime>,
    len: u64,
}

impl FileStamp {
    fn of(path: &Path) -> Option<Self> {
        let meta = fs::metadata(path).ok()?;
        Some(Self {
            path: path.to_path_buf(),
            modified: meta.modified().ok(),
            len: meta.len(),
        })
    }
}

/// `config.toml` の mtime/サイズ監視キャッシュ。変更や作成・削除に追従する。
pub struct ConfigCache {
    custom_path: Option<PathBuf>,
    stamp: Option<FileStamp>,
    config: Config,
}

impl ConfigCache {
    pub fn new(custom_path: Option<&Path>) -> Self {
        let mut cache = Self {
            custom_path: custom_path.map(Path::to_path_buf),
            stamp: None,
            config: Config::default(),
        };
        cache.refresh();
        cache
    }

    /// 変更があれば再読込した上で現在の `Config` を返す
    pub fn get(&mut self) -> &Config {
        self.refresh();
        &self.config
    }

    fn refresh(&mut self) {
        let resolved = resolve_config_path(self.custom_path.as_deref());
        let current_stamp = resolved.as_deref().and_then(FileStamp::of);

        if current_stamp == self.stamp {
            return;
        }

        self.config = match resolved.as_deref() {
            Some(path) if current_stamp.is_some() => load_config(Some(path)),
            _ => Config::default(),
        };
        self.stamp = current_stamp;
    }
}

/// `statusline-summary.json` の mtime/サイズ監視キャッシュ
#[derive(Default)]
pub struct ReviewSummaryCache {
    stamp: Option<FileStamp>,
    summary: Option<ReviewSummary>,
}

impl ReviewSummaryCache {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn reviews_for(
        &mut self,
        integration: &SquirrelNotifierConfig,
        repo: Option<&str>,
    ) -> RepoReviews {
        let Some(repo) = repo else {
            return RepoReviews::default();
        };

        let path = resolve_review_summary_path(integration);
        let current_stamp = path.as_deref().and_then(FileStamp::of);

        if current_stamp != self.stamp {
            self.summary = path
                .as_deref()
                .filter(|_| current_stamp.is_some())
                .and_then(load_summary);
            self.stamp = current_stamp;
        }

        let Some(summary) = &self.summary else {
            return RepoReviews::default();
        };
        let (active, waiting) = reviews_for_repo(summary, repo);
        RepoReviews {
            active: active.into_iter().cloned().collect(),
            waiting: waiting.into_iter().cloned().collect(),
        }
    }
}

#[derive(Debug, Clone)]
struct CachedGit {
    status: GitStatus,
    query: GitQuery,
    updated_at: Instant,
}

fn query_covers(cached: GitQuery, requested: GitQuery) -> bool {
    (!requested.dirty || cached.dirty) && (!requested.remote_repo || cached.remote_repo)
}

fn merge_query(a: GitQuery, b: GitQuery) -> GitQuery {
    GitQuery {
        dirty: a.dirty || b.dirty,
        remote_repo: a.remote_repo || b.remote_repo,
    }
}

struct SharedGitState {
    entries: HashMap<PathBuf, CachedGit>,
    pending_order: VecDeque<PathBuf>,
    pending_queries: HashMap<PathBuf, GitQuery>,
    in_flight: HashSet<PathBuf>,
    shutdown: bool,
}

/// Git 状態取得クロージャ型（DI 用）
pub type GitFetcher = Arc<dyn Fn(&Path, GitQuery) -> GitStatus + Send + Sync>;

/// 複数 `cwd` に対応した非ブロッキング Git 状態キャッシュ（バックグラウンド単一ワーカー）
pub struct GitStatusCache {
    shared: Arc<(Mutex<SharedGitState>, Condvar)>,
    ttl: Duration,
    initial_wait: Duration,
    worker: Option<JoinHandle<()>>,
}

impl GitStatusCache {
    pub fn new() -> Self {
        Self::with_fetcher(
            Arc::new(get_git_status),
            DEFAULT_GIT_TTL,
            DEFAULT_GIT_INITIAL_WAIT,
        )
    }

    /// テスト用 DI コンストラクタ（任意の Git 取得関数・TTL・初回待機時間を注入可能）
    pub fn with_fetcher(fetcher: GitFetcher, ttl: Duration, initial_wait: Duration) -> Self {
        let shared = Arc::new((
            Mutex::new(SharedGitState {
                entries: HashMap::new(),
                pending_order: VecDeque::new(),
                pending_queries: HashMap::new(),
                in_flight: HashSet::new(),
                shutdown: false,
            }),
            Condvar::new(),
        ));

        let worker_shared = Arc::clone(&shared);
        let worker = thread::spawn(move || worker_loop(&worker_shared, &fetcher));

        Self {
            shared,
            ttl,
            initial_wait,
            worker: Some(worker),
        }
    }

    /// 指定 `cwd` の Git 状態を取得する。
    ///
    /// - TTL 内の有効キャッシュがあれば即座に返す。
    /// - 期限切れの古いキャッシュがあれば、バックグラウンド更新を予約しつつ古いキャッシュを即座に返す。
    /// - 初回（キャッシュなし）のみ最大 `initial_wait` だけ完了を待ち、間に合わなければ `None` を返す。
    pub fn get(&self, cwd: &Path, query: GitQuery) -> Option<GitStatus> {
        let (lock, cvar) = &*self.shared;
        let mut guard = lock.lock().unwrap_or_else(PoisonError::into_inner);
        let now = Instant::now();

        if let Some(cached) = guard.entries.get(cwd) {
            let fresh = now.saturating_duration_since(cached.updated_at) < self.ttl;
            let covers = query_covers(cached.query, query);
            if fresh && covers {
                return Some(cached.status.clone());
            }
            // 古い値または不足クエリ: 更新をバックグラウンド予約しつつ既存値で即応答
            let target_query = merge_query(cached.query, query);
            let fallback = cached.status.clone();
            enqueue_locked(&mut guard, cwd, target_query, cvar);
            return Some(fallback);
        }

        // 初回ミス: ワーカーへ投入し、短い上限（initial_wait）だけ待機
        enqueue_locked(&mut guard, cwd, query, cvar);
        if self.initial_wait.is_zero() {
            return None;
        }

        let deadline = now + self.initial_wait;
        loop {
            if let Some(cached) = guard.entries.get(cwd) {
                return Some(cached.status.clone());
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return None;
            }
            let (new_guard, wait_res) = cvar
                .wait_timeout(guard, remaining)
                .unwrap_or_else(PoisonError::into_inner);
            guard = new_guard;
            if wait_res.timed_out() {
                return guard.entries.get(cwd).map(|c| c.status.clone());
            }
        }
    }
}

impl Drop for GitStatusCache {
    fn drop(&mut self) {
        let (lock, cvar) = &*self.shared;
        {
            let mut guard = lock.lock().unwrap_or_else(PoisonError::into_inner);
            guard.shutdown = true;
            guard.pending_order.clear();
            guard.pending_queries.clear();
            cvar.notify_all();
        }
        // 長時間ブロック中の fetcher を待って終了を遅延させないよう detach する
        let _ = self.worker.take();
    }
}

fn enqueue_locked(state: &mut SharedGitState, cwd: &Path, query: GitQuery, cvar: &Condvar) {
    if let Some(existing) = state.pending_queries.get_mut(cwd) {
        *existing = merge_query(*existing, query);
        return;
    }
    if state.in_flight.contains(cwd) {
        // 実行中に再要求が来たら次周用にキューへ積む
        let key = cwd.to_path_buf();
        state.pending_order.push_back(key.clone());
        state.pending_queries.insert(key, query);
        return;
    }
    let key = cwd.to_path_buf();
    state.pending_order.push_back(key.clone());
    state.pending_queries.insert(key, query);
    cvar.notify_all();
}

fn worker_loop(shared: &(Mutex<SharedGitState>, Condvar), fetcher: &GitFetcher) {
    let (lock, cvar) = shared;
    loop {
        let (cwd, query) = {
            let mut guard = lock.lock().unwrap_or_else(PoisonError::into_inner);
            loop {
                if guard.shutdown {
                    return;
                }
                if let Some(next_cwd) = guard.pending_order.pop_front() {
                    if let Some(next_query) = guard.pending_queries.remove(&next_cwd) {
                        guard.in_flight.insert(next_cwd.clone());
                        break (next_cwd, next_query);
                    }
                    continue;
                }
                guard = cvar.wait(guard).unwrap_or_else(PoisonError::into_inner);
            }
        };

        let status = fetcher(&cwd, query);
        let updated_at = Instant::now();

        let mut guard = lock.lock().unwrap_or_else(PoisonError::into_inner);
        guard.in_flight.remove(&cwd);
        if !guard.entries.contains_key(&cwd) && guard.entries.len() >= MAX_CACHED_REPOS {
            evict_oldest(&mut guard.entries);
        }
        guard.entries.insert(
            cwd,
            CachedGit {
                status,
                query,
                updated_at,
            },
        );
        cvar.notify_all();
    }
}

fn evict_oldest(entries: &mut HashMap<PathBuf, CachedGit>) {
    if let Some(oldest_key) = entries
        .iter()
        .min_by_key(|(_, v)| v.updated_at)
        .map(|(k, _)| k.clone())
    {
        entries.remove(&oldest_key);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    #[test]
    fn test_config_cache_tracks_create_modify_and_delete() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");

        let mut cache = ConfigCache::new(Some(&path));
        assert_eq!(cache.get().model.symbol, "🤖 ");

        // 新規作成を検知
        fs::write(&path, "[model]\nsymbol = \"M1 \"\n").unwrap();
        assert_eq!(cache.get().model.symbol, "M1 ");

        // 内容変更（サイズ変化）を検知
        fs::write(&path, "[model]\nsymbol = \"MODEL-2 \"\n").unwrap();
        assert_eq!(cache.get().model.symbol, "MODEL-2 ");

        // 削除時は既定値へ復帰
        fs::remove_file(&path).unwrap();
        assert_eq!(cache.get().model.symbol, "🤖 ");
    }

    #[test]
    fn test_review_summary_cache_tracks_updates_and_removal() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("statusline-summary.json");
        let integration = SquirrelNotifierConfig {
            enabled: true,
            output_dir: None,
            summary_path: Some(path.to_string_lossy().to_string()),
        };

        let mut cache = ReviewSummaryCache::new();
        assert!(
            cache
                .reviews_for(&integration, Some("o/r"))
                .active
                .is_empty()
        );

        fs::write(
            &path,
            r#"{"schemaVersion":1,"queue":{"totalWaiting":0,"items":[]},"activeReviews":[{"repository":"o/r","prNumber":10,"round":1,"agent":"claude"}]}"#,
        )
        .unwrap();
        let r1 = cache.reviews_for(&integration, Some("o/r"));
        assert_eq!(r1.active.len(), 1);
        assert_eq!(r1.active[0].pr_number, 10);

        // ファイル削除時は空へ戻る
        fs::remove_file(&path).unwrap();
        assert!(
            cache
                .reviews_for(&integration, Some("o/r"))
                .active
                .is_empty()
        );
    }

    #[test]
    fn test_git_cache_returns_fast_on_slow_fetcher_and_populates_async() {
        let calls = Arc::new(AtomicUsize::new(0));
        let calls_ref = Arc::clone(&calls);
        let fetcher: GitFetcher = Arc::new(move |cwd, query| {
            calls_ref.fetch_add(1, Ordering::SeqCst);
            thread::sleep(Duration::from_millis(120));
            GitStatus {
                branch: Some(format!(
                    "{}:dirty={}",
                    cwd.file_name().unwrap().to_string_lossy(),
                    query.dirty
                )),
                is_dirty: query.dirty,
                remote_repo: None,
            }
        });

        let cache = GitStatusCache::with_fetcher(
            fetcher,
            Duration::from_secs(5),
            Duration::from_millis(20),
        );

        let cwd = Path::new("/repo/slow-one");
        let query = GitQuery {
            dirty: true,
            remote_repo: false,
        };

        // 1 回目: 120ms かかる fetcher に対し initial_wait (20ms) で素早く None を返す
        let t0 = Instant::now();
        let first = cache.get(cwd, query);
        assert!(first.is_none());
        assert!(
            t0.elapsed() < Duration::from_millis(250),
            "should not block for full slow fetch"
        );

        // バックグラウンド完了後にキャッシュが埋まる
        thread::sleep(Duration::from_millis(160));
        let second = cache.get(cwd, query).expect("populated in background");
        assert_eq!(second.branch.as_deref(), Some("slow-one:dirty=true"));
        assert!(second.is_dirty);

        // TTL 内は再呼び出しされない
        let third = cache.get(cwd, query).expect("served from TTL cache");
        assert_eq!(third.branch.as_deref(), Some("slow-one:dirty=true"));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn test_git_cache_returns_immediately_on_fast_fetcher_and_handles_multiple_cwds() {
        let fetcher: GitFetcher = Arc::new(|cwd, _| GitStatus {
            branch: Some(cwd.file_name().unwrap().to_string_lossy().to_string()),
            is_dirty: false,
            remote_repo: None,
        });

        let cache = GitStatusCache::with_fetcher(
            fetcher,
            Duration::from_secs(5),
            Duration::from_millis(200),
        );

        let r1 = cache
            .get(Path::new("/work/repo-a"), GitQuery::default())
            .expect("fast repo-a");
        let r2 = cache
            .get(Path::new("/work/repo-b"), GitQuery::default())
            .expect("fast repo-b");

        assert_eq!(r1.branch.as_deref(), Some("repo-a"));
        assert_eq!(r2.branch.as_deref(), Some("repo-b"));
    }

    #[test]
    fn test_git_cache_refreshes_when_query_widens() {
        let calls = Arc::new(AtomicUsize::new(0));
        let calls_ref = Arc::clone(&calls);
        let fetcher: GitFetcher = Arc::new(move |_, query| {
            calls_ref.fetch_add(1, Ordering::SeqCst);
            GitStatus {
                branch: Some("main".to_string()),
                is_dirty: query.dirty,
                remote_repo: query.remote_repo.then(|| "owner/repo".to_string()),
            }
        });

        let cache = GitStatusCache::with_fetcher(
            fetcher,
            Duration::from_secs(10),
            Duration::from_millis(200),
        );
        let cwd = Path::new("/work/widening");

        let narrow = cache
            .get(
                cwd,
                GitQuery {
                    dirty: false,
                    remote_repo: false,
                },
            )
            .expect("narrow");
        assert!(!narrow.is_dirty);
        assert!(narrow.remote_repo.is_none());

        // より広いクエリを要求すると既存キャッシュで即応答しつつバックグラウンド更新が走る
        let _ = cache.get(
            cwd,
            GitQuery {
                dirty: true,
                remote_repo: true,
            },
        );
        thread::sleep(Duration::from_millis(80));

        let wide = cache
            .get(
                cwd,
                GitQuery {
                    dirty: true,
                    remote_repo: true,
                },
            )
            .expect("wide");
        assert!(wide.is_dirty);
        assert_eq!(wide.remote_repo.as_deref(), Some("owner/repo"));
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }
}
