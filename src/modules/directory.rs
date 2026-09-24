use std::path::Path;

/// パスをターミナル幅に合わせて中間省略する (/foo/.../bar)
pub fn format_path(path: &Path, max_len: usize) -> String {
    let s = path.to_string_lossy().replace('\\', "/");
    let char_count = s.chars().count();
    if max_len >= 10 && char_count > max_len {
        let head_len = (max_len / 2).saturating_sub(2);
        let tail_len = max_len.saturating_sub(head_len).saturating_sub(5);
        let head: String = s.chars().take(head_len).collect();
        let tail: String = s
            .chars()
            .skip(char_count.saturating_sub(tail_len))
            .collect();
        format!("{head}/.../{tail}")
    } else {
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_path_short() {
        let path = Path::new("C:/Users/jojob/src");
        let formatted = format_path(path, 40);
        assert_eq!(formatted, "C:/Users/jojob/src");
    }

    #[test]
    fn test_format_path_long_truncated() {
        let path = Path::new("C:/Users/jojob/src/very/long/nested/directory/structure/project");
        let formatted = format_path(path, 25);
        assert!(formatted.contains("/.../"));
        assert!(formatted.chars().count() <= 25);
    }

    #[test]
    fn test_format_path_tiny_max_len() {
        let path = Path::new("C:/Users/jojob/src");
        // max_len が極端に小さい場合も panic せずそのまま返す
        let formatted = format_path(path, 5);
        assert_eq!(formatted, "C:/Users/jojob/src");
    }
}
