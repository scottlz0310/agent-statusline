//! 部品一覧に並べる項目（部品、定型の固定テキスト、改行）と、配置への適用。

use crate::editor::layout::Token;
use crate::editor::layout_edit::{LayoutEditor, Placement};
use crate::engine::parts::part_ids;

/// 固定テキストの定型の候補。任意の文字列の入力は、テキスト入力欄ができるフェーズ 1-3 で扱う
const LITERALS: [&str; 4] = [" ", " | ", " / ", " - "];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaletteItem {
    Part(&'static str),
    Literal(&'static str),
    LineBreak,
}

/// 部品、固定テキスト、改行の順
pub fn items() -> Vec<PaletteItem> {
    part_ids()
        .into_iter()
        .map(PaletteItem::Part)
        .chain(LITERALS.into_iter().map(PaletteItem::Literal))
        .chain(std::iter::once(PaletteItem::LineBreak))
        .collect()
}

/// 部品一覧に出す名前。固定テキストは、空白を `␣` にして見分けられるようにする
pub fn label(item: PaletteItem) -> String {
    match item {
        PaletteItem::Part(id) => id.to_string(),
        PaletteItem::Literal(" ") => "空白 \"␣\"".to_string(),
        PaletteItem::Literal(text) => format!("区切り \"{}\"", text.replace(' ', "␣")),
        PaletteItem::LineBreak => "改行".to_string(),
    }
}

/// 項目を、選択中のトークンの前または後ろへ入れる。改行は、その位置で行を分ける
pub fn apply(item: PaletteItem, editor: &mut LayoutEditor, placement: Placement) {
    match (item, placement) {
        (PaletteItem::Part(id), _) => editor.insert(Token::Part(id.to_string()), placement),
        (PaletteItem::Literal(text), _) => {
            editor.insert(Token::Literal(text.to_string()), placement);
        }
        (PaletteItem::LineBreak, Placement::Before) => editor.break_line_before(),
        (PaletteItem::LineBreak, Placement::After) => editor.break_line_after(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::layout::Layout;

    #[test]
    fn test_items_cover_every_part_once_then_literals_and_break() {
        let items = items();
        let parts: Vec<_> = items
            .iter()
            .filter_map(|item| match item {
                PaletteItem::Part(id) => Some(*id),
                _ => None,
            })
            .collect();
        assert_eq!(parts, part_ids());
        assert_eq!(items.last(), Some(&PaletteItem::LineBreak));
        assert_eq!(items.len(), parts.len() + LITERALS.len() + 1);
    }

    #[test]
    fn test_labels() {
        let cases = [
            (PaletteItem::Part("git_branch"), "git_branch"),
            (PaletteItem::Literal(" "), "空白 \"␣\""),
            (PaletteItem::Literal(" | "), "区切り \"␣|␣\""),
            (PaletteItem::LineBreak, "改行"),
        ];
        for (item, expected) in cases {
            assert_eq!(label(item), expected);
        }
    }

    #[test]
    fn test_apply() {
        // (名前, 項目, 位置, 期待する format)
        let cases = [
            (
                "part-after",
                PaletteItem::Part("quota"),
                Placement::After,
                "$directory$quota $model",
            ),
            (
                "part-before",
                PaletteItem::Part("quota"),
                Placement::Before,
                "$quota$directory $model",
            ),
            (
                "literal-after",
                PaletteItem::Literal(" | "),
                Placement::After,
                "$directory |  $model",
            ),
            (
                "break-after",
                PaletteItem::LineBreak,
                Placement::After,
                "$directory\n $model",
            ),
            (
                "break-before",
                PaletteItem::LineBreak,
                Placement::Before,
                "\n$directory $model",
            ),
        ];
        for (name, item, placement, expected) in cases {
            let mut editor = LayoutEditor::new(Layout::parse("$directory $model"));
            apply(item, &mut editor, placement);
            assert_eq!(editor.layout().to_format(), expected, "{name}");
            assert!(editor.can_undo(), "{name}");
        }
    }
}
