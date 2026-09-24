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
        .map(|name| name.shorten().to_string());

    // 簡易 dirty 判定 (高速性を最優先)
    let is_dirty = repo.is_dirty().unwrap_or(false);

    GitStatus { branch, is_dirty }
}
