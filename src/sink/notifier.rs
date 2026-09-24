use crate::model::ratelimit::RatelimitPayload;
use std::fs;

/// `%LOCALAPPDATA%/SquirrelNotifier/ratelimit-status/<agent_id>.json` への原子的書き出し
pub fn write_ratelimit_status(payload: &RatelimitPayload) -> std::io::Result<()> {
    let Some(local_app_data) = dirs::data_local_dir() else {
        return Ok(());
    };

    let out_dir = local_app_data
        .join("SquirrelNotifier")
        .join("ratelimit-status");
    if !out_dir.exists() {
        fs::create_dir_all(&out_dir)?;
    }

    let tmp_path = out_dir.join(format!("{}.json.tmp", payload.agent_id));
    let final_path = out_dir.join(format!("{}.json", payload.agent_id));

    let json_bytes = serde_json::to_vec(payload)?;
    fs::write(&tmp_path, json_bytes)?;
    fs::rename(&tmp_path, &final_path)?;

    Ok(())
}
