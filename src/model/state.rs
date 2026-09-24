#![allow(dead_code)]

use std::path::PathBuf;

/// クライアントから抽出された統一ステータスライン情報
#[derive(Debug, Clone, Default)]
pub struct StatuslineState {
    pub cwd: PathBuf,
    pub model: Option<String>,
    pub context_used_percentage: Option<u8>,
    pub total_input_tokens: Option<u64>,
    pub total_output_tokens: Option<u64>,
    pub agent_state: Option<String>,
    pub sandbox_enabled: bool,
    pub plan_tier: Option<String>,
    pub quotas: Vec<QuotaItem>,
}

#[derive(Debug, Clone)]
pub struct QuotaItem {
    pub id: String,
    pub label: String,
    pub used_percentage: Option<u8>,
    pub reset_in_seconds: Option<i64>,
    pub reset_time_iso: Option<String>,
}

/// f64 のパーセンテージを 0..=100 の u8 に安全に丸めて変換する
#[inline]
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
pub fn clamp_pct(val: f64) -> u8 {
    val.round().clamp(0.0, 100.0) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_clamp_pct() {
        let cases = [
            (-10.0, 0),
            (0.0, 0),
            (8.43, 8),
            (8.6, 9),
            (50.0, 50),
            (99.9, 100),
            (150.0, 100),
        ];

        for (val, expected) in cases {
            assert_eq!(clamp_pct(val), expected, "failed for val: {val}");
        }
    }
}
