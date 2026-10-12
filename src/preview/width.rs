//! プレビューの幅の模擬。ANSI の装飾（SGR）を保ったまま、表示幅（`unicode-width`）で折り返す・切り詰める。
//!
//! 表示幅は `unicode-width` の見積もりで、端末やフォントの実際の幅と食い違うことがある
//! （異体字セレクタ、ZWJ で結合された絵文字、East Asian Ambiguous など）。

use unicode_width::UnicodeWidthChar;

const RESET: &str = "\x1b[0m";

/// 幅より長い行の扱い。実端末では、エージェント CLI 側の表示方式で変わる
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Overflow {
    /// 次の行へ折り返す
    Wrap,
    /// 超えた分を切り詰める
    Truncate,
}

/// 文字列の一部。装飾のエスケープシーケンスは幅を持たない
enum Piece<'a> {
    Sequence(&'a str),
    Char(char),
}

fn pieces(text: &str) -> impl Iterator<Item = Piece<'_>> {
    let mut rest = text;
    std::iter::from_fn(move || {
        let c = rest.chars().next()?;
        if c == '\x1b' && rest[1..].starts_with('[') {
            // CSI シーケンス: 終端（0x40..=0x7e）まで
            let end = rest[2..]
                .find(|b: char| ('\x40'..='\x7e').contains(&b))
                .map_or(rest.len(), |i| 2 + i + 1);
            let (sequence, tail) = rest.split_at(end);
            rest = tail;
            return Some(Piece::Sequence(sequence));
        }
        rest = &rest[c.len_utf8()..];
        Some(Piece::Char(c))
    })
}

/// 装飾を除いた表示幅
pub fn display_width(text: &str) -> usize {
    pieces(text)
        .map(|piece| match piece {
            Piece::Sequence(_) => 0,
            Piece::Char(c) => c.width().unwrap_or(0),
        })
        .sum()
}

/// 現在有効な装飾。リセットで空になる
#[derive(Default)]
struct ActiveStyle(String);

impl ActiveStyle {
    fn apply(&mut self, sequence: &str) {
        if sequence == RESET || sequence == "\x1b[m" {
            self.0.clear();
        } else if sequence.ends_with('m') {
            self.0.push_str(sequence);
        }
    }
}

/// 1 行を、幅 `width` に合わせて 1 行以上に分ける。折り返しでは、行をまたいで装飾を引き継ぐ
pub fn fit_line(line: &str, width: usize, overflow: Overflow) -> Vec<String> {
    let width = width.max(1);
    let mut lines = Vec::new();
    let mut current = String::new();
    let mut current_width = 0;
    let mut style = ActiveStyle::default();

    for piece in pieces(line) {
        match piece {
            Piece::Sequence(sequence) => {
                style.apply(sequence);
                current.push_str(sequence);
            }
            Piece::Char(c) => {
                let char_width = c.width().unwrap_or(0);
                if current_width + char_width > width && current_width > 0 {
                    if overflow == Overflow::Truncate {
                        break;
                    }
                    if !style.0.is_empty() {
                        current.push_str(RESET);
                    }
                    lines.push(std::mem::take(&mut current));
                    current.push_str(&style.0);
                    current_width = 0;
                }
                current.push(c);
                current_width += char_width;
            }
        }
    }
    if !style.0.is_empty() && !current.ends_with(RESET) {
        current.push_str(RESET);
    }
    lines.push(current);
    lines
}

/// 複数行の出力を、行ごとに `fit_line` する
pub fn fit_output(output: &str, width: usize, overflow: Overflow) -> Vec<String> {
    output
        .lines()
        .flat_map(|line| fit_line(line, width, overflow))
        .collect()
}

/// 装飾を取り除く
pub fn strip_ansi(text: &str) -> String {
    pieces(text)
        .filter_map(|piece| match piece {
            Piece::Sequence(_) => None,
            Piece::Char(c) => Some(c),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const BOLD: &str = "\x1b[1m";

    #[test]
    fn test_display_width() {
        let cases = [
            ("ascii", "hello", 5),
            ("empty", "", 0),
            ("ansi-ignored", "\x1b[1;36mhello\x1b[0m", 5),
            ("cjk-is-double", "日本語", 6),
            ("emoji-is-double", "🌿 main", 7),
            ("mixed", "📁 ab日", 7),
            ("lone-escape", "a\x1bb", 2),
        ];
        for (name, text, expected) in cases {
            assert_eq!(display_width(text), expected, "{name}");
        }
    }

    #[test]
    fn test_fit_line_fits_unchanged() {
        for overflow in [Overflow::Wrap, Overflow::Truncate] {
            assert_eq!(fit_line("hello", 5, overflow), ["hello"], "{overflow:?}");
            assert_eq!(fit_line("", 5, overflow), [""], "{overflow:?}");
        }
    }

    #[test]
    fn test_fit_line_wrap_and_truncate() {
        // (名前, 入力, 幅, 折り返し, 切り詰め)
        let cases: [(&str, &str, usize, &[&str], &str); 5] = [
            ("ascii", "abcdefg", 3, &["abc", "def", "g"], "abc"),
            ("cjk-boundary", "日本語", 5, &["日本", "語"], "日本"),
            ("emoji", "🌿🌿🌿", 4, &["🌿🌿", "🌿"], "🌿🌿"),
            // 1 文字が幅より広くても、無限に分けず 1 文字ずつ出す
            ("wider-than-width", "日本", 1, &["日", "本"], "日"),
            ("zero-width-is-one", "ab", 0, &["a", "b"], "a"),
        ];
        for (name, input, width, wrapped, truncated) in cases {
            assert_eq!(
                fit_line(input, width, Overflow::Wrap),
                wrapped,
                "{name}/wrap"
            );
            assert_eq!(
                fit_line(input, width, Overflow::Truncate),
                [truncated],
                "{name}/truncate"
            );
        }
    }

    #[test]
    fn test_fit_line_keeps_styles_across_wrap() {
        let line = format!("{BOLD}abcd\x1b[0mef");
        assert_eq!(
            fit_line(&line, 3, Overflow::Wrap),
            [format!("{BOLD}abc{RESET}"), format!("{BOLD}d{RESET}ef"),]
        );
        // 切り詰めでも、開いた装飾を閉じる
        assert_eq!(
            fit_line(&line, 3, Overflow::Truncate),
            [format!("{BOLD}abc{RESET}")]
        );
        // 装飾のない行は、余計なリセットを付けない
        assert_eq!(fit_line("abcd", 2, Overflow::Wrap), ["ab", "cd"]);
    }

    #[test]
    fn test_fit_output_splits_lines() {
        let lines = fit_output("abcd\nxy\n\nlongline", 3, Overflow::Wrap);
        assert_eq!(lines, ["abc", "d", "xy", "", "lon", "gli", "ne"]);
        let lines = fit_output("abcd\nxy", 3, Overflow::Truncate);
        assert_eq!(lines, ["abc", "xy"]);
    }

    #[test]
    fn test_fit_line_width_never_exceeds() {
        let line =
            "\x1b[1;36m📁 日本語のプロジェクト名 \x1b[0m\x1b[2m[/very/long/path/日本語]\x1b[0m";
        for width in 1..30 {
            for overflow in [Overflow::Wrap, Overflow::Truncate] {
                for fitted in fit_line(line, width, overflow) {
                    let w = display_width(&fitted);
                    // 1 文字が幅より広いときだけ、幅を超えてよい
                    assert!(w <= width.max(2), "{width}/{overflow:?}: {w} {fitted:?}");
                }
            }
        }
        // 折り返しは、装飾を除いた文字を失わない
        let wrapped: String = fit_line(line, 7, Overflow::Wrap)
            .iter()
            .map(|l| strip_ansi(l))
            .collect();
        assert_eq!(wrapped, strip_ansi(line));
    }
}
