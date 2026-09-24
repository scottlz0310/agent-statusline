use crate::engine::config::{
    AgentStateConfig, Config, ContextConfig, DirectoryConfig, GitBranchConfig, GitStatusConfig,
    ModelConfig, PlanConfig, QuotaConfig, SandboxConfig,
};
use crate::engine::style::{pct_style_name, render_styled_text};
use crate::model::state::StatuslineState;
use crate::modules::context::format_tokens;
use crate::modules::directory::format_path;
use crate::modules::git::get_git_status;
use crate::modules::quota::format_reset_seconds;

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
    let path_max = (terminal_width / 2).max(40);
    let short_path = format_path(&state.cwd, path_max);

    cfg.format
        .replace("$folder_style", &cfg.folder_style)
        .replace("$folder", &folder)
        .replace("$path_style", &cfg.path_style)
        .replace("$path", &short_path)
}

pub fn render_git_branch(
    cfg: &GitBranchConfig,
    git: Option<&crate::modules::git::GitStatus>,
) -> String {
    if cfg.disabled {
        return String::new();
    }
    let Some(git) = git else {
        return String::new();
    };
    let Some(branch) = &git.branch else {
        return String::new();
    };

    cfg.format
        .replace("$symbol", &cfg.symbol)
        .replace("$branch", branch)
        .replace("$style", &cfg.style)
}

pub fn render_git_status(
    cfg: &GitStatusConfig,
    git: Option<&crate::modules::git::GitStatus>,
) -> String {
    if cfg.disabled {
        return String::new();
    }
    let Some(git) = git else {
        return String::new();
    };
    if !git.is_dirty {
        return String::new();
    }

    cfg.format
        .replace("$status", &cfg.modified)
        .replace("$style", &cfg.style)
}

pub fn render_sandbox(cfg: &SandboxConfig, state: &StatuslineState) -> String {
    if cfg.disabled || !state.sandbox_enabled {
        return String::new();
    }

    cfg.format
        .replace("$symbol", &cfg.symbol)
        .replace("$style", &cfg.style)
}

pub fn render_model(cfg: &ModelConfig, state: &StatuslineState) -> String {
    if cfg.disabled {
        return String::new();
    }
    let Some(model) = &state.model else {
        return String::new();
    };

    cfg.format
        .replace("$symbol", &cfg.symbol)
        .replace("$model", model)
        .replace("$style", &cfg.style)
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

    cfg.format
        .replace("$symbol", &cfg.symbol)
        .replace("$percentage", &format!("{ctx}%"))
        .replace("$pct_style", pct_style)
        .replace("$tokens", &tokens)
        .replace("$tok_style", &cfg.tok_style)
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

    cfg.format
        .replace("$symbol", &cfg.symbol)
        .replace("$state", agent_state)
        .replace("$style", &cfg.style)
}

pub fn render_plan(cfg: &PlanConfig, state: &StatuslineState) -> String {
    if cfg.disabled {
        return String::new();
    }
    let Some(plan_tier) = &state.plan_tier else {
        return String::new();
    };

    cfg.format
        .replace("$tier", plan_tier)
        .replace("$style", &cfg.style)
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

        let style = pct_style_name(
            pct,
            &cfg.style_normal,
            &cfg.style_warning,
            &cfg.style_critical,
            cfg.threshold_warning,
            cfg.threshold_critical,
        );

        let mut item = cfg
            .format
            .replace("$symbol", &cfg.symbol)
            .replace("$label", &q.label)
            .replace("$percentage", &format!("{pct}%"))
            .replace("$style", style);

        if let Some(reset_sec) = q.reset_in_seconds {
            let rst = format_reset_seconds(reset_sec);
            item = item.replace("$reset_time", &rst);
        } else {
            item = item
                .replace(" (rst $reset_time)", "")
                .replace("(rst $reset_time)", "")
                .replace("$reset_time", "");
        }

        items.push(item);
    }

    if items.is_empty() {
        String::new()
    } else {
        items.join(&cfg.separator)
    }
}

/// 設定テンプレートに従ってステータスラインを描画する
pub fn render_template(config: &Config, state: &StatuslineState, terminal_width: usize) -> String {
    let dir_str = if config.format.contains("$directory") {
        render_directory(&config.directory, state, terminal_width)
    } else {
        String::new()
    };

    let needs_git = (config.format.contains("$git_branch") && !config.git_branch.disabled)
        || (config.format.contains("$git_status") && !config.git_status.disabled);

    let git_status = if needs_git {
        Some(get_git_status(&state.cwd))
    } else {
        None
    };

    let branch_str = if config.format.contains("$git_branch") {
        render_git_branch(&config.git_branch, git_status.as_ref())
    } else {
        String::new()
    };

    let git_status_str = if config.format.contains("$git_status") {
        render_git_status(&config.git_status, git_status.as_ref())
    } else {
        String::new()
    };

    let sandbox_str = if config.format.contains("$sandbox") {
        render_sandbox(&config.sandbox, state)
    } else {
        String::new()
    };

    let model_str = if config.format.contains("$model") {
        render_model(&config.model, state)
    } else {
        String::new()
    };

    let context_str = if config.format.contains("$context") {
        render_context(&config.context, state)
    } else {
        String::new()
    };

    let agent_state_str = if config.format.contains("$agent_state") {
        render_agent_state(&config.agent_state, state)
    } else {
        String::new()
    };

    let plan_str = if config.format.contains("$plan") {
        render_plan(&config.plan, state)
    } else {
        String::new()
    };

    let quota_str = if config.format.contains("$quota") {
        render_quota(&config.quota, state)
    } else {
        String::new()
    };

    let mut output_lines = Vec::new();

    for line in config.format.lines() {
        let filled = line
            .replace("$directory", &dir_str)
            .replace("$git_branch", &branch_str)
            .replace("$git_status", &git_status_str)
            .replace("$sandbox", &sandbox_str)
            .replace("$model", &model_str)
            .replace("$context", &context_str)
            .replace("$agent_state", &agent_state_str)
            .replace("$plan", &plan_str)
            .replace("$quota", &quota_str);

        let trimmed = filled.trim_end();
        if !trimmed.is_empty() {
            output_lines.push(render_styled_text(trimmed));
        }
    }

    output_lines.join("\n")
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
        let mut config = Config::default();
        config.format = "$model\n$directory".to_string();
        config.model.format = "MODEL: [$model]($style)".to_string();
        config.model.style = "cyan".to_string();

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
}
