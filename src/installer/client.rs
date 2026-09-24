use crate::cli::AgentKind;
use std::path::PathBuf;

pub struct AgentConfigTarget {
    pub agent: AgentKind,
    pub path: PathBuf,
}

impl AgentConfigTarget {
    pub fn resolve(agent: AgentKind) -> Option<Self> {
        let home = dirs::home_dir()?;
        let path = match agent {
            AgentKind::Agy => home
                .join(".gemini")
                .join("antigravity-cli")
                .join("settings.json"),
            AgentKind::Claude => home.join(".claude").join("settings.json"),
            AgentKind::Copilot => home.join(".copilot").join("config.json"),
        };
        Some(Self { agent, path })
    }
}
