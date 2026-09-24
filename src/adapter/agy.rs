use std::collections::HashMap;
use std::path::PathBuf;

use chrono::Utc;
use serde::Deserialize;

use crate::adapter::StatuslineAdapter;
use crate::model::ratelimit::{RatelimitLimitItem, RatelimitPayload};
use crate::model::state::{QuotaItem, StatuslineState, clamp_pct};

pub struct AntigravityAdapter;

#[derive(Deserialize, Debug)]
struct AgyInput {
    cwd: Option<PathBuf>,
    workspace: Option<AgyWorkspace>,
    model: Option<AgyModel>,
    context_window: Option<AgyContextWindow>,
    quota: Option<HashMap<String, AgyQuotaItem>>,
    agent_state: Option<String>,
    sandbox: Option<AgySandbox>,
    plan_tier: Option<String>,
}

#[derive(Deserialize, Debug)]
struct AgyWorkspace {
    current_dir: Option<PathBuf>,
}

#[derive(Deserialize, Debug)]
struct AgyModel {
    id: Option<String>,
    display_name: Option<String>,
}

#[derive(Deserialize, Debug)]
struct AgyContextWindow {
    total_input_tokens: Option<u64>,
    total_output_tokens: Option<u64>,
    used_percentage: Option<f64>,
}

#[derive(Deserialize, Debug)]
struct AgyQuotaItem {
    remaining_fraction: Option<f64>,
    reset_time: Option<String>,
    reset_in_seconds: Option<i64>,
}

#[derive(Deserialize, Debug)]
struct AgySandbox {
    enabled: Option<bool>,
}

impl StatuslineAdapter for AntigravityAdapter {
    fn parse(&self, raw_json: &str) -> (StatuslineState, Option<RatelimitPayload>) {
        let Ok(input) = serde_json::from_str::<AgyInput>(raw_json) else {
            return (StatuslineState::default(), None);
        };

        let cwd = input
            .workspace
            .and_then(|w| w.current_dir)
            .or(input.cwd)
            .unwrap_or_else(|| PathBuf::from("."));

        let model = input.model.and_then(|m| m.display_name.or(m.id));

        let (context_used_percentage, total_input_tokens, total_output_tokens) =
            if let Some(cw) = input.context_window {
                let pct = cw.used_percentage.map(clamp_pct);
                (pct, cw.total_input_tokens, cw.total_output_tokens)
            } else {
                (None, None, None)
            };

        let sandbox_enabled = input.sandbox.and_then(|s| s.enabled).unwrap_or(false);

        // クォータ項目の変換
        let mut quotas = Vec::new();
        let mut ratelimit_limits = Vec::new();

        if let Some(quota_map) = input.quota {
            // 表示順序を安定化 (gemini-5h, gemini-weekly, 3p-5h, 3p-weekly, その他)
            let priority_order = ["gemini-5h", "gemini-weekly", "3p-5h", "3p-weekly"];
            let mut keys: Vec<String> = quota_map.keys().cloned().collect();
            keys.sort_by_key(|k| {
                priority_order
                    .iter()
                    .position(|&p| p == k)
                    .unwrap_or(usize::MAX)
            });

            for key in keys {
                if let Some(item) = quota_map.get(&key) {
                    let used_pct = item
                        .remaining_fraction
                        .map(|rf| clamp_pct((1.0 - rf) * 100.0));

                    let label = match key.as_str() {
                        "gemini-5h" => "Gemini 5h".to_string(),
                        "gemini-weekly" => "Gemini Wk".to_string(),
                        "3p-5h" => "3P 5h".to_string(),
                        "3p-weekly" => "3P Wk".to_string(),
                        other => other.to_string(),
                    };

                    quotas.push(QuotaItem {
                        id: key.clone(),
                        label: label.clone(),
                        used_percentage: used_pct,
                        reset_in_seconds: item.reset_in_seconds,
                        reset_time_iso: item.reset_time.clone(),
                    });

                    if let Some(reset_at) = &item.reset_time {
                        ratelimit_limits.push(RatelimitLimitItem {
                            id: key,
                            label,
                            reset_at: reset_at.clone(),
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
                agent_id: "agy".to_string(),
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
            agent_state: input.agent_state,
            sandbox_enabled,
            plan_tier: input.plan_tier,
            quotas,
        };

        (state, ratelimit_payload)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_agy_real_json() {
        let raw = r#"{
            "cwd": "C:\\Users\\jojob\\src",
            "model": {
                "id": "Gemini 3.8 Flash (High)",
                "display_name": "Gemini 3.8 Flash (High)",
                "effort": "high"
            },
            "workspace": {
                "current_dir": "C:\\Users\\jojob\\src",
                "project_dir": "C:\\Users\\jojob\\src"
            },
            "context_window": {
                "total_input_tokens": 88396,
                "total_output_tokens": 20603,
                "used_percentage": 8.43
            },
            "quota": {
                "gemini-5h": {
                    "remaining_fraction": 1.0,
                    "reset_time": "2026-09-24T09:47:59Z",
                    "reset_in_seconds": 17410
                },
                "3p-5h": {
                    "remaining_fraction": 0.8,
                    "reset_time": "2026-09-24T09:47:59Z",
                    "reset_in_seconds": 17410
                }
            },
            "agent_state": "working",
            "sandbox": { "enabled": true },
            "plan_tier": "Google AI Pro"
        }"#;

        let adapter = AntigravityAdapter;
        let (state, payload) = adapter.parse(raw);

        assert_eq!(state.cwd, PathBuf::from("C:\\Users\\jojob\\src"));
        assert_eq!(state.model.as_deref(), Some("Gemini 3.8 Flash (High)"));
        assert_eq!(state.context_used_percentage, Some(8));
        assert_eq!(state.total_input_tokens, Some(88396));
        assert_eq!(state.total_output_tokens, Some(20603));
        assert!(state.sandbox_enabled);
        assert_eq!(state.agent_state.as_deref(), Some("working"));
        assert_eq!(state.plan_tier.as_deref(), Some("Google AI Pro"));
        assert_eq!(state.quotas.len(), 2);
        assert_eq!(state.quotas[0].id, "gemini-5h");
        assert_eq!(state.quotas[0].used_percentage, Some(0));
        assert_eq!(state.quotas[1].id, "3p-5h");
        assert_eq!(state.quotas[1].used_percentage, Some(20));

        assert!(payload.is_some());
        let p = payload.unwrap();
        assert_eq!(p.agent_id, "agy");
        assert_eq!(p.limits.len(), 2);
    }
}
