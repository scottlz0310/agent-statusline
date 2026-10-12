//! プレビュービューアの状態と、キー入力による状態遷移（画面に依存しない）。

use ratatui::crossterm::event::KeyCode;

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

#[derive(Debug)]
pub struct App {
    scenario_index: usize,
    width_index: usize,
    pub overflow: Overflow,
    pub render_mode: RenderMode,
    pub should_quit: bool,
    /// 描画方式の切り替えなどで、画面全体を描き直す必要がある
    pub needs_clear: bool,
}

impl App {
    pub fn new() -> Self {
        Self {
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

    pub fn handle_key(&mut self, key: KeyCode) {
        let count = Scenario::ALL.len();
        match key {
            KeyCode::Char('q') | KeyCode::Esc => self.should_quit = true,
            KeyCode::Right | KeyCode::Char('l') | KeyCode::Tab => {
                self.scenario_index = (self.scenario_index + 1) % count;
            }
            KeyCode::Left | KeyCode::Char('h') | KeyCode::BackTab => {
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_scenario_navigation_wraps_around() {
        let mut app = App::new();
        assert_eq!(app.scenario(), Scenario::ALL[0]);
        for key in [KeyCode::Right, KeyCode::Char('l'), KeyCode::Tab] {
            app.handle_key(key);
        }
        assert_eq!(app.scenario(), Scenario::ALL[3]);
        for key in [KeyCode::Left, KeyCode::Char('h'), KeyCode::BackTab] {
            app.handle_key(key);
        }
        assert_eq!(app.scenario(), Scenario::ALL[0]);
        // 先頭から戻ると末尾、末尾から進むと先頭
        app.handle_key(KeyCode::Left);
        assert_eq!(app.scenario(), Scenario::ALL[Scenario::ALL.len() - 1]);
        app.handle_key(KeyCode::Right);
        assert_eq!(app.scenario(), Scenario::ALL[0]);
    }

    #[test]
    fn test_width_cycle_and_columns() {
        let mut app = App::new();
        let mut seen = Vec::new();
        for _ in 0..WIDTHS.len() {
            seen.push(app.columns(100));
            app.handle_key(KeyCode::Char('w'));
        }
        // 実端末の幅（Auto）、80、120、160 と巡回し、一周で戻る
        assert_eq!(seen, [100, 80, 120, 160]);
        assert_eq!(app.width_choice(), WidthChoice::Auto);
    }

    #[test]
    fn test_toggles_and_quit() {
        let mut app = App::new();
        assert_eq!(app.overflow, Overflow::Truncate);
        app.handle_key(KeyCode::Char('o'));
        assert_eq!(app.overflow, Overflow::Wrap);
        app.handle_key(KeyCode::Char('o'));
        assert_eq!(app.overflow, Overflow::Truncate);

        assert!(!app.needs_clear);
        app.handle_key(KeyCode::Char('r'));
        assert_eq!(app.render_mode, RenderMode::RawAnsi);
        assert!(app.needs_clear, "描画方式を変えたら、全体を描き直す");
        app.handle_key(KeyCode::Char('r'));
        assert_eq!(app.render_mode, RenderMode::Ratatui);

        app.handle_key(KeyCode::Char('x'));
        assert!(!app.should_quit, "未割り当てのキーは何もしない");
        for key in [KeyCode::Char('q'), KeyCode::Esc] {
            let mut app = App::new();
            app.handle_key(key);
            assert!(app.should_quit, "{key:?}");
        }
    }
}
