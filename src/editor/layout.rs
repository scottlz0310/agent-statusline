//! トップレベル `format` の配置モデル。`format` と損失なしに往復できる。
//!
//! `format` は、行（`\n` 区切り）と、行内の並び（部品と固定テキスト）で構成される。
//! 認識できない `$xxx` は固定テキストとして保持し、描画と同じく置換されない。

use crate::engine::parts::part_ids;
use crate::engine::template::{Token as LexToken, collect_dollar_names, tokenize};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Token {
    Part(String),
    Literal(String),
}

pub type Line = Vec<Token>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layout {
    pub lines: Vec<Line>,
}

impl Layout {
    pub fn parse(format: &str) -> Self {
        let ids = part_ids();
        let lines = format
            .split('\n')
            .map(|line| {
                tokenize(line, &ids)
                    .into_iter()
                    .map(|token| match token {
                        LexToken::Var(id) => Token::Part(id.to_string()),
                        LexToken::Text(text) => Token::Literal(text.to_string()),
                    })
                    .collect()
            })
            .collect();
        Self { lines }
    }

    pub fn to_format(&self) -> String {
        self.lines
            .iter()
            .map(|line| {
                line.iter()
                    .map(|token| match token {
                        Token::Part(id) => format!("${id}"),
                        Token::Literal(text) => text.clone(),
                    })
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// 固定テキストに残った `$名前`（未知の変数）。プレビューと検証で警告する
    pub fn unknown_variables(&self) -> Vec<String> {
        let mut found: Vec<String> = Vec::new();
        for token in self.lines.iter().flatten() {
            let Token::Literal(text) = token else {
                continue;
            };
            collect_dollar_names(text, &mut found);
        }
        found
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::config::DEFAULT_FORMAT;

    fn part(id: &str) -> Token {
        Token::Part(id.to_string())
    }

    fn lit(text: &str) -> Token {
        Token::Literal(text.to_string())
    }

    #[test]
    fn test_round_trip_is_lossless() {
        let cases = [
            ("default", DEFAULT_FORMAT),
            ("empty", ""),
            ("single-newline", "\n"),
            ("trailing-newline", "$model\n"),
            ("blank-lines", "$model\n\n\n$quota"),
            ("crlf", "$model\r\n$quota\r\n"),
            ("separators", "$directory | $git_branch  $git_status"),
            ("suffix-text", "$context_x$modelY"),
            ("unknown", "$model $unknown $ $$ $5"),
            ("only-text", "plain text"),
            ("multibyte", "📁$directory ─ $model"),
        ];
        for (name, format) in cases {
            assert_eq!(Layout::parse(format).to_format(), format, "{name}");
        }
    }

    #[test]
    fn test_parse_structure() {
        let layout = Layout::parse("$directory $git_branch\n\n$context_x $unknown");
        assert_eq!(
            layout.lines,
            vec![
                vec![part("directory"), lit(" "), part("git_branch")],
                vec![],
                vec![part("context"), lit("_x $unknown")],
            ]
        );
    }

    #[test]
    fn test_unknown_variables() {
        let cases: [(&str, &[&str]); 6] = [
            ("$model $quota", &[]),
            ("$model $unknown", &["unknown"]),
            ("$foo$bar $foo", &["foo", "bar"]),
            ("$context_x", &[]),
            ("$ $5 $$", &[]),
            ("a $_b-c", &["_b"]),
        ];
        for (format, expected) in cases {
            assert_eq!(
                Layout::parse(format).unknown_variables(),
                expected,
                "{format}"
            );
        }
    }
}
