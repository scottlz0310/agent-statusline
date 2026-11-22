pub mod client;
pub mod codex;
pub mod patcher;

use std::io;

use crate::cli::AgentKind;

pub use client::AgentConfigTarget;
pub use patcher::{AgentStatusReport, PatchAction, PatchResult, StatusKind};

/// 対象エージェントの設定ファイルに statusline を登録・更新する
pub fn install_to_agent(target: &AgentConfigTarget, dry_run: bool) -> io::Result<PatchResult> {
    match target.agent {
        AgentKind::Codex => codex::install_codex(target, dry_run),
        AgentKind::Agy | AgentKind::Claude | AgentKind::Copilot => {
            patcher::install_json_agent(target, dry_run)
        }
    }
}

/// 対象エージェントの設定ファイルから statusline 設定を解除する
pub fn uninstall_from_agent(target: &AgentConfigTarget, dry_run: bool) -> io::Result<PatchResult> {
    match target.agent {
        AgentKind::Codex => codex::uninstall_codex(target, dry_run),
        AgentKind::Agy | AgentKind::Claude | AgentKind::Copilot => {
            patcher::uninstall_json_agent(target, dry_run)
        }
    }
}

/// 対象エージェントの設定ファイルの statusline 状態を診断する
#[must_use]
pub fn diagnose_agent(target: &AgentConfigTarget) -> AgentStatusReport {
    match target.agent {
        AgentKind::Codex => codex::diagnose_codex(target),
        AgentKind::Agy | AgentKind::Claude | AgentKind::Copilot => {
            patcher::diagnose_json_agent(target)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_dispatch_codex_roundtrip() {
        let dir = TempDir::new().unwrap();
        let target = AgentConfigTarget::resolve_with_home(AgentKind::Codex, dir.path());

        assert_eq!(diagnose_agent(&target).status, StatusKind::FileNotFound);
        assert_eq!(
            install_to_agent(&target, false).unwrap().action,
            PatchAction::Installed
        );
        assert_eq!(diagnose_agent(&target).status, StatusKind::Installed);
        assert_eq!(
            uninstall_from_agent(&target, false).unwrap().action,
            PatchAction::Uninstalled
        );
        assert_eq!(diagnose_agent(&target).status, StatusKind::NotConfigured);
    }
}
