#![allow(clippy::unnecessary_wraps)]

use crate::cli::AgentKind;
use crate::installer::client::AgentConfigTarget;

pub fn install_to_agent(_target: &AgentConfigTarget, _dry_run: bool) -> std::io::Result<()> {
    // Phase 2 で本実装
    Ok(())
}

pub fn uninstall_from_agent(_target: &AgentConfigTarget) -> std::io::Result<()> {
    // Phase 2 で本実装
    Ok(())
}

pub fn check_status(_agent: AgentKind) {
    // Phase 2 で本実装
}
