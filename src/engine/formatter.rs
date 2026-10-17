use crate::engine::config::{
    AgentStateConfig, Config, ContextConfig, DirectoryConfig, GitBranchConfig, GitStatusConfig,
    ModelConfig, PlanConfig, QuotaConfig, ReviewConfig, SandboxConfig, SquirrelNotifierConfig,
};
use crate::engine::parts::{PartInput, find, part_ids};
use crate::engine::style::{pct_style_name, render_styled_text};
use crate::engine::template::{Token, expand, tokenize};
use crate::model::review_summary::{ActiveReview, QueuedReview, RepoReviews};
use crate::model::state::StatuslineState;
use crate::modules::context::format_tokens;
use crate::modules::directory::{TruncationMode, format_path};
use crate::modules::git::{GitQuery, GitStatus, get_git_status};
use crate::modules::quota::format_reset_seconds;
use crate::modules::review::{default_summary_path, load_summary, reviews_for_repo};
use crate::sink::notifier::expand_env_path;

pub fn render_directory(
    cfg: &DirectoryConfig,
    state: &StatuslineState,
    terminal_width: usize,
) -> String {
    if cfg.disabled {
        return String::new();
    }
    let folder = state
        .cwd
        .file_name()
        .map_or_else(|| ".".to_string(), |f| f.to_string_lossy().to_string());
    // max_length を指定したときは、端末の幅の取得に左右されないよう、幅を見ない
    let path_max = if cfg.max_length > 0 {
        cfg.max_length
    } else {
        terminal_width / 2
    };
    // 認識できない値でも描画は止めない（保存時の検証が、誤りを知らせる）
    let mode = TruncationMode::parse(&cfg.truncation_mode).unwrap_or(TruncationMode::Middle);
    let short_path = format_path(&state.cwd, path_max, mode);

    expand(
        &cfg.format,
        &[
            ("symbol", &cfg.symbol),
            ("folder_style", &cfg.folder_style),
            ("folder", &folder),
            ("path_style", &cfg.path_style),
            ("path", &short_path),
        ],
    )
}

pub fn render_git_branch(cfg: &GitBranchConfig, git: Option<&GitStatus>) -> String {
    if cfg.disabled {
        return String::new();
    }
    let Some(git) = git else {
        return String::new();
    };
    let Some(branch) = &git.branch else {
        return String::new();
    };

    expand(
        &cfg.format,
        &[
            ("symbol", &cfg.symbol),
            ("branch", branch),
            ("style", &cfg.style),
        ],
    )
}

pub fn render_git_status(cfg: &GitStatusConfig, git: Option<&GitStatus>) -> String {
    if cfg.disabled {
        return String::new();
    }
    let Some(git) = git else {
        return String::new();
    };
    if !git.is_dirty {
        return String::new();
    }

    expand(
        &cfg.format,
        &[("status", &cfg.modified), ("style", &cfg.style)],
    )
}

pub fn render_sandbox(cfg: &SandboxConfig, state: &StatuslineState) -> String {
    if cfg.disabled || !state.sandbox_enabled {
        return String::new();
    }

    expand(
        &cfg.format,
        &[("symbol", &cfg.symbol), ("style", &cfg.style)],
    )
}

pub fn render_model(cfg: &ModelConfig, state: &StatuslineState) -> String {
    if cfg.disabled {
        return String::new();
    }
    let Some(model) = &state.model else {
        return String::new();
    };

    expand(
        &cfg.format,
        &[
            ("symbol", &cfg.symbol),
            ("model", model),
            ("style", &cfg.style),
        ],
    )
}

pub fn render_context(cfg: &ContextConfig, state: &StatuslineState) -> String {
    if cfg.disabled {
        return String::new();
    }
    let Some(ctx) = state.context_used_percentage else {
        return String::new();
    };

    let total_tokens =
        state.total_input_tokens.unwrap_or(0) + state.total_output_tokens.unwrap_or(0);
    let tokens = format_tokens(total_tokens);

    let pct_style = pct_style_name(
        ctx,
        &cfg.style_normal,
        &cfg.style_warning,
        &cfg.style_critical,
        cfg.threshold_warning,
        cfg.threshold_critical,
    );

    expand(
        &cfg.format,
        &[
            ("symbol", &cfg.symbol),
            ("percentage", &format!("{ctx}%")),
            ("pct_style", pct_style),
            ("tokens", &tokens),
            ("tok_style", &cfg.tok_style),
        ],
    )
}

pub fn render_agent_state(cfg: &AgentStateConfig, state: &StatuslineState) -> String {
    if cfg.disabled {
        return String::new();
    }
    let Some(agent_state) = &state.agent_state else {
        return String::new();
    };
    if agent_state == "idle" {
        return String::new();
    }

    expand(
        &cfg.format,
        &[
            ("symbol", &cfg.symbol),
            ("state", agent_state),
            ("style", &cfg.style),
        ],
    )
}

pub fn render_plan(cfg: &PlanConfig, state: &StatuslineState) -> String {
    if cfg.disabled {
        return String::new();
    }
    let Some(plan_tier) = &state.plan_tier else {
        return String::new();
    };

    expand(&cfg.format, &[("tier", plan_tier), ("style", &cfg.style)])
}

pub fn render_quota(cfg: &QuotaConfig, state: &StatuslineState) -> String {
    if cfg.disabled || state.quotas.is_empty() {
        return String::new();
    }

    let mut items = Vec::new();
    for q in &state.quotas {
        let Some(pct) = q.used_percentage else {
            continue;
        };

        if cfg.hide_zero && pct == 0 {
            continue;
        }

        let style = pct_style_name(
            pct,
            &cfg.style_normal,
            &cfg.style_warning,
            &cfg.style_critical,
            cfg.threshold_warning,
            cfg.threshold_critical,
        );

        // リセット時刻が不明なときは、`(rst $reset_time)` の括りごと取り除く
        let (format, reset_time) = match q.reset_in_seconds {
            Some(reset_sec) => (cfg.format.clone(), format_reset_seconds(reset_sec)),
            None => (
                cfg.format
                    .replace(" (rst $reset_time)", "")
                    .replace("(rst $reset_time)", ""),
                String::new(),
            ),
        };

        items.push(expand(
            &format,
            &[
                ("symbol", &cfg.symbol),
                ("label", &q.label),
                ("percentage", &format!("{pct}%")),
                ("style", style),
                ("reset_time", &reset_time),
            ],
        ));
    }

    if items.is_empty() {
        String::new()
    } else {
        items.join(&cfg.separator)
    }
}

pub fn render_review(
    cfg: &ReviewConfig,
    active: &[&ActiveReview],
    waiting: &[&QueuedReview],
) -> String {
    if cfg.disabled || (active.is_empty() && waiting.is_empty()) {
        return String::new();
    }

    let items = active
        .iter()
        .map(|r| {
            expand(
                &cfg.active_format,
                &[
                    ("pr", &r.pr_number.to_string()),
                    ("round", &r.round.to_string()),
                    ("agent", r.agent.as_deref().unwrap_or("")),
                ],
            )
        })
        .chain(waiting.iter().map(|r| {
            expand(
                &cfg.waiting_format,
                &[
                    ("pr", &r.pr_number.to_string()),
                    ("round", &r.round.to_string()),
                    ("reason", &r.reason),
                ],
            )
        }))
        .collect::<Vec<_>>()
        .join(&cfg.separator);

    expand(
        &cfg.format,
        &[
            ("symbol", &cfg.symbol),
            ("items", &items),
            ("style", &cfg.style),
        ],
    )
}

/// `format` が参照している部品 ID（既知の部品 ID の最長一致）
fn used_parts(format: &str) -> Vec<&str> {
    tokenize(format, &part_ids())
        .into_iter()
        .filter_map(|token| match token {
            Token::Var(id) => Some(id),
            Token::Text(_) => None,
        })
        .collect()
}

/// 収集済みの入力だけからステータスラインを合成する純関数。Git・ファイル・環境変数には触れない
pub fn compose(
    config: &Config,
    state: &StatuslineState,
    git: Option<&GitStatus>,
    reviews: &RepoReviews,
    terminal_width: usize,
) -> String {
    let input = PartInput {
        state,
        git,
        reviews,
        terminal_width,
    };
    let rendered: Vec<(&str, String)> = used_parts(&config.format)
        .into_iter()
        .filter_map(|id| find(id))
        .map(|part| (part.id, (part.render)(config, &input)))
        .collect();
    let vars: Vec<(&str, &str)> = rendered
        .iter()
        .map(|(id, value)| (*id, value.as_str()))
        .collect();

    let mut output_lines = Vec::new();

    for line in config.format.lines() {
        let filled = expand(line, &vars);
        let trimmed = filled.trim_end();
        if !trimmed.is_empty() {
            output_lines.push(render_styled_text(trimmed));
        }
    }

    output_lines.join("\n")
}

/// Git 状態とレビュー状態を、`format` が使う部品に必要な分だけ収集する
pub struct Collected {
    pub git: Option<GitStatus>,
    pub reviews: RepoReviews,
}

/// Squirrel Notifier のサマリからカレントリポジトリ分のレビュー状態を読み込む
fn load_repo_reviews(integration: &SquirrelNotifierConfig, repo: Option<&str>) -> RepoReviews {
    let Some(repo) = repo else {
        return RepoReviews::default();
    };
    let path = integration
        .summary_path
        .as_deref()
        .map(expand_env_path)
        .or_else(default_summary_path);
    let Some(summary) = path.as_deref().and_then(load_summary) else {
        return RepoReviews::default();
    };
    let (active, waiting) = reviews_for_repo(&summary, repo);
    RepoReviews {
        active: active.into_iter().cloned().collect(),
        waiting: waiting.into_iter().cloned().collect(),
    }
}

/// 副作用のある入力（Git のリポジトリ読み取り、サマリ JSON の読み込み）を集める
pub fn collect(config: &Config, state: &StatuslineState) -> Collected {
    let parts = used_parts(&config.format);
    let uses = |id: &str| parts.contains(&id);

    let needs_branch = uses("git_branch") && !config.git_branch.disabled;
    let needs_dirty = uses("git_status") && !config.git_status.disabled;
    let needs_review =
        uses("review") && !config.review.disabled && config.integrations.squirrel_notifier.enabled;

    let git = (needs_branch || needs_dirty || needs_review).then(|| {
        get_git_status(
            &state.cwd,
            GitQuery {
                dirty: needs_dirty,
                remote_repo: needs_review,
            },
        )
    });

    let reviews = if needs_review {
        load_repo_reviews(
            &config.integrations.squirrel_notifier,
            git.as_ref().and_then(|g| g.remote_repo.as_deref()),
        )
    } else {
        RepoReviews::default()
    };

    Collected { git, reviews }
}

/// 設定テンプレートに従ってステータスラインを描画する（収集してから合成する）
pub fn render_template(config: &Config, state: &StatuslineState, terminal_width: usize) -> String {
    let collected = collect(config, state);
    compose(
        config,
        state,
        collected.git.as_ref(),
        &collected.reviews,
        terminal_width,
    )
}

/// デフォルトの 3 行ステータスライン描画
#[allow(dead_code)]
#[must_use]
pub fn render_default(state: &StatuslineState, terminal_width: usize) -> String {
    let config = Config::default();
    render_template(&config, state, terminal_width)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::state::QuotaItem;
    use std::path::PathBuf;

    #[test]
    fn test_render_default_output() {
        let state = StatuslineState {
            cwd: PathBuf::from("C:\\Users\\jojob\\src\\myproject"),
            model: Some("Gemini 3.8 Flash".to_string()),
            context_used_percentage: Some(15),
            total_input_tokens: Some(50000),
            total_output_tokens: Some(12000),
            agent_state: Some("working".to_string()),
            sandbox_enabled: true,
            plan_tier: Some("Google AI Pro".to_string()),
            quotas: vec![QuotaItem {
                id: "gemini-5h".to_string(),
                label: "Gemini 5h".to_string(),
                used_percentage: Some(5),
                reset_in_seconds: Some(3600),
                reset_time_iso: None,
            }],
        };

        let config = Config::default();
        let rendered = render_template(&config, &state, 120);
        let lines: Vec<&str> = rendered.lines().collect();

        assert_eq!(lines.len(), 3);
        assert!(lines[0].contains("myproject"));
        assert!(lines[0].contains("sandbox"));
        assert!(lines[1].contains("Gemini 3.8 Flash"));
        assert!(lines[1].contains("Ctx: 15%"));
        assert!(lines[1].contains("62k tok"));
        assert!(lines[1].contains("working"));
        assert!(lines[1].contains("Google AI Pro"));
        assert!(lines[2].contains("Gemini 5h:"));
        assert!(lines[2].contains("5%"));
        assert!(lines[2].contains("rst 1h0m"));
    }

    #[test]
    fn test_render_template_custom_layout() {
        let config = Config {
            format: "$model\n$directory".to_string(),
            model: crate::engine::config::ModelConfig {
                format: "MODEL: [$model]($style)".to_string(),
                style: "cyan".to_string(),
                ..Default::default()
            },
            ..Default::default()
        };

        let state = StatuslineState {
            cwd: PathBuf::from("/home/user/project"),
            model: Some("Claude 3.7 Sonnet".to_string()),
            context_used_percentage: None,
            total_input_tokens: None,
            total_output_tokens: None,
            agent_state: None,
            sandbox_enabled: false,
            plan_tier: None,
            quotas: Vec::new(),
        };

        let rendered = render_template(&config, &state, 80);
        let lines: Vec<&str> = rendered.lines().collect();

        assert_eq!(lines.len(), 2);
        assert!(lines[0].contains("MODEL: "));
        assert!(lines[0].contains("Claude 3.7 Sonnet"));
        assert!(lines[1].contains("project"));
    }

    #[test]
    fn test_render_with_disabled_modules() {
        let mut config = Config::default();
        config.directory.disabled = true;
        config.sandbox.disabled = true;
        config.model.disabled = true;

        let state = StatuslineState {
            cwd: PathBuf::from("C:\\Users\\dev\\project"),
            model: Some("Gemini Flash".to_string()),
            context_used_percentage: Some(10),
            total_input_tokens: Some(1000),
            total_output_tokens: Some(500),
            agent_state: None,
            sandbox_enabled: true,
            plan_tier: None,
            quotas: Vec::new(),
        };

        let rendered = render_template(&config, &state, 80);
        assert!(!rendered.contains("project"));
        assert!(!rendered.contains("sandbox"));
        assert!(!rendered.contains("Gemini Flash"));
        assert!(rendered.contains("Ctx: 10%"));
    }

    #[test]
    fn test_render_quota_multiple_items_and_threshold_colors() {
        let mut config = Config::default();
        config.quota.separator = " --- ".to_string();

        let state = StatuslineState {
            cwd: PathBuf::from("C:\\Users\\dev\\project"),
            model: None,
            context_used_percentage: None,
            total_input_tokens: None,
            total_output_tokens: None,
            agent_state: None,
            sandbox_enabled: false,
            plan_tier: None,
            quotas: vec![
                QuotaItem {
                    id: "q1".to_string(),
                    label: "Quota 1".to_string(),
                    used_percentage: Some(30), // normal -> green (\x1b[32m)
                    reset_in_seconds: Some(120),
                    reset_time_iso: None,
                },
                QuotaItem {
                    id: "q2".to_string(),
                    label: "Quota 2".to_string(),
                    used_percentage: Some(85), // critical -> red (\x1b[31m)
                    reset_in_seconds: None,
                    reset_time_iso: None,
                },
            ],
        };

        let rendered = render_template(&config, &state, 120);
        assert!(rendered.contains("⏳ Quota 1: 30% (rst 2m)"));
        assert!(rendered.contains(" --- "));
        assert!(rendered.contains("⏳ Quota 2: 85%"));
        // Quota 2 は reset_in_seconds が None なので (rst ) が含まれないこと
        assert!(!rendered.contains("Quota 2: 85% (rst"));
        // ANSI カラーコードが付与されていること
        assert!(rendered.contains("\x1b[32m")); // green
        assert!(rendered.contains("\x1b[31m")); // red
    }

    #[test]
    fn test_render_quota_hide_zero() {
        let mut config = Config::default();
        config.quota.hide_zero = true;

        let state = StatuslineState {
            cwd: PathBuf::from("."),
            model: None,
            context_used_percentage: None,
            total_input_tokens: None,
            total_output_tokens: None,
            agent_state: None,
            sandbox_enabled: false,
            plan_tier: None,
            quotas: vec![
                QuotaItem {
                    id: "q1".to_string(),
                    label: "Quota 1".to_string(),
                    used_percentage: Some(0),
                    reset_in_seconds: Some(120),
                    reset_time_iso: None,
                },
                QuotaItem {
                    id: "q2".to_string(),
                    label: "Quota 2".to_string(),
                    used_percentage: Some(15),
                    reset_in_seconds: None,
                    reset_time_iso: None,
                },
            ],
        };

        let rendered = render_quota(&config.quota, &state);
        assert!(!rendered.contains("Quota 1"));
        assert!(rendered.contains("Quota 2: 15%"));

        // 全て 0% の場合は空文字列になること
        let state_all_zero = StatuslineState {
            quotas: vec![QuotaItem {
                id: "q1".to_string(),
                label: "Quota 1".to_string(),
                used_percentage: Some(0),
                reset_in_seconds: Some(120),
                reset_time_iso: None,
            }],
            ..state
        };
        assert_eq!(render_quota(&config.quota, &state_all_zero), "");
    }

    #[test]
    fn test_render_idle_agent_state_is_skipped() {
        let config = Config::default();
        let state = StatuslineState {
            cwd: PathBuf::from("C:\\Users\\dev\\project"),
            model: None,
            context_used_percentage: None,
            total_input_tokens: None,
            total_output_tokens: None,
            agent_state: Some("idle".to_string()),
            sandbox_enabled: false,
            plan_tier: None,
            quotas: Vec::new(),
        };

        let rendered = render_template(&config, &state, 80);
        assert!(!rendered.contains("idle"));
    }

    fn active(pr_number: u64, round: u32, agent: Option<&str>) -> ActiveReview {
        ActiveReview {
            repository: "o/r".to_string(),
            pr_number,
            round,
            agent: agent.map(str::to_string),
        }
    }

    fn waiting(pr_number: u64, round: u32, reason: &str) -> QueuedReview {
        QueuedReview {
            repository: "o/r".to_string(),
            pr_number,
            round,
            reason: reason.to_string(),
        }
    }

    #[test]
    fn test_render_review() {
        let plain = ReviewConfig {
            format: "$symbol$items".to_string(),
            ..Default::default()
        };
        let custom = ReviewConfig {
            format: "$items".to_string(),
            active_format: "$pr:$agent".to_string(),
            waiting_format: "$pr:$reason".to_string(),
            separator: ",".to_string(),
            ..Default::default()
        };
        let disabled = ReviewConfig {
            disabled: true,
            ..Default::default()
        };
        let a = [active(31, 2, Some("claude")), active(33, 1, None)];
        let w = [waiting(32, 1, "opened")];
        let cases = [
            (
                "both",
                &plain,
                vec![&a[0]],
                vec![&w[0]],
                "🐿 🔍#31 r2 ⏳#32 r1",
            ),
            ("waiting-only", &plain, vec![], vec![&w[0]], "🐿 ⏳#32 r1"),
            ("empty", &plain, vec![], vec![], ""),
            (
                "custom",
                &custom,
                vec![&a[0], &a[1]],
                vec![&w[0]],
                "31:claude,33:,32:opened",
            ),
            ("disabled", &disabled, vec![&a[0]], vec![&w[0]], ""),
        ];
        for (name, cfg, active, waiting, expected) in cases {
            assert_eq!(render_review(cfg, &active, &waiting), expected, "{name}");
        }
    }

    #[test]
    fn test_load_repo_reviews_reads_summary_path() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("statusline-summary.json");
        std::fs::write(
            &path,
            r#"{"schemaVersion":1,"queue":{"totalWaiting":1,"items":[
                {"repository":"Owner/Repo","prNumber":7,"round":1,"reason":"opened"}]},
                "activeReviews":[]}"#,
        )
        .unwrap();
        let cfg = ReviewConfig {
            format: "$items".to_string(),
            ..Default::default()
        };
        let integration = SquirrelNotifierConfig {
            summary_path: Some(path.to_string_lossy().to_string()),
            ..Default::default()
        };
        let missing = SquirrelNotifierConfig {
            summary_path: Some(dir.path().join("none.json").to_string_lossy().to_string()),
            ..Default::default()
        };
        let cases = [
            ("match", &integration, Some("owner/repo"), "⏳#7 r1"),
            ("other-repo", &integration, Some("owner/other"), ""),
            ("no-remote", &integration, None, ""),
            ("no-summary", &missing, Some("owner/repo"), ""),
        ];
        for (name, integration, repo, expected) in cases {
            let reviews = load_repo_reviews(integration, repo);
            let active: Vec<&ActiveReview> = reviews.active.iter().collect();
            let waiting: Vec<&QueuedReview> = reviews.waiting.iter().collect();
            assert_eq!(render_review(&cfg, &active, &waiting), expected, "{name}");
        }
    }

    fn compose_state() -> StatuslineState {
        StatuslineState {
            cwd: PathBuf::from("/nonexistent/dev/myproject"),
            model: Some("Claude Opus".to_string()),
            ..Default::default()
        }
    }

    fn compose_git(branch: &str, is_dirty: bool) -> GitStatus {
        GitStatus {
            branch: Some(branch.to_string()),
            is_dirty,
            remote_repo: None,
        }
    }

    #[test]
    fn test_compose_uses_only_given_inputs() {
        let mut config = Config {
            format: "$git_branch$git_status\n$review".to_string(),
            ..Default::default()
        };
        // 収集済みの入力だけで描画できる。存在しない cwd・サマリを指していても、読みに行かない
        config.integrations.squirrel_notifier.summary_path =
            Some("/nonexistent/statusline-summary.json".to_string());
        let reviews = RepoReviews {
            active: vec![active(31, 2, None)],
            waiting: vec![waiting(32, 1, "opened")],
        };
        let state = compose_state();
        let dirty = compose_git("main", true);
        let clean = compose_git("main", false);
        let cases = [
            ("dirty", Some(&dirty), "🌿 main ✗\n🐿 🔍#31 r2 ⏳#32 r1"),
            ("clean", Some(&clean), "🌿 main\n🐿 🔍#31 r2 ⏳#32 r1"),
            ("no-git", None, "🐿 🔍#31 r2 ⏳#32 r1"),
        ];
        for (name, git, expected) in cases {
            let rendered = compose(&config, &state, git, &reviews, 80);
            assert_eq!(strip_ansi(&rendered), expected, "{name}");
        }

        config.integrations.squirrel_notifier.enabled = false;
        let rendered = compose(&config, &state, Some(&dirty), &reviews, 80);
        assert_eq!(strip_ansi(&rendered), "🌿 main ✗", "integration-disabled");
    }

    #[test]
    fn test_compose_does_not_reexpand_values() {
        let config = Config {
            format: "$git_branch $model".to_string(),
            git_branch: GitBranchConfig {
                format: "$symbol$branch".to_string(),
                symbol: "$model:".to_string(),
                ..Default::default()
            },
            ..Default::default()
        };
        let git = compose_git("$model", false);
        let rendered = compose(
            &config,
            &compose_state(),
            Some(&git),
            &RepoReviews::default(),
            80,
        );
        assert_eq!(strip_ansi(&rendered), "$model:$model 🤖 Claude Opus");
    }

    #[test]
    fn test_render_directory_truncation_mode() {
        let state = StatuslineState {
            cwd: PathBuf::from(
                "/home/dev/src/very/deeply/nested/workspace/directory/for/the/project",
            ),
            ..Default::default()
        };
        let cfg_with = |mode: &str| DirectoryConfig {
            format: "$path".to_string(),
            truncation_mode: mode.to_string(),
            ..Default::default()
        };
        // 幅 80 での省略後の最大幅は 40
        let cases = [
            ("left", ".../workspace/directory/for/the/project"),
            ("middle", "/home/dev/src/very/.../y/for/the/project"),
            // 認識できない値でも描画は止まらず、既定（middle）と同じになる
            ("right", "/home/dev/src/very/.../y/for/the/project"),
            ("", "/home/dev/src/very/.../y/for/the/project"),
        ];
        for (mode, expected) in cases {
            let rendered = render_directory(&cfg_with(mode), &state, 80);
            assert_eq!(
                rendered.chars().count().min(40),
                rendered.chars().count(),
                "{mode}"
            );
            assert_eq!(rendered, expected, "{mode:?}");
        }
    }

    #[test]
    fn test_render_directory_follows_terminal_width() {
        const PATH: &str = "/home/dev/src/very/deeply/nested/workspace/directory/for/the/project";
        let state = StatuslineState {
            cwd: PathBuf::from(PATH),
            ..Default::default()
        };
        // (名前, 端末の幅, max_length, 省略後の最大文字数。None は省略しない)
        let cases = [
            ("width-80", 80, 0, Some(40)),
            ("width-120", 120, 0, Some(60)),
            ("narrow-follows-width", 60, 0, Some(30)),
            ("narrowest-truncating", 20, 0, Some(10)),
            ("too-narrow-keeps-path", 15, 0, None),
            ("max-length-on-narrow", 80, 25, Some(25)),
            ("max-length-ignores-width", 200, 25, Some(25)),
            ("max-length-longer-than-path", 80, 100, None),
        ];
        for (name, width, max_length, limit) in cases {
            let cfg = DirectoryConfig {
                format: "$path".to_string(),
                max_length,
                ..Default::default()
            };
            let rendered = render_directory(&cfg, &state, width);
            match limit {
                Some(limit) => {
                    assert_ne!(rendered, PATH, "{name}: 省略される");
                    assert!(rendered.chars().count() <= limit, "{name}: {rendered}");
                }
                None => assert_eq!(rendered, PATH, "{name}"),
            }
        }
    }

    #[test]
    fn test_render_directory_symbol() {
        let state = compose_state();
        let with_symbol = |symbol: &str| DirectoryConfig {
            symbol: symbol.to_string(),
            ..Default::default()
        };
        // `symbol` 導入前の設定（絵文字を format に直書き）は、そのまま動く
        let legacy = DirectoryConfig {
            format: "📁 [$folder]($folder_style)".to_string(),
            symbol: "> ".to_string(),
            ..Default::default()
        };
        let cases = [
            (
                "default",
                DirectoryConfig::default(),
                "📁 myproject [/nonexistent/dev/myproject] ",
            ),
            (
                "custom-symbol",
                with_symbol("> "),
                "> myproject [/nonexistent/dev/myproject] ",
            ),
            (
                "empty-symbol",
                with_symbol(""),
                "myproject [/nonexistent/dev/myproject] ",
            ),
            ("legacy-literal-format", legacy, "📁 myproject"),
        ];
        for (name, cfg, expected) in cases {
            let rendered = render_styled_text(&render_directory(&cfg, &state, 80));
            assert_eq!(strip_ansi(&rendered), expected, "{name}");
        }
    }
    fn strip_ansi(s: &str) -> String {
        let mut out = String::new();
        let mut chars = s.chars();
        while let Some(c) = chars.next() {
            if c == '\x1b' {
                for c in chars.by_ref() {
                    if c == 'm' {
                        break;
                    }
                }
            } else {
                out.push(c);
            }
        }
        out
    }
}
