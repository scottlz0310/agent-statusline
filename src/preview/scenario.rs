//! プレビューのシナリオ。実環境から独立した、あらかじめ用意したサンプル状態（設計書 §6.7）。
//!
//! Git・サマリ・更新確認・Notifier には触れない。プレビューは、ここで作った入力を `compose` へ渡すだけにする。

use std::path::PathBuf;

use crate::model::review_summary::{ActiveReview, QueuedReview, RepoReviews};
use crate::model::state::{QuotaItem, StatuslineState};
use crate::modules::git::GitStatus;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scenario {
    Normal,
    LongPathAndBranch,
    ContextWarning,
    ContextCritical,
    QuotaTight,
    ReviewWaiting,
    ReviewActive,
    AgentIdle,
}

/// シナリオが `compose` へ渡す入力
#[derive(Debug, Clone)]
pub struct SampleInputs {
    pub state: StatuslineState,
    pub git: Option<GitStatus>,
    pub reviews: RepoReviews,
}

impl Scenario {
    pub const ALL: [Self; 8] = [
        Self::Normal,
        Self::LongPathAndBranch,
        Self::ContextWarning,
        Self::ContextCritical,
        Self::QuotaTight,
        Self::ReviewWaiting,
        Self::ReviewActive,
        Self::AgentIdle,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Normal => "通常",
            Self::LongPathAndBranch => "長いパス・ブランチ",
            Self::ContextWarning => "コンテキスト警告",
            Self::ContextCritical => "コンテキスト上限間近",
            Self::QuotaTight => "クォータ逼迫",
            Self::ReviewWaiting => "レビュー待機",
            Self::ReviewActive => "レビュー実行中",
            Self::AgentIdle => "エージェント待機",
        }
    }

    pub fn inputs(self) -> SampleInputs {
        let mut inputs = SampleInputs {
            state: normal_state(),
            git: Some(git("main", true)),
            reviews: RepoReviews::default(),
        };
        match self {
            Self::Normal => {}
            Self::LongPathAndBranch => {
                inputs.state.cwd = PathBuf::from(
                    "/home/dev/src/organization/very-long-repository-name/packages/deeply/nested/workspace",
                );
                inputs.git = Some(git(
                    "feature/very-long-branch-name-for-preview-testing",
                    true,
                ));
            }
            Self::ContextWarning => inputs.state.context_used_percentage = Some(65),
            Self::ContextCritical => inputs.state.context_used_percentage = Some(92),
            Self::QuotaTight => {
                inputs.state.quotas = vec![
                    quota("5h", 88, Some(25 * 60)),
                    quota("7d", 96, Some(6 * 3600 + 40 * 60)),
                    quota("Opus 7d", 71, None),
                ];
            }
            Self::ReviewWaiting => {
                inputs.reviews.waiting = vec![queued(32, 1, "opened")];
            }
            Self::ReviewActive => {
                inputs.reviews.active = vec![active(31, 2, Some("claude"))];
                inputs.reviews.waiting = vec![queued(32, 1, "opened")];
            }
            Self::AgentIdle => inputs.state.agent_state = Some("idle".to_string()),
        }
        inputs
    }
}

fn normal_state() -> StatuslineState {
    StatuslineState {
        cwd: PathBuf::from("/home/dev/src/myproject"),
        model: Some("Claude Opus".to_string()),
        context_used_percentage: Some(42),
        total_input_tokens: Some(30_000),
        total_output_tokens: Some(12_000),
        agent_state: Some("working".to_string()),
        sandbox_enabled: true,
        plan_tier: Some("Pro".to_string()),
        quotas: vec![
            quota("5h", 12, Some(3 * 3600)),
            quota("7d", 30, Some(2 * 86400 + 3 * 3600)),
        ],
    }
}

fn git(branch: &str, is_dirty: bool) -> GitStatus {
    GitStatus {
        branch: Some(branch.to_string()),
        is_dirty,
        remote_repo: Some("owner/myproject".to_string()),
    }
}

fn quota(label: &str, pct: u8, reset_in_seconds: Option<i64>) -> QuotaItem {
    QuotaItem {
        id: label.to_string(),
        label: label.to_string(),
        used_percentage: Some(pct),
        reset_in_seconds,
        reset_time_iso: None,
    }
}

fn active(pr_number: u64, round: u32, agent: Option<&str>) -> ActiveReview {
    ActiveReview {
        repository: "owner/myproject".to_string(),
        pr_number,
        round,
        agent: agent.map(str::to_string),
    }
}

fn queued(pr_number: u64, round: u32, reason: &str) -> QueuedReview {
    QueuedReview {
        repository: "owner/myproject".to_string(),
        pr_number,
        round,
        reason: reason.to_string(),
    }
}
