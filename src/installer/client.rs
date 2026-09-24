use crate::cli::AgentKind;
use serde_json::json;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentConfigTarget {
    pub agent: AgentKind,
    pub path: PathBuf,
}

impl AgentConfigTarget {
    #[must_use]
    pub fn resolve(agent: AgentKind) -> Option<Self> {
        let home = dirs::home_dir()?;
        let copilot_home = std::env::var_os("COPILOT_HOME").map(PathBuf::from);
        Some(Self::resolve_with_home_and_env(
            agent,
            &home,
            copilot_home.as_deref(),
        ))
    }

    #[allow(dead_code)]
    #[must_use]
    pub fn resolve_with_home(agent: AgentKind, home: &Path) -> Self {
        Self::resolve_with_home_and_env(agent, home, None)
    }

    #[must_use]
    pub fn resolve_with_home_and_env(
        agent: AgentKind,
        home: &Path,
        copilot_home: Option<&Path>,
    ) -> Self {
        let path = match agent {
            AgentKind::Agy => home
                .join(".gemini")
                .join("antigravity-cli")
                .join("settings.json"),
            AgentKind::Claude => home.join(".claude").join("settings.json"),
            AgentKind::Copilot => {
                if let Some(ch) = copilot_home {
                    ch.join("settings.json")
                } else {
                    home.join(".copilot").join("settings.json")
                }
            }
        };
        Self { agent, path }
    }

    #[must_use]
    pub fn statusline_config_value(&self) -> serde_json::Value {
        let agent_str = self.agent.as_str();
        match self.agent {
            AgentKind::Agy => json!({
                "type": "command",
                "command": format!("agent-statusline render --agent {agent_str}"),
                "enabled": true
            }),
            AgentKind::Claude | AgentKind::Copilot => json!({
                "type": "command",
                "command": format!("agent-statusline render --agent {agent_str}")
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_with_home() {
        let dummy_home = Path::new("/mock/home");

        let agy = AgentConfigTarget::resolve_with_home(AgentKind::Agy, dummy_home);
        assert_eq!(
            agy.path,
            dummy_home
                .join(".gemini")
                .join("antigravity-cli")
                .join("settings.json")
        );

        let claude = AgentConfigTarget::resolve_with_home(AgentKind::Claude, dummy_home);
        assert_eq!(
            claude.path,
            dummy_home.join(".claude").join("settings.json")
        );

        let copilot = AgentConfigTarget::resolve_with_home(AgentKind::Copilot, dummy_home);
        assert_eq!(
            copilot.path,
            dummy_home.join(".copilot").join("settings.json")
        );
    }

    #[test]
    fn test_resolve_with_copilot_home() {
        let dummy_home = Path::new("/mock/home");
        let custom_copilot_home = Path::new("/custom/copilot/dir");

        let copilot = AgentConfigTarget::resolve_with_home_and_env(
            AgentKind::Copilot,
            dummy_home,
            Some(custom_copilot_home),
        );
        assert_eq!(copilot.path, custom_copilot_home.join("settings.json"));
    }

    #[test]
    fn test_statusline_config_value() {
        let dummy_home = Path::new("/mock/home");

        let agy = AgentConfigTarget::resolve_with_home(AgentKind::Agy, dummy_home);
        let val = agy.statusline_config_value();
        assert_eq!(val["type"], "command");
        assert_eq!(val["command"], "agent-statusline render --agent agy");
        assert_eq!(val["enabled"], true);

        let claude = AgentConfigTarget::resolve_with_home(AgentKind::Claude, dummy_home);
        let val = claude.statusline_config_value();
        assert_eq!(val["type"], "command");
        assert_eq!(val["command"], "agent-statusline render --agent claude");
        assert!(val.get("enabled").is_none());
    }
}
