pub mod client;
pub mod patcher;

pub use client::AgentConfigTarget;
pub use patcher::{
    AgentStatusReport, PatchAction, PatchResult, StatusKind, diagnose_agent, install_to_agent,
    uninstall_from_agent,
};
