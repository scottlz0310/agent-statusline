//! Codex CLI (`$CODEX_HOME/config.toml`) 向け `toml_edit` ベースの設定管理・診断。
//!
//! 契約根拠: `openai/codex` `rust-v0.171.0`
//! - `[features] tui_extensions_statusline = true`
//! - `[mcp_servers.agent-statusline]` (`command = "agent-statusline"`, `args = ["mcp", "codex"]`)
//! - `[tui] status_line_provider = { server = "agent-statusline", tool = "statusline" }`

use std::fs;
use std::io::{self, Error, ErrorKind};
use std::path::{Path, PathBuf};

use clap::Parser;
use serde::{Deserialize, Serialize};
use toml_edit::{Array, DocumentMut, InlineTable, Item, Table, TableLike, Value};

use crate::cli::{Cli, Commands, McpTarget};
use crate::installer::client::AgentConfigTarget;
use crate::installer::patcher::{
    AgentStatusReport, PatchAction, PatchResult, StatusKind, atomic_write, make_backup_path,
};
use crate::mcp::protocol::{SERVER_NAME, TOOL_NAME};

const FEATURES_TABLE: &str = "features";
const FEATURE_STATUSLINE_KEY: &str = "tui_extensions_statusline";
const MCP_SERVERS_TABLE: &str = "mcp_servers";
const TUI_TABLE: &str = "tui";
const STATUS_LINE_PROVIDER_KEY: &str = "status_line_provider";

const CODEX_COMMAND: &str = "agent-statusline";
const CODEX_ARGS: [&str; 2] = ["mcp", "codex"];
const CODEX_COMMAND_DISPLAY: &str = "agent-statusline mcp codex";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct SavedProvider {
    server: String,
    tool: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum McpServerState {
    Missing,
    Managed {
        command_display: String,
        enabled: bool,
    },
    OtherCommand(String),
    Invalid(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ProviderState {
    DefaultOrMissing,
    AgentStatusline,
    Custom { server: String, tool: String },
    Invalid(String),
}

/// Codex CLI の `config.toml` に MCP サーバー・feature flag・statusline provider を登録する
pub fn install_codex(target: &AgentConfigTarget, dry_run: bool) -> io::Result<PatchResult> {
    if target.path.exists() {
        let raw_content = fs::read_to_string(&target.path)?;
        let mut doc = parse_toml_doc(&raw_content)?;
        let previous_provider = inspect_provider(&doc);

        let changed = apply_codex_install(&mut doc)?;
        if !changed {
            return Ok(PatchResult {
                agent: target.agent,
                path: target.path.clone(),
                action: PatchAction::AlreadyInstalled,
                backup_path: None,
                preview: None,
            });
        }

        let updated_str = ensure_trailing_newline(doc.to_string());
        if dry_run {
            return Ok(PatchResult {
                agent: target.agent,
                path: target.path.clone(),
                action: PatchAction::Installed,
                backup_path: None,
                preview: Some(updated_str),
            });
        }

        let backup_path = make_backup_path(&target.path);
        fs::copy(&target.path, &backup_path)?;
        sync_origin_provider_file(&target.path, &previous_provider)?;
        atomic_write(&target.path, updated_str.as_bytes())?;

        Ok(PatchResult {
            agent: target.agent,
            path: target.path.clone(),
            action: PatchAction::Installed,
            backup_path: Some(backup_path),
            preview: None,
        })
    } else {
        let mut doc = DocumentMut::new();
        apply_codex_install(&mut doc)?;
        let updated_str = ensure_trailing_newline(doc.to_string());

        if dry_run {
            return Ok(PatchResult {
                agent: target.agent,
                path: target.path.clone(),
                action: PatchAction::Installed,
                backup_path: None,
                preview: Some(updated_str),
            });
        }

        if let Some(parent) = target.path.parent() {
            fs::create_dir_all(parent)?;
        }
        remove_origin_provider_file(&target.path);
        atomic_write(&target.path, updated_str.as_bytes())?;

        Ok(PatchResult {
            agent: target.agent,
            path: target.path.clone(),
            action: PatchAction::Installed,
            backup_path: None,
            preview: None,
        })
    }
}

/// Codex CLI の `config.toml` から `agent-statusline` 設定を安全に解除する
pub fn uninstall_codex(target: &AgentConfigTarget, dry_run: bool) -> io::Result<PatchResult> {
    uninstall_codex_with_origin_reader(target, dry_run, |path| fs::read_to_string(path))
}

fn uninstall_codex_with_origin_reader<F>(
    target: &AgentConfigTarget,
    dry_run: bool,
    read_origin: F,
) -> io::Result<PatchResult>
where
    F: FnOnce(&Path) -> io::Result<String>,
{
    if !target.path.exists() {
        return Ok(PatchResult {
            agent: target.agent,
            path: target.path.clone(),
            action: PatchAction::NotInstalled,
            backup_path: None,
            preview: None,
        });
    }

    let raw_content = fs::read_to_string(&target.path)?;
    if raw_content.trim().is_empty() {
        return Ok(PatchResult {
            agent: target.agent,
            path: target.path.clone(),
            action: PatchAction::NotInstalled,
            backup_path: None,
            preview: None,
        });
    }

    let mut doc = parse_toml_doc(&raw_content)?;
    let mcp_state = inspect_mcp_server(&doc);
    let provider_state = inspect_provider(&doc);

    let has_managed_mcp = matches!(mcp_state, McpServerState::Managed { .. });
    let has_our_provider = matches!(provider_state, ProviderState::AgentStatusline);

    if !has_managed_mcp && !has_our_provider {
        let is_custom = matches!(provider_state, ProviderState::Custom { .. })
            || matches!(mcp_state, McpServerState::OtherCommand(_));
        let action = if is_custom {
            PatchAction::SkippedCustom
        } else {
            PatchAction::NotInstalled
        };
        return Ok(PatchResult {
            agent: target.agent,
            path: target.path.clone(),
            action,
            backup_path: None,
            preview: None,
        });
    }

    // `.bak` 上書き前に元の独自 provider を読み出しておく（専用保存先を優先し不在時のみ `.bak` へフォールバック）
    // 専用保存先の読み取り・JSON 解析エラーは握りつぶさず、設定と復元情報を保持したまま失敗させる。
    let backup_path = make_backup_path(&target.path);
    let restored_provider = if has_our_provider {
        try_restore_custom_provider(&target.path, &backup_path, read_origin)?
    } else {
        None
    };
    let restored = restored_provider.is_some();

    if has_managed_mcp {
        remove_managed_mcp_server(&mut doc);
    }

    if has_our_provider {
        if let Some(custom_provider) = restored_provider {
            restore_provider_item(&mut doc, custom_provider)?;
        } else {
            remove_provider_key(&mut doc);
        }
    }

    let updated_str = ensure_trailing_newline(doc.to_string());
    let action = if restored {
        PatchAction::RestoredBackup
    } else {
        PatchAction::Uninstalled
    };

    if dry_run {
        return Ok(PatchResult {
            agent: target.agent,
            path: target.path.clone(),
            action,
            backup_path: None,
            preview: Some(updated_str),
        });
    }

    fs::copy(&target.path, &backup_path)?;
    atomic_write(&target.path, updated_str.as_bytes())?;
    remove_origin_provider_file(&target.path);

    Ok(PatchResult {
        agent: target.agent,
        path: target.path.clone(),
        action,
        backup_path: Some(backup_path),
        preview: None,
    })
}

/// Codex CLI の `config.toml` の 3 箇所の整合性を診断する
#[must_use]
pub fn diagnose_codex(target: &AgentConfigTarget) -> AgentStatusReport {
    if !target.path.exists() {
        return report_with_status(target, StatusKind::FileNotFound);
    }

    let raw_content = match fs::read_to_string(&target.path) {
        Ok(c) => c,
        Err(e) => return report_with_status(target, StatusKind::InvalidConfig(e.to_string())),
    };

    if raw_content.trim().is_empty() {
        return report_with_status(target, StatusKind::NotConfigured);
    }

    let doc = match parse_toml_doc(&raw_content) {
        Ok(d) => d,
        Err(e) => return report_with_status(target, StatusKind::InvalidConfig(e.to_string())),
    };

    diagnose_codex_doc(target, &doc)
}

fn diagnose_codex_doc(target: &AgentConfigTarget, doc: &DocumentMut) -> AgentStatusReport {
    if let Err(err) = validate_root_tables(doc) {
        return report_with_status(target, StatusKind::InvalidConfig(err));
    }

    let feature_enabled = is_feature_enabled(doc);
    let mcp_state = inspect_mcp_server(doc);
    let provider_state = inspect_provider(doc);

    if let McpServerState::Invalid(err) = &mcp_state {
        return report_with_status(target, StatusKind::InvalidConfig(err.clone()));
    }

    // 3 箇所すべて正常
    if feature_enabled
        && let McpServerState::Managed {
            command_display,
            enabled: true,
        } = &mcp_state
        && matches!(provider_state, ProviderState::AgentStatusline)
    {
        return AgentStatusReport {
            agent: target.agent,
            path: target.path.clone(),
            status: StatusKind::Installed,
            command: Some(command_display.clone()),
            enabled: Some(true),
        };
    }

    // agent-statusline が無関係なケースの仕分け
    if matches!(mcp_state, McpServerState::Missing) {
        return match provider_state {
            ProviderState::DefaultOrMissing => {
                report_with_status(target, StatusKind::NotConfigured)
            }
            ProviderState::Custom { server, tool } => {
                custom_provider_report(target, &server, &tool)
            }
            ProviderState::Invalid(err) => {
                report_with_status(target, StatusKind::InvalidConfig(err))
            }
            ProviderState::AgentStatusline => {
                build_incomplete_report(target, feature_enabled, &mcp_state, &provider_state)
            }
        };
    }

    // `[mcp_servers.agent-statusline]` が別コマンドかつ別プロバイダー選択中なら独自環境として扱う
    if let McpServerState::OtherCommand(_) = &mcp_state
        && let ProviderState::Custom { server, tool } = &provider_state
    {
        return custom_provider_report(target, server, tool);
    }

    build_incomplete_report(target, feature_enabled, &mcp_state, &provider_state)
}

fn report_with_status(target: &AgentConfigTarget, status: StatusKind) -> AgentStatusReport {
    AgentStatusReport {
        agent: target.agent,
        path: target.path.clone(),
        status,
        command: None,
        enabled: None,
    }
}

fn custom_provider_report(
    target: &AgentConfigTarget,
    server: &str,
    tool: &str,
) -> AgentStatusReport {
    let label = format!("{server}/{tool}");
    AgentStatusReport {
        agent: target.agent,
        path: target.path.clone(),
        status: StatusKind::Custom(label.clone()),
        command: Some(label),
        enabled: None,
    }
}

fn build_incomplete_report(
    target: &AgentConfigTarget,
    feature_enabled: bool,
    mcp_state: &McpServerState,
    provider_state: &ProviderState,
) -> AgentStatusReport {
    let mut issues = Vec::new();

    if !feature_enabled {
        issues.push(format!(
            "[{FEATURES_TABLE}] {FEATURE_STATUSLINE_KEY} is not true"
        ));
    }

    let (command, enabled) = match mcp_state {
        McpServerState::Managed {
            command_display,
            enabled: true,
        } => (Some(command_display.clone()), Some(true)),
        McpServerState::Managed {
            command_display,
            enabled: false,
        } => {
            issues.push(format!(
                "[{MCP_SERVERS_TABLE}.{SERVER_NAME}] is disabled (enabled = false)"
            ));
            (Some(command_display.clone()), Some(false))
        }
        McpServerState::Missing => {
            issues.push(format!("[{MCP_SERVERS_TABLE}.{SERVER_NAME}] is missing"));
            (None, None)
        }
        McpServerState::OtherCommand(cmd) => {
            issues.push(format!(
                "[{MCP_SERVERS_TABLE}.{SERVER_NAME}] uses a different command ({cmd})"
            ));
            (Some(cmd.clone()), None)
        }
        McpServerState::Invalid(err) => {
            issues.push(err.clone());
            (None, None)
        }
    };

    match provider_state {
        ProviderState::AgentStatusline => {}
        ProviderState::DefaultOrMissing => {
            issues.push(format!(
                "[{TUI_TABLE}] {STATUS_LINE_PROVIDER_KEY} is not set to {SERVER_NAME}/{TOOL_NAME}"
            ));
        }
        ProviderState::Custom { server, tool } => {
            issues.push(format!(
                "[{TUI_TABLE}] {STATUS_LINE_PROVIDER_KEY} selects another provider ({server}/{tool})"
            ));
        }
        ProviderState::Invalid(err) => {
            issues.push(err.clone());
        }
    }

    AgentStatusReport {
        agent: target.agent,
        path: target.path.clone(),
        status: StatusKind::Incomplete(issues),
        command,
        enabled,
    }
}

fn parse_toml_doc(raw: &str) -> io::Result<DocumentMut> {
    raw.parse::<DocumentMut>()
        .map_err(|e| Error::new(ErrorKind::InvalidData, format!("Invalid TOML: {e}")))
}

fn validate_root_tables(doc: &DocumentMut) -> Result<(), String> {
    for key in [FEATURES_TABLE, MCP_SERVERS_TABLE, TUI_TABLE] {
        if let Some(item) = doc.get(key)
            && !item.is_table_like()
        {
            return Err(format!("[{key}] must be a TOML table"));
        }
    }
    Ok(())
}

fn apply_codex_install(doc: &mut DocumentMut) -> io::Result<bool> {
    // 変更前に同名 MCP サーバーの競合・破損を検査し、他サービスなら設定を一切触らず拒否する
    let existing_mcp = inspect_mcp_server(doc);
    match &existing_mcp {
        McpServerState::OtherCommand(desc) => {
            return Err(Error::new(
                ErrorKind::AlreadyExists,
                format!(
                    "[{MCP_SERVERS_TABLE}.{SERVER_NAME}] is already configured for another server ({desc}); refusing to overwrite"
                ),
            ));
        }
        McpServerState::Invalid(err) => {
            return Err(Error::new(ErrorKind::InvalidData, err.clone()));
        }
        McpServerState::Missing | McpServerState::Managed { .. } => {}
    }

    let mut changed = false;

    // 1. [features] tui_extensions_statusline = true
    {
        let features = ensure_table_like_mut(doc.as_table_mut(), FEATURES_TABLE, false)?;
        if features.get(FEATURE_STATUSLINE_KEY).and_then(Item::as_bool) != Some(true) {
            set_value_preserving_decor(features, FEATURE_STATUSLINE_KEY, Value::from(true));
            changed = true;
        }
    }

    // 2. [mcp_servers.agent-statusline]
    // すでに有効な管理下コマンド（絶対パスや --config 指定を含む）が設定済みなら保持し、
    // 無効化（enabled = false）のみ修復する。
    {
        let mcp_servers = ensure_table_like_mut(doc.as_table_mut(), MCP_SERVERS_TABLE, true)?;
        let server = ensure_table_like_mut(mcp_servers, SERVER_NAME, false)?;

        match existing_mcp {
            McpServerState::Missing => {
                set_value_preserving_decor(server, "command", Value::from(CODEX_COMMAND));
                let mut arr = Array::new();
                for arg in CODEX_ARGS {
                    arr.push(arg);
                }
                set_value_preserving_decor(server, "args", Value::Array(arr));
                changed = true;
            }
            McpServerState::Managed { enabled: false, .. } => {
                set_value_preserving_decor(server, "enabled", Value::from(true));
                changed = true;
            }
            McpServerState::Managed { enabled: true, .. }
            | McpServerState::OtherCommand(_)
            | McpServerState::Invalid(_) => {}
        }
    }

    // 3. [tui] status_line_provider = { server = "agent-statusline", tool = "statusline" }
    {
        let tui = ensure_table_like_mut(doc.as_table_mut(), TUI_TABLE, false)?;
        if !matches!(
            inspect_provider_item(tui.get(STATUS_LINE_PROVIDER_KEY)),
            ProviderState::AgentStatusline
        ) {
            let inline = make_provider_inline_table(SERVER_NAME, TOOL_NAME);
            set_value_preserving_decor(tui, STATUS_LINE_PROVIDER_KEY, Value::InlineTable(inline));
            changed = true;
        }
    }

    Ok(changed)
}

fn ensure_table_like_mut<'a>(
    parent: &'a mut dyn TableLike,
    key: &str,
    implicit_if_new: bool,
) -> io::Result<&'a mut dyn TableLike> {
    if !parent.contains_key(key) {
        let mut table = Table::new();
        table.set_implicit(implicit_if_new);
        parent.insert(key, Item::Table(table));
    }

    let item = parent
        .get_mut(key)
        .ok_or_else(|| Error::other(format!("Failed to access table [{key}]")))?;

    if !item.is_table_like() {
        return Err(Error::new(
            ErrorKind::InvalidData,
            format!("[{key}] must be a TOML table"),
        ));
    }

    // `[tui.status_line_provider]` のみ存在するファイルでは `tui` が implicit のため、
    // 直接キーを書き込む際はヘッダ `[tui]` を明示化しないとトップレベルへ漏れる。
    if !implicit_if_new && let Some(table) = item.as_table_mut() {
        table.set_implicit(false);
    }

    item.as_table_like_mut().ok_or_else(|| {
        Error::new(
            ErrorKind::InvalidData,
            format!("[{key}] must be a TOML table"),
        )
    })
}

fn set_value_preserving_decor(table: &mut dyn TableLike, key: &str, mut value: Value) {
    if let Some(existing) = table.get_mut(key) {
        if let Some(old) = existing.as_value() {
            *value.decor_mut() = old.decor().clone();
        }
        *existing = Item::Value(value);
    } else {
        table.insert(key, Item::Value(value));
    }
}

fn make_provider_inline_table(server: &str, tool: &str) -> InlineTable {
    let mut inline = InlineTable::new();
    inline.insert("server", Value::from(server));
    inline.insert("tool", Value::from(tool));
    inline
}

fn is_feature_enabled(doc: &DocumentMut) -> bool {
    doc.get(FEATURES_TABLE)
        .and_then(Item::as_table_like)
        .and_then(|t| t.get(FEATURE_STATUSLINE_KEY))
        .and_then(Item::as_bool)
        == Some(true)
}

fn inspect_mcp_server(doc: &DocumentMut) -> McpServerState {
    let Some(servers_item) = doc.get(MCP_SERVERS_TABLE) else {
        return McpServerState::Missing;
    };
    let Some(servers) = servers_item.as_table_like() else {
        return McpServerState::Invalid(format!("[{MCP_SERVERS_TABLE}] must be a TOML table"));
    };
    let Some(server_item) = servers.get(SERVER_NAME) else {
        return McpServerState::Missing;
    };
    let Some(server) = server_item.as_table_like() else {
        return McpServerState::Invalid(format!(
            "[{MCP_SERVERS_TABLE}.{SERVER_NAME}] must be a TOML table"
        ));
    };

    let has_url = server.get("url").is_some();
    let has_command = server.get("command").is_some();
    if has_url && has_command {
        return McpServerState::Invalid(format!(
            "[{MCP_SERVERS_TABLE}.{SERVER_NAME}] cannot specify both 'url' and 'command'"
        ));
    }
    if let Some(url) = server.get("url").and_then(Item::as_str) {
        return McpServerState::OtherCommand(url.to_string());
    }
    if has_url {
        return McpServerState::Invalid(format!(
            "[{MCP_SERVERS_TABLE}.{SERVER_NAME}] 'url' must be a string"
        ));
    }

    let Some(cmd) = server.get("command").and_then(Item::as_str) else {
        return McpServerState::Invalid(format!(
            "[{MCP_SERVERS_TABLE}.{SERVER_NAME}] is missing string 'command'"
        ));
    };

    let args: Option<Vec<&str>> = server.get("args").and_then(|item| {
        let arr = item.as_array()?;
        arr.iter().map(Value::as_str).collect()
    });

    if is_managed_codex_command(cmd, args.as_deref()) {
        let enabled = server.get("enabled").and_then(Item::as_bool) != Some(false);
        let command_display = match args {
            Some(ref a) if !a.is_empty() => format!("{cmd} {}", a.join(" ")),
            _ => CODEX_COMMAND_DISPLAY.to_string(),
        };
        McpServerState::Managed {
            command_display,
            enabled,
        }
    } else {
        let display = match args {
            Some(ref a) if !a.is_empty() => format!("{cmd} {}", a.join(" ")),
            _ => cmd.to_string(),
        };
        McpServerState::OtherCommand(display)
    }
}

fn is_managed_codex_command(command: &str, args: Option<&[&str]>) -> bool {
    let file_name = command
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(command)
        .to_ascii_lowercase();
    let prog_name = file_name.strip_suffix(".exe").unwrap_or(&file_name);
    if prog_name != CODEX_COMMAND {
        return false;
    }

    let Some(args) = args else {
        return false;
    };
    is_valid_mcp_codex_args(args)
}

/// 実際の clap CLI 定義（`Cli::try_parse_from`）で `mcp codex` が受理する引数か厳密に検証する
fn is_valid_mcp_codex_args(args: &[&str]) -> bool {
    let cmdline = std::iter::once(CODEX_COMMAND).chain(args.iter().copied());
    matches!(
        Cli::try_parse_from(cmdline),
        Ok(Cli {
            command: Commands::Mcp {
                target: McpTarget::Codex { .. }
            }
        })
    )
}

fn inspect_provider(doc: &DocumentMut) -> ProviderState {
    let Some(tui) = doc.get(TUI_TABLE).and_then(Item::as_table_like) else {
        return ProviderState::DefaultOrMissing;
    };
    inspect_provider_item(tui.get(STATUS_LINE_PROVIDER_KEY))
}

fn inspect_provider_item(item: Option<&Item>) -> ProviderState {
    let Some(item) = item else {
        return ProviderState::DefaultOrMissing;
    };

    if let Some(s) = item.as_str() {
        return if s == "default" {
            ProviderState::DefaultOrMissing
        } else {
            ProviderState::Invalid(format!(
                "[{TUI_TABLE}] {STATUS_LINE_PROVIDER_KEY} has unknown string value '{s}'"
            ))
        };
    }

    if let Some(tbl) = item.as_table_like() {
        let server = tbl.get("server").and_then(Item::as_str);
        let tool = tbl.get("tool").and_then(Item::as_str);
        return match (server, tool) {
            (Some(s), Some(t)) if s == SERVER_NAME && t == TOOL_NAME => {
                ProviderState::AgentStatusline
            }
            (Some(s), Some(t)) => ProviderState::Custom {
                server: s.to_string(),
                tool: t.to_string(),
            },
            _ => ProviderState::Invalid(format!(
                "[{TUI_TABLE}] {STATUS_LINE_PROVIDER_KEY} must specify string 'server' and 'tool'"
            )),
        };
    }

    ProviderState::Invalid(format!(
        "[{TUI_TABLE}] {STATUS_LINE_PROVIDER_KEY} has an invalid type"
    ))
}

fn make_origin_provider_path(path: &Path) -> PathBuf {
    let mut origin = path.as_os_str().to_os_string();
    origin.push(".agent-statusline-origin.json");
    PathBuf::from(origin)
}

fn sync_origin_provider_file(config_path: &Path, previous: &ProviderState) -> io::Result<()> {
    match previous {
        ProviderState::Custom { server, tool } => {
            let origin_path = make_origin_provider_path(config_path);
            let payload = SavedProvider {
                server: server.clone(),
                tool: tool.clone(),
            };
            let json = serde_json::to_string_pretty(&payload)
                .map_err(|e| Error::other(e.to_string()))?
                + "\n";
            atomic_write(&origin_path, json.as_bytes())
        }
        // 修復インストール時は初回に記録した元プロバイダーを保持する
        ProviderState::AgentStatusline => Ok(()),
        ProviderState::DefaultOrMissing | ProviderState::Invalid(_) => {
            remove_origin_provider_file(config_path);
            Ok(())
        }
    }
}

fn remove_origin_provider_file(config_path: &Path) {
    let origin_path = make_origin_provider_path(config_path);
    let _ = fs::remove_file(origin_path);
}

fn load_origin_provider_with_reader<F>(
    config_path: &Path,
    read_origin: F,
) -> io::Result<Option<(String, String)>>
where
    F: FnOnce(&Path) -> io::Result<String>,
{
    let origin_path = make_origin_provider_path(config_path);
    let content = match read_origin(&origin_path) {
        Ok(content) => content,
        Err(err) if err.kind() == ErrorKind::NotFound => return Ok(None),
        Err(err) => {
            return Err(Error::new(
                err.kind(),
                format!(
                    "Failed to read saved Codex provider origin at {}: {err}",
                    origin_path.display()
                ),
            ));
        }
    };

    let saved: SavedProvider = serde_json::from_str(&content).map_err(|err| {
        Error::new(
            ErrorKind::InvalidData,
            format!(
                "Invalid saved Codex provider origin at {}: {err}",
                origin_path.display()
            ),
        )
    })?;

    if saved.server.trim().is_empty()
        || saved.tool.trim().is_empty()
        || (saved.server == SERVER_NAME && saved.tool == TOOL_NAME)
    {
        return Err(Error::new(
            ErrorKind::InvalidData,
            format!(
                "Invalid saved Codex provider entry at {}",
                origin_path.display()
            ),
        ));
    }

    Ok(Some((saved.server, saved.tool)))
}

fn try_restore_custom_provider<F>(
    config_path: &Path,
    backup_path: &Path,
    read_origin: F,
) -> io::Result<Option<(String, String)>>
where
    F: FnOnce(&Path) -> io::Result<String>,
{
    if let Some(origin) = load_origin_provider_with_reader(config_path, read_origin)? {
        return Ok(Some(origin));
    }
    if !backup_path.exists() {
        return Ok(None);
    }
    let bak_content = fs::read_to_string(backup_path)?;
    let Ok(bak_doc) = bak_content.parse::<DocumentMut>() else {
        return Ok(None);
    };
    match inspect_provider(&bak_doc) {
        ProviderState::Custom { server, tool } => Ok(Some((server, tool))),
        _ => Ok(None),
    }
}

fn restore_provider_item(
    doc: &mut DocumentMut,
    (server, tool): (String, String),
) -> io::Result<()> {
    let tui = ensure_table_like_mut(doc.as_table_mut(), TUI_TABLE, false)?;
    let inline = make_provider_inline_table(&server, &tool);
    set_value_preserving_decor(tui, STATUS_LINE_PROVIDER_KEY, Value::InlineTable(inline));
    Ok(())
}

fn remove_managed_mcp_server(doc: &mut DocumentMut) {
    let should_remove_parent = if let Some(servers) = doc
        .get_mut(MCP_SERVERS_TABLE)
        .and_then(Item::as_table_like_mut)
    {
        servers.remove(SERVER_NAME);
        servers.is_empty() && !table_item_has_comment(doc.get(MCP_SERVERS_TABLE))
    } else {
        false
    };
    if should_remove_parent {
        doc.remove(MCP_SERVERS_TABLE);
    }
}

fn remove_provider_key(doc: &mut DocumentMut) {
    let should_remove_parent =
        if let Some(tui) = doc.get_mut(TUI_TABLE).and_then(Item::as_table_like_mut) {
            tui.remove(STATUS_LINE_PROVIDER_KEY);
            tui.is_empty() && !table_item_has_comment(doc.get(TUI_TABLE))
        } else {
            false
        };
    if should_remove_parent {
        doc.remove(TUI_TABLE);
    }
}

fn table_item_has_comment(item: Option<&Item>) -> bool {
    let Some(table) = item.and_then(Item::as_table) else {
        return false;
    };
    let prefix = table
        .decor()
        .prefix()
        .and_then(|r| r.as_str())
        .unwrap_or("");
    let suffix = table
        .decor()
        .suffix()
        .and_then(|r| r.as_str())
        .unwrap_or("");
    prefix.contains('#') || suffix.contains('#')
}

fn ensure_trailing_newline(mut text: String) -> String {
    if !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::AgentKind;
    use tempfile::TempDir;

    fn codex_target(dir: &TempDir) -> AgentConfigTarget {
        AgentConfigTarget::resolve_with_home(AgentKind::Codex, dir.path())
    }

    #[test]
    fn test_codex_install_new_file_and_idempotency() {
        let dir = TempDir::new().unwrap();
        let target = codex_target(&dir);

        let res = install_codex(&target, false).unwrap();
        assert_eq!(res.action, PatchAction::Installed);
        assert!(res.backup_path.is_none());
        assert!(target.path.exists());

        let content = fs::read_to_string(&target.path).unwrap();
        let doc: DocumentMut = content.parse().unwrap();
        assert_eq!(
            doc["features"]["tui_extensions_statusline"].as_bool(),
            Some(true)
        );
        assert_eq!(
            doc["mcp_servers"]["agent-statusline"]["command"].as_str(),
            Some("agent-statusline")
        );
        assert_eq!(
            doc["tui"]["status_line_provider"]["server"].as_str(),
            Some("agent-statusline")
        );
        assert_eq!(
            doc["tui"]["status_line_provider"]["tool"].as_str(),
            Some("statusline")
        );
        // 余計な空 `[mcp_servers]` ヘッダを出さない
        assert!(!content.contains("\n[mcp_servers]\n"));

        // 2 回目は AlreadyInstalled
        let res2 = install_codex(&target, false).unwrap();
        assert_eq!(res2.action, PatchAction::AlreadyInstalled);
        assert!(res2.backup_path.is_none());

        // 診断は Installed
        let report = diagnose_codex(&target);
        assert_eq!(report.status, StatusKind::Installed);
        assert_eq!(
            report.command.as_deref(),
            Some("agent-statusline mcp codex")
        );
        assert_eq!(report.enabled, Some(true));
    }

    #[test]
    fn test_codex_install_preserves_comments_existing_settings_and_creates_backup() {
        let dir = TempDir::new().unwrap();
        let target = codex_target(&dir);
        fs::create_dir_all(target.path.parent().unwrap()).unwrap();

        let original = concat!(
            "# Top-level user comment\n",
            "model = \"gpt-6.2-sol\"\n\n",
            "[features]\n",
            "# Existing feature comment\n",
            "other_flag = true # inline comment\n\n",
            "[mcp_servers.other-server]\n",
            "command = \"other-mcp\"\n",
            "args = [\"serve\"]\n\n",
            "[tui]\n",
            "# TUI theme comment\n",
            "theme = \"dark\"\n",
        );
        fs::write(&target.path, original).unwrap();

        let res = install_codex(&target, false).unwrap();
        assert_eq!(res.action, PatchAction::Installed);
        let bak = res.backup_path.expect("backup created");
        assert_eq!(fs::read_to_string(&bak).unwrap(), original);

        let updated = fs::read_to_string(&target.path).unwrap();
        assert!(updated.contains("# Top-level user comment"));
        assert!(updated.contains("# Existing feature comment"));
        assert!(updated.contains("other_flag = true # inline comment"));
        assert!(updated.contains("# TUI theme comment"));
        assert!(updated.contains("[mcp_servers.other-server]"));

        let doc: DocumentMut = updated.parse().unwrap();
        assert_eq!(doc["model"].as_str(), Some("gpt-6.2-sol"));
        assert_eq!(doc["tui"]["theme"].as_str(), Some("dark"));
        assert_eq!(
            doc["features"]["tui_extensions_statusline"].as_bool(),
            Some(true)
        );
        assert_eq!(
            doc["tui"]["status_line_provider"]["server"].as_str(),
            Some("agent-statusline")
        );
    }

    #[test]
    fn test_codex_install_dry_run_does_not_modify_disk() {
        let dir = TempDir::new().unwrap();
        let target = codex_target(&dir);

        let res = install_codex(&target, true).unwrap();
        assert_eq!(res.action, PatchAction::Installed);
        let preview = res.preview.expect("preview present");
        assert!(preview.contains("tui_extensions_statusline = true"));
        assert!(preview.contains("[mcp_servers.agent-statusline]"));
        assert!(!target.path.exists());
    }

    #[test]
    fn test_codex_install_handles_subtable_provider_with_implicit_tui() {
        let dir = TempDir::new().unwrap();
        let target = codex_target(&dir);
        fs::create_dir_all(target.path.parent().unwrap()).unwrap();

        // `[tui]` ヘッダが無く `[tui.status_line_provider]` のみ存在するケース
        let original = "[tui.status_line_provider]\nserver = \"other\"\ntool = \"custom\"\n";
        fs::write(&target.path, original).unwrap();

        let res = install_codex(&target, false).unwrap();
        assert_eq!(res.action, PatchAction::Installed);

        let updated = fs::read_to_string(&target.path).unwrap();
        let doc: DocumentMut = updated.parse().unwrap();
        // トップレベルへ漏れず `[tui]` 配下に書かれていること
        assert!(doc.get("status_line_provider").is_none());
        assert_eq!(
            doc["tui"]["status_line_provider"]["server"].as_str(),
            Some("agent-statusline")
        );
        assert_eq!(diagnose_codex(&target).status, StatusKind::Installed);

        // uninstall で元の独自 provider が復元されること
        let unres = uninstall_codex(&target, false).unwrap();
        assert_eq!(unres.action, PatchAction::RestoredBackup);
        let restored = fs::read_to_string(&target.path).unwrap();
        let restored_doc: DocumentMut = restored.parse().unwrap();
        assert_eq!(
            restored_doc["tui"]["status_line_provider"]["server"].as_str(),
            Some("other")
        );
        assert_eq!(
            restored_doc["tui"]["status_line_provider"]["tool"].as_str(),
            Some("custom")
        );
        assert_eq!(
            diagnose_codex(&target).status,
            StatusKind::Custom("other/custom".into())
        );
    }

    #[test]
    fn test_codex_install_re_enables_disabled_mcp_server() {
        let dir = TempDir::new().unwrap();
        let target = codex_target(&dir);
        fs::create_dir_all(target.path.parent().unwrap()).unwrap();
        fs::write(
            &target.path,
            concat!(
                "[features]\n",
                "tui_extensions_statusline = true\n\n",
                "[mcp_servers.agent-statusline]\n",
                "command = \"agent-statusline\"\n",
                "args = [\"mcp\", \"codex\"]\n",
                "enabled = false\n\n",
                "[tui]\n",
                "status_line_provider = { server = \"agent-statusline\", tool = \"statusline\" }\n",
            ),
        )
        .unwrap();

        let before = diagnose_codex(&target);
        assert!(matches!(before.status, StatusKind::Incomplete(_)));

        let res = install_codex(&target, false).unwrap();
        assert_eq!(res.action, PatchAction::Installed);
        assert_eq!(diagnose_codex(&target).status, StatusKind::Installed);
    }

    #[test]
    fn test_codex_install_and_uninstall_with_inline_tables() {
        let dir = TempDir::new().unwrap();
        let target = codex_target(&dir);
        fs::create_dir_all(target.path.parent().unwrap()).unwrap();
        fs::write(
            &target.path,
            concat!(
                "features = { existing = true }\n",
                "mcp_servers = { other = { command = \"other-mcp\", args = [] } }\n",
                "tui = { theme = \"dark\" }\n",
            ),
        )
        .unwrap();

        let res = install_codex(&target, false).unwrap();
        assert_eq!(res.action, PatchAction::Installed);

        let updated = fs::read_to_string(&target.path).unwrap();
        let doc: DocumentMut = updated
            .parse()
            .expect("valid TOML after inline table patch");
        assert_eq!(doc["features"]["existing"].as_bool(), Some(true));
        assert_eq!(
            doc["features"]["tui_extensions_statusline"].as_bool(),
            Some(true)
        );
        assert_eq!(
            doc["mcp_servers"]["other"]["command"].as_str(),
            Some("other-mcp")
        );
        assert_eq!(diagnose_codex(&target).status, StatusKind::Installed);

        let unres = uninstall_codex(&target, false).unwrap();
        assert_eq!(unres.action, PatchAction::Uninstalled);
        let after: DocumentMut = fs::read_to_string(&target.path)
            .unwrap()
            .parse()
            .expect("valid TOML after uninstall");
        assert!(
            after["mcp_servers"]
                .as_table_like()
                .unwrap()
                .get("agent-statusline")
                .is_none()
        );
        assert_eq!(
            after["mcp_servers"]["other"]["command"].as_str(),
            Some("other-mcp")
        );
        assert_eq!(after["tui"]["theme"].as_str(), Some("dark"));
    }

    #[test]
    fn test_codex_uninstall_roundtrip_and_feature_preservation() {
        let dir = TempDir::new().unwrap();
        let target = codex_target(&dir);

        // ファイル不在時は NotInstalled
        assert_eq!(
            uninstall_codex(&target, false).unwrap().action,
            PatchAction::NotInstalled
        );

        // インストール → アンインストール
        install_codex(&target, false).unwrap();
        let dry = uninstall_codex(&target, true).unwrap();
        assert_eq!(dry.action, PatchAction::Uninstalled);
        assert!(dry.preview.is_some());
        assert_eq!(diagnose_codex(&target).status, StatusKind::Installed);

        let unres = uninstall_codex(&target, false).unwrap();
        assert_eq!(unres.action, PatchAction::Uninstalled);
        assert!(unres.backup_path.is_some());

        let after = fs::read_to_string(&target.path).unwrap();
        let doc: DocumentMut = after.parse().unwrap();
        // `[features] tui_extensions_statusline` は他プロバイダー保護のため残す
        assert_eq!(
            doc["features"]["tui_extensions_statusline"].as_bool(),
            Some(true)
        );
        assert!(doc.get("mcp_servers").is_none());
        assert!(doc.get("tui").is_none());
        assert_eq!(diagnose_codex(&target).status, StatusKind::NotConfigured);
    }

    #[test]
    fn test_codex_uninstall_protects_custom_provider_and_foreign_server() {
        let cases = [
            (
                "custom-provider-only",
                "[tui]\nstatus_line_provider = { server = \"other\", tool = \"bar\" }\n",
                PatchAction::SkippedCustom,
            ),
            (
                "wrapper-name-collision",
                "[mcp_servers.agent-statusline]\ncommand = \"my-agent-statusline-wrapper\"\nargs = [\"mcp\", \"codex\"]\n",
                PatchAction::SkippedCustom,
            ),
            (
                "default-provider-only",
                "[tui]\nstatus_line_provider = \"default\"\n",
                PatchAction::NotInstalled,
            ),
        ];

        for (name, toml, expected) in cases {
            let dir = TempDir::new().unwrap();
            let target = codex_target(&dir);
            fs::create_dir_all(target.path.parent().unwrap()).unwrap();
            fs::write(&target.path, toml).unwrap();

            let res = uninstall_codex(&target, false).unwrap();
            assert_eq!(res.action, expected, "{name}");
            assert_eq!(fs::read_to_string(&target.path).unwrap(), toml, "{name}");
        }
    }

    #[test]
    fn test_codex_uninstall_keeps_switched_custom_provider_while_removing_managed_server() {
        let dir = TempDir::new().unwrap();
        let target = codex_target(&dir);
        fs::create_dir_all(target.path.parent().unwrap()).unwrap();
        fs::write(
            &target.path,
            concat!(
                "[mcp_servers.agent-statusline]\n",
                "command = \"agent-statusline\"\n",
                "args = [\"mcp\", \"codex\"]\n\n",
                "[tui]\n",
                "status_line_provider = { server = \"other\", tool = \"bar\" }\n",
            ),
        )
        .unwrap();

        let res = uninstall_codex(&target, false).unwrap();
        assert_eq!(res.action, PatchAction::Uninstalled);
        let doc: DocumentMut = fs::read_to_string(&target.path).unwrap().parse().unwrap();
        assert!(doc.get("mcp_servers").is_none());
        assert_eq!(
            doc["tui"]["status_line_provider"]["server"].as_str(),
            Some("other")
        );
    }

    #[test]
    fn test_codex_diagnose_basic_and_invalid_states() {
        let dir = TempDir::new().unwrap();
        let target = codex_target(&dir);

        assert_eq!(diagnose_codex(&target).status, StatusKind::FileNotFound);
        fs::create_dir_all(target.path.parent().unwrap()).unwrap();

        let exact_cases = [
            ("empty", "  \n", StatusKind::NotConfigured),
            (
                "default-provider",
                "[tui]\nstatus_line_provider = \"default\"\n",
                StatusKind::NotConfigured,
            ),
            (
                "custom-provider",
                "[tui]\nstatus_line_provider = { server = \"other\", tool = \"line\" }\n",
                StatusKind::Custom("other/line".into()),
            ),
            (
                "installed-with-full-path-and-config",
                concat!(
                    "[features]\n",
                    "tui_extensions_statusline = true\n",
                    "[mcp_servers.agent-statusline]\n",
                    "command = \"C:\\\\bin\\\\agent-statusline.exe\"\n",
                    "args = [\"mcp\", \"codex\", \"--config\", \"C:\\\\my.toml\"]\n",
                    "[tui]\n",
                    "status_line_provider = { server = \"agent-statusline\", tool = \"statusline\" }\n",
                ),
                StatusKind::Installed,
            ),
        ];

        for (name, content, expected) in exact_cases {
            fs::write(&target.path, content).unwrap();
            assert_eq!(diagnose_codex(&target).status, expected, "{name}");
        }

        let invalid_cases = [
            ("invalid-toml", "[tui\nbroken", true),
            ("non-table-tui", "tui = \"oops\"\n", true),
            (
                "invalid-provider-type",
                "[tui]\nstatus_line_provider = 123\n",
                false,
            ),
            (
                "invalid-mcp-missing-command",
                "[mcp_servers.agent-statusline]\nargs = [\"mcp\", \"codex\"]\n",
                false,
            ),
        ];

        for (name, content, install_should_fail) in invalid_cases {
            fs::write(&target.path, content).unwrap();
            assert!(
                matches!(diagnose_codex(&target).status, StatusKind::InvalidConfig(_)),
                "{name}"
            );
            if install_should_fail {
                assert!(install_codex(&target, false).is_err(), "{name}");
            }
        }
    }

    #[test]
    fn test_codex_diagnose_incomplete_states() {
        let dir = TempDir::new().unwrap();
        let target = codex_target(&dir);
        fs::create_dir_all(target.path.parent().unwrap()).unwrap();

        let cases = [
            (
                "missing-feature",
                concat!(
                    "[mcp_servers.agent-statusline]\n",
                    "command = \"agent-statusline\"\n",
                    "args = [\"mcp\", \"codex\"]\n",
                    "[tui]\n",
                    "status_line_provider = { server = \"agent-statusline\", tool = \"statusline\" }\n",
                ),
                "tui_extensions_statusline",
            ),
            (
                "missing-mcp-server",
                concat!(
                    "[features]\n",
                    "tui_extensions_statusline = true\n",
                    "[tui]\n",
                    "status_line_provider = { server = \"agent-statusline\", tool = \"statusline\" }\n",
                ),
                "mcp_servers.agent-statusline",
            ),
            (
                "provider-not-selected",
                concat!(
                    "[features]\n",
                    "tui_extensions_statusline = true\n",
                    "[mcp_servers.agent-statusline]\n",
                    "command = \"agent-statusline\"\n",
                    "args = [\"mcp\", \"codex\"]\n",
                ),
                "status_line_provider",
            ),
            (
                "managed-server-with-other-provider",
                concat!(
                    "[features]\n",
                    "tui_extensions_statusline = true\n",
                    "[mcp_servers.agent-statusline]\n",
                    "command = \"agent-statusline\"\n",
                    "args = [\"mcp\", \"codex\"]\n",
                    "[tui]\n",
                    "status_line_provider = { server = \"other\", tool = \"bar\" }\n",
                ),
                "another provider (other/bar)",
            ),
            (
                "foreign-server-with-our-provider",
                concat!(
                    "[features]\n",
                    "tui_extensions_statusline = true\n",
                    "[mcp_servers.agent-statusline]\n",
                    "command = \"foreign-bin\"\n",
                    "args = [\"serve\"]\n",
                    "[tui]\n",
                    "status_line_provider = { server = \"agent-statusline\", tool = \"statusline\" }\n",
                ),
                "different command (foreign-bin serve)",
            ),
        ];

        for (name, content, expected_substring) in cases {
            fs::write(&target.path, content).unwrap();
            match diagnose_codex(&target).status {
                StatusKind::Incomplete(issues) => {
                    assert!(
                        issues.iter().any(|i| i.contains(expected_substring)),
                        "{name}: expected '{expected_substring}' in {issues:?}"
                    );
                }
                other => panic!("{name}: expected Incomplete, got {other:?}"),
            }
        }
    }

    #[test]
    fn test_codex_install_refuses_to_overwrite_conflicting_http_or_foreign_server() {
        let cases = [
            (
                "http-mcp-server",
                "[mcp_servers.agent-statusline]\nurl = \"https://example.com/mcp\"\n",
                ErrorKind::AlreadyExists,
            ),
            (
                "foreign-stdio-server",
                "[mcp_servers.agent-statusline]\ncommand = \"my-custom-mcp\"\nargs = [\"serve\"]\n",
                ErrorKind::AlreadyExists,
            ),
            (
                "mixed-url-and-command",
                "[mcp_servers.agent-statusline]\nurl = \"https://example.com/mcp\"\ncommand = \"agent-statusline\"\nargs = [\"mcp\", \"codex\"]\n",
                ErrorKind::InvalidData,
            ),
        ];

        for (name, content, expected_kind) in cases {
            let dir = TempDir::new().unwrap();
            let target = codex_target(&dir);
            fs::create_dir_all(target.path.parent().unwrap()).unwrap();
            fs::write(&target.path, content).unwrap();

            let err = install_codex(&target, false).expect_err(name);
            assert_eq!(err.kind(), expected_kind, "{name}");
            assert_eq!(fs::read_to_string(&target.path).unwrap(), content, "{name}");
            assert!(!make_backup_path(&target.path).exists(), "{name}");
            assert!(!make_origin_provider_path(&target.path).exists(), "{name}");
        }
    }

    #[test]
    fn test_codex_uninstall_restores_custom_provider_after_repair_install() {
        let dir = TempDir::new().unwrap();
        let target = codex_target(&dir);
        fs::create_dir_all(target.path.parent().unwrap()).unwrap();

        let original = concat!(
            "[tui]\n",
            "status_line_provider = { server = \"other\", tool = \"custom\" }\n",
        );
        fs::write(&target.path, original).unwrap();

        // 1. 初回インストール（元プロバイダーを origin ファイルへ退避）
        let first = install_codex(&target, false).unwrap();
        assert_eq!(first.action, PatchAction::Installed);
        assert!(make_origin_provider_path(&target.path).exists());

        // 2. feature flag を false に改変し、修復インストールを実行（.bak は上書きされる）
        let broken = fs::read_to_string(&target.path).unwrap().replace(
            "tui_extensions_statusline = true",
            "tui_extensions_statusline = false",
        );
        fs::write(&target.path, broken).unwrap();

        let repaired = install_codex(&target, false).unwrap();
        assert_eq!(repaired.action, PatchAction::Installed);
        // .bak には修復前の agent-statusline 設定が入っていることを確認
        let bak_content = fs::read_to_string(make_backup_path(&target.path)).unwrap();
        assert!(bak_content.contains("server = \"agent-statusline\""));

        // 3. アンインストールで元の独自プロバイダーが復元されること
        let unres = uninstall_codex(&target, false).unwrap();
        assert_eq!(unres.action, PatchAction::RestoredBackup);
        let doc: DocumentMut = fs::read_to_string(&target.path).unwrap().parse().unwrap();
        assert_eq!(
            doc["tui"]["status_line_provider"]["server"].as_str(),
            Some("other")
        );
        assert_eq!(
            doc["tui"]["status_line_provider"]["tool"].as_str(),
            Some("custom")
        );
        assert!(!make_origin_provider_path(&target.path).exists());
    }

    #[test]
    fn test_codex_install_preserves_custom_binary_path_and_config_arg() {
        let dir = TempDir::new().unwrap();
        let target = codex_target(&dir);
        fs::create_dir_all(target.path.parent().unwrap()).unwrap();

        let full_custom = concat!(
            "[features]\n",
            "tui_extensions_statusline = true\n\n",
            "[mcp_servers.agent-statusline]\n",
            "command = \"C:\\\\bin\\\\agent-statusline.exe\"\n",
            "args = [\"mcp\", \"codex\", \"--config\", \"C:\\\\my.toml\"]\n\n",
            "[tui]\n",
            "status_line_provider = { server = \"agent-statusline\", tool = \"statusline\" }\n",
        );
        fs::write(&target.path, full_custom).unwrap();

        // 既に有効な絶対パス + --config 構成は完全冪等（AlreadyInstalled）
        let idempotent = install_codex(&target, false).unwrap();
        assert_eq!(idempotent.action, PatchAction::AlreadyInstalled);
        assert_eq!(fs::read_to_string(&target.path).unwrap(), full_custom);

        // feature flag 修復（dry-run と実書き込みの両方）でも command / args を上書きしない
        let disabled_feature = full_custom.replace(
            "tui_extensions_statusline = true",
            "tui_extensions_statusline = false",
        );
        fs::write(&target.path, &disabled_feature).unwrap();

        let dry = install_codex(&target, true).unwrap();
        assert_eq!(dry.action, PatchAction::Installed);
        let preview = dry.preview.unwrap();
        assert!(preview.contains("C:\\\\bin\\\\agent-statusline.exe"));
        assert!(preview.contains("--config"));
        assert!(preview.contains("C:\\\\my.toml"));

        let repaired = install_codex(&target, false).unwrap();
        assert_eq!(repaired.action, PatchAction::Installed);
        let doc: DocumentMut = fs::read_to_string(&target.path).unwrap().parse().unwrap();
        assert_eq!(
            doc["mcp_servers"]["agent-statusline"]["command"].as_str(),
            Some("C:\\bin\\agent-statusline.exe")
        );
        let args: Vec<&str> = doc["mcp_servers"]["agent-statusline"]["args"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        assert_eq!(args, vec!["mcp", "codex", "--config", "C:\\my.toml"]);
    }

    #[test]
    fn test_codex_rejects_unsupported_mcp_codex_args() {
        let invalid_arg_sets: &[(&str, &[&str])] = &[
            ("unknown-flag", &["mcp", "codex", "--other-service"]),
            ("missing-config-value", &["mcp", "codex", "--config"]),
            ("help-flag", &["mcp", "codex", "--help"]),
            ("extra-positional", &["mcp", "codex", "extra"]),
            ("other-subcommand", &["render", "--agent", "claude"]),
        ];

        for (name, args) in invalid_arg_sets {
            let dir = TempDir::new().unwrap();
            let target = codex_target(&dir);
            fs::create_dir_all(target.path.parent().unwrap()).unwrap();
            let args_toml = args
                .iter()
                .map(|a| format!("\"{a}\""))
                .collect::<Vec<_>>()
                .join(", ");
            let content = format!(
                "[features]\ntui_extensions_statusline = true\n\n[mcp_servers.agent-statusline]\ncommand = \"agent-statusline\"\nargs = [{args_toml}]\n"
            );
            fs::write(&target.path, &content).unwrap();

            assert!(
                matches!(diagnose_codex(&target).status, StatusKind::Incomplete(_)),
                "{name} must not be diagnosed as Installed"
            );
            assert_eq!(
                uninstall_codex(&target, false).unwrap().action,
                PatchAction::SkippedCustom,
                "{name} must not be removed by uninstall"
            );
            assert_eq!(
                install_codex(&target, false).unwrap_err().kind(),
                ErrorKind::AlreadyExists,
                "{name} must fail install as conflict"
            );
            assert_eq!(fs::read_to_string(&target.path).unwrap(), content, "{name}");
        }

        // 有効な短縮・等号付き --config 形式は管理下として認識される
        assert!(is_valid_mcp_codex_args(&["mcp", "codex"]));
        assert!(is_valid_mcp_codex_args(&[
            "mcp",
            "codex",
            "-c",
            "/tmp/a.toml"
        ]));
        assert!(is_valid_mcp_codex_args(&[
            "mcp",
            "codex",
            "--config=/tmp/a.toml"
        ]));
    }

    #[test]
    fn test_codex_uninstall_preserves_config_and_origin_on_unreadable_or_corrupt_origin() {
        let dir = TempDir::new().unwrap();
        let target = codex_target(&dir);
        fs::create_dir_all(target.path.parent().unwrap()).unwrap();
        fs::write(
            &target.path,
            "[tui]\nstatus_line_provider = { server = \"other\", tool = \"custom\" }\n",
        )
        .unwrap();

        // 初回インストール → feature flag 改変 → 修復インストール（.bak を上書き）
        install_codex(&target, false).unwrap();
        let broken_flag = fs::read_to_string(&target.path).unwrap().replace(
            "tui_extensions_statusline = true",
            "tui_extensions_statusline = false",
        );
        fs::write(&target.path, broken_flag).unwrap();
        install_codex(&target, false).unwrap();

        let installed_config = fs::read_to_string(&target.path).unwrap();
        let origin_path = make_origin_provider_path(&target.path);
        let valid_origin = fs::read_to_string(&origin_path).unwrap();

        // 1. origin ファイル読み取り失敗（排他ロック相当）では設定・origin とも無変更でエラーを返す
        let lock_err = uninstall_codex_with_origin_reader(&target, false, |_| {
            Err(Error::from(ErrorKind::PermissionDenied))
        })
        .expect_err("unreadable origin must fail uninstall");
        assert_eq!(lock_err.kind(), ErrorKind::PermissionDenied);
        assert_eq!(fs::read_to_string(&target.path).unwrap(), installed_config);
        assert_eq!(fs::read_to_string(&origin_path).unwrap(), valid_origin);

        // 2. origin ファイルが不正 JSON の場合も設定・origin とも無変更でエラーを返す
        fs::write(&origin_path, "{broken json").unwrap();
        let parse_err = uninstall_codex(&target, false).expect_err("corrupt origin must fail");
        assert_eq!(parse_err.kind(), ErrorKind::InvalidData);
        assert_eq!(fs::read_to_string(&target.path).unwrap(), installed_config);
        assert_eq!(fs::read_to_string(&origin_path).unwrap(), "{broken json");

        // 3. origin ファイルを正常状態に戻して再実行すると元の独自プロバイダーが復元される
        fs::write(&origin_path, &valid_origin).unwrap();
        let recovered = uninstall_codex(&target, false).unwrap();
        assert_eq!(recovered.action, PatchAction::RestoredBackup);
        let doc: DocumentMut = fs::read_to_string(&target.path).unwrap().parse().unwrap();
        assert_eq!(
            doc["tui"]["status_line_provider"]["server"].as_str(),
            Some("other")
        );
        assert_eq!(
            doc["tui"]["status_line_provider"]["tool"].as_str(),
            Some("custom")
        );
        assert!(!origin_path.exists());
    }

    #[cfg(windows)]
    #[test]
    fn test_codex_uninstall_preserves_and_recovers_with_windows_exclusive_lock() {
        use std::os::windows::fs::OpenOptionsExt;

        let dir = TempDir::new().unwrap();
        let target = codex_target(&dir);
        fs::create_dir_all(target.path.parent().unwrap()).unwrap();
        fs::write(
            &target.path,
            "[tui]\nstatus_line_provider = { server = \"other\", tool = \"custom\" }\n",
        )
        .unwrap();

        install_codex(&target, false).unwrap();
        let installed_config = fs::read_to_string(&target.path).unwrap();
        let origin_path = make_origin_provider_path(&target.path);

        // Windows で排他オープン（共有モード 0）している間はエラーを返し設定を保持する
        let locked_file = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .share_mode(0)
            .open(&origin_path)
            .unwrap();
        let err = uninstall_codex(&target, false).expect_err("locked origin must fail uninstall");
        assert_ne!(err.kind(), ErrorKind::NotFound);
        assert_eq!(fs::read_to_string(&target.path).unwrap(), installed_config);
        assert!(origin_path.exists());
        drop(locked_file);

        // ロック解除後の再実行で元の独自プロバイダーが復元される
        let recovered = uninstall_codex(&target, false).unwrap();
        assert_eq!(recovered.action, PatchAction::RestoredBackup);
        let doc: DocumentMut = fs::read_to_string(&target.path).unwrap().parse().unwrap();
        assert_eq!(
            doc["tui"]["status_line_provider"]["server"].as_str(),
            Some("other")
        );
        assert!(!origin_path.exists());
    }
}
