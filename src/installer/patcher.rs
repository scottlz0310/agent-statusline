use crate::cli::AgentKind;
use crate::installer::client::AgentConfigTarget;
use serde_json::{Value, json};
use std::fs;
use std::io::{self, Error, ErrorKind};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PatchAction {
    Installed,
    AlreadyInstalled,
    Uninstalled,
    NotInstalled,
}

#[derive(Debug, Clone)]
pub struct PatchResult {
    pub agent: AgentKind,
    pub path: PathBuf,
    pub action: PatchAction,
    pub backup_path: Option<PathBuf>,
    pub preview: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StatusKind {
    Installed,
    Custom(String),
    NotConfigured,
    FileNotFound,
    InvalidJson(String),
}

#[derive(Debug, Clone)]
pub struct AgentStatusReport {
    pub agent: AgentKind,
    pub path: PathBuf,
    pub status: StatusKind,
    pub command: Option<String>,
    pub enabled: Option<bool>,
}

/// 対象エージェントの設定ファイルに statusLine を登録・更新する
pub fn install_to_agent(target: &AgentConfigTarget, dry_run: bool) -> io::Result<PatchResult> {
    let new_config = target.statusline_config_value();

    if target.path.exists() {
        let raw_content = fs::read_to_string(&target.path)?;
        let leading_comments = extract_leading_comments(&raw_content);
        let stripped = strip_json_comments(&raw_content);

        let mut json_val: Value = if stripped.trim().is_empty() {
            Value::Object(serde_json::Map::new())
        } else {
            serde_json::from_str(&stripped)
                .map_err(|e| Error::new(ErrorKind::InvalidData, format!("Invalid JSON: {e}")))?
        };

        let obj = json_val
            .as_object_mut()
            .ok_or_else(|| Error::new(ErrorKind::InvalidData, "Root JSON must be an object"))?;

        let mut changed = false;

        if obj.get("statusLine") != Some(&new_config) {
            obj.insert("statusLine".to_string(), new_config);
            changed = true;
        }

        // Copilot の場合は footer.showCustom = true も保証する
        if target.agent == AgentKind::Copilot {
            if let Some(footer_val) = obj.get_mut("footer") {
                if let Some(footer_obj) = footer_val.as_object_mut()
                    && footer_obj.get("showCustom") != Some(&json!(true))
                {
                    footer_obj.insert("showCustom".to_string(), json!(true));
                    changed = true;
                }
            } else {
                obj.insert("footer".to_string(), json!({ "showCustom": true }));
                changed = true;
            }
        }

        if !changed {
            return Ok(PatchResult {
                agent: target.agent,
                path: target.path.clone(),
                action: PatchAction::AlreadyInstalled,
                backup_path: None,
                preview: None,
            });
        }

        let body_str = serde_json::to_string_pretty(&json_val)
            .map_err(|e| Error::other(e.to_string()))?
            + "\n";
        let updated_str = format!("{leading_comments}{body_str}");

        if dry_run {
            return Ok(PatchResult {
                agent: target.agent,
                path: target.path.clone(),
                action: PatchAction::Installed,
                backup_path: None,
                preview: Some(updated_str),
            });
        }

        // バックアップ作成
        let backup_path = make_backup_path(&target.path);
        fs::copy(&target.path, &backup_path)?;

        // アトミック書き込み
        atomic_write(&target.path, updated_str.as_bytes())?;

        Ok(PatchResult {
            agent: target.agent,
            path: target.path.clone(),
            action: PatchAction::Installed,
            backup_path: Some(backup_path),
            preview: None,
        })
    } else {
        let mut map = serde_json::Map::new();
        map.insert("statusLine".to_string(), new_config);
        if target.agent == AgentKind::Copilot {
            map.insert("footer".to_string(), json!({ "showCustom": true }));
        }
        let json_val = Value::Object(map);
        let updated_str = serde_json::to_string_pretty(&json_val)
            .map_err(|e| Error::other(e.to_string()))?
            + "\n";

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

/// 対象エージェントの設定ファイルから statusLine 設定を解除する
pub fn uninstall_from_agent(target: &AgentConfigTarget, dry_run: bool) -> io::Result<PatchResult> {
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
    let leading_comments = extract_leading_comments(&raw_content);
    let stripped = strip_json_comments(&raw_content);

    if stripped.trim().is_empty() {
        return Ok(PatchResult {
            agent: target.agent,
            path: target.path.clone(),
            action: PatchAction::NotInstalled,
            backup_path: None,
            preview: None,
        });
    }

    let mut json_val: Value = serde_json::from_str(&stripped)
        .map_err(|e| Error::new(ErrorKind::InvalidData, format!("Invalid JSON: {e}")))?;

    let obj = json_val
        .as_object_mut()
        .ok_or_else(|| Error::new(ErrorKind::InvalidData, "Root JSON must be an object"))?;

    if obj.remove("statusLine").is_none() {
        return Ok(PatchResult {
            agent: target.agent,
            path: target.path.clone(),
            action: PatchAction::NotInstalled,
            backup_path: None,
            preview: None,
        });
    }

    let body_str =
        serde_json::to_string_pretty(&json_val).map_err(|e| Error::other(e.to_string()))? + "\n";
    let updated_str = format!("{leading_comments}{body_str}");

    if dry_run {
        return Ok(PatchResult {
            agent: target.agent,
            path: target.path.clone(),
            action: PatchAction::Uninstalled,
            backup_path: None,
            preview: Some(updated_str),
        });
    }

    let backup_path = make_backup_path(&target.path);
    fs::copy(&target.path, &backup_path)?;
    atomic_write(&target.path, updated_str.as_bytes())?;

    Ok(PatchResult {
        agent: target.agent,
        path: target.path.clone(),
        action: PatchAction::Uninstalled,
        backup_path: Some(backup_path),
        preview: None,
    })
}

/// 対象エージェントの設定ファイルの statusLine 診断
#[must_use]
pub fn diagnose_agent(target: &AgentConfigTarget) -> AgentStatusReport {
    if !target.path.exists() {
        return AgentStatusReport {
            agent: target.agent,
            path: target.path.clone(),
            status: StatusKind::FileNotFound,
            command: None,
            enabled: None,
        };
    }

    let raw_content = match fs::read_to_string(&target.path) {
        Ok(c) => c,
        Err(e) => {
            return AgentStatusReport {
                agent: target.agent,
                path: target.path.clone(),
                status: StatusKind::InvalidJson(e.to_string()),
                command: None,
                enabled: None,
            };
        }
    };

    let stripped = strip_json_comments(&raw_content);
    if stripped.trim().is_empty() {
        return AgentStatusReport {
            agent: target.agent,
            path: target.path.clone(),
            status: StatusKind::NotConfigured,
            command: None,
            enabled: None,
        };
    }

    let json_val: Value = match serde_json::from_str(&stripped) {
        Ok(v) => v,
        Err(e) => {
            return AgentStatusReport {
                agent: target.agent,
                path: target.path.clone(),
                status: StatusKind::InvalidJson(e.to_string()),
                command: None,
                enabled: None,
            };
        }
    };

    let Some(statusline) = json_val.get("statusLine") else {
        return AgentStatusReport {
            agent: target.agent,
            path: target.path.clone(),
            status: StatusKind::NotConfigured,
            command: None,
            enabled: None,
        };
    };

    let command = statusline
        .get("command")
        .and_then(Value::as_str)
        .map(String::from);
    let enabled = statusline.get("enabled").and_then(Value::as_bool);

    let status = if let Some(cmd) = &command {
        if cmd.contains("agent-statusline") {
            StatusKind::Installed
        } else {
            StatusKind::Custom(cmd.clone())
        }
    } else {
        StatusKind::NotConfigured
    };

    AgentStatusReport {
        agent: target.agent,
        path: target.path.clone(),
        status,
        command,
        enabled,
    }
}

/// JSONC のコメント行 (// と /* ... */) を安全に除去する
#[must_use]
pub fn strip_json_comments(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    let mut in_string = false;
    let mut is_escaped = false;

    while let Some(c) = chars.next() {
        if in_string {
            out.push(c);
            if is_escaped {
                is_escaped = false;
            } else if c == '\\' {
                is_escaped = true;
            } else if c == '"' {
                in_string = false;
            }
        } else if c == '"' {
            in_string = true;
            out.push(c);
        } else if c == '/' && chars.peek() == Some(&'/') {
            chars.next();
            for next_c in chars.by_ref() {
                if next_c == '\n' {
                    out.push('\n');
                    break;
                }
            }
        } else if c == '/' && chars.peek() == Some(&'*') {
            chars.next();
            let mut prev = ' ';
            for next_c in chars.by_ref() {
                if next_c == '\n' {
                    out.push('\n');
                }
                if prev == '*' && next_c == '/' {
                    break;
                }
                prev = next_c;
            }
        } else {
            out.push(c);
        }
    }

    out
}

/// ファイル先頭のコメントヘッダー行を抽出する
#[must_use]
pub fn extract_leading_comments(s: &str) -> String {
    let mut comments = String::new();
    for line in s.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("//") || trimmed.starts_with("/*") || trimmed.is_empty() {
            comments.push_str(line);
            comments.push('\n');
        } else {
            break;
        }
    }
    comments
}

fn make_backup_path(path: &Path) -> PathBuf {
    let mut bak = path.as_os_str().to_os_string();
    bak.push(".bak");
    PathBuf::from(bak)
}

fn atomic_write(path: &Path, content: &[u8]) -> io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| Error::new(ErrorKind::NotFound, "Parent directory not found"))?;

    let tmp_path = parent.join(format!(
        ".tmp-{}",
        path.file_name().unwrap_or_default().to_string_lossy()
    ));

    fs::write(&tmp_path, content)?;

    if let Err(e) = fs::rename(&tmp_path, path) {
        let _ = fs::remove_file(&tmp_path);
        return Err(e);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_strip_json_comments() {
        let jsonc = r#"// Single line comment
        {
            /* Block comment */
            "key": "value // not a comment",
            "url": "https://example.com"
        }
        "#;
        let stripped = strip_json_comments(jsonc);
        assert!(!stripped.contains("Single line comment"));
        assert!(!stripped.contains("Block comment"));
        assert!(stripped.contains("value // not a comment"));
        assert!(stripped.contains("https://example.com"));

        let val: Value = serde_json::from_str(&stripped).unwrap();
        assert_eq!(val["key"], "value // not a comment");
        assert_eq!(val["url"], "https://example.com");
    }

    #[test]
    fn test_extract_leading_comments() {
        let text = "// Line 1\n// Line 2\n\n{\n  \"foo\": \"bar\"\n}";
        let leading = extract_leading_comments(text);
        assert_eq!(leading, "// Line 1\n// Line 2\n\n");
    }

    #[test]
    fn test_install_new_file() {
        let temp_dir = TempDir::new().unwrap();
        let target = AgentConfigTarget::resolve_with_home(AgentKind::Agy, temp_dir.path());

        assert!(!target.path.exists());

        let res = install_to_agent(&target, false).unwrap();
        assert_eq!(res.action, PatchAction::Installed);
        assert!(target.path.exists());
        assert!(res.backup_path.is_none());

        let content = fs::read_to_string(&target.path).unwrap();
        let val: Value = serde_json::from_str(&content).unwrap();
        assert_eq!(val["statusLine"]["type"], "command");
        assert_eq!(
            val["statusLine"]["command"],
            "agent-statusline render --agent agy"
        );
        assert_eq!(val["statusLine"]["enabled"], true);
    }

    #[test]
    fn test_install_preserve_existing_keys_and_backup() {
        let temp_dir = TempDir::new().unwrap();
        let target = AgentConfigTarget::resolve_with_home(AgentKind::Claude, temp_dir.path());

        fs::create_dir_all(target.path.parent().unwrap()).unwrap();
        fs::write(&target.path, r#"{"theme": "dark", "apiKey": "secret123"}"#).unwrap();

        let res = install_to_agent(&target, false).unwrap();
        assert_eq!(res.action, PatchAction::Installed);
        assert!(res.backup_path.is_some());
        let bak = res.backup_path.unwrap();
        assert!(bak.exists());
        let bak_content = fs::read_to_string(&bak).unwrap();
        assert!(bak_content.contains("secret123"));

        let content = fs::read_to_string(&target.path).unwrap();
        let val: Value = serde_json::from_str(&content).unwrap();
        assert_eq!(val["theme"], "dark");
        assert_eq!(val["apiKey"], "secret123");
        assert_eq!(
            val["statusLine"]["command"],
            "agent-statusline render --agent claude"
        );
    }

    #[test]
    fn test_install_preserve_leading_comments() {
        let temp_dir = TempDir::new().unwrap();
        let target = AgentConfigTarget::resolve_with_home(AgentKind::Copilot, temp_dir.path());

        fs::create_dir_all(target.path.parent().unwrap()).unwrap();
        fs::write(
            &target.path,
            "// Custom header\n// Do not delete\n{\n  \"model\": \"gpt-4o\"\n}\n",
        )
        .unwrap();

        let res = install_to_agent(&target, false).unwrap();
        assert_eq!(res.action, PatchAction::Installed);

        let content = fs::read_to_string(&target.path).unwrap();
        assert!(content.starts_with("// Custom header\n// Do not delete\n"));
        let stripped = strip_json_comments(&content);
        let val: Value = serde_json::from_str(&stripped).unwrap();
        assert_eq!(val["model"], "gpt-4o");
        assert_eq!(
            val["statusLine"]["command"],
            "agent-statusline render --agent copilot"
        );
        assert_eq!(val["footer"]["showCustom"], true);
    }

    #[test]
    fn test_install_already_installed() {
        let temp_dir = TempDir::new().unwrap();
        let target = AgentConfigTarget::resolve_with_home(AgentKind::Copilot, temp_dir.path());

        install_to_agent(&target, false).unwrap();
        let res2 = install_to_agent(&target, false).unwrap();
        assert_eq!(res2.action, PatchAction::AlreadyInstalled);
    }

    #[test]
    fn test_install_dry_run() {
        let temp_dir = TempDir::new().unwrap();
        let target = AgentConfigTarget::resolve_with_home(AgentKind::Agy, temp_dir.path());

        let res = install_to_agent(&target, true).unwrap();
        assert_eq!(res.action, PatchAction::Installed);
        assert!(res.preview.is_some());
        assert!(!target.path.exists());
    }

    #[test]
    fn test_uninstall_success_and_backup() {
        let temp_dir = TempDir::new().unwrap();
        let target = AgentConfigTarget::resolve_with_home(AgentKind::Claude, temp_dir.path());

        install_to_agent(&target, false).unwrap();
        assert!(target.path.exists());

        let uninst = uninstall_from_agent(&target, false).unwrap();
        assert_eq!(uninst.action, PatchAction::Uninstalled);
        assert!(uninst.backup_path.is_some());

        let content = fs::read_to_string(&target.path).unwrap();
        let val: Value = serde_json::from_str(&content).unwrap();
        assert!(val.get("statusLine").is_none());
    }

    #[test]
    fn test_uninstall_not_installed() {
        let temp_dir = TempDir::new().unwrap();
        let target = AgentConfigTarget::resolve_with_home(AgentKind::Copilot, temp_dir.path());

        let uninst = uninstall_from_agent(&target, false).unwrap();
        assert_eq!(uninst.action, PatchAction::NotInstalled);
    }

    #[test]
    fn test_diagnose_states() {
        let temp_dir = TempDir::new().unwrap();
        let target = AgentConfigTarget::resolve_with_home(AgentKind::Agy, temp_dir.path());

        // 1. File missing
        let rep1 = diagnose_agent(&target);
        assert_eq!(rep1.status, StatusKind::FileNotFound);

        // 2. Not configured
        fs::create_dir_all(target.path.parent().unwrap()).unwrap();
        fs::write(&target.path, r#"{"other": 123}"#).unwrap();
        let rep2 = diagnose_agent(&target);
        assert_eq!(rep2.status, StatusKind::NotConfigured);

        // 3. Custom statusLine
        fs::write(
            &target.path,
            r#"{"statusLine": {"command": "custom-status.sh"}}"#,
        )
        .unwrap();
        let rep3 = diagnose_agent(&target);
        assert_eq!(rep3.status, StatusKind::Custom("custom-status.sh".into()));

        // 4. Installed
        install_to_agent(&target, false).unwrap();
        let rep4 = diagnose_agent(&target);
        assert_eq!(rep4.status, StatusKind::Installed);
        assert_eq!(
            rep4.command.as_deref(),
            Some("agent-statusline render --agent agy")
        );
        assert_eq!(rep4.enabled, Some(true));
    }

    #[test]
    fn test_empty_config_file_handling() {
        let temp_dir = TempDir::new().unwrap();
        let target = AgentConfigTarget::resolve_with_home(AgentKind::Copilot, temp_dir.path());

        fs::create_dir_all(target.path.parent().unwrap()).unwrap();
        fs::write(&target.path, "  \n").unwrap();

        // 診断時は NotConfigured
        let rep = diagnose_agent(&target);
        assert_eq!(rep.status, StatusKind::NotConfigured);

        // install すると正常に statusLine が書き込まれる
        let res = install_to_agent(&target, false).unwrap();
        assert_eq!(res.action, PatchAction::Installed);

        let content = fs::read_to_string(&target.path).unwrap();
        let val: Value = serde_json::from_str(&content).unwrap();
        assert_eq!(
            val["statusLine"]["command"],
            "agent-statusline render --agent copilot"
        );
    }
}
