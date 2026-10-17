//! スタイル仕様文字列（`bold cyan` など）の編集モデル。前景色・背景色・装飾を、トグルで選べる形で読み書きする。
//! 画面に依存しない。トークンの意味（どの語がどの色・装飾か）は、描画の `engine::style::style_code` を正本にする。
//!
//! 触っていないトークン（認識できないもの、別名の `purple`、複数の前景色など）は、元の表記と順序のまま残す。
//! 変えたカテゴリ（前景 / 背景 / 個々の装飾）の分だけを書き換える。

use crate::engine::style::style_code;

/// 前景か背景か
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Layer {
    Foreground,
    Background,
}

/// 選べる色（9 色）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StyleColor {
    Black,
    Red,
    Green,
    Yellow,
    Blue,
    Magenta,
    Cyan,
    White,
    Gray,
}

impl StyleColor {
    pub const ALL: [Self; 9] = [
        Self::Black,
        Self::Red,
        Self::Green,
        Self::Yellow,
        Self::Blue,
        Self::Magenta,
        Self::Cyan,
        Self::White,
        Self::Gray,
    ];

    /// 前景色としての名前（別名は使わない正規の綴り）
    pub fn name(self) -> &'static str {
        match self {
            Self::Black => "black",
            Self::Red => "red",
            Self::Green => "green",
            Self::Yellow => "yellow",
            Self::Blue => "blue",
            Self::Magenta => "magenta",
            Self::Cyan => "cyan",
            Self::White => "white",
            Self::Gray => "gray",
        }
    }

    /// 仕様文字列に書くトークン（背景は `bg:` 付き）
    pub fn token(self, layer: Layer) -> String {
        match layer {
            Layer::Foreground => self.name().to_string(),
            Layer::Background => format!("bg:{}", self.name()),
        }
    }

    /// ANSI の色コード（前景 30〜37・90、背景 40〜47・100）から読む
    fn from_code(code: u8) -> Option<(Layer, Self)> {
        let (layer, base) = match code {
            30..=37 => (Layer::Foreground, code - 30),
            90 => (Layer::Foreground, 8),
            40..=47 => (Layer::Background, code - 40),
            100 => (Layer::Background, 8),
            _ => return None,
        };
        Some((layer, Self::ALL[usize::from(base)]))
    }
}

/// 選べる装飾
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decoration {
    Bold,
    Dimmed,
    Italic,
    Underline,
    Inverted,
}

impl Decoration {
    pub const ALL: [Self; 5] = [
        Self::Bold,
        Self::Dimmed,
        Self::Italic,
        Self::Underline,
        Self::Inverted,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::Bold => "bold",
            Self::Dimmed => "dimmed",
            Self::Italic => "italic",
            Self::Underline => "underline",
            Self::Inverted => "inverted",
        }
    }

    fn from_code(code: u8) -> Option<Self> {
        match code {
            1 => Some(Self::Bold),
            2 => Some(Self::Dimmed),
            3 => Some(Self::Italic),
            4 => Some(Self::Underline),
            7 => Some(Self::Inverted),
            _ => None,
        }
    }
}

/// トークン 1 つの意味
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Meaning {
    Decoration(Decoration),
    Color(Layer, StyleColor),
}

fn meaning(token: &str) -> Option<Meaning> {
    let code = style_code(token)?;
    Decoration::from_code(code)
        .map(Meaning::Decoration)
        .or_else(|| StyleColor::from_code(code).map(|(layer, color)| Meaning::Color(layer, color)))
}

/// 編集中のスタイル仕様。トークンの並びを、元の表記のまま持つ
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct StyleSpec {
    tokens: Vec<String>,
}

impl StyleSpec {
    pub fn parse(spec: &str) -> Self {
        Self {
            tokens: spec.split_whitespace().map(str::to_string).collect(),
        }
    }

    /// 仕様文字列（トークンを空白 1 つで区切る）
    pub fn text(&self) -> String {
        self.tokens.join(" ")
    }

    /// 効いている色。同じ層に複数あるときは、最後のもの（端末では最後の指定が勝つ）
    pub fn color(&self, layer: Layer) -> Option<StyleColor> {
        self.tokens
            .iter()
            .filter_map(|token| match meaning(token)? {
                Meaning::Color(l, color) if l == layer => Some(color),
                _ => None,
            })
            .next_back()
    }

    /// 同じ層の色のトークンの数（2 以上なら、複数指定されている）
    pub fn color_count(&self, layer: Layer) -> usize {
        self.layer_indexes(layer).len()
    }

    /// 色を設定する。同じ層の既存のトークンは、最初のものの位置に新しいトークンを置いて、残りを取り除く。
    /// 既存がなければ、前景は背景の前に、背景は末尾に足す。`None` は、その層の色をすべて取り除く
    pub fn set_color(&mut self, layer: Layer, color: Option<StyleColor>) {
        let indexes = self.layer_indexes(layer);
        let Some(color) = color else {
            for index in indexes.into_iter().rev() {
                self.tokens.remove(index);
            }
            return;
        };
        let token = color.token(layer);
        if let Some((first, rest)) = indexes.split_first() {
            self.tokens[*first] = token;
            for index in rest.iter().rev() {
                self.tokens.remove(*index);
            }
        } else {
            let at = match layer {
                Layer::Foreground => self
                    .layer_indexes(Layer::Background)
                    .first()
                    .copied()
                    .unwrap_or(self.tokens.len()),
                Layer::Background => self.tokens.len(),
            };
            self.tokens.insert(at, token);
        }
    }

    pub fn has(&self, decoration: Decoration) -> bool {
        self.tokens
            .iter()
            .any(|token| meaning(token) == Some(Meaning::Decoration(decoration)))
    }

    /// 装飾をオン・オフする。オンにするときは、最初の色の前に足す（`bold cyan` の並びを保つ）。
    /// オフにするときは、同じ装飾のトークンをすべて取り除く
    pub fn set_decoration(&mut self, decoration: Decoration, on: bool) {
        if on {
            if self.has(decoration) {
                return;
            }
            let at = self
                .tokens
                .iter()
                .position(|token| matches!(meaning(token), Some(Meaning::Color(..))))
                .unwrap_or(self.tokens.len());
            self.tokens.insert(at, decoration.name().to_string());
        } else {
            self.tokens
                .retain(|token| meaning(token) != Some(Meaning::Decoration(decoration)));
        }
    }

    /// 認識できないトークン（そのまま残る）
    pub fn unknown_tokens(&self) -> Vec<&str> {
        self.tokens
            .iter()
            .map(String::as_str)
            .filter(|token| meaning(token).is_none())
            .collect()
    }

    fn layer_indexes(&self, layer: Layer) -> Vec<usize> {
        self.tokens
            .iter()
            .enumerate()
            .filter(|(_, token)| matches!(meaning(token), Some(Meaning::Color(l, _)) if l == layer))
            .map(|(index, _)| index)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::style::unknown_style_tokens;

    fn set(spec: &str, edit: impl FnOnce(&mut StyleSpec)) -> String {
        let mut spec = StyleSpec::parse(spec);
        edit(&mut spec);
        spec.text()
    }

    #[test]
    fn test_every_token_we_write_is_recognized_by_the_engine() {
        for color in StyleColor::ALL {
            for layer in [Layer::Foreground, Layer::Background] {
                let token = color.token(layer);
                assert!(unknown_style_tokens(&token).is_empty(), "{token}");
                assert_eq!(
                    meaning(&token),
                    Some(Meaning::Color(layer, color)),
                    "{token}"
                );
            }
        }
        for decoration in Decoration::ALL {
            assert_eq!(unknown_style_tokens(decoration.name()), Vec::<&str>::new());
            assert_eq!(
                meaning(decoration.name()),
                Some(Meaning::Decoration(decoration))
            );
        }
    }

    #[test]
    fn test_aliases_and_case_are_understood() {
        // (トークン, 期待する意味)
        let cases = [
            (
                "purple",
                Some(Meaning::Color(Layer::Foreground, StyleColor::Magenta)),
            ),
            (
                "grey",
                Some(Meaning::Color(Layer::Foreground, StyleColor::Gray)),
            ),
            (
                "bg:purple",
                Some(Meaning::Color(Layer::Background, StyleColor::Magenta)),
            ),
            (
                "bg:grey",
                Some(Meaning::Color(Layer::Background, StyleColor::Gray)),
            ),
            ("BOLD", Some(Meaning::Decoration(Decoration::Bold))),
            (
                "Cyan",
                Some(Meaning::Color(Layer::Foreground, StyleColor::Cyan)),
            ),
            ("cyna", None),
            ("bg:", None),
        ];
        for (token, expected) in cases {
            assert_eq!(meaning(token), expected, "{token}");
        }
    }

    #[test]
    fn test_reading_the_effective_color_and_decorations() {
        let spec = StyleSpec::parse("bold purple bg:blue red");
        assert_eq!(
            spec.color(Layer::Foreground),
            Some(StyleColor::Red),
            "最後の指定"
        );
        assert_eq!(spec.color_count(Layer::Foreground), 2);
        assert_eq!(spec.color(Layer::Background), Some(StyleColor::Blue));
        assert!(spec.has(Decoration::Bold));
        assert!(!spec.has(Decoration::Italic));
        assert_eq!(StyleSpec::parse("").color(Layer::Foreground), None);
        assert_eq!(StyleSpec::parse("  bold   cyan ").text(), "bold cyan");
    }

    #[test]
    fn test_set_color() {
        use StyleColor::{Cyan, Green, Red};
        // (名前, 元の仕様, 層, 色, 期待する仕様)
        let cases = [
            ("add-fg-to-empty", "", Layer::Foreground, Some(Red), "red"),
            (
                "add-fg-after-decoration",
                "bold",
                Layer::Foreground,
                Some(Cyan),
                "bold cyan",
            ),
            (
                "add-fg-before-bg",
                "bold bg:blue",
                Layer::Foreground,
                Some(Cyan),
                "bold cyan bg:blue",
            ),
            (
                "add-bg-at-the-end",
                "bold cyan",
                Layer::Background,
                Some(Green),
                "bold cyan bg:green",
            ),
            (
                "replace-keeps-position",
                "cyan bold",
                Layer::Foreground,
                Some(Red),
                "red bold",
            ),
            (
                "replace-alias",
                "bold purple",
                Layer::Foreground,
                Some(Red),
                "bold red",
            ),
            (
                "collapse-multiple",
                "red bold green",
                Layer::Foreground,
                Some(Cyan),
                "cyan bold",
            ),
            ("remove-fg", "bold cyan", Layer::Foreground, None, "bold"),
            (
                "remove-all-fg",
                "red bold green",
                Layer::Foreground,
                None,
                "bold",
            ),
            (
                "remove-keeps-other-layer",
                "cyan bg:red",
                Layer::Foreground,
                None,
                "bg:red",
            ),
            ("remove-absent", "bold", Layer::Background, None, "bold"),
            (
                "keeps-unknown-tokens",
                "cyna bold red",
                Layer::Foreground,
                Some(Green),
                "cyna bold green",
            ),
            (
                "bg-replace-alias",
                "bg:purple bold",
                Layer::Background,
                Some(Red),
                "bg:red bold",
            ),
        ];
        for (name, spec, layer, color, expected) in cases {
            assert_eq!(set(spec, |s| s.set_color(layer, color)), expected, "{name}");
        }
    }

    #[test]
    fn test_set_decoration() {
        use Decoration::{Bold, Dimmed, Italic};
        // (名前, 元の仕様, 装飾, オン・オフ, 期待する仕様)
        let cases = [
            ("on-empty", "", Bold, true, "bold"),
            ("on-before-first-color", "cyan", Bold, true, "bold cyan"),
            (
                "on-before-bg-when-only-bg",
                "bg:red",
                Italic,
                true,
                "italic bg:red",
            ),
            (
                "on-appends-after-decorations",
                "bold cyan",
                Italic,
                true,
                "bold italic cyan",
            ),
            ("on-already-on", "bold cyan", Bold, true, "bold cyan"),
            ("off", "bold cyan", Bold, false, "cyan"),
            (
                "off-removes-duplicates-and-case",
                "bold BOLD cyan",
                Bold,
                false,
                "cyan",
            ),
            ("off-absent", "cyan", Dimmed, false, "cyan"),
            (
                "keeps-unknown-tokens",
                "cyna cyan",
                Bold,
                true,
                "cyna bold cyan",
            ),
        ];
        for (name, spec, decoration, on, expected) in cases {
            assert_eq!(
                set(spec, |s| s.set_decoration(decoration, on)),
                expected,
                "{name}"
            );
        }
    }

    #[test]
    fn test_unknown_tokens_are_reported_and_kept() {
        let mut spec = StyleSpec::parse("bold cyna red");
        assert_eq!(spec.unknown_tokens(), ["cyna"]);
        spec.set_color(Layer::Foreground, Some(StyleColor::Blue));
        spec.set_decoration(Decoration::Bold, false);
        assert_eq!(spec.text(), "cyna blue");
        assert_eq!(spec.unknown_tokens(), ["cyna"]);
        assert_eq!(
            StyleSpec::parse("bold cyan").unknown_tokens(),
            Vec::<&str>::new()
        );
    }
}
