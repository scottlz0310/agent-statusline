//! Codex CLI Custom status line 用 stdio MCP サーバーエントリポイント。

pub mod cache;
pub mod protocol;

use std::io;
use std::path::Path;

use serde_json::Value;

use crate::adapter::codex::CodexAdapter;
use crate::engine::formatter::collection_needs;
use crate::engine::ui_tree::{UiTree, render_ui_tree};
use crate::mcp::cache::{ConfigCache, GitStatusCache, ReviewSummaryCache};
use crate::mcp::protocol::{CodexMcpHandler, StatuslineRenderer, serve_stdio};
use crate::model::review_summary::RepoReviews;
use crate::updater;

/// Codex スナップショットから `UiTree` を描画する常駐レンダラー
pub struct CodexRenderer {
    config: ConfigCache,
    git: GitStatusCache,
    reviews: ReviewSummaryCache,
}

impl CodexRenderer {
    pub fn new(config_path: Option<&Path>) -> Self {
        Self {
            config: ConfigCache::new(config_path),
            git: GitStatusCache::new(),
            reviews: ReviewSummaryCache::new(),
        }
    }

    #[cfg(test)]
    fn with_git_cache(config_path: Option<&Path>, git: GitStatusCache) -> Self {
        Self {
            config: ConfigCache::new(config_path),
            git,
            reviews: ReviewSummaryCache::new(),
        }
    }
}

impl StatuslineRenderer for CodexRenderer {
    fn render_snapshot(&mut self, snapshot: &Value) -> UiTree {
        let (state, _) = CodexAdapter::parse_value(snapshot);
        let config = self.config.get();
        let needs = collection_needs(config);

        let git = needs
            .git
            .then(|| self.git.get(&state.cwd, needs.git_query))
            .flatten();

        let reviews = if needs.review {
            self.reviews.reviews_for(
                &config.integrations.squirrel_notifier,
                git.as_ref().and_then(|g| g.remote_repo.as_deref()),
            )
        } else {
            RepoReviews::default()
        };

        render_ui_tree(config, &state, git.as_ref(), &reviews)
    }
}

/// `agent-statusline mcp codex` サブコマンド本体
pub fn run_codex_stdio(config_path: Option<&Path>) -> io::Result<()> {
    // 常駐サーバー起動時に 1 回だけバックグラウンド更新チェックを行う（resources/read を遅延させない）
    updater::check_update_background_if_needed();

    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut handler = CodexMcpHandler::new(CodexRenderer::new(config_path));
    serve_stdio(stdin.lock(), stdout.lock(), &mut handler)
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;
    use std::sync::Arc;
    use std::time::Duration;

    use super::*;
    use crate::modules::git::GitStatus;

    #[test]
    fn test_codex_mcp_end_to_end_over_stdio() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("config.toml");
        let summary_path = dir.path().join("statusline-summary.json");
        std::fs::write(
            &summary_path,
            r#"{"schemaVersion":1,"queue":{"totalWaiting":0,"items":[]},"activeReviews":[{"repository":"owner/repo","prNumber":99,"round":2,"agent":"claude"}]}"#,
        )
        .unwrap();
        std::fs::write(
            &config_path,
            format!(
                "[integrations.squirrel_notifier]\nenabled = true\nsummary_path = {:?}\n",
                summary_path.to_string_lossy()
            ),
        )
        .unwrap();

        let git_cache = GitStatusCache::with_fetcher(
            Arc::new(|_, _| GitStatus {
                branch: Some("main".to_string()),
                is_dirty: true,
                remote_repo: Some("owner/repo".to_string()),
            }),
            Duration::from_secs(5),
            Duration::from_millis(200),
        );

        let renderer = CodexRenderer::with_git_cache(Some(&config_path), git_cache);
        let mut handler = CodexMcpHandler::new(renderer);

        let req = r#"{"jsonrpc":"2.0","id":1,"method":"resources/read","params":{"uri":"ui://agent-statusline/footer","_meta":{"com.openai/codex-statusline-state":{"account":{"rate_limits":[{"limit_id":"codex","primary_limit":{"used_percent":15,"window_minutes":300,"resets_at":null}}]},"thread":{"workspace":{"cwd":"/work/myrepo"},"model":{"display_name":"GPT-6.2-Sol","reasoning_level":"medium"},"agent":{"state":"working"},"usage":{"context":{"used_percentage":20,"used_tokens":12000}}}}}}}"#;

        let mut out = Vec::new();
        serve_stdio(Cursor::new(format!("{req}\n")), &mut out, &mut handler).unwrap();

        let reply: Value = serde_json::from_slice(&out).unwrap();
        let text = reply["result"]["contents"][0]["text"].as_str().unwrap();
        assert!(text.contains("myrepo"));
        assert!(text.contains("main"));
        assert!(text.contains("\"type\":\"link\""));
        assert!(text.contains("https://github.com/owner/repo/pull/99"));
        assert!(text.contains("#99"));
        assert!(text.contains(" r2"));
        assert!(text.contains("GPT-6.2-Sol (medium)"));
        assert!(text.contains("Ctx: 20%"));
        assert!(text.contains("5h: 15%"));
        assert!(!text.contains("\"type\":\"row\""));
    }
}
