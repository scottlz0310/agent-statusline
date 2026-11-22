//! Codex CLI (`com.openai/codex-statusline-state`) 向けスナップショット変換アダプタ。
//!
//! Phase 2 で `agent-statusline mcp codex` サブコマンドから接続するまで、
//! 非テストビルドでは未使用となる。
#![allow(dead_code)]

use std::path::PathBuf;

use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::Value;

use crate::adapter::StatuslineAdapter;
use crate::model::ratelimit::RatelimitPayload;
use crate::model::state::{QuotaItem, StatuslineState, clamp_pct};

pub struct CodexAdapter;

#[derive(Deserialize, Debug, Default)]
struct CodexSnapshot {
    account: Option<CodexAccount>,
    thread: Option<CodexThread>,
}

#[derive(Deserialize, Debug, Default)]
struct CodexAccount {
    rate_limits: Option<Vec<CodexRateLimitSnapshot>>,
    workspace: Option<CodexAccountWorkspace>,
}

#[derive(Deserialize, Debug, Default)]
struct CodexAccountWorkspace {
    headline: Option<String>,
}

#[derive(Deserialize, Debug, Default)]
struct CodexRateLimitSnapshot {
    limit_id: Option<String>,
    limit_name: Option<String>,
    primary_limit: Option<CodexRateLimitWindow>,
    secondary_limit: Option<CodexRateLimitWindow>,
}

#[derive(Deserialize, Debug, Default)]
struct CodexRateLimitWindow {
    used_percent: Option<f64>,
    window_minutes: Option<i64>,
    resets_at: Option<i64>,
}

#[derive(Deserialize, Debug, Default)]
struct CodexThread {
    agent: Option<CodexAgent>,
    model: Option<CodexModel>,
    workspace: Option<CodexWorkspace>,
    usage: Option<CodexUsage>,
    permissions: Option<CodexPermissions>,
}

#[derive(Deserialize, Debug, Default)]
struct CodexAgent {
    state: Option<String>,
}

#[derive(Deserialize, Debug, Default)]
struct CodexModel {
    display_name: Option<String>,
    id: Option<String>,
    reasoning_level: Option<String>,
    service_tier_display_name: Option<String>,
}

#[derive(Deserialize, Debug, Default)]
struct CodexWorkspace {
    cwd: Option<PathBuf>,
}

#[derive(Deserialize, Debug, Default)]
struct CodexUsage {
    context: Option<CodexContext>,
    total: Option<CodexTokenUsage>,
    cost: Option<CodexCost>,
}

#[derive(Deserialize, Debug, Default)]
struct CodexContext {
    used_tokens: Option<i64>,
    used_percentage: Option<f64>,
}

#[derive(Deserialize, Debug, Default)]
struct CodexTokenUsage {
    total_input_tokens: Option<i64>,
    output_tokens: Option<i64>,
}

#[derive(Deserialize, Debug, Default)]
struct CodexCost {
    total_cost_usd: Option<f64>,
}

#[derive(Deserialize, Debug, Default)]
struct CodexPermissions {
    #[serde(rename = "type")]
    kind: Option<String>,
    file_system: Option<CodexFileSystem>,
}

#[derive(Deserialize, Debug, Default)]
struct CodexFileSystem {
    #[serde(rename = "type")]
    kind: Option<String>,
}

fn non_empty_trimmed(opt: Option<String>) -> Option<String> {
    opt.and_then(|s| {
        let trimmed = s.trim();
        (!trimmed.is_empty()).then(|| trimmed.to_string())
    })
}

fn format_model_name(model: Option<CodexModel>) -> Option<String> {
    let model = model?;
    let base = non_empty_trimmed(model.display_name).or_else(|| non_empty_trimmed(model.id))?;
    let Some(level) = non_empty_trimmed(model.reasoning_level) else {
        return Some(base);
    };
    let suffix_lower = format!("({})", level.to_ascii_lowercase());
    if base.to_ascii_lowercase().contains(&suffix_lower) {
        Some(base)
    } else {
        Some(format!("{base} ({level})"))
    }
}

fn format_window_label(window_minutes: Option<i64>, fallback: &str) -> String {
    match window_minutes {
        Some(300) => "5h".to_string(),
        Some(10_080) => "7d".to_string(),
        Some(m) if m > 0 && m % 1_440 == 0 => format!("{}d", m / 1_440),
        Some(m) if m > 0 && m % 60 == 0 => format!("{}h", m / 60),
        Some(m) if m > 0 => format!("{m}m"),
        _ => fallback.to_string(),
    }
}

fn build_quota_item(
    limit_id: &str,
    prefix: Option<&str>,
    slot: &str,
    window: &CodexRateLimitWindow,
    now_ts: i64,
) -> QuotaItem {
    let used_percentage = window.used_percent.map(clamp_pct);
    let (reset_in_seconds, reset_time_iso) = window
        .resets_at
        .and_then(|ts| DateTime::from_timestamp(ts, 0).map(|dt| (ts, dt)))
        .map_or((None, None), |(ts, dt)| {
            let diff = ts.saturating_sub(now_ts).max(0);
            (Some(diff), Some(dt.to_rfc3339()))
        });

    let window_label = format_window_label(window.window_minutes, slot);
    let label = match prefix {
        Some(p) => format!("{p} {window_label}"),
        None => window_label,
    };

    QuotaItem {
        id: format!("{limit_id}-{slot}"),
        label,
        used_percentage,
        reset_in_seconds,
        reset_time_iso,
    }
}

impl CodexAdapter {
    /// すでにパース済みの `serde_json::Value`（`params._meta["com.openai/codex-statusline-state"]`）から変換する
    pub fn parse_value(value: &Value) -> (StatuslineState, Option<RatelimitPayload>) {
        Self::parse_value_with_now(value, Utc::now().timestamp())
    }

    /// 時刻を注入して `serde_json::Value` から変換する（テスト用 DI）
    pub fn parse_value_with_now(
        value: &Value,
        now_ts: i64,
    ) -> (StatuslineState, Option<RatelimitPayload>) {
        let Ok(snapshot) = serde_json::from_value::<CodexSnapshot>(value.clone()) else {
            return (StatuslineState::default(), None);
        };
        convert_snapshot(snapshot, now_ts)
    }

    /// 時刻を注入して JSON 文字列から変換する（テスト用 DI）
    pub fn parse_with_now(
        raw_json: &str,
        now_ts: i64,
    ) -> (StatuslineState, Option<RatelimitPayload>) {
        let Ok(snapshot) = serde_json::from_str::<CodexSnapshot>(raw_json) else {
            return (StatuslineState::default(), None);
        };
        convert_snapshot(snapshot, now_ts)
    }
}

fn convert_snapshot(
    snapshot: CodexSnapshot,
    now_ts: i64,
) -> (StatuslineState, Option<RatelimitPayload>) {
    let thread = snapshot.thread.unwrap_or_default();
    let account = snapshot.account.unwrap_or_default();

    let cwd = thread
        .workspace
        .and_then(|w| w.cwd)
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| PathBuf::from("."));

    let service_tier_display = thread
        .model
        .as_ref()
        .and_then(|m| non_empty_trimmed(m.service_tier_display_name.clone()));

    let model = format_model_name(thread.model);

    let usage = thread.usage.unwrap_or_default();
    let context_used_percentage = usage
        .context
        .as_ref()
        .and_then(|c| c.used_percentage)
        .map(clamp_pct);

    let (total_input_tokens, total_output_tokens) = if let Some(used_tokens) = usage
        .context
        .as_ref()
        .and_then(|c| c.used_tokens)
        .and_then(|v| u64::try_from(v).ok())
    {
        // context.used_tokens は現コンテキストの全体トークン数のため累積 output を合算しない
        (Some(used_tokens), None)
    } else if let Some(total) = usage.total {
        (
            total.total_input_tokens.and_then(|v| u64::try_from(v).ok()),
            total.output_tokens.and_then(|v| u64::try_from(v).ok()),
        )
    } else {
        (None, None)
    };

    let agent_state = thread
        .agent
        .and_then(|a| non_empty_trimmed(a.state))
        .filter(|s| !s.eq_ignore_ascii_case("ready") && !s.eq_ignore_ascii_case("idle"));

    let sandbox_enabled = thread.permissions.as_ref().is_some_and(|p| {
        p.kind.as_deref() == Some("managed")
            && p.file_system.as_ref().and_then(|fs| fs.kind.as_deref()) == Some("restricted")
    });

    let cost_tier = usage
        .cost
        .and_then(|c| c.total_cost_usd)
        .filter(|cost| cost.is_finite() && *cost >= 0.0)
        .map(|cost| format!("${cost:.2}"));

    let workspace_headline = account
        .workspace
        .and_then(|w| non_empty_trimmed(w.headline));

    let plan_tier = cost_tier.or(workspace_headline).or(service_tier_display);

    let mut quotas = Vec::new();
    if let Some(rate_limits) = account.rate_limits {
        let multiple = rate_limits.len() > 1;
        for rl in &rate_limits {
            let limit_id = rl
                .limit_id
                .as_deref()
                .filter(|s| !s.trim().is_empty())
                .unwrap_or("codex");
            let prefix = if multiple {
                rl.limit_name
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .or(Some(limit_id))
            } else {
                None
            };

            if let Some(primary) = &rl.primary_limit {
                quotas.push(build_quota_item(
                    limit_id, prefix, "primary", primary, now_ts,
                ));
            }
            if let Some(secondary) = &rl.secondary_limit {
                quotas.push(build_quota_item(
                    limit_id,
                    prefix,
                    "secondary",
                    secondary,
                    now_ts,
                ));
            }
        }
    }

    let state = StatuslineState {
        cwd,
        model,
        context_used_percentage,
        total_input_tokens,
        total_output_tokens,
        agent_state,
        sandbox_enabled,
        plan_tier,
        quotas,
    };

    // Squirrel Notifier は Codex のレートリミットを codex app-server から直接取得する契約のため、
    // statusline 経由での ratelimit-status/codex.json 書き出しは行わない。
    (state, None)
}

impl StatuslineAdapter for CodexAdapter {
    fn parse(&self, raw_json: &str) -> (StatuslineState, Option<RatelimitPayload>) {
        Self::parse_with_now(raw_json, Utc::now().timestamp())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_codex_real_turn0_snapshot() {
        // Phase 0 スパイクで Codex CLI 0.171.0 から実測採取した 0 ターン実スナップショット
        let raw = r#"{
            "account": {
                "rate_limits": [
                    {
                        "limit_id": "codex",
                        "limit_name": "codex",
                        "primary_limit": {
                            "used_percent": 11,
                            "window_minutes": 300,
                            "resets_at": 1795365799
                        },
                        "secondary_limit": {
                            "used_percent": 40,
                            "window_minutes": 10080,
                            "resets_at": 1795605837
                        }
                    }
                ]
            },
            "app": {
                "cli_version": "0.171.0",
                "display_mode": "standard"
            },
            "config": {
                "statusline_items": ["model-with-reasoning", "current-dir"]
            },
            "thread": {
                "id": "01a20332-d985-7c40-b67d-2a9e13abf0ce",
                "name": null,
                "agent": { "state": "ready" },
                "model": {
                    "access_program": "standard",
                    "display_name": "GPT-6.2-Sol",
                    "id": "gpt-6.2-sol",
                    "provider": "openai",
                    "reasoning_level": "medium",
                    "service_tier": "normal"
                },
                "workspace": {
                    "cwd": "C:\\Users\\dev\\src\\agent-statusline"
                },
                "usage": {
                    "context": {}
                },
                "approval": {
                    "policy": "never",
                    "reviewer": "user"
                },
                "permissions": {
                    "display_name": "Full Access",
                    "type": "disabled"
                }
            }
        }"#;

        let now = 1_795_362_199;
        let (state, payload) = CodexAdapter::parse_with_now(raw, now);

        assert_eq!(
            state.cwd,
            PathBuf::from("C:\\Users\\dev\\src\\agent-statusline")
        );
        assert_eq!(state.model.as_deref(), Some("GPT-6.2-Sol (medium)"));
        assert_eq!(state.context_used_percentage, None);
        assert_eq!(state.total_input_tokens, None);
        assert_eq!(state.total_output_tokens, None);
        assert_eq!(state.agent_state, None);
        assert!(!state.sandbox_enabled);
        assert_eq!(state.plan_tier, None);

        assert_eq!(state.quotas.len(), 2);
        assert_eq!(state.quotas[0].id, "codex-primary");
        assert_eq!(state.quotas[0].label, "5h");
        assert_eq!(state.quotas[0].used_percentage, Some(11));
        assert_eq!(state.quotas[0].reset_in_seconds, Some(3600));
        assert_eq!(state.quotas[1].id, "codex-secondary");
        assert_eq!(state.quotas[1].label, "7d");
        assert_eq!(state.quotas[1].used_percentage, Some(40));

        // Squirrel Notifier への ratelimit ファイル出力は行わない
        assert!(payload.is_none());
    }

    #[test]
    fn test_parse_codex_context_and_plan_priority() {
        let cases = [
            (
                "used_tokens-preferred-and-cost-wins",
                r#"{
                    "account": { "workspace": { "headline": "Team Pro" } },
                    "thread": {
                        "workspace": { "cwd": "/work/repo" },
                        "model": {
                            "display_name": "GPT-6.2-Sol",
                            "service_tier_display_name": "Fast"
                        },
                        "usage": {
                            "context": { "used_percentage": 34.6, "used_tokens": 42000 },
                            "total": { "total_input_tokens": 90000, "output_tokens": 12000 },
                            "cost": { "total_cost_usd": 1.25 }
                        }
                    }
                }"#,
                Some(35),
                Some(42_000),
                None,
                Some("$1.25"),
            ),
            (
                "fallback-to-total-and-workspace-headline",
                r#"{
                    "account": { "workspace": { "headline": "Team Pro" } },
                    "thread": {
                        "workspace": { "cwd": "/work/repo" },
                        "model": {
                            "display_name": "GPT-6.2-Sol",
                            "service_tier_display_name": "Fast"
                        },
                        "usage": {
                            "context": { "used_percentage": 10.0 },
                            "total": { "total_input_tokens": 15000, "output_tokens": 3000 }
                        }
                    }
                }"#,
                Some(10),
                Some(15_000),
                Some(3_000),
                Some("Team Pro"),
            ),
            (
                "fallback-to-service-tier-display-name",
                r#"{
                    "thread": {
                        "workspace": { "cwd": "/work/repo" },
                        "model": {
                            "id": "gpt-6.2-sol",
                            "service_tier_display_name": "Fast"
                        },
                        "usage": { "context": { "used_tokens": -5 } }
                    }
                }"#,
                None,
                None,
                None,
                Some("Fast"),
            ),
        ];

        for (name, raw, expected_pct, expected_in, expected_out, expected_plan) in cases {
            let (state, payload) = CodexAdapter::parse_with_now(raw, 1_700_000_000);
            assert_eq!(state.context_used_percentage, expected_pct, "{name}");
            assert_eq!(state.total_input_tokens, expected_in, "{name}");
            assert_eq!(state.total_output_tokens, expected_out, "{name}");
            assert_eq!(state.plan_tier.as_deref(), expected_plan, "{name}");
            assert!(payload.is_none(), "{name}");
        }
    }

    #[test]
    fn test_parse_codex_model_agent_state_and_sandbox_variants() {
        let cases = [
            (
                "managed-restricted-working",
                r#"{
                    "thread": {
                        "model": { "display_name": "GPT-6.2-Sol", "reasoning_level": "high" },
                        "agent": { "state": "working" },
                        "permissions": {
                            "type": "managed",
                            "file_system": { "type": "restricted" },
                            "network": "restricted"
                        }
                    }
                }"#,
                Some("GPT-6.2-Sol (high)"),
                Some("working"),
                true,
            ),
            (
                "managed-unrestricted-thinking",
                r#"{
                    "thread": {
                        "model": { "display_name": "GPT-6.2-Sol (high)", "reasoning_level": "high" },
                        "agent": { "state": "thinking" },
                        "permissions": {
                            "type": "managed",
                            "file_system": { "type": "unrestricted" },
                            "network": "enabled"
                        }
                    }
                }"#,
                Some("GPT-6.2-Sol (high)"),
                Some("thinking"),
                false,
            ),
            (
                "external-ready-id-fallback",
                r#"{
                    "thread": {
                        "model": { "display_name": "  ", "id": "o4-mini" },
                        "agent": { "state": "ready" },
                        "permissions": { "type": "external", "network": "restricted" }
                    }
                }"#,
                Some("o4-mini"),
                None,
                false,
            ),
            (
                "disabled-waiting",
                r#"{
                    "thread": {
                        "model": { "display_name": "Codex" },
                        "agent": { "state": "waiting" },
                        "permissions": { "type": "disabled" }
                    }
                }"#,
                Some("Codex"),
                Some("waiting"),
                false,
            ),
        ];

        for (name, raw, expected_model, expected_state, expected_sandbox) in cases {
            let (state, _) = CodexAdapter::parse_with_now(raw, 1_700_000_000);
            assert_eq!(state.model.as_deref(), expected_model, "{name}");
            assert_eq!(state.agent_state.as_deref(), expected_state, "{name}");
            assert_eq!(state.sandbox_enabled, expected_sandbox, "{name}");
        }
    }

    #[test]
    fn test_parse_codex_multiple_rate_limits_and_custom_windows() {
        let raw = r#"{
            "account": {
                "rate_limits": [
                    {
                        "limit_id": "codex",
                        "limit_name": "Codex",
                        "primary_limit": { "used_percent": 120.5, "window_minutes": 180, "resets_at": 1700000000 }
                    },
                    {
                        "limit_id": "spark",
                        "limit_name": "Spark",
                        "secondary_limit": { "used_percent": 25.0, "window_minutes": 45, "resets_at": null }
                    }
                ]
            }
        }"#;

        let value: Value = serde_json::from_str(raw).unwrap();
        let (state, payload) = CodexAdapter::parse_value_with_now(&value, 1_700_000_100);
        assert!(payload.is_none());
        assert_eq!(state.cwd, PathBuf::from("."));
        assert_eq!(state.quotas.len(), 2);
        assert_eq!(state.quotas[0].id, "codex-primary");
        assert_eq!(state.quotas[0].label, "Codex 3h");
        assert_eq!(state.quotas[0].used_percentage, Some(100));
        assert_eq!(state.quotas[0].reset_in_seconds, Some(0));
        assert_eq!(state.quotas[1].id, "spark-secondary");
        assert_eq!(state.quotas[1].label, "Spark 45m");
        assert_eq!(state.quotas[1].used_percentage, Some(25));
        assert_eq!(state.quotas[1].reset_in_seconds, None);
    }

    #[test]
    fn test_parse_codex_resets_at_boundaries() {
        let now = 1_700_000_000i64;
        let cases = [
            ("future-valid", Some(now + 3_600), Some(3_600), true),
            ("past-valid-clamped-to-zero", Some(now - 600), Some(0), true),
            ("epoch-zero-valid-past", Some(0), Some(0), true),
            ("out-of-range-min", Some(i64::MIN), None, false),
            ("out-of-range-max", Some(i64::MAX), None, false),
            ("missing", None, None, false),
        ];

        for (name, resets_at, expected_secs, expect_iso) in cases {
            let resets_json = resets_at.map_or_else(|| "null".to_string(), |v| v.to_string());
            let raw = format!(
                r#"{{
                    "account": {{
                        "rate_limits": [{{
                            "limit_id": "codex",
                            "primary_limit": {{
                                "used_percent": 42.0,
                                "window_minutes": 300,
                                "resets_at": {resets_json}
                            }}
                        }}]
                    }}
                }}"#
            );
            let (state, payload) = CodexAdapter::parse_with_now(&raw, now);
            assert!(payload.is_none(), "{name}");
            assert_eq!(state.quotas.len(), 1, "{name}");
            assert_eq!(state.quotas[0].used_percentage, Some(42), "{name}");
            assert_eq!(state.quotas[0].reset_in_seconds, expected_secs, "{name}");
            assert_eq!(
                state.quotas[0].reset_time_iso.is_some(),
                expect_iso,
                "{name}"
            );
        }
    }

    #[test]
    fn test_parse_codex_invalid_json_returns_default() {
        let (state, payload) = CodexAdapter.parse("{invalid");
        assert_eq!(state.cwd, PathBuf::new());
        assert!(state.model.is_none());
        assert!(state.quotas.is_empty());
        assert!(payload.is_none());
    }
}
