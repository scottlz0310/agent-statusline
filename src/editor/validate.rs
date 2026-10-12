//! 保存前の検証。エラーは保存を止め、警告は表示するだけにする。
//!
//! エラー: 解析できない、`Config` として読めない、閾値の範囲、スタイル名が認識できない。
//! 警告: 認識できないキー・テーブル（綴りの誤りを含む）、`format` の未知の変数と括弧の不整合。
//! 警告の対象は書き換えない（CLI は未認識のキーを無視するため、動作は壊れない）。

use toml_edit::{DocumentMut, Item, TableLike};

use crate::editor::layout::Layout;
use crate::engine::config::Config;
use crate::engine::parts::{PARTS, PropKind, PropSpec};
use crate::engine::style::unknown_style_tokens;
use crate::engine::template::unknown_variables;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Issue {
    pub severity: Severity,
    /// 対象のキー（例: `context.threshold_warning`）。文書全体のときは空
    pub path: String,
    pub message: String,
}

#[derive(Debug)]
pub struct Report {
    /// `Config` として読めたときだけ `Some`
    pub config: Option<Config>,
    pub issues: Vec<Issue>,
}

impl Report {
    pub fn has_errors(&self) -> bool {
        self.issues.iter().any(|i| i.severity == Severity::Error)
    }
}

const INTEGRATIONS_KEYS: [&str; 1] = ["squirrel_notifier"];
const SQUIRREL_NOTIFIER_KEYS: [&str; 3] = ["enabled", "output_dir", "summary_path"];

pub fn validate(text: &str) -> Report {
    let mut issues = Vec::new();
    let doc: DocumentMut = match text.parse() {
        Ok(doc) => doc,
        Err(err) => {
            let message = format!("TOML の構文エラー: {}", err.message());
            issues.push(error("", message));
            return Report {
                config: None,
                issues,
            };
        }
    };

    let config = match toml_edit::de::from_str::<Config>(text) {
        Ok(config) => {
            check_thresholds(&config, &mut issues);
            Some(config)
        }
        Err(err) => {
            issues.push(error(
                "",
                format!("設定として読めません: {}", err.message()),
            ));
            None
        }
    };

    check_document(&doc, &mut issues);
    Report { config, issues }
}

fn error(path: &str, message: String) -> Issue {
    Issue {
        severity: Severity::Error,
        path: path.to_string(),
        message,
    }
}

fn warning(path: &str, message: String) -> Issue {
    Issue {
        severity: Severity::Warning,
        path: path.to_string(),
        message,
    }
}

fn check_thresholds(config: &Config, issues: &mut Vec<Issue>) {
    let parts = [
        (
            "context",
            config.context.threshold_warning,
            config.context.threshold_critical,
        ),
        (
            "quota",
            config.quota.threshold_warning,
            config.quota.threshold_critical,
        ),
    ];
    for (part, warn, critical) in parts {
        for (key, value) in [
            ("threshold_warning", warn),
            ("threshold_critical", critical),
        ] {
            if value > 100 {
                let path = format!("{part}.{key}");
                issues.push(error(
                    &path,
                    format!("0〜100 の範囲で指定してください: {value}"),
                ));
            }
        }
        if warn >= critical {
            let path = format!("{part}.threshold_warning");
            let message = format!(
                "threshold_warning（{warn}）は threshold_critical（{critical}）より小さくしてください"
            );
            issues.push(error(&path, message));
        }
    }
}

fn check_document(doc: &DocumentMut, issues: &mut Vec<Issue>) {
    for (key, item) in doc.iter() {
        match key {
            "format" => {
                if let Some(format) = item.as_str() {
                    for name in Layout::parse(format).unknown_variables() {
                        let message = format!("未知の変数 ${name} はそのまま出力されます");
                        issues.push(warning("format", message));
                    }
                }
            }
            "integrations" => check_integrations(item, issues),
            _ => {
                let Some(spec) = PARTS.iter().find(|part| part.id == key) else {
                    issues.push(warning(
                        key,
                        "認識できないキーまたはテーブルです".to_string(),
                    ));
                    continue;
                };
                if let Some(table) = item.as_table_like() {
                    check_part_table(spec.id, spec.props, table, issues);
                }
            }
        }
    }
}

fn check_integrations(item: &Item, issues: &mut Vec<Issue>) {
    let Some(table) = item.as_table_like() else {
        return;
    };
    for (key, item) in table.iter() {
        let path = format!("integrations.{key}");
        if !INTEGRATIONS_KEYS.contains(&key) {
            issues.push(warning(&path, "認識できないキーです".to_string()));
            continue;
        }
        let Some(inner) = item.as_table_like() else {
            continue;
        };
        for (inner_key, _) in inner.iter() {
            if !SQUIRREL_NOTIFIER_KEYS.contains(&inner_key) {
                let path = format!("{path}.{inner_key}");
                issues.push(warning(&path, "認識できないキーです".to_string()));
            }
        }
    }
}

fn check_part_table(id: &str, props: &[PropSpec], table: &dyn TableLike, issues: &mut Vec<Issue>) {
    for (key, item) in table.iter() {
        let path = format!("{id}.{key}");
        let Some(prop) = props.iter().find(|p| p.key == key) else {
            issues.push(warning(
                &path,
                "認識できないキーです（綴りの誤りの可能性）".to_string(),
            ));
            continue;
        };
        let Some(value) = item.as_str() else {
            continue;
        };
        match prop.kind {
            PropKind::Style => {
                let unknown = unknown_style_tokens(value);
                if !unknown.is_empty() {
                    let message = format!("認識できないスタイル名: {}", unknown.join(" "));
                    issues.push(error(&path, message));
                }
            }
            PropKind::Format(placeholders) => {
                for name in unknown_variables(value, placeholders) {
                    let message = format!("未知の変数 ${name} はそのまま出力されます");
                    issues.push(warning(&path, message));
                }
                for message in bracket_issues(value) {
                    issues.push(warning(&path, message));
                }
            }
            _ => {}
        }
    }
}

/// `[text](style)` の括弧の不整合と、インラインのスタイル名の誤り。描画（`render_styled_text`）と同じ規則で読む。
/// スタイルに `$変数` を含むもの（`($style)` など）は、展開前なので名前を検査しない
fn bracket_issues(template: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut pos = 0;
    let mut chars = template.char_indices();
    while let Some((i, c)) = chars.next() {
        if i < pos {
            continue;
        }
        match c {
            '\\' => {
                // `\[` `\]` `\\` は括弧ではない
                if let Some((j, next)) = chars.clone().next()
                    && matches!(next, '[' | ']' | '\\')
                {
                    chars.next();
                    pos = j + 1;
                }
            }
            '[' => {
                let rest = &template[i + 1..];
                let Some(close) = rest.find("](") else {
                    found.push("`[` に対応する `](スタイル)` がありません".to_string());
                    continue;
                };
                let spec_start = i + 1 + close + 2;
                let Some(end) = template[spec_start..].find(')') else {
                    found.push("`](` の後ろの `)` が閉じられていません".to_string());
                    continue;
                };
                let spec = &template[spec_start..spec_start + end];
                if !spec.contains('$') {
                    let unknown = unknown_style_tokens(spec);
                    if !unknown.is_empty() {
                        found.push(format!("認識できないスタイル名: {}", unknown.join(" ")));
                    }
                }
                pos = spec_start + end + 1;
            }
            _ => {}
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    fn messages(report: &Report, severity: Severity) -> Vec<(String, String)> {
        report
            .issues
            .iter()
            .filter(|i| i.severity == severity)
            .map(|i| (i.path.clone(), i.message.clone()))
            .collect()
    }

    #[test]
    fn test_default_config_has_no_issues() {
        let text = toml_edit::ser::to_string(&Config::default()).unwrap();
        for (name, text) in [
            ("serialized-default", text.as_str()),
            ("empty", ""),
            ("comment-only", "# nothing\n"),
        ] {
            let report = validate(text);
            assert!(report.issues.is_empty(), "{name}: {:?}", report.issues);
            assert!(report.config.is_some(), "{name}");
        }
    }

    #[test]
    fn test_validate_errors() {
        // (名前, 内容, 期待するエラーの (path, メッセージの一部))
        let cases = [
            ("syntax", "format = [[[\n", "", "TOML の構文エラー"),
            (
                "type",
                "[context]\nthreshold_warning = \"high\"\n",
                "",
                "設定として読めません",
            ),
            (
                "warning-ge-critical",
                "[context]\nthreshold_warning = 80\nthreshold_critical = 80\n",
                "context.threshold_warning",
                "より小さく",
            ),
            (
                "quota-inverted",
                "[quota]\nthreshold_warning = 90\nthreshold_critical = 10\n",
                "quota.threshold_warning",
                "より小さく",
            ),
            (
                "over-100",
                "[context]\nthreshold_warning = 50\nthreshold_critical = 150\n",
                "context.threshold_critical",
                "0〜100",
            ),
            (
                "unknown-style",
                "[model]\nstyle = \"bold cyna\"\n",
                "model.style",
                "cyna",
            ),
            (
                "unknown-style-in-set",
                "[context]\nstyle_warning = \"yelow\"\n",
                "context.style_warning",
                "yelow",
            ),
        ];
        for (name, text, path, fragment) in cases {
            let report = validate(text);
            assert!(report.has_errors(), "{name}");
            let errors = messages(&report, Severity::Error);
            assert!(
                errors
                    .iter()
                    .any(|(p, m)| p == path && m.contains(fragment)),
                "{name}: {errors:?}"
            );
        }
    }

    #[test]
    fn test_validate_warnings_do_not_block() {
        // (名前, 内容, 期待する警告の (path, メッセージの一部))
        let cases = [
            (
                "unknown-top-level-key",
                "frmat = \"$model\"\n",
                "frmat",
                "認識できない",
            ),
            (
                "unknown-table",
                "[modle]\nsymbol = \"x\"\n",
                "modle",
                "認識できない",
            ),
            (
                "typo-key",
                "[model]\nsymbl = \"x\"\n",
                "model.symbl",
                "綴りの誤り",
            ),
            (
                "unknown-integration-key",
                "[integrations.squirrel_notifier]\nenable = true\n",
                "integrations.squirrel_notifier.enable",
                "認識できない",
            ),
            (
                "unknown-integration",
                "[integrations.other]\nx = 1\n",
                "integrations.other",
                "認識できない",
            ),
            (
                "unknown-layout-variable",
                "format = \"$model $modle\"\n",
                "format",
                "$modle",
            ),
            (
                "unknown-format-variable",
                "[model]\nformat = \"[$symbol$name]($style)\"\n",
                "model.format",
                "$name",
            ),
            (
                "unclosed-bracket",
                "[model]\nformat = \"[$symbol$model $style\"\n",
                "model.format",
                "対応する",
            ),
            (
                "unclosed-paren",
                "[model]\nformat = \"[$model]($style\"\n",
                "model.format",
                "閉じられていません",
            ),
            (
                "inline-style-typo",
                "[model]\nformat = \"[$model](bold cyna)\"\n",
                "model.format",
                "cyna",
            ),
        ];
        for (name, text, path, fragment) in cases {
            let report = validate(text);
            assert!(!report.has_errors(), "{name}: {:?}", report.issues);
            assert!(report.config.is_some(), "{name}");
            let warnings = messages(&report, Severity::Warning);
            assert!(
                warnings
                    .iter()
                    .any(|(p, m)| p == path && m.contains(fragment)),
                "{name}: {warnings:?}"
            );
        }
    }

    #[test]
    fn test_bracket_issues() {
        let cases: [(&str, usize); 8] = [
            ("[$symbol$branch]($style) ", 0),
            ("[($tokens tok)]($tok_style) ", 0),
            ("[\\[$path\\]]($path_style) ", 0),
            ("plain text", 0),
            ("[text](bold) [more](red)", 0),
            ("[a](bold cyan) [b", 1),
            ("[a](bold", 1),
            ("\\[not a bracket", 0),
        ];
        for (template, count) in cases {
            assert_eq!(bracket_issues(template).len(), count, "{template}");
        }
    }
}
