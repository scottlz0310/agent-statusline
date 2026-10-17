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

/// 部品一覧のカテゴリ（設計書 §6.2）。`All` は絞り込まない
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Category {
    All,
    Workspace,
    Agent,
    Review,
    Layout,
}

impl Category {
    const ORDER: [Self; 5] = [
        Self::All,
        Self::Workspace,
        Self::Agent,
        Self::Review,
        Self::Layout,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::All => "すべて",
            Self::Workspace => "作業場所・Git",
            Self::Agent => "エージェント",
            Self::Review => "レビュー",
            Self::Layout => "レイアウト",
        }
    }

    /// 次のカテゴリ。末尾の次は先頭
    pub fn next(self) -> Self {
        self.shifted(1)
    }

    /// 前のカテゴリ。先頭の前は末尾
    pub fn previous(self) -> Self {
        self.shifted(Self::ORDER.len() - 1)
    }

    fn shifted(self, by: usize) -> Self {
        let index = Self::ORDER.iter().position(|c| *c == self).unwrap_or(0);
        Self::ORDER[(index + by) % Self::ORDER.len()]
    }
}

/// 部品ごとのカテゴリ。部品を足したときは、ここにも足す（`test_every_part_has_a_category` が検知する）
const PART_CATEGORIES: [(&str, Category); 10] = [
    ("directory", Category::Workspace),
    ("git_branch", Category::Workspace),
    ("git_status", Category::Workspace),
    ("model", Category::Agent),
    ("context", Category::Agent),
    ("agent_state", Category::Agent),
    ("plan", Category::Agent),
    ("quota", Category::Agent),
    ("sandbox", Category::Agent),
    ("review", Category::Review),
];

impl PaletteItem {
    /// 項目のカテゴリ（`All` にはならない）。固定テキストと改行は、レイアウト
    pub fn category(self) -> Category {
        match self {
            Self::Part(id) => PART_CATEGORIES
                .iter()
                .find(|(part, _)| *part == id)
                .map(|(_, category)| *category)
                .expect("すべての部品がカテゴリに登録されている（テストで保証）"),
            Self::Literal(_) | Self::LineBreak => Category::Layout,
        }
    }
}

/// カテゴリと検索語で絞り込んだ項目。検索語は、一覧に出る名前への部分一致（大文字小文字は区別しない）。
/// 検索語が空なら、カテゴリだけで絞る
pub fn visible(items: &[PaletteItem], category: Category, query: &str) -> Vec<PaletteItem> {
    let query = query.to_lowercase();
    items
        .iter()
        .copied()
        .filter(|item| category == Category::All || item.category() == category)
        .filter(|item| query.is_empty() || label(*item).to_lowercase().contains(&query))
        .collect()
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
    fn test_every_part_has_exactly_one_category() {
        let registered: Vec<_> = part_ids();
        let mut categorized: Vec<_> = PART_CATEGORIES.iter().map(|(id, _)| *id).collect();
        let mut sorted = registered.clone();
        sorted.sort_unstable();
        categorized.sort_unstable();
        assert_eq!(categorized, sorted, "レジストリの部品とカテゴリの対応");
        for id in registered {
            assert_ne!(PaletteItem::Part(id).category(), Category::All, "{id}");
        }
    }

    #[test]
    fn test_category_cycles_in_both_directions() {
        let mut forward = Category::All;
        let mut seen = Vec::new();
        for _ in 0..Category::ORDER.len() {
            forward = forward.next();
            seen.push(forward);
        }
        assert_eq!(seen.last(), Some(&Category::All), "一周して戻る");
        assert_eq!(Category::All.previous(), Category::Layout);
        assert_eq!(Category::Workspace.previous(), Category::All);
        for category in Category::ORDER {
            assert_eq!(category.next().previous(), category);
        }
    }

    #[test]
    fn test_visible() {
        let all = items();
        let ids = |list: Vec<PaletteItem>| -> Vec<String> { list.into_iter().map(label).collect() };
        // (名前, カテゴリ, 検索語, 期待する項目の名前)
        let cases: [(&str, Category, &str, Vec<&str>); 9] = [
            (
                "workspace",
                Category::Workspace,
                "",
                vec!["directory", "git_branch", "git_status"],
            ),
            ("review", Category::Review, "", vec!["review"]),
            (
                "layout",
                Category::Layout,
                "",
                vec![
                    "空白 \"␣\"",
                    "区切り \"␣|␣\"",
                    "区切り \"␣/␣\"",
                    "区切り \"␣-␣\"",
                    "改行",
                ],
            ),
            (
                "substring-in-all",
                Category::All,
                "git",
                vec!["git_branch", "git_status"],
            ),
            (
                "case-insensitive",
                Category::All,
                "GIT_B",
                vec!["git_branch"],
            ),
            (
                "literal-by-visible-name",
                Category::All,
                "区切り",
                vec!["区切り \"␣|␣\"", "区切り \"␣/␣\"", "区切り \"␣-␣\""],
            ),
            (
                "category-and-query-combine",
                Category::Agent,
                "s",
                vec!["sandbox", "agent_state"],
            ),
            ("no-match", Category::Review, "git", vec![]),
            (
                "query-hits-other-category-only",
                Category::Workspace,
                "quota",
                vec![],
            ),
        ];
        for (name, category, query, expected) in cases {
            assert_eq!(ids(visible(&all, category, query)), expected, "{name}");
        }
        // 絞り込まなければ、全項目が元の順で出る
        assert_eq!(visible(&all, Category::All, ""), all);
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
