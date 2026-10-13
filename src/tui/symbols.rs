//! 記号（`PropKind::Symbol`）の候補。部品ごとの候補、共通の記号、ASCII の代替の順に並べる。
//!
//! 候補は、末尾の空白も含めた値（`"🌿 "`）。絵文字を入力できない環境（Windows）でも選べるようにする（設計書 §6.4、§10）。

/// 部品・キーごとの候補
fn specific(part: &str, key: &str) -> &'static [&'static str] {
    match (part, key) {
        ("directory", "symbol") => &["📁 ", "📂 ", "🗂 "],
        ("git_branch", "symbol") => &["🌿 ", "🌱 ", "🔀 ", "⎇ "],
        ("git_status", "modified") => &["✗", "✘", "±", "●"],
        ("sandbox", "symbol") => &["🔒 ", "🔐 ", "🛡 "],
        ("model", "symbol") => &["🤖 ", "🧠 ", "✨ "],
        ("context", "symbol") => &["🧠 ", "📊 ", "◔ "],
        ("agent_state", "symbol") => &["⚡ ", "▶ ", "● "],
        ("quota", "symbol") => &["⏳ ", "⌛ ", "◷ "],
        ("review", "symbol") => &["🔍 ", "👀 ", "🐿 "],
        _ => &[],
    }
}

/// どの部品でも使える記号
const COMMON: &[&str] = &["★ ", "◆ ", "● ", "○ ", "▶ ", "» ", "› "];

/// 絵文字が崩れる端末向けの、ASCII だけの代替。最初の `""` は、記号なし
const ASCII: &[&str] = &["", "* ", "> ", "# ", "+ ", "- "];

/// 候補の一覧。`current`（今の値）と `default`（既定値）が一覧になければ、先頭に足す。重複は除く
pub fn candidates(part: &str, key: &str, current: &str, default: &str) -> Vec<String> {
    let mut list: Vec<String> = Vec::new();
    let mut push = |value: &str| {
        if !list.iter().any(|existing| existing == value) {
            list.push(value.to_string());
        }
    };
    // 一覧にない値は、先頭に（現在の値、既定値の順）
    for value in [current, default] {
        let in_catalog = specific(part, key)
            .iter()
            .chain(COMMON)
            .chain(ASCII)
            .any(|candidate| *candidate == value);
        if !in_catalog {
            push(value);
        }
    }
    for value in specific(part, key).iter().chain(COMMON).chain(ASCII) {
        push(value);
    }
    // 既定値は、部品ごとの先頭の候補と同じことが多い。今の値と既定値が一覧にあるときは、そのまま
    list
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::parts::{PARTS, PropKind};

    #[test]
    fn test_candidates_have_no_duplicates_and_end_with_ascii_alternatives() {
        let list = candidates("git_branch", "symbol", "🌿 ", "🌿 ");
        let mut sorted = list.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), list.len(), "{list:?}");
        assert_eq!(list[0], "🌿 ", "部品ごとの候補が先頭");
        // 記号なし（空文字列）と、ASCII だけの代替がある
        assert!(list.iter().any(String::is_empty));
        assert!(list.iter().any(|c| c == "> "));
        assert!(
            list.iter().any(|c| c == "★ "),
            "共通の記号が含まれる: {list:?}"
        );
    }

    #[test]
    fn test_current_and_default_values_are_always_available() {
        // (名前, 今の値, 既定値, 先頭から期待する並び)
        let cases: [(&str, &str, &str, &[&str]); 3] = [
            // 一覧にない値は、先頭に（今の値、既定値の順）
            ("custom-current", "Z", "🌿 ", &["Z"]),
            ("custom-both", "Z", "Y", &["Z", "Y"]),
            // 一覧にある値は、足さない（既定の並びのまま）
            ("in-catalog", "🌱 ", "🌿 ", &["🌿 ", "🌱 "]),
        ];
        for (name, current, default, head) in cases {
            let list = candidates("git_branch", "symbol", current, default);
            assert_eq!(&list[..head.len()], head, "{name}: {list:?}");
            assert!(list.iter().any(|c| c == current), "{name}: 今の値がない");
            assert!(list.iter().any(|c| c == default), "{name}: 既定値がない");
        }
    }

    #[test]
    fn test_every_symbol_prop_has_specific_candidates() {
        // 記号のプロパティを足したのに、候補を足し忘れることを検知する
        for part in &PARTS {
            for prop in part.props {
                if matches!(prop.kind, PropKind::Symbol) {
                    assert!(
                        !specific(part.id, prop.key).is_empty(),
                        "{}.{} の候補がない",
                        part.id,
                        prop.key
                    );
                }
            }
        }
    }
}
