//! 保存前の内容の世代保存。クラウド側の履歴と二重にならないよう、ローカルのデータディレクトリに置く。
//!
//! 設定ファイルごと（実ファイルのパスごと）にサブディレクトリを分け、複数の設定ファイルが混ざらないようにする。

use std::io;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use sha2::{Digest, Sha256};

/// 保持する世代数
pub const DEFAULT_KEEP: usize = 20;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryEntry {
    pub path: PathBuf,
}

impl HistoryEntry {
    pub fn read(&self) -> io::Result<String> {
        std::fs::read_to_string(&self.path)
    }
}

fn dir_for(root: &Path, real: &Path) -> PathBuf {
    let digest = Sha256::digest(real.to_string_lossy().as_bytes());
    let id = digest
        .iter()
        .take(8)
        .fold(0_u64, |acc, byte| (acc << 8) | u64::from(*byte));
    root.join(format!("{id:016x}"))
}

/// `content`（保存前の内容）を 1 世代として保存し、`keep` 世代を超えた古いものを削除する
pub fn record(
    root: &Path,
    real: &Path,
    content: &[u8],
    now: DateTime<Utc>,
    keep: usize,
) -> io::Result<HistoryEntry> {
    let dir = dir_for(root, real);
    std::fs::create_dir_all(&dir)?;

    // 同じ時刻（ミリ秒）の世代でも、連番で並びと一意性を保つ
    let stamp = now.format("%Y%m%dT%H%M%S%3fZ");
    let mut path = dir.join(format!("{stamp}-00.toml"));
    for n in 1..100 {
        if !path.exists() {
            break;
        }
        path = dir.join(format!("{stamp}-{n:02}.toml"));
    }
    std::fs::write(&path, content)?;

    let entries = list(root, real)?;
    for old in entries.into_iter().skip(keep) {
        std::fs::remove_file(&old.path)?;
    }
    Ok(HistoryEntry { path })
}

/// 新しい世代から順に返す。履歴がなければ空
pub fn list(root: &Path, real: &Path) -> io::Result<Vec<HistoryEntry>> {
    let dir = dir_for(root, real);
    let dir_entries = match std::fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => return Err(err),
    };
    let mut paths = Vec::new();
    for entry in dir_entries {
        let path = entry?.path();
        if path.extension().is_some_and(|ext| ext == "toml") {
            paths.push(path);
        }
    }
    paths.sort();
    paths.reverse();
    Ok(paths
        .into_iter()
        .map(|path| HistoryEntry { path })
        .collect())
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;

    fn at(secs: i64) -> DateTime<Utc> {
        Utc.timestamp_opt(1_800_000_000 + secs, 0).unwrap()
    }

    #[test]
    fn test_record_list_and_prune() {
        let dir = tempfile::tempdir().unwrap();
        let real = Path::new("/cfg/config.toml");
        let other = Path::new("/cfg/other.toml");
        assert_eq!(list(dir.path(), real).unwrap(), []);

        for i in 0..5 {
            record(dir.path(), real, format!("v{i}").as_bytes(), at(i), 3).unwrap();
        }
        record(dir.path(), other, b"other", at(0), 3).unwrap();

        let contents: Vec<String> = list(dir.path(), real)
            .unwrap()
            .iter()
            .map(|e| e.read().unwrap())
            .collect();
        assert_eq!(contents, ["v4", "v3", "v2"], "新しい順、keep 世代まで");
        assert_eq!(
            list(dir.path(), other).unwrap().len(),
            1,
            "設定ファイルごとに分かれる"
        );
    }

    #[test]
    fn test_record_same_instant_keeps_order() {
        let dir = tempfile::tempdir().unwrap();
        let real = Path::new("/cfg/config.toml");
        for i in 0..3 {
            record(dir.path(), real, format!("v{i}").as_bytes(), at(0), 20).unwrap();
        }
        let contents: Vec<String> = list(dir.path(), real)
            .unwrap()
            .iter()
            .map(|e| e.read().unwrap())
            .collect();
        assert_eq!(contents, ["v2", "v1", "v0"]);
    }
}
