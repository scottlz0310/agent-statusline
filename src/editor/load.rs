//! 設定ファイルの厳密な読み込み。`render` 用の `load_config` と異なり、失敗を返す。
//!
//! 構文エラーのときは既定値で置き換えず、元の内容とエラー位置を保持して返す。

use std::fmt;
use std::io;
use std::ops::Range;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};
use toml_edit::DocumentMut;

use crate::engine::config::{Config, default_config_path, resolve_config_path_with};

/// シンボリックリンクを辿る最大回数
const MAX_LINK_DEPTH: usize = 40;

/// 設定ファイルの内容ハッシュ。外部変更の検知に使う
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContentHash([u8; 32]);

impl ContentHash {
    pub fn of(bytes: &[u8]) -> Self {
        Self(Sha256::digest(bytes).into())
    }
}

/// 指定されたパスと、リンクを解決した実ファイルのパス
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedPath {
    pub requested: PathBuf,
    pub real: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigSource {
    Existing {
        path: ResolvedPath,
        fingerprint: ContentHash,
    },
    /// まだ作られていない。保存先だけを持ち、履歴・ハッシュは持たない
    Missing { target: PathBuf },
}

#[derive(Debug)]
pub struct LoadedConfig {
    pub config: Config,
    /// コメントや未知のキーを保ったままの編集元
    pub document: DocumentMut,
    pub source: ConfigSource,
}

#[derive(Debug)]
pub enum ConfigError {
    NoConfigDir,
    Read {
        path: PathBuf,
        source: io::Error,
    },
    Parse {
        path: PathBuf,
        /// 読み込んだ元の内容。エラー位置の表示に使う
        content: String,
        message: String,
        span: Option<Range<usize>>,
    },
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoConfigDir => write!(f, "設定ディレクトリを特定できません"),
            Self::Read { path, source } => {
                write!(
                    f,
                    "設定ファイルを読み込めません: {}: {source}",
                    path.display()
                )
            }
            Self::Parse { path, message, .. } => {
                write!(
                    f,
                    "設定ファイルを解析できません: {}: {message}",
                    path.display()
                )
            }
        }
    }
}

impl std::error::Error for ConfigError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Read { source, .. } => Some(source),
            _ => None,
        }
    }
}

/// 設定ファイルの探索に使う環境。テストで差し替えられる
#[derive(Debug, Clone, Default)]
pub struct ConfigEnv {
    pub env_path: Option<String>,
    pub config_dir: Option<PathBuf>,
    pub home_dir: Option<PathBuf>,
}

impl ConfigEnv {
    pub fn from_process() -> Self {
        Self {
            env_path: std::env::var("AGENT_STATUSLINE_CONFIG").ok(),
            config_dir: dirs::config_dir(),
            home_dir: dirs::home_dir(),
        }
    }
}

/// `render` と同じ順序で設定ファイルを探して読み込む。見つからなければ `Missing`（既定値）で開始する
pub fn load_config_strict(
    custom_path: Option<&Path>,
    env: &ConfigEnv,
) -> Result<LoadedConfig, ConfigError> {
    let resolved = resolve_config_path_with(
        custom_path,
        env.env_path.as_deref(),
        env.config_dir.as_deref(),
        env.home_dir.as_deref(),
        Path::exists,
    );
    let requested = match resolved {
        Some(path) => path,
        None => default_config_path(env.config_dir.as_deref().ok_or(ConfigError::NoConfigDir)?),
    };
    load_from_path(&requested)
}

fn load_from_path(requested: &Path) -> Result<LoadedConfig, ConfigError> {
    let read_error = |source| ConfigError::Read {
        path: requested.to_path_buf(),
        source,
    };

    let bytes = match std::fs::read(requested) {
        Ok(bytes) => bytes,
        // リンク先のないシンボリックリンクも NotFound になる
        Err(err) if err.kind() == io::ErrorKind::NotFound => {
            let target = resolve_missing_target(requested).map_err(read_error)?;
            return Ok(LoadedConfig {
                config: Config::default(),
                document: DocumentMut::new(),
                source: ConfigSource::Missing { target },
            });
        }
        Err(err) => return Err(read_error(err)),
    };

    let real = std::fs::canonicalize(requested).map_err(read_error)?;
    let content = String::from_utf8(bytes.clone())
        .map_err(|err| read_error(io::Error::new(io::ErrorKind::InvalidData, err)))?;
    let parse_error = |message: String, span: Option<Range<usize>>| ConfigError::Parse {
        path: requested.to_path_buf(),
        content: content.clone(),
        message,
        span,
    };

    let document: DocumentMut = content
        .parse()
        .map_err(|err: toml_edit::TomlError| parse_error(err.message().to_string(), err.span()))?;
    let config: Config = toml_edit::de::from_str(&content)
        .map_err(|err| parse_error(err.message().to_string(), err.span()))?;

    Ok(LoadedConfig {
        config,
        document,
        source: ConfigSource::Existing {
            path: ResolvedPath {
                requested: requested.to_path_buf(),
                real,
            },
            fingerprint: ContentHash::of(&bytes),
        },
    })
}

/// 存在しないファイルの保存先を決める。リンク切れのリンクは、リンク先のパスにする。
/// 相対リンクは、リンクの親ディレクトリを基準にする。`canonicalize` は対象が存在しないと使えない
fn resolve_missing_target(path: &Path) -> io::Result<PathBuf> {
    let mut current = path.to_path_buf();
    for _ in 0..MAX_LINK_DEPTH {
        match std::fs::read_link(&current) {
            Ok(link) if link.is_absolute() => current = link,
            Ok(link) => {
                current = current.parent().unwrap_or_else(|| Path::new("")).join(link);
            }
            // リンクではない、または存在しない: current が保存先
            Err(err)
                if matches!(
                    err.kind(),
                    io::ErrorKind::NotFound | io::ErrorKind::InvalidInput
                ) =>
            {
                return Ok(current);
            }
            Err(err) => return Err(err),
        }
    }
    Err(io::Error::other(
        "シンボリックリンクが深すぎます（循環の可能性）",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env_in(dir: &Path) -> ConfigEnv {
        ConfigEnv {
            env_path: None,
            config_dir: Some(dir.join("cfg")),
            home_dir: Some(dir.join("home")),
        }
    }

    /// 権限がなくシンボリックリンクを作れない環境（Windows の通常ユーザーなど）では `false`
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

    #[test]
    fn test_load_existing_keeps_document_and_fingerprint() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let content =
            "# コメント\nformat = \"$model\"\nunknown_key = 1\n[model]\nsymbol = \"M \"\n";
        std::fs::write(&path, content).unwrap();

        let loaded = load_config_strict(Some(&path), &ConfigEnv::default()).unwrap();
        assert_eq!(loaded.config.format, "$model");
        assert_eq!(loaded.config.model.symbol, "M ");
        assert_eq!(loaded.document.to_string(), content);
        let ConfigSource::Existing {
            path: resolved,
            fingerprint,
        } = loaded.source
        else {
            panic!("Existing のはず");
        };
        assert_eq!(resolved.requested, path);
        assert_eq!(resolved.real, std::fs::canonicalize(&path).unwrap());
        assert_eq!(fingerprint, ContentHash::of(content.as_bytes()));
        assert_ne!(fingerprint, ContentHash::of(b"other"));
    }

    #[test]
    fn test_load_search_order_and_missing() {
        let dir = tempfile::tempdir().unwrap();
        let env = env_in(dir.path());
        let standard = default_config_path(env.config_dir.as_deref().unwrap());

        // どこにも無い: 標準の場所を保存先にして、既定値で開始する
        let loaded = load_config_strict(None, &env).unwrap();
        assert_eq!(loaded.config.format, Config::default().format);
        assert_eq!(loaded.document.to_string(), "");
        assert_eq!(
            loaded.source,
            ConfigSource::Missing {
                target: standard.clone()
            }
        );

        // 指定されたパスが存在しない: そのパスが保存先
        let custom = dir.path().join("custom.toml");
        let loaded = load_config_strict(Some(&custom), &env).unwrap();
        assert_eq!(
            loaded.source,
            ConfigSource::Missing {
                target: custom.clone()
            }
        );
        let with_env = ConfigEnv {
            env_path: Some(custom.display().to_string()),
            ..env.clone()
        };
        let loaded = load_config_strict(None, &with_env).unwrap();
        assert_eq!(loaded.source, ConfigSource::Missing { target: custom });

        // 標準の場所に作られたら、それを読む
        std::fs::create_dir_all(standard.parent().unwrap()).unwrap();
        std::fs::write(&standard, "format = \"$plan\"").unwrap();
        let loaded = load_config_strict(None, &env).unwrap();
        assert_eq!(loaded.config.format, "$plan");
        assert!(matches!(loaded.source, ConfigSource::Existing { .. }));
    }

    #[test]
    fn test_load_no_config_dir_is_error() {
        let err = load_config_strict(None, &ConfigEnv::default()).unwrap_err();
        assert!(matches!(err, ConfigError::NoConfigDir), "{err}");
    }

    #[test]
    fn test_load_invalid_toml_keeps_content_and_span() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let cases = [
            ("syntax", "format = [[[\n"),
            ("type", "[context]\nthreshold_warning = \"high\"\n"),
        ];
        for (name, content) in cases {
            std::fs::write(&path, content).unwrap();
            let err = load_config_strict(Some(&path), &ConfigEnv::default()).unwrap_err();
            let ConfigError::Parse {
                content: kept,
                message,
                span,
                ..
            } = &err
            else {
                panic!("{name}: Parse のはず: {err}");
            };
            assert_eq!(kept, content, "{name}");
            assert!(!message.is_empty(), "{name}");
            let span = span
                .clone()
                .unwrap_or_else(|| panic!("{name}: span がない"));
            assert!(span.end <= content.len(), "{name}");
            assert!(err.to_string().contains("解析できません"), "{name}");
        }
    }

    #[test]
    fn test_load_non_utf8_is_read_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, [0xff, 0xfe, 0x00]).unwrap();
        let err = load_config_strict(Some(&path), &ConfigEnv::default()).unwrap_err();
        assert!(matches!(err, ConfigError::Read { .. }), "{err}");
    }

    #[test]
    fn test_load_through_symlink_resolves_real_path() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("cloud.toml");
        let link = dir.path().join("link.toml");
        std::fs::write(&real, "format = \"$plan\"").unwrap();
        if !try_symlink(&real, &link) {
            return;
        }
        let loaded = load_config_strict(Some(&link), &ConfigEnv::default()).unwrap();
        let ConfigSource::Existing { path, .. } = loaded.source else {
            panic!("Existing のはず");
        };
        assert_eq!(path.requested, link);
        assert_eq!(path.real, std::fs::canonicalize(&real).unwrap());
    }

    #[test]
    fn test_load_dangling_symlink_is_missing_with_resolved_target() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("cloud").join("config.toml");
        let link = dir.path().join("link.toml");
        // 先の親ディレクトリも存在しない（未同期のクラウドフォルダ）。Missing の判定はそれを問わない
        if !try_symlink(&target, &link) {
            return;
        }
        let loaded = load_config_strict(Some(&link), &ConfigEnv::default()).unwrap();
        assert_eq!(loaded.source, ConfigSource::Missing { target });
    }

    #[test]
    fn test_resolve_missing_target_relative_and_chained_links() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("sub")).unwrap();
        let first = dir.path().join("first.toml");
        let second = dir.path().join("sub").join("second.toml");
        // first -> sub/second.toml（相対）、second -> ../final.toml（相対、リンクの親が基準）
        if !try_symlink(Path::new("sub").join("second.toml").as_path(), &first)
            || !try_symlink(Path::new("..").join("final.toml").as_path(), &second)
        {
            return;
        }
        let target = resolve_missing_target(&first).unwrap();
        assert_eq!(
            std::fs::canonicalize(target.parent().unwrap()).unwrap(),
            std::fs::canonicalize(dir.path()).unwrap()
        );
        assert_eq!(target.file_name().unwrap(), "final.toml");
    }

    #[test]
    fn test_resolve_missing_target_loop_is_error() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.toml");
        let b = dir.path().join("b.toml");
        if !try_symlink(&b, &a) || !try_symlink(&a, &b) {
            return;
        }
        assert!(resolve_missing_target(&a).is_err());
    }
}
