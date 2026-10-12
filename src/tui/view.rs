//! 設定 TUI の画面。状態（`App`）と設定から、描画だけを行う。

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph};

use crate::editor::layout::Token;
use crate::engine::config::Config;
use crate::engine::parts::part_ids;
use crate::preview::width::{Overflow, display_width, fit_line};
use crate::preview::{layout_visibility, render_part_sample, render_preview};
use crate::tui::ansi::ansi_line;
use crate::tui::app::{App, Focus, RenderMode, WidthChoice};

/// プレビューの最大の表示行数。それ以上は、切り詰めて表示する
const MAX_PREVIEW_ROWS: usize = 8;

pub struct ViewContext<'a> {
    pub config: &'a Config,
    /// 編集対象の設定ファイルの説明（ヘッダーに表示する）
    pub source_label: &'a str,
}

/// ratatui の外で、生の ANSI として端末へ出力するプレビューの内容と位置
#[derive(Debug, PartialEq, Eq)]
pub struct RawOverlay {
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub lines: Vec<String>,
}

pub fn draw(frame: &mut Frame, app: &App, ctx: &ViewContext) -> Option<RawOverlay> {
    // 編集中の配置を、設定の `format` として描画する（保存するまで、設定ファイルは変わらない）
    let mut edited = ctx.config.clone();
    edited.format = app.editor.layout().to_format();
    let config = &edited;
    let area = frame.area();
    // uto は、プレビューの枠の内側に収まる幅（実端末の幅から枠を除いたもの）
    let inner_width = usize::from(area.width).saturating_sub(2);
    let columns = app.columns(inner_width);

    // 指定した幅が実端末より広いときは、横を切り詰めて表示する
    let too_wide = columns > inner_width;
    let lines: Vec<String> = render_preview(config, app.scenario(), columns, app.overflow)
        .into_iter()
        .flat_map(|line| {
            if too_wide {
                fit_line(&line, inner_width, Overflow::Truncate)
            } else {
                vec![line]
            }
        })
        .take(MAX_PREVIEW_ROWS)
        .collect();
    let preview_height = u16::try_from(lines.len() + 2).unwrap_or(u16::MAX);

    let [header, preview, middle, footer] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(preview_height),
        Constraint::Min(3),
        Constraint::Length(1),
    ])
    .areas(area);
    let [layout_area, parts] =
        Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)]).areas(middle);

    frame.render_widget(
        Line::from(format!(
            "agent-statusline config — 配置の編集（まだ保存できません）  設定: {}",
            ctx.source_label
        ))
        .style(Style::default().add_modifier(Modifier::BOLD)),
        header,
    );

    let overlay = draw_preview(frame, app, preview, &lines, columns, too_wide);
    draw_layout(frame, app, ctx.config, layout_area, columns);
    draw_parts(frame, app, ctx.config, parts, columns);
    frame.render_widget(
        Line::from(footer_text(app.focus)).style(Style::default().add_modifier(Modifier::DIM)),
        footer,
    );
    overlay
}

fn footer_text(focus: Focus) -> &'static str {
    match focus {
        Focus::Layout => {
            "Tab: プレビュー  ↑↓←→: 選択  Shift+矢印: 移動  d: 削除  n: 改行  a: 行追加  u: 戻す  q: 終了"
        }
        Focus::Preview => {
            "Tab: 配置  ←/→: シナリオ  w: 幅  o: 折返/切詰  r: 描画方式  u: 戻す  q: 終了"
        }
    }
}

/// フォーカスのある枠は、色を付けて示す
fn focus_block(title: String, focused: bool) -> Block<'static> {
    let block = Block::bordered().title(title);
    if focused {
        block.border_style(Style::default().fg(Color::Cyan))
    } else {
        block
    }
}

fn draw_preview(
    frame: &mut Frame,
    app: &App,
    area: Rect,
    lines: &[String],
    columns: usize,
    too_wide: bool,
) -> Option<RawOverlay> {
    let width = match app.width_choice() {
        WidthChoice::Auto => format!("auto（{columns}）"),
        WidthChoice::Columns(n) => n.to_string(),
    };
    let overflow = match app.overflow {
        Overflow::Wrap => "折り返し",
        Overflow::Truncate => "切り詰め",
    };
    let mode = match app.render_mode {
        RenderMode::Ratatui => "ratatui",
        RenderMode::RawAnsi => "生 ANSI",
    };
    let mut block = focus_block(
        format!(
            " プレビュー ─ 幅: {width} ─ シナリオ: {} ─ {overflow} ─ 描画: {mode} ",
            app.scenario().label()
        ),
        app.focus == Focus::Preview,
    );
    if too_wide {
        block =
            block.title_bottom(" 指定した幅が実端末より広いため、横を切り詰めて表示しています ");
    }
    let inner = block.inner(area);
    frame.render_widget(block, area);

    match app.render_mode {
        RenderMode::Ratatui => {
            let text: Vec<Line> = lines.iter().map(|line| ansi_line(line)).collect();
            frame.render_widget(Paragraph::new(text), inner);
            None
        }
        RenderMode::RawAnsi => Some(RawOverlay {
            x: inner.x,
            y: inner.y,
            width: inner.width,
            lines: clip_to_area(lines, inner),
        }),
    }
}

/// 生 ANSI で端末へ直接出す行を、枠の内側（`area`）に収める。
/// 行数は高さまで、各行は幅までにし、幅より広い 1 文字しか入らない行は空にする。
/// ratatui を介さないので、はみ出すと枠や部品の表示を上書きしてしまう
fn clip_to_area(lines: &[String], area: Rect) -> Vec<String> {
    if area.width == 0 || area.height == 0 {
        return Vec::new();
    }
    let width = usize::from(area.width);
    lines
        .iter()
        .take(usize::from(area.height))
        .map(|line| {
            let clipped = fit_line(line, width, Overflow::Truncate).swap_remove(0);
            if display_width(&clipped) <= width {
                clipped
            } else {
                String::new()
            }
        })
        .collect()
}

/// 配置の編集領域。行ごとに、部品と固定テキストを並べ、選択中のトークンを反転する。
/// 何も表示しない部品は薄く示し、出力されない行にはその旨を添える（設計書 §6.5）
fn draw_layout(frame: &mut Frame, app: &App, config: &Config, area: Rect, columns: usize) {
    let focused = app.focus == Focus::Layout;
    let block = focus_block(" 配置 ".to_string(), focused);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let dim = Style::default().add_modifier(Modifier::DIM);
    let layout = app.editor.layout();
    let cursor = app.editor.cursor();
    let visibility = layout_visibility(config, app.scenario(), layout, columns);

    let rows: Vec<Line> = layout
        .lines
        .iter()
        .zip(&visibility)
        .enumerate()
        .map(|(line_index, (tokens, visibility))| {
            let mut spans = vec![Span::raw(format!("{:>2}: ", line_index + 1))];
            for (index, (token, hidden)) in tokens.iter().zip(&visibility.hidden).enumerate() {
                let mut style = if *hidden { dim } else { Style::default() };
                if (cursor.line, cursor.index) == (line_index, index) {
                    style = style.add_modifier(if focused {
                        Modifier::REVERSED
                    } else {
                        Modifier::UNDERLINED
                    });
                }
                spans.push(Span::styled(token_label(token), style));
                spans.push(Span::raw(" "));
            }
            if tokens.is_empty() {
                spans.push(Span::styled("（空の行）", dim));
            }
            if visibility.blank {
                spans.push(Span::styled(" ← この行は出力されません", dim));
            }
            Line::from(spans)
        })
        .collect();
    frame.render_widget(Paragraph::new(rows), inner);
}

/// 部品は ID、固定テキストは引用符で囲み、空白は `␣` にして、見分けられるようにする
fn token_label(token: &Token) -> String {
    match token {
        Token::Part(id) => id.clone(),
        Token::Literal(text) => format!("\"{}\"", text.replace(' ', "␣")),
    }
}

fn draw_parts(frame: &mut Frame, app: &App, config: &Config, area: Rect, columns: usize) {
    let block = Block::bordered().title(format!(" 部品 ─ {}のサンプル ", app.scenario().label()));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let rows: Vec<Line> = part_ids()
        .into_iter()
        .map(|id| {
            let mut spans = vec![Span::raw(format!("{id:<12} "))];
            match render_part_sample(config, app.scenario(), id, columns) {
                Some(sample) if !sample.is_empty() => spans.extend(ansi_line(&sample).spans),
                _ => spans.push(Span::styled(
                    "（非表示）",
                    Style::default().add_modifier(Modifier::DIM),
                )),
            }
            Line::from(spans)
        })
        .collect();
    frame.render_widget(Paragraph::new(rows), inner);
}

#[cfg(test)]
mod tests {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::crossterm::event::{KeyCode, KeyEvent};

    use super::*;
    use crate::editor::layout::Layout as FormatLayout;
    use crate::engine::config::DEFAULT_FORMAT;

    fn new_app() -> App {
        App::new(FormatLayout::parse(DEFAULT_FORMAT))
    }

    /// シナリオ・幅・描画方式のキーを受け付ける、プレビューにフォーカスのある状態
    fn preview_app() -> App {
        let mut app = new_app();
        press(&mut app, KeyCode::Tab);
        assert_eq!(app.focus, Focus::Preview);
        app
    }

    fn press(app: &mut App, code: KeyCode) {
        app.handle_key(KeyEvent::from(code));
    }

    /// 画面の文字（左上から右下）を行ごとに返す。全角文字の 2 セル目（空）は飛ばす
    fn screen(terminal: &Terminal<TestBackend>) -> Vec<String> {
        let buffer = terminal.backend().buffer();
        (0..buffer.area.height)
            .map(|y| {
                let mut line = String::new();
                let mut x = 0;
                while x < buffer.area.width {
                    let symbol = buffer[(x, y)].symbol();
                    line.push_str(symbol);
                    // 全角文字は 2 セルを占める。2 セル目は飛ばす
                    x += u16::try_from(display_width(symbol).max(1)).unwrap_or(1);
                }
                line
            })
            .collect()
    }

    fn render(app: &App, width: u16, height: u16) -> (Vec<String>, Option<RawOverlay>) {
        let config = Config::default();
        let ctx = ViewContext {
            config: &config,
            source_label: "（テスト）",
        };
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        let mut overlay = None;
        terminal
            .draw(|frame| overlay = draw(frame, app, &ctx))
            .unwrap();
        (screen(&terminal), overlay)
    }

    #[test]
    fn test_draw_shows_header_preview_parts_and_footer() {
        let (screen, overlay) = render(&new_app(), 100, 24);
        let text = screen.join("\n");
        for expected in [
            "配置の編集（まだ保存できません）",
            "設定: （テスト）",
            "シナリオ: 通常",
            "描画: ratatui",
            "myproject",
            "Claude Opus",
            "directory",
            "review",
            "（非表示）", // 通常のシナリオでは、レビューは表示されない
            " 1: ",
            "Shift+矢印: 移動", // 配置にフォーカスがある
            "q: 終了",
        ] {
            assert!(text.contains(expected), "{expected} がない\n{text}");
        }
        assert_eq!(overlay, None, "ratatui 描画では、生 ANSI の出力はない");
    }

    #[test]
    fn test_draw_follows_scenario_and_width() {
        let mut app = preview_app();
        press(&mut app, KeyCode::Right); // 長いパス・ブランチ
        let (screen, _) = render(&app, 100, 24);
        assert!(screen.join("\n").contains("シナリオ: 長いパス・ブランチ"));

        // 幅 160 は、実端末（100 列）より広い: その旨を表示する
        press(&mut app, KeyCode::Char('w'));
        press(&mut app, KeyCode::Char('w'));
        press(&mut app, KeyCode::Char('w'));
        let (screen, _) = render(&app, 100, 24);
        let text = screen.join("\n");
        assert!(text.contains("幅: 160"), "{text}");
        assert!(text.contains("横を切り詰めて表示しています"), "{text}");

        // 実端末の幅（auto）では、その表示は出ない
        press(&mut app, KeyCode::Char('w'));
        let (screen, _) = render(&app, 100, 24);
        let text = screen.join("\n");
        assert!(text.contains("幅: auto（98）"), "{text}");
        assert!(!text.contains("横を切り詰めて"), "{text}");
    }

    #[test]
    fn test_draw_raw_mode_returns_overlay_inside_the_border() {
        let mut app = preview_app();
        press(&mut app, KeyCode::Char('r'));
        let (screen, overlay) = render(&app, 100, 24);
        let overlay = overlay.expect("生 ANSI の出力がある");
        // ヘッダー 1 行、枠 1 行の次から、枠の内側の幅で出す
        assert_eq!((overlay.x, overlay.y, overlay.width), (1, 2, 98));
        assert_eq!(overlay.lines.len(), 3);
        assert!(overlay.lines[0].contains("\x1b["), "{:?}", overlay.lines[0]);
        let text = screen.join("\n");
        assert!(text.contains("描画: 生 ANSI"));
        // 枠の内側は ratatui では描かない（端末へ直接出す）
        for row in &screen[2..5] {
            assert!(!row.contains("myproject"), "{row}");
        }
        // 部品のサンプルは、これまでどおり ratatui で描く
        assert!(screen[7..].join("\n").contains("myproject"), "{text}");
    }

    #[test]
    fn test_raw_overlay_stays_inside_the_preview_area() {
        // 小さい端末・縮小リサイズでも、生 ANSI の出力は枠の内側（行数・各行の幅）に収まる
        let sizes = [
            (1, 1),
            (3, 24),
            (10, 5),
            (20, 8),
            (100, 5),
            (60, 12),
            (100, 24),
        ];
        for (width, height) in sizes {
            for steps in 0..8 {
                let mut app = preview_app();
                press(&mut app, KeyCode::Char('r'));
                for _ in 0..steps {
                    press(&mut app, KeyCode::Right);
                }
                let (_, overlay) = render(&app, width, height);
                let overlay = overlay.expect("生 ANSI の出力がある");
                let (cols, rows) = (usize::from(overlay.width), overlay.lines.len());
                assert!(
                    overlay.x + overlay.width <= width,
                    "{width}x{height}: {overlay:?}"
                );
                assert!(
                    overlay.lines.is_empty() || overlay.y + u16::try_from(rows).unwrap() <= height,
                    "{width}x{height}/{steps}: 行数 {rows} が高さを超える"
                );
                for line in &overlay.lines {
                    assert!(
                        display_width(line) <= cols,
                        "{width}x{height}/{steps}: {line:?} が幅 {cols} を超える"
                    );
                }
                if overlay.width == 0 {
                    assert!(
                        overlay.lines.is_empty(),
                        "{width}x{height}: 幅 0 では出力しない"
                    );
                }
            }
        }
    }

    #[test]
    fn test_clip_to_area() {
        let line = |s: &str| s.to_string();
        let lines = [line("abcdef"), line("🌿🌿🌿"), line("xy"), line("z")];
        let area = |width, height| Rect::new(0, 0, width, height);
        // (名前, 領域の幅, 高さ, 期待する結果)
        let cases: [(&str, u16, u16, Vec<String>); 5] = [
            ("fits", 10, 4, lines.to_vec()),
            (
                "height-limited",
                10,
                2,
                vec![line("abcdef"), line("🌿🌿🌿")],
            ),
            (
                "width-limited",
                3,
                4,
                vec![line("abc"), line("🌿"), line("xy"), line("z")],
            ),
            // 幅 1 の領域に、幅 2 の絵文字は入らない: 空にする
            ("char-wider-than-area", 1, 2, vec![line("a"), line("")]),
            ("empty-area", 0, 3, vec![]),
        ];
        for (name, width, height, expected) in cases {
            assert_eq!(
                clip_to_area(&lines, area(width, height)),
                expected,
                "{name}"
            );
        }
        assert_eq!(clip_to_area(&lines, area(5, 0)), Vec::<String>::new());
    }

    #[test]
    fn test_draw_survives_tiny_terminals() {
        // 極端に小さい端末でも panic しない
        for (width, height) in [(1, 1), (10, 5), (20, 8), (40, 12)] {
            for scenario_steps in 0..8 {
                let mut app = preview_app();
                for _ in 0..scenario_steps {
                    press(&mut app, KeyCode::Right);
                }
                let _ = render(&app, width, height);
            }
        }
    }

    #[test]
    fn test_draw_layout_shows_tokens_and_hidden_notes() {
        let config = Config::default();
        let ctx = ViewContext {
            config: &config,
            source_label: "（テスト）",
        };
        let mut app = App::new(FormatLayout::parse(
            "$directory | $review

$review",
        ));
        let mut terminal = Terminal::new(TestBackend::new(100, 24)).unwrap();
        terminal
            .draw(|frame| {
                draw(frame, &app, &ctx);
            })
            .unwrap();
        let text = screen(&terminal).join(
            "
",
        );
        // 固定テキストは引用符で囲み、空白は `␣` にする
        assert!(text.contains("directory \"␣|␣\" review"), "{text}");
        assert!(text.contains("（空の行）"), "{text}");
        // 出力されない行（レビューだけの行と、空の行）に、その旨を添える
        assert_eq!(text.matches("この行は出力されません").count(), 2, "{text}");

        // プレビューにフォーカスを移すと、配置の操作ではなく、プレビューの操作を案内する
        press(&mut app, KeyCode::Tab);
        terminal
            .draw(|frame| {
                draw(frame, &app, &ctx);
            })
            .unwrap();
        let text = screen(&terminal).join(
            "
",
        );
        assert!(text.contains("w: 幅"), "{text}");
        assert!(!text.contains("Shift+矢印"), "{text}");
    }

    #[test]
    fn test_draw_preview_follows_the_edited_layout() {
        let mut app = App::new(FormatLayout::parse("$model $quota"));
        let (before, _) = render(&app, 100, 24);
        assert!(
            before[2..5]
                .join(
                    "
"
                )
                .contains("Claude Opus")
        );

        // 選択中（先頭）の `$model` を消すと、プレビューにも反映される。設定自体は変わらない
        press(&mut app, KeyCode::Char('d'));
        let (after, _) = render(&app, 100, 24);
        assert!(
            !after[2..5]
                .join(
                    "
"
                )
                .contains("Claude Opus"),
            "{after:?}"
        );

        press(&mut app, KeyCode::Char('u'));
        let (restored, _) = render(&app, 100, 24);
        assert!(
            restored[2..5]
                .join(
                    "
"
                )
                .contains("Claude Opus")
        );
    }
}
