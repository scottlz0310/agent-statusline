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
