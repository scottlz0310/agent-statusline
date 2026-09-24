use std::fs;
use std::path::Path;

use crate::model::ratelimit::RatelimitPayload;

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
}
