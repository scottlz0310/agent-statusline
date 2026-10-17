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
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyEventKind, MouseButton, MouseEventKind,
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
    run_loop(
        &mut app,
        |app| {
            // 生 ANSI の出力は、ratatui が把握していない（差分描画で消えない）ので、毎回全体を描き直す
            terminal.clear()?;
            let mut overlays = Vec::new();
            terminal.draw(|frame| overlays = draw(frame, app))?;
            write_overlays(&mut io::stdout(), &overlays)
        },
        event::read,
    )
}

/// 描画と入力を繰り返す。端末（描画と入力）を差し込めるので、テストで描画の回数を確かめられる。
/// 画面を変えない入力（マウスの移動など）では、描き直さずに次の入力を待つ
fn run_loop(
    app: &mut App,
    mut draw_screen: impl FnMut(&App) -> io::Result<()>,
    mut next_event: impl FnMut() -> io::Result<Event>,
) -> io::Result<()> {
    while !app.should_quit {
        draw_screen(app)?;
        loop {
            let event = next_event()?;
            let redraw = needs_redraw(&event);
            // Windows ではキーの押下と解放の両方が届くので、押下だけを扱う
            match event {
                Event::Key(key) if key.kind == KeyEventKind::Press => app.handle_key(key),
                Event::Mouse(mouse) => app.handle_mouse(mouse),
                _ => {}
            }
            if redraw {
                break;
            }
        }
    }
    Ok(())
}

/// 画面を変えうる入力か。マウスのキャプチャは、ボタンを押さない移動も通知する（`?1003h`）ので、
/// 状態を変えない入力（移動・ドラッグ・ボタンの解放・左以外のボタン・横ホイール・キーの解放）では、描き直さない。
/// 端末の大きさの変更は、描き直す
fn needs_redraw(event: &Event) -> bool {
    match event {
        Event::Key(key) => key.kind == KeyEventKind::Press,
        Event::Mouse(mouse) => matches!(
            mouse.kind,
            MouseEventKind::Down(MouseButton::Left)
                | MouseEventKind::ScrollUp
                | MouseEventKind::ScrollDown
        ),
        Event::Resize(..) => true,
        _ => false,
    }
}

/// カーソルの位置の保存と復元（DECSC / DECRC）
const SAVE_CURSOR: &str = "\x1b7";
const RESTORE_CURSOR: &str = "\x1b8";

/// プレビューなどを、ratatui を介さず生の ANSI として端末へ出す。直前の内容を空白で消してから書く。
/// 書く前後でカーソルの位置を戻す。入力中は、ratatui が入力欄にカーソルを置いており（IME の変換ウィンドウは
/// そこに出る）、書いた末尾に残ると、変換ウィンドウが別の位置に出る
fn write_overlays(out: &mut impl Write, overlays: &[RawOverlay]) -> io::Result<()> {
    queue!(out, Print(SAVE_CURSOR))?;
    for overlay in overlays {
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
    }
    queue!(out, Print(RESTORE_CURSOR))?;
    out.flush()
}

#[cfg(test)]
mod tests {
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent};
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
    fn test_write_overlays_restores_the_cursor() {
        let overlay = |x, lines: &[&str]| RawOverlay {
            x,
            y: 1,
            width: 6,
            lines: lines.iter().map(ToString::to_string).collect(),
        };
        for (name, overlays) in [
            ("none", vec![]),
            ("one", vec![overlay(2, &["a"])]),
            ("two", vec![overlay(2, &["a", "b"]), overlay(30, &["c"])]),
        ] {
            let mut out = Vec::new();
            write_overlays(&mut out, &overlays).unwrap();
            let written = String::from_utf8(out).unwrap();
            // 書く前に保存し、最後に戻す。途中では、保存も復元もしない
            assert!(written.starts_with(SAVE_CURSOR), "{name}: {written:?}");
            assert!(written.ends_with(RESTORE_CURSOR), "{name}: {written:?}");
            assert_eq!(written.matches(SAVE_CURSOR).count(), 1, "{name}");
            assert_eq!(written.matches(RESTORE_CURSOR).count(), 1, "{name}");
            for line in overlays.iter().flat_map(|overlay| &overlay.lines) {
                assert!(written.contains(line.as_str()), "{name}: {line}");
            }
        }
    }

    fn key(code: KeyCode, kind: KeyEventKind) -> Event {
        let mut event = KeyEvent::from(code);
        event.kind = kind;
        Event::Key(event)
    }

    fn mouse(kind: MouseEventKind) -> Event {
        Event::Mouse(MouseEvent {
            kind,
            column: 3,
            row: 3,
            modifiers: KeyModifiers::NONE,
        })
    }

    #[test]
    fn test_needs_redraw() {
        // (名前, 入力, 描き直すか)
        let cases = [
            (
                "key-press",
                key(KeyCode::Char('x'), KeyEventKind::Press),
                true,
            ),
            (
                "key-release",
                key(KeyCode::Char('x'), KeyEventKind::Release),
                false,
            ),
            (
                "left-click",
                mouse(MouseEventKind::Down(MouseButton::Left)),
                true,
            ),
            ("wheel-up", mouse(MouseEventKind::ScrollUp), true),
            ("wheel-down", mouse(MouseEventKind::ScrollDown), true),
            ("move", mouse(MouseEventKind::Moved), false),
            (
                "drag",
                mouse(MouseEventKind::Drag(MouseButton::Left)),
                false,
            ),
            (
                "left-release",
                mouse(MouseEventKind::Up(MouseButton::Left)),
                false,
            ),
            (
                "right-click",
                mouse(MouseEventKind::Down(MouseButton::Right)),
                false,
            ),
            ("wheel-left", mouse(MouseEventKind::ScrollLeft), false),
            ("resize", Event::Resize(100, 40), true),
            ("focus-gained", Event::FocusGained, false),
        ];
        for (name, event, expected) in cases {
            assert_eq!(needs_redraw(&event), expected, "{name}");
        }
    }

    /// 入力を順に渡し、描画の回数を数える。入力が尽きたら、読み取りの失敗でループを抜ける
    fn run_with(events: Vec<Event>) -> (usize, App) {
        use crate::editor::draft::Draft;
        let mut app = App::for_test(Draft::with_format("$model"));
        let mut draws = 0;
        let mut events = events.into_iter();
        let result = run_loop(
            &mut app,
            |_| {
                draws += 1;
                Ok(())
            },
            || {
                events
                    .next()
                    .ok_or_else(|| io::Error::new(io::ErrorKind::UnexpectedEof, "入力が尽きた"))
            },
        );
        // 終了したか、入力が尽きたか（どちらも想定内）
        assert!(result.is_ok() || result.is_err_and(|e| e.kind() == io::ErrorKind::UnexpectedEof));
        (draws, app)
    }

    #[test]
    fn test_pointer_movement_does_not_redraw() {
        // 移動・ドラッグ・解放だけが 1000 回届いても、描画は最初の 1 回だけ
        let mut events: Vec<Event> = (0..1000)
            .map(|i| match i % 3 {
                0 => mouse(MouseEventKind::Moved),
                1 => mouse(MouseEventKind::Drag(MouseButton::Left)),
                _ => mouse(MouseEventKind::Up(MouseButton::Left)),
            })
            .collect();
        events.push(key(KeyCode::Char('x'), KeyEventKind::Release));
        let (draws, _) = run_with(events);
        assert_eq!(draws, 1);
    }

    #[test]
    fn test_redraws_once_per_meaningful_input_even_between_pointer_movements() {
        let events = vec![
            mouse(MouseEventKind::Moved),
            mouse(MouseEventKind::Moved),
            // 意味のある入力（左クリック）で 1 回
            mouse(MouseEventKind::Down(MouseButton::Left)),
            mouse(MouseEventKind::Moved),
            // キーの押下で 1 回。解放は描き直さない
            key(KeyCode::Char('x'), KeyEventKind::Press),
            key(KeyCode::Char('x'), KeyEventKind::Release),
            mouse(MouseEventKind::Moved),
            // ホイールで 1 回
            mouse(MouseEventKind::ScrollDown),
            // 大きさの変更で 1 回
            Event::Resize(80, 24),
        ];
        let (draws, _) = run_with(events);
        // 最初の描画 + クリック・キー・ホイール・大きさの変更の 4 回
        assert_eq!(draws, 5);
    }

    #[test]
    fn test_loop_ends_after_quit_without_an_extra_draw() {
        let events = vec![
            mouse(MouseEventKind::Moved),
            key(KeyCode::Char('q'), KeyEventKind::Press),
            // 終了した後の入力は読まない
            key(KeyCode::Char('x'), KeyEventKind::Press),
        ];
        let (draws, app) = run_with(events);
        assert!(app.should_quit);
        assert_eq!(draws, 1, "終了のキーの後は、描き直さずに抜ける");
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
