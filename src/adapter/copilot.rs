use std::path::PathBuf;

use chrono::{DateTime, Utc};
use serde::Deserialize;

use crate::adapter::StatuslineAdapter;
use crate::model::ratelimit::{RatelimitLimitItem, RatelimitPayload};
use crate::model::state::{QuotaItem, StatuslineState, clamp_pct};

pub struct CopilotAdapter;

#[derive(Deserialize, Debug)]
struct CopilotInput {
    cwd: Option<PathBuf>,
    workspace: Option<CopilotWorkspace>,
    model: Option<CopilotModelInfo>,
    context: Option<CopilotContext>,
    context_window: Option<CopilotContextWindow>,
    rate_limit: Option<CopilotRateLimit>,
    status: Option<String>,
}

#[derive(Deserialize, Debug)]
struct CopilotWorkspace {
    current_dir: Option<PathBuf>,
}

#[derive(Deserialize, Debug)]
#[serde(untagged)]
enum CopilotModelInfo {
    Object {
        id: Option<String>,
        name: Option<String>,
    },
    String(String),
}

#[derive(Deserialize, Debug)]
struct CopilotContext {
    percentage: Option<f64>,
    total_tokens: Option<u64>,
}

#[derive(Deserialize, Debug)]
struct CopilotContextWindow {
    current_context_used_percentage: Option<f64>,
    current_context_tokens: Option<u64>,
    used_percentage: Option<f64>,
    total_input_tokens: Option<u64>,
    total_output_tokens: Option<u64>,
}

#[derive(Deserialize, Debug)]
struct CopilotRateLimit {
    remaining_percentage: Option<f64>,
    used_percentage: Option<f64>,
    reset_time: Option<String>,
    reset_in_seconds: Option<i64>,
}

impl StatuslineAdapter for CopilotAdapter {
    fn parse(&self, raw_json: &str) -> (StatuslineState, Option<RatelimitPayload>) {
        let Ok(input) = serde_json::from_str::<CopilotInput>(raw_json) else {
            return (StatuslineState::default(), None);
        };

        let cwd = input
            .workspace
            .and_then(|w| w.current_dir)
            .or(input.cwd)
            .unwrap_or_else(|| PathBuf::from("."));

        let model = match input.model {
            Some(CopilotModelInfo::Object { name, id }) => name.or(id),
            Some(CopilotModelInfo::String(s)) => Some(s),
            None => None,
        };

        let (context_used_percentage, total_input_tokens, total_output_tokens) =
            if let Some(cw) = input.context_window {
                let pct = cw
                    .current_context_used_percentage
                    .or(cw.used_percentage)
                    .map(clamp_pct);
                if let Some(current_tokens) = cw.current_context_tokens {
                    // current_context_tokens は現コンテキストの全体トークン数のため累積 output を合算しない
                    (pct, Some(current_tokens), None)
                } else {
                    (pct, cw.total_input_tokens, cw.total_output_tokens)
                }
            } else if let Some(ctx) = input.context {
                let pct = ctx.percentage.map(clamp_pct);
                (pct, ctx.total_tokens, None)
            } else {
                (None, None, None)
            };

        let mut quotas = Vec::new();
        let mut ratelimit_limits = Vec::new();

        if let Some(rl) = input.rate_limit {
            let used_pct = rl
                .used_percentage
                .or_else(|| rl.remaining_percentage.map(|rem| 100.0 - rem))
                .map(clamp_pct);

            let now_ts = Utc::now().timestamp();
            let (reset_in_seconds, reset_time_iso) = if let Some(sec) = rl.reset_in_seconds {
                let iso = DateTime::from_timestamp(now_ts + sec, 0)
                    .map_or_else(|| Utc::now().to_rfc3339(), |dt| dt.to_rfc3339());
                (Some(sec), Some(iso))
            } else if let Some(ref iso) = rl.reset_time {
                let diff = DateTime::parse_from_rfc3339(iso)
                    .ok()
                    .map(|dt| (dt.timestamp() - now_ts).max(0));
                (diff, Some(iso.clone()))
            } else {
                (None, None)
            };

            let label = "Copilot".to_string();
            let id = "copilot-requests".to_string();

            quotas.push(QuotaItem {
                id: id.clone(),
                label: label.clone(),
                used_percentage: used_pct,
                reset_in_seconds,
                reset_time_iso: reset_time_iso.clone(),
            });

            if let Some(reset_at) = reset_time_iso {
                ratelimit_limits.push(RatelimitLimitItem {
                    id,
                    label,
                    reset_at,
                    used_percentage: used_pct,
                });
            }
        }

        let ratelimit_payload = if ratelimit_limits.is_empty() {
            None
        } else {
            Some(RatelimitPayload {
                schema_version: 1,
                agent_id: "copilot".to_string(),
                observed_at: Utc::now().to_rfc3339(),
                limits: ratelimit_limits,
            })
        };

        let state = StatuslineState {
            cwd,
            model,
            context_used_percentage,
            total_input_tokens,
            total_output_tokens,
            agent_state: input.status,
            sandbox_enabled: false,
            plan_tier: None,
            quotas,
        };

        (state, ratelimit_payload)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_copilot_json_context_window() {
        // GitHub Copilot CLI issue 4233 報告形式の fixture
        let raw = r#"{
            "cwd": "C:\\Users\\jojob\\src\\myproject",
            "model": {
                "id": "claude-3.7-sonnet",
                "name": "Claude 3.7 Sonnet"
            },
            "workspace": {
                "current_dir": "C:\\Users\\jojob\\src\\myproject"
            },
            "context_window": {
                "current_context_used_percentage": 23.4,
                "current_context_tokens": 29800,
                "total_output_tokens": 1200
            },
            "rate_limit": {
                "used_percentage": 10.0,
                "reset_in_seconds": 1800
            },
            "status": "idle"
        }"#;

        let adapter = CopilotAdapter;
        let (state, payload) = adapter.parse(raw);

        assert_eq!(state.cwd, PathBuf::from("C:\\Users\\jojob\\src\\myproject"));
        assert_eq!(state.model.as_deref(), Some("Claude 3.7 Sonnet"));
        assert_eq!(state.context_used_percentage, Some(23));
        assert_eq!(state.total_input_tokens, Some(29800));
        assert_eq!(state.total_output_tokens, None);
        assert_eq!(state.agent_state.as_deref(), Some("idle"));
        assert_eq!(state.quotas.len(), 1);
        assert_eq!(state.quotas[0].used_percentage, Some(10));
        assert_eq!(state.quotas[0].reset_in_seconds, Some(1800));

        assert!(payload.is_some());

        // render_default で 29,800 tokens が累積 output と合算されず "29k tok" となることを検証
        let rendered = crate::engine::formatter::render_default(&state, 120);
        assert!(rendered.contains("Ctx: 23%"));
        assert!(rendered.contains("29k tok"));
    }

    #[test]
    fn test_parse_copilot_json_legacy_context() {
        let raw = r#"{
            "cwd": "C:\\Users\\jojob\\src\\myproject",
            "model": "claude-3.7-sonnet",
            "context": {
                "percentage": 18.5,
                "total_tokens": 24000
            },
            "rate_limit": {
                "remaining_percentage": 75.0,
                "reset_in_seconds": 3600
            },
            "status": "thinking"
        }"#;

        let adapter = CopilotAdapter;
        let (state, payload) = adapter.parse(raw);

        assert_eq!(state.cwd, PathBuf::from("C:\\Users\\jojob\\src\\myproject"));
        assert_eq!(state.model.as_deref(), Some("claude-3.7-sonnet"));
        assert_eq!(state.context_used_percentage, Some(19));
        assert_eq!(state.total_input_tokens, Some(24000));
        assert_eq!(state.agent_state.as_deref(), Some("thinking"));
        assert_eq!(state.quotas.len(), 1);
        assert_eq!(state.quotas[0].used_percentage, Some(25));
        assert_eq!(state.quotas[0].reset_in_seconds, Some(3600));

        assert!(payload.is_some());
        let p = payload.unwrap();
        assert_eq!(p.agent_id, "copilot");
    }
}
