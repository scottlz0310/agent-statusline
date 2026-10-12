//! 設定ファイルの安全な保存（設計書 §5.4）。
//!
//! - 既存のファイル: リンクを解決した実ファイルへ、同じディレクトリの一時ファイル経由で書き込む。
//!   保存直前に内容ハッシュを再確認し、外部で変更・削除されていたら上書きしない。保存前の内容は履歴に残す
//! - 未作成のファイル: 他のプロセスが先に作っていたら上書きしない。履歴は作らない
//!
//! 内容ハッシュによる検知は、確認と書き込みの間の変更までは防げない。その限界は履歴と差分での復旧で補う。

use std::fmt;
use std::fs::OpenOptions;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};

use crate::editor::history::{self, DEFAULT_KEEP};
use crate::editor::load::{ConfigSource, ContentHash, ResolvedPath};
use crate::editor::validate::{Issue, Severity, validate};

pub struct SaveEnv {
    /// 履歴の保存先。`None` のとき、既存ファイルの上書きはできない（復旧できないため）
    pub history_root: Option<PathBuf>,
    pub keep: usize,
    pub now: Box<dyn Fn() -> DateTime<Utc>>,
}

impl SaveEnv {
    pub fn from_process() -> Self {
        Self {
            history_root: dirs::data_local_dir()
                .map(|dir| dir.join("agent-statusline").join("history")),
            keep: DEFAULT_KEEP,
            now: Box::new(Utc::now),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConflictKind {
    /// 読み込み後に、内容が外部で変更された
    Changed,
    /// 読み込み後に、ファイルが削除された
    Deleted,
    /// 未作成のはずのファイルが、他で作られた
    Created,
}

#[derive(Debug)]
pub enum SaveError {
    /// 検証でエラーになった。保存しない
    Invalid(Vec<Issue>),
    /// 外部の変更と衝突した。再読み込み・差分の確認・別名で保存のいずれかを選ぶ
    Conflict(ConflictKind),
    /// 履歴の保存先を決められない
    NoHistoryDir,
    /// リンク切れのリンク先の親ディレクトリがない。未同期・未マウントを空のディレクトリで隠さないため、作成しない
    ParentMissing(PathBuf),
    Io {
        context: &'static str,
        path: PathBuf,
        source: io::Error,
    },
}

impl fmt::Display for SaveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(issues) => write!(
                f,
                "設定に誤りがあるため保存できません（{} 件）",
                issues.len()
            ),
            Self::Conflict(ConflictKind::Changed) => {
                write!(
                    f,
                    "設定ファイルが外部で変更されたため、上書きしませんでした"
                )
            }
            Self::Conflict(ConflictKind::Deleted) => {
                write!(f, "設定ファイルが外部で削除されたため、保存しませんでした")
            }
            Self::Conflict(ConflictKind::Created) => {
                write!(f, "設定ファイルが他で作成されたため、上書きしませんでした")
            }
            Self::NoHistoryDir => write!(
                f,
                "履歴の保存先を決められないため、既存の設定を上書きできません"
            ),
            Self::ParentMissing(dir) => write!(
                f,
                "リンク先のディレクトリがありません（クラウドフォルダが未同期の可能性）: {}",
                dir.display()
            ),
            Self::Io {
                context,
                path,
                source,
            } => {
                write!(f, "{context}: {}: {source}", path.display())
            }
        }
    }
}

impl std::error::Error for SaveError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}

fn io_error<'a>(context: &'static str, path: &'a Path) -> impl FnOnce(io::Error) -> SaveError + 'a {
    move |source| SaveError::Io {
        context,
        path: path.to_path_buf(),
        source,
    }
}

/// `text` を保存し、保存後の `ConfigSource`（常に `Existing`）を返す。検証のエラーがあれば何も書かない
pub fn save(source: &ConfigSource, text: &str, env: &SaveEnv) -> Result<ConfigSource, SaveError> {
    let report = validate(text);
    if report.has_errors() {
        let errors = report
            .issues
            .into_iter()
            .filter(|i| i.severity == Severity::Error)
            .collect();
        return Err(SaveError::Invalid(errors));
    }

    match source {
        ConfigSource::Existing { path, fingerprint } => save_existing(path, fingerprint, text, env),
        ConfigSource::Missing { target, from_link } => {
            save_missing(target, *from_link, text, &|from: &Path, to: &Path| {
                std::fs::hard_link(from, to)
            })
        }
    }
}

fn save_existing(
    path: &ResolvedPath,
    fingerprint: &ContentHash,
    text: &str,
    env: &SaveEnv,
) -> Result<ConfigSource, SaveError> {
    let real = &path.real;
    let current = match std::fs::read(real) {
        Ok(bytes) => bytes,
        Err(err) if err.kind() == io::ErrorKind::NotFound => {
            return Err(SaveError::Conflict(ConflictKind::Deleted));
        }
        Err(err) => {
            return Err(io_error("設定ファイルを再読み込みできません", real)(
                err,
            ));
        }
    };
    if ContentHash::of(&current) != *fingerprint {
        return Err(SaveError::Conflict(ConflictKind::Changed));
    }

    let root = env.history_root.as_deref().ok_or(SaveError::NoHistoryDir)?;
    history::record(root, real, &current, (env.now)(), env.keep)
        .map_err(io_error("履歴を保存できません", root))?;

    write_replace(real, text.as_bytes(), &|from: &Path, to: &Path| {
        std::fs::rename(from, to)
    })?;
    Ok(ConfigSource::Existing {
        path: path.clone(),
        fingerprint: ContentHash::of(text.as_bytes()),
    })
}

/// 同じディレクトリの一時ファイルに書いてから、`rename` で置き換える。失敗したら旧内容のまま残す
fn write_replace(
    real: &Path,
    bytes: &[u8],
    rename: &dyn Fn(&Path, &Path) -> io::Result<()>,
) -> Result<(), SaveError> {
    let tmp = temp_path(real);
    let result = (|| {
        write_new_file(&tmp, bytes)?;
        if let Ok(meta) = std::fs::metadata(real) {
            std::fs::set_permissions(&tmp, meta.permissions())?;
        }
        rename(&tmp, real)
    })();
    if let Err(source) = result {
        // 一時ファイルの掃除の失敗は、元のエラーを優先して報告する
        let _ = std::fs::remove_file(&tmp);
        return Err(SaveError::Io {
            context: "設定ファイルを書き込めません（旧内容は保たれています）",
            path: real.to_path_buf(),
            source,
        });
    }
    Ok(())
}

fn temp_path(target: &Path) -> PathBuf {
    let name = target.file_name().map_or_else(
        || "config.toml".into(),
        |n| n.to_string_lossy().into_owned(),
    );
    target.with_file_name(format!(".{name}.{}.tmp", std::process::id()))
}

fn write_new_file(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
    file.write_all(bytes)?;
    file.sync_all()
}

fn save_missing(
    target: &Path,
    from_link: bool,
    text: &str,
    hard_link: &dyn Fn(&Path, &Path) -> io::Result<()>,
) -> Result<ConfigSource, SaveError> {
    let parent = target
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    if !parent.exists() {
        if from_link {
            return Err(SaveError::ParentMissing(parent.to_path_buf()));
        }
        std::fs::create_dir_all(parent)
            .map_err(io_error("ディレクトリを作成できません", parent))?;
    }

    let tmp = temp_path(target);
    let published = publish_new(&tmp, target, text.as_bytes(), hard_link);
    // 一時ファイルは、成功・失敗のどちらでも残さない
    let _ = std::fs::remove_file(&tmp);
    published?;

    let real = std::fs::canonicalize(target)
        .map_err(io_error("保存した設定ファイルを解決できません", target))?;
    Ok(ConfigSource::Existing {
        path: ResolvedPath {
            requested: target.to_path_buf(),
            real,
        },
        fingerprint: ContentHash::of(text.as_bytes()),
    })
}

/// 一時ファイルを書き、`hard_link` で `target` に公開する（既にあれば失敗し、上書きしない）。
/// `hard_link` が使えないファイルシステムでは、`create_new` で `target` に直接書く。
/// その場合、書き込み中の内容が他のプロセスから見える可能性がある
fn publish_new(
    tmp: &Path,
    target: &Path,
    bytes: &[u8],
    hard_link: &dyn Fn(&Path, &Path) -> io::Result<()>,
) -> Result<(), SaveError> {
    write_new_file(tmp, bytes).map_err(io_error("一時ファイルを書き込めません", tmp))?;
    match hard_link(tmp, target) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == io::ErrorKind::AlreadyExists => {
            Err(SaveError::Conflict(ConflictKind::Created))
        }
        Err(_) => match write_new_file(target, bytes) {
            Ok(()) => Ok(()),
            Err(err) if err.kind() == io::ErrorKind::AlreadyExists => {
                Err(SaveError::Conflict(ConflictKind::Created))
            }
            Err(err) => Err(io_error("設定ファイルを作成できません", target)(
                err,
            )),
        },
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use chrono::TimeZone;

    use super::*;
    use crate::editor::load::{ConfigEnv, load_config_strict};

    const NEW_TEXT: &str = "format = \"$model\"\n";

    fn env_with(history: &Path) -> SaveEnv {
        let tick = Cell::new(0_i64);
        SaveEnv {
            history_root: Some(history.to_path_buf()),
            keep: 3,
            now: Box::new(move || {
                tick.set(tick.get() + 1);
                Utc.timestamp_opt(1_800_000_000 + tick.get(), 0).unwrap()
            }),
        }
    }

    fn load(path: &Path) -> ConfigSource {
        load_config_strict(Some(path), &ConfigEnv::default())
            .unwrap()
            .source
    }

    fn try_symlink(target: &Path, link: &Path) -> bool {
        #[cfg(unix)]
        let result = std::os::unix::fs::symlink(target, link);
        #[cfg(windows)]
        let result = std::os::windows::fs::symlink_file(target, link);
        match result {
            Ok(()) => true,
            Err(err) => {
                eprintln!("シンボリックリンクを作成できないためスキップ: {err}");
                false
            }
        }
    }

    fn files_in(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn test_save_existing_updates_file_and_records_history() {
        let dir = tempfile::tempdir().unwrap();
        let history = dir.path().join("history");
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "# 旧\nformat = \"$plan\"\n").unwrap();
        let env = env_with(&history);

        let saved = save(&load(&path), NEW_TEXT, &env).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), NEW_TEXT);
        assert_eq!(
            files_in(dir.path()),
            ["config.toml", "history"],
            "一時ファイルを残さない"
        );
        let ConfigSource::Existing {
            path: resolved,
            fingerprint,
        } = &saved
        else {
            panic!("Existing のはず");
        };
        assert_eq!(*fingerprint, ContentHash::of(NEW_TEXT.as_bytes()));

        let entries = history::list(&history, &resolved.real).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].read().unwrap(), "# 旧\nformat = \"$plan\"\n");

        // 保存後の source で続けて保存できる（ハッシュが更新されている）
        save(&saved, "format = \"$quota\"\n", &env).unwrap();
        assert_eq!(history::list(&history, &resolved.real).unwrap().len(), 2);
    }

    #[test]
    fn test_save_existing_prunes_history_to_keep() {
        let dir = tempfile::tempdir().unwrap();
        let history = dir.path().join("history");
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "format = \"$model\"\n").unwrap();
        let env = env_with(&history);
        let mut source = load(&path);
        for i in 0..5 {
            source = save(&source, &format!("format = \"$model\" # v{i}\n"), &env).unwrap();
        }
        let ConfigSource::Existing { path: resolved, .. } = &source else {
            panic!("Existing のはず");
        };
        let contents: Vec<String> = history::list(&history, &resolved.real)
            .unwrap()
            .iter()
            .map(|e| e.read().unwrap())
            .collect();
        assert_eq!(
            contents,
            [
                "format = \"$model\" # v3\n",
                "format = \"$model\" # v2\n",
                "format = \"$model\" # v1\n"
            ]
        );
    }

    #[test]
    fn test_save_existing_detects_external_changes() {
        let dir = tempfile::tempdir().unwrap();
        let history = dir.path().join("history");
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "format = \"$plan\"\n").unwrap();
        let env = env_with(&history);
        let source = load(&path);

        std::fs::write(&path, "format = \"$quota\" # 外部で編集\n").unwrap();
        let err = save(&source, NEW_TEXT, &env).unwrap_err();
        assert!(
            matches!(err, SaveError::Conflict(ConflictKind::Changed)),
            "{err}"
        );
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "format = \"$quota\" # 外部で編集\n",
            "上書きしない"
        );
        assert!(!history.exists(), "競合のときは履歴も作らない");

        std::fs::remove_file(&path).unwrap();
        let err = save(&source, NEW_TEXT, &env).unwrap_err();
        assert!(
            matches!(err, SaveError::Conflict(ConflictKind::Deleted)),
            "{err}"
        );
        assert!(!path.exists(), "削除されたファイルを復活させない");
    }

    #[test]
    fn test_save_existing_through_symlink_keeps_link() {
        let dir = tempfile::tempdir().unwrap();
        let history = dir.path().join("history");
        let real = dir.path().join("cloud.toml");
        let link = dir.path().join("link.toml");
        std::fs::write(&real, "format = \"$plan\"\n").unwrap();
        if !try_symlink(&real, &link) {
            return;
        }
        save(&load(&link), NEW_TEXT, &env_with(&history)).unwrap();
        assert!(
            std::fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(std::fs::read_to_string(&real).unwrap(), NEW_TEXT);
        assert_eq!(std::fs::read_to_string(&link).unwrap(), NEW_TEXT);
    }

    #[test]
    fn test_save_existing_requires_history_dir_and_valid_text() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let original = "format = \"$plan\"\n";
        std::fs::write(&path, original).unwrap();
        let source = load(&path);

        let no_history = SaveEnv {
            history_root: None,
            ..env_with(dir.path())
        };
        let err = save(&source, NEW_TEXT, &no_history).unwrap_err();
        assert!(matches!(err, SaveError::NoHistoryDir), "{err}");

        let cases = [
            ("syntax", "format = [[["),
            (
                "threshold",
                "[context]\nthreshold_warning = 90\nthreshold_critical = 10\n",
            ),
            ("style", "[model]\nstyle = \"cyna\"\n"),
        ];
        for (name, text) in cases {
            let err = save(&source, text, &env_with(&dir.path().join("h"))).unwrap_err();
            let SaveError::Invalid(issues) = err else {
                panic!("{name}: Invalid のはず: {err}");
            };
            assert!(!issues.is_empty(), "{name}");
        }
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            original,
            "旧内容のまま"
        );
    }

    #[test]
    fn test_write_replace_failure_keeps_old_content_and_removes_temp() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "old").unwrap();

        let fail = |_: &Path, _: &Path| Err(io::Error::other("rename 失敗"));
        let err = write_replace(&path, b"new", &fail).unwrap_err();
        assert!(matches!(err, SaveError::Io { .. }), "{err}");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "old");
        assert_eq!(
            files_in(dir.path()),
            ["config.toml"],
            "一時ファイルを残さない"
        );
    }

    #[test]
    fn test_save_missing_creates_file_and_directories_without_history() {
        let dir = tempfile::tempdir().unwrap();
        let history = dir.path().join("history");
        let target = dir.path().join("a").join("b").join("config.toml");
        let source = ConfigSource::Missing {
            target: target.clone(),
            from_link: false,
        };
        let env = env_with(&history);

        let saved = save(&source, NEW_TEXT, &env).unwrap();
        assert_eq!(std::fs::read_to_string(&target).unwrap(), NEW_TEXT);
        assert_eq!(
            files_in(target.parent().unwrap()),
            ["config.toml"],
            "一時ファイルを残さない"
        );
        assert!(!history.exists(), "初回保存は履歴を作らない");
        let ConfigSource::Existing { .. } = &saved else {
            panic!("保存後は Existing のはず");
        };

        // 次の保存で、初回保存の内容が履歴に残る
        let ConfigSource::Existing { path, .. } =
            save(&saved, "format = \"$quota\"\n", &env).unwrap()
        else {
            panic!("Existing のはず");
        };
        let entries = history::list(&history, &path.real).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].read().unwrap(), NEW_TEXT);
    }

    #[test]
    fn test_save_missing_does_not_overwrite_file_created_meanwhile() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("config.toml");
        let source = load(&target);
        assert!(matches!(source, ConfigSource::Missing { .. }));

        std::fs::write(&target, "# 他で作られた\n").unwrap();
        let err = save(&source, NEW_TEXT, &env_with(&dir.path().join("h"))).unwrap_err();
        assert!(
            matches!(err, SaveError::Conflict(ConflictKind::Created)),
            "{err}"
        );
        assert_eq!(
            std::fs::read_to_string(&target).unwrap(),
            "# 他で作られた\n"
        );
        assert_eq!(
            files_in(dir.path()),
            ["config.toml"],
            "一時ファイルを残さない"
        );
    }

    #[test]
    fn test_save_missing_without_hard_link_support() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("config.toml");
        let unsupported = |_: &Path, _: &Path| Err(io::Error::from(io::ErrorKind::Unsupported));

        save_missing(&target, false, NEW_TEXT, &unsupported).unwrap();
        assert_eq!(std::fs::read_to_string(&target).unwrap(), NEW_TEXT);
        assert_eq!(files_in(dir.path()), ["config.toml"]);

        // create_new の経路でも、既存のファイルは上書きしない
        let err = save_missing(&target, false, "format = \"$plan\"\n", &unsupported).unwrap_err();
        assert!(
            matches!(err, SaveError::Conflict(ConflictKind::Created)),
            "{err}"
        );
        assert_eq!(std::fs::read_to_string(&target).unwrap(), NEW_TEXT);
    }

    #[test]
    fn test_save_missing_from_dangling_link() {
        let dir = tempfile::tempdir().unwrap();
        let link = dir.path().join("link.toml");

        // リンク先の親ディレクトリがない: 作成せずエラー
        let missing_parent = dir.path().join("cloud").join("config.toml");
        if !try_symlink(&missing_parent, &link) {
            return;
        }
        let source = load(&link);
        let err = save(&source, NEW_TEXT, &env_with(&dir.path().join("h"))).unwrap_err();
        assert!(matches!(err, SaveError::ParentMissing(_)), "{err}");
        assert!(
            !dir.path().join("cloud").exists(),
            "空のディレクトリで隠さない"
        );

        // 親ディレクトリがある: リンク先に作成し、リンクは残る
        std::fs::create_dir(dir.path().join("cloud")).unwrap();
        let saved = save(&source, NEW_TEXT, &env_with(&dir.path().join("h"))).unwrap();
        assert_eq!(std::fs::read_to_string(&missing_parent).unwrap(), NEW_TEXT);
        assert!(
            std::fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(std::fs::read_to_string(&link).unwrap(), NEW_TEXT);
        assert!(matches!(saved, ConfigSource::Existing { .. }));
    }
}
