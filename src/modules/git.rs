use std::path::Path;

use gix::sec::Trust;
use gix::sec::trust::DefaultForLevel;

use super::git_system_config::system_config_path;

#[derive(Debug, Clone, Default)]
pub struct GitStatus {
    pub branch: Option<String>,
    pub is_dirty: bool,
    /// fetch 用既定リモートが GitHub の場合の `owner/repo`
    pub remote_repo: Option<String>,
}

/// 描画に必要な項目だけを取得するための指定
#[derive(Debug, Clone, Copy, Default)]
pub struct GitQuery {
    pub dirty: bool,
    pub remote_repo: bool,
}

/// gix によるインプロセス Git 情報取得
pub fn get_git_status(path: &Path, query: GitQuery) -> GitStatus {
    let Some(repo) = discover(path, system_config_path().as_deref()) else {
        return GitStatus::default();
    };

    let branch = repo
        .head_name()
        .ok()
        .flatten()
        .map(|name| name.shorten().to_string())
        .or_else(|| {
            repo.head_id()
                .ok()
                .map(|id| id.to_hex_with_len(7).to_string())
        });

    // 簡易 dirty 判定 (高速性を最優先)
    let is_dirty = query.dirty && repo.is_dirty().unwrap_or(false);

    // 現在ブランチの upstream リモート → origin → 唯一のリモートの順で解決される
    let remote_repo = if query.remote_repo {
        repo.find_default_remote(gix::remote::Direction::Fetch)
            .and_then(Result::ok)
            .and_then(|remote| {
                remote
                    .url(gix::remote::Direction::Fetch)
                    .and_then(|url| github_repo_slug(url.host(), &url.path.to_string()))
            })
    } else {
        None
    };

    GitStatus {
        branch,
        is_dirty,
        remote_repo,
    }
}

fn discover(path: &Path, system_config: Option<&Path>) -> Option<gix::Repository> {
    let trust_map = gix::sec::trust::Mapping {
        full: open_options(Trust::Full, system_config),
        reduced: open_options(Trust::Reduced, system_config),
    };
    gix::ThreadSafeRepository::discover_opts(
        path,
        gix::discover::upwards::Options::default(),
        trust_map,
    )
    .ok()
    .map(Into::into)
}

/// 描画のたびに `git.exe` を起動しないよう、システム設定の場所を gix に探させない
fn open_options(level: Trust, system_config: Option<&Path>) -> gix::open::Options {
    let mut permissions = gix::open::Permissions::default_for_level(level);
    permissions.config.system &= system_config.is_some();
    // Windows ではシステム gitattributes の場所を調べるためにも `git.exe` が起動する。
    // Git for Windows 既定の内容は diff ドライバの指定だけで、変更状態の判定には影響しない。
    if cfg!(windows) {
        permissions.attributes.system = false;
    }
    let options = gix::open::Options::default_for_level(level).permissions(permissions);
    match system_config {
        Some(path) => options.system_config_path(path),
        None => options,
    }
}

/// GitHub のリモート URL (ホストとパス) から `owner/repo` を取り出す。
/// Squirrel Notifier のサマリは GitHub の `owner/repo` だけを持つため、他ホストは対象外とする。
/// `~/.ssh/config` のホスト別名は gix が解決しないため一致しない。
fn github_repo_slug(host: Option<&str>, path: &str) -> Option<String> {
    let host = host?;
    if !(host.eq_ignore_ascii_case("github.com") || host.eq_ignore_ascii_case("ssh.github.com")) {
        return None;
    }
    let path = path.trim_matches('/');
    let path = path.strip_suffix(".git").unwrap_or(path);
    let (owner, repo) = path.split_once('/')?;
    if owner.is_empty() || repo.is_empty() || repo.contains('/') {
        return None;
    }
    Some(format!("{owner}/{repo}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fmt::Write;

    #[test]
    fn test_get_git_status_in_repo() {
        let status = get_git_status(Path::new("."), GitQuery::default());
        assert!(status.branch.is_some());
        let b = status.branch.unwrap();
        assert_ne!(b, "");
        assert!(status.remote_repo.is_none());
    }

    #[test]
    fn test_get_git_status_non_repo() {
        let temp_dir = std::env::temp_dir();
        // temp ディレクトリ直下は通常 git リポジトリではないか、discover で安全に処理される
        let _status = get_git_status(
            &temp_dir,
            GitQuery {
                dirty: true,
                remote_repo: true,
            },
        );
    }

    #[test]
    fn test_discover_loads_only_given_system_config() {
        let dir = tempfile::tempdir().unwrap();
        gix::init(dir.path()).unwrap();
        let system_config = dir.path().join("system-gitconfig");
        std::fs::write(&system_config, "[agentstatusline]\n\tprobe = true\n").unwrap();

        let cases = [(Some(system_config.as_path()), true), (None, false)];
        for (system_config, expected) in cases {
            let repo = discover(dir.path(), system_config).unwrap();
            let config = repo.config_snapshot();
            let has_system_section = config
                .plumbing()
                .sections()
                .any(|section| section.meta().source == gix::config::Source::System);
            assert_eq!(has_system_section, expected, "{system_config:?}");
            assert_eq!(
                config.boolean("agentstatusline.probe"),
                expected.then_some(true),
                "{system_config:?}"
            );
        }
    }

    #[test]
    fn test_remote_repo_from_configured_remote() {
        let cases = [
            ("git@github.com:Owner/Repo.git", Some("Owner/Repo")),
            ("https://github.com/Owner/Repo", Some("Owner/Repo")),
            ("git@gitlab.com:Owner/Repo.git", None),
        ];
        for (url, expected) in cases {
            let dir = tempfile::tempdir().unwrap();
            gix::init(dir.path()).unwrap();
            let config = dir.path().join(".git").join("config");
            let mut content = std::fs::read_to_string(&config).unwrap();
            write!(content, "[remote \"origin\"]\n\turl = {url}\n").unwrap();
            std::fs::write(&config, content).unwrap();

            let status = get_git_status(
                dir.path(),
                GitQuery {
                    dirty: false,
                    remote_repo: true,
                },
            );
            assert_eq!(status.remote_repo.as_deref(), expected, "{url}");
        }
    }

    #[test]
    fn test_github_repo_slug() {
        let cases = [
            (
                Some("github.com"),
                "/scottlz0310/agent-statusline.git",
                Some("scottlz0310/agent-statusline"),
            ),
            (
                Some("GitHub.com"),
                "scottlz0310/agent-statusline",
                Some("scottlz0310/agent-statusline"),
            ),
            (
                Some("ssh.github.com"),
                "/scottlz0310/agent-statusline/",
                Some("scottlz0310/agent-statusline"),
            ),
            (
                Some("gitlab.com"),
                "/scottlz0310/agent-statusline.git",
                None,
            ),
            (
                Some("github.example.com"),
                "/scottlz0310/agent-statusline.git",
                None,
            ),
            (None, "/scottlz0310/agent-statusline.git", None),
            (
                Some("github.com"),
                "/git/scottlz0310/agent-statusline.git",
                None,
            ),
            (Some("github.com"), "/agent-statusline.git", None),
            (Some("github.com"), "", None),
        ];
        for (host, path, expected) in cases {
            assert_eq!(
                github_repo_slug(host, path).as_deref(),
                expected,
                "{host:?} {path}"
            );
        }
    }
}
