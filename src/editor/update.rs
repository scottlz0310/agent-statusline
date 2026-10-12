//! `toml_edit::DocumentMut` への部分更新。変更したキーだけを書き込み、
//! コメント・並び・未知のキー・触れていない項目はそのまま残す。

use std::fmt;

use toml_edit::{DocumentMut, Item, Table, Value};

use crate::editor::layout::Layout;
use crate::engine::config::Config;
use crate::engine::parts::{PropKind, find};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PropValue {
    Str(String),
    Int(i64),
    Bool(bool),
}

impl PropValue {
    fn from_item(item: &Item) -> Option<Self> {
        let value = item.as_value()?;
        match value {
            Value::String(s) => Some(Self::Str(s.value().clone())),
            Value::Integer(i) => Some(Self::Int(*i.value())),
            Value::Boolean(b) => Some(Self::Bool(*b.value())),
            _ => None,
        }
    }

    fn to_value(&self) -> Value {
        match self {
            Self::Str(s) => Value::from(s.as_str()),
            Self::Int(i) => Value::from(*i),
            Self::Bool(b) => Value::from(*b),
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum UpdateError {
    UnknownPart(String),
    UnknownKey {
        part: String,
        key: String,
    },
    KindMismatch {
        part: String,
        key: String,
        expected: &'static str,
    },
    OutOfRange {
        part: String,
        key: String,
        value: i64,
    },
    NotATable(String),
}

impl fmt::Display for UpdateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownPart(part) => write!(f, "未知の部品: {part}"),
            Self::UnknownKey { part, key } => write!(f, "未知のキー: {part}.{key}"),
            Self::KindMismatch {
                part,
                key,
                expected,
            } => {
                write!(f, "{part}.{key} の値の型が違います（{expected} が必要）")
            }
            Self::OutOfRange { part, key, value } => {
                write!(f, "{part}.{key} は 0〜100 の整数が必要です: {value}")
            }
            Self::NotATable(part) => write!(f, "[{part}] がテーブルではありません"),
        }
    }
}

impl std::error::Error for UpdateError {}

/// `Config::default()` の値。既定値と同じ値に戻すときの扱い（キーを書かない）の判定に使う
pub struct DefaultValues(DocumentMut);

impl DefaultValues {
    pub fn new() -> Self {
        let text = toml_edit::ser::to_string(&Config::default())
            .expect("Config::default() は TOML にできる");
        Self(text.parse().expect("直列化した TOML は解析できる"))
    }

    fn get(&self, part: &str, key: &str) -> Option<PropValue> {
        let table = self.0.get(part)?.as_table_like()?;
        PropValue::from_item(table.get(key)?)
    }
}

impl Default for DefaultValues {
    fn default() -> Self {
        Self::new()
    }
}

/// 部品のプロパティを 1 つ更新する。
///
/// - キーが既にあれば、値だけを置き換える（行末コメントなどは残す）
/// - キーがなければ、既定値と異なるときだけ追加する（テーブルがなければ作る）
pub fn set_prop(
    doc: &mut DocumentMut,
    defaults: &DefaultValues,
    part: &str,
    key: &str,
    value: &PropValue,
) -> Result<(), UpdateError> {
    check_value(part, key, value)?;
    let is_default = defaults.get(part, key).as_ref() == Some(value);

    let Some(item) = doc.get_mut(part) else {
        if !is_default {
            let mut table = Table::new();
            table.insert(key, Item::Value(value.to_value()));
            doc.insert(part, Item::Table(table));
        }
        return Ok(());
    };
    let table = item
        .as_table_like_mut()
        .ok_or_else(|| UpdateError::NotATable(part.to_string()))?;
    match table.get_mut(key) {
        Some(existing) => replace_value(existing, value),
        None if is_default => {}
        None => {
            table.insert(key, Item::Value(value.to_value()));
        }
    }
    Ok(())
}

fn check_value(part: &str, key: &str, value: &PropValue) -> Result<(), UpdateError> {
    let spec = find(part).ok_or_else(|| UpdateError::UnknownPart(part.to_string()))?;
    let prop = spec
        .props
        .iter()
        .find(|p| p.key == key)
        .ok_or_else(|| UpdateError::UnknownKey {
            part: part.to_string(),
            key: key.to_string(),
        })?;
    let mismatch = |expected| UpdateError::KindMismatch {
        part: part.to_string(),
        key: key.to_string(),
        expected,
    };
    match (&prop.kind, value) {
        (PropKind::Bool, PropValue::Bool(_)) => Ok(()),
        (PropKind::Bool, _) => Err(mismatch("真偽値")),
        (PropKind::Threshold, PropValue::Int(v)) if (0..=100).contains(v) => Ok(()),
        (PropKind::Threshold, PropValue::Int(v)) => Err(UpdateError::OutOfRange {
            part: part.to_string(),
            key: key.to_string(),
            value: *v,
        }),
        (PropKind::Threshold, _) => Err(mismatch("整数")),
        (_, PropValue::Str(_)) => Ok(()),
        (_, _) => Err(mismatch("文字列")),
    }
}

/// 既存の値を置き換える。同じ値なら何もしない（元の表記を保つ）。前後の空白・コメントは引き継ぐ
fn replace_value(existing: &mut Item, value: &PropValue) {
    if PropValue::from_item(existing).as_ref() == Some(value) {
        return;
    }
    let mut new = value.to_value();
    if let Some(old) = existing.as_value() {
        *new.decor_mut() = old.decor().clone();
    }
    *existing = Item::Value(new);
}

/// トップレベルの `format` を更新する。元の書き方（複数行か 1 行か、リテラル文字列か）を引き継ぐ
pub fn set_format(doc: &mut DocumentMut, layout: &Layout) {
    let format = layout.to_format();
    let existing = doc.get("format").and_then(Item::as_value);
    if existing.and_then(Value::as_str) == Some(format.as_str()) {
        return;
    }
    let style = existing
        .and_then(|v| v.as_str().and(raw_repr(v)))
        .map_or(StringStyle::MultiLineBasic, |raw| StringStyle::detect(&raw));

    let mut new =
        value_with_notation(&style.encode(&format)).unwrap_or_else(|| Value::from(format.as_str()));
    if let Some(old) = existing {
        *new.decor_mut() = old.decor().clone();
    }
    // 既存のキーは値だけを差し替える。`insert` はキーごと置き換え、直前のコメントを失う
    match doc.get_mut("format") {
        Some(item) => *item = Item::Value(new),
        None => {
            doc.insert("format", Item::Value(new));
        }
    }
}

/// TOML の表記（`"..."`、`'''...'''` など）をそのまま保った値を作る。表記が不正なら `None`
fn value_with_notation(raw: &str) -> Option<Value> {
    let doc: DocumentMut = format!("v = {raw}").parse().ok()?;
    doc.get("v")?.as_value().cloned()
}

fn raw_repr(value: &Value) -> Option<String> {
    match value {
        Value::String(s) => s.as_repr()?.as_raw().as_str().map(str::to_string),
        _ => None,
    }
}

/// TOML の文字列の書き方
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StringStyle {
    Basic,
    Literal,
    MultiLineBasic,
    MultiLineLiteral,
}

impl StringStyle {
    fn detect(raw: &str) -> Self {
        if raw.starts_with("\"\"\"") {
            Self::MultiLineBasic
        } else if raw.starts_with("'''") {
            Self::MultiLineLiteral
        } else if raw.starts_with('\'') {
            Self::Literal
        } else {
            Self::Basic
        }
    }

    /// `value` を表す表記。この書き方で表せない内容は、複数行の基本文字列で書く
    fn encode(self, value: &str) -> String {
        match self {
            Self::Basic => format!("\"{}\"", escape_basic(value, false)),
            Self::Literal if !value.contains(['\'', '\n', '\r']) => format!("'{value}'"),
            Self::MultiLineLiteral
                if !value.contains("'''") && !value.ends_with('\'') && !value.contains('\r') =>
            {
                format!("'''\n{value}'''")
            }
            _ => format!("\"\"\"\n{}\"\"\"", escape_basic(value, true)),
        }
    }
}

/// 基本文字列のエスケープ。複数行では改行をそのまま書く
fn escape_basic(value: &str, multiline: bool) -> String {
    let mut out = String::with_capacity(value.len());
    let chars: Vec<char> = value.chars().collect();
    for (i, &c) in chars.iter().enumerate() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => {
                // 複数行では、`"""` を作る連続や、閉じの `"""` と続く末尾の `"` をエスケープする
                let run_of_three = i + 2 < chars.len()
                    && chars[i + 1] == '"'
                    && chars[i + 2] == '"'
                    || i >= 1 && chars[i - 1] == '"' && i + 1 < chars.len() && chars[i + 1] == '"'
                    || i >= 2 && chars[i - 1] == '"' && chars[i - 2] == '"';
                let at_end = i + 1 == chars.len();
                if !multiline || run_of_three || at_end {
                    out.push_str("\\\"");
                } else {
                    out.push('"');
                }
            }
            '\n' if multiline => out.push('\n'),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 || c as u32 == 0x7f => {
                out.extend(format!("\\u{:04X}", c as u32).chars());
            }
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> DocumentMut {
        text.parse().unwrap()
    }

    fn s(v: &str) -> PropValue {
        PropValue::Str(v.to_string())
    }

    #[test]
    fn test_set_prop_replaces_value_keeping_comments_and_unknown_keys() {
        let original = "# 先頭のコメント\nformat = \"$model\" # 配置\nunknown = 1\n\n[model]\n# 記号\nsymbol = \"M \" # 行末\nstyle = \"cyan\"\nextra = true\n\n[other]\nkey = \"v\"\n";
        let mut doc = parse(original);
        set_prop(
            &mut doc,
            &DefaultValues::new(),
            "model",
            "symbol",
            &s("🤖 "),
        )
        .unwrap();
        assert_eq!(
            doc.to_string(),
            original.replace("symbol = \"M \" # 行末", "symbol = \"🤖 \" # 行末")
        );
    }

    #[test]
    fn test_set_prop_default_value_handling() {
        let defaults = DefaultValues::new();
        // (名前, 元の内容, 部品, キー, 値, 期待する内容)
        let cases = [
            // キーがなく、既定値のまま: 書かない（テーブルも作らない）
            ("absent-default", "", "model", "symbol", s("🤖 "), ""),
            (
                "absent-default-in-table",
                "[model]\nstyle = \"cyan\"\n",
                "model",
                "symbol",
                s("🤖 "),
                "[model]\nstyle = \"cyan\"\n",
            ),
            // キーがなく、既定値と異なる: 追加する（テーブルがなければ作る）
            (
                "absent-new-table",
                "",
                "model",
                "symbol",
                s("M "),
                "[model]\nsymbol = \"M \"\n",
            ),
            (
                "absent-new-key",
                "[model]\nstyle = \"cyan\"\n",
                "model",
                "symbol",
                s("M "),
                "[model]\nstyle = \"cyan\"\nsymbol = \"M \"\n",
            ),
            // キーがあり、既定値に戻す: その値に更新する（キーは消さない）
            (
                "existing-back-to-default",
                "[model]\nsymbol = \"M \"\n",
                "model",
                "symbol",
                s("🤖 "),
                "[model]\nsymbol = \"🤖 \"\n",
            ),
            // 同じ値: 表記をそのまま保つ
            (
                "same-value-keeps-notation",
                "[model]\nsymbol = 'M '\n",
                "model",
                "symbol",
                s("M "),
                "[model]\nsymbol = 'M '\n",
            ),
            // 数値・真偽値
            (
                "threshold",
                "[context]\nthreshold_warning = 40 # 注意\n",
                "context",
                "threshold_warning",
                PropValue::Int(60),
                "[context]\nthreshold_warning = 60 # 注意\n",
            ),
            (
                "bool-new",
                "",
                "model",
                "disabled",
                PropValue::Bool(true),
                "[model]\ndisabled = true\n",
            ),
            (
                "bool-default",
                "",
                "model",
                "disabled",
                PropValue::Bool(false),
                "",
            ),
            // インラインテーブル
            (
                "inline-table",
                "model = { symbol = \"M \" }\n",
                "model",
                "style",
                s("red"),
                // toml_edit は、元の最後の値の後ろの空白を、新しいキーの前に残す
                "model = { symbol = \"M \" , style = \"red\" }\n",
            ),
            // 末尾に改行がない
            (
                "no-trailing-newline",
                "format = \"$model\"",
                "model",
                "symbol",
                s("M "),
                "format = \"$model\"\n\n[model]\nsymbol = \"M \"\n",
            ),
        ];
        for (name, original, part, key, value, expected) in cases {
            let mut doc = parse(original);
            set_prop(&mut doc, &defaults, part, key, &value).unwrap();
            assert_eq!(doc.to_string(), expected, "{name}");
            // 結果は Config として読める
            toml_edit::de::from_str::<Config>(&doc.to_string()).unwrap();
        }
    }

    #[test]
    fn test_set_prop_errors() {
        let defaults = DefaultValues::new();
        let cases = [
            (
                "unknown-part",
                "",
                "nope",
                "symbol",
                s("x"),
                UpdateError::UnknownPart("nope".into()),
            ),
            (
                "unknown-key",
                "",
                "model",
                "symbl",
                s("x"),
                UpdateError::UnknownKey {
                    part: "model".into(),
                    key: "symbl".into(),
                },
            ),
            (
                "string-to-bool",
                "",
                "model",
                "disabled",
                s("true"),
                UpdateError::KindMismatch {
                    part: "model".into(),
                    key: "disabled".into(),
                    expected: "真偽値",
                },
            ),
            (
                "bool-to-string",
                "",
                "model",
                "symbol",
                PropValue::Bool(true),
                UpdateError::KindMismatch {
                    part: "model".into(),
                    key: "symbol".into(),
                    expected: "文字列",
                },
            ),
            (
                "threshold-type",
                "",
                "context",
                "threshold_warning",
                s("50"),
                UpdateError::KindMismatch {
                    part: "context".into(),
                    key: "threshold_warning".into(),
                    expected: "整数",
                },
            ),
            (
                "threshold-range",
                "",
                "context",
                "threshold_warning",
                PropValue::Int(101),
                UpdateError::OutOfRange {
                    part: "context".into(),
                    key: "threshold_warning".into(),
                    value: 101,
                },
            ),
            (
                "not-a-table",
                "model = 3\n",
                "model",
                "symbol",
                s("x"),
                UpdateError::NotATable("model".into()),
            ),
        ];
        for (name, original, part, key, value, expected) in cases {
            let mut doc = parse(original);
            let err = set_prop(&mut doc, &defaults, part, key, &value).unwrap_err();
            assert_eq!(err, expected, "{name}");
            assert_eq!(doc.to_string(), original, "{name}: 失敗時は変更しない");
        }
    }

    const DEFAULT_LAYOUT: &str =
        "$directory$git_branch$git_status$sandbox$review\n$model$context$agent_state$plan\n$quota";

    #[test]
    fn test_set_format_keeps_notation() {
        let layout = Layout::parse("$model $plan\n$quota\n");
        let cases = [
            (
                "absent",
                "",
                "format = \"\"\"\n$model $plan\n$quota\n\"\"\"\n",
            ),
            (
                "multiline-basic",
                "format = \"\"\"\n$directory\n$model\n\"\"\" # 配置\n[model]\nsymbol = \"M \"\n",
                "format = \"\"\"\n$model $plan\n$quota\n\"\"\" # 配置\n[model]\nsymbol = \"M \"\n",
            ),
            (
                "single-line-basic",
                "format = \"$directory\" # 配置\n",
                "format = \"$model $plan\\n$quota\\n\" # 配置\n",
            ),
            (
                "multiline-literal",
                "format = '''\n$directory\n'''\n",
                "format = '''\n$model $plan\n$quota\n'''\n",
            ),
            // 改行を含む内容は、1 行のリテラル文字列では書けないので複数行の基本文字列にする
            (
                "literal-cannot-hold-newline",
                "format = '$directory'\n",
                "format = \"\"\"\n$model $plan\n$quota\n\"\"\"\n",
            ),
        ];
        for (name, original, expected) in cases {
            let mut doc = parse(original);
            set_format(&mut doc, &layout);
            assert_eq!(doc.to_string(), expected, "{name}");
            let config: Config = toml_edit::de::from_str(&doc.to_string()).unwrap();
            assert_eq!(config.format, layout.to_format(), "{name}: 往復");
        }
    }

    #[test]
    fn test_set_format_round_trips_awkward_strings() {
        let styles = [
            "",
            "format = \"x\"\n",
            "format = 'x'\n",
            "format = \"\"\"\nx\"\"\"\n",
            "format = '''\nx'''\n",
        ];
        let formats = [
            "",
            "\n",
            "$model",
            "a\\b \"quoted\" ' single",
            "\"\"\" triple \"\"\"",
            "ends with quote\"",
            "ends with quotes\"\"",
            "crlf\r\nline\r\n",
            "tab\there\u{1}ctrl",
            "$model\n\n$quota\n",
            "'''",
            "ends'",
            "日本語 📁 $directory",
        ];
        for original in styles {
            for format in formats {
                let mut doc = parse(original);
                set_format(&mut doc, &Layout::parse(format));
                let reparsed = parse(&doc.to_string());
                assert_eq!(
                    reparsed["format"].as_str(),
                    Some(format),
                    "{original:?} -> {format:?} -> {doc}"
                );
            }
        }
    }

    #[test]
    fn test_set_format_noop_when_unchanged() {
        let original = format!(
            "# c\nformat = \"{}\"\n",
            DEFAULT_LAYOUT.replace('\n', "\\n")
        );
        let mut doc = parse(&original);
        set_format(&mut doc, &Layout::parse(DEFAULT_LAYOUT));
        assert_eq!(doc.to_string(), original);
    }

    #[test]
    fn test_edit_then_validate_end_to_end() {
        use crate::editor::validate::validate;

        let original = "# 私の設定\nformat = \"\"\"\n$directory $model\n$quota\n\"\"\"\nmy_note = \"残す\"\n\n[model]\n# モデルの見た目\nsymbol = \"M \" # 記号\n";
        let mut doc = parse(original);
        let defaults = DefaultValues::new();
        set_prop(&mut doc, &defaults, "model", "style", &s("bold green")).unwrap();
        set_prop(
            &mut doc,
            &defaults,
            "quota",
            "threshold_warning",
            &PropValue::Int(30),
        )
        .unwrap();
        set_format(&mut doc, &Layout::parse("$model\n$directory\n$quota\n"));

        let text = doc.to_string();
        assert!(text.contains("# 私の設定"), "{text}");
        assert!(text.contains("my_note = \"残す\""), "{text}");
        assert!(text.contains("symbol = \"M \" # 記号"), "{text}");
        assert!(text.contains("# モデルの見た目"), "{text}");

        let report = validate(&text);
        let config = report.config.as_ref().expect("Config として読める");
        assert_eq!(config.format, "$model\n$directory\n$quota\n");
        assert_eq!(config.model.style, "bold green");
        assert_eq!(config.quota.threshold_warning, 30);
        // 未知のキーは警告にとどめる
        assert!(!report.has_errors(), "{:?}", report.issues);
        assert!(
            report.issues.iter().any(|i| i.path == "my_note"),
            "{:?}",
            report.issues
        );
    }
}
