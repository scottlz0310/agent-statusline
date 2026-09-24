use serde::{Deserialize, Serialize};

/// Squirrel Notifier 共通レートリミットスキーマ (schemaVersion: 1)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RatelimitPayload {
    pub schema_version: u32,
    pub agent_id: String,
    pub observed_at: String,
    pub limits: Vec<RatelimitLimitItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RatelimitLimitItem {
    pub id: String,
    pub label: String,
    pub reset_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub used_percentage: Option<u8>,
}
