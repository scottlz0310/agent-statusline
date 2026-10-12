//! 設定のプレビュー基盤（設定 TUI フェーズ 0-4a）。画面に依存しない。
//!
//! シナリオ（実環境から独立したサンプル状態）を `compose` へ渡して描画し、端末幅に合わせて
//! 折り返す・切り詰める。Git・サマリ・更新確認・Notifier には触れない。
//!
//! フェーズ 1 の TUI が呼ぶまで、非テストビルドでは未使用の項目がある。
#![allow(dead_code)]

pub mod scenario;
pub mod width;

use crate::editor::layout::{Layout, Token};
use crate::engine::config::Config;
use crate::engine::formatter::compose;
use crate::engine::parts::{PartInput, find};
use crate::engine::style::render_styled_text;
use scenario::Scenario;
use width::{Overflow, fit_output, strip_ansi};

/// 全体のプレビュー。`columns` は、`compose` に渡す端末幅と、折り返し・切り詰めの幅を兼ねる
pub fn render_preview(
    config: &Config,
    scenario: Scenario,
    columns: usize,
    overflow: Overflow,
) -> Vec<String> {
    let inputs = scenario.inputs();
    let output = compose(
        config,
        &inputs.state,
        inputs.git.as_ref(),
        &inputs.reviews,
        columns,
    );
    fit_output(&output, columns, overflow)
}

/// 部品 1 つの表示例（部品一覧に並べる短い表示。ANSI の装飾つき）。何も表示されないときは空文字列
pub fn render_part_sample(
    config: &Config,
    scenario: Scenario,
    part_id: &str,
    columns: usize,
) -> Option<String> {
    let part = find(part_id)?;
    let inputs = scenario.inputs();
    let input = PartInput {
        state: &inputs.state,
        git: inputs.git.as_ref(),
        reviews: &inputs.reviews,
        terminal_width: columns,
    };
    // 全体のプレビュー（compose）と同じく、[text](style) を装飾へ変換する
    Some(render_styled_text(&(part.render)(config, &input)))
}

/// 配置の 1 行について、非表示の部品と、行そのものが出力されるか（設計書 §6.5）
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineVisibility {
    /// トークンごと。何も表示しない部品は `true`（固定テキストは常に `false`）
    pub hidden: Vec<bool>,
    /// 行全体が空になり、描画から除かれる
    pub blank: bool,
}

pub fn layout_visibility(
    config: &Config,
    scenario: Scenario,
    layout: &Layout,
    columns: usize,
) -> Vec<LineVisibility> {
    layout
        .lines
        .iter()
        .map(|line| {
            let mut text = String::new();
            let hidden = line
                .iter()
                .map(|token| match token {
                    Token::Literal(literal) => {
                        text.push_str(literal);
                        false
                    }
                    Token::Part(id) => {
                        let sample = render_part_sample(config, scenario, id, columns)
                            .map(|sample| strip_ansi(&sample))
                            .unwrap_or_default();
                        text.push_str(&sample);
                        sample.trim().is_empty()
                    }
                })
                .collect();
            LineVisibility {
                hidden,
                // `compose` と同じく、行末の空白を除いて空かどうかを見る
                blank: text.trim_end().is_empty(),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::parts::part_ids;
    use width::display_width;

    fn plain_lines(config: &Config, scenario: Scenario, columns: usize) -> Vec<String> {
        render_preview(config, scenario, columns, Overflow::Truncate)
            .iter()
            .map(|line| strip_ansi(line))
            .collect()
    }

    #[test]
    fn test_scenarios_render_expected_content() {
        let config = Config::default();
        // (シナリオ, 含まれる文字列, 含まれない文字列)
        let cases: [(Scenario, &[&str], &[&str]); 8] = [
            (
                Scenario::Normal,
                &[
                    "myproject",
                    "🌿 main",
                    "✗",
                    "sandbox",
                    "Claude Opus",
                    "Ctx: 42%",
                    "working",
                    "Pro",
                    "5h: 12%",
                ],
                &["🐿"],
            ),
            (
                Scenario::LongPathAndBranch,
                &["/.../", "feature/very-long-branch-name"],
                &[],
            ),
            (Scenario::ContextWarning, &["Ctx: 65%"], &[]),
            (Scenario::ContextCritical, &["Ctx: 92%"], &[]),
            (
                Scenario::QuotaTight,
                &["5h: 88% (rst 25m)", "7d: 96%", "Opus 7d: 71%"],
                &["Opus 7d: 71% (rst"],
            ),
            (Scenario::ReviewWaiting, &["🐿 ⏳#32 r1"], &["🔍"]),
            (Scenario::ReviewActive, &["🐿 🔍#31 r2 ⏳#32 r1"], &[]),
            (Scenario::AgentIdle, &["Claude Opus"], &["idle", "working"]),
        ];
        for (scenario, contains, excludes) in cases {
            let text = plain_lines(&config, scenario, 120).join("\n");
            for expected in contains {
                assert!(
                    text.contains(expected),
                    "{scenario:?}: {expected} がない\n{text}"
                );
            }
            for unexpected in excludes {
                assert!(
                    !text.contains(unexpected),
                    "{scenario:?}: {unexpected} がある\n{text}"
                );
            }
        }
    }

    #[test]
    fn test_threshold_scenarios_change_colors() {
        let config = Config::default();
        let ansi = |scenario| render_preview(&config, scenario, 120, Overflow::Truncate).join("\n");
        // 通常は緑、警告は黄、上限間近は赤（既定の閾値 50 / 80）
        assert!(ansi(Scenario::Normal).contains("\x1b[32m🧠 Ctx: 42%"));
        assert!(ansi(Scenario::ContextWarning).contains("\x1b[33m🧠 Ctx: 65%"));
        assert!(ansi(Scenario::ContextCritical).contains("\x1b[31m🧠 Ctx: 92%"));
    }

    #[test]
    fn test_preview_respects_columns() {
        let config = Config::default();
        for scenario in Scenario::ALL {
            for columns in [20, 40, 80, 120, 160] {
                for overflow in [Overflow::Wrap, Overflow::Truncate] {
                    for line in render_preview(&config, scenario, columns, overflow) {
                        // 1 文字が幅より広いときだけ、幅を超えてよい
                        assert!(
                            display_width(&line) <= columns.max(2),
                            "{scenario:?}/{columns}/{overflow:?}: {line:?}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn test_wrap_keeps_text_truncate_drops_it() {
        let config = Config::default();
        let plain = |overflow| {
            render_preview(&config, Scenario::Normal, 30, overflow)
                .iter()
                .map(|line| strip_ansi(line))
                .collect::<Vec<_>>()
        };
        let wrapped = plain(Overflow::Wrap);
        let truncated = plain(Overflow::Truncate);
        assert_eq!(truncated.len(), 3, "切り詰めは行数を変えない");
        assert!(wrapped.len() > 3, "折り返しは行を増やす");
        assert!(
            wrapped.join("").contains("7d: 30% (rst 2d3h)"),
            "折り返しは文字を失わない"
        );
        assert!(!truncated.join("").contains("7d: 30% (rst 2d3h)"));
    }

    #[test]
    fn test_directory_path_depends_on_columns() {
        let config = Config::default();
        let wide = plain_lines(&config, Scenario::LongPathAndBranch, 400).join("\n");
        let narrow = plain_lines(&config, Scenario::LongPathAndBranch, 80).join("\n");
        assert!(wide.contains("deeply/nested/workspace"), "{wide}");
        assert!(narrow.contains("/.../"), "{narrow}");
    }

    #[test]
    fn test_part_samples() {
        let config = Config::default();
        // (シナリオ, 部品, 表示の有無)
        let cases = [
            (Scenario::Normal, "git_branch", true),
            (Scenario::Normal, "git_status", true),
            (Scenario::Normal, "review", false),
            (Scenario::ReviewActive, "review", true),
            (Scenario::AgentIdle, "agent_state", false),
            (Scenario::Normal, "agent_state", true),
            (Scenario::Normal, "plan", true),
        ];
        for (scenario, part, visible) in cases {
            let sample = render_part_sample(&config, scenario, part, 120).unwrap();
            assert_eq!(
                !sample.is_empty(),
                visible,
                "{scenario:?}/{part}: {sample:?}"
            );
        }
        assert_eq!(
            render_part_sample(&config, Scenario::Normal, "nope", 120),
            None
        );

        // `disabled` の部品は、どのシナリオでも空
        let mut disabled = Config::default();
        disabled.model.disabled = true;
        assert_eq!(
            render_part_sample(&disabled, Scenario::Normal, "model", 120).as_deref(),
            Some("")
        );
    }

    #[test]
    fn test_part_samples_apply_styles_like_the_full_preview() {
        let sample = |config: &Config, id: &str| {
            render_part_sample(config, Scenario::Normal, id, 120).unwrap()
        };
        // 既定のスタイル: テンプレート構文（`[text](style)`）を露出せず、装飾に変換する
        let default = Config::default();
        assert_eq!(sample(&default, "git_branch"), "\x1b[32m🌿 main\x1b[0m ");
        assert!(sample(&default, "model").contains("\x1b[35m"));

        // 変更したスタイルも反映される
        let mut custom = Config::default();
        custom.git_branch.style = "bold red".to_string();
        assert_eq!(sample(&custom, "git_branch"), "\x1b[1;31m🌿 main\x1b[0m ");

        // 全部品・全シナリオで、全体のプレビューと同じ結果になる（末尾の空白を除く）
        for id in part_ids() {
            for scenario in Scenario::ALL {
                let config = Config {
                    format: format!("${id}"),
                    ..Config::default()
                };
                let expected = render_preview(&config, scenario, 120, Overflow::Truncate);
                let actual = render_part_sample(&config, scenario, id, 120).unwrap();
                assert_eq!(expected.join("\n"), actual.trim_end(), "{scenario:?}/{id}");
                assert!(!actual.contains("]("), "{scenario:?}/{id}: {actual:?}");
            }
        }
    }

    #[test]
    fn test_every_part_is_visible_in_some_scenario() {
        // 全部品が、いずれかのシナリオで表示される（部品を足したのにサンプルがない、を検知する）
        let config = Config::default();
        for id in part_ids() {
            let visible = Scenario::ALL.iter().any(|scenario| {
                render_part_sample(&config, *scenario, id, 120).is_some_and(|s| !s.is_empty())
            });
            assert!(visible, "{id}");
        }
    }

    #[test]
    fn test_scenario_labels_are_unique() {
        let labels: Vec<&str> = Scenario::ALL.iter().map(|s| s.label()).collect();
        for (i, label) in labels.iter().enumerate() {
            assert!(!labels[..i].contains(label), "{label}");
        }
    }

    /// 1 行ぶんの期待値: (トークンごとの非表示, 行が空か)
    type ExpectedLine = (&'static [bool], bool);

    #[test]
    fn test_layout_visibility() {
        let config = Config::default();
        let cases: [(&str, &str, &[ExpectedLine]); 4] = [
            (
                "all-visible",
                "$directory $model",
                &[(&[false, false, false], false)],
            ),
            (
                // 通常のシナリオでは、レビューは表示されない。行の固定テキストが空白だけなら、行も出力されない
                "hidden-part-and-blank-line",
                "$review $review
$model",
                &[(&[true, false, true], true), (&[false], false)],
            ),
            (
                // 固定テキストが残る行は出力される
                "literal-keeps-line",
                "$review |",
                &[(&[true, false], false)],
            ),
            (
                "empty-line",
                "$model
",
                &[(&[false], false), (&[], true)],
            ),
        ];
        for (name, format, expected) in cases {
            let visibility =
                layout_visibility(&config, Scenario::Normal, &Layout::parse(format), 100);
            let actual: Vec<(Vec<bool>, bool)> = visibility
                .into_iter()
                .map(|line| (line.hidden, line.blank))
                .collect();
            let expected: Vec<(Vec<bool>, bool)> = expected
                .iter()
                .map(|(hidden, blank)| (hidden.to_vec(), *blank))
                .collect();
            assert_eq!(actual, expected, "{name}");
        }
    }
}
