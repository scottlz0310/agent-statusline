//! 設定 TUI の状態と、キー入力による状態遷移（画面に依存しない）。

use std::cell::Cell;
use std::path::{Path, PathBuf};

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::editor::draft::{Draft, PropError};
use crate::editor::layout::Token;
use crate::editor::layout_edit::{Direction, LayoutEditor, Placement};
use crate::editor::load::ConfigSource;
use crate::editor::save::{ConflictKind, SaveError};
use crate::editor::update::PropValue;
use crate::engine::config::Config;
use crate::engine::parts::{PartSpec, PropKind, find};
use crate::modules::directory::{MAX_PATH_LEN, MIN_PATH_LEN};
use crate::preview::scenario::Scenario;
use crate::preview::width::Overflow;
use crate::tui::palette::{self, Category, PaletteItem};
use crate::tui::storage::Storage;
use crate::tui::symbols;

/// プレビューの幅の選択。`Auto` は実端末の幅
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WidthChoice {
    Auto,
    Columns(usize),
}

const WIDTHS: [WidthChoice; 4] = [
    WidthChoice::Auto,
    WidthChoice::Columns(80),
    WidthChoice::Columns(120),
    WidthChoice::Columns(160),
];

/// 閾値の増減の幅。`Shift` を押しているときは、大きい方
const THRESHOLD_STEP: i64 = 1;
const THRESHOLD_BIG_STEP: i64 = 10;
/// 文字数の上限の増減の幅
const LENGTH_STEP: usize = 1;
const LENGTH_BIG_STEP: usize = 10;

/// キー入力の宛先。`Tab` で、配置 → プロパティ → 部品一覧 → プレビューの順に切り替える
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Preview,
    Layout,
    Parts,
    Props,
}

impl Focus {
    fn next(self) -> Self {
        match self {
            Self::Layout => Self::Props,
            Self::Props => Self::Parts,
            Self::Parts => Self::Preview,
            Self::Preview => Self::Layout,
        }
    }

    fn previous(self) -> Self {
        match self {
            Self::Layout => Self::Preview,
            Self::Props => Self::Layout,
            Self::Parts => Self::Props,
            Self::Preview => Self::Parts,
        }
    }
}

/// 文字列の入力の対象
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputTarget {
    /// 部品のプロパティの値
    Prop {
        part: &'static str,
        key: &'static str,
    },
    /// 別名で保存するパス
    SavePath,
    /// 部品一覧の検索語。入力中は、その内容で一覧を絞り込む
    PartsSearch,
}

/// 文字列の入力。カーソルは、文字単位の位置（`0..=文字数`）
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextInput {
    pub target: InputTarget,
    chars: Vec<char>,
    cursor: usize,
}

impl TextInput {
    fn new(target: InputTarget, initial: &str) -> Self {
        let chars: Vec<char> = initial.chars().collect();
        Self {
            target,
            cursor: chars.len(),
            chars,
        }
    }

    fn prop(part: &'static str, key: &'static str, initial: &str) -> Self {
        Self::new(InputTarget::Prop { part, key }, initial)
    }

    pub fn value(&self) -> String {
        self.chars.iter().collect()
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    fn insert(&mut self, c: char) {
        self.chars.insert(self.cursor, c);
        self.cursor += 1;
    }

    fn backspace(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
            self.chars.remove(self.cursor);
        }
    }

    fn delete(&mut self) {
        if self.cursor < self.chars.len() {
            self.chars.remove(self.cursor);
        }
    }
}

/// 記号（`PropKind::Symbol`）の候補の選択。選んでいる間も、プレビューに反映する
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Picker {
    pub part: &'static str,
    pub key: &'static str,
    pub candidates: Vec<String>,
    index: usize,
}

impl Picker {
    pub fn index(&self) -> usize {
        self.index
    }

    pub fn selected(&self) -> &str {
        &self.candidates[self.index]
    }
}

/// 検証結果（エラー・警告）の一覧の表示。`v` で開く。状態行には先頭の 1 件しか出ないので、全件を見るために使う
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct IssuesView {
    index: usize,
    /// 選んでいる項目の中で、スクロールした行数。項目が一覧の高さを超えるときだけ使う
    scroll: usize,
    /// 選んでいる項目が一覧の高さを超える行数（描画のたびに更新する）。`↑↓` が項目の中をスクロールできる上限
    max_scroll: Cell<usize>,
}

impl IssuesView {
    pub fn index(&self) -> usize {
        self.index
    }

    pub fn scroll(&self) -> usize {
        self.scroll
    }

    /// 描画が、選んでいる項目の高さから求めた上限を伝える
    pub fn set_max_scroll(&self, max_scroll: usize) {
        self.max_scroll.set(max_scroll);
    }
}

/// 画面の下に出す確認・選択の状態。`Normal` 以外のとき、キーはその選択に使われる
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Normal,
    /// 未保存の変更があるまま終了しようとした
    ConfirmQuit,
    /// 保存しようとしたら、外部の変更と衝突した
    Conflict(ConflictKind),
}

pub struct App {
    pub focus: Focus,
    pub draft: Draft,
    /// 部品一覧の全項目（絞り込む前）
    pub palette: Vec<PaletteItem>,
    /// 絞り込んだ一覧（`visible_palette`）の中での、選択位置
    palette_index: usize,
    palette_category: Category,
    /// 確定した検索語。空なら絞り込まない
    palette_filter: String,
    props_index: usize,
    /// プロパティ欄の対象を、配置と部品一覧のどちらから決めるか。配置から `Tab` で入れば配置、
    /// 部品一覧から `BackTab` で入れば部品一覧
    props_from: Focus,
    pub input: Option<TextInput>,
    /// 記号の候補の選択（開いているとき）
    pub picker: Option<Picker>,
    /// 検証結果の一覧（開いているとき）
    pub issues_view: Option<IssuesView>,
    /// 直前の操作が受け付けられなかった理由（次のキー入力で消える）
    pub message: Option<String>,
    /// 直前の操作の結果の知らせ（保存した、など。次のキー入力で消える）
    pub notice: Option<String>,
    pub mode: Mode,
    storage: Box<dyn Storage>,
    /// 編集中の設定の元になっているファイル（保存すると、保存先に更新される）
    source: ConfigSource,
    /// 最後に保存した（または読み込んだ）TOML。これと違えば、未保存の変更がある
    saved_toml: String,
    scenario_index: usize,
    width_index: usize,
    pub overflow: Overflow,
    pub should_quit: bool,
}

#[cfg(test)]
impl App {
    /// テスト用: 未作成の設定ファイル（`test.toml`）を、何も書かない `Storage` で編集する
    pub fn for_test(draft: Draft) -> Self {
        Self::with_storage_for_test(
            draft,
            Box::<crate::tui::storage::fake::FakeStorage>::default(),
        )
    }

    pub fn with_storage_for_test(draft: Draft, storage: Box<dyn Storage>) -> Self {
        let source = ConfigSource::Missing {
            target: PathBuf::from("test.toml"),
            from_link: false,
        };
        Self::new(draft, source, storage)
    }
}

impl App {
    pub fn new(draft: Draft, source: ConfigSource, storage: Box<dyn Storage>) -> Self {
        Self {
            focus: Focus::Layout,
            saved_toml: draft.to_toml(),
            draft,
            source,
            storage,
            notice: None,
            mode: Mode::Normal,
            palette: palette::items(),
            palette_index: 0,
            palette_category: Category::All,
            palette_filter: String::new(),
            props_index: 0,
            props_from: Focus::Layout,
            input: None,
            picker: None,
            issues_view: None,
            message: None,
            scenario_index: 0,
            width_index: 0,
            overflow: Overflow::Truncate,
            should_quit: false,
        }
    }

    /// 絞り込んだ一覧の中での、選択位置
    pub fn palette_index(&self) -> usize {
        self.palette_index
    }

    pub fn palette_category(&self) -> Category {
        self.palette_category
    }

    /// 一覧に使う検索語。検索語を入力中は入力欄の内容、そうでなければ確定した検索語
    pub fn palette_query(&self) -> String {
        match &self.input {
            Some(input) if input.target == InputTarget::PartsSearch => input.value(),
            _ => self.palette_filter.clone(),
        }
    }

    /// カテゴリと検索語で絞り込んだ部品一覧
    pub fn visible_palette(&self) -> Vec<PaletteItem> {
        palette::visible(&self.palette, self.palette_category, &self.palette_query())
    }

    /// 選択中の項目。絞り込みの結果が空のときは `None`
    pub fn selected_item(&self) -> Option<PaletteItem> {
        self.visible_palette().get(self.palette_index).copied()
    }

    /// 絞り込み（カテゴリ・検索語）を変えた後、変える前に選んでいた項目が残っていればそれを、
    /// 無ければ先頭を選ぶ
    fn keep_palette_selection(&mut self, before: Option<PaletteItem>) {
        let visible = self.visible_palette();
        self.palette_index = before
            .and_then(|item| visible.iter().position(|v| *v == item))
            .unwrap_or(0);
    }

    /// 部品一覧にフォーカスがあるとき、選択中の項目を配置のカーソルの後ろに仮に入れた状態。
    /// 確定（`Enter`）するまで、下書きは変わらない
    pub fn tentative_editor(&self) -> Option<LayoutEditor> {
        if self.focus != Focus::Parts {
            return None;
        }
        let mut editor = self.draft.layout().without_history();
        palette::apply(self.selected_item()?, &mut editor, Placement::After);
        Some(editor)
    }

    /// プロパティ欄が対象にする部品。部品一覧から入ったときは、一覧の選択項目、
    /// そうでなければ、配置のカーソルの部品。部品でなければ `None`
    pub fn target_part(&self) -> Option<&'static PartSpec> {
        let from_parts = self.focus == Focus::Parts
            || (self.focus == Focus::Props && self.props_from == Focus::Parts);
        if from_parts {
            match self.selected_item() {
                Some(PaletteItem::Part(id)) => find(id),
                _ => None,
            }
        } else {
            match self.draft.layout().selected() {
                Some(Token::Part(id)) => find(id),
                _ => None,
            }
        }
    }

    /// プロパティ欄で選択中の行（対象の部品のプロパティ数に収める）
    pub fn props_index(&self) -> usize {
        let count = self.target_part().map_or(0, |part| part.props.len());
        self.props_index.min(count.saturating_sub(1))
    }

    /// プロパティの文字列を入力中のとき、確定したと仮定した設定。入力中のプレビューに使う（下書きは変えない）
    pub fn input_preview(&self) -> Option<Result<Config, PropError>> {
        let input = self.input.as_ref()?;
        let InputTarget::Prop { part, key } = input.target else {
            return None;
        };
        let value = PropValue::Str(input.value());
        Some(self.draft.preview_prop(part, key, &value))
    }

    /// 記号の候補を選んでいるとき、その候補を確定したと仮定した設定。プレビューに使う（下書きは変えない）
    pub fn picker_preview(&self) -> Option<Result<Config, PropError>> {
        let picker = self.picker.as_ref()?;
        let value = PropValue::Str(picker.selected().to_string());
        Some(self.draft.preview_prop(picker.part, picker.key, &value))
    }

    /// 確定前の入力・選択を、確定したと仮定した設定（プレビュー用）。何も入力・選択していなければ `None`
    pub fn live_preview(&self) -> Option<Result<Config, PropError>> {
        self.picker_preview().or_else(|| self.input_preview())
    }

    /// 記号の入力・選択の対象の値（表示幅の案内用）。記号以外を編集しているときは `None`
    pub fn symbol_in_progress(&self) -> Option<String> {
        if let Some(picker) = &self.picker {
            return Some(picker.selected().to_string());
        }
        let input = self.input.as_ref()?;
        let InputTarget::Prop { part, key } = input.target else {
            return None;
        };
        let spec = find(part)?.props.iter().find(|prop| prop.key == key)?;
        matches!(spec.kind, PropKind::Symbol).then(|| input.value())
    }

    /// 未保存の変更があるか。元に戻して、最後に保存した内容と同じになれば、なくなる
    pub fn is_dirty(&self) -> bool {
        self.draft.to_toml() != self.saved_toml
    }

    /// 編集中の設定ファイルの説明（ヘッダーに出す）
    pub fn source_label(&self) -> String {
        match &self.source {
            ConfigSource::Existing { path, .. } if path.requested == path.real => {
                path.requested.display().to_string()
            }
            ConfigSource::Existing { path, .. } => {
                format!("{} → {}", path.requested.display(), path.real.display())
            }
            ConfigSource::Missing { target, .. } => {
                format!("未作成（保存すると作成されます: {}）", target.display())
            }
        }
    }

    fn source_path(&self) -> &Path {
        match &self.source {
            ConfigSource::Existing { path, .. } => &path.requested,
            ConfigSource::Missing { target, .. } => target,
        }
    }

    pub fn scenario(&self) -> Scenario {
        Scenario::ALL[self.scenario_index]
    }

    pub fn width_choice(&self) -> WidthChoice {
        WIDTHS[self.width_index]
    }

    /// プレビューの幅（列数）。`Auto` のときは実端末の幅
    pub fn columns(&self, terminal_width: usize) -> usize {
        match self.width_choice() {
            WidthChoice::Auto => terminal_width,
            WidthChoice::Columns(columns) => columns,
        }
    }

    fn set_focus(&mut self, focus: Focus) {
        if matches!(focus, Focus::Layout | Focus::Parts) {
            self.props_from = focus;
        }
        if focus == Focus::Props {
            self.props_index = 0;
        }
        self.focus = focus;
    }

    pub fn handle_key(&mut self, key: KeyEvent) {
        self.message = None;
        self.notice = None;
        // 文字列の入力中は、`q` や `u` も含め、すべてのキーが入力へ行く
        if self.input.is_some() {
            self.handle_input_key(key);
            return;
        }
        if self.picker.is_some() {
            self.handle_picker_key(key.code);
            return;
        }
        if self.issues_view.is_some() {
            self.handle_issues_key(key.code);
            return;
        }
        if self.mode != Mode::Normal {
            self.handle_mode_key(key.code);
            return;
        }
        match key.code {
            // 部品一覧で検索語が効いているときの `Esc` は、終了ではなく、検索の解除
            KeyCode::Esc if self.focus == Focus::Parts && !self.palette_filter.is_empty() => {
                let before = self.selected_item();
                self.palette_filter.clear();
                self.keep_palette_selection(before);
            }
            KeyCode::Char('q') | KeyCode::Esc => {
                if self.is_dirty() {
                    self.mode = Mode::ConfirmQuit;
                } else {
                    self.should_quit = true;
                }
            }
            KeyCode::Char('s') => {
                self.save();
            }
            KeyCode::Tab => self.set_focus(self.focus.next()),
            KeyCode::BackTab => self.set_focus(self.focus.previous()),
            KeyCode::Char('u') => self.draft.undo(),
            KeyCode::Char('v') => self.issues_view = Some(IssuesView::default()),
            code => match self.focus {
                Focus::Preview => self.handle_preview_key(code),
                Focus::Layout => self.handle_layout_key(code, key.modifiers),
                Focus::Parts => self.handle_parts_key(code),
                Focus::Props => self.handle_props_key(code, key.modifiers),
            },
        }
    }

    /// 記号の候補の選択。`Enter` で決定、`e` で自由に入力、`Esc` で閉じる
    fn handle_picker_key(&mut self, key: KeyCode) {
        let Some(picker) = self.picker.as_mut() else {
            return;
        };
        match key {
            KeyCode::Up => picker.index = picker.index.saturating_sub(1),
            KeyCode::Down => picker.index = (picker.index + 1).min(picker.candidates.len() - 1),
            KeyCode::Esc => self.picker = None,
            KeyCode::Char('e') => {
                let (part, key, value) = (picker.part, picker.key, picker.selected().to_string());
                self.picker = None;
                self.input = Some(TextInput::prop(part, key, &value));
            }
            KeyCode::Enter => {
                let value = PropValue::Str(picker.selected().to_string());
                // 受け付けられないときは、開いたままにして、理由を示す
                match self.draft.set_prop(picker.part, picker.key, &value) {
                    Ok(()) => self.picker = None,
                    Err(err) => self.message = Some(err.to_string()),
                }
            }
            _ => {}
        }
    }

    /// 検証結果の一覧のキー。`↑↓` で選び、`Esc` / `v` で閉じる。
    /// 選んでいる項目が一覧の高さを超えるときは、`↑↓` が項目の中を 1 行ずつスクロールし、端まで行ってから隣の項目へ進む
    fn handle_issues_key(&mut self, key: KeyCode) {
        let last = self.draft.issues().len().saturating_sub(1);
        let Some(view) = self.issues_view.as_mut() else {
            return;
        };
        match key {
            KeyCode::Up if view.scroll > 0 => view.scroll -= 1,
            KeyCode::Up => view.index = view.index.saturating_sub(1),
            KeyCode::Down if view.scroll < view.max_scroll.get() => view.scroll += 1,
            KeyCode::Down if view.index < last => {
                view.index += 1;
                view.scroll = 0;
            }
            KeyCode::Esc | KeyCode::Char('v') => self.issues_view = None,
            _ => {}
        }
    }

    /// 確認・選択の状態のキー。選択肢以外のキーは、何もせずに戻る
    fn handle_mode_key(&mut self, key: KeyCode) {
        let mode = std::mem::replace(&mut self.mode, Mode::Normal);
        match (mode, key) {
            (Mode::ConfirmQuit, KeyCode::Char('y')) => self.should_quit = true,
            (Mode::ConfirmQuit, KeyCode::Char('s')) => {
                // 保存できなければ、終了しない（競合なら、その選択へ進む）
                if self.save() {
                    self.should_quit = true;
                }
            }
            (Mode::Conflict(_), KeyCode::Char('r')) => self.reload(),
            (Mode::Conflict(_), KeyCode::Char('a')) => {
                let default = format!("{}.new", self.source_path().display());
                self.input = Some(TextInput::new(InputTarget::SavePath, &default));
            }
            _ => {}
        }
    }

    /// 保存する。保存できたら `true`。エラーが残っているとき、変更がないときは、保存しない。
    /// 外部の変更と衝突したときは、再読み込みか別名で保存かを選ぶ状態へ移る
    fn save(&mut self) -> bool {
        if let Some(issue) = self
            .draft
            .issues()
            .into_iter()
            .find(|issue| issue.severity == crate::editor::validate::Severity::Error)
        {
            self.message = Some(format!(
                "エラーが残っているため保存できません: {}",
                issue.message
            ));
            return false;
        }
        if !self.is_dirty() {
            self.notice = Some("変更がないため、保存しませんでした".to_string());
            return false;
        }
        let text = self.draft.to_toml();
        match self.storage.save(&self.source, &text) {
            Ok(source) => {
                self.source = source;
                self.saved_toml = text;
                self.notice = Some(format!("保存しました: {}", self.source_path().display()));
                true
            }
            Err(SaveError::Conflict(kind)) => {
                self.mode = Mode::Conflict(kind);
                false
            }
            Err(err) => {
                self.message = Some(err.to_string());
                false
            }
        }
    }

    /// 別のパスへ保存する。`true` なら保存できた
    fn save_as(&mut self, path: &str) -> bool {
        if path.is_empty() {
            self.message = Some("保存するパスを入力してください".to_string());
            return false;
        }
        let text = self.draft.to_toml();
        match self.storage.save_as(&PathBuf::from(path), &text) {
            Ok(source) => {
                self.source = source;
                self.saved_toml = text;
                self.notice = Some(format!("保存しました: {}", self.source_path().display()));
                true
            }
            Err(SaveError::Conflict(_)) => {
                self.message = Some(format!("既にあるファイルは上書きしません: {path}"));
                false
            }
            Err(err) => {
                self.message = Some(err.to_string());
                false
            }
        }
    }

    /// 設定ファイルを読み込み直す。**今の編集は破棄される**（競合の選択で、利用者が選んだとき）
    fn reload(&mut self) {
        // 未作成のときも、今の保存先を読む（起動時の指定へは戻らない）
        let path = self.source_path().to_path_buf();
        match self.storage.reload(&path) {
            Ok(loaded) => {
                self.draft = Draft::new(loaded.config, loaded.document);
                self.source = loaded.source;
                self.saved_toml = self.draft.to_toml();
                self.notice =
                    Some("再読み込みしました（それまでの編集は破棄しました）".to_string());
            }
            Err(err) => self.message = Some(err.to_string()),
        }
    }

    fn handle_preview_key(&mut self, key: KeyCode) {
        let count = Scenario::ALL.len();
        match key {
            KeyCode::Right | KeyCode::Char('l') => {
                self.scenario_index = (self.scenario_index + 1) % count;
            }
            KeyCode::Left | KeyCode::Char('h') => {
                self.scenario_index = (self.scenario_index + count - 1) % count;
            }
            KeyCode::Char('w') => self.width_index = (self.width_index + 1) % WIDTHS.len(),
            KeyCode::Char('o') => {
                self.overflow = match self.overflow {
                    Overflow::Wrap => Overflow::Truncate,
                    Overflow::Truncate => Overflow::Wrap,
                };
            }
            _ => {}
        }
    }

    /// 矢印は選択の移動、`Shift` を足すとトークンの移動
    fn handle_layout_key(&mut self, key: KeyCode, modifiers: KeyModifiers) {
        let direction = match key {
            KeyCode::Up => Some(Direction::Up),
            KeyCode::Down => Some(Direction::Down),
            KeyCode::Left => Some(Direction::Left),
            KeyCode::Right => Some(Direction::Right),
            _ => None,
        };
        if let Some(direction) = direction {
            if modifiers.contains(KeyModifiers::SHIFT) {
                self.draft.edit_layout(|e| e.move_selected(direction));
            } else {
                self.draft.edit_layout(|e| e.select(direction));
            }
            return;
        }
        match key {
            KeyCode::Char('d') | KeyCode::Delete => {
                self.draft.edit_layout(LayoutEditor::delete_selected);
            }
            KeyCode::Char('n') => self.draft.edit_layout(LayoutEditor::break_line_before),
            KeyCode::Char('a') => self.draft.edit_layout(LayoutEditor::add_line),
            _ => {}
        }
    }

    /// `↑↓` で選び、`←→` でカテゴリを切り替え、`/` で検索する。
    /// `Enter` は選択中のトークンの後ろへ、`i` は前へ追加する（絞り込みの結果が空のときは何もしない）
    fn handle_parts_key(&mut self, key: KeyCode) {
        let before = self.selected_item();
        match key {
            KeyCode::Up => self.palette_index = self.palette_index.saturating_sub(1),
            KeyCode::Down => {
                let last = self.visible_palette().len().saturating_sub(1);
                self.palette_index = (self.palette_index + 1).min(last);
            }
            KeyCode::Left | KeyCode::Right => {
                self.palette_category = if key == KeyCode::Right {
                    self.palette_category.next()
                } else {
                    self.palette_category.previous()
                };
                self.keep_palette_selection(before);
            }
            KeyCode::Char('/') => {
                self.input = Some(TextInput::new(
                    InputTarget::PartsSearch,
                    &self.palette_filter,
                ));
            }
            KeyCode::Enter => {
                if let Some(item) = before {
                    self.draft
                        .edit_layout(|e| palette::apply(item, e, Placement::After));
                }
            }
            KeyCode::Char('i') => {
                if let Some(item) = before {
                    self.draft
                        .edit_layout(|e| palette::apply(item, e, Placement::Before));
                }
            }
            _ => {}
        }
    }

    /// 行の選択（`↑↓`）と、値の変更。値の変え方は、プロパティの種類で決まる
    fn handle_props_key(&mut self, key: KeyCode, modifiers: KeyModifiers) {
        let Some(part) = self.target_part() else {
            return;
        };
        let selected = self.props_index();
        match key {
            KeyCode::Up => {
                self.props_index = selected.saturating_sub(1);
                return;
            }
            KeyCode::Down => {
                self.props_index = (selected + 1).min(part.props.len() - 1);
                return;
            }
            _ => {}
        }
        let prop = &part.props[selected];
        let Some(current) = self.draft.prop_value(part.id, prop.key) else {
            return;
        };
        let new_value = match (&prop.kind, current) {
            (PropKind::Bool, PropValue::Bool(on)) => {
                matches!(key, KeyCode::Enter | KeyCode::Char(' ')).then_some(PropValue::Bool(!on))
            }
            (PropKind::Choice(choices), PropValue::Str(value)) => {
                cycle_choice(choices, &value, key).map(PropValue::from)
            }
            (PropKind::Threshold, PropValue::Int(value)) => {
                let step = if modifiers.contains(KeyModifiers::SHIFT) {
                    THRESHOLD_BIG_STEP
                } else {
                    THRESHOLD_STEP
                };
                match key {
                    KeyCode::Left => Some(PropValue::Int((value - step).max(0))),
                    KeyCode::Right => Some(PropValue::Int((value + step).min(100))),
                    _ => None,
                }
            }
            (PropKind::Length, PropValue::Int(value)) => {
                let step = if modifiers.contains(KeyModifiers::SHIFT) {
                    LENGTH_BIG_STEP
                } else {
                    LENGTH_STEP
                };
                let value = usize::try_from(value).unwrap_or(0);
                step_length(value, key, step)
                    .and_then(|v| i64::try_from(v).ok())
                    .map(PropValue::Int)
            }
            // 記号は、候補から選ぶ（絵文字を入力できない環境がある）。自由に入力するには、候補の一覧で `e`
            (PropKind::Symbol, PropValue::Str(value)) if key == KeyCode::Enter => {
                let default = match self.draft.default_value(part.id, prop.key) {
                    Some(PropValue::Str(default)) => default,
                    _ => String::new(),
                };
                let candidates = symbols::candidates(part.id, prop.key, &value, &default);
                let index = candidates.iter().position(|c| *c == value).unwrap_or(0);
                self.picker = Some(Picker {
                    part: part.id,
                    key: prop.key,
                    candidates,
                    index,
                });
                None
            }
            (_, PropValue::Str(value)) if key == KeyCode::Enter => {
                self.input = Some(TextInput::prop(part.id, prop.key, &value));
                None
            }
            _ => None,
        };
        if let Some(value) = new_value
            && let Err(err) = self.draft.set_prop(part.id, prop.key, &value)
        {
            self.message = Some(err.to_string());
        }
    }

    fn handle_input_key(&mut self, key: KeyEvent) {
        // 部品一覧の検索は、入力のたびに一覧が絞り込まれる。選んでいた項目が残っていれば、選び続ける
        let searching = self
            .input
            .as_ref()
            .is_some_and(|input| input.target == InputTarget::PartsSearch);
        let before = if searching {
            self.selected_item()
        } else {
            None
        };
        let Some(input) = self.input.as_mut() else {
            return;
        };
        match key.code {
            KeyCode::Esc => {
                self.input = None;
                if searching {
                    // 検索を取り消すと、絞り込みも解く
                    self.palette_filter.clear();
                }
            }
            KeyCode::Enter => match input.target {
                InputTarget::PartsSearch => {
                    self.palette_filter = input.value().trim().to_string();
                    self.input = None;
                }
                InputTarget::Prop { part, key } => {
                    let value = PropValue::Str(input.value());
                    // 受け付けられないときは、入力を続けて、理由を示す
                    match self.draft.set_prop(part, key, &value) {
                        Ok(()) => self.input = None,
                        Err(err) => self.message = Some(err.to_string()),
                    }
                }
                InputTarget::SavePath => {
                    let path = input.value();
                    if self.save_as(path.trim()) {
                        self.input = None;
                    }
                }
            },
            KeyCode::Char(c)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                input.insert(c);
            }
            KeyCode::Backspace => input.backspace(),
            KeyCode::Delete => input.delete(),
            KeyCode::Left => input.cursor = input.cursor.saturating_sub(1),
            KeyCode::Right => input.cursor = (input.cursor + 1).min(input.chars.len()),
            KeyCode::Home => input.cursor = 0,
            KeyCode::End => input.cursor = input.chars.len(),
            _ => {}
        }
        if searching {
            self.keep_palette_selection(before);
        }
    }
}

/// 文字数の上限の増減。0（自動）と `MIN_PATH_LEN` の間の値は受け付けないので、飛び越える
fn step_length(value: usize, key: KeyCode, step: usize) -> Option<usize> {
    match key {
        KeyCode::Left if value <= MIN_PATH_LEN => Some(0),
        KeyCode::Left => Some(value.saturating_sub(step).max(MIN_PATH_LEN)),
        KeyCode::Right if value == 0 => Some(MIN_PATH_LEN),
        KeyCode::Right => Some((value + step).min(MAX_PATH_LEN)),
        _ => None,
    }
}

/// 選択肢の巡回。`Enter` / `→` は次、`←` は前。現在の値が選択肢にないときは、`Enter` / `→` で先頭、
/// `←` で末尾にする
fn cycle_choice(
    choices: &'static [&'static str],
    current: &str,
    key: KeyCode,
) -> Option<&'static str> {
    let position = choices.iter().position(|choice| *choice == current);
    let len = choices.len();
    match (key, position) {
        (KeyCode::Enter | KeyCode::Right, Some(i)) => Some(choices[(i + 1) % len]),
        (KeyCode::Enter | KeyCode::Right, None) => choices.first().copied(),
        (KeyCode::Left, Some(i)) => Some(choices[(i + len - 1) % len]),
        (KeyCode::Left, None) => choices.last().copied(),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::layout_edit::Cursor;

    fn app(format: &str) -> App {
        App::for_test(Draft::with_format(format))
    }

    fn app_with_focus(format: &str, focus: Focus) -> App {
        let mut app = app(format);
        app.focus = focus;
        app
    }

    fn press(app: &mut App, code: KeyCode) {
        app.handle_key(KeyEvent::from(code));
    }

    fn press_shift(app: &mut App, code: KeyCode) {
        app.handle_key(KeyEvent::new(code, KeyModifiers::SHIFT));
    }

    fn type_text(app: &mut App, text: &str) {
        for c in text.chars() {
            press(app, KeyCode::Char(c));
        }
    }

    fn format(app: &App) -> String {
        app.draft.layout().layout().to_format()
    }

    #[test]
    fn test_starts_on_layout() {
        assert_eq!(app("$model").focus, Focus::Layout);
    }

    #[test]
    fn test_tab_cycles_focus_and_routes_keys() {
        let mut app = app("$directory $model");
        // 配置にフォーカスがあるとき、矢印は選択を動かし、シナリオは変わらない
        press(&mut app, KeyCode::Right);
        assert_eq!(app.draft.layout().cursor(), Cursor { line: 0, index: 1 });
        assert_eq!(app.scenario(), Scenario::ALL[0]);

        // Tab: 配置 → プロパティ → 部品一覧 → プレビュー → 配置
        for expected in [Focus::Props, Focus::Parts, Focus::Preview, Focus::Layout] {
            press(&mut app, KeyCode::Tab);
            assert_eq!(app.focus, expected);
        }
        // BackTab は逆順
        for expected in [Focus::Preview, Focus::Parts, Focus::Props, Focus::Layout] {
            press(&mut app, KeyCode::BackTab);
            assert_eq!(app.focus, expected);
        }

        // プレビューにフォーカスがあるとき、矢印はシナリオを動かし、選択は変わらない
        app.focus = Focus::Preview;
        press(&mut app, KeyCode::Right);
        assert_eq!(app.scenario(), Scenario::ALL[1]);
        press(&mut app, KeyCode::Left);
        assert_eq!(app.draft.layout().cursor(), Cursor { line: 0, index: 1 });
    }

    #[test]
    fn test_scenario_navigation_wraps_around() {
        let mut app = app_with_focus("$model", Focus::Preview);
        assert_eq!(app.scenario(), Scenario::ALL[0]);
        for key in [KeyCode::Right, KeyCode::Char('l'), KeyCode::Right] {
            press(&mut app, key);
        }
        assert_eq!(app.scenario(), Scenario::ALL[3]);
        for key in [KeyCode::Left, KeyCode::Char('h'), KeyCode::Left] {
            press(&mut app, key);
        }
        assert_eq!(app.scenario(), Scenario::ALL[0]);
        // 先頭から戻ると末尾、末尾から進むと先頭
        press(&mut app, KeyCode::Left);
        assert_eq!(app.scenario(), Scenario::ALL[Scenario::ALL.len() - 1]);
        press(&mut app, KeyCode::Right);
        assert_eq!(app.scenario(), Scenario::ALL[0]);
    }

    #[test]
    fn test_width_cycle_and_columns() {
        let mut app = app_with_focus("$model", Focus::Preview);
        let mut seen = Vec::new();
        for _ in 0..WIDTHS.len() {
            seen.push(app.columns(100));
            press(&mut app, KeyCode::Char('w'));
        }
        // 実端末の幅（Auto）、80、120、160 と巡回し、一周で戻る
        assert_eq!(seen, [100, 80, 120, 160]);
        assert_eq!(app.width_choice(), WidthChoice::Auto);
    }

    #[test]
    fn test_preview_toggles() {
        let mut app = app_with_focus("$model", Focus::Preview);
        assert_eq!(app.overflow, Overflow::Truncate);
        press(&mut app, KeyCode::Char('o'));
        assert_eq!(app.overflow, Overflow::Wrap);
        press(&mut app, KeyCode::Char('o'));
        assert_eq!(app.overflow, Overflow::Truncate);
    }

    #[test]
    fn test_preview_keys_do_nothing_on_layout_focus() {
        let mut app = app("$model");
        for key in [KeyCode::Char('w'), KeyCode::Char('o')] {
            press(&mut app, key);
        }
        assert_eq!(app.width_choice(), WidthChoice::Auto);
        assert_eq!(app.overflow, Overflow::Truncate);
    }

    #[test]
    fn test_layout_keys_do_nothing_on_other_focus() {
        for focus in [Focus::Preview, Focus::Parts, Focus::Props] {
            let mut app = app_with_focus("$directory $model", focus);
            for key in [KeyCode::Char('d'), KeyCode::Char('n'), KeyCode::Char('a')] {
                press(&mut app, key);
            }
            assert_eq!(format(&app), "$directory $model", "{focus:?}");
            assert!(!app.draft.can_undo(), "{focus:?}");
        }
    }

    /// 部品一覧の `target` の項目まで、`Down` で進める
    fn select_item(app: &mut App, target: PaletteItem) {
        while app.selected_item() != Some(target) {
            press(app, KeyCode::Down);
        }
    }

    #[test]
    fn test_parts_navigation_clamps_at_both_ends() {
        let mut app = app_with_focus("$model", Focus::Parts);
        press(&mut app, KeyCode::Up);
        assert_eq!(app.palette_index(), 0);
        for _ in 0..app.palette.len() + 3 {
            press(&mut app, KeyCode::Down);
        }
        assert_eq!(app.palette_index(), app.palette.len() - 1);
        assert_eq!(app.selected_item(), Some(PaletteItem::LineBreak));
    }

    fn visible_ids(app: &App) -> Vec<String> {
        app.visible_palette()
            .into_iter()
            .map(palette::label)
            .collect()
    }

    #[test]
    fn test_parts_category_keys_filter_the_list_and_wrap_around() {
        let mut app = app_with_focus("$directory", Focus::Parts);
        assert_eq!(app.palette_category(), Category::All);
        assert_eq!(app.visible_palette().len(), app.palette.len());

        press(&mut app, KeyCode::Right);
        assert_eq!(app.palette_category(), Category::Workspace);
        assert_eq!(visible_ids(&app), ["directory", "git_branch", "git_status"]);
        // 絞り込んだ一覧の先頭が選ばれ、Enter で追加するのは、その項目
        assert_eq!(app.palette_index(), 0);
        press(&mut app, KeyCode::Down);
        press(&mut app, KeyCode::Enter);
        assert_eq!(format(&app), "$directory$git_branch");

        // 末尾の次は先頭（すべて）、先頭の前は末尾（レイアウト）
        for _ in 0..4 {
            press(&mut app, KeyCode::Right);
        }
        assert_eq!(app.palette_category(), Category::All);
        press(&mut app, KeyCode::Left);
        assert_eq!(app.palette_category(), Category::Layout);
        assert_eq!(app.selected_item(), Some(PaletteItem::Literal(" ")));
        assert_eq!(app.visible_palette().len(), 5);
    }

    #[test]
    fn test_changing_the_category_keeps_the_selected_item_when_it_stays_visible() {
        let mut app = app_with_focus("$directory", Focus::Parts);
        select_item(&mut app, PaletteItem::Part("quota"));
        // すべて → 作業場所・Git: quota は消えるので先頭になる
        press(&mut app, KeyCode::Right);
        assert_eq!(app.selected_item(), Some(PaletteItem::Part("directory")));
        // エージェントへ: 先頭
        press(&mut app, KeyCode::Right);
        select_item(&mut app, PaletteItem::Part("quota"));
        // エージェント → レビュー → ... → すべて と戻っても、quota は残らないが、
        // エージェント → すべて（前へ 2 回）では quota が見えるので選び続ける
        press(&mut app, KeyCode::Left);
        press(&mut app, KeyCode::Left);
        assert_eq!(app.palette_category(), Category::All);
        assert_eq!(app.selected_item(), Some(PaletteItem::Part("directory")));
    }

    #[test]
    fn test_parts_search_filters_live_and_confirms_with_enter() {
        let mut app = app_with_focus("$directory", Focus::Parts);
        press(&mut app, KeyCode::Char('/'));
        let input = app.input.as_ref().expect("検索の入力が始まる");
        assert_eq!(input.target, InputTarget::PartsSearch);
        assert_eq!(input.value(), "");

        // 入力のたびに一覧が絞り込まれる。`q` や `u` や `v` も文字（終了も元に戻すも検証結果も開かない）
        type_text(&mut app, "GIT");
        assert_eq!(visible_ids(&app), ["git_branch", "git_status"]);
        type_text(&mut app, "q");
        assert_eq!(app.visible_palette(), Vec::new());
        assert!(!app.should_quit);
        press(&mut app, KeyCode::Backspace);
        assert_eq!(visible_ids(&app), ["git_branch", "git_status"]);
        // 下書きは、確定するまで変わらない
        assert_eq!(format(&app), "$directory");

        press(&mut app, KeyCode::Enter);
        assert!(app.input.is_none());
        assert_eq!(visible_ids(&app), ["git_branch", "git_status"]);
        // 確定した検索語は、カテゴリと組み合わさる
        press(&mut app, KeyCode::Right);
        press(&mut app, KeyCode::Right);
        assert_eq!(app.palette_category(), Category::Agent);
        assert_eq!(app.visible_palette(), Vec::new());
        press(&mut app, KeyCode::Left);
        assert_eq!(visible_ids(&app), ["git_branch", "git_status"]);
    }

    #[test]
    fn test_parts_search_keeps_the_selected_item_while_typing() {
        let mut app = app_with_focus("$directory", Focus::Parts);
        select_item(&mut app, PaletteItem::Part("git_status"));
        press(&mut app, KeyCode::Char('/'));
        type_text(&mut app, "git");
        // git_status は絞り込みの後も見えるので、選び続ける
        assert_eq!(app.selected_item(), Some(PaletteItem::Part("git_status")));
        type_text(&mut app, "_b");
        // git_status が消えたら、先頭
        assert_eq!(app.selected_item(), Some(PaletteItem::Part("git_branch")));
    }

    #[test]
    fn test_parts_search_esc_clears_the_filter_and_esc_with_a_filter_does_not_quit() {
        let mut app = app_with_focus("$directory", Focus::Parts);
        press(&mut app, KeyCode::Char('/'));
        type_text(&mut app, "git");
        // 入力中の Esc は、検索を取り消して、絞り込みを解く
        press(&mut app, KeyCode::Esc);
        assert!(app.input.is_none());
        assert_eq!(app.visible_palette().len(), app.palette.len());

        // 確定した検索語がある間の Esc は、終了ではなく検索の解除
        press(&mut app, KeyCode::Char('/'));
        type_text(&mut app, "git");
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.visible_palette().len(), 2);
        press(&mut app, KeyCode::Esc);
        assert!(!app.should_quit);
        assert_eq!(app.visible_palette().len(), app.palette.len());
        // 解除した後の Esc は、これまでどおり終了
        press(&mut app, KeyCode::Esc);
        assert!(app.should_quit);
    }

    #[test]
    fn test_parts_search_with_no_match_is_inert_and_does_not_panic() {
        let mut app = app_with_focus("$directory", Focus::Parts);
        press(&mut app, KeyCode::Char('/'));
        type_text(&mut app, "zzz");
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.visible_palette(), Vec::new());
        assert_eq!(app.selected_item(), None);
        for key in [
            KeyCode::Up,
            KeyCode::Down,
            KeyCode::Enter,
            KeyCode::Char('i'),
        ] {
            press(&mut app, key);
        }
        assert_eq!(format(&app), "$directory", "何も追加されない");
        assert_eq!(app.tentative_editor().map(|e| e.layout().to_format()), None);
        assert!(app.target_part().is_none());
        // 検索を解除すると、元の一覧に戻る
        press(&mut app, KeyCode::Esc);
        assert_eq!(app.visible_palette().len(), app.palette.len());
        assert_eq!(app.palette_index(), 0);
    }

    #[test]
    fn test_parts_search_and_category_keys_only_work_on_the_parts_list() {
        for focus in [Focus::Layout, Focus::Preview] {
            let mut app = app_with_focus("$directory", focus);
            press(&mut app, KeyCode::Char('/'));
            assert!(app.input.is_none(), "{focus:?}");
            assert_eq!(app.palette_category(), Category::All, "{focus:?}");
        }
    }

    #[test]
    fn test_parts_enter_inserts_after_and_i_inserts_before() {
        let mut app = app_with_focus("$directory", Focus::Parts);
        select_item(&mut app, PaletteItem::Part("quota"));
        press(&mut app, KeyCode::Enter);
        assert_eq!(format(&app), "$directory$quota");
        // 追加したトークンが選ばれるので、続けて Enter を押すと、その後ろへ足される
        select_item(&mut app, PaletteItem::Literal(" | "));
        press(&mut app, KeyCode::Enter);
        assert_eq!(format(&app), "$directory$quota | ");
        // `i` は、選択中のトークンの前
        select_item(&mut app, PaletteItem::LineBreak);
        press(&mut app, KeyCode::Char('i'));
        assert_eq!(format(&app), "$directory$quota\n | ");
        // 元に戻す（フォーカスによらない）
        for expected in ["$directory$quota | ", "$directory$quota", "$directory"] {
            press(&mut app, KeyCode::Char('u'));
            assert_eq!(format(&app), expected);
        }
    }

    #[test]
    fn test_tentative_editor_does_not_change_the_layout() {
        let mut app = app("$directory");
        assert!(
            app.tentative_editor().is_none(),
            "部品一覧にフォーカスがないとき"
        );

        app.focus = Focus::Parts;
        select_item(&mut app, PaletteItem::Part("model"));
        let tentative = app.tentative_editor().expect("仮挿入がある");
        assert_eq!(tentative.layout().to_format(), "$directory$model");
        assert_eq!(
            tentative.selected(),
            Some(&Token::Part("model".to_string()))
        );
        // 確定するまで、設定の配置も履歴も変わらない
        assert_eq!(format(&app), "$directory");
        assert!(!app.draft.can_undo());

        // カーソルを動かすと、仮挿入も追従する
        press(&mut app, KeyCode::Up);
        let tentative = app.tentative_editor().unwrap();
        assert_eq!(tentative.layout().to_format(), "$directory$sandbox");
    }

    #[test]
    fn test_layout_edit_keys() {
        let mut app = app("$directory $model\n$quota");
        // 選択を `$model` まで右へ → Shift+左で、前の固定テキストと入れ替え
        press(&mut app, KeyCode::Right);
        press(&mut app, KeyCode::Right);
        press_shift(&mut app, KeyCode::Left);
        assert_eq!(format(&app), "$directory$model \n$quota");
        // Shift+下で、隣の行へ移す
        press_shift(&mut app, KeyCode::Down);
        assert_eq!(format(&app), "$directory \n$quota$model");
        // 削除
        press(&mut app, KeyCode::Char('d'));
        assert_eq!(format(&app), "$directory \n$quota");
        // 改行を挿入・行を追加
        press(&mut app, KeyCode::Char('n'));
        assert_eq!(format(&app), "$directory \n\n$quota");
        press(&mut app, KeyCode::Char('a'));
        assert_eq!(format(&app), "$directory \n\n$quota\n");
        // 元に戻す（フォーカスによらない）
        press(&mut app, KeyCode::Tab);
        for expected in ["$directory \n\n$quota", "$directory \n$quota"] {
            press(&mut app, KeyCode::Char('u'));
            assert_eq!(format(&app), expected);
        }
    }

    #[test]
    fn test_quit_and_unassigned_keys() {
        let mut unassigned = app("$model");
        press(&mut unassigned, KeyCode::Char('x'));
        assert!(!unassigned.should_quit, "未割り当てのキーは何もしない");
        assert!(!unassigned.draft.can_undo());
        for key in [KeyCode::Char('q'), KeyCode::Esc] {
            for focus in [Focus::Preview, Focus::Layout, Focus::Parts, Focus::Props] {
                let mut app = app_with_focus("$model", focus);
                press(&mut app, key);
                assert!(app.should_quit, "{key:?}/{focus:?}");
            }
        }
    }

    // ---- プロパティ欄 ----

    /// 配置の先頭が `part` のアプリで、プロパティ欄にフォーカスを移す
    fn props_app(part: &str) -> App {
        let mut app = app(&format!("${part}"));
        press(&mut app, KeyCode::Tab); // プロパティ（配置のカーソルの部品）
        assert_eq!(app.focus, Focus::Props);
        app
    }

    /// 対象の部品のプロパティ欄で、`key` の行まで `Down` で進める
    fn select_prop(app: &mut App, key: &str) {
        let part = app.target_part().expect("対象の部品がある");
        let index = part.props.iter().position(|p| p.key == key).unwrap();
        for _ in 0..index {
            press(app, KeyCode::Down);
        }
        assert_eq!(part.props[app.props_index()].key, key);
    }

    #[test]
    fn test_target_part_follows_layout_cursor_or_palette_item() {
        let mut app = app("$directory $model\n$quota");
        assert_eq!(app.target_part().map(|p| p.id), Some("directory"));
        press(&mut app, KeyCode::Right); // 固定テキスト
        assert!(app.target_part().is_none());
        press(&mut app, KeyCode::Right);
        assert_eq!(app.target_part().map(|p| p.id), Some("model"));

        // 配置から Tab で入ったプロパティ欄は、配置のカーソルの部品
        press(&mut app, KeyCode::Tab);
        assert_eq!(app.focus, Focus::Props);
        assert_eq!(app.target_part().map(|p| p.id), Some("model"));
        // 部品一覧にフォーカスがあるときは、一覧の項目（部品）。固定テキストや改行は対象なし
        press(&mut app, KeyCode::Tab);
        assert_eq!(app.focus, Focus::Parts);
        assert_eq!(app.target_part().map(|p| p.id), Some("directory"));
        select_item(&mut app, PaletteItem::Part("quota"));
        assert_eq!(app.target_part().map(|p| p.id), Some("quota"));
        select_item(&mut app, PaletteItem::LineBreak);
        assert!(app.target_part().is_none());
        // 一覧から BackTab で入ったプロパティ欄は、一覧の項目のまま
        press(&mut app, KeyCode::BackTab);
        assert_eq!(app.focus, Focus::Props);
        assert!(app.target_part().is_none(), "改行の項目のまま");
        // 配置に戻ると、配置のカーソルの部品
        press(&mut app, KeyCode::BackTab);
        assert_eq!(app.focus, Focus::Layout);
        assert_eq!(app.target_part().map(|p| p.id), Some("model"));
    }

    #[test]
    fn test_props_navigation_clamps() {
        let mut app = props_app("git_branch");
        let count = app.target_part().unwrap().props.len();
        press(&mut app, KeyCode::Up);
        assert_eq!(app.props_index(), 0);
        for _ in 0..count + 3 {
            press(&mut app, KeyCode::Down);
        }
        assert_eq!(app.props_index(), count - 1);
        // 部品でないトークンでは、何も起きない
        let mut empty = app_with_focus(" ", Focus::Props);
        press(&mut empty, KeyCode::Down);
        press(&mut empty, KeyCode::Enter);
        assert!(empty.input.is_none());
        assert!(!empty.draft.can_undo());
    }

    #[test]
    fn test_bool_toggle() {
        let mut app = props_app("git_branch");
        select_prop(&mut app, "disabled");
        for key in [KeyCode::Enter, KeyCode::Char(' ')] {
            let before = app.draft.prop_value("git_branch", "disabled");
            press(&mut app, key);
            assert_ne!(
                app.draft.prop_value("git_branch", "disabled"),
                before,
                "{key:?}"
            );
        }
        // 2 回反転して、元に戻っている。履歴には 2 件（元に戻せる）
        assert_eq!(
            app.draft.prop_value("git_branch", "disabled"),
            Some(PropValue::Bool(false))
        );
        press(&mut app, KeyCode::Char('u'));
        assert_eq!(
            app.draft.prop_value("git_branch", "disabled"),
            Some(PropValue::Bool(true))
        );
        // 矢印では反転しない
        press(&mut app, KeyCode::Right);
        assert_eq!(
            app.draft.prop_value("git_branch", "disabled"),
            Some(PropValue::Bool(true))
        );
    }

    #[test]
    fn test_choice_cycles() {
        let mut app = props_app("directory");
        select_prop(&mut app, "truncation_mode");
        let mode = |app: &App| app.draft.prop_value("directory", "truncation_mode");
        assert_eq!(mode(&app), Some(PropValue::from("middle")));
        // (キー, 期待する値)
        let steps = [
            (KeyCode::Enter, "left"),
            (KeyCode::Right, "middle"),
            (KeyCode::Left, "left"),
            (KeyCode::Left, "middle"),
        ];
        for (key, expected) in steps {
            press(&mut app, key);
            assert_eq!(mode(&app), Some(PropValue::from(expected)), "{key:?}");
        }
    }

    #[test]
    fn test_cycle_choice_from_unknown_value() {
        let choices: &'static [&'static str] = &["left", "middle"];
        assert_eq!(cycle_choice(choices, "right", KeyCode::Enter), Some("left"));
        assert_eq!(cycle_choice(choices, "right", KeyCode::Right), Some("left"));
        assert_eq!(
            cycle_choice(choices, "right", KeyCode::Left),
            Some("middle")
        );
        assert_eq!(cycle_choice(choices, "left", KeyCode::Up), None);
    }

    #[test]
    fn test_threshold_steps_and_rejections() {
        let mut app = props_app("context");
        select_prop(&mut app, "threshold_warning");
        let warning = |app: &App| app.draft.prop_value("context", "threshold_warning");
        assert_eq!(warning(&app), Some(PropValue::Int(50)));
        press(&mut app, KeyCode::Right);
        assert_eq!(warning(&app), Some(PropValue::Int(51)));
        press(&mut app, KeyCode::Left);
        press(&mut app, KeyCode::Left);
        assert_eq!(warning(&app), Some(PropValue::Int(49)));
        press_shift(&mut app, KeyCode::Left);
        assert_eq!(warning(&app), Some(PropValue::Int(39)));
        // 0 で止まる（負にならない）
        for _ in 0..5 {
            press_shift(&mut app, KeyCode::Left);
        }
        assert_eq!(warning(&app), Some(PropValue::Int(0)));
        assert!(app.message.is_none());

        // critical（80）以上にはできない: 受け付けず、理由を示す。値は変わらない
        for _ in 0..8 {
            press_shift(&mut app, KeyCode::Right);
        }
        assert_eq!(warning(&app), Some(PropValue::Int(70)));
        press_shift(&mut app, KeyCode::Right);
        assert_eq!(warning(&app), Some(PropValue::Int(70)));
        let message = app.message.clone().expect("理由が示される");
        assert!(message.contains("threshold_warning"), "{message}");
        // 次のキー入力で、理由は消える
        press(&mut app, KeyCode::Down);
        assert!(app.message.is_none());
    }

    #[test]
    fn test_threshold_upper_bound() {
        let mut app = props_app("context");
        select_prop(&mut app, "threshold_critical");
        for _ in 0..4 {
            press_shift(&mut app, KeyCode::Right);
        }
        assert_eq!(
            app.draft.prop_value("context", "threshold_critical"),
            Some(PropValue::Int(100))
        );
    }

    #[test]
    fn test_step_length() {
        // (名前, 現在の値, キー, 増減の幅, 期待する値)。型は usize
        let cases = [
            ("auto to min", 0, KeyCode::Right, 1, Some(10)),
            ("auto stays", 0, KeyCode::Left, 1, Some(0)),
            ("min to auto", 10, KeyCode::Left, 1, Some(0)),
            ("big step stops at min", 15, KeyCode::Left, 10, Some(10)),
            ("step down", 40, KeyCode::Left, 1, Some(39)),
            ("step up", 40, KeyCode::Right, 1, Some(41)),
            ("big step up", 40, KeyCode::Right, 10, Some(50)),
            ("stops at max", 995, KeyCode::Right, 10, Some(1000)),
            ("other key", 40, KeyCode::Enter, 1, None),
        ];
        for (name, value, key, step, expected) in cases {
            assert_eq!(step_length(value, key, step), expected, "{name}");
        }
    }

    #[test]
    fn test_max_length_prop_steps_through_the_ui() {
        let mut app = props_app("directory");
        select_prop(&mut app, "max_length");
        let value = |app: &App| app.draft.prop_value("directory", "max_length");
        assert_eq!(value(&app), Some(PropValue::Int(0)));
        press(&mut app, KeyCode::Right);
        assert_eq!(value(&app), Some(PropValue::Int(10)));
        press_shift(&mut app, KeyCode::Right);
        assert_eq!(value(&app), Some(PropValue::Int(20)));
        press(&mut app, KeyCode::Left);
        assert_eq!(value(&app), Some(PropValue::Int(19)));
        // 0（自動）まで戻れて、9 以下の値は経由しない
        press_shift(&mut app, KeyCode::Left);
        assert_eq!(value(&app), Some(PropValue::Int(10)));
        press(&mut app, KeyCode::Left);
        assert_eq!(value(&app), Some(PropValue::Int(0)));
        assert!(app.message.is_none());
    }

    #[test]
    fn test_text_input_commit_and_cancel() {
        let mut app = props_app("git_branch");
        select_prop(&mut app, "symbol");
        press(&mut app, KeyCode::Enter); // 記号は、候補の一覧が開く
        press(&mut app, KeyCode::Char('e')); // 自由に入力する
        let input = app.input.as_ref().expect("入力が始まる");
        assert_eq!(input.value(), "🌿 ");
        assert_eq!(input.cursor(), 2);

        // 入力中は、`q` や `u`、Tab も文字・何もしない（終了も元に戻すもしない）
        press(&mut app, KeyCode::Backspace);
        press(&mut app, KeyCode::Backspace);
        type_text(&mut app, "qu");
        press(&mut app, KeyCode::Tab);
        assert!(!app.should_quit);
        assert_eq!(app.focus, Focus::Props);
        assert_eq!(app.input.as_ref().unwrap().value(), "qu");
        // 確定前は、下書きも履歴も変わらない
        assert_eq!(
            app.draft.prop_value("git_branch", "symbol"),
            Some(PropValue::from("🌿 "))
        );
        assert!(!app.draft.can_undo());

        press(&mut app, KeyCode::Enter);
        assert!(app.input.is_none());
        assert_eq!(
            app.draft.prop_value("git_branch", "symbol"),
            Some(PropValue::from("qu"))
        );
        press(&mut app, KeyCode::Char('u'));
        assert_eq!(
            app.draft.prop_value("git_branch", "symbol"),
            Some(PropValue::from("🌿 "))
        );

        // Esc で取り消すと、何も変わらない
        press(&mut app, KeyCode::Enter);
        press(&mut app, KeyCode::Char('e'));
        type_text(&mut app, "xyz");
        press(&mut app, KeyCode::Esc);
        assert!(app.input.is_none());
        assert!(!app.should_quit, "Esc は入力の取り消しだけ");
        assert_eq!(
            app.draft.prop_value("git_branch", "symbol"),
            Some(PropValue::from("🌿 "))
        );
        assert!(!app.draft.can_undo());
    }

    #[test]
    fn test_text_input_editing_keys() {
        let mut app = props_app("git_branch");
        select_prop(&mut app, "symbol");
        app.input = Some(TextInput::prop("git_branch", "symbol", "ab🌿c"));
        // (名前, 操作, 期待する値, 期待するカーソル)
        let cases: [(&str, KeyCode, &str, usize); 10] = [
            ("home", KeyCode::Home, "ab🌿c", 0),
            ("right", KeyCode::Right, "ab🌿c", 1),
            ("insert-in-the-middle", KeyCode::Char('X'), "aXb🌿c", 2),
            ("end", KeyCode::End, "aXb🌿c", 5),
            ("left", KeyCode::Left, "aXb🌿c", 4),
            (
                "backspace-removes-a-whole-char",
                KeyCode::Backspace,
                "aXbc",
                3,
            ),
            (
                "delete-removes-the-char-at-cursor",
                KeyCode::Delete,
                "aXb",
                3,
            ),
            ("delete-at-end-does-nothing", KeyCode::Delete, "aXb", 3),
            ("home-again", KeyCode::Home, "aXb", 0),
            (
                "backspace-at-start-does-nothing",
                KeyCode::Backspace,
                "aXb",
                0,
            ),
        ];
        for (name, key, value, cursor) in cases {
            press(&mut app, key);
            let input = app.input.as_ref().unwrap();
            assert_eq!(input.value(), value, "{name}");
            assert_eq!(input.cursor(), cursor, "{name}");
        }
        press(&mut app, KeyCode::Delete);
        assert_eq!(app.input.as_ref().unwrap().value(), "Xb");
        // 文字の挿入は、Ctrl / Alt を伴うときは入らない
        let before = app.input.clone();
        app.handle_key(KeyEvent::new(KeyCode::Char('z'), KeyModifiers::CONTROL));
        app.handle_key(KeyEvent::new(KeyCode::Char('z'), KeyModifiers::ALT));
        assert_eq!(app.input, before);
    }

    #[test]
    fn test_text_input_rejected_value_keeps_the_input_open() {
        let mut app = props_app("git_branch");
        select_prop(&mut app, "style");
        press(&mut app, KeyCode::Enter);
        for _ in 0..5 {
            press(&mut app, KeyCode::Backspace);
        }
        type_text(&mut app, "bold nonexistent");
        // 入力中のプレビューは、エラーを返す
        assert!(matches!(app.input_preview(), Some(Err(_))));
        press(&mut app, KeyCode::Enter);
        assert!(app.input.is_some(), "受け付けられないときは、入力を続ける");
        let message = app.message.clone().expect("理由が示される");
        assert!(message.contains("nonexistent"), "{message}");
        assert!(!app.draft.can_undo());

        // 直すと、確定できる。入力中のプレビューは、確定したと仮定した設定
        for _ in 0.."nonexistent".len() {
            press(&mut app, KeyCode::Backspace);
        }
        type_text(&mut app, "cyan");
        let preview = app.input_preview().unwrap().unwrap();
        assert_eq!(preview.git_branch.style, "bold cyan");
        assert_eq!(app.draft.config().git_branch.style, "green");
        press(&mut app, KeyCode::Enter);
        assert!(app.input.is_none());
        assert_eq!(app.draft.config().git_branch.style, "bold cyan");
    }

    #[test]
    fn test_props_edit_is_undone_in_order_with_layout_edits() {
        let mut app = app("$directory");
        // 配置の編集 → プロパティの編集 → 元に戻す 2 回
        press(&mut app, KeyCode::Char('a'));
        app.focus = Focus::Layout;
        press(&mut app, KeyCode::Up);
        app.focus = Focus::Props;
        press(&mut app, KeyCode::Down);
        select_prop(&mut app, "disabled");
        press(&mut app, KeyCode::Enter);
        press(&mut app, KeyCode::Char('u'));
        assert!(app.draft.layout().layout().to_format().ends_with('\n'));
        press(&mut app, KeyCode::Char('u'));
        assert_eq!(format(&app), "$directory");
    }

    // ---- 保存・未保存・競合 ----

    use crate::editor::load::{ContentHash, LoadedConfig, ResolvedPath};
    use crate::tui::storage::fake::FakeStorage;

    fn app_with_storage(format: &str) -> (App, FakeStorage) {
        let storage = FakeStorage::default();
        let app = App::with_storage_for_test(Draft::with_format(format), Box::new(storage.clone()));
        (app, storage)
    }

    /// 配置を 1 つ変えて、未保存の変更がある状態にする
    fn make_dirty(app: &mut App) {
        press(app, KeyCode::Char('a')); // 末尾に空の行を足す（配置にフォーカスがある）
        assert!(app.is_dirty());
    }

    fn loaded(text: &str) -> LoadedConfig {
        LoadedConfig {
            config: toml_edit::de::from_str(text).unwrap(),
            document: text.parse().unwrap(),
            source: ConfigSource::Existing {
                path: ResolvedPath {
                    requested: PathBuf::from("reloaded.toml"),
                    real: PathBuf::from("reloaded.toml"),
                },
                fingerprint: ContentHash::of(text.as_bytes()),
            },
        }
    }

    #[test]
    fn test_dirty_follows_edits_and_undo() {
        let (mut app, _) = app_with_storage("$model");
        assert!(!app.is_dirty());
        make_dirty(&mut app);
        press(&mut app, KeyCode::Char('u'));
        assert!(!app.is_dirty(), "元に戻すと、未保存の変更はなくなる");
    }

    #[test]
    fn test_save_writes_the_toml_and_clears_dirty() {
        let (mut app, storage) = app_with_storage("$model");
        make_dirty(&mut app);
        let expected = app.draft.to_toml();
        press(&mut app, KeyCode::Char('s'));
        assert_eq!(storage.0.borrow().saves, [expected]);
        assert!(!app.is_dirty());
        let notice = app.notice.clone().expect("保存した知らせ");
        assert!(notice.contains("保存しました"), "{notice}");
        assert!(notice.contains("test.toml"), "{notice}");
        assert!(app.source_label().contains("test.toml"));
        assert!(
            !app.source_label().contains("未作成"),
            "保存すると、既存のファイルになる"
        );
        // 次のキー入力で、知らせは消える
        press(&mut app, KeyCode::Down);
        assert!(app.notice.is_none());
    }

    #[test]
    fn test_save_without_changes_does_nothing() {
        let (mut app, storage) = app_with_storage("$model");
        press(&mut app, KeyCode::Char('s'));
        assert_eq!(storage.0.borrow().saves.len(), 0);
        assert!(app.notice.clone().unwrap().contains("変更がない"));
    }

    #[test]
    fn test_save_is_refused_while_errors_remain() {
        let toml = "[model]\nstyle = \"cyna\"\n";
        let draft = Draft::new(
            toml_edit::de::from_str(toml).unwrap(),
            toml.parse().unwrap(),
        );
        let storage = FakeStorage::default();
        let mut app = App::with_storage_for_test(draft, Box::new(storage.clone()));
        press(&mut app, KeyCode::Char('a')); // 変更を作る
        press(&mut app, KeyCode::Char('s'));
        assert_eq!(storage.0.borrow().saves.len(), 0, "保存しない");
        let message = app.message.clone().expect("理由が示される");
        assert!(message.contains("エラーが残っている"), "{message}");
        assert!(message.contains("cyna"), "{message}");
        assert!(app.is_dirty());
    }

    #[test]
    fn test_save_error_is_shown_and_keeps_the_edit() {
        let (mut app, storage) = app_with_storage("$model");
        make_dirty(&mut app);
        storage.0.borrow_mut().save_error = Some(SaveError::NoHistoryDir);
        press(&mut app, KeyCode::Char('s'));
        assert!(app.message.clone().unwrap().contains("履歴の保存先"));
        assert!(app.is_dirty());
        assert_eq!(app.mode, Mode::Normal);
    }

    #[test]
    fn test_issues_view_opens_scrolls_and_closes() {
        let toml = "[nothing]\nx = 1\n\n[other]\nx = 1\n";
        let config = toml_edit::de::from_str::<Config>(toml).unwrap();
        let mut app = App::for_test(Draft::new(config, toml.parse().unwrap()));
        assert_eq!(app.issues_view, None);

        press(&mut app, KeyCode::Char('v'));
        assert_eq!(app.issues_view.as_ref().map(IssuesView::index), Some(0));
        // 一覧が開いている間は、`q` も `s` も `u` も効かない（終了も保存もしない）
        for key in [KeyCode::Char('q'), KeyCode::Char('s'), KeyCode::Char('u')] {
            press(&mut app, key);
            assert!(!app.should_quit);
            assert_eq!(app.mode, Mode::Normal);
            assert!(app.issues_view.is_some());
        }
        // 範囲の外へは進まない
        press(&mut app, KeyCode::Up);
        assert_eq!(app.issues_view.as_ref().map(IssuesView::index), Some(0));
        for _ in 0..5 {
            press(&mut app, KeyCode::Down);
        }
        assert_eq!(app.issues_view.as_ref().map(IssuesView::index), Some(1));

        // `Esc` で閉じる（終了しない）。もう一度 `v` で開くと、先頭から
        press(&mut app, KeyCode::Esc);
        assert_eq!(app.issues_view, None);
        assert!(!app.should_quit);
        press(&mut app, KeyCode::Char('v'));
        assert_eq!(app.issues_view.as_ref().map(IssuesView::index), Some(0));
        press(&mut app, KeyCode::Char('v'));
        assert_eq!(app.issues_view, None);
    }

    #[test]
    fn test_v_is_a_character_while_typing() {
        let mut app = props_app("model");
        select_prop(&mut app, "symbol");
        press(&mut app, KeyCode::Enter); // 記号は、候補の一覧が開く
        press(&mut app, KeyCode::Char('e')); // 自由に入力する
        press(&mut app, KeyCode::Char('v'));
        assert_eq!(app.issues_view, None);
        assert!(app.input.as_ref().unwrap().value().ends_with('v'));
    }

    #[test]
    fn test_s_is_a_character_while_typing() {
        let mut app = props_app("git_branch");
        select_prop(&mut app, "symbol");
        press(&mut app, KeyCode::Enter);
        press(&mut app, KeyCode::Char('e'));
        press(&mut app, KeyCode::Char('s'));
        assert_eq!(app.input.as_ref().unwrap().value(), "🌿 s");
        assert!(!app.draft.can_undo(), "保存でも確定でもない");
    }

    #[test]
    fn test_quit_when_clean_exits_immediately() {
        for key in [KeyCode::Char('q'), KeyCode::Esc] {
            let (mut app, _) = app_with_storage("$model");
            press(&mut app, key);
            assert!(app.should_quit, "{key:?}");
        }
    }

    #[test]
    fn test_quit_when_dirty_asks_first() {
        for key in [KeyCode::Char('q'), KeyCode::Esc] {
            let (mut app, storage) = app_with_storage("$model");
            make_dirty(&mut app);
            press(&mut app, key);
            assert!(!app.should_quit, "{key:?}");
            assert_eq!(app.mode, Mode::ConfirmQuit, "{key:?}");

            // 選択肢以外のキー（`q` を含む）は、何もせずに戻る
            press(&mut app, KeyCode::Char('q'));
            assert_eq!(app.mode, Mode::Normal);
            assert!(!app.should_quit);
            assert!(app.is_dirty());

            // y: 保存せず終了
            press(&mut app, key);
            press(&mut app, KeyCode::Char('y'));
            assert!(app.should_quit);
            assert_eq!(storage.0.borrow().saves.len(), 0);
        }
    }

    #[test]
    fn test_quit_with_save() {
        let (mut app, storage) = app_with_storage("$model");
        make_dirty(&mut app);
        press(&mut app, KeyCode::Char('q'));
        press(&mut app, KeyCode::Char('s'));
        assert!(app.should_quit);
        assert_eq!(storage.0.borrow().saves.len(), 1);
    }

    #[test]
    fn test_quit_with_save_does_not_quit_when_saving_fails() {
        let (mut app, storage) = app_with_storage("$model");
        make_dirty(&mut app);
        storage.0.borrow_mut().save_error = Some(SaveError::NoHistoryDir);
        press(&mut app, KeyCode::Char('q'));
        press(&mut app, KeyCode::Char('s'));
        assert!(!app.should_quit);
        assert!(app.message.is_some());
        assert_eq!(app.mode, Mode::Normal);

        // 衝突したときは、その選択へ進む（終了しない）
        storage.0.borrow_mut().save_error = Some(SaveError::Conflict(ConflictKind::Changed));
        press(&mut app, KeyCode::Char('q'));
        press(&mut app, KeyCode::Char('s'));
        assert!(!app.should_quit);
        assert_eq!(app.mode, Mode::Conflict(ConflictKind::Changed));
    }

    #[test]
    fn test_conflict_cancel_keeps_the_edit() {
        let (mut app, storage) = app_with_storage("$model");
        make_dirty(&mut app);
        storage.0.borrow_mut().save_error = Some(SaveError::Conflict(ConflictKind::Changed));
        press(&mut app, KeyCode::Char('s'));
        assert_eq!(app.mode, Mode::Conflict(ConflictKind::Changed));
        // 選択肢以外のキーは、何もせずに戻る
        press(&mut app, KeyCode::Esc);
        assert_eq!(app.mode, Mode::Normal);
        assert!(!app.should_quit);
        assert!(app.is_dirty(), "編集は残る");
        assert_eq!(storage.0.borrow().reloads.len(), 0);
    }

    #[test]
    fn test_conflict_reload_discards_the_edit() {
        let (mut app, storage) = app_with_storage("$model");
        make_dirty(&mut app);
        {
            let mut state = storage.0.borrow_mut();
            state.save_error = Some(SaveError::Conflict(ConflictKind::Changed));
            state.reload_to = Some(loaded("format = \"$plan\"\n"));
        }
        press(&mut app, KeyCode::Char('s'));
        press(&mut app, KeyCode::Char('r'));
        assert_eq!(app.mode, Mode::Normal);
        assert_eq!(format(&app), "$plan", "ディスク上の内容になる");
        assert!(!app.is_dirty());
        assert!(!app.draft.can_undo(), "履歴も新しくなる");
        assert!(app.notice.clone().unwrap().contains("再読み込みしました"));
        assert!(app.source_label().contains("reloaded.toml"));
        // 未作成のときも、今の保存先を渡す
        assert_eq!(storage.0.borrow().reloads, [PathBuf::from("test.toml")]);
    }

    #[test]
    fn test_reload_always_uses_the_current_path_even_after_save_as_and_deletion() {
        let (mut app, storage) = app_with_storage("$model");
        let conflict = |kind| Some(SaveError::Conflict(kind));
        let missing = |path: &str| LoadedConfig {
            config: Config::default(),
            document: toml_edit::DocumentMut::new(),
            source: ConfigSource::Missing {
                target: PathBuf::from(path),
                from_link: false,
            },
        };

        // 起動時のパス（test.toml）で衝突 → 別名（test.toml.new）で保存する
        make_dirty(&mut app);
        storage.0.borrow_mut().save_error = conflict(ConflictKind::Changed);
        press(&mut app, KeyCode::Char('s'));
        press(&mut app, KeyCode::Char('a'));
        press(&mut app, KeyCode::Enter);
        assert!(app.source_label().contains("test.toml.new"));

        // 別名のファイルが外部で削除された → 保存で衝突 → 再読み込み。未作成になり、保存先は別名のまま
        make_dirty(&mut app);
        {
            let mut state = storage.0.borrow_mut();
            state.save_error = conflict(ConflictKind::Deleted);
            state.reload_to = Some(missing("test.toml.new"));
        }
        press(&mut app, KeyCode::Char('s'));
        press(&mut app, KeyCode::Char('r'));
        assert!(app.source_label().contains("未作成"));
        assert!(app.source_label().contains("test.toml.new"));

        // 外部で再作成された → 保存で衝突 → 再読み込み。起動時のパスへ戻らず、別名のパスを読む
        make_dirty(&mut app);
        {
            let mut state = storage.0.borrow_mut();
            state.save_error = conflict(ConflictKind::Created);
            state.reload_to = Some(loaded("format = \"$plan\"\n"));
        }
        press(&mut app, KeyCode::Char('s'));
        press(&mut app, KeyCode::Char('r'));
        assert_eq!(
            storage.0.borrow().reloads,
            [
                PathBuf::from("test.toml.new"),
                PathBuf::from("test.toml.new")
            ],
            "どちらも、そのときの編集対象のパス"
        );
        assert_eq!(format(&app), "$plan");
    }

    #[test]
    fn test_conflict_reload_failure_is_shown() {
        let (mut app, storage) = app_with_storage("$model");
        make_dirty(&mut app);
        storage.0.borrow_mut().save_error = Some(SaveError::Conflict(ConflictKind::Deleted));
        press(&mut app, KeyCode::Char('s'));
        press(&mut app, KeyCode::Char('r')); // `reload_to` がないので、失敗する
        assert!(app.message.is_some());
        assert!(app.is_dirty(), "読み込めなかったときは、編集を捨てない");
        assert_eq!(format(&app), "$model\n");
    }

    #[test]
    fn test_conflict_save_as() {
        let (mut app, storage) = app_with_storage("$model");
        make_dirty(&mut app);
        storage.0.borrow_mut().save_error = Some(SaveError::Conflict(ConflictKind::Changed));
        press(&mut app, KeyCode::Char('s'));
        press(&mut app, KeyCode::Char('a'));
        assert_eq!(app.mode, Mode::Normal);
        let input = app.input.as_ref().expect("パスの入力が始まる");
        assert_eq!(input.target, InputTarget::SavePath);
        assert_eq!(
            input.value(),
            "test.toml.new",
            "元のパスに .new を足した既定"
        );

        // パスを書き換えて保存する。入力中は、`q` や `s` も文字になる
        for _ in 0.."test.toml.new".len() {
            press(&mut app, KeyCode::Backspace);
        }
        type_text(&mut app, "other.toml");
        let expected = app.draft.to_toml();
        press(&mut app, KeyCode::Enter);
        assert!(app.input.is_none());
        assert_eq!(
            storage.0.borrow().save_as,
            [(PathBuf::from("other.toml"), expected)]
        );
        assert!(!app.is_dirty());
        assert!(
            app.source_label().contains("other.toml"),
            "保存先が編集中のファイルになる"
        );
    }

    #[test]
    fn test_save_as_refuses_existing_files_and_empty_paths() {
        let (mut app, storage) = app_with_storage("$model");
        make_dirty(&mut app);
        storage.0.borrow_mut().save_error = Some(SaveError::Conflict(ConflictKind::Changed));
        press(&mut app, KeyCode::Char('s'));
        press(&mut app, KeyCode::Char('a'));

        // 既にファイルがあるパスは、上書きせず、入力を続ける
        storage.0.borrow_mut().save_as_error = Some(SaveError::Conflict(ConflictKind::Created));
        press(&mut app, KeyCode::Enter);
        assert!(app.input.is_some());
        let message = app.message.clone().unwrap();
        assert!(message.contains("上書きしません"), "{message}");
        assert!(message.contains("test.toml.new"), "{message}");
        assert!(app.is_dirty());

        // 空のパスは、保存せず、入力を続ける
        for _ in 0.."test.toml.new".len() {
            press(&mut app, KeyCode::Backspace);
        }
        let calls = storage.0.borrow().save_as.len();
        press(&mut app, KeyCode::Enter);
        assert!(app.input.is_some());
        assert_eq!(storage.0.borrow().save_as.len(), calls, "保存を試みない");
        assert!(app.message.clone().unwrap().contains("パスを入力"));

        // Esc で取り消すと、編集は残り、通常の状態へ戻る
        press(&mut app, KeyCode::Esc);
        assert!(app.input.is_none());
        assert_eq!(app.mode, Mode::Normal);
        assert!(app.is_dirty());
    }

    // ---- 記号の候補の選択 ----

    #[test]
    fn test_enter_on_a_symbol_opens_the_picker_at_the_current_value() {
        let mut app = props_app("git_branch");
        select_prop(&mut app, "symbol");
        press(&mut app, KeyCode::Enter);
        assert!(
            app.input.is_none(),
            "記号は、すぐには文字列の入力にならない"
        );
        let picker = app.picker.as_ref().expect("候補の一覧が開く");
        assert_eq!((picker.part, picker.key), ("git_branch", "symbol"));
        assert_eq!(picker.selected(), "🌿 ", "今の値を選んでいる");
        assert!(picker.candidates.iter().any(|c| c == "🌱 "));
        assert!(picker.candidates.iter().any(String::is_empty), "記号なし");
    }

    #[test]
    fn test_picker_previews_then_applies_or_cancels() {
        let mut app = props_app("git_branch");
        select_prop(&mut app, "symbol");
        press(&mut app, KeyCode::Enter);
        press(&mut app, KeyCode::Down);
        assert_eq!(app.picker.as_ref().unwrap().selected(), "🌱 ");
        // 選んでいる間も、プレビューには反映する。下書きは変えない
        let preview = app.live_preview().unwrap().unwrap();
        assert_eq!(preview.git_branch.symbol, "🌱 ");
        assert_eq!(app.draft.config().git_branch.symbol, "🌿 ");
        assert!(!app.draft.can_undo());

        // Enter で決定。元に戻せる
        press(&mut app, KeyCode::Enter);
        assert!(app.picker.is_none());
        assert_eq!(app.draft.config().git_branch.symbol, "🌱 ");
        press(&mut app, KeyCode::Char('u'));
        assert_eq!(app.draft.config().git_branch.symbol, "🌿 ");

        // Esc は、一覧を閉じるだけ（終了しない）。何も変わらない
        press(&mut app, KeyCode::Enter);
        press(&mut app, KeyCode::Down);
        press(&mut app, KeyCode::Esc);
        assert!(app.picker.is_none());
        assert!(!app.should_quit);
        assert_eq!(app.draft.config().git_branch.symbol, "🌿 ");
        assert!(!app.draft.can_undo());
    }

    #[test]
    fn test_picker_navigation_clamps_and_ignores_other_keys() {
        let mut app = props_app("git_branch");
        select_prop(&mut app, "symbol");
        press(&mut app, KeyCode::Enter);
        for _ in 0..3 {
            press(&mut app, KeyCode::Up);
        }
        assert_eq!(app.picker.as_ref().unwrap().index(), 0);
        let last = app.picker.as_ref().unwrap().candidates.len() - 1;
        for _ in 0..last + 5 {
            press(&mut app, KeyCode::Down);
        }
        assert_eq!(app.picker.as_ref().unwrap().index(), last);

        // 終了・保存・元に戻す・フォーカス移動は、一覧が開いている間は効かない
        for key in [
            KeyCode::Char('q'),
            KeyCode::Char('s'),
            KeyCode::Char('u'),
            KeyCode::Tab,
        ] {
            press(&mut app, key);
            assert!(app.picker.is_some(), "{key:?}");
            assert!(!app.should_quit, "{key:?}");
            assert_eq!(app.focus, Focus::Props, "{key:?}");
        }
    }

    #[test]
    fn test_picker_e_switches_to_free_text_input_with_the_selected_candidate() {
        let mut app = props_app("git_branch");
        select_prop(&mut app, "symbol");
        press(&mut app, KeyCode::Enter);
        press(&mut app, KeyCode::Down);
        press(&mut app, KeyCode::Char('e'));
        assert!(app.picker.is_none());
        let input = app.input.as_ref().expect("自由な入力が始まる");
        assert_eq!(input.value(), "🌱 ", "選んでいた候補から始める");
        assert_eq!(app.draft.config().git_branch.symbol, "🌿 ");
    }

    #[test]
    fn test_picker_keeps_a_custom_current_value_at_the_top() {
        let mut app = props_app("git_branch");
        app.draft
            .set_prop("git_branch", "symbol", &PropValue::from("Z"))
            .unwrap();
        select_prop(&mut app, "symbol");
        press(&mut app, KeyCode::Enter);
        let picker = app.picker.as_ref().unwrap();
        assert_eq!(picker.selected(), "Z");
        assert_eq!(picker.index(), 0);
        assert!(
            picker.candidates.iter().any(|c| c == "🌿 "),
            "既定値も選べる"
        );
    }

    #[test]
    fn test_non_symbol_rows_still_open_a_text_input_directly() {
        let mut app = props_app("git_branch");
        select_prop(&mut app, "style");
        press(&mut app, KeyCode::Enter);
        assert!(app.picker.is_none());
        assert!(app.input.is_some());
    }

    #[test]
    fn test_symbol_in_progress_follows_the_picker_and_symbol_inputs_only() {
        let mut app = props_app("git_branch");
        assert_eq!(app.symbol_in_progress(), None);
        select_prop(&mut app, "symbol");
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.symbol_in_progress().as_deref(), Some("🌿 "));
        press(&mut app, KeyCode::Char('e'));
        type_text(&mut app, "X");
        assert_eq!(app.symbol_in_progress().as_deref(), Some("🌿 X"));
        press(&mut app, KeyCode::Esc);

        // 記号でない行の入力は、対象外
        press(&mut app, KeyCode::Down);
        press(&mut app, KeyCode::Enter);
        assert!(app.input.is_some());
        assert_eq!(app.symbol_in_progress(), None);
    }
}
