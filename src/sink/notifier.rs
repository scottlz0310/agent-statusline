use std::fs;
use std::path::{Path, PathBuf};

use crate::model::ratelimit::RatelimitPayload;

/// パス文字列内の環境変数（`%VAR%` または `$VAR` / `${VAR}`）を展開する
#[must_use]
pub fn expand_env_path(raw_path: &str) -> PathBuf {
    expand_env_path_with_lookup(raw_path, |var| std::env::var(var).ok())
}

/// 任意の環境変数ルックアップ関数を受け取り、パス文字列内の環境変数を展開する (DI 対応)
pub fn expand_env_path_with_lookup<F>(raw_path: &str, lookup: F) -> PathBuf
where
    F: Fn(&str) -> Option<String>,
{
    let mut expanded = raw_path.to_string();

    // 1. Windows の %VAR% を展開
    let mut start_idx = 0;
    while let Some(open) = expanded[start_idx..].find('%') {
        let actual_open = start_idx + open;
        if let Some(close) = expanded[actual_open + 1..].find('%') {
            let actual_close = actual_open + 1 + close;
            let var_name = &expanded[actual_open + 1..actual_close];
            if let Some(val) = lookup(var_name) {
                expanded.replace_range(actual_open..=actual_close, &val);
                start_idx = actual_open + val.len();
            } else {
                start_idx = actual_close + 1;
            }
        } else {
            break;
        }
    }

    // 2. Unix の ${VAR} を展開
    start_idx = 0;
    while let Some(open) = expanded[start_idx..].find("${") {
        let actual_open = start_idx + open;
        if let Some(close) = expanded[actual_open + 2..].find('}') {
            let actual_close = actual_open + 2 + close;
            let var_name = &expanded[actual_open + 2..actual_close];
            if let Some(val) = lookup(var_name) {
                expanded.replace_range(actual_open..=actual_close, &val);
                start_idx = actual_open + val.len();
            } else {
                start_idx = actual_close + 1;
            }
        } else {
            break;
        }
    }

    // 3. Unix の $VAR を展開（英数字とアンダースコアのみ）
    start_idx = 0;
    while let Some(dollar_idx) = expanded[start_idx..].find('$') {
        let actual_dollar = start_idx + dollar_idx;
        let rest = &expanded[actual_dollar + 1..];
        let var_len = rest
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
            .count();
        if var_len > 0 {
            let var_name = &rest[..var_len];
            if let Some(val) = lookup(var_name) {
                expanded.replace_range(actual_dollar..actual_dollar + 1 + var_len, &val);
                start_idx = actual_dollar + val.len();
            } else {
                start_idx = actual_dollar + 1 + var_len;
            }
        } else {
            start_idx = actual_dollar + 1;
        }
    }

    PathBuf::from(expanded)
}

/// 指定ディレクトリへ `%LOCALAPPDATA%/SquirrelNotifier/ratelimit-status/<agent_id>.json` 相当の原子的書き出しを行う
pub fn write_ratelimit_status_to_dir(
    payload: &RatelimitPayload,
    out_dir: &Path,
) -> std::io::Result<()> {
    if !out_dir.exists() {
        fs::create_dir_all(out_dir)?;
    }

    let pid = std::process::id();
    let tmp_path = out_dir.join(format!("{}.json.tmp.{}", payload.agent_id, pid));
    let final_path = out_dir.join(format!("{}.json", payload.agent_id));

    let json_bytes = serde_json::to_vec(payload)?;
    fs::write(&tmp_path, json_bytes)?;
    fs::rename(&tmp_path, &final_path)?;

    Ok(())
}

/// `%LOCALAPPDATA%/SquirrelNotifier/ratelimit-status/<agent_id>.json` への原子的書き出し
pub fn write_ratelimit_status(payload: &RatelimitPayload) -> std::io::Result<()> {
    let Some(local_app_data) = dirs::data_local_dir() else {
        return Ok(());
    };

    let out_dir = local_app_data
        .join("SquirrelNotifier")
        .join("ratelimit-status");
    write_ratelimit_status_to_dir(payload, &out_dir)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    #[test]
    fn test_write_ratelimit_status_atomic() {
        let temp_dir =
            env::temp_dir().join(format!("agent_statusline_test_{}", std::process::id()));
        let payload = RatelimitPayload {
            schema_version: 1,
            agent_id: "test-agent".to_string(),
            observed_at: "2026-09-24T00:00:00Z".to_string(),
            limits: vec![],
        };

        let res = write_ratelimit_status_to_dir(&payload, &temp_dir);
        assert!(res.is_ok());

        let target_file = temp_dir.join("test-agent.json");
        assert!(target_file.exists());

        let read_back = fs::read_to_string(&target_file).unwrap();
        assert!(read_back.contains("\"schemaVersion\":1"));
        assert!(read_back.contains("\"agentId\":\"test-agent\""));

        // クリーンアップ
        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_expand_env_path() {
        let mock_env = |var: &str| match var {
            "TEST_AGENT_DIR" => Some("my_ratelimit_dir".to_string()),
            _ => None,
        };

        // 1. Windows %VAR% 形式
        let p1 = expand_env_path_with_lookup("%TEST_AGENT_DIR%/sub", mock_env);
        assert_eq!(p1, PathBuf::from("my_ratelimit_dir/sub"));

        // 2. Unix ${VAR} 形式
        let p2 = expand_env_path_with_lookup("${TEST_AGENT_DIR}/sub2", mock_env);
        assert_eq!(p2, PathBuf::from("my_ratelimit_dir/sub2"));

        // 3. Unix $VAR 形式
        let p3 = expand_env_path_with_lookup("$TEST_AGENT_DIR/sub3", mock_env);
        assert_eq!(p3, PathBuf::from("my_ratelimit_dir/sub3"));

        // 4. 存在しない環境変数はそのまま残る
        let p4 = expand_env_path_with_lookup("%NON_EXISTENT_VAR%/sub", mock_env);
        assert_eq!(p4, PathBuf::from("%NON_EXISTENT_VAR%/sub"));
    }
}
