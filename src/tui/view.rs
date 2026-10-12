//! プレビュービューアの画面。状態（`App`）と設定から、描画だけを行う。

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph};

use crate::engine::config::Config;
use crate::engine::parts::part_ids;
use crate::preview::width::{Overflow, fit_line};
use crate::preview::{render_part_sample, render_preview};
use crate::tui::ansi::ansi_line;
use crate::tui::app::{App, RenderMode, WidthChoice};

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
    let area = frame.area();
    // uto は、プレビューの枠の内側に収まる幅（実端末の幅から枠を除いたもの）
    let inner_width = usize::from(area.width).saturating_sub(2);
    let columns = app.columns(inner_width);

    // 指定した幅が実端末より広いときは、横を切り詰めて表示する
    let too_wide = columns > inner_width;
    let lines: Vec<String> = render_preview(ctx.config, app.scenario(), columns, app.overflow)
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

    let [header, preview, parts, footer] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(preview_height),
        Constraint::Min(3),
        Constraint::Length(1),
    ])
    .areas(area);

    frame.render_widget(
        Line::from(format!(
            "agent-statusline config — プレビュー（読み取り専用）  設定: {}",
            ctx.source_label
        ))
        .style(Style::default().add_modifier(Modifier::BOLD)),
        header,
    );

    let overlay = draw_preview(frame, app, preview, &lines, columns, too_wide);
    draw_parts(frame, app, ctx.config, parts, columns);
    frame.render_widget(
        Line::from("←/→ Tab: シナリオ  w: 幅  o: 折り返し/切り詰め  r: 描画方式（ratatui/生 ANSI）  q: 終了")
            .style(Style::default().add_modifier(Modifier::DIM)),
        footer,
    );
    overlay
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
    let mut block = Block::bordered().title(format!(
        " プレビュー ─ 幅: {width} ─ シナリオ: {} ─ {overflow} ─ 描画: {mode} ",
        app.scenario().label()
    ));
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
            lines: lines.to_vec(),
        }),
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
    use ratatui::crossterm::event::KeyCode;

    use super::*;
    use crate::preview::width::display_width;

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
        let (screen, overlay) = render(&App::new(), 100, 24);
        let text = screen.join("\n");
        for expected in [
            "プレビュー（読み取り専用）",
            "設定: （テスト）",
            "シナリオ: 通常",
            "描画: ratatui",
            "myproject",
            "Claude Opus",
            "directory",
            "review",
            "（非表示）", // 通常のシナリオでは、レビューは表示されない
            "q: 終了",
        ] {
            assert!(text.contains(expected), "{expected} がない\n{text}");
        }
        assert_eq!(overlay, None, "ratatui 描画では、生 ANSI の出力はない");
    }

    #[test]
    fn test_draw_follows_scenario_and_width() {
        let mut app = App::new();
        app.handle_key(KeyCode::Right); // 長いパス・ブランチ
        let (screen, _) = render(&app, 100, 24);
        assert!(screen.join("\n").contains("シナリオ: 長いパス・ブランチ"));

        // 幅 160 は、実端末（100 列）より広い: その旨を表示する
        app.handle_key(KeyCode::Char('w'));
        app.handle_key(KeyCode::Char('w'));
        app.handle_key(KeyCode::Char('w'));
        let (screen, _) = render(&app, 100, 24);
        let text = screen.join("\n");
        assert!(text.contains("幅: 160"), "{text}");
        assert!(text.contains("横を切り詰めて表示しています"), "{text}");

        // 実端末の幅（auto）では、その表示は出ない
        app.handle_key(KeyCode::Char('w'));
        let (screen, _) = render(&app, 100, 24);
        let text = screen.join("\n");
        assert!(text.contains("幅: auto（98）"), "{text}");
        assert!(!text.contains("横を切り詰めて"), "{text}");
    }

    #[test]
    fn test_draw_raw_mode_returns_overlay_inside_the_border() {
        let mut app = App::new();
        app.handle_key(KeyCode::Char('r'));
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
    fn test_draw_survives_tiny_terminals() {
        // 極端に小さい端末でも panic しない
        for (width, height) in [(1, 1), (10, 5), (20, 8), (40, 12)] {
            for scenario_steps in 0..8 {
                let mut app = App::new();
                for _ in 0..scenario_steps {
                    app.handle_key(KeyCode::Right);
                }
                let _ = render(&app, width, height);
            }
        }
    }
}
