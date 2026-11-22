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
        let copilot_home = non_empty_env_path("COPILOT_HOME");
        let codex_home = non_empty_env_path("CODEX_HOME");
        Some(Self::resolve_with_home_and_envs(
            agent,
            &home,
            copilot_home.as_deref(),
            codex_home.as_deref(),
        ))
    }

    #[allow(dead_code)]
    #[must_use]
    pub fn resolve_with_home(agent: AgentKind, home: &Path) -> Self {
        Self::resolve_with_home_and_envs(agent, home, None, None)
    }

    #[allow(dead_code)]
    #[must_use]
    pub fn resolve_with_home_and_env(
        agent: AgentKind,
        home: &Path,
        copilot_home: Option<&Path>,
    ) -> Self {
        Self::resolve_with_home_and_envs(agent, home, copilot_home, None)
    }

    #[must_use]
    pub fn resolve_with_home_and_envs(
        agent: AgentKind,
        home: &Path,
        copilot_home: Option<&Path>,
        codex_home: Option<&Path>,
    ) -> Self {
        let path = match agent {
            AgentKind::Agy => home
                .join(".gemini")
                .join("antigravity-cli")
                .join("settings.json"),
            AgentKind::Claude => home.join(".claude").join("settings.json"),
            AgentKind::Copilot => copilot_home
                .filter(|p| !p.as_os_str().is_empty())
                .map_or_else(
                    || home.join(".copilot").join("settings.json"),
                    |ch| ch.join("settings.json"),
                ),
            AgentKind::Codex => codex_home
                .filter(|p| !p.as_os_str().is_empty())
                .map_or_else(
                    || home.join(".codex").join("config.toml"),
                    |ch| ch.join("config.toml"),
                ),
        };
        Self { agent, path }
    }

    #[must_use]
    pub fn statusline_config_value(&self) -> Option<serde_json::Value> {
        let agent_str = self.agent.as_str();
        match self.agent {
            AgentKind::Agy => Some(json!({
                "type": "command",
                "command": format!("agent-statusline render --agent {agent_str}"),
                "enabled": true
            })),
            AgentKind::Claude | AgentKind::Copilot => Some(json!({
                "type": "command",
                "command": format!("agent-statusline render --agent {agent_str}")
            })),
            AgentKind::Codex => None,
        }
    }
}

fn non_empty_env_path(key: &str) -> Option<PathBuf> {
    std::env::var_os(key)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
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

        let codex = AgentConfigTarget::resolve_with_home(AgentKind::Codex, dummy_home);
        assert_eq!(codex.path, dummy_home.join(".codex").join("config.toml"));
    }

    #[test]
    fn test_resolve_with_custom_env_homes() {
        let dummy_home = Path::new("/mock/home");
        let custom_copilot_home = Path::new("/custom/copilot/dir");
        let custom_codex_home = Path::new("/custom/codex/dir");

        let copilot = AgentConfigTarget::resolve_with_home_and_env(
            AgentKind::Copilot,
            dummy_home,
            Some(custom_copilot_home),
        );
        assert_eq!(copilot.path, custom_copilot_home.join("settings.json"));

        let codex = AgentConfigTarget::resolve_with_home_and_envs(
            AgentKind::Codex,
            dummy_home,
            None,
            Some(custom_codex_home),
        );
        assert_eq!(codex.path, custom_codex_home.join("config.toml"));

        // 空パス指定時は既定ホームへフォールバックする
        let codex_empty = AgentConfigTarget::resolve_with_home_and_envs(
            AgentKind::Codex,
            dummy_home,
            None,
            Some(Path::new("")),
        );
        assert_eq!(
            codex_empty.path,
            dummy_home.join(".codex").join("config.toml")
        );
    }

    #[test]
    fn test_statusline_config_value() {
        let dummy_home = Path::new("/mock/home");

        let agy = AgentConfigTarget::resolve_with_home(AgentKind::Agy, dummy_home);
        let val = agy.statusline_config_value().unwrap();
        assert_eq!(val["type"], "command");
        assert_eq!(val["command"], "agent-statusline render --agent agy");
        assert_eq!(val["enabled"], true);

        let claude = AgentConfigTarget::resolve_with_home(AgentKind::Claude, dummy_home);
        let val = claude.statusline_config_value().unwrap();
        assert_eq!(val["type"], "command");
        assert_eq!(val["command"], "agent-statusline render --agent claude");
        assert!(val.get("enabled").is_none());

        let codex = AgentConfigTarget::resolve_with_home(AgentKind::Codex, dummy_home);
        assert!(codex.statusline_config_value().is_none());
    }
}
