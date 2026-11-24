use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};

use crate::model::review_summary::{
    ActiveReview, QueuedReview, RepoReviews, ReviewAlert, ReviewStatusDocument, ReviewSummary,
    SUPPORTED_SCHEMA_VERSION,
};
use crate::model::state::clamp_pct;

/// `review-status.json` の鮮度上限（秒）。Squirrel Notifier の 60 秒 heartbeat 契約に基づき 3 分超を古いとみなす
pub const REVIEW_STATUS_STALE_SECS: i64 = 180;

/// `%LOCALAPPDATA%/SquirrelNotifier/statusline-summary.json` 相当の既定パス
pub fn default_summary_path() -> Option<PathBuf> {
    dirs::data_local_dir().map(|dir| dir.join("SquirrelNotifier").join("statusline-summary.json"))
}

/// `%LOCALAPPDATA%/SquirrelNotifier/review-status.json` 相当の既定パス
pub fn default_status_path() -> Option<PathBuf> {
    dirs::data_local_dir().map(|dir| dir.join("SquirrelNotifier").join("review-status.json"))
}

/// サマリ JSON 文字列をパースする。破損・未対応スキーマは `None`
pub fn parse_summary(content: &str) -> Option<ReviewSummary> {
    let summary: ReviewSummary = serde_json::from_str(content).ok()?;
    (summary.schema_version == SUPPORTED_SCHEMA_VERSION).then_some(summary)
}

/// サマリを読み込む。ファイル不在 (Squirrel Notifier 未起動)・破損・未対応スキーマは `None`
pub fn load_summary(path: &Path) -> Option<ReviewSummary> {
    let content = std::fs::read_to_string(path).ok()?;
    parse_summary(&content)
}

/// `review-status.json` 文字列をパースする。破損・未対応スキーマは `None`
pub fn parse_review_status(content: &str) -> Option<ReviewStatusDocument> {
    let doc: ReviewStatusDocument = serde_json::from_str(content).ok()?;
    (doc.schema_version == SUPPORTED_SCHEMA_VERSION).then_some(doc)
}

/// `review-status.json` を読み込む。ファイル不在・破損・未対応スキーマは `None`
pub fn load_review_status(path: &Path) -> Option<ReviewStatusDocument> {
    let content = std::fs::read_to_string(path).ok()?;
    parse_review_status(&content)
}

/// 指定リポジトリ (`owner/repo`、大文字小文字を区別しない) のレビュー状態 (`statusline-summary.json`)
pub fn reviews_for_repo<'a>(
    summary: &'a ReviewSummary,
    repo: &str,
) -> (Vec<&'a ActiveReview>, Vec<&'a QueuedReview>) {
    let active = summary
        .active_reviews
        .iter()
        .filter(|r| r.repository.eq_ignore_ascii_case(repo))
        .collect();
    let waiting = summary
        .queue
        .items
        .iter()
        .filter(|r| r.repository.eq_ignore_ascii_case(repo))
        .collect();
    (active, waiting)
}

/// `review-status.json` から指定リポジトリのレビュー状態と異常警告を抽出する
pub fn reviews_from_status(doc: &ReviewStatusDocument, repo: &str) -> RepoReviews {
    reviews_from_status_at(doc, repo, Utc::now().timestamp())
}

/// 時刻を注入して `review-status.json` から抽出する（テスト用 DI）
pub fn reviews_from_status_at(doc: &ReviewStatusDocument, repo: &str, now_ts: i64) -> RepoReviews {
    if is_status_stale(doc.updated_at.as_deref(), now_ts) {
        // 異常終了で残った古い状態の PR 一覧を誤表示しないよう、鮮度切れ時は警告のみ返す
        return RepoReviews {
            active: Vec::new(),
            waiting: Vec::new(),
            alert: Some(ReviewAlert::Stale),
        };
    }

    let alert = doc
        .subscription
        .as_ref()
        .and_then(|sub| match sub.state.as_str() {
            "authRequired" => Some(ReviewAlert::AuthRequired),
            "error" => Some(ReviewAlert::SubscriptionError),
            "stopped" => Some(ReviewAlert::SubscriptionStopped),
            _ => None,
        });

    let mut active = Vec::new();
    let mut waiting = Vec::new();
    for item in &doc.items {
        if !item.repository.eq_ignore_ascii_case(repo) {
            continue;
        }
        match item.state.as_str() {
            "running" => active.push(ActiveReview {
                repository: item.repository.clone(),
                pr_number: item.pr_number,
                round: item.round,
                agent: item.agent.clone(),
                started_at: item.started_at.clone(),
            }),
            "waiting" => waiting.push(QueuedReview {
                repository: item.repository.clone(),
                pr_number: item.pr_number,
                round: item.round,
                reason: item.reason.clone(),
                hold_reason: item.hold_reason.clone(),
                queue_position: item.queue_position,
                auto_pause_used_pct: item
                    .auto_pause
                    .as_ref()
                    .and_then(|ap| ap.used_percentage)
                    .map(clamp_pct),
            }),
            _ => {}
        }
    }

    RepoReviews {
        active,
        waiting,
        alert,
    }
}

fn is_status_stale(updated_at: Option<&str>, now_ts: i64) -> bool {
    let Some(raw) = updated_at else {
        return true;
    };
    let Ok(parsed) = DateTime::parse_from_rfc3339(raw) else {
        return true;
    };
    now_ts.saturating_sub(parsed.timestamp()) > REVIEW_STATUS_STALE_SECS
}

/// 実行開始時刻（ISO 8601）から経過時間の短縮文字列を生成する（例: `45s`, `3m`, `1h2m`）
pub fn format_elapsed(started_at: Option<&str>, now_ts: i64) -> String {
    let Some(raw) = started_at else {
        return String::new();
    };
    let Ok(parsed) = DateTime::parse_from_rfc3339(raw) else {
        return String::new();
    };
    let secs = now_ts.saturating_sub(parsed.timestamp()).max(0);
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        format!("{}m", secs / 60)
    } else {
        format!("{}h{}m", secs / 3600, (secs % 3600) / 60)
    }
}

/// 待機理由（`holdReason`）と付随情報から簡潔なバッジ文字列を生成する
pub fn format_hold_badge(
    hold_reason: Option<&str>,
    queue_position: Option<u32>,
    auto_pause_used_pct: Option<u8>,
) -> String {
    let Some(reason) = hold_reason.map(str::trim).filter(|s| !s.is_empty()) else {
        return String::new();
    };
    match reason {
        "ciPending" => "(CI)".to_string(),
        "busy" => match queue_position {
            Some(pos) if pos > 0 => format!("(busy#{pos})"),
            _ => "(busy)".to_string(),
        },
        "autoPause" => match auto_pause_used_pct {
            Some(pct) => format!("(pause {pct}%)"),
            None => "(pause)".to_string(),
        },
        "manual" => "(manual)".to_string(),
        other => format!("({other})"),
    }
}

/// Squirrel Notifier の異常・鮮度切れ警告の短い表示文言
pub const fn format_alert(alert: ReviewAlert) -> &'static str {
    match alert {
        ReviewAlert::AuthRequired => "⚠auth",
        ReviewAlert::SubscriptionError => "⚠err",
        ReviewAlert::SubscriptionStopped => "⚠stopped",
        ReviewAlert::Stale => "⚠stale",
    }
}

/// GitHub PR URL を組み立てる。`owner/repo` 形式として安全でない場合は `None`
pub fn build_github_pr_url(repository: &str, pr_number: u64) -> Option<String> {
    let trimmed = repository.trim();
    let (owner, repo) = trimmed.split_once('/')?;
    if owner.is_empty()
        || repo.is_empty()
        || repo.contains('/')
        || !is_safe_repo_segment(owner)
        || !is_safe_repo_segment(repo)
    {
        return None;
    }
    Some(format!(
        "https://github.com/{owner}/{repo}/pull/{pr_number}"
    ))
}

fn is_safe_repo_segment(segment: &str) -> bool {
    segment
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SUMMARY: &str = r#"{
        "schemaVersion": 1,
        "updatedAt": "2026-09-27T00:00:00Z",
        "queue": {
            "totalWaiting": 2,
            "items": [
                { "repository": "scottlz0310/agent-statusline", "prNumber": 32, "round": 1, "reason": "opened" },
                { "repository": "scottlz0310/other", "prNumber": 5, "round": 1, "reason": "opened" }
            ]
        },
        "activeReviews": [
            { "repository": "scottlz0310/Agent-Statusline", "prNumber": 31, "round": 2, "agent": null }
        ]
    }"#;

    const STATUS_JSON: &str = r#"{
        "schemaVersion": 1,
        "updatedAt": "2026-11-24T10:00:00Z",
        "subscription": { "state": "running", "since": "2026-11-24T09:00:00Z" },
        "items": [
            {
                "key": "scottlz0310/agent-statusline#31",
                "repository": "scottlz0310/Agent-Statusline",
                "prNumber": 31,
                "round": 2,
                "reason": "synchronized",
                "state": "running",
                "startedAt": "2026-11-24T09:57:00Z",
                "agent": "codex"
            },
            {
                "key": "scottlz0310/agent-statusline#32",
                "repository": "scottlz0310/agent-statusline",
                "prNumber": 32,
                "round": 1,
                "reason": "opened",
                "state": "waiting",
                "holdReason": "autoPause",
                "queuePosition": 1,
                "autoPause": { "agentId": "codex", "usedPercentage": 96.4 }
            },
            {
                "key": "scottlz0310/other#9",
                "repository": "scottlz0310/other",
                "prNumber": 9,
                "round": 1,
                "reason": "opened",
                "state": "waiting",
                "holdReason": "ciPending"
            }
        ]
    }"#;

    #[test]
    fn test_load_summary() {
        let dir = tempfile::tempdir().unwrap();
        let cases = [
            ("valid", Some(SUMMARY), true),
            ("missing", None, false),
            ("broken", Some("{\"schemaVersion\": 1,"), false),
            ("future-schema", Some(r#"{"schemaVersion": 2}"#), false),
            (
                "missing-field",
                Some(r#"{"schemaVersion": 1, "activeReviews": [{"repository": "a/b"}]}"#),
                false,
            ),
            ("empty", Some(r#"{"schemaVersion": 1}"#), true),
        ];
        for (name, content, loaded) in cases {
            let path = dir.path().join(format!("{name}.json"));
            if let Some(content) = content {
                std::fs::write(&path, content).unwrap();
            }
            assert_eq!(load_summary(&path).is_some(), loaded, "{name}");
        }
    }

    #[test]
    fn test_reviews_for_repo() {
        let summary: ReviewSummary = serde_json::from_str(SUMMARY).unwrap();
        let cases = [
            ("scottlz0310/agent-statusline", vec![31], vec![32]),
            ("SCOTTLZ0310/AGENT-STATUSLINE", vec![31], vec![32]),
            ("scottlz0310/other", vec![], vec![5]),
            ("scottlz0310/none", vec![], vec![]),
        ];
        for (repo, active_prs, waiting_prs) in cases {
            let (active, waiting) = reviews_for_repo(&summary, repo);
            let active: Vec<u64> = active.iter().map(|r| r.pr_number).collect();
            let waiting: Vec<u64> = waiting.iter().map(|r| r.pr_number).collect();
            assert_eq!(active, active_prs, "{repo}");
            assert_eq!(waiting, waiting_prs, "{repo}");
        }
    }

    #[test]
    fn test_reviews_from_status_at_extracts_active_waiting_and_staleness() {
        let doc = parse_review_status(STATUS_JSON).unwrap();
        let updated_ts = DateTime::parse_from_rfc3339("2026-11-24T10:00:00Z")
            .unwrap()
            .timestamp();

        // 鮮度境界: ちょうど 180 秒は新鮮
        let fresh = reviews_from_status_at(&doc, "scottlz0310/agent-statusline", updated_ts + 180);
        assert_eq!(fresh.alert, None);
        assert_eq!(fresh.active.len(), 1);
        assert_eq!(fresh.active[0].pr_number, 31);
        assert_eq!(
            fresh.active[0].started_at.as_deref(),
            Some("2026-11-24T09:57:00Z")
        );
        assert_eq!(fresh.waiting.len(), 1);
        assert_eq!(fresh.waiting[0].pr_number, 32);
        assert_eq!(fresh.waiting[0].hold_reason.as_deref(), Some("autoPause"));
        assert_eq!(fresh.waiting[0].queue_position, Some(1));
        assert_eq!(fresh.waiting[0].auto_pause_used_pct, Some(96));

        // 181 秒は鮮度切れ: 古い PR は落とされ Stale 警告のみ返る
        let stale = reviews_from_status_at(&doc, "scottlz0310/agent-statusline", updated_ts + 181);
        assert_eq!(stale.alert, Some(ReviewAlert::Stale));
        assert!(stale.active.is_empty());
        assert!(stale.waiting.is_empty());
    }

    #[test]
    fn test_reviews_from_status_subscription_alerts() {
        let now = DateTime::parse_from_rfc3339("2026-11-24T10:01:00Z")
            .unwrap()
            .timestamp();
        let cases = [
            ("running", None),
            ("starting", None),
            ("authRequired", Some(ReviewAlert::AuthRequired)),
            ("error", Some(ReviewAlert::SubscriptionError)),
            ("stopped", Some(ReviewAlert::SubscriptionStopped)),
        ];
        for (state, expected) in cases {
            let raw = format!(
                r#"{{"schemaVersion":1,"updatedAt":"2026-11-24T10:00:00Z","subscription":{{"state":"{state}"}},"items":[]}}"#
            );
            let doc = parse_review_status(&raw).unwrap();
            let got = reviews_from_status_at(&doc, "o/r", now);
            assert_eq!(got.alert, expected, "{state}");
        }
    }

    #[test]
    fn test_format_hold_badge_and_elapsed() {
        let hold_cases = [
            (None, None, None, ""),
            (Some("  "), None, None, ""),
            (Some("ciPending"), None, None, "(CI)"),
            (Some("busy"), Some(2), None, "(busy#2)"),
            (Some("busy"), None, None, "(busy)"),
            (Some("autoPause"), None, Some(96), "(pause 96%)"),
            (Some("autoPause"), None, None, "(pause)"),
            (Some("manual"), None, None, "(manual)"),
            (Some("customHold"), None, None, "(customHold)"),
        ];
        for (reason, pos, pct, expected) in hold_cases {
            assert_eq!(format_hold_badge(reason, pos, pct), expected);
        }

        let base = DateTime::parse_from_rfc3339("2026-11-24T10:00:00Z")
            .unwrap()
            .timestamp();
        let elapsed_cases = [
            (None, base, ""),
            (Some("invalid"), base, ""),
            (Some("2026-11-24T10:00:00Z"), base - 10, "0s"),
            (Some("2026-11-24T10:00:00Z"), base + 45, "45s"),
            (Some("2026-11-24T10:00:00Z"), base + 185, "3m"),
            (Some("2026-11-24T10:00:00Z"), base + 3720, "1h2m"),
        ];
        for (started, now, expected) in elapsed_cases {
            assert_eq!(format_elapsed(started, now), expected);
        }
    }

    #[test]
    fn test_build_github_pr_url() {
        let cases = [
            (
                "scottlz0310/agent-statusline",
                149,
                Some("https://github.com/scottlz0310/agent-statusline/pull/149"),
            ),
            (
                "owner.1/repo_2",
                7,
                Some("https://github.com/owner.1/repo_2/pull/7"),
            ),
            ("invalid", 1, None),
            ("a/b/c", 1, None),
            ("owner/repo?x=1", 1, None),
            ("/repo", 1, None),
        ];
        for (repo, pr, expected) in cases {
            assert_eq!(build_github_pr_url(repo, pr).as_deref(), expected, "{repo}");
        }
    }
}
