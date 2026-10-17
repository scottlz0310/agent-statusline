//! 2 つの TOML の行単位の差分。変更箇所だけを文脈つきで出す形（unified diff）と、
//! 全文に変更行の印を付けた形を作る。画面に依存しない。

use similar::{ChangeTag, DiffOp, TextDiff};

/// 変更箇所の前後に付ける文脈の行数
const CONTEXT_LINES: usize = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineKind {
    /// 変更のない行
    Context,
    Added,
    Removed,
    /// 変更箇所の見出し（`@@ -3,5 +3,6 @@`）
    Hunk,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffMode {
    /// 変更箇所だけを、前後の文脈つきで出す
    Changes,
    /// 全文を出し、変更行に印を付ける
    Full,
}

impl DiffMode {
    pub fn toggled(self) -> Self {
        match self {
            Self::Changes => Self::Full,
            Self::Full => Self::Changes,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffLine {
    pub kind: LineKind,
    /// 行の内容。印（`+` `-`）と行末の改行は含まない
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diff {
    pub lines: Vec<DiffLine>,
    pub added: usize,
    pub removed: usize,
}

impl Diff {
    pub fn has_changes(&self) -> bool {
        self.added + self.removed > 0
    }
}

/// `old` から `new` への差分
pub fn diff(old: &str, new: &str, mode: DiffMode) -> Diff {
    let text_diff = TextDiff::from_lines(old, new);
    let mut lines = Vec::new();
    let mut added = 0;
    let mut removed = 0;
    let mut push = |lines: &mut Vec<DiffLine>, change_tag: ChangeTag, value: &str| {
        let kind = match change_tag {
            ChangeTag::Equal => LineKind::Context,
            ChangeTag::Insert => {
                added += 1;
                LineKind::Added
            }
            ChangeTag::Delete => {
                removed += 1;
                LineKind::Removed
            }
        };
        lines.push(DiffLine {
            kind,
            text: value.trim_end_matches(['\n', '\r']).to_string(),
        });
    };

    match mode {
        DiffMode::Full => {
            for change in text_diff.iter_all_changes() {
                push(&mut lines, change.tag(), change.value());
            }
        }
        DiffMode::Changes => {
            for group in text_diff.grouped_ops(CONTEXT_LINES) {
                lines.push(DiffLine {
                    kind: LineKind::Hunk,
                    text: hunk_header(&group),
                });
                for op in &group {
                    for change in text_diff.iter_changes(op) {
                        push(&mut lines, change.tag(), change.value());
                    }
                }
            }
        }
    }
    Diff {
        lines,
        added,
        removed,
    }
}

/// `@@ -古い開始行,行数 +新しい開始行,行数 @@`
fn hunk_header(group: &[DiffOp]) -> String {
    let (Some(first), Some(last)) = (group.first(), group.last()) else {
        return "@@ @@".to_string();
    };
    let old = first.old_range().start..last.old_range().end;
    let new = first.new_range().start..last.new_range().end;
    format!(
        "@@ -{},{} +{},{} @@",
        old.start + 1,
        old.len(),
        new.start + 1,
        new.len()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds_and_text(diff: &Diff) -> Vec<(LineKind, &str)> {
        diff.lines
            .iter()
            .map(|line| (line.kind, line.text.as_str()))
            .collect()
    }

    #[test]
    fn test_identical_texts_have_no_changes() {
        for mode in [DiffMode::Changes, DiffMode::Full] {
            let result = diff("a = 1\nb = 2\n", "a = 1\nb = 2\n", mode);
            assert!(!result.has_changes(), "{mode:?}");
            assert_eq!((result.added, result.removed), (0, 0), "{mode:?}");
        }
        // 変更箇所だけの形では、何も出ない。全文では、全行が文脈
        assert_eq!(diff("a\n", "a\n", DiffMode::Changes).lines, Vec::new());
        assert_eq!(
            kinds_and_text(&diff("a\n", "a\n", DiffMode::Full)),
            [(LineKind::Context, "a")]
        );
    }

    #[test]
    fn test_changes_mode_shows_hunks_with_context() {
        let old = "1\n2\n3\n4\n5\n6\n7\n8\n9\n10\n";
        let new = "1\n2\n3\n4\nFIVE\n6\n7\n8\n9\n10\n";
        let result = diff(old, new, DiffMode::Changes);
        assert_eq!((result.added, result.removed), (1, 1));
        assert_eq!(
            kinds_and_text(&result),
            [
                (LineKind::Hunk, "@@ -2,7 +2,7 @@"),
                (LineKind::Context, "2"),
                (LineKind::Context, "3"),
                (LineKind::Context, "4"),
                (LineKind::Removed, "5"),
                (LineKind::Added, "FIVE"),
                (LineKind::Context, "6"),
                (LineKind::Context, "7"),
                (LineKind::Context, "8"),
            ]
        );
    }

    #[test]
    fn test_distant_changes_make_separate_hunks() {
        let old = (1..=30).fold(String::new(), |mut text, i| {
            text.push_str(&i.to_string());
            text.push('\n');
            text
        });
        let new = old
            .replacen("2\n", "TWO\n", 1)
            .replacen("29\n", "TWENTY-NINE\n", 1);
        let result = diff(&old, &new, DiffMode::Changes);
        let hunks: Vec<_> = result
            .lines
            .iter()
            .filter(|line| line.kind == LineKind::Hunk)
            .map(|line| line.text.as_str())
            .collect();
        assert_eq!(hunks, ["@@ -1,5 +1,5 @@", "@@ -26,5 +26,5 @@"]);
        // 間の変更のない行は出ない
        assert!(!result.lines.iter().any(|line| line.text == "15"));
    }

    #[test]
    fn test_full_mode_keeps_every_line_in_order() {
        let result = diff("a\nb\nc\n", "a\nB\nc\nd\n", DiffMode::Full);
        assert_eq!(
            kinds_and_text(&result),
            [
                (LineKind::Context, "a"),
                (LineKind::Removed, "b"),
                (LineKind::Added, "B"),
                (LineKind::Context, "c"),
                (LineKind::Added, "d"),
            ]
        );
        assert_eq!((result.added, result.removed), (2, 1));
    }

    #[test]
    fn test_whole_file_added_or_removed() {
        // 新規（古い側が空）と全削除（新しい側が空）
        let created = diff("", "a = 1\nb = 2\n", DiffMode::Changes);
        assert_eq!((created.added, created.removed), (2, 0));
        assert_eq!(created.lines[0].text, "@@ -1,0 +1,2 @@");
        let removed = diff("a = 1\n", "", DiffMode::Changes);
        assert_eq!((removed.added, removed.removed), (0, 1));
    }

    #[test]
    fn test_line_endings_and_missing_final_newline_are_not_shown() {
        let result = diff("a\r\nb", "a\r\nc", DiffMode::Full);
        assert_eq!(
            kinds_and_text(&result),
            [
                (LineKind::Context, "a"),
                (LineKind::Removed, "b"),
                (LineKind::Added, "c"),
            ]
        );
    }

    #[test]
    fn test_mode_toggles() {
        assert_eq!(DiffMode::Changes.toggled(), DiffMode::Full);
        assert_eq!(DiffMode::Full.toggled(), DiffMode::Changes);
    }
}
