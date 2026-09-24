use std::fmt::Write;

use crate::engine::style::{BLUE, BOLD, CYAN, GRAY, GREEN, MAGENTA, RED, RESET, YELLOW, pct_color};
use crate::model::state::StatuslineState;
use crate::modules::context::format_tokens;
use crate::modules::directory::format_path;
use crate::modules::git::get_git_status;
use crate::modules::quota::format_reset_seconds;

/// デフォルトの 3 行ステータスライン描画
pub fn render_default(state: &StatuslineState, terminal_width: usize) -> String {
    let mut output_lines = Vec::new();

    // 1行目: 📁 folder [path]  🌿 branch✗  🔒sandbox
    let folder = state
        .cwd
        .file_name()
        .map_or_else(|| ".".to_string(), |f| f.to_string_lossy().to_string());

    let path_max = (terminal_width / 2).max(40);
    let short_path = format_path(&state.cwd, path_max);
    let mut header_row = format!("📁 {BOLD}{CYAN}{folder}{RESET} {GRAY}[{short_path}]{RESET}");

    let git = get_git_status(&state.cwd);
    if let Some(branch) = git.branch {
        if git.is_dirty {
            let _ = write!(header_row, "  🌿 {GREEN}{branch}{RESET}{RED}✗{RESET}");
        } else {
            let _ = write!(header_row, "  🌿 {GREEN}{branch}{RESET}");
        }
    }
    if state.sandbox_enabled {
        let _ = write!(header_row, "  🔒{YELLOW}sandbox{RESET}");
    }
    output_lines.push(header_row);

    // 2行目: 🤖 model | 🧠 Ctx: 8% (108k tok) | ⚡ working | tier
    let sep = format!("  {GRAY}|{RESET}  ");
    let mut line2_parts = Vec::new();

    if let Some(model) = &state.model {
        line2_parts.push(format!("🤖 {MAGENTA}{model}{RESET}"));
    }

    if let Some(ctx) = state.context_used_percentage {
        let total_tokens =
            state.total_input_tokens.unwrap_or(0) + state.total_output_tokens.unwrap_or(0);
        let tok_str = format_tokens(total_tokens);
        let color = pct_color(ctx);
        line2_parts.push(format!(
            "🧠 {color}Ctx: {ctx}%{RESET} {GRAY}({tok_str} tok){RESET}"
        ));
    }

    if let Some(agent_state) = &state.agent_state
        && agent_state != "idle"
    {
        line2_parts.push(format!("{YELLOW}⚡ {agent_state}{RESET}"));
    }

    if let Some(plan_tier) = &state.plan_tier {
        line2_parts.push(format!("{BLUE}{plan_tier}{RESET}"));
    }

    if !line2_parts.is_empty() {
        output_lines.push(line2_parts.join(&sep));
    }

    // 3行目: ⏳ 3p-5h: 0% (rst 4h50m) | ...
    let mut line3_parts = Vec::new();
    for q in &state.quotas {
        if let Some(pct) = q.used_percentage {
            let color = pct_color(pct);
            let rst_str = q.reset_in_seconds.map(format_reset_seconds);
            if let Some(rst) = rst_str {
                line3_parts.push(format!("⏳ {}: {color}{pct}%{RESET} (rst {rst})", q.label));
            } else {
                line3_parts.push(format!("⏳ {}: {color}{pct}%{RESET}", q.label));
            }
        }
    }

    if !line3_parts.is_empty() {
        output_lines.push(line3_parts.join(&sep));
    }

    output_lines.join("\n")
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

        let rendered = render_default(&state, 120);
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
}
