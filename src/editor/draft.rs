//! 編集中の設定（下書き）。TUI が操作する単位で、保存するまでファイルには書かない。
//!
//! 元の TOML 文書（コメント・未知のキーを保つ）、そこから導いた設定、配置の編集状態、
//! 検証の結果を持つ。配置とプロパティの変更は、時系列で 1 本の履歴に積み、`undo` で操作の逆順に戻す。
//! プロパティの変更は、**新しいエラーを増やす値**を受け付けず、下書きを変えない。
//! 読み込んだ設定が意味的なエラー（閾値の大小、認識できないスタイル名など。`Config` としては読める）を
//! 持っているときも、エラーのキーを 1 つずつ直せる。エラーが残る間は、下書きは保存に適さない（`has_errors`）。

use std::fmt;

use toml_edit::DocumentMut;

use crate::editor::layout::Layout;
use crate::editor::layout_edit::LayoutEditor;
use crate::editor::update::{DefaultValues, PropValue, UpdateError, set_format, set_prop};
use crate::editor::validate::{Issue, Severity, validate};
use crate::engine::config::Config;

#[derive(Debug)]
pub enum PropError {
    Update(UpdateError),
    /// 新しいエラーになった。最初の新しいエラーを持つ
    Rejected(Issue),
}

impl fmt::Display for PropError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Update(err) => write!(f, "{err}"),
            Self::Rejected(issue) if issue.path.is_empty() => write!(f, "{}", issue.message),
            Self::Rejected(issue) => write!(f, "{}: {}", issue.path, issue.message),
        }
    }
}

impl std::error::Error for PropError {}

/// 元に戻す履歴の 1 件
#[derive(Debug)]
enum Edit {
    /// 配置の変更。戻す内容は `LayoutEditor` が持つ
    Layout,
    /// プロパティの変更。変更前の TOML
    Prop(String),
}

/// 検証を通った変更の結果
struct Candidate {
    document: DocumentMut,
    text: String,
    config: Config,
}

#[derive(Debug)]
pub struct Draft {
    document: DocumentMut,
    defaults: DefaultValues,
    config: Config,
    /// `config` を TOML にしたもの。プロパティの現在の値を引く
    effective: DocumentMut,
    /// 文書の検証の結果（`format` の警告は、配置から出すので除く）
    document_issues: Vec<Issue>,
    layout: LayoutEditor,
    history: Vec<Edit>,
}

impl Draft {
    pub fn new(config: Config, document: DocumentMut) -> Self {
        let layout = LayoutEditor::new(Layout::parse(&config.format));
        let mut draft = Self {
            effective: effective_document(&config),
            document_issues: Vec::new(),
            document,
            defaults: DefaultValues::new(),
            config,
            layout,
            history: Vec::new(),
        };
        draft.document_issues = document_issues(&draft.document.to_string());
        draft
    }

    /// 文書から導いた設定。`format` は、配置の編集に追従しない（描画には `layout().to_format()` を使う）
    pub fn config(&self) -> &Config {
        &self.config
    }

    pub fn layout(&self) -> &LayoutEditor {
        &self.layout
    }

    /// テスト用: `format` だけを指定した下書き（ほかは既定）
    #[cfg(test)]
    pub fn with_format(format: &str) -> Self {
        let config = Config {
            format: format.to_string(),
            ..Config::default()
        };
        Self::new(config, DocumentMut::new())
    }

    /// 配置を編集する。履歴に積まれた変更だけを、時系列の履歴へ記録する
    pub fn edit_layout(&mut self, edit: impl FnOnce(&mut LayoutEditor)) {
        let before = self.layout.undo_len();
        edit(&mut self.layout);
        for _ in before..self.layout.undo_len() {
            self.history.push(Edit::Layout);
        }
    }

    /// プロパティを 1 つ更新する。**新しいエラー**（変更前にエラーのなかったキーのエラー）になる値は
    /// 受け付けず、下書きを変えない。変更前から残っているエラーは、変更の妨げにならない
    /// （複数のエラーがあっても、1 つずつ直せる。エラーのキーを別の誤った値にする変更も受け付ける）。
    /// 警告は受け付ける。値が変わらないとき（既定値と同じでキーがない場合を含む）は、履歴に積まない
    pub fn set_prop(&mut self, part: &str, key: &str, value: &PropValue) -> Result<(), PropError> {
        let Some(candidate) = self.evaluate(part, key, value)? else {
            return Ok(());
        };
        self.history.push(Edit::Prop(self.document.to_string()));
        self.effective = effective_document(&candidate.config);
        self.config = candidate.config;
        self.document_issues = document_issues(&candidate.text);
        self.document = candidate.document;
        Ok(())
    }

    /// `set_prop` した結果の設定。確定せず、下書きは変えない（入力中のプレビュー用）。
    /// 値が変わらないときは、現在の設定を返す
    pub fn preview_prop(
        &self,
        part: &str,
        key: &str,
        value: &PropValue,
    ) -> Result<Config, PropError> {
        Ok(self
            .evaluate(part, key, value)?
            .map_or_else(|| self.config.clone(), |candidate| candidate.config))
    }

    /// 変更を、候補の文書へ当てて検証する。変わらなければ `None`。新しいエラーなら `Err`
    fn evaluate(
        &self,
        part: &str,
        key: &str,
        value: &PropValue,
    ) -> Result<Option<Candidate>, PropError> {
        let mut document = self.document.clone();
        set_prop(&mut document, &self.defaults, part, key, value).map_err(PropError::Update)?;
        let text = document.to_string();
        if text == self.document.to_string() {
            return Ok(None);
        }

        let report = validate(&text);
        let existing: Vec<&str> = self
            .document_issues
            .iter()
            .filter(|issue| issue.severity == Severity::Error)
            .map(|issue| issue.path.as_str())
            .collect();
        if let Some(issue) = report.issues.iter().find(|issue| {
            issue.severity == Severity::Error && !existing.contains(&issue.path.as_str())
        }) {
            return Err(PropError::Rejected(issue.clone()));
        }
        // 設定として読めなくなる変更は、文書全体のエラー（対象のキーが空）で、既存のエラーにはならない
        let Some(config) = report.config else {
            unreachable!("新しいエラーがなければ、設定として読めている")
        };
        Ok(Some(Candidate {
            document,
            text,
            config,
        }))
    }

    /// プロパティの現在の値。部品かキーが未知なら `None`
    pub fn prop_value(&self, part: &str, key: &str) -> Option<PropValue> {
        let table = self.effective.get(part)?.as_table_like()?;
        PropValue::from_item(table.get(key)?)
    }

    pub fn default_value(&self, part: &str, key: &str) -> Option<PropValue> {
        self.defaults.get(part, key)
    }

    /// 現在の検証の結果。文書の警告に、配置の未知の変数の警告を足す
    pub fn issues(&self) -> Vec<Issue> {
        let mut issues = self.document_issues.clone();
        for name in self.layout.layout().unknown_variables() {
            issues.push(Issue {
                severity: Severity::Warning,
                path: "format".to_string(),
                message: format!("未知の変数 ${name} はそのまま出力されます"),
            });
        }
        issues
    }

    /// エラーが残っているか。残っている間は、保存に適さない
    pub fn has_errors(&self) -> bool {
        self.document_issues
            .iter()
            .any(|issue| issue.severity == Severity::Error)
    }

    pub fn can_undo(&self) -> bool {
        !self.history.is_empty()
    }

    /// 直近の変更（配置・プロパティを問わず）を元に戻す
    pub fn undo(&mut self) {
        match self.history.pop() {
            Some(Edit::Layout) => self.layout.undo(),
            Some(Edit::Prop(text)) => {
                let document: DocumentMut = text.parse().expect("履歴の TOML は解析できる");
                let report = validate(&text);
                let config = report
                    .config
                    .expect("履歴の TOML は、受け付けた時点で設定として読めている");
                self.effective = effective_document(&config);
                self.config = config;
                self.document = document;
                self.document_issues = document_issues(&text);
            }
            None => {}
        }
    }

    /// 保存する TOML。配置を `format` として書き込む（元の書き方を引き継ぐ）
    pub fn to_toml(&self) -> String {
        let mut document = self.document.clone();
        // 文書の `format`（なければ既定）から変えていないときは、書かない（キーのない文書にキーを足さない）
        if self.layout.layout().to_format() != self.config.format {
            set_format(&mut document, self.layout.layout());
        }
        document.to_string()
    }
}

fn effective_document(config: &Config) -> DocumentMut {
    toml_edit::ser::to_string(config)
        .expect("Config は TOML にできる")
        .parse()
        .expect("直列化した TOML は解析できる")
}

/// 文書の検証の警告。`format` は配置から出す（`Draft::issues`）ので除く
fn document_issues(text: &str) -> Vec<Issue> {
    validate(text)
        .issues
        .into_iter()
        .filter(|issue| issue.path != "format")
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::layout::Token;
    use crate::editor::layout_edit::{Direction, Placement};

    fn draft(text: &str) -> Draft {
        let document: DocumentMut = text.parse().unwrap();
        let config = toml_edit::de::from_str::<Config>(text).unwrap();
        Draft::new(config, document)
    }

    fn int(value: i64) -> PropValue {
        PropValue::Int(value)
    }

    fn string(value: &str) -> PropValue {
        PropValue::Str(value.to_string())
    }

    #[test]
    fn test_set_prop_updates_config_and_preserves_the_document() {
        let text = "# 自分の設定\nunknown_key = 1\n\n[context]\nthreshold_warning = 60 # 警告\n";
        let mut draft = draft(text);
        assert_eq!(draft.config().context.threshold_warning, 60);

        draft
            .set_prop("context", "threshold_warning", &int(70))
            .unwrap();
        assert_eq!(draft.config().context.threshold_warning, 70);
        assert_eq!(
            draft.prop_value("context", "threshold_warning"),
            Some(int(70))
        );
        // コメント・未知のキー・行末のコメントは残る
        let toml = draft.to_toml();
        assert!(toml.contains("# 自分の設定"), "{toml}");
        assert!(toml.contains("unknown_key = 1"), "{toml}");
        assert!(toml.contains("threshold_warning = 70 # 警告"), "{toml}");
    }

    #[test]
    fn test_set_prop_rejects_errors_and_leaves_the_draft_unchanged() {
        // (名前, 部品, キー, 値)
        let cases = [
            // 閾値の大小（既定: warning 70 < critical 90 のとき、warning を 95 にする）
            (
                "warning-above-critical",
                "context",
                "threshold_warning",
                int(95),
            ),
            ("out-of-range", "context", "threshold_critical", int(101)),
            (
                "unknown-style",
                "git_branch",
                "style",
                string("bold nonexistent"),
            ),
            (
                "invalid-choice",
                "directory",
                "truncation_mode",
                string("right"),
            ),
            ("kind-mismatch", "git_branch", "disabled", string("yes")),
            ("unknown-key", "git_branch", "symbl", string("x")),
            ("unknown-part", "nothing", "style", string("bold")),
        ];
        for (name, part, key, value) in cases {
            let mut draft = draft("");
            let before = draft.to_toml();
            assert!(draft.set_prop(part, key, &value).is_err(), "{name}");
            assert_eq!(draft.to_toml(), before, "{name}: 文書が変わらない");
            assert!(!draft.can_undo(), "{name}: 履歴に積まない");
            assert_eq!(draft.config().format, Config::default().format, "{name}");
        }
    }

    #[test]
    fn test_existing_errors_can_be_fixed_one_by_one() {
        // どちらも `Config` としては読めるが、スタイル名が誤っている
        let mut draft = draft(
            "[model]
style = \"cyna\"

[git_branch]
style = \"gren\"
",
        );
        assert!(draft.has_errors());
        let error_paths = |draft: &Draft| -> Vec<String> {
            draft
                .issues()
                .into_iter()
                .filter(|issue| issue.severity == Severity::Error)
                .map(|issue| issue.path)
                .collect()
        };
        assert_eq!(error_paths(&draft), ["model.style", "git_branch.style"]);

        // 片方を直すと、もう片方のエラーが残っていても受け付ける
        draft.set_prop("model", "style", &string("cyan")).unwrap();
        assert_eq!(error_paths(&draft), ["git_branch.style"]);
        assert!(draft.has_errors(), "エラーが残る間は、保存に適さない");

        // 無関係なプロパティの変更も、既存のエラーに妨げられない
        draft
            .set_prop("git_branch", "symbol", &string("B"))
            .unwrap();
        // 直した箇所を、再び誤った値にする変更は、新しいエラーなので受け付けない
        let before = draft.to_toml();
        assert!(draft.set_prop("model", "style", &string("cyna")).is_err());
        // エラーのないキーを、新たに誤った値にする変更も受け付けない
        assert!(
            draft
                .set_prop("plan", "style", &string("nonexistent"))
                .is_err()
        );
        assert_eq!(draft.to_toml(), before, "拒否した変更は、下書きを変えない");

        // 残りも直すと、エラーがなくなる
        draft
            .set_prop("git_branch", "style", &string("green"))
            .unwrap();
        assert!(!draft.has_errors());
        assert_eq!(error_paths(&draft), Vec::<String>::new());

        // 元に戻すと、エラーも戻る
        draft.undo();
        assert_eq!(error_paths(&draft), ["git_branch.style"]);
        draft.undo();
        draft.undo();
        assert_eq!(error_paths(&draft), ["model.style", "git_branch.style"]);
    }

    #[test]
    fn test_erroneous_key_may_move_between_invalid_values() {
        // すでにエラーのキーを、別の誤った値にしても、エラーのキーは増えない（受け付ける）
        let mut draft = draft(
            "[model]
style = \"cyna\"
",
        );
        draft.set_prop("model", "style", &string("cynaa")).unwrap();
        assert!(draft.has_errors());
        assert_eq!(draft.config().model.style, "cynaa");
    }

    #[test]
    fn test_preview_prop_does_not_change_the_draft() {
        let draft = draft("");
        let before = draft.to_toml();
        let config = draft
            .preview_prop("git_branch", "symbol", &string("B "))
            .unwrap();
        assert_eq!(config.git_branch.symbol, "B ");
        assert_eq!(draft.config().git_branch.symbol, "🌿 ");
        assert_eq!(draft.to_toml(), before);
        assert!(!draft.can_undo());

        // エラーの値は Err。値が変わらないときは、現在の設定
        assert!(
            draft
                .preview_prop("git_branch", "style", &string("nonexistent"))
                .is_err()
        );
        let unchanged = draft
            .preview_prop("git_branch", "symbol", &string("🌿 "))
            .unwrap();
        assert_eq!(unchanged.git_branch.symbol, "🌿 ");
    }

    #[test]
    fn test_rejected_error_message_names_the_key() {
        let mut draft = draft("");
        let err = draft
            .set_prop("git_branch", "style", &string("bold nonexistent"))
            .unwrap_err();
        let message = err.to_string();
        assert!(message.contains("git_branch.style"), "{message}");
        assert!(message.contains("nonexistent"), "{message}");
    }

    #[test]
    fn test_set_prop_accepts_warnings_and_reports_them() {
        let mut draft = draft("");
        assert_eq!(draft.issues(), Vec::new());
        // `format` の未知の変数は警告にとどまる
        draft
            .set_prop("git_branch", "format", &string("$symbol$brnch"))
            .unwrap();
        assert_eq!(draft.config().git_branch.format, "$symbol$brnch");
        let issues = draft.issues();
        assert!(
            issues
                .iter()
                .any(|issue| issue.severity == Severity::Warning
                    && issue.path == "git_branch.format"),
            "{issues:?}"
        );
        assert!(
            issues
                .iter()
                .all(|issue| issue.severity == Severity::Warning)
        );
    }

    #[test]
    fn test_unchanged_value_is_not_recorded() {
        // (名前, 元の TOML, 部品, キー, 値)
        let cases = [
            // 既定値と同じで、キーがない: 書かない
            (
                "default-without-key",
                "",
                "git_branch",
                "disabled",
                PropValue::Bool(false),
            ),
            // 同じ値
            (
                "same-value",
                "[context]\nthreshold_warning = 60\n",
                "context",
                "threshold_warning",
                int(60),
            ),
        ];
        for (name, text, part, key, value) in cases {
            let mut draft = draft(text);
            draft.set_prop(part, key, &value).unwrap();
            assert!(!draft.can_undo(), "{name}");
            assert_eq!(draft.to_toml(), text, "{name}");
        }
    }

    #[test]
    fn test_prop_value_and_default_value() {
        let mut draft = draft("[git_branch]\nsymbol = \"G\"\n");
        assert_eq!(draft.prop_value("git_branch", "symbol"), Some(string("G")));
        assert_eq!(
            draft.default_value("git_branch", "symbol"),
            Some(string("🌿 "))
        );
        // 設定ファイルにないキーも、既定値が現在の値になる
        assert_eq!(
            draft.prop_value("git_branch", "disabled"),
            Some(PropValue::Bool(false))
        );
        assert_eq!(draft.prop_value("nothing", "x"), None);
        assert_eq!(draft.prop_value("git_branch", "nothing"), None);

        draft
            .set_prop("git_branch", "disabled", &PropValue::Bool(true))
            .unwrap();
        assert_eq!(
            draft.prop_value("git_branch", "disabled"),
            Some(PropValue::Bool(true))
        );
        assert_eq!(
            draft.default_value("git_branch", "disabled"),
            Some(PropValue::Bool(false))
        );
    }

    #[test]
    fn test_undo_walks_back_layout_and_props_in_order() {
        let mut draft = draft("[context]\nthreshold_warning = 60\n");
        let original_toml = draft.to_toml();

        draft.edit_layout(|editor| {
            editor.insert(Token::Part("plan".to_string()), Placement::After);
        });
        draft
            .set_prop("context", "threshold_warning", &int(65))
            .unwrap();
        draft.edit_layout(|editor| editor.move_selected(Direction::Left));
        draft
            .set_prop("git_branch", "symbol", &string("B"))
            .unwrap();
        let after_all = draft.to_toml();
        assert_ne!(after_all, original_toml);

        // 逆順に戻る: プロパティ → 配置 → プロパティ → 配置
        draft.undo();
        assert_eq!(
            draft.prop_value("git_branch", "symbol"),
            Some(string("🌿 "))
        );
        assert_eq!(draft.config().context.threshold_warning, 65);
        let moved = draft.layout().layout().to_format();

        draft.undo();
        assert_ne!(
            draft.layout().layout().to_format(),
            moved,
            "配置の移動が戻る"
        );
        assert_eq!(draft.config().context.threshold_warning, 65);

        draft.undo();
        assert_eq!(draft.config().context.threshold_warning, 60);
        assert!(draft.layout().layout().to_format().contains("$plan"));

        draft.undo();
        assert_eq!(draft.to_toml(), original_toml);
        assert!(!draft.can_undo());
        draft.undo(); // 履歴がなくても、何も起きない
        assert_eq!(draft.to_toml(), original_toml);
    }

    #[test]
    fn test_edit_layout_records_only_real_changes() {
        let mut draft = draft("");
        // 選択の移動・端での移動は、履歴に積まれない
        draft.edit_layout(|editor| editor.select(Direction::Right));
        draft.edit_layout(|editor| editor.move_selected(Direction::Up));
        assert!(!draft.can_undo());

        draft.edit_layout(LayoutEditor::add_line);
        assert!(draft.can_undo());
        draft.undo();
        assert!(!draft.can_undo());
    }

    #[test]
    fn test_issues_include_layout_and_document_warnings() {
        let mut draft = draft("[git_branch]\nsymbl = \"x\"\n");
        let issues = draft.issues();
        assert_eq!(issues.len(), 1, "{issues:?}");
        assert_eq!(issues[0].path, "git_branch.symbl");

        // 配置の未知の変数も警告にする。文書の `format` の警告とは重複しない
        draft.edit_layout(|editor| {
            editor.insert(Token::Literal("$bogus".to_string()), Placement::After);
        });
        let issues = draft.issues();
        let layout_warnings: Vec<_> = issues.iter().filter(|i| i.path == "format").collect();
        assert_eq!(layout_warnings.len(), 1, "{issues:?}");
        assert!(layout_warnings[0].message.contains("$bogus"));
        assert!(issues.iter().all(|i| i.severity == Severity::Warning));
    }

    #[test]
    fn test_to_toml_writes_the_layout_and_round_trips() {
        let mut draft = draft("# 設定\nformat = \"\"\"\n$directory\n$model\n\"\"\"\n");
        draft.edit_layout(LayoutEditor::add_line);
        draft.edit_layout(|editor| {
            editor.insert(Token::Part("quota".to_string()), Placement::After);
        });
        draft.set_prop("model", "symbol", &string("M ")).unwrap();

        let toml = draft.to_toml();
        assert!(toml.starts_with("# 設定\n"), "{toml}");
        // 保存した TOML を読み直すと、配置もプロパティも同じ
        let reloaded = validate(&toml);
        assert!(!reloaded.has_errors(), "{:?}", reloaded.issues);
        let config = reloaded.config.unwrap();
        assert_eq!(config.format, draft.layout().layout().to_format());
        assert_eq!(config.model.symbol, "M ");
    }

    #[test]
    fn test_initial_issues_come_from_the_loaded_document() {
        let draft = draft("[nothing]\nx = 1\n");
        let issues = draft.issues();
        assert_eq!(issues.len(), 1, "{issues:?}");
        assert_eq!(issues[0].path, "nothing");
        assert!(!draft.can_undo());
    }
}
