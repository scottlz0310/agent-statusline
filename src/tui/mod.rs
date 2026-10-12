//! 設定 TUI（`agent-statusline config`）。現状は、プレビュー専用のビューア（設定 TUI フェーズ 0-4b の試作）。
//!
//! 状態遷移は `app`、画面は `view`、ANSI の変換は `ansi` に置き、ここは端末とイベントの入出力だけを行う。

mod ansi;
mod app;
mod view;

use std::io::{self, IsTerminal, Write};
use std::path::Path;

use ratatui::DefaultTerminal;
use ratatui::crossterm::cursor::MoveTo;
use ratatui::crossterm::event::{self, Event, KeyEventKind};
use ratatui::crossterm::queue;
use ratatui::crossterm::style::Print;

use crate::editor::load::{ConfigEnv, ConfigSource, load_config_strict};
use app::{App, RenderMode};
use view::{RawOverlay, ViewContext, draw};

pub fn run(config_path: Option<&Path>) -> io::Result<()> {
    ensure_interactive(io::stdin().is_terminal(), io::stdout().is_terminal())?;
    let loaded =
        load_config_strict(config_path, &ConfigEnv::from_process()).map_err(io::Error::other)?;
    let label = match &loaded.source {
        ConfigSource::Existing { path, .. } if path.requested == path.real => {
            path.requested.display().to_string()
        }
        ConfigSource::Existing { path, .. } => {
            format!("{} → {}", path.requested.display(), path.real.display())
        }
        ConfigSource::Missing { target, .. } => {
            format!("未作成（既定値で表示。保存先: {}）", target.display())
        }
    };
    let ctx = ViewContext {
        config: &loaded.config,
        source_label: &label,
    };

    enable_color_output();
    let mut terminal = ratatui::init();
    let result = event_loop(&mut terminal, &ctx);
    ratatui::restore();
    result
}

/// プレビューは、`NO_COLOR` が設定されていても色つきで出す。
/// ratatui（crossterm）は `NO_COLOR` で色を出さなくなるが、ステータスライン本体（`render`）は
/// `NO_COLOR` に関係なく色つきで出力するので、無効のままだと実際の表示と食い違う
fn enable_color_output() {
    ratatui::crossterm::style::force_color_output(true);
}

/// 対話的な端末でなければ、キー入力を待ち続けずに、すぐ失敗する
fn ensure_interactive(stdin_is_terminal: bool, stdout_is_terminal: bool) -> io::Result<()> {
    if stdin_is_terminal && stdout_is_terminal {
        Ok(())
    } else {
        Err(io::Error::other(
            "`config` は対話的な端末で実行してください（標準入力または標準出力が端末ではありません）",
        ))
    }
}

fn event_loop(terminal: &mut DefaultTerminal, ctx: &ViewContext) -> io::Result<()> {
    let mut app = App::new();
    while !app.should_quit {
        // 生 ANSI の出力は、ratatui が把握していない（差分描画で消えない）ので、毎回全体を描き直す
        let needs_clear = std::mem::take(&mut app.needs_clear);
        if needs_clear || app.render_mode == RenderMode::RawAnsi {
            terminal.clear()?;
        }
        let mut overlay = None;
        terminal.draw(|frame| overlay = draw(frame, &app, ctx))?;
        if let Some(overlay) = overlay {
            write_overlay(&overlay)?;
        }

        // Windows ではキーの押下と解放の両方が届くので、押下だけを扱う
        if let Event::Key(key) = event::read()?
            && key.kind == KeyEventKind::Press
        {
            app.handle_key(key.code);
        }
    }
    Ok(())
}

/// プレビューを、ratatui を介さず生の ANSI として端末へ出す。直前の内容を空白で消してから書く
fn write_overlay(overlay: &RawOverlay) -> io::Result<()> {
    let mut out = io::stdout();
    let blank = " ".repeat(usize::from(overlay.width));
    for (row, line) in (0..).zip(&overlay.lines) {
        queue!(
            out,
            MoveTo(overlay.x, overlay.y + row),
            Print(&blank),
            MoveTo(overlay.x, overlay.y + row),
            Print(line),
            Print("\x1b[0m")
        )?;
    }
    out.flush()
}

#[cfg(test)]
mod tests {
    use ratatui::crossterm::style::{Color, Colored};

    use super::*;

    #[test]
    fn test_enable_color_output_overrides_no_color() {
        // `NO_COLOR` の有無にかかわらず、有効になる
        enable_color_output();
        let sequence = Colored::ForegroundColor(Color::DarkCyan).to_string();
        assert!(!sequence.is_empty(), "色の指定が出力されない");
    }

    #[test]
    fn test_ensure_interactive() {
        let cases = [
            (true, true, true),
            (false, true, false),
            (true, false, false),
            (false, false, false),
        ];
        for (stdin, stdout, ok) in cases {
            assert_eq!(
                ensure_interactive(stdin, stdout).is_ok(),
                ok,
                "{stdin}/{stdout}"
            );
        }
    }
}
