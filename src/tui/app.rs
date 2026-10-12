//! 設定 TUI の状態と、キー入力による状態遷移（画面に依存しない）。

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::editor::layout::Layout;
use crate::editor::layout_edit::{Direction, LayoutEditor, Placement};
use crate::preview::scenario::Scenario;
use crate::preview::width::Overflow;
use crate::tui::palette::{self, PaletteItem};

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

/// キー入力の宛先。`Tab` で、配置 → 部品一覧 → プレビューの順に切り替える
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Preview,
    Layout,
    Parts,
}

impl Focus {
    fn next(self) -> Self {
        match self {
            Self::Layout => Self::Parts,
            Self::Parts => Self::Preview,
            Self::Preview => Self::Layout,
        }
    }

    fn previous(self) -> Self {
        match self {
            Self::Layout => Self::Preview,
            Self::Parts => Self::Layout,
            Self::Preview => Self::Parts,
        }
    }
}

#[derive(Debug)]
pub struct App {
    pub focus: Focus,
    pub editor: LayoutEditor,
    pub palette: Vec<PaletteItem>,
    palette_index: usize,
    scenario_index: usize,
    width_index: usize,
    pub overflow: Overflow,
    pub should_quit: bool,
}

impl App {
    pub fn new(layout: Layout) -> Self {
        Self {
            focus: Focus::Layout,
            editor: LayoutEditor::new(layout),
            palette: palette::items(),
            palette_index: 0,
            scenario_index: 0,
            width_index: 0,
            overflow: Overflow::Truncate,
            should_quit: false,
        }
    }

    pub fn palette_index(&self) -> usize {
        self.palette_index
    }

    pub fn selected_item(&self) -> PaletteItem {
        self.palette[self.palette_index]
    }

    /// 部品一覧にフォーカスがあるとき、選択中の項目を、配置のカーソルの後ろに仮に入れた状態。
    /// 確定（`Enter`）するまで、`editor` は変わらない
    pub fn tentative_editor(&self) -> Option<LayoutEditor> {
        if self.focus != Focus::Parts {
            return None;
        }
        let mut editor = self.editor.without_history();
        palette::apply(self.selected_item(), &mut editor, Placement::After);
        Some(editor)
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

    pub fn handle_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => self.should_quit = true,
            KeyCode::Tab => self.focus = self.focus.next(),
            KeyCode::BackTab => self.focus = self.focus.previous(),
            KeyCode::Char('u') => self.editor.undo(),
            code => match self.focus {
                Focus::Preview => self.handle_preview_key(code),
                Focus::Layout => self.handle_layout_key(code, key.modifiers),
                Focus::Parts => self.handle_parts_key(code),
            },
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

    /// `Enter` は選択中のトークンの後ろへ、`i` は前へ追加する
    fn handle_parts_key(&mut self, key: KeyCode) {
        match key {
            KeyCode::Up => self.palette_index = self.palette_index.saturating_sub(1),
            KeyCode::Down => {
                self.palette_index = (self.palette_index + 1).min(self.palette.len() - 1);
            }
            KeyCode::Enter => {
                palette::apply(self.selected_item(), &mut self.editor, Placement::After);
            }
            KeyCode::Char('i') => {
                palette::apply(self.selected_item(), &mut self.editor, Placement::Before);
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
                self.editor.move_selected(direction);
            } else {
                self.editor.select(direction);
            }
            return;
        }
        match key {
            KeyCode::Char('d') | KeyCode::Delete => self.editor.delete_selected(),
            KeyCode::Char('n') => self.editor.break_line_before(),
            KeyCode::Char('a') => self.editor.add_line(),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::layout::Token;
    use crate::editor::layout_edit::Cursor;

    fn app(format: &str) -> App {
        App::new(Layout::parse(format))
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

    #[test]
    fn test_starts_on_layout() {
        assert_eq!(app("$model").focus, Focus::Layout);
    }

    #[test]
    fn test_tab_cycles_focus_and_routes_keys() {
        let mut app = app("$directory $model");
        // 配置にフォーカスがあるとき、矢印は選択を動かし、シナリオは変わらない
        press(&mut app, KeyCode::Right);
        assert_eq!(app.editor.cursor(), Cursor { line: 0, index: 1 });
        assert_eq!(app.scenario(), Scenario::ALL[0]);

        // Tab: 配置 → 部品一覧 → プレビュー → 配置
        for expected in [Focus::Parts, Focus::Preview, Focus::Layout] {
            press(&mut app, KeyCode::Tab);
            assert_eq!(app.focus, expected);
        }
        // BackTab は逆順
        for expected in [Focus::Preview, Focus::Parts, Focus::Layout] {
            press(&mut app, KeyCode::BackTab);
            assert_eq!(app.focus, expected);
        }

        // プレビューにフォーカスがあるとき、矢印はシナリオを動かし、選択は変わらない
        app.focus = Focus::Preview;
        press(&mut app, KeyCode::Right);
        assert_eq!(app.scenario(), Scenario::ALL[1]);
        press(&mut app, KeyCode::Left);
        assert_eq!(app.editor.cursor(), Cursor { line: 0, index: 1 });
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
        for focus in [Focus::Preview, Focus::Parts] {
            let mut app = app_with_focus("$directory $model", focus);
            for key in [KeyCode::Char('d'), KeyCode::Char('n'), KeyCode::Char('a')] {
                press(&mut app, key);
            }
            assert_eq!(
                app.editor.layout().to_format(),
                "$directory $model",
                "{focus:?}"
            );
            assert!(!app.editor.can_undo(), "{focus:?}");
        }
    }

    /// 部品一覧の `target` の項目まで、`Down` で進める
    fn select_item(app: &mut App, target: PaletteItem) {
        while app.selected_item() != target {
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
        assert_eq!(app.selected_item(), PaletteItem::LineBreak);
    }

    #[test]
    fn test_parts_enter_inserts_after_and_i_inserts_before() {
        let mut app = app_with_focus("$directory", Focus::Parts);
        select_item(&mut app, PaletteItem::Part("quota"));
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.editor.layout().to_format(), "$directory$quota");
        // 追加したトークンが選ばれるので、続けて Enter を押すと、その後ろへ足される
        select_item(&mut app, PaletteItem::Literal(" | "));
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.editor.layout().to_format(), "$directory$quota | ");
        // `i` は、選択中のトークンの前
        select_item(&mut app, PaletteItem::LineBreak);
        press(&mut app, KeyCode::Char('i'));
        assert_eq!(
            app.editor.layout().to_format(),
            "$directory$quota
 | "
        );
        // 元に戻す（フォーカスによらない）
        for expected in ["$directory$quota | ", "$directory$quota", "$directory"] {
            press(&mut app, KeyCode::Char('u'));
            assert_eq!(app.editor.layout().to_format(), expected);
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
        assert_eq!(app.editor.layout().to_format(), "$directory");
        assert!(!app.editor.can_undo());

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
        assert_eq!(app.editor.layout().to_format(), "$directory$model \n$quota");
        // Shift+下で、隣の行へ移す
        press_shift(&mut app, KeyCode::Down);
        assert_eq!(app.editor.layout().to_format(), "$directory \n$quota$model");
        // 削除
        press(&mut app, KeyCode::Char('d'));
        assert_eq!(app.editor.layout().to_format(), "$directory \n$quota");
        // 改行を挿入・行を追加
        press(&mut app, KeyCode::Char('n'));
        assert_eq!(app.editor.layout().to_format(), "$directory \n\n$quota");
        press(&mut app, KeyCode::Char('a'));
        assert_eq!(app.editor.layout().to_format(), "$directory \n\n$quota\n");
        // 元に戻す（フォーカスによらない）
        press(&mut app, KeyCode::Tab);
        for expected in ["$directory \n\n$quota", "$directory \n$quota"] {
            press(&mut app, KeyCode::Char('u'));
            assert_eq!(app.editor.layout().to_format(), expected);
        }
    }

    #[test]
    fn test_quit_and_unassigned_keys() {
        let mut unassigned = app("$model");
        press(&mut unassigned, KeyCode::Char('x'));
        assert!(!unassigned.should_quit, "未割り当てのキーは何もしない");
        assert!(!unassigned.editor.can_undo());
        for key in [KeyCode::Char('q'), KeyCode::Esc] {
            for focus in [Focus::Preview, Focus::Layout, Focus::Parts] {
                let mut app = app_with_focus("$model", focus);
                press(&mut app, key);
                assert!(app.should_quit, "{key:?}/{focus:?}");
            }
        }
    }
}
