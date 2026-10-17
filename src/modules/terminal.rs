/// 端末の幅を取れないときの幅
const DEFAULT_WIDTH: usize = 80;

/// 描画に使う端末の幅。実際の端末の幅を優先し、取れないときは環境変数 `COLUMNS`、それも使えなければ 80。
///
/// エージェント CLI（Claude Code など）から実行されると stdin / stdout / stderr がすべてパイプで、
/// 端末の幅は取れない。そのとき `COLUMNS` には実際のウィンドウ幅が入っている。
pub fn width() -> usize {
    let size = terminal_size::terminal_size().map(|(w, _)| usize::from(w.0));
    resolve_width(size, std::env::var("COLUMNS").ok().as_deref())
}

/// 幅の決定規則。0 や数値でない値は「取れなかった」ものとして扱う
fn resolve_width(size: Option<usize>, columns: Option<&str>) -> usize {
    size.filter(|w| *w > 0)
        .or_else(|| {
            columns
                .and_then(|c| c.trim().parse::<usize>().ok())
                .filter(|w| *w > 0)
        })
        .unwrap_or(DEFAULT_WIDTH)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_width() {
        // (名前, 端末の幅, COLUMNS, 期待する幅)
        let cases = [
            ("terminal wins over COLUMNS", Some(100), Some("60"), 100),
            ("terminal only", Some(100), None, 100),
            ("COLUMNS when no terminal", None, Some("118"), 118),
            ("COLUMNS with spaces", None, Some(" 49 "), 49),
            (
                "zero terminal falls back to COLUMNS",
                Some(0),
                Some("90"),
                90,
            ),
            ("zero COLUMNS", None, Some("0"), DEFAULT_WIDTH),
            ("non-numeric COLUMNS", None, Some("wide"), DEFAULT_WIDTH),
            ("negative COLUMNS", None, Some("-5"), DEFAULT_WIDTH),
            ("empty COLUMNS", None, Some(""), DEFAULT_WIDTH),
            ("nothing", None, None, DEFAULT_WIDTH),
        ];
        for (name, size, columns, expected) in cases {
            assert_eq!(resolve_width(size, columns), expected, "{name}");
        }
    }
}
