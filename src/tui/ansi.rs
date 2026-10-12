//! ANSI の装飾（SGR）つきの文字列を、ratatui の `Line` へ変換する。
//!
//! 描画エンジンが出力する装飾（太字・薄く・斜体・下線・反転、16 色の前景・背景）だけを扱う。

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

pub fn ansi_line(text: &str) -> Line<'static> {
    let mut spans = Vec::new();
    let mut style = Style::default();
    let mut buffer = String::new();
    let mut chars = text.char_indices().peekable();

    while let Some((i, c)) = chars.next() {
        if c == '\x1b' && text[i + 1..].starts_with('[') {
            let params_start = i + 2;
            let Some(end) = text[params_start..].find(|b: char| ('\x40'..='\x7e').contains(&b))
            else {
                break;
            };
            let final_byte = text[params_start + end..].chars().next();
            if !buffer.is_empty() {
                spans.push(Span::styled(std::mem::take(&mut buffer), style));
            }
            if final_byte == Some('m') {
                style = apply_sgr(style, &text[params_start..params_start + end]);
            }
            // シーケンスの終端まで読み飛ばす
            while let Some(&(j, _)) = chars.peek() {
                if j <= params_start + end {
                    chars.next();
                } else {
                    break;
                }
            }
            continue;
        }
        buffer.push(c);
    }
    if !buffer.is_empty() {
        spans.push(Span::styled(buffer, style));
    }
    Line::from(spans)
}

fn apply_sgr(mut style: Style, params: &str) -> Style {
    // `ESC[m` は、パラメーターなしのリセット
    if params.is_empty() {
        return Style::default();
    }
    for code in params.split(';').filter_map(|p| p.parse::<u8>().ok()) {
        style = match code {
            0 => Style::default(),
            1 => style.add_modifier(Modifier::BOLD),
            2 => style.add_modifier(Modifier::DIM),
            3 => style.add_modifier(Modifier::ITALIC),
            4 => style.add_modifier(Modifier::UNDERLINED),
            7 => style.add_modifier(Modifier::REVERSED),
            30..=37 => style.fg(ansi_color(code - 30)),
            90 => style.fg(Color::DarkGray),
            40..=47 => style.bg(ansi_color(code - 40)),
            100 => style.bg(Color::DarkGray),
            _ => style,
        };
    }
    style
}

fn ansi_color(index: u8) -> Color {
    match index {
        0 => Color::Black,
        1 => Color::Red,
        2 => Color::Green,
        3 => Color::Yellow,
        4 => Color::Blue,
        5 => Color::Magenta,
        6 => Color::Cyan,
        _ => Color::Gray,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    type Case = (&'static str, &'static str, Vec<(String, Style)>);

    fn spans(text: &str) -> Vec<(String, Style)> {
        ansi_line(text)
            .spans
            .into_iter()
            .map(|s| (s.content.into_owned(), s.style))
            .collect()
    }

    #[test]
    fn test_ansi_line() {
        let bold_cyan = Style::default()
            .add_modifier(Modifier::BOLD)
            .fg(Color::Cyan);
        let cases: [Case; 7] = [
            ("plain", "hello", vec![("hello".into(), Style::default())]),
            ("empty", "", vec![]),
            (
                "color-then-reset",
                "\x1b[32mok\x1b[0m rest",
                vec![
                    ("ok".into(), Style::default().fg(Color::Green)),
                    (" rest".into(), Style::default()),
                ],
            ),
            (
                "combined-params",
                "\x1b[1;36mfolder\x1b[0m",
                vec![("folder".into(), bold_cyan)],
            ),
            (
                "dim-and-background",
                "\x1b[2m\x1b[41mx\x1b[m",
                vec![(
                    "x".into(),
                    Style::default().add_modifier(Modifier::DIM).bg(Color::Red),
                )],
            ),
            (
                "bright-gray",
                "\x1b[90mgray\x1b[0m",
                vec![("gray".into(), Style::default().fg(Color::DarkGray))],
            ),
            (
                "emoji-and-cjk-text",
                "\x1b[33m🔒 日本語\x1b[0m",
                vec![("🔒 日本語".into(), Style::default().fg(Color::Yellow))],
            ),
        ];
        for (name, input, expected) in cases {
            assert_eq!(spans(input), expected, "{name}");
        }
    }

    #[test]
    fn test_ansi_line_ignores_unknown_and_incomplete_sequences() {
        // 未対応のコードは無視し、途中で終わるシーケンスは捨てる
        assert_eq!(
            spans("\x1b[38;5;200mtext"),
            vec![("text".into(), Style::default())]
        );
        assert_eq!(spans("ab\x1b[3"), vec![("ab".into(), Style::default())]);
    }
}
