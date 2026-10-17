use std::path::Path;

/// パスを省略する位置
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TruncationMode {
    /// 先頭側を省略する (`.../foo/bar`)
    Left,
    /// 中間を省略する (`/foo/.../bar`)
    Middle,
}

impl TruncationMode {
    /// 設定 `[directory] truncation_mode` で指定できる値
    pub const CHOICES: [&'static str; 2] = ["left", "middle"];

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "left" => Some(Self::Left),
            "middle" => Some(Self::Middle),
            _ => None,
        }
    }
}

/// 省略できる上限の最小値。これより短い上限では、省略の結果が読めなくなるので省略しない
pub const MIN_PATH_LEN: usize = 10;

/// `[directory] max_length` の最大値
pub const MAX_PATH_LEN: usize = 1000;

/// `[directory] max_length` として受け付ける値。0 は「端末の幅の半分」（自動）
pub fn is_valid_max_length(value: i64) -> bool {
    usize::try_from(value).is_ok_and(|v| v == 0 || (MIN_PATH_LEN..=MAX_PATH_LEN).contains(&v))
}

/// パスを `max_len` 文字以内に省略する。`max_len` が `MIN_PATH_LEN` 未満のときは省略しない
pub fn format_path(path: &Path, max_len: usize, mode: TruncationMode) -> String {
    let s = path.to_string_lossy().replace('\\', "/");
    let char_count = s.chars().count();
    if max_len < MIN_PATH_LEN || char_count <= max_len {
        return s;
    }

    match mode {
        TruncationMode::Middle => {
            let head_len = (max_len / 2).saturating_sub(2);
            let tail_len = max_len.saturating_sub(head_len).saturating_sub(5);
            let head: String = s.chars().take(head_len).collect();
            let tail: String = s
                .chars()
                .skip(char_count.saturating_sub(tail_len))
                .collect();
            format!("{head}/.../{tail}")
        }
        TruncationMode::Left => {
            let tail_len = max_len.saturating_sub(3);
            let tail: String = s
                .chars()
                .skip(char_count.saturating_sub(tail_len))
                .collect();
            // 末尾側の最初の区切りから残し、途中で切れた名前を ...// のように見せない
            tail.find('/')
                .map_or_else(|| format!("...{tail}"), |i| format!("...{}", &tail[i..]))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LONG: &str = "C:/Users/jojob/src/very/long/nested/directory/structure/project";

    #[test]
    fn test_format_path_short() {
        for mode in [TruncationMode::Left, TruncationMode::Middle] {
            let formatted = format_path(Path::new("C:/Users/jojob/src"), 40, mode);
            assert_eq!(formatted, "C:/Users/jojob/src", "{mode:?}");
        }
    }

    #[test]
    fn test_format_path_long_truncated() {
        // (モード, 最大幅, 期待する結果)
        let cases = [
            (TruncationMode::Middle, 25, "C:/Users/j/.../re/project"),
            (TruncationMode::Left, 25, ".../structure/project"),
            (TruncationMode::Left, 10, "...project"),
            (TruncationMode::Middle, 10, "C://.../ct"),
        ];
        for (mode, max_len, expected) in cases {
            let formatted = format_path(Path::new(LONG), max_len, mode);
            assert!(
                formatted.chars().count() <= max_len,
                "{mode:?}/{max_len}: {formatted}"
            );
            assert_eq!(formatted, expected, "{mode:?}/{max_len}");
        }
    }

    #[test]
    fn test_format_path_tiny_max_len() {
        // max_len が極端に小さい場合も panic せずそのまま返す
        for mode in [TruncationMode::Left, TruncationMode::Middle] {
            let formatted = format_path(Path::new("C:/Users/jojob/src"), 5, mode);
            assert_eq!(formatted, "C:/Users/jojob/src", "{mode:?}");
        }
    }

    #[test]
    fn test_truncation_mode_parse() {
        let cases = [
            ("left", Some(TruncationMode::Left)),
            ("middle", Some(TruncationMode::Middle)),
            ("Left", None),
            ("right", None),
            ("", None),
        ];
        for (value, expected) in cases {
            assert_eq!(TruncationMode::parse(value), expected, "{value:?}");
        }
        for choice in TruncationMode::CHOICES {
            assert!(TruncationMode::parse(choice).is_some(), "{choice}");
        }
    }
}
