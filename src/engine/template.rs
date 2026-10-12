//! `$name` 形式のテンプレート変数の字句解析と展開。
//!
//! `$` の直後は、その文脈で既知の変数名のうち最長一致するものとして読み、直後の文字列は固定テキストとして保つ。
//! 既知の変数名に一致しない `$xxx` は、そのまま出力する。展開は 1 パスで行い、展開結果は再び解釈しない。

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Token<'a> {
    Text(&'a str),
    /// `$` を除いた既知の変数名
    Var(&'a str),
}

/// `template` を固定テキストと既知の変数に分解する。トークンを連結すると（`$` を戻せば）元の文字列になる
pub fn tokenize<'a>(template: &'a str, names: &[&str]) -> Vec<Token<'a>> {
    let mut tokens = Vec::new();
    let mut text_start = 0;
    let mut pos = 0;
    while let Some(offset) = template[pos..].find('$') {
        let dollar = pos + offset;
        let after = dollar + 1;
        let longest = names
            .iter()
            .filter(|name| template[after..].starts_with(**name))
            .map(|name| name.len())
            .max();
        match longest {
            Some(len) if len > 0 => {
                if text_start < dollar {
                    tokens.push(Token::Text(&template[text_start..dollar]));
                }
                tokens.push(Token::Var(&template[after..after + len]));
                pos = after + len;
                text_start = pos;
            }
            _ => pos = after,
        }
    }
    if text_start < template.len() {
        tokens.push(Token::Text(&template[text_start..]));
    }
    tokens
}

/// 既知の変数だけを 1 パスで値に置き換える
pub fn expand(template: &str, vars: &[(&str, &str)]) -> String {
    let names: Vec<&str> = vars.iter().map(|(name, _)| *name).collect();
    let mut out = String::with_capacity(template.len());
    for token in tokenize(template, &names) {
        match token {
            Token::Text(text) => out.push_str(text),
            Token::Var(name) => {
                if let Some((_, value)) = vars.iter().find(|(n, _)| *n == name) {
                    out.push_str(value);
                }
            }
        }
    }
    out
}

/// テキストに残っている `$名前`（識別子として読める範囲）を、重複なしで `found` に追加する
pub fn collect_dollar_names(text: &str, found: &mut Vec<String>) {
    for segment in text.split('$').skip(1) {
        let name: String = segment
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
            .collect();
        if name.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_') && !found.contains(&name)
        {
            found.push(name);
        }
    }
}

/// `names` に含まれない `$名前`。展開では置換されず、そのまま出力される
pub fn unknown_variables(template: &str, names: &[&str]) -> Vec<String> {
    let mut found = Vec::new();
    for token in tokenize(template, names) {
        if let Token::Text(text) = token {
            collect_dollar_names(text, &mut found);
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    type Case<'a> = (&'a str, &'a [(&'a str, &'a str)], &'a str, &'a str);

    #[test]
    fn test_expand() {
        let folder_vars = [
            ("folder_style", "bold"),
            ("folder", "proj"),
            ("path", "/a/b"),
        ];
        let context_vars = [("symbol", "🧠 "), ("tokens", "62k"), ("model", "M")];
        let cases: [Case; 11] = [
            ("plain", &folder_vars, "no variables", "no variables"),
            (
                "longest-match",
                &folder_vars,
                "$folder_style/$folder",
                "bold/proj",
            ),
            ("suffix-is-text", &context_vars, "$symbolCtx", "🧠 Ctx"),
            ("suffix-digit", &context_vars, "$tokensX$model1", "62kXM1"),
            ("suffix-underscore", &folder_vars, "$folder_styleY", "boldY"),
            ("unknown-kept", &context_vars, "$unknown $", "$unknown $"),
            ("trailing-dollar", &context_vars, "cost: $", "cost: $"),
            ("double-dollar", &context_vars, "$$model", "$M"),
            (
                "no-reexpansion",
                &[("model", "$symbol"), ("symbol", "S")],
                "$model",
                "$symbol",
            ),
            (
                "no-reexpansion-2",
                &[("branch", "x$model"), ("model", "M")],
                "$branch/$model",
                "x$model/M",
            ),
            ("adjacent", &context_vars, "$model$model$tokens", "MM62k"),
        ];
        for (name, vars, template, expected) in cases {
            assert_eq!(expand(template, vars), expected, "{name}");
        }
    }

    #[test]
    fn test_tokenize_round_trips() {
        let names = ["directory", "git_branch", "git_status", "context"];
        let cases = [
            "",
            "$directory$git_branch",
            "$directory $git_status\n\n$context_x $unknown",
            "text only",
            "$$directory$",
            "$git_branchX$git_statu",
        ];
        for template in cases {
            let rebuilt: String = tokenize(template, &names)
                .into_iter()
                .map(|token| match token {
                    Token::Text(text) => text.to_string(),
                    Token::Var(name) => format!("${name}"),
                })
                .collect();
            assert_eq!(rebuilt, template, "{template:?}");
        }
    }

    #[test]
    fn test_tokenize_tokens() {
        let names = ["context", "model"];
        let tokens = tokenize("a$model$context_x$other", &names);
        assert_eq!(
            tokens,
            [
                Token::Text("a"),
                Token::Var("model"),
                Token::Var("context"),
                Token::Text("_x$other"),
            ]
        );
    }
}
