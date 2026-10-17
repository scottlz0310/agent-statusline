//! 設定 TUI（`agent-statusline config`）。配置とプロパティの編集、プレビュー、保存。
//!
//! 状態遷移は `app`、部品一覧の項目は `palette`、画面は `view` に置き、ここは端末とイベントの入出力だけを行う。

mod app;
mod palette;
mod storage;
mod symbols;
mod view;

use std::io::{self, IsTerminal, Write};
use std::panic;
use std::path::Path;

use ratatui::DefaultTerminal;
use ratatui::crossterm::cursor::MoveTo;
use ratatui::crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyEventKind,
};
use ratatui::crossterm::style::Print;
use ratatui::crossterm::{execute, queue};

use crate::editor::draft::Draft;
use crate::editor::load::{ConfigEnv, load_config_strict};
use app::App;
use storage::FileStorage;
use view::{RawOverlay, draw};

/// `mouse` が偽のときは、マウスを使わない（端末のテキスト選択が、そのまま使える）
pub fn run(config_path: Option<&Path>, mouse: bool) -> io::Result<()> {
    ensure_interactive(io::stdin().is_terminal(), io::stdout().is_terminal())?;
    let loaded =
        load_config_strict(config_path, &ConfigEnv::from_process()).map_err(io::Error::other)?;
    let draft = Draft::new(loaded.config, loaded.document);
    let storage = FileStorage::from_process();
    let app = App::new(draft, loaded.source, Box::new(storage));

    enable_color_output();
    let mut terminal = ratatui::init();
    // マウスのキャプチャは、`ratatui::restore` より前に解除する（エラーの戻りでも、`Drop` で解除される）
    let capture = if mouse {
        MouseCapture::enable().map(Some)
    } else {
        Ok(None)
    };
    let result = capture.and_then(|capture| {
        let result = event_loop(&mut terminal, app);
        drop(capture);
        result
    });
    ratatui::restore();
    result
}

/// 端末のマウスのキャプチャを、有効にしている間だけ持つ。`Drop` と panic フックで、必ず解除する
/// （リリースビルドは `panic = "abort"` で `Drop` が走らないので、panic フックでも解除する）
struct MouseCapture;

impl MouseCapture {
    fn enable() -> io::Result<Self> {
        execute!(io::stdout(), EnableMouseCapture)?;
        let previous = panic::take_hook();
        panic::set_hook(Box::new(move |info| {
            let _ = disable_mouse_capture();
            previous(info);
        }));
        Ok(Self)
    }
}

impl Drop for MouseCapture {
    fn drop(&mut self) {
        let _ = disable_mouse_capture();
    }
}

fn disable_mouse_capture() -> io::Result<()> {
    execute!(io::stdout(), DisableMouseCapture)
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

fn event_loop(terminal: &mut DefaultTerminal, mut app: App) -> io::Result<()> {
    while !app.should_quit {
        // 生 ANSI の出力は、ratatui が把握していない（差分描画で消えない）ので、毎回全体を描き直す
        terminal.clear()?;
        let mut overlays = Vec::new();
        terminal.draw(|frame| overlays = draw(frame, &app))?;
        for overlay in &overlays {
            write_overlay(overlay)?;
        }

        // Windows ではキーの押下と解放の両方が届くので、押下だけを扱う
        match event::read()? {
            Event::Key(key) if key.kind == KeyEventKind::Press => app.handle_key(key),
            Event::Mouse(mouse) => app.handle_mouse(mouse),
            _ => {}
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
    fn test_config_subcommand_accepts_no_mouse() {
        use clap::Parser;

        use crate::cli::{Cli, Commands};

        // (引数, 期待する no_mouse)
        let cases = [
            (vec!["agent-statusline", "config"], false),
            (vec!["agent-statusline", "config", "--no-mouse"], true),
            (
                vec![
                    "agent-statusline",
                    "config",
                    "--config",
                    "c.toml",
                    "--no-mouse",
                ],
                true,
            ),
        ];
        for (args, expected) in cases {
            let cli = Cli::try_parse_from(&args).unwrap();
            let Commands::Config { no_mouse, .. } = cli.command else {
                panic!("config サブコマンド: {args:?}");
            };
            assert_eq!(no_mouse, expected, "{args:?}");
        }
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
