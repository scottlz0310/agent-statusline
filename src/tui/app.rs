//! 設定 TUI の状態と、キー入力による状態遷移（画面に依存しない）。

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::editor::layout::Layout;
use crate::editor::layout_edit::{Direction, LayoutEditor};
use crate::preview::scenario::Scenario;
use crate::preview::width::Overflow;

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

/// プレビューの描画方式。生の ANSI は、ratatui の文字幅の計算を介さず端末へ直接出力する（絵文字の崩れの比較用）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderMode {
    Ratatui,
    RawAnsi,
}

/// キー入力の宛先。`Tab` で切り替える
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Preview,
    Layout,
}

#[derive(Debug)]
pub struct App {
    pub focus: Focus,
    pub editor: LayoutEditor,
    scenario_index: usize,
    width_index: usize,
    pub overflow: Overflow,
    pub render_mode: RenderMode,
    pub should_quit: bool,
    /// 描画方式の切り替えなどで、画面全体を描き直す必要がある
    pub needs_clear: bool,
}

impl App {
    pub fn new(layout: Layout) -> Self {
        Self {
            focus: Focus::Layout,
            editor: LayoutEditor::new(layout),
            scenario_index: 0,
            width_index: 0,
            overflow: Overflow::Truncate,
            render_mode: RenderMode::Ratatui,
            should_quit: false,
            needs_clear: false,
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

    pub fn handle_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => self.should_quit = true,
            KeyCode::Tab | KeyCode::BackTab => {
                self.focus = match self.focus {
                    Focus::Preview => Focus::Layout,
                    Focus::Layout => Focus::Preview,
                };
            }
            KeyCode::Char('u') => self.editor.undo(),
            code => match self.focus {
                Focus::Preview => self.handle_preview_key(code),
                Focus::Layout => self.handle_layout_key(code, key.modifiers),
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
            KeyCode::Char('r') => {
                self.render_mode = match self.render_mode {
                    RenderMode::Ratatui => RenderMode::RawAnsi,
                    RenderMode::RawAnsi => RenderMode::Ratatui,
                };
                self.needs_clear = true;
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
    fn test_tab_toggles_focus_and_routes_keys() {
        let mut app = app("$directory $model");
        // 配置にフォーカスがあるとき、矢印は選択を動かし、シナリオは変わらない
        press(&mut app, KeyCode::Right);
        assert_eq!(app.editor.cursor(), Cursor { line: 0, index: 1 });
        assert_eq!(app.scenario(), Scenario::ALL[0]);

        for key in [KeyCode::Tab, KeyCode::BackTab] {
            press(&mut app, key);
            assert_eq!(app.focus, Focus::Preview, "{key:?}");
            // プレビューにフォーカスがあるとき、矢印はシナリオを動かし、選択は変わらない
            press(&mut app, KeyCode::Right);
            assert_eq!(app.scenario(), Scenario::ALL[1], "{key:?}");
            press(&mut app, KeyCode::Left);
            assert_eq!(app.editor.cursor(), Cursor { line: 0, index: 1 }, "{key:?}");
            press(&mut app, key);
            assert_eq!(app.focus, Focus::Layout, "{key:?}");
        }
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

        assert!(!app.needs_clear);
        press(&mut app, KeyCode::Char('r'));
        assert_eq!(app.render_mode, RenderMode::RawAnsi);
        assert!(app.needs_clear, "描画方式を変えたら、全体を描き直す");
        press(&mut app, KeyCode::Char('r'));
        assert_eq!(app.render_mode, RenderMode::Ratatui);
    }

    #[test]
    fn test_preview_keys_do_nothing_on_layout_focus() {
        let mut app = app("$model");
        for key in [KeyCode::Char('w'), KeyCode::Char('o'), KeyCode::Char('r')] {
            press(&mut app, key);
        }
        assert_eq!(app.width_choice(), WidthChoice::Auto);
        assert_eq!(app.overflow, Overflow::Truncate);
        assert_eq!(app.render_mode, RenderMode::Ratatui);
    }

    #[test]
    fn test_layout_keys_do_nothing_on_preview_focus() {
        let mut app = app_with_focus("$directory $model", Focus::Preview);
        for key in [KeyCode::Char('d'), KeyCode::Char('n'), KeyCode::Char('a')] {
            press(&mut app, key);
        }
        assert_eq!(app.editor.layout().to_format(), "$directory $model");
        assert!(!app.editor.can_undo());
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
            for focus in [Focus::Preview, Focus::Layout] {
                let mut app = app_with_focus("$model", focus);
                press(&mut app, key);
                assert!(app.should_quit, "{key:?}/{focus:?}");
            }
        }
    }
}
