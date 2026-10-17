//! 配置（`Layout`）の編集状態。カーソルと元に戻す履歴を持ち、画面には依存しない。
//!
//! カーソルは、行内の 1 つのトークンを選ぶ。空の行では、トークンを選ばず `index` が 0 になる。
//! 変更が起きなかった操作（端での移動、空の削除など）は、元に戻す履歴に積まない。

use super::layout::{Layout, Token};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cursor {
    pub line: usize,
    pub index: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Up,
    Down,
    Left,
    Right,
}

/// 選択中のトークンに対する、挿入の位置
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Placement {
    Before,
    After,
}

#[derive(Debug, Clone)]
pub struct LayoutEditor {
    layout: Layout,
    cursor: Cursor,
    undo: Vec<(Layout, Cursor)>,
}

impl LayoutEditor {
    pub fn new(layout: Layout) -> Self {
        Self {
            layout,
            cursor: Cursor { line: 0, index: 0 },
            undo: Vec::new(),
        }
    }

    pub fn layout(&self) -> &Layout {
        &self.layout
    }

    pub fn cursor(&self) -> Cursor {
        self.cursor
    }

    /// 元に戻す履歴を持たない複製。仮の編集を試して表示するために使う
    pub fn without_history(&self) -> Self {
        Self {
            layout: self.layout.clone(),
            cursor: self.cursor,
            undo: Vec::new(),
        }
    }

    /// 元に戻せる変更の数。操作の前後で比べると、履歴に積まれたかが分かる
    pub fn undo_len(&self) -> usize {
        self.undo.len()
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    /// 選択中のトークン。空の行では `None`
    pub fn selected(&self) -> Option<&Token> {
        self.layout
            .lines
            .get(self.cursor.line)
            .and_then(|line| line.get(self.cursor.index))
    }

    /// 指定した位置のトークンを選ぶ（マウスのクリック用）。位置が配置の外なら、何もしない。
    /// 空の行は、`index` が 0 のときだけ選べる
    pub fn select_at(&mut self, line: usize, index: usize) {
        let Some(tokens) = self.layout.lines.get(line) else {
            return;
        };
        if index < tokens.len() || (tokens.is_empty() && index == 0) {
            self.cursor = Cursor { line, index };
        }
    }

    /// 選択を動かす。左右は行内、上下は行を移る（行内の位置は行の長さに収める）
    pub fn select(&mut self, direction: Direction) {
        let Cursor { line, index } = self.cursor;
        match direction {
            Direction::Left => self.cursor.index = index.saturating_sub(1),
            Direction::Right => {
                let last = self.layout.lines[line].len().saturating_sub(1);
                self.cursor.index = (index + 1).min(last);
            }
            Direction::Up => self.cursor = self.clamped(line.saturating_sub(1), index),
            Direction::Down => self.cursor = self.clamped(line + 1, index),
        }
    }

    /// トークンを挿入して、そのトークンを選ぶ。空の行では、位置の指定によらず先頭に入れる
    pub fn insert(&mut self, token: Token, placement: Placement) {
        self.record();
        let Cursor { line, index } = self.cursor;
        let at = match (self.selected(), placement) {
            (None, _) | (Some(_), Placement::Before) => index,
            (Some(_), Placement::After) => index + 1,
        };
        self.layout.lines[line].insert(at, token);
        self.cursor.index = at;
        self.merge_adjacent_literals(line);
    }

    /// 選択中のトークンを、同じ行の中で左右へ入れ替える。行の端を超えるときは、隣の行の端へ移す。
    /// 上下は、隣の行の同じ位置（行が短ければ末尾）へ移す。動かせないときは何もしない
    pub fn move_selected(&mut self, direction: Direction) {
        let Cursor { line, index } = self.cursor;
        let Some(_) = self.selected() else {
            return;
        };
        let len = self.layout.lines[line].len();
        let last_line = self.layout.lines.len() - 1;
        let (target_line, target_index) = match direction {
            Direction::Left if index > 0 => {
                self.record();
                self.layout.lines[line].swap(index, index - 1);
                self.cursor.index = index - 1;
                self.merge_adjacent_literals(line);
                return;
            }
            Direction::Right if index + 1 < len => {
                self.record();
                self.layout.lines[line].swap(index, index + 1);
                self.cursor.index = index + 1;
                self.merge_adjacent_literals(line);
                return;
            }
            Direction::Left if line > 0 => (line - 1, self.layout.lines[line - 1].len()),
            Direction::Right if line < last_line => (line + 1, 0),
            Direction::Up if line > 0 => (line - 1, index.min(self.layout.lines[line - 1].len())),
            Direction::Down if line < last_line => {
                (line + 1, index.min(self.layout.lines[line + 1].len()))
            }
            _ => return,
        };
        self.record();
        let token = self.layout.lines[line].remove(index);
        self.layout.lines[target_line].insert(target_index, token);
        self.cursor = Cursor {
            line: target_line,
            index: target_index,
        };
        // 取り除いた部品の両隣にあった固定テキストが、移動元で隣り合う
        self.merge_adjacent_literals(line);
        self.merge_adjacent_literals(target_line);
    }

    /// 選択中のトークンを削除する。空の行では、その行を取り除く（最後の 1 行は残す）
    pub fn delete_selected(&mut self) {
        let Cursor { line, index } = self.cursor;
        if self.selected().is_some() {
            self.record();
            self.layout.lines[line].remove(index);
            self.cursor = self.clamped(line, index);
            self.merge_adjacent_literals(line);
        } else if self.layout.lines.len() > 1 {
            self.record();
            self.layout.lines.remove(line);
            self.cursor = self.clamped(line, 0);
        }
    }

    /// 選択中のトークンの前に改行を入れ、そのトークン以降を次の行へ送る（選択は同じトークンのまま）。
    /// 空の行では、空の行を 1 つ足す
    pub fn break_line_before(&mut self) {
        self.record();
        let Cursor { line, index } = self.cursor;
        let rest = self.layout.lines[line].split_off(index);
        self.layout.lines.insert(line + 1, rest);
        self.cursor = Cursor {
            line: line + 1,
            index: 0,
        };
    }

    /// 選択中のトークンの後ろに改行を入れ、残りを次の行へ送る（選択は同じトークンのまま）。
    /// 空の行では、空の行を 1 つ足す
    pub fn break_line_after(&mut self) {
        self.record();
        let Cursor { line, index } = self.cursor;
        let at = if self.selected().is_some() {
            index + 1
        } else {
            0
        };
        let rest = self.layout.lines[line].split_off(at);
        self.layout.lines.insert(line + 1, rest);
    }

    /// 末尾に空の行を足し、そこを選ぶ
    pub fn add_line(&mut self) {
        self.record();
        self.layout.lines.push(Vec::new());
        self.cursor = Cursor {
            line: self.layout.lines.len() - 1,
            index: 0,
        };
    }

    pub fn undo(&mut self) {
        if let Some((layout, cursor)) = self.undo.pop() {
            self.layout = layout;
            self.cursor = cursor;
        }
    }

    fn record(&mut self) {
        self.undo.push((self.layout.clone(), self.cursor));
    }

    fn clamped(&self, line: usize, index: usize) -> Cursor {
        let line = line.min(self.layout.lines.len() - 1);
        let last = self.layout.lines[line].len().saturating_sub(1);
        Cursor {
            line,
            index: index.min(last),
        }
    }

    /// 隣り合う固定テキストを 1 つにまとめる（`format` へ書いて読み直したときの構造と揃える）。
    /// 選択は、同じ文字を指したまま残す
    fn merge_adjacent_literals(&mut self, line: usize) {
        let tokens = &mut self.layout.lines[line];
        let mut index = 1;
        while index < tokens.len() {
            if let (Token::Literal(_), Token::Literal(next)) = (&tokens[index - 1], &tokens[index])
            {
                let next = next.clone();
                if let Token::Literal(text) = &mut tokens[index - 1] {
                    text.push_str(&next);
                }
                tokens.remove(index);
                if self.cursor.line == line && self.cursor.index >= index {
                    self.cursor.index -= 1;
                }
            } else {
                index += 1;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn editor(format: &str) -> LayoutEditor {
        LayoutEditor::new(Layout::parse(format))
    }

    fn at(editor: &mut LayoutEditor, line: usize, index: usize) {
        editor.cursor = Cursor { line, index };
    }

    fn part(id: &str) -> Token {
        Token::Part(id.to_string())
    }

    fn lit(text: &str) -> Token {
        Token::Literal(text.to_string())
    }

    #[test]
    fn test_select_moves_within_bounds() {
        // (名前, 初期の位置, 操作, 期待する位置)
        let cases = [
            ("right", (0, 0), Direction::Right, (0, 1)),
            ("right-stops-at-end", (0, 2), Direction::Right, (0, 2)),
            ("left", (0, 2), Direction::Left, (0, 1)),
            ("left-stops-at-start", (0, 0), Direction::Left, (0, 0)),
            ("down-keeps-index", (0, 1), Direction::Down, (1, 1)),
            ("down-clamps-to-short-line", (1, 2), Direction::Down, (2, 0)),
            ("down-into-empty-line", (2, 0), Direction::Down, (3, 0)),
            ("down-stops-at-last-line", (3, 0), Direction::Down, (3, 0)),
            ("up", (1, 1), Direction::Up, (0, 1)),
            ("up-clamps-to-short-line", (3, 0), Direction::Up, (2, 0)),
            ("up-stops-at-first-line", (0, 1), Direction::Up, (0, 1)),
        ];
        for (name, (line, index), direction, expected) in cases {
            let mut editor = editor(
                "$directory $model
$plan $quota
$quota
",
            );
            at(&mut editor, line, index);
            editor.select(direction);
            assert_eq!(
                (editor.cursor().line, editor.cursor().index),
                expected,
                "{name}"
            );
            assert!(!editor.can_undo(), "{name}: 選択は履歴に積まない");
        }
    }

    #[test]
    fn test_insert() {
        // (名前, 初期の位置, 位置の指定, 期待する format, 期待する選択)
        let cases = [
            (
                "after",
                (0, 0),
                Placement::After,
                "$directory$quota $model
",
                (0, 1),
            ),
            (
                "before",
                (0, 0),
                Placement::Before,
                "$quota$directory $model
",
                (0, 0),
            ),
            (
                "after-last",
                (0, 2),
                Placement::After,
                "$directory $model$quota
",
                (0, 3),
            ),
            (
                "empty-line-ignores-placement",
                (1, 0),
                Placement::After,
                "$directory $model\n$quota",
                (1, 0),
            ),
        ];
        for (name, (line, index), placement, expected, cursor) in cases {
            let mut editor = editor("$directory $model\n");
            at(&mut editor, line, index);
            editor.insert(part("quota"), placement);
            assert_eq!(editor.layout().to_format(), expected, "{name}");
            assert_eq!(
                (editor.cursor().line, editor.cursor().index),
                cursor,
                "{name}"
            );
            assert_eq!(editor.selected(), Some(&part("quota")), "{name}");
        }
    }

    #[test]
    fn test_insert_literal_merges_with_neighbors() {
        let mut editor = editor("$directory | $model");
        at(&mut editor, 0, 1);
        editor.insert(lit("/"), Placement::After);
        assert_eq!(editor.layout().to_format(), "$directory | /$model");
        assert_eq!(
            editor.layout().lines[0].len(),
            3,
            "固定テキストが 1 つになる"
        );
        // 選択は、まとめた後の固定テキスト（挿入した文字を含む）を指す
        assert_eq!(editor.selected(), Some(&lit(" | /")));
    }

    #[test]
    fn test_move_selected() {
        // (名前, 初期の位置, 方向, 期待する format, 期待する選択)
        let cases = [
            (
                "swap-right",
                (0, 0),
                Direction::Right,
                "$model$directory\n$quota",
                (0, 1),
            ),
            (
                "swap-left",
                (0, 1),
                Direction::Left,
                "$model$directory\n$quota",
                (0, 0),
            ),
            (
                "right-edge-moves-to-next-line-start",
                (0, 1),
                Direction::Right,
                "$directory\n$model$quota",
                (1, 0),
            ),
            (
                "left-edge-moves-to-prev-line-end",
                (1, 0),
                Direction::Left,
                "$directory$model$quota\n",
                (0, 2),
            ),
            (
                "down-same-index",
                (0, 0),
                Direction::Down,
                "$model\n$directory$quota",
                (1, 0),
            ),
            (
                "down-clamps-to-line-end",
                (0, 1),
                Direction::Down,
                "$directory\n$quota$model",
                (1, 1),
            ),
            (
                "up",
                (1, 0),
                Direction::Up,
                "$quota$directory$model\n",
                (0, 0),
            ),
        ];
        for (name, (line, index), direction, expected, cursor) in cases {
            let mut editor = editor("$directory$model\n$quota");
            at(&mut editor, line, index);
            editor.move_selected(direction);
            assert_eq!(editor.layout().to_format(), expected, "{name}");
            assert_eq!(
                (editor.cursor().line, editor.cursor().index),
                cursor,
                "{name}"
            );
            assert!(editor.can_undo(), "{name}");
        }
    }

    #[test]
    fn test_move_selected_noop_does_not_record() {
        // (名前, format, 初期の位置, 方向)
        let cases = [
            (
                "left-at-start",
                "$directory\n$quota",
                (0, 0),
                Direction::Left,
            ),
            (
                "right-at-end",
                "$directory\n$quota",
                (1, 0),
                Direction::Right,
            ),
            ("up-at-top", "$directory\n$quota", (0, 0), Direction::Up),
            (
                "down-at-bottom",
                "$directory\n$quota",
                (1, 0),
                Direction::Down,
            ),
            ("empty-line", "$directory\n", (1, 0), Direction::Up),
        ];
        for (name, format, (line, index), direction) in cases {
            let mut editor = editor(format);
            at(&mut editor, line, index);
            editor.move_selected(direction);
            assert_eq!(editor.layout().to_format(), format, "{name}");
            assert!(!editor.can_undo(), "{name}");
        }
    }

    #[test]
    fn test_delete_selected() {
        // (名前, format, 初期の位置, 期待する format, 期待する選択)
        let cases = [
            ("token", "$directory $model", (0, 0), " $model", (0, 0)),
            (
                "last-token-moves-left",
                "$directory $model",
                (0, 2),
                "$directory ",
                (0, 1),
            ),
            (
                "only-token-leaves-empty-line",
                "$directory\n$quota",
                (0, 0),
                "\n$quota",
                (0, 0),
            ),
            (
                "empty-line-is-removed",
                "$directory\n\n$quota",
                (1, 0),
                "$directory\n$quota",
                (1, 0),
            ),
            (
                "last-empty-line-is-removed",
                "$directory\n",
                (1, 0),
                "$directory",
                (0, 0),
            ),
        ];
        for (name, format, (line, index), expected, cursor) in cases {
            let mut editor = editor(format);
            at(&mut editor, line, index);
            editor.delete_selected();
            assert_eq!(editor.layout().to_format(), expected, "{name}");
            assert_eq!(
                (editor.cursor().line, editor.cursor().index),
                cursor,
                "{name}"
            );
            assert!(editor.can_undo(), "{name}");
        }
    }

    #[test]
    fn test_delete_merges_literals_left_adjacent() {
        // `$model` を消すと、`-` と `+` が隣り合って 1 つの固定テキストになる
        let mut editor = editor("$directory-$model+");
        at(&mut editor, 0, 2);
        editor.delete_selected();
        assert_eq!(editor.layout().to_format(), "$directory-+");
        assert_eq!(editor.layout().lines[0].len(), 2);
        assert_eq!(editor.selected(), Some(&lit("-+")));
    }

    #[test]
    fn test_move_across_lines_merges_literals_at_source() {
        // (名前, 方向)。`$model` は固定テキスト `-` と `+` に挟まれている
        for (name, direction) in [("down", Direction::Down), ("right-edge", Direction::Right)] {
            let original = "$directory-$model+
$quota";
            let mut editor = editor(original);
            at(&mut editor, 0, 2);
            if name == "right-edge" {
                // 行の右端へ寄せてから、次の行へ送る
                editor.move_selected(Direction::Right);
                editor.move_selected(Direction::Right);
            }
            editor.move_selected(direction);
            let layout = editor.layout().clone();
            assert_eq!(
                layout,
                Layout::parse(&layout.to_format()),
                "{name}: 編集後の配置と、書いて読み直した配置が一致する"
            );
            assert!(
                layout.lines[0].contains(&lit("-+")) || layout.lines[0].contains(&lit("+-")),
                "{name}: 移動元の固定テキストが 1 つにまとまる: {:?}",
                layout.lines[0]
            );

            // 元の位置へ戻す操作と、元に戻す操作で、構造が崩れない
            editor.move_selected(Direction::Up);
            let layout = editor.layout().clone();
            assert_eq!(layout, Layout::parse(&layout.to_format()), "{name}: up");
            while editor.can_undo() {
                editor.undo();
            }
            assert_eq!(editor.layout(), &Layout::parse(original), "{name}: undo");
        }
    }

    #[test]
    fn test_delete_keeps_the_last_line() {
        let mut editor = editor("");
        editor.delete_selected();
        assert_eq!(editor.layout().lines.len(), 1);
        assert!(!editor.can_undo());
    }

    #[test]
    fn test_break_line_before_and_add_line() {
        let mut editor = editor("$directory $model");
        at(&mut editor, 0, 2);
        editor.break_line_before();
        assert_eq!(editor.layout().to_format(), "$directory \n$model");
        assert_eq!(
            editor.selected(),
            Some(&part("model")),
            "同じトークンを選んだまま"
        );
        assert_eq!(editor.cursor().line, 1);

        // 先頭のトークンの前なら、上に空の行ができる
        at(&mut editor, 0, 0);
        editor.break_line_before();
        assert_eq!(editor.layout().to_format(), "\n$directory \n$model");
        assert_eq!(editor.selected(), Some(&part("directory")));

        editor.add_line();
        assert_eq!(editor.layout().to_format(), "\n$directory \n$model\n");
        assert_eq!((editor.cursor().line, editor.cursor().index), (3, 0));
        assert_eq!(editor.selected(), None);
    }

    #[test]
    fn test_break_line_after() {
        // (名前, format, 初期の位置, 期待する format, 期待する選択)
        let cases = [
            (
                "middle",
                "$directory $model",
                (0, 0),
                "$directory
 $model",
                (0, 0),
            ),
            (
                "last-token-adds-empty-line",
                "$directory $model",
                (0, 2),
                "$directory $model
",
                (0, 2),
            ),
            (
                "empty-line",
                "$directory

$quota",
                (1, 0),
                "$directory


$quota",
                (1, 0),
            ),
        ];
        for (name, format, (line, index), expected, cursor) in cases {
            let mut editor = editor(format);
            at(&mut editor, line, index);
            editor.break_line_after();
            assert_eq!(editor.layout().to_format(), expected, "{name}");
            assert_eq!(
                (editor.cursor().line, editor.cursor().index),
                cursor,
                "{name}"
            );
            editor.undo();
            assert_eq!(editor.layout().to_format(), format, "{name}: undo");
        }
    }

    #[test]
    fn test_without_history_drops_undo_only() {
        let mut editor = editor("$directory");
        editor.add_line();
        let copy = editor.without_history();
        assert!(!copy.can_undo());
        assert_eq!(copy.layout(), editor.layout());
        assert_eq!(copy.cursor(), editor.cursor());
        assert!(editor.can_undo(), "元の履歴は残る");
    }

    #[test]
    fn test_undo_restores_layout_and_cursor() {
        let mut editor = editor("$directory $model\n$quota");
        let original = editor.layout().clone();
        at(&mut editor, 0, 1);
        let before = editor.cursor();

        editor.insert(part("plan"), Placement::After);
        editor.move_selected(Direction::Down);
        editor.delete_selected();
        editor.break_line_before();
        editor.add_line();
        assert_ne!(editor.layout(), &original);

        for _ in 0..5 {
            assert!(editor.can_undo());
            editor.undo();
        }
        assert_eq!(editor.layout(), &original);
        assert_eq!(editor.cursor(), before);
        assert!(!editor.can_undo());
        // 履歴がないときは何もしない
        editor.undo();
        assert_eq!(editor.layout(), &original);
    }

    #[test]
    fn test_edits_round_trip_through_format() {
        // 編集後の配置を `format` へ書いて読み直すと、同じ構造に戻る（描画と編集の見え方が一致する）
        let mut editor = editor("$directory | $model\n$quota");
        at(&mut editor, 0, 1);
        editor.insert(lit(" "), Placement::Before);
        editor.insert(part("git_branch"), Placement::After);
        editor.move_selected(Direction::Down);
        editor.break_line_before();
        editor.delete_selected();
        let reparsed = Layout::parse(&editor.layout().to_format());
        assert_eq!(&reparsed, editor.layout());
    }
}
