use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub const DEFAULT_FORMAT: &str =
    "$directory$git_branch$git_status$sandbox\n$model$context$agent_state$plan\n$quota";

fn default_format() -> String {
    DEFAULT_FORMAT.to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default = "default_format")]
    pub format: String,

    #[serde(default)]
    pub directory: DirectoryConfig,

    #[serde(default)]
    pub git_branch: GitBranchConfig,

    #[serde(default)]
    pub git_status: GitStatusConfig,

    #[serde(default)]
    pub sandbox: SandboxConfig,

    #[serde(default)]
    pub model: ModelConfig,

    #[serde(default)]
    pub context: ContextConfig,

    #[serde(default)]
    pub quota: QuotaConfig,

    #[serde(default)]
    pub agent_state: AgentStateConfig,

    #[serde(default)]
    pub plan: PlanConfig,

    #[serde(default)]
    pub integrations: IntegrationsConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            format: default_format(),
            directory: DirectoryConfig::default(),
            git_branch: GitBranchConfig::default(),
            git_status: GitStatusConfig::default(),
            sandbox: SandboxConfig::default(),
            model: ModelConfig::default(),
            context: ContextConfig::default(),
            quota: QuotaConfig::default(),
            agent_state: AgentStateConfig::default(),
            plan: PlanConfig::default(),
            integrations: IntegrationsConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DirectoryConfig {
    #[serde(default = "default_directory_format")]
    pub format: String,

    #[serde(default = "default_directory_folder_style")]
    pub folder_style: String,

    #[serde(default = "default_directory_path_style")]
    pub path_style: String,

    #[serde(default = "default_truncation_mode")]
    pub truncation_mode: String,

    #[serde(default)]
    pub disabled: bool,
}

fn default_directory_format() -> String {
    "📁 [$folder]($folder_style) [\\[$path\\]]($path_style) ".to_string()
}
fn default_directory_folder_style() -> String {
    "bold cyan".to_string()
}
fn default_directory_path_style() -> String {
    "dimmed".to_string()
}
fn default_truncation_mode() -> String {
    "middle".to_string()
}

impl Default for DirectoryConfig {
    fn default() -> Self {
        Self {
            format: default_directory_format(),
            folder_style: default_directory_folder_style(),
            path_style: default_directory_path_style(),
            truncation_mode: default_truncation_mode(),
            disabled: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitBranchConfig {
    #[serde(default = "default_git_branch_format")]
    pub format: String,

    #[serde(default = "default_git_branch_symbol")]
    pub symbol: String,

    #[serde(default = "default_git_branch_style")]
    pub style: String,

    #[serde(default)]
    pub disabled: bool,
}

fn default_git_branch_format() -> String {
    "[$symbol$branch]($style) ".to_string()
}
fn default_git_branch_symbol() -> String {
    "🌿 ".to_string()
}
fn default_git_branch_style() -> String {
    "green".to_string()
}

impl Default for GitBranchConfig {
    fn default() -> Self {
        Self {
            format: default_git_branch_format(),
            symbol: default_git_branch_symbol(),
            style: default_git_branch_style(),
            disabled: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitStatusConfig {
    #[serde(default = "default_git_status_format")]
    pub format: String,

    #[serde(default = "default_git_status_modified")]
    pub modified: String,

    #[serde(default = "default_git_status_style")]
    pub style: String,

    #[serde(default)]
    pub disabled: bool,
}

fn default_git_status_format() -> String {
    "[$status]($style) ".to_string()
}
fn default_git_status_modified() -> String {
    "✗".to_string()
}
fn default_git_status_style() -> String {
    "red".to_string()
}

impl Default for GitStatusConfig {
    fn default() -> Self {
        Self {
            format: default_git_status_format(),
            modified: default_git_status_modified(),
            style: default_git_status_style(),
            disabled: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SandboxConfig {
    #[serde(default = "default_sandbox_format")]
    pub format: String,

    #[serde(default = "default_sandbox_symbol")]
    pub symbol: String,

    #[serde(default = "default_sandbox_style")]
    pub style: String,

    #[serde(default)]
    pub disabled: bool,
}

fn default_sandbox_format() -> String {
    "[$symbol]($style) ".to_string()
}
fn default_sandbox_symbol() -> String {
    "🔒 sandbox".to_string()
}
fn default_sandbox_style() -> String {
    "yellow".to_string()
}

impl Default for SandboxConfig {
    fn default() -> Self {
        Self {
            format: default_sandbox_format(),
            symbol: default_sandbox_symbol(),
            style: default_sandbox_style(),
            disabled: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelConfig {
    #[serde(default = "default_model_format")]
    pub format: String,

    #[serde(default = "default_model_symbol")]
    pub symbol: String,

    #[serde(default = "default_model_style")]
    pub style: String,

    #[serde(default)]
    pub disabled: bool,
}

fn default_model_format() -> String {
    "[$symbol$model]($style) ".to_string()
}
fn default_model_symbol() -> String {
    "🤖 ".to_string()
}
fn default_model_style() -> String {
    "magenta".to_string()
}

impl Default for ModelConfig {
    fn default() -> Self {
        Self {
            format: default_model_format(),
            symbol: default_model_symbol(),
            style: default_model_style(),
            disabled: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextConfig {
    #[serde(default = "default_context_format")]
    pub format: String,

    #[serde(default = "default_context_symbol")]
    pub symbol: String,

    #[serde(default = "default_context_tok_style")]
    pub tok_style: String,

    #[serde(default = "default_threshold_warning")]
    pub threshold_warning: u8,

    #[serde(default = "default_threshold_critical")]
    pub threshold_critical: u8,

    #[serde(default = "default_style_normal")]
    pub style_normal: String,

    #[serde(default = "default_style_warning")]
    pub style_warning: String,

    #[serde(default = "default_style_critical")]
    pub style_critical: String,

    #[serde(default)]
    pub disabled: bool,
}

fn default_context_format() -> String {
    "[$symbolCtx: $percentage]($pct_style) [($tokens tok)]($tok_style) ".to_string()
}
fn default_context_symbol() -> String {
    "🧠 ".to_string()
}
fn default_context_tok_style() -> String {
    "dimmed".to_string()
}
fn default_threshold_warning() -> u8 {
    50
}
fn default_threshold_critical() -> u8 {
    80
}
fn default_style_normal() -> String {
    "green".to_string()
}
fn default_style_warning() -> String {
    "yellow".to_string()
}
fn default_style_critical() -> String {
    "red".to_string()
}

impl Default for ContextConfig {
    fn default() -> Self {
        Self {
            format: default_context_format(),
            symbol: default_context_symbol(),
            tok_style: default_context_tok_style(),
            threshold_warning: default_threshold_warning(),
            threshold_critical: default_threshold_critical(),
            style_normal: default_style_normal(),
            style_warning: default_style_warning(),
            style_critical: default_style_critical(),
            disabled: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuotaConfig {
    #[serde(default = "default_quota_format")]
    pub format: String,

    #[serde(default = "default_quota_symbol")]
    pub symbol: String,

    #[serde(default = "default_quota_separator")]
    pub separator: String,

    #[serde(default = "default_threshold_warning")]
    pub threshold_warning: u8,

    #[serde(default = "default_threshold_critical")]
    pub threshold_critical: u8,

    #[serde(default = "default_style_normal")]
    pub style_normal: String,

    #[serde(default = "default_style_warning")]
    pub style_warning: String,

    #[serde(default = "default_style_critical")]
    pub style_critical: String,

    #[serde(default)]
    pub disabled: bool,
}

fn default_quota_format() -> String {
    "[$symbol$label: $percentage (rst $reset_time)]($style) ".to_string()
}
fn default_quota_symbol() -> String {
    "⏳ ".to_string()
}
fn default_quota_separator() -> String {
    "  |  ".to_string()
}

impl Default for QuotaConfig {
    fn default() -> Self {
        Self {
            format: default_quota_format(),
            symbol: default_quota_symbol(),
            separator: default_quota_separator(),
            threshold_warning: default_threshold_warning(),
            threshold_critical: default_threshold_critical(),
            style_normal: default_style_normal(),
            style_warning: default_style_warning(),
            style_critical: default_style_critical(),
            disabled: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentStateConfig {
    #[serde(default = "default_agent_state_format")]
    pub format: String,

    #[serde(default = "default_agent_state_symbol")]
    pub symbol: String,

    #[serde(default = "default_agent_state_style")]
    pub style: String,

    #[serde(default)]
    pub disabled: bool,
}

fn default_agent_state_format() -> String {
    "[$symbol$state]($style) ".to_string()
}
fn default_agent_state_symbol() -> String {
    "⚡ ".to_string()
}
fn default_agent_state_style() -> String {
    "yellow".to_string()
}

impl Default for AgentStateConfig {
    fn default() -> Self {
        Self {
            format: default_agent_state_format(),
            symbol: default_agent_state_symbol(),
            style: default_agent_state_style(),
            disabled: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanConfig {
    #[serde(default = "default_plan_format")]
    pub format: String,

    #[serde(default = "default_plan_style")]
    pub style: String,

    #[serde(default)]
    pub disabled: bool,
}

fn default_plan_format() -> String {
    "[$tier]($style) ".to_string()
}
fn default_plan_style() -> String {
    "blue".to_string()
}

impl Default for PlanConfig {
    fn default() -> Self {
        Self {
            format: default_plan_format(),
            style: default_plan_style(),
            disabled: false,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct IntegrationsConfig {
    #[serde(default)]
    pub squirrel_notifier: SquirrelNotifierConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SquirrelNotifierConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,

    #[serde(default)]
    pub output_dir: Option<String>,
}

fn default_true() -> bool {
    true
}

impl Default for SquirrelNotifierConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            output_dir: None,
        }
    }
}

/// 設定ファイルパスを探索・解決する
pub fn resolve_config_path(custom_path: Option<&Path>) -> Option<PathBuf> {
    if let Some(path) = custom_path {
        return Some(path.to_path_buf());
    }

    if let Ok(env_path) = std::env::var("AGENT_STATUSLINE_CONFIG")
        && !env_path.trim().is_empty()
    {
        return Some(PathBuf::from(env_path));
    }

    if let Some(config_dir) = dirs::config_dir() {
        let p = config_dir.join("agent-statusline").join("config.toml");
        if p.exists() {
            return Some(p);
        }
    }

    if let Some(home) = dirs::home_dir() {
        let p = home
            .join(".config")
            .join("agent-statusline")
            .join("config.toml");
        if p.exists() {
            return Some(p);
        }
    }

    None
}

/// 設定ファイルをロードするか、存在しない・エラー時はデフォルト設定を返す
pub fn load_config(custom_path: Option<&Path>) -> Config {
    let Some(path) = resolve_config_path(custom_path) else {
        return Config::default();
    };

    if !path.exists() {
        return Config::default();
    }

    match std::fs::read_to_string(&path) {
        Ok(content) => match toml_edit::de::from_str::<Config>(&content) {
            Ok(cfg) => cfg,
            Err(err) => {
                let p = path.display();
                eprintln!("agent-statusline: failed to parse config at {p}: {err}");
                Config::default()
            }
        },
        Err(err) => {
            let p = path.display();
            eprintln!("agent-statusline: failed to read config at {p}: {err}");
            Config::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config_parses_and_matches() {
        let cfg = Config::default();
        assert_eq!(cfg.format, DEFAULT_FORMAT);
        assert_eq!(cfg.directory.folder_style, "bold cyan");
        assert_eq!(cfg.git_branch.symbol, "🌿 ");
        assert_eq!(cfg.context.threshold_warning, 50);
        assert_eq!(cfg.context.threshold_critical, 80);
        assert!(cfg.integrations.squirrel_notifier.enabled);
    }

    #[test]
    fn test_parse_custom_toml() {
        let toml_str = r#"
format = "$directory$git_branch\n$model"

[directory]
format = "[$folder]($folder_style)"
folder_style = "magenta"

[git_branch]
symbol = " "
style = "cyan"
"#;
        let cfg: Config = toml_edit::de::from_str(toml_str).unwrap();
        assert_eq!(cfg.format, "$directory$git_branch\n$model");
        assert_eq!(cfg.directory.folder_style, "magenta");
        assert_eq!(cfg.directory.format, "[$folder]($folder_style)");
        assert_eq!(cfg.git_branch.symbol, " ");
        assert_eq!(cfg.git_branch.style, "cyan");
        // 省略されたセクションはデフォルト値が適用される
        assert_eq!(cfg.model.symbol, "🤖 ");
    }

    #[test]
    fn test_load_config_from_file_and_fallback() {
        use tempfile::TempDir;

        let temp_dir = TempDir::new().unwrap();
        let valid_file = temp_dir.path().join("config.toml");
        std::fs::write(
            &valid_file,
            "format = \"$model\"\n[model]\nsymbol = \"🧠 \"\n",
        )
        .unwrap();

        // 1. 正常なファイルの読み込み
        let cfg = load_config(Some(&valid_file));
        assert_eq!(cfg.format, "$model");
        assert_eq!(cfg.model.symbol, "🧠 ");

        // 2. 存在しないファイル -> デフォルトにフォールバック
        let non_existent = temp_dir.path().join("missing.toml");
        let fallback_cfg = load_config(Some(&non_existent));
        assert_eq!(fallback_cfg.format, DEFAULT_FORMAT);

        // 3. 不正な TOML -> デフォルトにフォールバック
        let invalid_file = temp_dir.path().join("invalid.toml");
        std::fs::write(&invalid_file, "this is not valid toml = [[[").unwrap();
        let invalid_cfg = load_config(Some(&invalid_file));
        assert_eq!(invalid_cfg.format, DEFAULT_FORMAT);
    }
}
