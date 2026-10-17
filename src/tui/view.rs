//! 設定 TUI の画面。状態（`App`）と設定から、描画だけを行う。

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph};

use crate::editor::layout::Token;
use crate::editor::layout_edit::LayoutEditor;
use crate::editor::save::ConflictKind;
use crate::editor::update::PropValue;
use crate::editor::validate::Severity;
use crate::engine::config::Config;
use crate::engine::parts::PropKind;
use crate::preview::width::{Overflow, display_width, fit_line, symbol_notes};
use crate::preview::{layout_visibility, render_part_sample, render_preview};
use crate::tui::app::{App, Focus, InputTarget, Mode, WidthChoice};
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
pub fn draw(frame: &mut Frame, app: &App) -> Vec<RawOverlay> {
    // 部品一覧にフォーカスがあるときは、選択中の項目を仮に入れた配置を見せる
    let tentative = app.tentative_editor();
    let editor = tentative.as_ref().unwrap_or(app.draft.layout());
    // 編集中の配置を、設定の `format` として描画する（保存するまで、設定ファイルは変わらない）
    // 文字列の入力中は、確定したと仮定した設定で描く（エラーのときは、確定済みの設定）
    let live = app.live_preview().and_then(Result::ok);
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
    let status_rows = status_lines(app, usize::from(area.width));
    let status_height = u16::try_from(status_rows.len()).unwrap_or(1);
    let [header, preview, middle, props, status, footer] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(preview_height),
        Constraint::Min(3),
        Constraint::Length(props_height),
        Constraint::Length(status_height),
        Constraint::Length(1),
    ])
    .areas(area);
    let [layout_area, parts] =
        Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)]).areas(middle);

    frame.render_widget(
        Line::from(format!(
            "agent-statusline config — {}  設定: {}",
            if app.is_dirty() {
                "未保存の変更あり"
            } else {
                "変更なし"
            },
            app.source_label()
        ))
        .style(Style::default().add_modifier(Modifier::BOLD)),
        header,
    );

    let mut overlays = vec![draw_preview(frame, app, preview, &lines, columns, too_wide)];
    if app.issues_view.is_some() {
        // 検証結果の一覧は、配置と部品一覧の場所いっぱいに出す（部品の表示例の生 ANSI も出さない）
        draw_issues(frame, app, middle);
    } else {
        draw_layout(frame, app, editor, config, layout_area, columns);
        if app.picker.is_some() {
            // 記号の候補を選んでいる間は、部品一覧の場所に候補を出す（部品の表示例の生 ANSI と重ならない）
            draw_picker(frame, app, parts);
        } else {
            overlays.extend(draw_parts(frame, app, config, parts, columns));
        }
    }
    draw_props(frame, app, props);
    frame.render_widget(Paragraph::new(status_rows), status);
    frame.render_widget(
        Line::from(footer_text(app)).style(Style::default().add_modifier(Modifier::DIM)),
        footer,
    );
    overlays
}

fn footer_text(app: &App) -> &'static str {
    if let Some(input) = &app.input {
        return match input.target {
            InputTarget::Prop { .. } => {
                "入力中  Enter: 確定  Esc: 取り消し  ←→ Home End: カーソル  Backspace Delete: 削除"
            }
            InputTarget::SavePath => {
                "別名で保存  Enter: 保存  Esc: 取り消し  ←→ Home End: カーソル  Backspace Delete: 削除"
            }
        };
    }
    if app.picker.is_some() {
        return "記号を選ぶ  ↑↓: 選ぶ  Enter: 決定  e: 自由に入力  Esc: 閉じる";
    }
    if app.issues_view.is_some() {
        return "検証結果  ↑↓: 選ぶ（長い項目は 1 行ずつスクロール）  Esc / v: 閉じる";
    }
    // 保存と終了は、端末が狭くて切れても見えるよう、先頭に置く
    match app.focus {
        Focus::Layout => {
            "s: 保存  q: 終了  v: 検証結果  Tab: プロパティ  ↑↓←→: 選択  Shift+矢印: 移動  d: 削除  n: 改行  a: 行追加  u: 戻す"
        }
        Focus::Parts => {
            "s: 保存  q: 終了  v: 検証結果  Tab: プレビュー  ↑↓: 部品を選ぶ  Enter: 後ろに追加  i: 前に追加  u: 戻す"
        }
        Focus::Props => {
            "s: 保存  q: 終了  v: 検証結果  Tab: 部品一覧  ↑↓: 行  Enter/Space: 切替・入力  ←→: 増減・選択（Shift: ±10）  u: 戻す"
        }
        Focus::Preview => {
            "s: 保存  q: 終了  v: 検証結果  Tab: 配置  ←/→: シナリオ  w: 幅  o: 折返/切詰  u: 戻す"
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
            let target = InputTarget::Prop {
                part: part.id,
                key: prop.key,
            };
            let editing = app.input.as_ref().filter(|input| input.target == target);
            if let Some(input) = editing {
                // 行頭の印（`> `）、項目名、区切りの空白を除いた、入力欄に使える幅
                let room = usize::from(inner.width).saturating_sub(2 + PROP_KEY_WIDTH + 1);
                spans.extend(input_spans(&input.value(), input.cursor(), room));
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

/// 入力中の文字列。カーソルの位置の文字（末尾なら空白）を反転する。
/// `width`（`[` `]` を含む表示幅）に収まらないときは、カーソルが見える範囲へ横にずらし、
/// 切れた側の端に `…` を出す
fn input_spans(value: &str, cursor: usize, width: usize) -> Vec<Span<'static>> {
    let chars: Vec<char> = value.chars().collect();
    let char_width = |c: char| display_width(&c.to_string());
    let cursor_cell = chars.get(cursor).map_or(1, |c| char_width(*c).max(1));
    // カーソルの前の文字を、後ろから詰めて、カーソルが収まる最小の開始位置を選ぶ
    let inner = width.saturating_sub(2).max(cursor_cell);
    let fits = |start: usize, room: usize| {
        let before: usize = chars[start..cursor.min(chars.len())]
            .iter()
            .map(|c| char_width(*c))
            .sum();
        before + cursor_cell <= room
    };
    // カーソルの後ろに文字があるときは、右を切る場合の `…` の 1 列を、常に空けておく
    let reserve = usize::from(cursor + 1 < chars.len());
    let mut start = 0;
    let mut lead = 0;
    if !fits(0, inner.saturating_sub(reserve).max(cursor_cell)) {
        // 左を切るので、`…` の 1 列を引く
        lead = 1;
        let room = inner.saturating_sub(lead + reserve).max(cursor_cell);
        while start < cursor && !fits(start, room) {
            start += 1;
        }
    }
    let before: String = chars[start..cursor.min(chars.len())].iter().collect();
    let at = chars.get(cursor).map_or(" ".to_string(), char::to_string);
    let used = lead + display_width(&before) + cursor_cell;
    let room = inner.saturating_sub(used);

    // カーソルの後ろは、収まる分だけ。残りがあれば、最後の 1 列を `…` にする
    let rest = chars.get(cursor + 1..).unwrap_or_default();
    let rest_width: usize = rest.iter().map(|c| char_width(*c)).sum();
    let (after, trail) = if rest_width <= room {
        (rest.iter().collect::<String>(), false)
    } else {
        let mut taken = String::new();
        let mut taken_width = 0;
        for c in rest {
            if taken_width + char_width(*c) + 1 > room {
                break;
            }
            taken.push(*c);
            taken_width += char_width(*c);
        }
        (taken, true)
    };

    let dim = Style::default().add_modifier(Modifier::DIM);
    let mut spans = vec![Span::raw("[")];
    if lead > 0 {
        spans.push(Span::styled("…", dim));
    }
    spans.push(Span::raw(before));
    spans.push(Span::styled(
        at,
        Style::default().add_modifier(Modifier::REVERSED),
    ));
    spans.push(Span::raw(after));
    if trail {
        spans.push(Span::styled("…", dim));
    }
    spans.push(Span::raw("]"));
    spans
}

/// 状態の行。別名で保存するパスの入力中は 2 行（上: 保存できなかった理由か案内、下: 入力）、
/// それ以外は 1 行（`status_line`）。理由が入力欄に押し出されて、画面の外に切れないようにする
fn status_lines(app: &App, width: usize) -> Vec<Line<'static>> {
    let Some(input) = &app.input else {
        return vec![status_line(app)];
    };
    if input.target != InputTarget::SavePath {
        return vec![status_line(app)];
    }
    let label = "別名で保存するパス: ";
    let room = width.saturating_sub(display_width(label));
    let mut spans = vec![Span::raw(label)];
    spans.extend(input_spans(&input.value(), input.cursor(), room));
    let first = match &app.message {
        Some(message) => Line::from(Span::styled(
            format!("受け付けられません: {message}"),
            Style::default().fg(Color::Red),
        )),
        None => Line::from(Span::styled(
            "保存先のパスを入力してください（既にあるファイルは上書きしません）",
            Style::default().add_modifier(Modifier::DIM),
        )),
    };
    vec![first, Line::from(spans)]
}

/// 状態行。優先順位は、(1) 確認・選択（未保存で終了、保存の競合）、
/// (2) 入力中に受け付けられない値の理由、(3) 直前の操作が受け付けられなかった理由、(4) 直前の操作の知らせ、
/// (5) 検証の結果（エラーと警告の件数と、先頭の 1 件）
fn status_line(app: &App) -> Line<'static> {
    let red = Style::default().fg(Color::Red);
    let yellow = Style::default().fg(Color::Yellow);
    let text = |text: String, style: Style| Line::from(Span::styled(text, style));

    match app.mode {
        Mode::ConfirmQuit => {
            return text(
                "未保存の変更があります。 y: 保存せず終了  s: 保存して終了  その他のキー: 戻る"
                    .to_string(),
                yellow,
            );
        }
        Mode::Conflict(kind) => {
            let reason = match kind {
                ConflictKind::Changed => "設定ファイルが外部で変更されました",
                ConflictKind::Deleted => "設定ファイルが外部で削除されました",
                ConflictKind::Created => "設定ファイルが他で作成されました",
            };
            return text(
                format!(
                    "{reason}。 r: 再読み込み（今の編集を破棄）  a: 別名で保存  その他のキー: 戻る"
                ),
                red,
            );
        }
        Mode::Normal => {}
    }
    if let Some(Err(err)) = app.live_preview() {
        return text(format!("入力は受け付けられません: {err}"), red);
    }
    if let Some(message) = &app.message {
        return text(format!("受け付けられません: {message}"), red);
    }
    if let Some(notice) = &app.notice {
        return text(notice.clone(), Style::default().fg(Color::Green));
    }
    if let Some(symbol) = app.symbol_in_progress() {
        let (text_line, style) = symbol_info(&symbol);
        return text(text_line, style);
    }
    let issues = app.draft.issues();
    if issues.is_empty() {
        return text(
            "検証: 問題なし".to_string(),
            Style::default().fg(Color::Green),
        );
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
    let style = if errors > 0 { red } else { yellow };
    // 先頭の 1 件しか出ないので、ほかにもあるときは、全件を見る方法を添える
    let more = if issues.len() > 1 {
        "（v: 全件を表示）"
    } else {
        ""
    };
    text(
        format!(
            "検証: エラー {errors} 件 / 警告 {warnings} 件{more} — {location}{}",
            first.message
        ),
        style,
    )
}

/// 検証結果の一覧の、行頭の印（`▶ ` など）の表示幅
const ISSUE_MARKER_WIDTH: usize = 2;

/// 検証結果の一覧。エラーを先に、続けて警告を出し、長いメッセージは折り返す。
/// メッセージには設定ファイルの文字列が含まれるので、制御文字は無害な文字に置き換える
fn draw_issues(frame: &mut Frame, app: &App, area: Rect) {
    let Some(view) = &app.issues_view else {
        return;
    };
    let mut issues = app.draft.issues();
    issues.sort_by_key(|issue| issue.severity != Severity::Error);
    let errors = issues
        .iter()
        .filter(|issue| issue.severity == Severity::Error)
        .count();
    let block = focus_block(
        format!(
            " 検証結果 ─ エラー {errors} 件 / 警告 {} 件 ",
            issues.len() - errors
        ),
        true,
    );
    let inner = block.inner(area);
    if inner.width == 0 || inner.height == 0 {
        frame.render_widget(block, area);
        return;
    }
    if issues.is_empty() {
        frame.render_widget(block, area);
        frame.render_widget(Paragraph::new("問題はありません"), inner);
        return;
    }

    let text_width = usize::from(inner.width).saturating_sub(ISSUE_MARKER_WIDTH);
    let selected = view.index().min(issues.len() - 1);
    let mut lines: Vec<Line> = Vec::new();
    let mut selected_rows = 0..0;
    for (index, issue) in issues.iter().enumerate() {
        let (label, color) = match issue.severity {
            Severity::Error => ("エラー", Color::Red),
            Severity::Warning => ("警告", Color::Yellow),
        };
        let location = if issue.path.is_empty() {
            String::new()
        } else {
            format!("{}: ", issue.path)
        };
        let message: String = format!("{label} {location}{}", issue.message)
            .chars()
            .map(|c| if c.is_control() { '\u{FFFD}' } else { c })
            .collect();
        let start = lines.len();
        let mut style = Style::default().fg(color);
        if index == selected {
            style = style.add_modifier(Modifier::BOLD);
        }
        for (row, part) in fit_line(&message, text_width, Overflow::Wrap)
            .into_iter()
            .enumerate()
        {
            let marker = if index == selected && row == 0 {
                "▶ "
            } else {
                "  "
            };
            lines.push(Line::from(vec![
                Span::raw(marker),
                Span::styled(part, style),
            ]));
        }
        if index == selected {
            selected_rows = start..lines.len();
        }
    }

    let height = usize::from(inner.height);
    // 選んでいる項目が一覧の高さを超えるときは、項目の中を行単位でスクロールする（`↑↓`）。
    // 超える行数を App に伝え、`↑↓` が、項目の末尾までスクロールしてから次の項目へ進めるようにする
    let max_scroll = selected_rows.len().saturating_sub(height);
    view.set_max_scroll(max_scroll);
    let scroll = view.scroll().min(max_scroll);
    // 収まる項目は全部見える位置まで、収まらない項目は先頭の行から（スクロールした分だけ後ろへ）出す
    let offset = if max_scroll > 0 {
        selected_rows.start + scroll
    } else {
        selected_rows
            .end
            .saturating_sub(height)
            .min(selected_rows.start)
    };
    let block = if scroll < max_scroll {
        block.title_bottom(" ↓ 続きがあります ")
    } else {
        block
    };
    frame.render_widget(block, area);
    let visible: Vec<Line> = lines.into_iter().skip(offset).take(height).collect();
    frame.render_widget(Paragraph::new(visible), inner);
}

/// 記号の表示幅と、端末によって食い違いうる点の注意（状態行用）
fn symbol_info(symbol: &str) -> (String, Style) {
    let notes = symbol_notes(symbol);
    let width = display_width(symbol);
    if notes.is_empty() {
        return (
            format!("記号の表示幅: {width}"),
            Style::default().add_modifier(Modifier::DIM),
        );
    }
    (
        format!("記号の表示幅: {width}（注意: {}）", notes.join(" / ")),
        Style::default().fg(Color::Yellow),
    )
}

/// 記号の候補の一覧（部品一覧の場所）。各候補に、表示幅と、注意があるときは `!` を添える。
/// 選んでいる候補の詳しい注意は、状態行に出す。表示幅の列を先に出し、絵文字の幅の食い違いで、右側がずれにくくする
fn draw_picker(frame: &mut Frame, app: &App, area: Rect) {
    let Some(picker) = &app.picker else {
        return;
    };
    let mut block = focus_block(
        format!(" 記号の候補: {}.{} ", picker.part, picker.key),
        true,
    );
    if cfg!(windows) {
        block = block.title_bottom(" Windows では絵文字を入力できないため、ここから選びます ");
    }
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let height = usize::from(inner.height);
    let offset = (picker.index() + 1).saturating_sub(height);
    let dim = Style::default().add_modifier(Modifier::DIM);
    let rows: Vec<Line> = picker
        .candidates
        .iter()
        .enumerate()
        .skip(offset)
        .take(height)
        .map(|(index, candidate)| {
            let is_selected = index == picker.index();
            let label = if candidate.is_empty() {
                "（記号なし）".to_string()
            } else {
                format!("\"{}\"", candidate.replace(' ', "␣"))
            };
            let width_style = if is_selected {
                Style::default().add_modifier(Modifier::REVERSED)
            } else {
                Style::default()
            };
            let mark = if symbol_notes(candidate).is_empty() {
                " "
            } else {
                "!"
            };
            Line::from(vec![
                Span::raw(if is_selected { "> " } else { "  " }),
                Span::styled(format!("幅 {:<2}", display_width(candidate)), width_style),
                Span::styled(format!(" {mark} "), dim),
                Span::raw(label),
            ])
        })
        .collect();
    frame.render_widget(Paragraph::new(rows), inner);
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
        App::for_test(Draft::with_format(DEFAULT_FORMAT))
    }

    fn app_with_focus(format: &str, focus: Focus) -> App {
        let mut app = App::for_test(Draft::with_format(format));
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
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        let mut overlays = Vec::new();
        terminal.draw(|frame| overlays = draw(frame, app)).unwrap();
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
            "変更なし",
            "設定: 未作成（保存すると作成されます: test.toml）",
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
        let app = App::for_test(Draft::new(config, toml_edit::DocumentMut::new()));
        let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
        let mut overlays = Vec::new();
        terminal.draw(|frame| overlays = draw(frame, &app)).unwrap();
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

    /// 設定の文字列から、検証結果を持つアプリを作る
    fn app_from_toml(toml: &str) -> App {
        let config = toml_edit::de::from_str::<Config>(toml).unwrap();
        App::for_test(Draft::new(config, toml.parse().unwrap()))
    }

    /// 認識できないテーブルが `count` 個ある設定（`unknown00`, `unknown01`, ...）
    fn unknown_tables(count: usize) -> String {
        use std::fmt::Write;
        (0..count).fold(String::new(), |mut toml, i| {
            write!(toml, "[unknown{i:02}]\nx = 1\n").unwrap();
            toml
        })
    }

    #[test]
    fn test_issues_view_lists_every_issue_errors_first() {
        let toml = "[directory]\nsymbl = \"x\"\n\n[nothing]\nx = 1\n\n[model]\nstyle = \"cyna\"\n";
        let mut app = app_from_toml(toml);
        // 状態行は先頭の 1 件だけなので、全件を見る方法を添える
        assert!(text(&app, 130, 40).contains("警告 2 件（v: 全件を表示）"));

        press(&mut app, KeyCode::Char('v'));
        let (screen, overlays) = render(&app, 100, 40);
        let shown = screen.join("\n");
        for expected in [
            "検証結果 ─ エラー 1 件 / 警告 2 件",
            "model.style: 認識できないスタイル名: cyna",
            "directory.symbl: 認識できないキーです",
            "nothing: 認識できないキーまたはテーブルです",
            "Esc / v: 閉じる",
        ] {
            assert!(shown.contains(expected), "{expected} がない\n{shown}");
        }
        let row = |needle: &str| {
            screen
                .iter()
                .position(|line| line.contains(needle))
                .unwrap()
        };
        assert!(
            row("エラー model.style") < row("警告 directory.symbl"),
            "{shown}"
        );
        assert!(row("警告 directory.symbl") < row("警告 nothing"), "{shown}");
        // 選んでいる項目に印を付ける
        assert!(
            screen[row("エラー model.style")].contains("▶ エラー"),
            "{shown}"
        );
        // 一覧の間は、配置・部品一覧と、部品の表示例（生 ANSI）を出さない
        assert!(!shown.contains("この行は出力されません"), "{shown}");
        assert_eq!(overlays.len(), 1, "プレビューだけ");

        press(&mut app, KeyCode::Down);
        let shown = text(&app, 100, 40);
        assert!(shown.contains("▶ 警告 directory.symbl"), "{shown}");
    }

    #[test]
    fn test_issues_view_without_issues_and_with_a_single_issue() {
        let mut app = new_app();
        press(&mut app, KeyCode::Char('v'));
        assert!(text(&app, 100, 30).contains("問題はありません"));

        // 1 件だけなら、全件を見る案内は要らない
        let app = app_from_toml("[nothing]\nx = 1\n");
        assert!(!text(&app, 130, 40).contains("v: 全件を表示"));
    }

    #[test]
    fn test_issues_view_scrolls_to_keep_the_selection_visible() {
        let toml = unknown_tables(30);
        let mut app = app_from_toml(&toml);
        press(&mut app, KeyCode::Char('v'));
        let first = text(&app, 100, 24);
        assert!(first.contains("▶ 警告 unknown00"), "{first}");
        assert!(!first.contains("unknown29"), "{first}");

        for _ in 0..40 {
            press(&mut app, KeyCode::Down);
        }
        let last = text(&app, 100, 24);
        assert!(last.contains("▶ 警告 unknown29"), "{last}");
        // 一覧は末尾へスクロールした（状態行には、先頭の問題が出続ける）
        assert!(!last.contains("警告 unknown00"), "{last}");
    }

    #[test]
    fn test_issues_view_scrolls_inside_an_item_taller_than_the_list() {
        // 一覧の高さを超える 1 件（折り返すと十数行になる長いキー）と、その次の 1 件
        let long_key = format!("{}END_OF_KEY", "a".repeat(800));
        let toml = format!("[\"{long_key}\"]\nx = 1\n\n[zzz_next]\nx = 1\n");
        let mut app = app_from_toml(&toml);
        press(&mut app, KeyCode::Char('v'));

        // 状態行にも先頭の問題は出るので、一覧の枠の内側だけを見る
        let list = |app: &App| {
            let shown = text(app, 80, 24);
            let lines: Vec<&str> = shown.lines().collect();
            let top = lines.iter().position(|l| l.contains("検証結果 ─")).unwrap();
            let bottom = lines[top..]
                .iter()
                .position(|l| l.starts_with('└'))
                .unwrap();
            lines[top..=top + bottom].join("\n")
        };

        let first = list(&app);
        assert!(first.contains("▶ 警告 aaaa"), "{first}");
        assert!(!first.contains("END_OF_KEY"), "{first}");
        assert!(first.contains("↓ 続きがあります"), "{first}");

        // ↓ で項目の中を 1 行ずつ進み、末尾（END_OF_KEY）に届く。まだ次の項目へは進まない
        let mut saw_end_of_key = false;
        for _ in 0..40 {
            press(&mut app, KeyCode::Down);
            let shown = list(&app);
            assert!(!shown.contains("zzz_next"), "{shown}");
            saw_end_of_key |= shown.contains("END_OF_KEY");
            if !shown.contains("↓ 続きがあります") {
                break;
            }
        }
        // 長いキーの末尾も、その後のメッセージの末尾も、順に見える
        assert!(saw_end_of_key);
        let end = list(&app);
        assert!(end.contains("テーブルです"), "{end}");
        assert!(!end.contains("↓ 続きがあります"), "{end}");

        // 末尾まで行ってから ↓ を押すと、次の項目へ進む（先頭から出る）
        press(&mut app, KeyCode::Down);
        let next = list(&app);
        assert!(next.contains("▶ 警告 zzz_next"), "{next}");

        // ↑ で戻ると、前の項目の先頭から出て、↑ で上へ戻れる
        press(&mut app, KeyCode::Up);
        let back = list(&app);
        assert!(back.contains("▶ 警告 aaaa"), "{back}");
        assert!(!back.contains("END_OF_KEY"), "{back}");
        for _ in 0..40 {
            if !list(&app).contains("↓ 続きがあります") {
                break;
            }
            press(&mut app, KeyCode::Down);
        }
        assert!(list(&app).contains("テーブルです"));
        for _ in 0..40 {
            press(&mut app, KeyCode::Up);
        }
        let top = list(&app);
        assert!(top.contains("▶ 警告 aaaa"), "{top}");
        assert!(!top.contains("END_OF_KEY"), "{top}");
    }

    #[test]
    fn test_issues_view_wraps_long_messages_and_neutralizes_control_characters() {
        let toml = "[model]\nstyle = \"bold cyan nosuchstyleaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\"\n";
        let mut app = app_from_toml(toml);
        press(&mut app, KeyCode::Char('v'));
        let shown = text(&app, 60, 30);
        // 折り返されても、末尾まで見える
        assert!(shown.contains("aaaaa"), "{shown}");
        assert!(shown.matches("aaaa").count() >= 2, "折り返される\n{shown}");

        let app = app_from_toml("[\"evil\\u001b[31mred\"]\nx = 1\n");
        let mut app = app;
        press(&mut app, KeyCode::Char('v'));
        let shown = text(&app, 80, 20);
        assert!(!shown.contains('\u{1b}'), "{shown:?}");
        assert!(shown.contains("evil"), "{shown}");
    }

    #[test]
    fn test_issues_view_survives_tiny_terminals() {
        let toml = unknown_tables(5);
        let mut app = app_from_toml(&toml);
        press(&mut app, KeyCode::Char('v'));
        for (width, height) in [(1, 1), (3, 3), (10, 5), (20, 8), (40, 12)] {
            let _ = render(&app, width, height);
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
        let mut app = App::for_test(Draft::with_format("$model $quota"));
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
        let mut app = App::for_test(Draft::with_format(format));
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
        let mut app = App::for_test(Draft::with_format("$directory |"));
        press(&mut app, KeyCode::Right); // 固定テキスト
        let shown = text(&app, 110, 40);
        assert!(
            shown.contains("部品を選ぶと、その部品のプロパティを編集できます"),
            "{shown}"
        );
    }

    #[test]
    fn test_props_pane_follows_the_parts_list_selection() {
        let mut app = App::for_test(Draft::with_format("$directory"));
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
        press(&mut app, KeyCode::Enter); // 記号は、候補の一覧が開く
        press(&mut app, KeyCode::Char('e')); // 自由に入力する
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
        let app = App::for_test(Draft::new(config, toml.parse().unwrap()));
        let shown = text(&app, 130, 40);
        // エラーを先に出す
        assert!(
            shown.contains(
                "検証: エラー 1 件 / 警告 1 件（v: 全件を表示） — model.style: 認識できないスタイル名: cyna"
            ),
            "{shown}"
        );
    }

    #[test]
    fn test_status_line_reports_warnings_when_there_are_no_errors() {
        let toml = "[nothing]\nx = 1\n";
        let config = toml_edit::de::from_str::<Config>(toml).unwrap();
        let app = App::for_test(Draft::new(config, toml.parse().unwrap()));
        let shown = text(&app, 130, 40);
        assert!(
            shown.contains("検証: エラー 0 件 / 警告 1 件 — nothing: "),
            "{shown}"
        );
    }

    /// `input_spans` の結果を、文字列（装飾を除く）と、反転した文字に分ける
    fn flatten(spans: &[Span<'static>]) -> (String, String) {
        let text: String = spans.iter().map(|span| span.content.as_ref()).collect();
        let cursor: String = spans
            .iter()
            .filter(|span| span.style.add_modifier.contains(Modifier::REVERSED))
            .map(|span| span.content.as_ref())
            .collect();
        (text, cursor)
    }

    #[test]
    fn test_input_spans_scroll_to_keep_the_cursor_visible() {
        let alphabet = "abcdefghijklmnopqrstuvwxyz";
        let wide = "あいうえおかきくけこ";
        // (名前, 値, カーソル, 幅, 期待する表示, 期待する反転の文字)
        let cases = [
            ("fits", "abc", 3, 20, "[abc ]", " "),
            ("fits-cursor-in-the-middle", "abc", 1, 20, "[abc]", "b"),
            ("empty", "", 0, 10, "[ ]", " "),
            // 末尾にカーソルがあると、左を切る
            ("long-cursor-at-end", alphabet, 26, 10, "[…uvwxyz ]", " "),
            // 先頭にカーソルがあると、右を切る
            ("long-cursor-at-start", alphabet, 0, 10, "[abcdefg…]", "a"),
            // 途中なら、両側を切る
            (
                "long-cursor-in-the-middle",
                alphabet,
                13,
                12,
                "[…ghijklmn…]",
                "n",
            ),
            ("wide-cursor-at-end", wide, 10, 12, "[…きくけこ ]", " "),
            ("wide-cursor-at-start", wide, 0, 12, "[あいうえ…]", "あ"),
            ("wide-cursor-on-a-wide-char", wide, 5, 8, "[…おか…]", "か"),
        ];
        for (name, value, cursor, width, expected, reversed) in cases {
            let (text, at) = flatten(&input_spans(value, cursor, width));
            assert_eq!(text, expected, "{name}");
            assert_eq!(at, reversed, "{name}: 反転する文字");
            assert!(
                display_width(&text) <= width,
                "{name}: {text} が幅 {width} を超える"
            );
        }
    }

    #[test]
    fn test_input_spans_survive_tiny_widths() {
        for width in 0..8 {
            for cursor in 0..6 {
                let spans = input_spans("あいabc🌿", cursor, width);
                let (_, at) = flatten(&spans);
                assert!(!at.is_empty(), "{width}/{cursor}: カーソルが消える");
            }
        }
    }

    #[test]
    fn test_editing_a_long_value_keeps_the_cursor_inside_the_pane() {
        // 80 列の端末で、既定の `directory.format` を編集しても、カーソルが入力欄に出る
        let mut app = props_app("$directory");
        press(&mut app, KeyCode::Enter); // format（先頭の行）
        assert!(app.input.is_some());
        for (name, keys) in [
            ("end", vec![]),
            ("home", vec![KeyCode::Home]),
            ("type-at-home", vec![KeyCode::Char('Z'), KeyCode::Char('Y')]),
            ("end-again", vec![KeyCode::End]),
        ] {
            for key in keys {
                press(&mut app, key);
            }
            let mut terminal = Terminal::new(TestBackend::new(80, 40)).unwrap();
            terminal.draw(|frame| _ = draw(frame, &app)).unwrap();
            let buffer = terminal.backend().buffer();
            // 入力欄は、項目名の列（x = 1 + 2 + 20 + 1）より右。そこに反転したセルが 1 つ以上ある
            let value_x = 1 + 2 + 20 + 1;
            let visible = (0..buffer.area.height).any(|y| {
                (value_x..buffer.area.width - 1).any(|x| {
                    buffer[(x, y)].modifier.contains(Modifier::REVERSED)
                        && screen_row_has_input_marker(buffer, y)
                })
            });
            assert!(visible, "{name}: カーソルが入力欄に出ない");
        }
    }

    /// その行が、入力中の欄（`[` で始まる）を含むか
    fn screen_row_has_input_marker(buffer: &ratatui::buffer::Buffer, y: u16) -> bool {
        (0..buffer.area.width).any(|x| buffer[(x, y)].symbol() == "[")
    }

    #[test]
    fn test_props_rows_are_capped_and_scroll_to_the_selection() {
        // quota は 10 個のプロパティを持つ。上限の行数で、選択中の行が見えるようにずらす
        let mut app = props_app("$quota");
        for _ in 0..9 {
            press(&mut app, KeyCode::Down);
        }
        let shown = text(&app, 110, 50);
        assert!(shown.contains("> disabled"), "{shown}");
        assert!(
            !shown.contains("  format "),
            "先頭の行は、上へずれて見えない\n{shown}"
        );
    }

    // ---- 保存・未保存・競合の表示 ----

    use crate::editor::save::SaveError;
    use crate::tui::storage::fake::FakeStorage;

    fn storage_app(format: &str) -> (App, FakeStorage) {
        let storage = FakeStorage::default();
        let app = App::with_storage_for_test(Draft::with_format(format), Box::new(storage.clone()));
        (app, storage)
    }

    #[test]
    fn test_header_and_footer_show_the_save_state() {
        let (mut app, _) = storage_app("$model");
        let shown = text(&app, 100, 40);
        assert!(shown.contains("変更なし"), "{shown}");
        assert!(shown.contains("s: 保存  q: 終了"), "{shown}");

        press(&mut app, KeyCode::Char('a'));
        let shown = text(&app, 100, 40);
        assert!(shown.contains("未保存の変更あり"), "{shown}");
        assert!(!shown.contains("変更なし"), "{shown}");

        // どのフォーカスでも、保存と終了のキーは見える（狭い端末でも切れない先頭に置く）
        for focus in [Focus::Layout, Focus::Parts, Focus::Props, Focus::Preview] {
            app.focus = focus;
            let shown = text(&app, 60, 40);
            assert!(shown.contains("s: 保存  q: 終了"), "{focus:?}\n{shown}");
        }
    }

    #[test]
    fn test_status_shows_the_saved_notice_then_clears_it() {
        let (mut app, _) = storage_app("$model");
        press(&mut app, KeyCode::Char('a'));
        press(&mut app, KeyCode::Char('s'));
        let shown = text(&app, 100, 40);
        assert!(shown.contains("保存しました: test.toml"), "{shown}");
        assert!(shown.contains("変更なし"), "{shown}");
        press(&mut app, KeyCode::Down);
        assert!(!text(&app, 100, 40).contains("保存しました"));
    }

    #[test]
    fn test_status_shows_the_quit_confirmation() {
        let (mut app, _) = storage_app("$model");
        press(&mut app, KeyCode::Char('a'));
        press(&mut app, KeyCode::Char('q'));
        let shown = text(&app, 120, 40);
        assert!(
            shown.contains("未保存の変更があります。 y: 保存せず終了  s: 保存して終了"),
            "{shown}"
        );
    }

    #[test]
    fn test_status_shows_the_conflict_choices() {
        // (名前, 衝突の種類, 期待する理由)
        let cases = [
            ("changed", ConflictKind::Changed, "外部で変更されました"),
            ("deleted", ConflictKind::Deleted, "外部で削除されました"),
            ("created", ConflictKind::Created, "他で作成されました"),
        ];
        for (name, kind, reason) in cases {
            let (mut app, storage) = storage_app("$model");
            press(&mut app, KeyCode::Char('a'));
            storage.0.borrow_mut().save_error = Some(SaveError::Conflict(kind));
            press(&mut app, KeyCode::Char('s'));
            let shown = text(&app, 140, 40);
            assert!(shown.contains(reason), "{name}\n{shown}");
            assert!(
                shown.contains("r: 再読み込み（今の編集を破棄）"),
                "{name}\n{shown}"
            );
            assert!(shown.contains("a: 別名で保存"), "{name}\n{shown}");
        }
    }

    #[test]
    fn test_status_shows_the_save_path_input_and_its_error() {
        let (mut app, storage) = storage_app("$model");
        press(&mut app, KeyCode::Char('a'));
        storage.0.borrow_mut().save_error = Some(SaveError::Conflict(ConflictKind::Changed));
        press(&mut app, KeyCode::Char('s'));
        press(&mut app, KeyCode::Char('a'));
        let shown = text(&app, 120, 40);
        assert!(
            shown.contains("別名で保存するパス: [test.toml.new ]"),
            "{shown}"
        );
        assert!(shown.contains("別名で保存  Enter: 保存"), "{shown}");

        // 保存できなかった理由は、同じ行に出る
        storage.0.borrow_mut().save_as_error = Some(SaveError::Conflict(ConflictKind::Created));
        press(&mut app, KeyCode::Enter);
        let shown = text(&app, 160, 40);
        assert!(
            shown.contains("別名で保存するパス: [test.toml.new ]"),
            "{shown}"
        );
        assert!(shown.contains("上書きしません"), "{shown}");
    }

    #[test]
    fn test_save_path_error_is_visible_on_narrow_terminals_and_long_paths() {
        // (名前, 端末の幅, 端末の高さ)
        for (name, width) in [
            ("80-columns", 80_u16),
            ("120-columns", 120),
            ("60-columns", 60),
        ] {
            let (mut app, storage) = storage_app("$model");
            press(&mut app, KeyCode::Char('a'));
            storage.0.borrow_mut().save_error = Some(SaveError::Conflict(ConflictKind::Changed));
            press(&mut app, KeyCode::Char('s'));
            press(&mut app, KeyCode::Char('a'));

            // 長いパスを足す
            for _ in 0..3 {
                for c in "/very/long/directory/name/that/does/not/fit".chars() {
                    press(&mut app, KeyCode::Char(c));
                }
            }
            storage.0.borrow_mut().save_as_error = Some(SaveError::Conflict(ConflictKind::Created));
            press(&mut app, KeyCode::Enter);
            assert!(app.input.is_some(), "{name}");

            let shown = text(&app, width, 40);
            // 失敗の理由と、入力（カーソルの `]`）の両方が、画面に出る
            assert!(
                shown.contains("上書きしません"),
                "{name}: 理由が出ない\n{shown}"
            );
            assert!(shown.contains("別名で保存するパス: ["), "{name}\n{shown}");
            assert!(
                shown
                    .lines()
                    .any(|line| line.contains("別名で保存するパス: [") && line.contains(']')),
                "{name}: 入力欄が閉じていない（カーソルが切れている）\n{shown}"
            );
        }
    }

    #[test]
    fn test_save_path_hint_is_shown_before_any_error() {
        let (mut app, storage) = storage_app("$model");
        press(&mut app, KeyCode::Char('a'));
        storage.0.borrow_mut().save_error = Some(SaveError::Conflict(ConflictKind::Changed));
        press(&mut app, KeyCode::Char('s'));
        press(&mut app, KeyCode::Char('a'));
        let shown = text(&app, 100, 40);
        assert!(shown.contains("保存先のパスを入力してください"), "{shown}");
    }

    // ---- 記号の候補の表示 ----

    #[test]
    fn test_picker_replaces_the_parts_list_and_shows_widths() {
        let mut app = props_app("$git_branch");
        press(&mut app, KeyCode::Down); // symbol
        press(&mut app, KeyCode::Enter);
        let (screen, overlays) = render(&app, 110, 40);
        let shown = screen.join("\n");
        assert!(shown.contains("記号の候補: git_branch.symbol"), "{shown}");
        assert!(shown.contains("記号を選ぶ  ↑↓: 選ぶ"), "{shown}");
        // 候補の表示幅（絵文字 + 空白 = 3）と、記号なし
        assert!(shown.contains("幅 3"), "{shown}");
        assert!(shown.contains("（記号なし）"), "{shown}");
        // 部品の表示例（生 ANSI）は出さない。プレビューだけ
        assert_eq!(overlays.len(), 1, "{overlays:?}");
        assert!(!shown.contains("> directory"), "部品一覧は隠れる\n{shown}");
    }

    #[test]
    fn test_picker_previews_the_selected_candidate_live() {
        let mut app = props_app("$git_branch");
        press(&mut app, KeyCode::Down);
        press(&mut app, KeyCode::Enter);
        press(&mut app, KeyCode::Down); // 🌱
        let (_, overlays) = render(&app, 110, 40);
        let preview = overlay_text(&overlays[..1]);
        assert!(preview.contains("🌱 main"), "{preview}");
    }

    #[test]
    fn test_status_shows_the_width_and_notes_of_the_selected_symbol() {
        // git_branch の既定の記号は、注意なし
        let mut plain = props_app("$git_branch");
        press(&mut plain, KeyCode::Down); // symbol
        press(&mut plain, KeyCode::Enter);
        let shown = text(&plain, 130, 40);
        assert!(shown.contains("記号の表示幅: 3"), "{shown}");
        assert!(!shown.contains("注意:"), "{shown}");

        // review の既定の記号 🐿 は、絵文字だが幅 1 と数える。注意が出る
        let mut app = props_app("$review");
        press(&mut app, KeyCode::Down); // symbol
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.picker.as_ref().unwrap().selected(), "🐿 ");
        let shown = text(&app, 160, 40);
        assert!(
            shown.contains("記号の表示幅: 2（注意: 絵文字だが幅 1 と数える"),
            "{shown}"
        );
        // 一覧にも、注意のある候補の印（`!`）が出る
        assert!(shown.contains(" ! \"🐿␣\""), "{shown}");
    }

    #[test]
    fn test_status_shows_the_width_while_typing_a_symbol() {
        let mut app = props_app("$git_branch");
        press(&mut app, KeyCode::Down);
        press(&mut app, KeyCode::Enter);
        press(&mut app, KeyCode::Char('e'));
        let shown = text(&app, 120, 40);
        assert!(shown.contains("記号の表示幅: 3"), "{shown}");
        // 入力でも、注意が出る（★ は East Asian Ambiguous）
        press(&mut app, KeyCode::Char('★'));
        let shown = text(&app, 160, 40);
        assert!(shown.contains("East Asian Ambiguous"), "{shown}");
    }
}
