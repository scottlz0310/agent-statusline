use std::path::{Path, PathBuf};

use crate::model::review_summary::{
    ActiveReview, QueuedReview, ReviewSummary, SUPPORTED_SCHEMA_VERSION,
};

/// `%LOCALAPPDATA%/SquirrelNotifier/statusline-summary.json` 相当の既定パス
pub fn default_summary_path() -> Option<PathBuf> {
    dirs::data_local_dir().map(|dir| dir.join("SquirrelNotifier").join("statusline-summary.json"))
}

/// サマリを読み込む。ファイル不在 (Squirrel Notifier 未起動)・破損・未対応スキーマは `None`
pub fn load_summary(path: &Path) -> Option<ReviewSummary> {
    let content = std::fs::read_to_string(path).ok()?;
    let summary: ReviewSummary = serde_json::from_str(&content).ok()?;
    (summary.schema_version == SUPPORTED_SCHEMA_VERSION).then_some(summary)
}

/// 指定リポジトリ (`owner/repo`、大文字小文字を区別しない) のレビュー状態
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
}
