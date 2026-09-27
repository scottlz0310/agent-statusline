use serde::Deserialize;

/// Squirrel Notifier が書き出す `statusline-summary.json` の対応スキーマバージョン
pub const SUPPORTED_SCHEMA_VERSION: u32 = 1;

/// Squirrel Notifier のレビュー・キュー状態サマリ (`statusline-summary.json`)
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewSummary {
    pub schema_version: u32,
    #[serde(default)]
    pub queue: ReviewQueue,
    #[serde(default)]
    pub active_reviews: Vec<ActiveReview>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct ReviewQueue {
    #[serde(default)]
    pub items: Vec<QueuedReview>,
}

/// reviewer の起動を待っている PR
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueuedReview {
    pub repository: String,
    pub pr_number: u64,
    pub round: u32,
    pub reason: String,
}

/// reviewer を実行中の PR
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActiveReview {
    pub repository: String,
    pub pr_number: u64,
    pub round: u32,
    pub agent: Option<String>,
}
