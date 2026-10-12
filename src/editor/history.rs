//! 保存前の内容の世代保存。クラウド側の履歴と二重にならないよう、ローカルのデータディレクトリに置く。
//!
//! 設定ファイルごと（実ファイルのパスごと）にサブディレクトリを分け、複数の設定ファイルが混ざらないようにする。

use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use sha2::{Digest, Sha256};

/// 保持する世代数
pub const DEFAULT_KEEP: usize = 20;

const MAX_CREATE_ATTEMPTS: u64 = 1000;

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

/// `content`（保存前の内容）を 1 世代として保存し、`keep` 世代を超えた古いものを削除する。
/// 今保存した世代は、時刻が巻き戻った場合でも削除しない
pub fn record(
    root: &Path,
    real: &Path,
    content: &[u8],
    now: DateTime<Utc>,
    keep: usize,
) -> io::Result<HistoryEntry> {
    let dir = dir_for(root, real);
    std::fs::create_dir_all(&dir)?;

    // 同じ時刻（ミリ秒）の世代は連番で区別する。連番は既存の最大値の次にする。
    // 削除済みの番号を再利用すると、直後の削除で最新の世代を失う
    let prefix = format!("{}-", now.format("%Y%m%dT%H%M%S%3fZ"));
    let next = list(root, real)?
        .iter()
        .filter_map(|entry| sequence_of(&entry.path, &prefix))
        .max()
        .map_or(0, |max| max + 1);

    let (path, mut file) = create_entry(&dir, &prefix, next)?;
    file.write_all(content)?;
    file.sync_all()?;

    for old in list(root, real)?.into_iter().skip(keep.max(1)) {
        if old.path != path {
            std::fs::remove_file(&old.path)?;
        }
    }
    Ok(HistoryEntry { path })
}

/// `prefix` に続く連番のファイルを、他の世代を上書きせずに作る
fn create_entry(dir: &Path, prefix: &str, start: u64) -> io::Result<(PathBuf, File)> {
    for seq in start..start + MAX_CREATE_ATTEMPTS {
        let path = dir.join(format!("{prefix}{seq:06}.toml"));
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => return Ok((path, file)),
            Err(err) if err.kind() == io::ErrorKind::AlreadyExists => {}
            Err(err) => return Err(err),
        }
    }
    Err(io::Error::other("履歴のファイル名を決められません"))
}

fn sequence_of(path: &Path, prefix: &str) -> Option<u64> {
    path.file_stem()?
        .to_str()?
        .strip_prefix(prefix)?
        .parse()
        .ok()
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
    fn test_record_same_instant_beyond_keep_never_loses_newest() {
        let dir = tempfile::tempdir().unwrap();
        let real = Path::new("/cfg/config.toml");
        let mut last = None;
        // 同じ時刻のまま、保持数（20）を超えて記録する
        for i in 0..22 {
            let entry = record(dir.path(), real, format!("v{i}").as_bytes(), at(0), 20).unwrap();
            assert!(entry.path.exists(), "v{i}: 返した世代が存在する");
            last = Some(entry);
        }
        let entries = list(dir.path(), real).unwrap();
        assert_eq!(entries.len(), 20);
        assert_eq!(entries[0].read().unwrap(), "v21", "最新の世代が先頭");
        assert_eq!(entries[19].read().unwrap(), "v2", "古い 2 世代だけが消える");
        assert_eq!(entries[0], last.unwrap());
    }

    #[test]
    fn test_record_keeps_newest_even_if_clock_goes_back() {
        let dir = tempfile::tempdir().unwrap();
        let real = Path::new("/cfg/config.toml");
        for i in 0..3 {
            record(
                dir.path(),
                real,
                format!("new{i}").as_bytes(),
                at(100 + i),
                3,
            )
            .unwrap();
        }
        // 時刻が過去へ巻き戻っても、今保存した世代は削除しない
        let entry = record(dir.path(), real, b"back", at(0), 3).unwrap();
        assert!(entry.path.exists());
        assert_eq!(entry.read().unwrap(), "back");
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
