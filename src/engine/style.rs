#![allow(dead_code)]

pub const RESET: &str = "\x1b[0m";
pub const BOLD: &str = "\x1b[1m";
pub const DIMMED: &str = "\x1b[2m";
pub const ITALIC: &str = "\x1b[3m";
pub const UNDERLINE: &str = "\x1b[4m";
pub const INVERTED: &str = "\x1b[7m";

pub const RED: &str = "\x1b[31m";
pub const GREEN: &str = "\x1b[32m";
pub const YELLOW: &str = "\x1b[33m";
pub const BLUE: &str = "\x1b[34m";
pub const MAGENTA: &str = "\x1b[35m";
pub const CYAN: &str = "\x1b[36m";
pub const GRAY: &str = "\x1b[90m";

/// パーセンテージに応じた ANSI 色を返す
pub fn pct_color(pct: u8) -> &'static str {
    if pct >= 80 {
        RED
    } else if pct >= 50 {
        YELLOW
    } else {
        GREEN
    }
}

/// パーセンテージに応じたスタイル名を返す
pub fn pct_style_name<'a>(
    pct: u8,
    normal: &'a str,
    warning: &'a str,
    critical: &'a str,
    th_warn: u8,
    th_crit: u8,
) -> &'a str {
    if pct >= th_crit {
        critical
    } else if pct >= th_warn {
        warning
    } else {
        normal
    }
}

/// スタイルのトークン 1 つに対応する ANSI コード。認識できなければ `None`
pub fn style_code(token: &str) -> Option<u8> {
    let code = match token.to_ascii_lowercase().as_str() {
        "bold" => 1,
        "dimmed" => 2,
        "italic" => 3,
        "underline" => 4,
        "inverted" => 7,
        "black" => 30,
        "red" => 31,
        "green" => 32,
        "yellow" => 33,
        "blue" => 34,
        "magenta" | "purple" => 35,
        "cyan" => 36,
        "white" => 37,
        "gray" | "grey" => 90,
        "bg:black" => 40,
        "bg:red" => 41,
        "bg:green" => 42,
        "bg:yellow" => 43,
        "bg:blue" => 44,
        "bg:magenta" | "bg:purple" => 45,
        "bg:cyan" => 46,
        "bg:white" => 47,
        "bg:gray" | "bg:grey" => 100,
        _ => return None,
    };
    Some(code)
}

/// スタイル仕様文字列（例: "bold cyan", "red", "dimmed"）を ANSI コード群へパースする。認識できないトークンは無視する
pub fn parse_style_tokens(style_spec: &str) -> Vec<u8> {
    style_spec
        .split_whitespace()
        .filter_map(style_code)
        .collect()
}

/// スタイル仕様文字列の中の、認識できないトークン
pub fn unknown_style_tokens(style_spec: &str) -> Vec<&str> {
    style_spec
        .split_whitespace()
        .filter(|token| style_code(token).is_none())
        .collect()
}
/// ANSI コード列を `\x1b[...m` 形式にフォーマットする
pub fn style_to_ansi(codes: &[u8]) -> String {
    if codes.is_empty() {
        String::new()
    } else {
        let codes_str: Vec<String> = codes.iter().map(std::string::ToString::to_string).collect();
        format!("\x1b[{}m", codes_str.join(";"))
    }
}

/// インラインスタイル構文 `[text](style)` を分解した中間表現の 1 スパン
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StyledSpan {
    pub text: String,
    pub codes: Vec<u8>,
    pub href: Option<String>,
}

/// 入力文字列中のバイト範囲 `[start..end)` に紐づくハイパーリンク
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkRange {
    pub start: usize,
    pub end: usize,
    pub href: String,
}

fn href_at(links: &[LinkRange], byte_idx: usize) -> Option<&str> {
    links
        .iter()
        .find(|link| byte_idx >= link.start && byte_idx < link.end)
        .map(|link| link.href.as_str())
}

fn push_char(spans: &mut Vec<StyledSpan>, ch: char, codes: &[u8], href: Option<&str>) {
    if let Some(last) = spans.last_mut()
        && last.codes.as_slice() == codes
        && last.href.as_deref() == href
    {
        last.text.push(ch);
    } else {
        spans.push(StyledSpan {
            text: ch.to_string(),
            codes: codes.to_vec(),
            href: href.map(str::to_string),
        });
    }
}

fn push_bracket_inner(
    spans: &mut Vec<StyledSpan>,
    input: &str,
    start: usize,
    end: usize,
    codes: &[u8],
    links: &[LinkRange],
) {
    let slice = &input[start..end];
    let mut chars = slice.char_indices().peekable();
    while let Some((rel_i, ch)) = chars.next() {
        let abs_i = start + rel_i;
        if ch == '\\'
            && let Some(&(next_rel_i, next_ch)) = chars.peek()
            && (next_ch == '[' || next_ch == ']')
        {
            let next_abs_i = start + next_rel_i;
            push_char(spans, next_ch, codes, href_at(links, next_abs_i));
            chars.next();
            continue;
        }
        push_char(spans, ch, codes, href_at(links, abs_i));
    }
}

/// インラインスタイル構文 `[text](style)` をスタイル付きスパン列へ解析する
pub fn parse_styled_spans(input: &str) -> Vec<StyledSpan> {
    parse_styled_spans_with_links(input, &[])
}

/// インラインスタイル構文 `[text](style)` を解析しつつ、入力上のバイト範囲 `links` を
/// 対応するスパンの `href` へ引き継ぐ
pub fn parse_styled_spans_with_links(input: &str, links: &[LinkRange]) -> Vec<StyledSpan> {
    let mut spans = Vec::new();
    let mut chars = input.char_indices().peekable();

    while let Some((i, c)) = chars.next() {
        if c == '\\' {
            if let Some(&(next_i, next_c)) = chars.peek()
                && (next_c == '[' || next_c == ']' || next_c == '\\')
            {
                push_char(&mut spans, next_c, &[], href_at(links, next_i));
                chars.next();
                continue;
            }
            push_char(&mut spans, c, &[], href_at(links, i));
            continue;
        }

        if c == '['
            && let Some(close_bracket_pos) = input[i + 1..].find("](")
        {
            let mid = i + 1 + close_bracket_pos;
            let after_paren = mid + 2;
            if let Some(close_paren_pos) = input[after_paren..].find(')') {
                let end = after_paren + close_paren_pos;
                let style_spec = &input[after_paren..end];
                let codes = parse_style_tokens(style_spec);
                push_bracket_inner(&mut spans, input, i + 1, mid, &codes, links);

                // chars を end 以降に進める
                while let Some(&(next_idx, _)) = chars.peek() {
                    if next_idx <= end {
                        chars.next();
                    } else {
                        break;
                    }
                }
                continue;
            }
        }

        push_char(&mut spans, c, &[], href_at(links, i));
    }

    spans
}

/// スタイル付きスパン列を ANSI 装飾付き文字列へ変換する。
/// ANSI 出力は `href` を使わないため、同じ `codes` が連続するスパンはまとめて 1 回の SGR 装飾にする
pub fn render_styled_spans(spans: &[StyledSpan]) -> String {
    let mut result = String::new();
    let mut i = 0;
    while i < spans.len() {
        let codes = &spans[i].codes;
        let mut j = i + 1;
        while j < spans.len() && spans[j].codes == *codes {
            j += 1;
        }
        if codes.is_empty() {
            for span in &spans[i..j] {
                result.push_str(&span.text);
            }
        } else {
            result.push_str(&style_to_ansi(codes));
            for span in &spans[i..j] {
                result.push_str(&span.text);
            }
            result.push_str(RESET);
        }
        i = j;
    }
    result
}

/// インラインスタイル構文 `[text](style)` を ANSI 装飾付き文字列に変換する
pub fn render_styled_text(input: &str) -> String {
    render_styled_spans(&parse_styled_spans(input))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pct_color() {
        let cases = [
            (0, GREEN),
            (49, GREEN),
            (50, YELLOW),
            (79, YELLOW),
            (80, RED),
            (100, RED),
        ];

        for (pct, expected) in cases {
            assert_eq!(pct_color(pct), expected, "failed for pct: {pct}");
        }
    }

    #[test]
    fn test_pct_style_name() {
        assert_eq!(
            pct_style_name(10, "green", "yellow", "red", 50, 80),
            "green"
        );
        assert_eq!(
            pct_style_name(50, "green", "yellow", "red", 50, 80),
            "yellow"
        );
        assert_eq!(pct_style_name(85, "green", "yellow", "red", 50, 80), "red");
    }

    #[test]
    fn test_parse_style_tokens() {
        assert_eq!(parse_style_tokens("bold cyan"), vec![1, 36]);
        assert_eq!(parse_style_tokens("dimmed red bg:blue"), vec![2, 31, 44]);
        assert_eq!(parse_style_tokens("none"), Vec::<u8>::new());
        assert_eq!(parse_style_tokens(""), Vec::<u8>::new());
    }

    #[test]
    fn test_render_styled_text() {
        let out = render_styled_text("hello [world](bold red) test");
        assert_eq!(out, "hello \x1b[1;31mworld\x1b[0m test");

        let tmpl = "[$symbol$branch]($style)";
        let filled = tmpl
            .replace("$symbol", "🌿 ")
            .replace("$branch", "main")
            .replace("$style", "green");
        let rendered = render_styled_text(&filled);
        assert_eq!(rendered, "\x1b[32m🌿 main\x1b[0m");

        let nested_paren = render_styled_text("[($tokens tok)](dimmed)");
        assert_eq!(nested_paren, "\x1b[2m($tokens tok)\x1b[0m");

        let plain = render_styled_text("no style here");
        assert_eq!(plain, "no style here");

        let escaped = render_styled_text(r"\[not styled\]");
        assert_eq!(escaped, "[not styled]");

        let styled_bracketed = render_styled_text(r"[\[bracketed\]](dimmed)");
        assert_eq!(styled_bracketed, "\x1b[2m[bracketed]\x1b[0m");
    }

    #[test]
    fn test_parse_styled_spans_merges_adjacent_and_preserves_escapes() {
        let spans = parse_styled_spans(r"a [b](bold cyan)[c](bold cyan) [d](unknown) \[e\]");
        assert_eq!(
            spans,
            vec![
                StyledSpan {
                    text: "a ".to_string(),
                    codes: vec![],
                    href: None,
                },
                StyledSpan {
                    text: "bc".to_string(),
                    codes: vec![1, 36],
                    href: None,
                },
                StyledSpan {
                    text: " d [e]".to_string(),
                    codes: vec![],
                    href: None,
                },
            ]
        );
    }

    #[test]
    fn test_parse_styled_spans_with_links_preserves_style_and_escapes() {
        let input = r"[Review \[#42\]|#43](red)";
        let link42_start = input.find("#42").unwrap();
        let link43_start = input.find("#43").unwrap();
        let links = vec![
            LinkRange {
                start: link42_start,
                end: link42_start + 3,
                href: "https://github.com/o/r/pull/42".to_string(),
            },
            LinkRange {
                start: link43_start,
                end: link43_start + 3,
                href: "https://github.com/o/r/pull/43".to_string(),
            },
        ];
        let spans = parse_styled_spans_with_links(input, &links);
        assert_eq!(
            spans,
            vec![
                StyledSpan {
                    text: "Review [".to_string(),
                    codes: vec![31],
                    href: None,
                },
                StyledSpan {
                    text: "#42".to_string(),
                    codes: vec![31],
                    href: Some("https://github.com/o/r/pull/42".to_string()),
                },
                StyledSpan {
                    text: "]|".to_string(),
                    codes: vec![31],
                    href: None,
                },
                StyledSpan {
                    text: "#43".to_string(),
                    codes: vec![31],
                    href: Some("https://github.com/o/r/pull/43".to_string()),
                },
            ]
        );
        assert_eq!(
            render_styled_spans(&spans),
            "\x1b[31mReview [#42]|#43\x1b[0m"
        );
    }
}
