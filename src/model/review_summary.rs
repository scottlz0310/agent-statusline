use serde::Deserialize;

/// Squirrel Notifier が書き出す公開契約 (`statusline-summary.json` / `review-status.json`) の対応スキーマバージョン
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

/// Squirrel Notifier の詳細公開状態 (`review-status.json`, `schemaVersion: 1`)
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewStatusDocument {
    pub schema_version: u32,
    #[serde(default)]
    pub updated_at: Option<String>,
    #[serde(default)]
    pub subscription: Option<ReviewSubscription>,
    #[serde(default)]
    pub items: Vec<ReviewStatusItem>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewSubscription {
    pub state: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewStatusItem {
    pub repository: String,
    pub pr_number: u64,
    pub round: u32,
    #[serde(default)]
    pub reason: String,
    pub state: String,
    #[serde(default)]
    pub started_at: Option<String>,
    #[serde(default)]
    pub agent: Option<String>,
    #[serde(default)]
    pub hold_reason: Option<String>,
    #[serde(default)]
    pub queue_position: Option<u32>,
    #[serde(default)]
    pub auto_pause: Option<ReviewAutoPause>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewAutoPause {
    #[serde(default)]
    pub used_percentage: Option<f64>,
}

/// Squirrel Notifier の購読異常・鮮度切れ警告
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReviewAlert {
    /// 購読に再ログインが必要 (`subscription.state == "authRequired"`)
    AuthRequired,
    /// 購読エラー (`subscription.state == "error"`)
    SubscriptionError,
    /// 購読停止中 (`subscription.state == "stopped"`)
    SubscriptionStopped,
    /// `review-status.json` の `updatedAt` が 3 分超古い（Squirrel 停止の可能性）
    Stale,
}

/// reviewer を実行中の PR
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActiveReview {
    pub repository: String,
    pub pr_number: u64,
    pub round: u32,
    pub agent: Option<String>,
    #[serde(default)]
    pub started_at: Option<String>,
}

/// reviewer の起動を待っている PR
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueuedReview {
    pub repository: String,
    pub pr_number: u64,
    pub round: u32,
    pub reason: String,
    #[serde(default)]
    pub hold_reason: Option<String>,
    #[serde(default)]
    pub queue_position: Option<u32>,
    #[serde(default)]
    pub auto_pause_used_pct: Option<u8>,
}

/// 1 つのリポジトリに属するレビュー状態。描画の入力として渡す
#[derive(Debug, Clone, Default)]
pub struct RepoReviews {
    pub active: Vec<ActiveReview>,
    pub waiting: Vec<QueuedReview>,
    pub alert: Option<ReviewAlert>,
}
