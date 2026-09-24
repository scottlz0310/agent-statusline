/// トークン数を k 単位で短縮表示 (例: 12500 -> "12k")
pub fn format_tokens(tokens: u64) -> String {
    if tokens >= 1000 {
        format!("{}k", tokens / 1000)
    } else {
        tokens.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_tokens() {
        let cases = [
            (0, "0"),
            (500, "500"),
            (999, "999"),
            (1000, "1k"),
            (12_500, "12k"),
            (88_396, "88k"),
            (1_048_576, "1048k"),
        ];

        for (tokens, expected) in cases {
            assert_eq!(
                format_tokens(tokens),
                expected,
                "failed for tokens: {tokens}"
            );
        }
    }
}
