use std::path::Path;

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

/// gix によるインプロセス Git 情報取得 (git.exe 呼び出しなし)
pub fn get_git_status(path: &Path, query: GitQuery) -> GitStatus {
    let Ok(repo) = gix::discover(path) else {
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

    #[test]
    fn test_get_git_status_in_repo() {
        let status = get_git_status(Path::new("."), GitQuery::default());
        assert!(status.branch.is_some());
        let b = status.branch.unwrap();
        assert!(!b.is_empty());
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
            content.push_str(&format!("[remote \"origin\"]\n\turl = {url}\n"));
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
