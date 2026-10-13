//! 設定 TUI の画面。状態（`App`）と設定から、描画だけを行う。

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph};

use crate::editor::layout::Token;
use crate::editor::layout_edit::LayoutEditor;
use crate::editor::update::PropValue;
use crate::editor::validate::Severity;
use crate::engine::config::Config;
use crate::engine::parts::PropKind;
use crate::preview::width::{Overflow, display_width, fit_line};
use crate::preview::{layout_visibility, render_part_sample, render_preview};
use crate::tui::app::{App, Focus, WidthChoice};
use crate::tui::palette::{self, PaletteItem};

/// プレビューの最大の表示行数。それ以上は、切り詰めて表示する
const MAX_PREVIEW_ROWS: usize = 8;

/// 部品一覧の、項目名の列の幅（表示幅）
const LABEL_WIDTH: u16 = 20;
/// 項目名の前の印（`> `）の幅
const MARKER_WIDTH: u16 = 2;
/// プロパティ欄の、項目名の列の幅（表示幅）
const PROP_KEY_WIDTH: usize = 20;
/// プロパティ欄で、同時に見せる行数の上限。それ以上は、選択中の行が見える位置までずらす
const MAX_PROP_ROWS: usize = 7;

pub struct ViewContext<'a> {
    /// 編集対象の設定ファイルの説明（ヘッダーに表示する）
    pub source_label: &'a str,
}

/// ratatui の外で、生の ANSI として端末へ出力する内容と位置
#[derive(Debug, PartialEq, Eq)]
pub struct RawOverlay {
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub lines: Vec<String>,
}

/// 画面を描き、ratatui を介さず生の ANSI で出す領域（プレビューと部品の表示例）を返す。
/// 生の ANSI は、ratatui の色の指定（256 色の指標）を通さないので、`render` の出力と同じ色になる
pub fn draw(frame: &mut Frame, app: &App, ctx: &ViewContext) -> Vec<RawOverlay> {
    // 部品一覧にフォーカスがあるときは、選択中の項目を仮に入れた配置を見せる
    let tentative = app.tentative_editor();
    let editor = tentative.as_ref().unwrap_or(app.draft.layout());
    // 編集中の配置を、設定の `format` として描画する（保存するまで、設定ファイルは変わらない）
    // 文字列の入力中は、確定したと仮定した設定で描く（エラーのときは、確定済みの設定）
    let live = app.input_preview().and_then(Result::ok);
    let mut edited = live.unwrap_or_else(|| app.draft.config().clone());
    edited.format = editor.layout().to_format();
    let config = &edited;
    let area = frame.area();
    // auto は、プレビューの枠の内側に収まる幅（実端末の幅から枠を除いたもの）
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

    let props_height = u16::try_from(props_rows(app) + 2).unwrap_or(u16::MAX);
    let [header, preview, middle, props, status, footer] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(preview_height),
        Constraint::Min(3),
        Constraint::Length(props_height),
        Constraint::Length(1),
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

    let mut overlays = vec![draw_preview(frame, app, preview, &lines, columns, too_wide)];
    draw_layout(frame, app, editor, config, layout_area, columns);
    overlays.extend(draw_parts(frame, app, config, parts, columns));
    draw_props(frame, app, props);
    frame.render_widget(status_line(app), status);
    frame.render_widget(
        Line::from(footer_text(app)).style(Style::default().add_modifier(Modifier::DIM)),
        footer,
    );
    overlays
}

fn footer_text(app: &App) -> &'static str {
    if app.input.is_some() {
        return "入力中  Enter: 確定  Esc: 取り消し  ←→ Home End: カーソル  Backspace Delete: 削除";
    }
    match app.focus {
        Focus::Layout => {
            "Tab: プロパティ  ↑↓←→: 選択  Shift+矢印: 移動  d: 削除  n: 改行  a: 行追加  u: 戻す  q: 終了"
        }
        Focus::Parts => {
            "Tab: プレビュー  ↑↓: 部品を選ぶ  Enter: 後ろに追加  i: 前に追加  u: 戻す  q: 終了"
        }
        Focus::Props => {
            "Tab: 部品一覧  ↑↓: 行  Enter/Space: 切替・入力  ←→: 増減・選択（Shift: ±10）  u: 戻す  q: 終了"
        }
        Focus::Preview => "Tab: 配置  ←/→: シナリオ  w: 幅  o: 折返/切詰  u: 戻す  q: 終了",
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
) -> RawOverlay {
    let width = match app.width_choice() {
        WidthChoice::Auto => format!("auto（{columns}）"),
        WidthChoice::Columns(n) => n.to_string(),
    };
    let overflow = match app.overflow {
        Overflow::Wrap => "折り返し",
        Overflow::Truncate => "切り詰め",
    };
    let mut block = focus_block(
        format!(
            " プレビュー ─ 幅: {width} ─ シナリオ: {} ─ {overflow} ",
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

    RawOverlay {
        x: inner.x,
        y: inner.y,
        width: inner.width,
        lines: clip_to_area(lines, inner),
    }
}

/// 生 ANSI で端末へ直接出す行を、枠の内側（`area`）に収める。
/// 行数は高さまで、各行は幅までにし、幅より広い 1 文字しか入らない行は空にする。
/// 改行などの制御文字は、カーソルを領域の外へ動かすので、見える文字に置き換える
/// （部品の `format` に `\n` を書けるため、部品の表示例にも現れる）。
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
            let clipped =
                fit_line(&defang_controls(line), width, Overflow::Truncate).swap_remove(0);
            if display_width(&clipped) <= width {
                clipped
            } else {
                String::new()
            }
        })
        .collect()
}

/// 改行（`\n`、`\r`）は `⏎`、それ以外の制御文字は `\u{fffd}` にする。装飾の開始（ESC）は残す
fn defang_controls(line: &str) -> String {
    line.chars()
        .map(|c| match c {
            '\n' | '\r' => '⏎',
            '\x1b' => c,
            c if c.is_control() => '\u{fffd}',
            c => c,
        })
        .collect()
}

/// 配置の編集領域。行ごとに、部品と固定テキストを並べ、選択中のトークンを示す。
/// 何も表示しない部品は薄く示し、出力されない行にはその旨を添える（設計書 §6.5）。
/// 部品一覧にフォーカスがあるときは、`editor` が仮に入れた状態で、入れたトークンを緑の下線で示す
fn draw_layout(
    frame: &mut Frame,
    app: &App,
    editor: &LayoutEditor,
    config: &Config,
    area: Rect,
    columns: usize,
) {
    let block = focus_block(" 配置 ".to_string(), app.focus == Focus::Layout);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let dim = Style::default().add_modifier(Modifier::DIM);
    let layout = editor.layout();
    let cursor = editor.cursor();
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
                    style = match app.focus {
                        Focus::Layout => style.add_modifier(Modifier::REVERSED),
                        Focus::Parts => Style::default()
                            .fg(Color::Green)
                            .add_modifier(Modifier::UNDERLINED),
                        Focus::Props | Focus::Preview => style.add_modifier(Modifier::UNDERLINED),
                    };
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

/// プロパティ欄の、見せる行数（対象の部品がないときは、案内の 1 行）
fn props_rows(app: &App) -> usize {
    app.target_part()
        .map_or(1, |part| part.props.len().min(MAX_PROP_ROWS))
}

/// プロパティ欄。対象の部品の設定キーを、現在の値とともに並べる。
/// 既定値と異なる値には、既定値を添える。文字列の入力中は、その行に入力中の内容とカーソルを出す
fn draw_props(frame: &mut Frame, app: &App, area: Rect) {
    let focused = app.focus == Focus::Props;
    let Some(part) = app.target_part() else {
        let block = focus_block(" プロパティ ".to_string(), focused);
        let inner = block.inner(area);
        frame.render_widget(block, area);
        frame.render_widget(
            Paragraph::new(Span::styled(
                "（配置または部品一覧で部品を選ぶと、その部品のプロパティを編集できます）",
                Style::default().add_modifier(Modifier::DIM),
            )),
            inner,
        );
        return;
    };
    let block = focus_block(format!(" プロパティ: {} ", part.id), focused);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let height = usize::from(inner.height);
    let selected = app.props_index();
    let offset = (selected + 1).saturating_sub(height);
    let dim = Style::default().add_modifier(Modifier::DIM);

    let rows: Vec<Line> = part
        .props
        .iter()
        .enumerate()
        .skip(offset)
        .take(height)
        .map(|(index, prop)| {
            let is_selected = index == selected;
            let key = format!("{:<width$}", prop.key, width = PROP_KEY_WIDTH);
            let key_style = if is_selected && focused {
                Style::default().add_modifier(Modifier::REVERSED)
            } else {
                Style::default()
            };
            let mut spans = vec![
                Span::raw(if is_selected { "> " } else { "  " }),
                Span::styled(key, key_style),
                Span::raw(" "),
            ];
            let editing = app
                .input
                .as_ref()
                .filter(|input| input.part == part.id && input.key == prop.key);
            if let Some(input) = editing {
                spans.extend(input_spans(&input.value(), input.cursor()));
                return Line::from(spans);
            }
            let current = app.draft.prop_value(part.id, prop.key);
            spans.push(Span::raw(
                current.as_ref().map_or_else(String::new, show_value),
            ));
            if let PropKind::Choice(choices) = prop.kind {
                spans.push(Span::styled(format!("  [{}]", choices.join("|")), dim));
            }
            let default = app.draft.default_value(part.id, prop.key);
            if current != default
                && let Some(default) = default
            {
                let note = format!("  （既定: {}）", show_value(&default));
                spans.push(Span::styled(note, dim));
            }
            Line::from(spans)
        })
        .collect();
    frame.render_widget(Paragraph::new(rows), inner);
}

/// 値の表示。文字列は引用符で囲み、前後の空白が見えるようにする
fn show_value(value: &PropValue) -> String {
    match value {
        PropValue::Str(text) => format!("\"{text}\""),
        PropValue::Int(number) => number.to_string(),
        PropValue::Bool(flag) => flag.to_string(),
    }
}

/// 入力中の文字列。カーソルの位置の文字（末尾なら空白）を反転する
fn input_spans(value: &str, cursor: usize) -> Vec<Span<'static>> {
    let chars: Vec<char> = value.chars().collect();
    let before: String = chars.iter().take(cursor).collect();
    let at = chars.get(cursor).map_or(" ".to_string(), char::to_string);
    let after: String = chars.iter().skip(cursor + 1).collect();
    vec![
        Span::raw("["),
        Span::raw(before),
        Span::styled(at, Style::default().add_modifier(Modifier::REVERSED)),
        Span::raw(after),
        Span::raw("]"),
    ]
}

/// 状態行。入力中に受け付けられない値になっていれば、その理由、直前の操作が受け付けられなかった理由、
/// 検証の結果（エラーと警告の件数と、先頭の 1 件）の順に出す
fn status_line(app: &App) -> Line<'static> {
    let red = Style::default().fg(Color::Red);
    if let Some(Err(err)) = app.input_preview() {
        return Line::from(Span::styled(
            format!("入力は受け付けられません: {err}"),
            red,
        ));
    }
    if let Some(message) = &app.message {
        return Line::from(Span::styled(format!("受け付けられません: {message}"), red));
    }
    let issues = app.draft.issues();
    if issues.is_empty() {
        return Line::from(Span::styled(
            "検証: 問題なし",
            Style::default().fg(Color::Green),
        ));
    }
    let errors = issues
        .iter()
        .filter(|issue| issue.severity == Severity::Error)
        .count();
    let warnings = issues.len() - errors;
    let first = issues
        .iter()
        .find(|issue| issue.severity == Severity::Error)
        .unwrap_or(&issues[0]);
    let location = if first.path.is_empty() {
        String::new()
    } else {
        format!("{}: ", first.path)
    };
    let style = if errors > 0 {
        red
    } else {
        Style::default().fg(Color::Yellow)
    };
    Line::from(Span::styled(
        format!(
            "検証: エラー {errors} 件 / 警告 {warnings} 件 — {location}{}",
            first.message
        ),
        style,
    ))
}

/// 部品一覧。項目名に続けて、選択中のシナリオでの表示例を、生の ANSI で出す。
/// 項目が領域に収まらないときは、選択中の項目が見える位置までずらす
fn draw_parts(
    frame: &mut Frame,
    app: &App,
    config: &Config,
    area: Rect,
    columns: usize,
) -> Vec<RawOverlay> {
    let block = focus_block(
        format!(" 部品 ─ {}のサンプル ", app.scenario().label()),
        app.focus == Focus::Parts,
    );
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let height = usize::from(inner.height);
    let selected = app.palette_index();
    let offset = (selected + 1).saturating_sub(height);
    let sample_x = inner.x.saturating_add(MARKER_WIDTH + LABEL_WIDTH + 1);
    let sample_width = inner.right().saturating_sub(sample_x);
    let dim = Style::default().add_modifier(Modifier::DIM);

    let mut rows = Vec::new();
    let mut overlays = Vec::new();
    for (row, (index, item)) in (0..).zip(app.palette.iter().enumerate().skip(offset).take(height))
    {
        let is_selected = index == selected;
        let label = palette::label(*item);
        let padding = " ".repeat(usize::from(LABEL_WIDTH).saturating_sub(display_width(&label)));
        let label_style = if is_selected && app.focus == Focus::Parts {
            Style::default().add_modifier(Modifier::REVERSED)
        } else {
            Style::default()
        };
        let mut spans = vec![
            Span::raw(if is_selected { "> " } else { "  " }),
            Span::styled(label, label_style),
            Span::raw(format!("{padding} ")),
        ];
        match item {
            PaletteItem::Part(id) => {
                let sample = render_part_sample(config, app.scenario(), id, columns)
                    .filter(|sample| !sample.is_empty());
                match sample {
                    // 項目名の列で領域が埋まっているときは、表示例を出さない
                    Some(_) if sample_width == 0 => {}
                    Some(sample) => {
                        let y = inner.y + row;
                        overlays.push(RawOverlay {
                            x: sample_x,
                            y,
                            width: sample_width,
                            lines: clip_to_area(&[sample], Rect::new(sample_x, y, sample_width, 1)),
                        });
                    }
                    None => spans.push(Span::styled("（非表示）", dim)),
                }
            }
            PaletteItem::Literal(_) => {}
            PaletteItem::LineBreak => spans.push(Span::styled("（行を分ける）", dim)),
        }
        rows.push(Line::from(spans));
    }
    frame.render_widget(Paragraph::new(rows), inner);
    overlays
}

#[cfg(test)]
mod tests {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::crossterm::event::{KeyCode, KeyEvent};

    use super::*;
    use crate::editor::draft::Draft;
    use crate::engine::config::DEFAULT_FORMAT;

    fn new_app() -> App {
        App::new(Draft::with_format(DEFAULT_FORMAT))
    }

    fn app_with_focus(format: &str, focus: Focus) -> App {
        let mut app = App::new(Draft::with_format(format));
        app.focus = focus;
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

    fn render(app: &App, width: u16, height: u16) -> (Vec<String>, Vec<RawOverlay>) {
        let ctx = ViewContext {
            source_label: "（テスト）",
        };
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        let mut overlays = Vec::new();
        terminal
            .draw(|frame| overlays = draw(frame, app, &ctx))
            .unwrap();
        (screen(&terminal), overlays)
    }

    /// 生 ANSI の出力の文字列（装飾を除く）。プレビューは最初、部品の表示例は項目ごとに 1 行
    fn overlay_text(overlays: &[RawOverlay]) -> String {
        overlays
            .iter()
            .flat_map(|overlay| overlay.lines.iter())
            .map(|line| crate::preview::width::strip_ansi(line))
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn test_draw_shows_header_layout_parts_and_footer() {
        let (screen, overlays) = render(&new_app(), 100, 40);
        let text = screen.join("\n");
        for expected in [
            "配置の編集（まだ保存できません）",
            "設定: （テスト）",
            "シナリオ: 通常",
            "directory", // 配置領域と部品一覧の両方
            "review",
            "（非表示）", // 通常のシナリオでは、レビューは表示されない
            " 1: ",
            "Shift+矢印: 移動", // 配置にフォーカスがある
            "q: 終了",
        ] {
            assert!(text.contains(expected), "{expected} がない\n{text}");
        }
        // プレビューと、部品の表示例は、生 ANSI で出す（ratatui の画面には含まれない）
        let raw = overlay_text(&overlays);
        for expected in ["myproject", "Claude Opus", "🌿 main"] {
            assert!(raw.contains(expected), "{expected} がない\n{raw}");
        }
        assert!(!text.contains("Claude Opus"), "{text}");
    }

    #[test]
    fn test_draw_follows_scenario_and_width() {
        let mut app = app_with_focus(DEFAULT_FORMAT, Focus::Preview);
        press(&mut app, KeyCode::Right); // 長いパス・ブランチ
        let (screen, _) = render(&app, 100, 24);
        assert!(screen.join("\n").contains("シナリオ: 長いパス・ブランチ"));

        // 幅 160 は、実端末（100 列）より広い: その旨を表示する
        for _ in 0..3 {
            press(&mut app, KeyCode::Char('w'));
        }
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
    fn test_draw_preview_overlay_is_inside_the_border() {
        let (screen, overlays) = render(&new_app(), 100, 24);
        let preview = &overlays[0];
        // ヘッダー 1 行、枠 1 行の次から、枠の内側の幅で出す
        assert_eq!((preview.x, preview.y, preview.width), (1, 2, 98));
        assert_eq!(preview.lines.len(), 3);
        assert!(preview.lines[0].contains("\x1b["), "{:?}", preview.lines[0]);
        // 枠の内側は ratatui では描かない（端末へ直接出す）
        for row in &screen[2..5] {
            assert!(!row.contains("myproject"), "{row}");
        }
    }

    #[test]
    fn test_draw_parts_samples_follow_the_rows() {
        let (screen, overlays) = render(&new_app(), 100, 30);
        // 表示例は、項目の行（画面上の同じ行）の、項目名の右に出す
        let sample_rows: Vec<u16> = overlays[1..].iter().map(|overlay| overlay.y).collect();
        let directory_row = screen
            .iter()
            .position(|row| row.contains("> directory"))
            .expect("選択中の項目");
        assert!(sample_rows.contains(&u16::try_from(directory_row).unwrap()));
        let first = &overlays[1];
        assert!(first.lines[0].contains("myproject"), "{:?}", first.lines);
        // 項目名の列（印 + 名前の幅 + 区切り）より右に出す
        assert!(first.x > MARKER_WIDTH + LABEL_WIDTH);
        // 非表示の項目（レビュー）には、表示例の領域を出さない
        assert!(overlays[1..].len() < app_part_count());
    }

    fn app_part_count() -> usize {
        crate::engine::parts::part_ids().len()
    }

    #[test]
    fn test_overlays_stay_inside_the_terminal() {
        // 小さい端末・縮小リサイズでも、生 ANSI の出力は端末（行数・各行の幅）に収まり、
        // 部品一覧の選択が末尾にあっても同様
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
            for focus in [Focus::Layout, Focus::Parts, Focus::Props, Focus::Preview] {
                for steps in 0..8 {
                    let mut app = app_with_focus(DEFAULT_FORMAT, focus);
                    for _ in 0..steps * 3 {
                        press(&mut app, KeyCode::Down);
                    }
                    app.focus = focus;
                    let (_, overlays) = render(&app, width, height);
                    for overlay in &overlays {
                        let (cols, rows) = (usize::from(overlay.width), overlay.lines.len());
                        let context = format!("{width}x{height}/{focus:?}/{steps}: {overlay:?}");
                        assert!(overlay.x + overlay.width <= width, "{context}");
                        assert!(
                            overlay.lines.is_empty()
                                || overlay.y + u16::try_from(rows).unwrap() <= height,
                            "{context}: 行数が高さを超える"
                        );
                        for line in &overlay.lines {
                            assert!(display_width(line) <= cols, "{context}: 幅を超える");
                        }
                        if overlay.width == 0 {
                            assert!(overlay.lines.is_empty(), "{context}: 幅 0 では出力しない");
                        }
                    }
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
    fn test_clip_to_area_neutralizes_control_characters() {
        let area = Rect::new(0, 0, 20, 1);
        // (名前, 入力, 期待する結果)
        let cases = [
            ("lf", "A\nB", "A⏎B"),
            ("crlf", "A\r\nB", "A⏎⏎B"),
            ("tab-and-bell", "A\tB\x07", "A\u{fffd}B\u{fffd}"),
            // 装飾の開始は残す
            ("sgr-kept", "\x1b[1mA\x1b[0m", "\x1b[1mA\x1b[0m"),
        ];
        for (name, input, expected) in cases {
            assert_eq!(
                clip_to_area(&[input.to_string()], area),
                vec![expected.to_string()],
                "{name}"
            );
        }
    }

    #[test]
    fn test_part_sample_with_newline_stays_on_one_row() {
        // 部品の `format` に改行を書いても、表示例は 1 行の領域に収まる
        let mut config = Config::default();
        config.directory.format = "A\nB".to_string();
        let ctx = ViewContext {
            source_label: "（テスト）",
        };
        let app = App::new(Draft::new(config, toml_edit::DocumentMut::new()));
        let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
        let mut overlays = Vec::new();
        terminal
            .draw(|frame| overlays = draw(frame, &app, &ctx))
            .unwrap();
        // 部品の表示例は、プレビューの次から。どの overlay にも、改行・復帰は含まれない
        let sample = &overlays[1];
        assert_eq!(sample.lines.len(), 1);
        assert!(sample.lines[0].contains("A⏎B"), "{:?}", sample.lines);
        for overlay in &overlays {
            for line in &overlay.lines {
                assert!(!line.contains(['\n', '\r']), "{line:?}");
            }
        }
    }

    #[test]
    fn test_draw_survives_tiny_terminals() {
        // 極端に小さい端末でも panic しない
        for (width, height) in [(1, 1), (10, 5), (20, 8), (40, 12)] {
            for focus in [Focus::Layout, Focus::Parts, Focus::Props, Focus::Preview] {
                for scenario_steps in 0..8 {
                    let mut app = app_with_focus(DEFAULT_FORMAT, Focus::Preview);
                    for _ in 0..scenario_steps {
                        press(&mut app, KeyCode::Right);
                    }
                    app.focus = focus;
                    let _ = render(&app, width, height);
                }
            }
        }
    }

    #[test]
    fn test_draw_layout_shows_tokens_and_hidden_notes() {
        let mut app = app_with_focus("$directory | $review\n\n$review", Focus::Layout);
        let (screen, _) = render(&app, 100, 24);
        let text = screen.join("\n");
        // 固定テキストは引用符で囲み、空白は `␣` にする
        assert!(text.contains("directory \"␣|␣\" review"), "{text}");
        assert!(text.contains("（空の行）"), "{text}");
        // 出力されない行（レビューだけの行と、空の行）に、その旨を添える
        assert_eq!(text.matches("この行は出力されません").count(), 2, "{text}");

        // プレビューにフォーカスを移すと、配置の操作ではなく、プレビューの操作を案内する
        app.focus = Focus::Preview;
        let (screen, _) = render(&app, 100, 24);
        let text = screen.join("\n");
        assert!(text.contains("w: 幅"), "{text}");
        assert!(!text.contains("Shift+矢印"), "{text}");
    }

    #[test]
    fn test_draw_preview_follows_the_edited_layout() {
        let mut app = App::new(Draft::with_format("$model $quota"));
        let (_, overlays) = render(&app, 100, 24);
        assert!(overlay_text(&overlays[..1]).contains("Claude Opus"));

        // 選択中（先頭）の `$model` を消すと、プレビューにも反映される。設定自体は変わらない
        press(&mut app, KeyCode::Char('d'));
        let (_, overlays) = render(&app, 100, 24);
        assert!(!overlay_text(&overlays[..1]).contains("Claude Opus"));

        press(&mut app, KeyCode::Char('u'));
        let (_, overlays) = render(&app, 100, 24);
        assert!(overlay_text(&overlays[..1]).contains("Claude Opus"));
    }

    #[test]
    fn test_parts_focus_previews_the_tentative_insertion() {
        let mut app = app_with_focus("$directory", Focus::Parts);
        let quota = app
            .palette
            .iter()
            .position(|item| *item == PaletteItem::Part("quota"))
            .unwrap();
        for _ in 0..quota {
            press(&mut app, KeyCode::Down);
        }
        let (screen, overlays) = render(&app, 100, 30);
        let text = screen.join("\n");
        // 配置領域にも、プレビューにも、仮に入れた状態が出る。部品一覧の選択位置を示す
        assert!(text.contains("directory quota"), "{text}");
        assert!(text.contains("> quota"), "{text}");
        assert!(text.contains("後ろに追加"), "{text}");
        assert!(
            overlay_text(&overlays[..1]).contains("5h:"),
            "クォータが見える"
        );
        // 確定するまで、配置は変わらない
        assert_eq!(app.draft.layout().layout().to_format(), "$directory");

        // 確定すると、フォーカスを移しても残る
        press(&mut app, KeyCode::Enter);
        app.focus = Focus::Layout;
        let (screen, overlays) = render(&app, 100, 30);
        assert!(screen.join("\n").contains("directory quota"));
        assert!(overlay_text(&overlays[..1]).contains("5h:"));
    }

    #[test]
    fn test_parts_list_scrolls_to_keep_the_selection_visible() {
        let mut app = app_with_focus(DEFAULT_FORMAT, Focus::Parts);
        for _ in 0..app.palette.len() {
            press(&mut app, KeyCode::Down);
        }
        // 部品一覧の枠の内側が 3 行しかなくても、末尾の項目（改行）が見える
        let (screen, _) = render(&app, 100, 12);
        let text = screen.join("\n");
        assert!(text.contains("> 改行"), "{text}");
        assert!(!text.contains("> directory"), "{text}");
    }

    /// 配置の先頭が `format` のアプリで、プロパティ欄にフォーカスを移す
    fn props_app(format: &str) -> App {
        let mut app = App::new(Draft::with_format(format));
        press(&mut app, KeyCode::Tab);
        assert_eq!(app.focus, Focus::Props);
        app
    }

    fn text(app: &App, width: u16, height: u16) -> String {
        render(app, width, height).0.join("\n")
    }

    #[test]
    fn test_props_pane_lists_keys_values_and_defaults() {
        let mut app = props_app("$directory $context");
        let shown = text(&app, 110, 40);
        for expected in [
            "プロパティ: directory",
            "truncation_mode",
            "[left|middle]", // 選択肢
            "\"middle\"",
            "検証: 問題なし",
        ] {
            assert!(shown.contains(expected), "{expected} がない\n{shown}");
        }
        assert!(
            !shown.contains("（既定:"),
            "既定値のままなら、既定値は添えない\n{shown}"
        );

        // 選択を `$context` へ移し、閾値を変えると、既定値が添えられる
        app.focus = Focus::Layout;
        for _ in 0..2 {
            press(&mut app, KeyCode::Right);
        }
        app.focus = Focus::Props;
        for _ in 0..3 {
            press(&mut app, KeyCode::Down);
        }
        press(&mut app, KeyCode::Right);
        let shown = text(&app, 110, 40);
        assert!(shown.contains("プロパティ: context"), "{shown}");
        assert!(shown.contains("threshold_warning"), "{shown}");
        assert!(shown.contains("51  （既定: 50）"), "{shown}");
        assert!(shown.contains("Enter/Space: 切替・入力"), "{shown}");
    }

    #[test]
    fn test_props_pane_without_a_target_part_shows_guidance() {
        let mut app = App::new(Draft::with_format("$directory |"));
        press(&mut app, KeyCode::Right); // 固定テキスト
        let shown = text(&app, 110, 40);
        assert!(
            shown.contains("部品を選ぶと、その部品のプロパティを編集できます"),
            "{shown}"
        );
    }

    #[test]
    fn test_props_pane_follows_the_parts_list_selection() {
        let mut app = App::new(Draft::with_format("$directory"));
        press(&mut app, KeyCode::BackTab); // プレビュー
        press(&mut app, KeyCode::BackTab); // 部品一覧
        for _ in 0..4 {
            press(&mut app, KeyCode::Down); // model
        }
        press(&mut app, KeyCode::BackTab); // 部品一覧から入るプロパティ欄は、一覧の項目の部品
        assert_eq!(app.focus, Focus::Props);
        let shown = text(&app, 110, 40);
        assert!(shown.contains("プロパティ: model"), "{shown}");
    }

    #[test]
    fn test_text_input_is_shown_and_previewed_live() {
        let mut app = props_app("$git_branch");
        press(&mut app, KeyCode::Down); // symbol
        press(&mut app, KeyCode::Enter);
        // 入力中の内容とカーソルが、その行に出る。案内も入力用になる
        let shown = text(&app, 110, 40);
        assert!(shown.contains("symbol"), "{shown}");
        assert!(shown.contains("[🌿  ]"), "{shown}");
        assert!(shown.contains("入力中  Enter: 確定"), "{shown}");

        // 入力すると、確定前でも、プレビューに反映される。下書きは変わらない
        press(&mut app, KeyCode::Backspace);
        press(&mut app, KeyCode::Backspace);
        press(&mut app, KeyCode::Char('Z'));
        let (_, overlays) = render(&app, 110, 40);
        let preview = overlay_text(&overlays[..1]);
        assert!(preview.contains("Zmain"), "{preview}");
        assert_eq!(app.draft.config().git_branch.symbol, "🌿 ");

        // Esc で取り消すと、元に戻る
        press(&mut app, KeyCode::Esc);
        let (_, overlays) = render(&app, 110, 40);
        assert!(overlay_text(&overlays[..1]).contains("🌿 main"));
    }

    #[test]
    fn test_invalid_input_shows_the_reason_and_keeps_the_last_good_preview() {
        let mut app = props_app("$git_branch");
        for _ in 0..2 {
            press(&mut app, KeyCode::Down); // style
        }
        press(&mut app, KeyCode::Enter);
        for c in " nonexistent".chars() {
            press(&mut app, KeyCode::Char(c));
        }
        let (screen, overlays) = render(&app, 110, 40);
        let shown = screen.join("\n");
        assert!(shown.contains("入力は受け付けられません"), "{shown}");
        assert!(shown.contains("nonexistent"), "{shown}");
        // プレビューは、確定済みの設定のまま
        assert!(overlay_text(&overlays[..1]).contains("main"));

        // Enter で拒否されたときの理由
        press(&mut app, KeyCode::Enter);
        assert!(app.input.is_some());
        assert!(text(&app, 110, 40).contains("受け付けられません"));
    }

    #[test]
    fn test_status_line_reports_existing_issues() {
        let toml = "[nothing]\nx = 1\n\n[model]\nstyle = \"cyna\"\n";
        let config = toml_edit::de::from_str::<Config>(toml).unwrap();
        let app = App::new(Draft::new(config, toml.parse().unwrap()));
        let shown = text(&app, 130, 40);
        // エラーを先に出す
        assert!(
            shown.contains(
                "検証: エラー 1 件 / 警告 1 件 — model.style: 認識できないスタイル名: cyna"
            ),
            "{shown}"
        );
    }

    #[test]
    fn test_status_line_reports_warnings_when_there_are_no_errors() {
        let toml = "[nothing]\nx = 1\n";
        let config = toml_edit::de::from_str::<Config>(toml).unwrap();
        let app = App::new(Draft::new(config, toml.parse().unwrap()));
        let shown = text(&app, 130, 40);
        assert!(
            shown.contains("検証: エラー 0 件 / 警告 1 件 — nothing: "),
            "{shown}"
        );
    }

    #[test]
    fn test_props_rows_are_capped_and_scroll_to_the_selection() {
        // quota は 9 個のプロパティを持つ。上限の行数で、選択中の行が見えるようにずらす
        let mut app = props_app("$quota");
        for _ in 0..8 {
            press(&mut app, KeyCode::Down);
        }
        let shown = text(&app, 110, 50);
        assert!(shown.contains("> disabled"), "{shown}");
        assert!(
            !shown.contains("  format "),
            "先頭の行は、上へずれて見えない\n{shown}"
        );
    }
}
