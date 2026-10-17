//! 設定 TUI の状態と、キー入力による状態遷移（画面に依存しない）。

use std::cell::Cell;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::editor::diff::{self, Diff, DiffMode};
use crate::editor::draft::{Draft, PropError};
use crate::editor::history::Generation;
use crate::editor::layout::Token;
use crate::editor::layout_edit::{Direction, LayoutEditor, Placement};
use crate::editor::load::ConfigSource;
use crate::editor::save::{ConflictKind, SaveError};
use crate::editor::style_spec::{Decoration, Layer, StyleColor, StyleSpec};
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

/// スタイル編集画面の行
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StyleRow {
    Color(Layer),
    Decoration(Decoration),
}

/// スタイル編集画面の行（上から）。前景・背景の色、各装飾
pub const STYLE_ROWS: [StyleRow; 7] = [
    StyleRow::Color(Layer::Foreground),
    StyleRow::Color(Layer::Background),
    StyleRow::Decoration(Decoration::Bold),
    StyleRow::Decoration(Decoration::Dimmed),
    StyleRow::Decoration(Decoration::Italic),
    StyleRow::Decoration(Decoration::Underline),
    StyleRow::Decoration(Decoration::Inverted),
];

/// スタイル（色・装飾）のトグル選択。`Enter` で決定し、`Esc` で取り消す。選んでいる間も、プレビューに反映する
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StyleEditor {
    pub part: &'static str,
    pub key: &'static str,
    spec: StyleSpec,
    /// 開いたときの内容（変えずに決定したときは、値を書き換えない）
    original: StyleSpec,
    row: usize,
}

impl StyleEditor {
    pub fn spec(&self) -> &StyleSpec {
        &self.spec
    }

    pub fn row(&self) -> usize {
        self.row
    }
}

/// 色を巡回する。「なし → 9 色 → なし」の順。端から先は、なしを挟んで反対側へ回る
fn cycle_color(current: Option<StyleColor>, forward: bool) -> Option<StyleColor> {
    let count = StyleColor::ALL.len() + 1;
    let index = current.map_or(0, |color| {
        1 + StyleColor::ALL
            .iter()
            .position(|c| *c == color)
            .unwrap_or(0)
    });
    let next = if forward {
        (index + 1) % count
    } else {
        (index + count - 1) % count
    };
    next.checked_sub(1).map(|i| StyleColor::ALL[i])
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

/// 差分の比べる元
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffBase {
    /// 最後に保存（または読み込み）した内容
    Saved,
    /// 競合したときの、ディスク上の今の内容。`exists` が偽なら、外部で削除されている
    Disk { exists: bool },
    /// 復元の確認。今の下書きから、履歴の世代（保存した日時）へ変わる差分
    Generation { recorded_at: Option<DateTime<Utc>> },
}

/// 差分の表示。`t` で保存済みとの差分を、保存の競合のときは `d` でディスク上の内容との差分を開く。
/// 開いた時点の内容を固定して比べる（開いている間、下書きは変わらない）
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffView {
    pub base: DiffBase,
    old: String,
    new: String,
    pub mode: DiffMode,
    scroll: usize,
    /// 描画が求めた、スクロールできる上限（行数が高さを超える分）
    max_scroll: Cell<usize>,
    /// 履歴の復元の確認のとき、復元する世代の番号（`Enter` / `y` で復元する）
    restore: Option<usize>,
}

impl DiffView {
    fn new(base: DiffBase, old: String, new: String) -> Self {
        Self {
            base,
            old,
            new,
            mode: DiffMode::Changes,
            scroll: 0,
            max_scroll: Cell::new(0),
            restore: None,
        }
    }

    /// 復元の確認か（`Enter` / `y` で復元できる差分か）
    pub fn is_restore_preview(&self) -> bool {
        self.restore.is_some()
    }

    pub fn diff(&self) -> Diff {
        diff::diff(&self.old, &self.new, self.mode)
    }

    pub fn scroll(&self) -> usize {
        self.scroll
    }

    /// 描画が、行数と高さから求めた上限を伝える
    pub fn set_max_scroll(&self, max_scroll: usize) {
        self.max_scroll.set(max_scroll);
    }
}

/// 差分で `PageUp` / `PageDown` が動かす行数
const DIFF_PAGE: usize = 10;

/// 履歴の一覧。`H` で開く。各世代に、今の下書きから、その世代へ変わる行数を添える。
/// `Enter` で、その世代への差分（復元の確認）を開く
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryView {
    pub generations: Vec<Generation>,
    /// 世代ごとの、今の下書きから変わる行数（追加, 削除）
    pub changes: Vec<(usize, usize)>,
    index: usize,
    /// 設定ファイルが保存済みか（履歴がないときの説明に使う）
    pub saved_before: bool,
}

impl HistoryView {
    pub fn index(&self) -> usize {
        self.index
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
    /// スタイルのトグル選択（開いているとき）
    pub style_editor: Option<StyleEditor>,
    /// 検証結果の一覧（開いているとき）
    pub issues_view: Option<IssuesView>,
    /// 差分の表示（開いているとき）
    pub diff_view: Option<DiffView>,
    /// 履歴の一覧（開いているとき）
    pub history_view: Option<HistoryView>,
    /// 履歴の世代を復元する直前の下書き（TOML）。復元の直後に `u` で戻すために 1 件だけ持つ
    pre_restore: Option<String>,
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
            style_editor: None,
            issues_view: None,
            diff_view: None,
            history_view: None,
            pre_restore: None,
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

    /// スタイルを選んでいるとき、その仕様を確定したと仮定した設定。プレビューに使う（下書きは変えない）
    pub fn style_preview(&self) -> Option<Result<Config, PropError>> {
        let editor = self.style_editor.as_ref()?;
        let value = PropValue::Str(editor.spec.text());
        Some(self.draft.preview_prop(editor.part, editor.key, &value))
    }

    /// 確定前の入力・選択を、確定したと仮定した設定（プレビュー用）。何も入力・選択していなければ `None`
    pub fn live_preview(&self) -> Option<Result<Config, PropError>> {
        self.picker_preview()
            .or_else(|| self.style_preview())
            .or_else(|| self.input_preview())
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
        if self.style_editor.is_some() {
            self.handle_style_key(key.code);
            return;
        }
        if self.issues_view.is_some() {
            self.handle_issues_key(key.code);
            return;
        }
        if self.diff_view.is_some() {
            self.handle_diff_key(key.code);
            return;
        }
        if self.history_view.is_some() {
            self.handle_history_key(key.code);
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
            KeyCode::Char('u') => self.undo(),
            KeyCode::Char('H') => self.open_history(),
            KeyCode::Char('v') => self.issues_view = Some(IssuesView::default()),
            KeyCode::Char('t') => {
                self.diff_view = Some(DiffView::new(
                    DiffBase::Saved,
                    self.saved_toml.clone(),
                    self.draft.to_toml(),
                ));
            }
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

    /// スタイル編集画面のキー。`↑↓` で行を選び、色の行は `←→`（`Space` で次へ）で巡回、装飾の行は `Space` でオン・オフ
    /// （`←` オフ、`→` オン）。`Enter` で決定、`Esc` で取り消し、`e` で自由入力（テキスト入力）に切り替える
    fn handle_style_key(&mut self, key: KeyCode) {
        let Some(editor) = self.style_editor.as_mut() else {
            return;
        };
        let row = STYLE_ROWS[editor.row];
        match key {
            KeyCode::Up => editor.row = editor.row.saturating_sub(1),
            KeyCode::Down => editor.row = (editor.row + 1).min(STYLE_ROWS.len() - 1),
            KeyCode::Left | KeyCode::Right | KeyCode::Char(' ') => match row {
                StyleRow::Color(layer) => {
                    let color = cycle_color(editor.spec.color(layer), key != KeyCode::Left);
                    editor.spec.set_color(layer, color);
                }
                StyleRow::Decoration(decoration) => {
                    let on = match key {
                        KeyCode::Left => false,
                        KeyCode::Right => true,
                        _ => !editor.spec.has(decoration),
                    };
                    editor.spec.set_decoration(decoration, on);
                }
            },
            KeyCode::Esc => self.style_editor = None,
            KeyCode::Char('e') => {
                let (part, key, text) = (editor.part, editor.key, editor.spec.text());
                self.style_editor = None;
                self.input = Some(TextInput::prop(part, key, &text));
            }
            KeyCode::Enter => {
                // 変えていなければ、値を書き換えない（元の表記を保つ）
                if editor.spec == editor.original {
                    self.style_editor = None;
                    return;
                }
                let value = PropValue::Str(editor.spec.text());
                // 受け付けられないときは、開いたままにして、理由を示す
                match self.draft.set_prop(editor.part, editor.key, &value) {
                    Ok(()) => self.style_editor = None,
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

    /// 差分のキー。`↑↓` / `PageUp` / `PageDown` / `Home` / `End` でスクロールし、`f` で変更箇所と全文を切り替え、
    /// `Esc` / `t` / `d` で閉じる
    fn handle_diff_key(&mut self, key: KeyCode) {
        let Some(view) = self.diff_view.as_mut() else {
            return;
        };
        let max = view.max_scroll.get();
        match key {
            KeyCode::Up => view.scroll = view.scroll.saturating_sub(1),
            KeyCode::Down => view.scroll = (view.scroll + 1).min(max),
            KeyCode::PageUp => view.scroll = view.scroll.saturating_sub(DIFF_PAGE),
            KeyCode::PageDown => view.scroll = (view.scroll + DIFF_PAGE).min(max),
            KeyCode::Home => view.scroll = 0,
            KeyCode::End => view.scroll = max,
            KeyCode::Char('f') => {
                view.mode = view.mode.toggled();
                view.scroll = 0;
            }
            KeyCode::Enter | KeyCode::Char('y') if view.restore.is_some() => self.restore(),
            KeyCode::Esc | KeyCode::Char('t' | 'd') => self.diff_view = None,
            _ => {}
        }
    }

    /// 元に戻す。履歴の世代を復元した直後（その後の編集がまだない）なら、復元前の下書きに戻す。
    /// それ以外は、配置・プロパティの編集を 1 件ずつ戻す
    fn undo(&mut self) {
        if !self.draft.can_undo()
            && let Some(previous) = self.pre_restore.take()
        {
            match Draft::from_text(&previous) {
                Ok(draft) => {
                    self.draft = draft;
                    self.notice =
                        Some("復元を取り消しました（復元前の下書きに戻しました）".to_string());
                }
                Err(err) => {
                    self.message = Some(format!("復元前の下書きに戻せません: {err}"));
                    self.pre_restore = Some(previous);
                }
            }
            return;
        }
        self.draft.undo();
    }

    /// 履歴の一覧を開く。各世代に、今の下書きから変わる行数を添える。読めないときは、理由を示す
    fn open_history(&mut self) {
        match self.storage.history(&self.source) {
            Ok(generations) => {
                let current = self.draft.to_toml();
                let changes = generations
                    .iter()
                    .map(|generation| {
                        let diff = diff::diff(&current, &generation.text, DiffMode::Changes);
                        (diff.added, diff.removed)
                    })
                    .collect();
                self.history_view = Some(HistoryView {
                    generations,
                    changes,
                    index: 0,
                    saved_before: matches!(self.source, ConfigSource::Existing { .. }),
                });
            }
            Err(err) => self.message = Some(format!("履歴を読めません: {err}")),
        }
    }

    /// 履歴の一覧のキー。`↑↓` で選び、`Enter` で復元の確認（差分）を開き、`Esc` / `H` で閉じる
    fn handle_history_key(&mut self, key: KeyCode) {
        let current = self.draft.to_toml();
        let Some(view) = self.history_view.as_mut() else {
            return;
        };
        let last = view.generations.len().saturating_sub(1);
        match key {
            KeyCode::Up => view.index = view.index.saturating_sub(1),
            KeyCode::Down => view.index = (view.index + 1).min(last),
            KeyCode::Enter => {
                if let Some(generation) = view.generations.get(view.index) {
                    let mut diff_view = DiffView::new(
                        DiffBase::Generation {
                            recorded_at: generation.recorded_at,
                        },
                        current,
                        generation.text.clone(),
                    );
                    diff_view.restore = Some(view.index);
                    self.diff_view = Some(diff_view);
                }
            }
            KeyCode::Esc | KeyCode::Char('H') => self.history_view = None,
            _ => {}
        }
    }

    /// 復元の確認の差分で選んだ世代を、下書きに復元する。保存はしない（`s`）。
    /// 世代が読めない内容のときは、理由を示して、差分を開いたままにする
    fn restore(&mut self) {
        let Some(index) = self.diff_view.as_ref().and_then(|view| view.restore) else {
            return;
        };
        let Some(generation) = self
            .history_view
            .as_ref()
            .and_then(|view| view.generations.get(index))
        else {
            return;
        };
        let restored = match Draft::from_text(&generation.text) {
            Ok(draft) => draft,
            Err(err) => {
                self.message = Some(format!("この世代は復元できません: {err}"));
                return;
            }
        };
        let before = self.draft.to_toml();
        self.diff_view = None;
        self.history_view = None;
        if restored.to_toml() == before {
            self.notice = Some("今の下書きと同じなので、復元しませんでした".to_string());
            return;
        }
        self.pre_restore = Some(before);
        self.draft = restored;
        self.notice = Some(
            "世代を下書きに復元しました（まだ保存していません: s で保存、u で復元前に戻す）"
                .to_string(),
        );
    }

    /// 保存の競合のとき、ディスク上の今の内容と下書きの差分を開く。
    /// 読めないときは、理由を示して競合の選択に戻る
    fn open_disk_diff(&mut self) {
        match self.storage.read(self.source_path()) {
            Ok(disk) => {
                let exists = disk.is_some();
                self.diff_view = Some(DiffView::new(
                    DiffBase::Disk { exists },
                    disk.unwrap_or_default(),
                    self.draft.to_toml(),
                ));
            }
            Err(err) => {
                self.message = Some(format!("ディスク上の内容を読めません: {err}"));
            }
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
            // 差分を見るときは、競合の選択を残す（差分を閉じると、選択に戻る）
            (Mode::Conflict(kind), KeyCode::Char('d')) => {
                self.mode = Mode::Conflict(kind);
                self.open_disk_diff();
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
                // 読み込み直した下書きに、復元前の下書きへ戻す道は残さない
                self.pre_restore = None;
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
            // スタイルは、色と装飾のトグルで選ぶ。自由に入力するには、編集画面で `e`
            (PropKind::Style, PropValue::Str(value)) if key == KeyCode::Enter => {
                let spec = StyleSpec::parse(&value);
                self.style_editor = Some(StyleEditor {
                    part: part.id,
                    key: prop.key,
                    original: spec.clone(),
                    spec,
                    row: 0,
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
    use crate::editor::diff::LineKind;
    use crate::editor::history::Generation;
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
        press(&mut app, KeyCode::Char('e')); // スタイルは、編集画面から自由入力に切り替える
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

    fn diff_lines(app: &App) -> Vec<(LineKind, String)> {
        app.diff_view
            .as_ref()
            .expect("差分が開いている")
            .diff()
            .lines
            .into_iter()
            .map(|line| (line.kind, line.text))
            .collect()
    }

    #[test]
    fn test_t_opens_the_diff_against_the_saved_text_and_closes_it() {
        let (mut app, _storage) = app_with_storage("$model");
        press(&mut app, KeyCode::Char('t'));
        let view = app.diff_view.as_ref().expect("開く");
        assert_eq!(view.base, DiffBase::Saved);
        assert!(!view.diff().has_changes(), "変更がなければ、差分は空");

        // 閉じて、配置を変えてから開き直すと、保存済みとの差が出る
        press(&mut app, KeyCode::Esc);
        assert_eq!(app.diff_view, None);
        assert!(!app.should_quit, "Esc は、差分を閉じるだけ");
        make_dirty(&mut app);
        press(&mut app, KeyCode::Char('t'));
        let diff = app.diff_view.as_ref().unwrap().diff();
        assert!(diff.has_changes());
        assert!(
            diff_lines(&app)
                .iter()
                .any(|(kind, _)| *kind == LineKind::Added),
            "{:?}",
            diff_lines(&app)
        );
        // `t` でも閉じる
        press(&mut app, KeyCode::Char('t'));
        assert_eq!(app.diff_view, None);
    }

    #[test]
    fn test_saving_resets_the_diff_baseline() {
        let (mut app, _storage) = app_with_storage("$model");
        make_dirty(&mut app);
        press(&mut app, KeyCode::Char('s'));
        assert!(!app.is_dirty());
        press(&mut app, KeyCode::Char('t'));
        assert!(!app.diff_view.as_ref().unwrap().diff().has_changes());
    }

    #[test]
    fn test_diff_view_keys_scroll_toggle_the_mode_and_swallow_other_keys() {
        let (mut app, _storage) = app_with_storage("$model");
        make_dirty(&mut app);
        press(&mut app, KeyCode::Char('t'));
        assert_eq!(app.diff_view.as_ref().unwrap().mode, DiffMode::Changes);
        press(&mut app, KeyCode::Char('f'));
        assert_eq!(app.diff_view.as_ref().unwrap().mode, DiffMode::Full);
        press(&mut app, KeyCode::Char('f'));
        assert_eq!(app.diff_view.as_ref().unwrap().mode, DiffMode::Changes);

        // 描画がスクロールの上限を伝えるまで（まだ描画していない）、スクロールは 0 のまま
        for key in [KeyCode::Down, KeyCode::PageDown, KeyCode::End] {
            press(&mut app, key);
            assert_eq!(app.diff_view.as_ref().unwrap().scroll(), 0);
        }
        // 上限が伝わると、その範囲で動く
        app.diff_view.as_ref().unwrap().set_max_scroll(25);
        press(&mut app, KeyCode::Down);
        assert_eq!(app.diff_view.as_ref().unwrap().scroll(), 1);
        press(&mut app, KeyCode::PageDown);
        assert_eq!(app.diff_view.as_ref().unwrap().scroll(), 11);
        press(&mut app, KeyCode::End);
        assert_eq!(app.diff_view.as_ref().unwrap().scroll(), 25);
        press(&mut app, KeyCode::Down);
        assert_eq!(app.diff_view.as_ref().unwrap().scroll(), 25, "上限で止まる");
        press(&mut app, KeyCode::PageUp);
        assert_eq!(app.diff_view.as_ref().unwrap().scroll(), 15);
        press(&mut app, KeyCode::Home);
        assert_eq!(app.diff_view.as_ref().unwrap().scroll(), 0);
        press(&mut app, KeyCode::Up);
        assert_eq!(app.diff_view.as_ref().unwrap().scroll(), 0);
        // 表示の形を切り替えると、先頭に戻る
        press(&mut app, KeyCode::End);
        press(&mut app, KeyCode::Char('f'));
        assert_eq!(app.diff_view.as_ref().unwrap().scroll(), 0);

        // 開いている間は、`q` `s` `u` `v` が効かない（終了も保存も元に戻すも検証結果も開かない）
        let saved_before = app.is_dirty();
        for key in [
            KeyCode::Char('q'),
            KeyCode::Char('s'),
            KeyCode::Char('u'),
            KeyCode::Char('v'),
        ] {
            press(&mut app, key);
            assert!(app.diff_view.is_some());
            assert!(!app.should_quit);
            assert_eq!(app.issues_view, None);
        }
        assert_eq!(app.is_dirty(), saved_before);
    }

    #[test]
    fn test_t_is_a_character_while_typing() {
        let mut app = props_app("model");
        select_prop(&mut app, "symbol");
        press(&mut app, KeyCode::Enter);
        press(&mut app, KeyCode::Char('e'));
        press(&mut app, KeyCode::Char('t'));
        assert_eq!(app.diff_view, None);
        assert!(app.input.as_ref().unwrap().value().ends_with('t'));
    }

    #[test]
    fn test_conflict_d_shows_the_disk_content_and_returns_to_the_choices() {
        let (mut app, storage) = app_with_storage("$model");
        make_dirty(&mut app);
        {
            let mut state = storage.0.borrow_mut();
            state.save_error = Some(SaveError::Conflict(ConflictKind::Changed));
            state.disk = Some("format = \"$plan\"\n".to_string());
        }
        press(&mut app, KeyCode::Char('s'));
        assert_eq!(app.mode, Mode::Conflict(ConflictKind::Changed));

        press(&mut app, KeyCode::Char('d'));
        assert_eq!(
            app.mode,
            Mode::Conflict(ConflictKind::Changed),
            "選択は残る"
        );
        let view = app.diff_view.as_ref().expect("差分が開く");
        assert_eq!(view.base, DiffBase::Disk { exists: true });
        let lines = diff_lines(&app);
        // ディスク上にだけある行は `-`、下書きにある行は `+`
        assert!(
            lines.contains(&(LineKind::Removed, "format = \"$plan\"".to_string())),
            "{lines:?}"
        );
        assert!(
            lines
                .iter()
                .any(|(kind, text)| *kind == LineKind::Added && text.contains("model")),
            "{lines:?}"
        );
        assert_eq!(storage.0.borrow().reads, [PathBuf::from("test.toml")]);

        // 閉じると、競合の選択に戻る（まだ何も失っていない）。続けて `r` で再読み込みできる
        press(&mut app, KeyCode::Esc);
        assert_eq!(app.diff_view, None);
        assert_eq!(app.mode, Mode::Conflict(ConflictKind::Changed));
        assert!(app.is_dirty());
        storage.0.borrow_mut().reload_to = Some(loaded("format = \"$plan\"\n"));
        press(&mut app, KeyCode::Char('r'));
        assert_eq!(app.mode, Mode::Normal);
        assert_eq!(format(&app), "$plan");
    }

    #[test]
    fn test_conflict_d_when_the_file_was_deleted_or_cannot_be_read() {
        let (mut app, storage) = app_with_storage("$model");
        make_dirty(&mut app);
        storage.0.borrow_mut().save_error = Some(SaveError::Conflict(ConflictKind::Deleted));
        press(&mut app, KeyCode::Char('s'));

        // ディスクに無い: 比べる元は空で、下書きの全行が `+`
        press(&mut app, KeyCode::Char('d'));
        assert_eq!(
            app.diff_view.as_ref().unwrap().base,
            DiffBase::Disk { exists: false }
        );
        let lines = diff_lines(&app);
        assert!(lines.iter().all(|(kind, _)| *kind != LineKind::Removed));
        assert!(lines.iter().any(|(kind, _)| *kind == LineKind::Added));
        press(&mut app, KeyCode::Char('d'));
        assert_eq!(app.diff_view, None);
        assert_eq!(app.mode, Mode::Conflict(ConflictKind::Deleted));

        // 読めない: 理由を示して、差分は開かず、競合の選択に残る
        storage.0.borrow_mut().read_error = Some("permission denied".to_string());
        press(&mut app, KeyCode::Char('d'));
        assert_eq!(app.diff_view, None);
        assert_eq!(app.mode, Mode::Conflict(ConflictKind::Deleted));
        let message = app.message.clone().expect("理由が示される");
        assert!(
            message.contains("ディスク上の内容を読めません"),
            "{message}"
        );
        assert!(message.contains("permission denied"), "{message}");
    }

    fn generation(secs: i64, format: &str) -> Generation {
        use chrono::TimeZone;
        Generation {
            recorded_at: Some(chrono::Utc.timestamp_opt(1_800_000_000 + secs, 0).unwrap()),
            text: format!("format = \"{format}\"\n"),
        }
    }

    /// 保存済み（`Existing`）の状態にして、履歴の世代を差し込む
    fn app_with_history(format: &str, generations: Vec<Generation>) -> (App, FakeStorage) {
        let (mut app, storage) = app_with_storage(format);
        make_dirty(&mut app);
        press(&mut app, KeyCode::Char('s'));
        assert!(!app.is_dirty());
        storage.0.borrow_mut().generations = generations;
        (app, storage)
    }

    #[test]
    fn test_h_opens_the_history_with_change_counts_and_closes_it() {
        let (mut app, _storage) = app_with_history(
            "$model",
            vec![generation(2, "$plan"), generation(1, "$model")],
        );
        press(&mut app, KeyCode::Char('H'));
        let view = app.history_view.as_ref().expect("開く");
        assert_eq!(view.generations.len(), 2);
        assert!(view.saved_before);
        // 今の下書きから各世代へ変わる行数。`$model` の世代は、今の下書きと `format` の書き方だけが違う
        assert!(
            view.changes[0].0 + view.changes[0].1 > 0,
            "{:?}",
            view.changes
        );
        assert_eq!(view.index(), 0);

        press(&mut app, KeyCode::Down);
        press(&mut app, KeyCode::Down);
        assert_eq!(
            app.history_view.as_ref().unwrap().index(),
            1,
            "末尾で止まる"
        );
        press(&mut app, KeyCode::Up);
        press(&mut app, KeyCode::Up);
        assert_eq!(app.history_view.as_ref().unwrap().index(), 0);

        // 開いている間は、`q` `s` `u` が効かない
        for key in [KeyCode::Char('q'), KeyCode::Char('s'), KeyCode::Char('u')] {
            press(&mut app, key);
            assert!(!app.should_quit);
            assert!(app.history_view.is_some());
        }
        press(&mut app, KeyCode::Esc);
        assert_eq!(app.history_view, None);
        assert!(!app.should_quit);
        press(&mut app, KeyCode::Char('H'));
        press(&mut app, KeyCode::Char('H'));
        assert_eq!(app.history_view, None, "H でも閉じる");
    }

    #[test]
    fn test_h_without_history_or_with_unreadable_history() {
        // まだ保存されていない設定ファイル: 履歴はない（説明が変わる）
        let (mut app, _storage) = app_with_storage("$model");
        press(&mut app, KeyCode::Char('H'));
        let view = app.history_view.as_ref().expect("開く");
        assert_eq!(view.generations, Vec::new());
        assert!(!view.saved_before);
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.diff_view, None, "世代がなければ、差分は開かない");
        press(&mut app, KeyCode::Esc);

        // 履歴を読めない: 理由を示して、開かない
        let (mut app, storage) = app_with_history("$model", vec![generation(1, "$plan")]);
        storage.0.borrow_mut().history_error = Some("permission denied".to_string());
        press(&mut app, KeyCode::Char('H'));
        assert_eq!(app.history_view, None);
        let message = app.message.clone().expect("理由が示される");
        assert!(message.contains("履歴を読めません"), "{message}");
        assert!(message.contains("permission denied"), "{message}");
    }

    #[test]
    fn test_history_enter_shows_the_diff_to_that_generation_and_esc_returns_to_the_list() {
        let (mut app, _storage) = app_with_history(
            "$model",
            vec![generation(2, "$plan"), generation(1, "$quota")],
        );
        press(&mut app, KeyCode::Char('H'));
        press(&mut app, KeyCode::Down);
        press(&mut app, KeyCode::Enter);
        let view = app.diff_view.as_ref().expect("復元の確認の差分");
        assert!(view.is_restore_preview());
        let DiffBase::Generation { recorded_at } = view.base else {
            panic!("世代との差分: {:?}", view.base);
        };
        assert_eq!(
            recorded_at,
            app.history_view.as_ref().unwrap().generations[1].recorded_at
        );
        // 今の下書きから、その世代へ: 世代にだけある行が `+`
        let lines = diff_lines(&app);
        assert!(
            lines
                .iter()
                .any(|(kind, text)| *kind == LineKind::Added && text.contains("quota")),
            "{lines:?}"
        );
        // 差分を閉じても、復元はされない。一覧に戻る
        press(&mut app, KeyCode::Esc);
        assert_eq!(app.diff_view, None);
        assert!(app.history_view.is_some());
        assert_eq!(format(&app), "$model\n");
    }

    #[test]
    fn test_restore_replaces_the_draft_and_u_undoes_it_right_after() {
        let (mut app, _storage) = app_with_history("$model", vec![generation(1, "$plan")]);
        press(&mut app, KeyCode::Char('H'));
        press(&mut app, KeyCode::Enter);
        press(&mut app, KeyCode::Enter); // 差分の画面で復元
        assert_eq!(app.diff_view, None);
        assert_eq!(app.history_view, None);
        assert_eq!(format(&app), "$plan", "世代の内容になる");
        assert!(app.is_dirty(), "保存はしない");
        assert!(app.notice.clone().unwrap().contains("復元しました"));

        // 復元の直後なら、`u` で復元前の下書きに戻る
        press(&mut app, KeyCode::Char('u'));
        assert_eq!(format(&app), "$model\n");
        assert!(app.notice.clone().unwrap().contains("復元を取り消しました"));
        // 戻した後の `u` は、もう何もしない（履歴がない）
        press(&mut app, KeyCode::Char('u'));
        assert_eq!(format(&app), "$model\n");
    }

    #[test]
    fn test_u_after_edits_following_a_restore_undoes_the_edits_first() {
        let (mut app, _storage) = app_with_history("$model", vec![generation(1, "$plan")]);
        press(&mut app, KeyCode::Char('H'));
        press(&mut app, KeyCode::Enter);
        press(&mut app, KeyCode::Char('y')); // y でも復元できる
        assert_eq!(format(&app), "$plan");

        // 復元の後の編集は、これまでどおり 1 件ずつ戻せる。戻し終えると、復元自体を取り消せる
        press(&mut app, KeyCode::Char('a'));
        assert!(app.draft.can_undo());
        press(&mut app, KeyCode::Char('u'));
        assert_eq!(format(&app), "$plan", "まず、復元後の編集");
        assert!(!app.draft.can_undo());
        press(&mut app, KeyCode::Char('u'));
        assert_eq!(format(&app), "$model\n", "次に、復元そのもの");
    }

    #[test]
    fn test_restore_of_an_identical_generation_does_nothing() {
        let (mut app, _storage) = app_with_history("$model", vec![]);
        let same = Generation {
            recorded_at: None,
            text: app.draft.to_toml(),
        };
        _storage.0.borrow_mut().generations = vec![same];
        press(&mut app, KeyCode::Char('H'));
        press(&mut app, KeyCode::Enter);
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.diff_view, None);
        assert!(app.notice.clone().unwrap().contains("同じなので"));
        // 復元していないので、`u` は復元の取り消しではなく、ふつうの元に戻す（直前の配置の編集を戻す）
        press(&mut app, KeyCode::Char('u'));
        assert_eq!(format(&app), "$model");
        assert!(
            app.notice
                .as_ref()
                .is_none_or(|notice| !notice.contains("復元を取り消し"))
        );
    }

    #[test]
    fn test_restore_of_an_unreadable_generation_is_refused_and_keeps_the_diff_open() {
        let broken = Generation {
            recorded_at: None,
            text: "format = \n".to_string(),
        };
        let (mut app, _storage) = app_with_history("$model", vec![broken]);
        press(&mut app, KeyCode::Char('H'));
        press(&mut app, KeyCode::Enter);
        press(&mut app, KeyCode::Enter);
        let message = app.message.clone().expect("理由が示される");
        assert!(message.contains("この世代は復元できません"), "{message}");
        assert!(app.diff_view.is_some(), "差分は開いたまま");
        assert!(app.history_view.is_some());
        assert_eq!(format(&app), "$model\n", "下書きは変わらない");
    }

    #[test]
    fn test_reload_forgets_the_pre_restore_draft() {
        let (mut app, storage) = app_with_history("$model", vec![generation(1, "$plan")]);
        press(&mut app, KeyCode::Char('H'));
        press(&mut app, KeyCode::Enter);
        press(&mut app, KeyCode::Enter);
        assert_eq!(format(&app), "$plan");
        {
            let mut state = storage.0.borrow_mut();
            state.save_error = Some(SaveError::Conflict(ConflictKind::Changed));
            state.reload_to = Some(loaded("format = \"$quota\"\n"));
        }
        press(&mut app, KeyCode::Char('s'));
        press(&mut app, KeyCode::Char('r'));
        assert_eq!(format(&app), "$quota");
        // 読み込み直した下書きに、復元前の下書きへ戻る道は残らない
        press(&mut app, KeyCode::Char('u'));
        assert_eq!(format(&app), "$quota");
    }

    #[test]
    fn test_h_is_a_character_while_typing() {
        let mut app = props_app("model");
        select_prop(&mut app, "symbol");
        press(&mut app, KeyCode::Enter);
        press(&mut app, KeyCode::Char('e'));
        press(&mut app, KeyCode::Char('H'));
        assert_eq!(app.history_view, None);
        assert!(app.input.as_ref().unwrap().value().ends_with('H'));
    }

    #[test]
    fn test_d_still_deletes_in_the_layout_when_there_is_no_conflict() {
        let (mut app, _storage) = app_with_storage("$model $plan");
        press(&mut app, KeyCode::Char('d'));
        assert_eq!(app.diff_view, None);
        assert_eq!(format(&app), " $plan", "d は配置の削除のまま");
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

    /// `directory.folder_style`（既定は `bold cyan`）の行にいて、`Enter` でスタイル編集画面を開く
    fn style_editor_app() -> App {
        let mut app = props_app("directory");
        select_prop(&mut app, "folder_style");
        press(&mut app, KeyCode::Enter);
        assert!(app.style_editor.is_some());
        app
    }

    fn style_text(app: &App) -> String {
        app.style_editor.as_ref().expect("開いている").spec().text()
    }

    /// 設定の TOML を読み込んだ状態で、配置のカーソル（先頭の部品）のプロパティ欄にいるアプリ
    fn props_app_from_toml(toml: &str) -> App {
        let config = toml_edit::de::from_str::<Config>(toml).unwrap();
        let mut app = App::for_test(Draft::new(config, toml.parse().unwrap()));
        press(&mut app, KeyCode::Tab);
        assert_eq!(app.focus, Focus::Props);
        app
    }

    #[test]
    fn test_enter_on_a_style_row_opens_the_toggle_editor_and_e_switches_to_free_text() {
        let mut app = style_editor_app();
        let editor = app.style_editor.as_ref().unwrap();
        assert_eq!((editor.part, editor.key), ("directory", "folder_style"));
        assert_eq!(editor.row(), 0);
        assert_eq!(style_text(&app), "bold cyan");
        assert!(app.input.is_none() && app.picker.is_none());

        press(&mut app, KeyCode::Char('e'));
        assert!(app.style_editor.is_none());
        assert_eq!(app.input.as_ref().unwrap().value(), "bold cyan");
    }

    #[test]
    fn test_style_editor_cycles_colors_and_toggles_decorations() {
        let mut app = style_editor_app();
        // 前景: cyan → white → gray → なし → black → ...（巡回。なしを挟む）
        for expected in ["bold white", "bold gray", "bold", "bold black"] {
            press(&mut app, KeyCode::Right);
            assert_eq!(style_text(&app), expected);
        }
        for expected in ["bold", "bold gray", "bold white", "bold cyan"] {
            press(&mut app, KeyCode::Left);
            assert_eq!(style_text(&app), expected);
        }
        // Space は、色の行では次の色へ
        press(&mut app, KeyCode::Char(' '));
        assert_eq!(style_text(&app), "bold white");
        press(&mut app, KeyCode::Left);

        // 背景: 行を移して、色を足す。前景の後ろに入る
        press(&mut app, KeyCode::Down);
        press(&mut app, KeyCode::Right);
        assert_eq!(style_text(&app), "bold cyan bg:black");
        // bold の行: Space でオフ、オン。← でオフ、→ でオン
        press(&mut app, KeyCode::Down);
        press(&mut app, KeyCode::Char(' '));
        assert_eq!(style_text(&app), "cyan bg:black");
        press(&mut app, KeyCode::Char(' '));
        assert_eq!(style_text(&app), "bold cyan bg:black");
        press(&mut app, KeyCode::Left);
        assert_eq!(style_text(&app), "cyan bg:black");
        press(&mut app, KeyCode::Right);
        press(&mut app, KeyCode::Right);
        assert_eq!(style_text(&app), "bold cyan bg:black", "オンのまま");
        // 別の装飾を足すと、色の前に入る
        press(&mut app, KeyCode::Down);
        press(&mut app, KeyCode::Right);
        assert_eq!(style_text(&app), "bold dimmed cyan bg:black");

        // 行の端で止まる
        for _ in 0..20 {
            press(&mut app, KeyCode::Down);
        }
        assert_eq!(
            app.style_editor.as_ref().unwrap().row(),
            STYLE_ROWS.len() - 1
        );
        for _ in 0..20 {
            press(&mut app, KeyCode::Up);
        }
        assert_eq!(app.style_editor.as_ref().unwrap().row(), 0);
    }

    #[test]
    fn test_style_editor_previews_live_and_enter_commits() {
        let mut app = style_editor_app();
        press(&mut app, KeyCode::Left); // cyan → magenta
        press(&mut app, KeyCode::Left); // → blue
        // 決定前は、プレビューだけが変わり、下書きは変わらない
        let preview = app
            .style_preview()
            .expect("選んでいる間のプレビュー")
            .unwrap();
        assert_eq!(preview.directory.folder_style, "bold blue");
        assert_eq!(
            app.live_preview().unwrap().unwrap().directory.folder_style,
            "bold blue"
        );
        assert_eq!(
            app.draft.prop_value("directory", "folder_style"),
            Some(PropValue::Str("bold cyan".to_string()))
        );
        assert!(!app.draft.can_undo());

        press(&mut app, KeyCode::Enter);
        assert!(app.style_editor.is_none());
        assert_eq!(
            app.draft.prop_value("directory", "folder_style"),
            Some(PropValue::Str("bold blue".to_string()))
        );
        assert!(app.draft.can_undo(), "ふつうの編集として、元に戻せる");
        assert!(app.style_preview().is_none());
    }

    #[test]
    fn test_style_editor_esc_cancels_and_enter_without_changes_keeps_the_original_text() {
        let toml = "[directory]\nfolder_style = \"bold   purple\" # 色\n";
        let mut app = props_app_from_toml(toml);
        select_prop(&mut app, "folder_style");

        // 変えずに決定: 値も、空白や行末のコメントも、そのまま（履歴にも積まない）
        press(&mut app, KeyCode::Enter);
        press(&mut app, KeyCode::Enter);
        assert!(app.style_editor.is_none());
        assert!(!app.draft.can_undo());
        assert_eq!(app.draft.to_toml(), toml);

        // 変えて Esc: 取り消し。プレビューも戻る
        press(&mut app, KeyCode::Enter);
        press(&mut app, KeyCode::Right);
        assert_ne!(
            app.style_preview().unwrap().unwrap().directory.folder_style,
            "bold   purple"
        );
        press(&mut app, KeyCode::Esc);
        assert!(app.style_editor.is_none());
        assert!(app.style_preview().is_none());
        assert_eq!(app.draft.to_toml(), toml);
    }

    #[test]
    fn test_style_editor_keeps_tokens_it_did_not_touch() {
        // 別名（purple）、前景色が複数（purple と red）、認識できないトークン（cyna）
        let toml = "[directory]\nfolder_style = \"cyna bold purple red\"\n";
        let mut app = props_app_from_toml(toml);
        select_prop(&mut app, "folder_style");
        press(&mut app, KeyCode::Enter);
        // bold の行（3 行目）だけ触る: 前景色の別名・複数・不明なトークンは、そのまま
        press(&mut app, KeyCode::Down);
        press(&mut app, KeyCode::Down);
        press(&mut app, KeyCode::Char(' '));
        assert_eq!(style_text(&app), "cyna purple red");
        press(&mut app, KeyCode::Enter);
        assert_eq!(
            app.draft.prop_value("directory", "folder_style"),
            Some(PropValue::Str("cyna purple red".to_string()))
        );

        // 前景の行を触ると、前景色だけが 1 つにまとまる（不明なトークンは残る）
        press(&mut app, KeyCode::Enter);
        press(&mut app, KeyCode::Right);
        let text = style_text(&app);
        assert!(text.starts_with("cyna "), "{text}");
        assert_eq!(
            text.matches("purple").count() + text.matches("red").count(),
            0,
            "{text}"
        );
    }

    #[test]
    fn test_style_editor_swallows_other_keys() {
        let mut app = style_editor_app();
        for key in [
            KeyCode::Char('q'),
            KeyCode::Char('s'),
            KeyCode::Char('u'),
            KeyCode::Char('v'),
            KeyCode::Char('t'),
            KeyCode::Char('H'),
            KeyCode::Tab,
        ] {
            press(&mut app, key);
            assert!(app.style_editor.is_some());
            assert!(!app.should_quit);
            assert_eq!(
                (app.issues_view.is_some(), app.diff_view.is_some()),
                (false, false)
            );
        }
        assert_eq!(style_text(&app), "bold cyan", "値は変わらない");
    }

    #[test]
    fn test_non_symbol_rows_still_open_a_text_input_directly() {
        let mut app = props_app("git_branch");
        select_prop(&mut app, "format");
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
        press(&mut app, KeyCode::Up);
        press(&mut app, KeyCode::Enter);
        assert!(app.input.is_some());
        assert_eq!(app.symbol_in_progress(), None);
    }
}
