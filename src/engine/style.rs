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

/// スタイル仕様文字列（例: "bold cyan", "red", "dimmed"）を ANSI コード群へパースする
pub fn parse_style_tokens(style_spec: &str) -> Vec<u8> {
    let mut codes = Vec::new();
    for token in style_spec.split_whitespace() {
        match token.to_ascii_lowercase().as_str() {
            "bold" => codes.push(1),
            "dimmed" => codes.push(2),
            "italic" => codes.push(3),
            "underline" => codes.push(4),
            "inverted" => codes.push(7),
            "black" => codes.push(30),
            "red" => codes.push(31),
            "green" => codes.push(32),
            "yellow" => codes.push(33),
            "blue" => codes.push(34),
            "magenta" | "purple" => codes.push(35),
            "cyan" => codes.push(36),
            "white" => codes.push(37),
            "gray" | "grey" => codes.push(90),
            "bg:black" => codes.push(40),
            "bg:red" => codes.push(41),
            "bg:green" => codes.push(42),
            "bg:yellow" => codes.push(43),
            "bg:blue" => codes.push(44),
            "bg:magenta" | "bg:purple" => codes.push(45),
            "bg:cyan" => codes.push(46),
            "bg:white" => codes.push(47),
            "bg:gray" | "bg:grey" => codes.push(100),
            _ => {}
        }
    }
    codes
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

fn unescape_brackets(s: &str) -> String {
    s.replace(r"\[", "[").replace(r"\]", "]")
}

/// インラインスタイル構文 `[text](style)` を ANSI 装飾付き文字列に変換する
pub fn render_styled_text(input: &str) -> String {
    let mut result = String::with_capacity(input.len());
    let mut chars = input.char_indices().peekable();

    while let Some((i, c)) = chars.next() {
        if c == '\\' {
            if let Some(&(_, next_c)) = chars.peek()
                && (next_c == '[' || next_c == ']' || next_c == '\\')
            {
                result.push(next_c);
                chars.next();
                continue;
            }
            result.push(c);
            continue;
        }

        if c == '['
            && let Some(close_bracket_pos) = input[i + 1..].find("](")
        {
            let mid = i + 1 + close_bracket_pos;
            let text = &input[i + 1..mid];
            let after_paren = mid + 2;
            if let Some(close_paren_pos) = input[after_paren..].find(')') {
                let end = after_paren + close_paren_pos;
                let style_spec = &input[after_paren..end];

                let codes = parse_style_tokens(style_spec);
                let unescaped_text = unescape_brackets(text);
                if codes.is_empty() {
                    result.push_str(&unescaped_text);
                } else {
                    result.push_str(&style_to_ansi(&codes));
                    result.push_str(&unescaped_text);
                    result.push_str(RESET);
                }

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

        result.push(c);
    }

    result
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
}
