use std::collections::HashMap;
use std::path::PathBuf;

use chrono::{DateTime, Utc};
use serde::Deserialize;

use crate::adapter::StatuslineAdapter;
use crate::model::ratelimit::{RatelimitLimitItem, RatelimitPayload};
use crate::model::state::{QuotaItem, StatuslineState, clamp_pct};

pub struct ClaudeAdapter;

#[derive(Deserialize, Debug)]
struct ClaudeInput {
    cwd: Option<PathBuf>,
    workspace: Option<ClaudeWorkspace>,
    model: Option<ClaudeModelInfo>,
    context_window: Option<ClaudeContextWindow>,
    rate_limits: Option<HashMap<String, ClaudeRateLimitItem>>,
    cost: Option<ClaudeCost>,
}

#[derive(Deserialize, Debug)]
struct ClaudeWorkspace {
    current_dir: Option<PathBuf>,
}

#[derive(Deserialize, Debug)]
#[serde(untagged)]
enum ClaudeModelInfo {
    Object {
        id: Option<String>,
        display_name: Option<String>,
    },
    String(String),
}

#[derive(Deserialize, Debug)]
struct ClaudeContextWindow {
    total_input_tokens: Option<u64>,
    total_output_tokens: Option<u64>,
    used_percentage: Option<f64>,
}

#[derive(Deserialize, Debug)]
struct ClaudeCost {
    total_cost_usd: Option<f64>,
}

#[derive(Deserialize, Debug)]
struct ClaudeRateLimitItem {
    used_percentage: Option<f64>,
    resets_at: Option<ClaudeResetTime>,
}

#[derive(Deserialize, Debug)]
#[serde(untagged)]
enum ClaudeResetTime {
    Timestamp(i64),
    Iso(String),
}

impl StatuslineAdapter for ClaudeAdapter {
    fn parse(&self, raw_json: &str) -> (StatuslineState, Option<RatelimitPayload>) {
        let Ok(input) = serde_json::from_str::<ClaudeInput>(raw_json) else {
            return (StatuslineState::default(), None);
        };

        let cwd = input
            .workspace
            .and_then(|w| w.current_dir)
            .or(input.cwd)
            .unwrap_or_else(|| PathBuf::from("."));

        let model = match input.model {
            Some(ClaudeModelInfo::Object { display_name, id }) => display_name.or(id),
            Some(ClaudeModelInfo::String(s)) => Some(s),
            None => None,
        };

        let (context_used_percentage, total_input_tokens, total_output_tokens) =
            if let Some(cw) = input.context_window {
                let pct = cw.used_percentage.map(clamp_pct);
                (pct, cw.total_input_tokens, cw.total_output_tokens)
            } else {
                (None, None, None)
            };

        let plan_tier = input
            .cost
            .and_then(|c| c.total_cost_usd)
            .map(|cost| format!("${cost:.2}"));

        let mut quotas = Vec::new();
        let mut ratelimit_limits = Vec::new();

        if let Some(rl_map) = input.rate_limits {
            let priority_order = ["five_hour", "fiveHour", "seven_day", "sevenDay"];
            let mut keys: Vec<String> = rl_map.keys().cloned().collect();
            keys.sort_by_key(|k| {
                priority_order
                    .iter()
                    .position(|&p| p == k)
                    .unwrap_or(usize::MAX)
            });

            let now_ts = Utc::now().timestamp();

            for key in keys {
                if let Some(item) = rl_map.get(&key) {
                    let used_pct = item.used_percentage.map(clamp_pct);

                    let (reset_in_seconds, reset_time_iso) = match &item.resets_at {
                        Some(ClaudeResetTime::Timestamp(ts)) => {
                            let diff = (*ts - now_ts).max(0);
                            let iso = DateTime::from_timestamp(*ts, 0)
                                .map_or_else(|| Utc::now().to_rfc3339(), |dt| dt.to_rfc3339());
                            (Some(diff), Some(iso))
                        }
                        Some(ClaudeResetTime::Iso(iso)) => {
                            let diff = DateTime::parse_from_rfc3339(iso)
                                .ok()
                                .map(|dt| (dt.timestamp() - now_ts).max(0));
                            (diff, Some(iso.clone()))
                        }
                        None => (None, None),
                    };

                    let label = match key.as_str() {
                        "five_hour" | "fiveHour" => "5h".to_string(),
                        "seven_day" | "sevenDay" => "7d".to_string(),
                        other => other.to_string(),
                    };

                    quotas.push(QuotaItem {
                        id: key.clone(),
                        label: label.clone(),
                        used_percentage: used_pct,
                        reset_in_seconds,
                        reset_time_iso: reset_time_iso.clone(),
                    });

                    if let Some(reset_at) = reset_time_iso {
                        ratelimit_limits.push(RatelimitLimitItem {
                            id: key,
                            label,
                            reset_at,
                            used_percentage: used_pct,
                        });
                    }
                }
            }
        }

        let ratelimit_payload = if ratelimit_limits.is_empty() {
            None
        } else {
            Some(RatelimitPayload {
                schema_version: 1,
                agent_id: "claude".to_string(),
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
            agent_state: None,
            sandbox_enabled: false,
            plan_tier,
            quotas,
        };

        (state, ratelimit_payload)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_claude_json() {
        let raw = r#"{
            "session_id": "claude-123",
            "model": {
                "id": "claude-3-7-sonnet-20250219",
                "display_name": "Claude 3.7 Sonnet"
            },
            "workspace": {
                "current_dir": "/home/user/project"
            },
            "context_window": {
                "total_input_tokens": 15000,
                "total_output_tokens": 2500,
                "used_percentage": 12.3
            },
            "cost": {
                "total_cost_usd": 0.42
            },
            "rate_limits": {
                "five_hour": {
                    "used_percentage": 25.0,
                    "resets_at": 1711234567
                }
            }
        }"#;

        let adapter = ClaudeAdapter;
        let (state, payload) = adapter.parse(raw);

        assert_eq!(state.cwd, PathBuf::from("/home/user/project"));
        assert_eq!(state.model.as_deref(), Some("Claude 3.7 Sonnet"));
        assert_eq!(state.context_used_percentage, Some(12));
        assert_eq!(state.total_input_tokens, Some(15000));
        assert_eq!(state.total_output_tokens, Some(2500));
        assert_eq!(state.plan_tier.as_deref(), Some("$0.42"));
        assert_eq!(state.quotas.len(), 1);
        assert_eq!(state.quotas[0].label, "5h");
        assert_eq!(state.quotas[0].used_percentage, Some(25));

        assert!(payload.is_some());
        let p = payload.unwrap();
        assert_eq!(p.agent_id, "claude");
    }

    #[test]
    fn test_parse_claude_string_model() {
        let raw = r#"{
            "cwd": "/tmp/repo",
            "model": "claude-3-5-haiku-20241022"
        }"#;

        let adapter = ClaudeAdapter;
        let (state, _) = adapter.parse(raw);
        assert_eq!(state.cwd, PathBuf::from("/tmp/repo"));
        assert_eq!(state.model.as_deref(), Some("claude-3-5-haiku-20241022"));
    }
}
