//! 設定ファイルの保存と再読み込み。画面の状態遷移（`app`）が、ファイルシステムに直接触れないよう、
//! トレイトの背後に置く（テストで差し替えられる）。

use std::path::Path;

use crate::editor::load::{ConfigEnv, ConfigError, ConfigSource, LoadedConfig, load_config_strict};
use crate::editor::save::{SaveEnv, SaveError, save};

pub trait Storage {
    /// `text` を、読み込んだ設定ファイルへ保存し、保存後の `ConfigSource` を返す
    fn save(&self, source: &ConfigSource, text: &str) -> Result<ConfigSource, SaveError>;

    /// 別のパスへ保存する。既にファイルがあれば、上書きせず `Conflict` を返す
    fn save_as(&self, path: &Path, text: &str) -> Result<ConfigSource, SaveError>;

    /// 設定を読み込み直す。`path` は、今の編集対象のパス（未作成のときは、保存先）。
    /// 起動時の指定や標準の探索へは戻らない（別名で保存した後も、そのファイルを読む）
    fn reload(&self, path: &Path) -> Result<LoadedConfig, ConfigError>;
}

/// 実際のファイルシステムへ保存する
pub struct FileStorage {
    save_env: SaveEnv,
    config_env: ConfigEnv,
}

impl FileStorage {
    pub fn from_process() -> Self {
        Self {
            save_env: SaveEnv::from_process(),
            config_env: ConfigEnv::from_process(),
        }
    }
}

impl Storage for FileStorage {
    fn save(&self, source: &ConfigSource, text: &str) -> Result<ConfigSource, SaveError> {
        save(source, text, &self.save_env)
    }

    fn save_as(&self, path: &Path, text: &str) -> Result<ConfigSource, SaveError> {
        // 未作成のファイルとして保存する。他で作られていれば、上書きしない
        let source = ConfigSource::Missing {
            target: path.to_path_buf(),
            from_link: false,
        };
        save(&source, text, &self.save_env)
    }

    fn reload(&self, path: &Path) -> Result<LoadedConfig, ConfigError> {
        load_config_strict(Some(path), &self.config_env)
    }
}

#[cfg(test)]
mod tests {
    use chrono::{TimeZone, Utc};

    use super::*;
    use crate::editor::save::ConflictKind;

    const TEXT: &str = "format = \"$model\"\n";

    /// 一時ディレクトリを、設定の探索と履歴の保存先にした `FileStorage`
    fn storage(dir: &Path) -> FileStorage {
        FileStorage {
            save_env: SaveEnv {
                history_root: Some(dir.join("history")),
                keep: 3,
                now: Box::new(|| Utc.timestamp_opt(1_800_000_000, 0).unwrap()),
            },
            config_env: ConfigEnv {
                env_path: None,
                config_dir: Some(dir.join("config")),
                home_dir: Some(dir.join("home")),
            },
        }
    }

    #[test]
    fn test_save_as_creates_a_new_file_and_refuses_to_overwrite() {
        let dir = tempfile::tempdir().unwrap();
        let storage = storage(dir.path());
        let target = dir.path().join("out").join("a.toml");

        let source = storage.save_as(&target, TEXT).unwrap();
        assert!(matches!(source, ConfigSource::Existing { .. }));
        assert_eq!(std::fs::read_to_string(&target).unwrap(), TEXT);

        // 既にあるパスは、上書きしない
        let err = storage
            .save_as(&target, "format = \"$plan\"\n")
            .unwrap_err();
        assert!(
            matches!(err, SaveError::Conflict(ConflictKind::Created)),
            "{err}"
        );
        assert_eq!(std::fs::read_to_string(&target).unwrap(), TEXT);
    }

    #[test]
    fn test_save_writes_the_existing_file() {
        let dir = tempfile::tempdir().unwrap();
        let storage = storage(dir.path());
        let path = dir.path().join("c.toml");
        std::fs::write(&path, "format = \"$plan\"\n").unwrap();
        let loaded = storage.reload(&path).unwrap();

        let source = storage.save(&loaded.source, TEXT).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), TEXT);
        assert!(matches!(source, ConfigSource::Existing { .. }));
    }

    #[test]
    fn test_reload_reads_exactly_the_given_path() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.toml");
        let b = dir.path().join("b.toml");
        std::fs::write(&a, "format = \"$plan\"\n").unwrap();
        std::fs::write(&b, "format = \"$model\"\n").unwrap();
        let storage = storage(dir.path());

        assert_eq!(storage.reload(&b).unwrap().config.format, "$model");
        assert_eq!(storage.reload(&a).unwrap().config.format, "$plan");

        // 削除されたファイルは、そのパスを保存先とする未作成として読む（別のファイルへ移らない）。
        // 再び作られれば、そのパスの内容を読む
        std::fs::remove_file(&b).unwrap();
        let loaded = storage.reload(&b).unwrap();
        assert!(matches!(
            &loaded.source,
            ConfigSource::Missing { target, .. } if target == &b
        ));
        std::fs::write(&b, "format = \"$quota\"\n").unwrap();
        assert_eq!(storage.reload(&b).unwrap().config.format, "$quota");
    }
}

#[cfg(test)]
pub mod fake {
    //! テスト用の `Storage`。呼び出しを記録し、次の結果を差し込める

    use std::cell::RefCell;
    use std::path::{Path, PathBuf};
    use std::rc::Rc;

    use super::*;
    use crate::editor::load::{ContentHash, ResolvedPath};

    #[derive(Default)]
    pub struct FakeState {
        /// `save` で渡された TOML
        pub saves: Vec<String>,
        /// `save_as` で渡されたパスと TOML
        pub save_as: Vec<(PathBuf, String)>,
        /// `reload` で渡されたパス
        pub reloads: Vec<PathBuf>,
        /// 次の `save` を失敗させる
        pub save_error: Option<SaveError>,
        /// 次の `save_as` を失敗させる
        pub save_as_error: Option<SaveError>,
        /// 次の `reload` が返す設定（なければ、読み込みエラー）
        pub reload_to: Option<LoadedConfig>,
    }

    /// 状態を共有するので、`App` に渡した後も、テストから呼び出しを確認できる
    #[derive(Clone, Default)]
    pub struct FakeStorage(pub Rc<RefCell<FakeState>>);

    fn existing(path: &Path, text: &str) -> ConfigSource {
        ConfigSource::Existing {
            path: ResolvedPath {
                requested: path.to_path_buf(),
                real: path.to_path_buf(),
            },
            fingerprint: ContentHash::of(text.as_bytes()),
        }
    }

    impl Storage for FakeStorage {
        fn save(&self, source: &ConfigSource, text: &str) -> Result<ConfigSource, SaveError> {
            let mut state = self.0.borrow_mut();
            state.saves.push(text.to_string());
            if let Some(err) = state.save_error.take() {
                return Err(err);
            }
            let path = match source {
                ConfigSource::Existing { path, .. } => path.requested.clone(),
                ConfigSource::Missing { target, .. } => target.clone(),
            };
            Ok(existing(&path, text))
        }

        fn save_as(&self, path: &Path, text: &str) -> Result<ConfigSource, SaveError> {
            let mut state = self.0.borrow_mut();
            state.save_as.push((path.to_path_buf(), text.to_string()));
            if let Some(err) = state.save_as_error.take() {
                return Err(err);
            }
            Ok(existing(path, text))
        }

        fn reload(&self, path: &Path) -> Result<LoadedConfig, ConfigError> {
            let mut state = self.0.borrow_mut();
            state.reloads.push(path.to_path_buf());
            state.reload_to.take().ok_or(ConfigError::NoConfigDir)
        }
    }
}
