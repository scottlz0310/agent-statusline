pub mod agy;
pub mod claude;
pub mod copilot;

use crate::model::ratelimit::RatelimitPayload;
use crate::model::state::StatuslineState;

pub trait StatuslineAdapter {
    fn parse(&self, raw_json: &str) -> (StatuslineState, Option<RatelimitPayload>);
}

#[cfg(test)]
mod tests {
    use super::StatuslineAdapter;
    use super::agy::AntigravityAdapter;
    use super::claude::ClaudeAdapter;
    use super::copilot::CopilotAdapter;
    use crate::sink::notifier::write_ratelimit_status_to_dir;

    // Squirrel Notifier の契約 (docs/statusline-integration.md) で固定された agentId。
    // Squirrel Notifier 側の ID は settings.json に保存されるため変更できない。
    #[test]
    fn test_ratelimit_agent_id_matches_notifier_contract() {
        let cases: [(&str, &dyn StatuslineAdapter, &str, &str); 3] = [
            (
                "claude",
                &ClaudeAdapter,
                r#"{"rate_limits":{"five_hour":{"used_percentage":25.0,"resets_at":1711234567}}}"#,
                "claude-code",
            ),
            (
                "agy",
                &AntigravityAdapter,
                r#"{"quota":{"gemini-5h":{"remaining_fraction":0.5,"reset_time":"2026-09-24T09:47:59Z"}}}"#,
                "agy",
            ),
            (
                "copilot",
                &CopilotAdapter,
                r#"{"rate_limit":{"used_percentage":10.0,"reset_in_seconds":1800}}"#,
                "copilot",
            ),
        ];

        for (name, adapter, raw, expected_id) in cases {
            let (_, payload) = adapter.parse(raw);
            let payload = payload.unwrap_or_else(|| panic!("{name}: payload missing"));
            assert_eq!(payload.agent_id, expected_id, "{name}");

            let dir = tempfile::tempdir().unwrap();
            write_ratelimit_status_to_dir(&payload, dir.path()).unwrap();
            assert!(
                dir.path().join(format!("{expected_id}.json")).exists(),
                "{name}"
            );
        }
    }
}
