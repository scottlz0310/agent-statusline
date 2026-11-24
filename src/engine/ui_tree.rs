//! Codex CLI の Custom status line (`UiTree` JSON) 変換モジュール。
//!
//! 契約根拠: `openai/codex` `rust-v0.171.0` の `codex-rs/tui/src/ui_tree.rs` / `decode.rs`。

use serde::Serialize;

use crate::engine::config::Config;
use crate::engine::formatter::compose_lines;
use crate::engine::style::StyledSpan;
use crate::model::review_summary::RepoReviews;
use crate::model::state::StatuslineState;
use crate::modules::git::GitStatus;

/// Codex は端末幅（`COLUMNS` を含む）を MCP プロセスに渡さないため、
/// `[directory] max_length = 0`（自動）時の基準幅として 120 列を採用する。
pub const CODEX_DEFAULT_TERMINAL_WIDTH: usize = 120;

/// Codex `UiTree` デコーダのハード上限（`codex-rs/tui/src/ui_tree/decode.rs`）
pub const MAX_JSON_BYTES: usize = 64 * 1024;
pub const MAX_TEXT_BYTES: usize = 16 * 1024;
pub const MAX_NODES: usize = 256;
#[cfg(test)]
pub const MAX_DEPTH: usize = 16;

/// `dimmed` および `gray`/`grey` の近似色（Phase 0 実機検証で合意）
const DIMMED_OR_GRAY_HEX: &str = "#808080";

/// ルートが `null` を取りうる Codex `UiTree`
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UiTree(pub Option<UiNode>);

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum UiNode {
    Text {
        #[serde(skip_serializing_if = "TextProps::is_default")]
        props: TextProps,
        children: Vec<UiNode>,
    },
    Link {
        props: LinkProps,
        children: Vec<UiNode>,
    },
    #[serde(untagged)]
    String(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LinkProps {
    pub href: String,
}

/// Codex `LinkHref` の UTF-8 バイト上限（`statusline.d.ts`: 8 KiB）
pub const MAX_LINK_HREF_BYTES: usize = 8 * 1024;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct TextProps {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub weight: Option<TextWeight>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub italic: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub underline: Option<bool>,
}

impl TextProps {
    fn is_default(&self) -> bool {
        self.color.is_none()
            && self.weight.is_none()
            && self.italic.is_none()
            && self.underline.is_none()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TextWeight {
    Bold,
}

/// ANSI SGR コード列を Codex `UiTree` の `TextProps` に変換する。
///
/// - `bold` (1) → `weight: "bold"`
/// - `italic` (3) → `italic: true`
/// - `underline` (4) → `underline: true`
/// - 前景 8 色 (30..=37) → 対応 ANSI 色名（複数指定時は後勝ち）
/// - `gray` / `grey` (90) → `"#808080"`（後勝ち）
/// - `dimmed` (2) → 明示前景色が未指定の場合のみ `"#808080"` で近似
/// - `inverted` (7) / `bg:*` (40..=47, 100) → 非対応のため無視
pub fn text_props_from_ansi_codes(codes: &[u8]) -> TextProps {
    let mut props = TextProps::default();
    let mut saw_dimmed = false;

    for &code in codes {
        match code {
            1 => props.weight = Some(TextWeight::Bold),
            2 => saw_dimmed = true,
            3 => props.italic = Some(true),
            4 => props.underline = Some(true),
            30 => props.color = Some("black".to_string()),
            31 => props.color = Some("red".to_string()),
            32 => props.color = Some("green".to_string()),
            33 => props.color = Some("yellow".to_string()),
            34 => props.color = Some("blue".to_string()),
            35 => props.color = Some("magenta".to_string()),
            36 => props.color = Some("cyan".to_string()),
            37 => props.color = Some("white".to_string()),
            90 => props.color = Some(DIMMED_OR_GRAY_HEX.to_string()),
            _ => {}
        }
    }

    if props.color.is_none() && saw_dimmed {
        props.color = Some(DIMMED_OR_GRAY_HEX.to_string());
    }

    props
}

fn sanitize_segment(raw: &str) -> String {
    raw.chars().filter(|&ch| ch != '\r').collect()
}

fn truncate_utf8(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        return s;
    }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct EntryStyle {
    props: TextProps,
    href: Option<String>,
}

fn sanitize_href(href: Option<&str>) -> Option<String> {
    let raw = href?.trim();
    if !(raw.starts_with("https://") || raw.starts_with("http://")) {
        return None;
    }
    if raw.len() > MAX_LINK_HREF_BYTES || raw.chars().any(char::is_control) {
        return None;
    }
    Some(raw.to_string())
}

fn push_merged_entry(entries: &mut Vec<(EntryStyle, String)>, style: EntryStyle, text: &str) {
    if text.is_empty() {
        return;
    }
    if let Some((last_style, last_text)) = entries.last_mut()
        && *last_style == style
    {
        last_text.push_str(text);
    } else {
        entries.push((style, text.to_string()));
    }
}

fn node_cost(style: &EntryStyle) -> usize {
    let base = if style.props.is_default() { 1 } else { 2 };
    base + usize::from(style.href.is_some())
}

fn entry_to_node(style: EntryStyle, text: String) -> UiNode {
    let inner = if style.props.is_default() {
        UiNode::String(text)
    } else {
        UiNode::Text {
            props: style.props,
            children: vec![UiNode::String(text)],
        }
    };
    match style.href {
        Some(href) => UiNode::Link {
            props: LinkProps { href },
            children: vec![inner],
        },
        None => inner,
    }
}

/// 行ごとのスタイル付きスパン列を Codex `UiTree` へ変換する。
///
/// 複数行は純粋な `text` + `link` + `string` + `"\n"` で構成する（`row` を混在させると
/// Codex の `prepare.rs` により暗黙横並び分割され 2 行目以降が崩れるため）。
/// 非 null ノード数（256）・リーフ UTF-8 バイト数（16 KiB）・深さ（16）の上限を常に保証する。
pub fn spans_to_ui_tree(lines: &[Vec<StyledSpan>]) -> UiTree {
    let mut entries: Vec<(EntryStyle, String)> = Vec::new();
    let mut remaining_bytes = MAX_TEXT_BYTES;

    for (line_idx, line) in lines.iter().enumerate() {
        if remaining_bytes == 0 {
            break;
        }
        if line_idx > 0 {
            push_merged_entry(
                &mut entries,
                EntryStyle {
                    props: TextProps::default(),
                    href: None,
                },
                "\n",
            );
            remaining_bytes = remaining_bytes.saturating_sub(1);
            if remaining_bytes == 0 {
                break;
            }
        }
        for span in line {
            if remaining_bytes == 0 {
                break;
            }
            let cleaned = sanitize_segment(&span.text);
            let clipped = truncate_utf8(&cleaned, remaining_bytes);
            if clipped.is_empty() {
                continue;
            }
            remaining_bytes -= clipped.len();
            let style = EntryStyle {
                props: text_props_from_ansi_codes(&span.codes),
                href: sanitize_href(span.href.as_deref()),
            };
            push_merged_entry(&mut entries, style, clipped);
        }
    }

    if entries.is_empty() {
        return UiTree(None);
    }

    // 単一エントリのみならラッパーのルート text を省いてそのまま返す（ノード 1〜3 個、深さ 0〜2）
    if entries.len() == 1 {
        let (style, text) = entries.pop().unwrap();
        return UiTree(Some(entry_to_node(style, text)));
    }

    // ルート Text (1 ノード) + 各子要素の合計ノード数を MAX_NODES (256) 以内に収める。
    // 超過する場合は末尾側を 1 つのプレーン String (1 ノード) に畳み込む。
    let total_entries = entries.len();
    let mut children: Vec<UiNode> = Vec::with_capacity(total_entries);
    let mut used_nodes = 1usize; // ルート Text 分
    let mut overflow_plain = String::new();

    for (idx, (style, text)) in entries.into_iter().enumerate() {
        if !overflow_plain.is_empty() {
            overflow_plain.push_str(&text);
            continue;
        }
        let cost = node_cost(&style);
        // この要素をスタイル付きで入れた後、残りの要素があれば最低 1 ノード（畳み込み String）を残す
        let reserve_for_tail = usize::from(idx + 1 < total_entries);
        if used_nodes + cost + reserve_for_tail <= MAX_NODES {
            used_nodes += cost;
            children.push(entry_to_node(style, text));
        } else {
            overflow_plain.push_str(&text);
        }
    }

    if !overflow_plain.is_empty() {
        if let Some(UiNode::String(last)) = children.last_mut() {
            last.push_str(&overflow_plain);
        } else {
            children.push(UiNode::String(overflow_plain));
        }
    }

    UiTree(Some(UiNode::Text {
        props: TextProps::default(),
        children,
    }))
}

fn flatten_text(node: &UiNode, out: &mut String) {
    match node {
        UiNode::String(s) => out.push_str(s),
        UiNode::Text { children, .. } | UiNode::Link { children, .. } => {
            for child in children {
                flatten_text(child, out);
            }
        }
    }
}

impl UiTree {
    /// Codex `resources/read` の `contents[0].text` 用 JSON 文字列を生成する。
    /// 64 KiB 上限（`MAX_JSON_BYTES`）を必ず満たす。
    pub fn to_json_string(&self) -> String {
        let json = serde_json::to_string(self).unwrap_or_else(|_| "null".to_string());
        if json.len() <= MAX_JSON_BYTES {
            return json;
        }

        // JSON エスケープ膨張等で 64 KiB を超えた場合、プレーン文字列へ縮約して再シリアライズ
        let Some(root) = &self.0 else {
            return "null".to_string();
        };
        let mut flat = String::new();
        flatten_text(root, &mut flat);
        let mut limit = flat.len().min(MAX_TEXT_BYTES);
        loop {
            let sliced = truncate_utf8(&flat, limit);
            let candidate =
                serde_json::to_string(&UiTree(Some(UiNode::String(sliced.to_string()))))
                    .unwrap_or_else(|_| "null".to_string());
            if candidate.len() <= MAX_JSON_BYTES || limit == 0 {
                return candidate;
            }
            limit /= 2;
        }
    }
}

/// 収集済みの入力から Codex `UiTree` を合成する純関数
pub fn render_ui_tree(
    config: &Config,
    state: &StatuslineState,
    git: Option<&GitStatus>,
    reviews: &RepoReviews,
) -> UiTree {
    let lines = compose_lines(config, state, git, reviews, CODEX_DEFAULT_TERMINAL_WIDTH);
    spans_to_ui_tree(&lines)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::state::QuotaItem;
    use serde_json::Value;
    use std::path::PathBuf;

    /// Codex `codex-rs/tui/src/ui_tree/decode.rs` と同一の 4 上限を検証する
    fn assert_codex_limits(json: &str) {
        assert!(
            json.len() <= MAX_JSON_BYTES,
            "JSON size {} exceeds {MAX_JSON_BYTES}",
            json.len()
        );
        let val: Value = serde_json::from_str(json).expect("valid JSON");
        if val.is_null() {
            return;
        }
        let mut stack = vec![(&val, 0usize)];
        let mut nodes = 0usize;
        let mut text_bytes = 0usize;
        while let Some((node, depth)) = stack.pop() {
            assert!(depth <= MAX_DEPTH, "depth {depth} exceeds {MAX_DEPTH}");
            nodes += 1;
            assert!(nodes <= MAX_NODES, "nodes {nodes} exceeds {MAX_NODES}");
            match node {
                Value::String(s) => {
                    text_bytes += s.len();
                    assert!(
                        text_bytes <= MAX_TEXT_BYTES,
                        "text_bytes {text_bytes} exceeds {MAX_TEXT_BYTES}"
                    );
                }
                Value::Object(map) => {
                    let kind = map.get("type").and_then(Value::as_str);
                    assert!(
                        matches!(kind, Some("text" | "link")),
                        "unexpected UiTree element type: {kind:?}"
                    );
                    if kind == Some("link") {
                        let href = map
                            .get("props")
                            .and_then(|p| p.get("href"))
                            .and_then(Value::as_str)
                            .expect("link node has string props.href");
                        assert!(
                            href.starts_with("https://") || href.starts_with("http://"),
                            "invalid href: {href}"
                        );
                        assert!(href.len() <= MAX_LINK_HREF_BYTES);
                    }
                    let children = map
                        .get("children")
                        .and_then(Value::as_array)
                        .expect("element node has children array");
                    for child in children {
                        if !child.is_null() {
                            stack.push((child, depth + 1));
                        }
                    }
                }
                other => panic!("unexpected UiTree node: {other}"),
            }
        }
    }

    #[test]
    fn test_text_props_from_ansi_codes() {
        type PropsCase<'a> = (
            &'a str,
            &'a [u8],
            Option<&'a str>,
            Option<TextWeight>,
            Option<bool>,
            Option<bool>,
        );
        let cases: [PropsCase; 8] = [
            ("plain", &[], None, None, None, None),
            (
                "bold-cyan",
                &[1, 36],
                Some("cyan"),
                Some(TextWeight::Bold),
                None,
                None,
            ),
            ("dimmed-only", &[2], Some("#808080"), None, None, None),
            ("dimmed-with-red", &[2, 31], Some("red"), None, None, None),
            ("gray", &[90], Some("#808080"), None, None, None),
            (
                "italic-underline-magenta",
                &[3, 4, 35],
                Some("magenta"),
                None,
                Some(true),
                Some(true),
            ),
            (
                "bg-and-inverted-ignored",
                &[7, 44, 100],
                None,
                None,
                None,
                None,
            ),
            (
                "last-fg-wins",
                &[31, 32, 33],
                Some("yellow"),
                None,
                None,
                None,
            ),
        ];

        for (name, codes, color, weight, italic, underline) in cases {
            let props = text_props_from_ansi_codes(codes);
            assert_eq!(props.color.as_deref(), color, "{name}: color");
            assert_eq!(props.weight, weight, "{name}: weight");
            assert_eq!(props.italic, italic, "{name}: italic");
            assert_eq!(props.underline, underline, "{name}: underline");
        }
    }

    #[test]
    fn test_empty_input_renders_null() {
        let tree = spans_to_ui_tree(&[]);
        assert_eq!(tree, UiTree(None));
        let json = tree.to_json_string();
        assert_eq!(json, "null");
        assert_codex_limits(&json);
    }

    #[test]
    fn test_render_ui_tree_default_3_lines() {
        let state = StatuslineState {
            cwd: PathBuf::from("C:\\Users\\dev\\src\\agent-statusline"),
            model: Some("GPT-6.2-Sol (medium)".to_string()),
            context_used_percentage: Some(25),
            total_input_tokens: Some(15_000),
            total_output_tokens: None,
            agent_state: Some("working".to_string()),
            sandbox_enabled: true,
            plan_tier: Some("$0.42".to_string()),
            quotas: vec![QuotaItem {
                id: "codex-primary".to_string(),
                label: "5h".to_string(),
                used_percentage: Some(11),
                reset_in_seconds: Some(3600),
                reset_time_iso: None,
            }],
        };
        let git = GitStatus {
            branch: Some("main".to_string()),
            is_dirty: true,
            remote_repo: None,
        };

        let tree = render_ui_tree(
            &Config::default(),
            &state,
            Some(&git),
            &RepoReviews::default(),
        );
        let json = tree.to_json_string();
        assert_codex_limits(&json);

        // row ノードを含まないこと（複数行崩れ防止）
        assert!(!json.contains("\"type\":\"row\""));
        // 3 行分の内容と改行が含まれること
        assert!(json.contains("agent-statusline"));
        assert!(json.contains("main"));
        assert!(json.contains("GPT-6.2-Sol (medium)"));
        assert!(json.contains("Ctx: 25%"));
        assert!(json.contains("5h: 11%"));
        assert!(json.contains("\\n"));
        // dimmed が #808080 に近似されていること
        assert!(json.contains("#808080"));
    }

    #[test]
    fn test_node_limit_256_is_enforced() {
        // 境界値（127 / 128 / 129 / 255 / 300）でノード数上限 256 を検証
        for count in [127usize, 128, 129, 255, 300] {
            let line: Vec<StyledSpan> = (0..count)
                .map(|i| StyledSpan {
                    text: format!("s{i} "),
                    codes: vec![if i % 2 == 0 { 31 } else { 32 }],
                    href: None,
                })
                .collect();

            let tree = spans_to_ui_tree(&[line]);
            let json = tree.to_json_string();
            assert_codex_limits(&json);
            // 末尾のテキストもプレーン畳み込みで失われないこと
            let last_token = format!("s{}", count - 1);
            assert!(json.contains(&last_token), "count={count}");
        }
    }

    #[test]
    fn test_link_nodes_render_and_respect_limits() {
        use crate::model::review_summary::{ActiveReview, QueuedReview};

        let state = StatuslineState {
            cwd: PathBuf::from("C:\\Users\\dev\\src\\agent-statusline"),
            model: Some("GPT-6.2-Sol".to_string()),
            ..Default::default()
        };
        let reviews = RepoReviews {
            active: vec![ActiveReview {
                repository: "scottlz0310/agent-statusline".to_string(),
                pr_number: 149,
                round: 1,
                agent: Some("codex".to_string()),
                started_at: None,
            }],
            waiting: vec![QueuedReview {
                repository: "scottlz0310/agent-statusline".to_string(),
                pr_number: 150,
                round: 2,
                reason: "opened".to_string(),
                hold_reason: Some("ciPending".to_string()),
                queue_position: None,
                auto_pause_used_pct: None,
            }],
            alert: None,
        };

        let tree = render_ui_tree(&Config::default(), &state, None, &reviews);
        let json = tree.to_json_string();
        assert_codex_limits(&json);
        assert!(json.contains("\"type\":\"link\""), "{json}");
        assert!(
            json.contains("https://github.com/scottlz0310/agent-statusline/pull/149"),
            "{json}"
        );
        assert!(
            json.contains("https://github.com/scottlz0310/agent-statusline/pull/150"),
            "{json}"
        );
        assert!(json.contains("(CI)"), "{json}");

        // リンクノード大量投入時も 256 ノード上限を満たすこと
        let many_links: Vec<StyledSpan> = (0..120)
            .map(|i| StyledSpan {
                text: format!("#{i} "),
                codes: vec![33],
                href: Some(format!("https://github.com/o/r/pull/{i}")),
            })
            .collect();
        let bounded = spans_to_ui_tree(&[many_links]).to_json_string();
        assert_codex_limits(&bounded);
        assert!(bounded.contains("#119"));

        // トップレベルの装飾構文が $review をまたぐ場合も Codex UiTree で色とリンクが両立すること
        let outer_styled = Config {
            format: "[Review $review](red)".to_string(),
            review: crate::engine::config::ReviewConfig {
                format: "$items".to_string(),
                waiting_format: "#$pr".to_string(),
                separator: "|".to_string(),
                ..Default::default()
            },
            ..Default::default()
        };
        let two_waiting = RepoReviews {
            active: vec![],
            waiting: vec![
                QueuedReview {
                    repository: "o/r".to_string(),
                    pr_number: 42,
                    round: 1,
                    reason: "opened".to_string(),
                    hold_reason: None,
                    queue_position: None,
                    auto_pause_used_pct: None,
                },
                QueuedReview {
                    repository: "o/r".to_string(),
                    pr_number: 43,
                    round: 1,
                    reason: "opened".to_string(),
                    hold_reason: None,
                    queue_position: None,
                    auto_pause_used_pct: None,
                },
            ],
            alert: None,
        };
        let outer_json = render_ui_tree(&outer_styled, &state, None, &two_waiting).to_json_string();
        assert_codex_limits(&outer_json);
        assert!(!outer_json.contains("[Review "), "{outer_json}");
        assert!(!outer_json.contains("](red)"), "{outer_json}");
        assert!(outer_json.contains("\"color\":\"red\""), "{outer_json}");
        assert!(
            outer_json.contains("https://github.com/o/r/pull/42"),
            "{outer_json}"
        );
        assert!(
            outer_json.contains("https://github.com/o/r/pull/43"),
            "{outer_json}"
        );
    }

    #[test]
    fn test_text_byte_limit_16kib_is_enforced() {
        // マルチバイト文字（日本語 3 バイト）を含む 20 KiB 入力
        let huge = "あ".repeat(7_000); // 21,000 bytes
        let line = vec![StyledSpan {
            text: huge,
            codes: vec![36],
            href: None,
        }];

        let tree = spans_to_ui_tree(&[line]);
        let json = tree.to_json_string();
        assert_codex_limits(&json);
    }

    #[test]
    fn test_json_byte_limit_64kib_with_heavy_escaping() {
        // 制御文字は JSON で \u0001 (6 倍) に膨らむため、16 KiB リーフでも 64 KiB を超えうる
        let heavily_escaped = "\x01".repeat(MAX_TEXT_BYTES);
        let line = vec![StyledSpan {
            text: heavily_escaped,
            codes: vec![1, 31],
            href: None,
        }];

        let tree = spans_to_ui_tree(&[line]);
        let json = tree.to_json_string();
        assert_codex_limits(&json);
    }
}
