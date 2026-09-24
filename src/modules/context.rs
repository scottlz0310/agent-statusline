/// トークン数を k 単位で短縮表示 (例: 12500 -> "12k")
pub fn format_tokens(tokens: u64) -> String {
    if tokens >= 1000 {
        format!("{}k", tokens / 1000)
    } else {
        tokens.to_string()
    }
}
