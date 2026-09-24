use std::path::Path;

#[derive(Debug, Clone, Default)]
pub struct GitStatus {
    pub branch: Option<String>,
    pub is_dirty: bool,
}

/// gix によるインプロセス Git 情報取得 (git.exe 呼び出しなし)
pub fn get_git_status(path: &Path) -> GitStatus {
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
    let is_dirty = repo.is_dirty().unwrap_or(false);

    GitStatus { branch, is_dirty }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_git_status_in_repo() {
        let status = get_git_status(Path::new("."));
        assert!(status.branch.is_some());
        let b = status.branch.unwrap();
        assert!(!b.is_empty());
    }

    #[test]
    fn test_get_git_status_non_repo() {
        let temp_dir = std::env::temp_dir();
        // temp ディレクトリ直下は通常 git リポジトリではないか、discover で安全に処理される
        let _status = get_git_status(&temp_dir);
    }
}
