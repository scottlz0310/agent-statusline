use std::path::Path;

/// パスをターミナル幅に合わせて中間省略する (/foo/.../bar)
pub fn format_path(path: &Path, max_len: usize) -> String {
    let s = path.to_string_lossy().replace('\\', "/");
    if max_len > 0 && s.chars().count() > max_len {
        let head_len = max_len / 2 - 2;
        let tail_len = max_len - head_len - 5;
        let head: String = s.chars().take(head_len).collect();
        let tail: String = s.chars().skip(s.chars().count() - tail_len).collect();
        format!("{head}/.../{tail}")
    } else {
        s
    }
}
