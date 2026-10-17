//! 部品レジストリ。描画の分岐、設定のキー、`format` で使える変数の、共通の根拠を 1 か所に宣言する。
//!
//! `props` と `PropKind` は、設定の編集サービスと TUI（設定 TUI フェーズ 0-3 以降）が参照する。

use crate::engine::config::Config;
use crate::engine::formatter::{
    render_agent_state, render_context, render_directory, render_git_branch, render_git_status,
    render_model, render_plan, render_quota, render_review, render_sandbox,
};
use crate::model::review_summary::{ActiveReview, QueuedReview, RepoReviews};
use crate::model::state::StatuslineState;
use crate::modules::directory::TruncationMode;
use crate::modules::git::GitStatus;

/// 部品の描画に渡す、収集済みの入力
pub struct PartInput<'a> {
    pub state: &'a StatuslineState,
    pub git: Option<&'a GitStatus>,
    pub reviews: &'a RepoReviews,
    pub terminal_width: usize,
}

/// 編集できる設定キーの種類
#[allow(dead_code)]
pub enum PropKind {
    Symbol,
    Style,
    Text,
    Separator,
    Bool,
    /// 決められた値のどれか（例: "left"、"middle"）
    Choice(&'static [&'static str]),
    /// 0..=100 の整数
    Threshold,
    /// 文字数の上限。0（自動）または `MIN_PATH_LEN`〜`MAX_PATH_LEN` の整数
    Length,
    /// `format` 文字列。`$変数` で使える変数名を持つ
    Format(&'static [&'static str]),
}

#[allow(dead_code)]
pub struct PropSpec {
    pub key: &'static str,
    pub kind: PropKind,
}

const fn prop(key: &'static str, kind: PropKind) -> PropSpec {
    PropSpec { key, kind }
}

pub struct PartSpec {
    /// `$id` と、`config.toml` のテーブル名に対応する
    pub id: &'static str,
    #[allow(dead_code)]
    pub props: &'static [PropSpec],
    pub render: fn(&Config, &PartInput) -> String,
}

const DISABLED: PropSpec = prop("disabled", PropKind::Bool);

pub static PARTS: [PartSpec; 10] = [
    PartSpec {
        id: "directory",
        props: &[
            prop(
                "format",
                PropKind::Format(&["symbol", "folder_style", "folder", "path_style", "path"]),
            ),
            prop("symbol", PropKind::Symbol),
            prop("folder_style", PropKind::Style),
            prop("path_style", PropKind::Style),
            prop(
                "truncation_mode",
                PropKind::Choice(&TruncationMode::CHOICES),
            ),
            prop("max_length", PropKind::Length),
            DISABLED,
        ],
        render: |c, i| render_directory(&c.directory, i.state, i.terminal_width),
    },
    PartSpec {
        id: "git_branch",
        props: &[
            prop("format", PropKind::Format(&["symbol", "branch", "style"])),
            prop("symbol", PropKind::Symbol),
            prop("style", PropKind::Style),
            DISABLED,
        ],
        render: |c, i| render_git_branch(&c.git_branch, i.git),
    },
    PartSpec {
        id: "git_status",
        props: &[
            prop("format", PropKind::Format(&["status", "style"])),
            prop("modified", PropKind::Symbol),
            prop("style", PropKind::Style),
            DISABLED,
        ],
        render: |c, i| render_git_status(&c.git_status, i.git),
    },
    PartSpec {
        id: "sandbox",
        props: &[
            prop("format", PropKind::Format(&["symbol", "style"])),
            prop("symbol", PropKind::Symbol),
            prop("style", PropKind::Style),
            DISABLED,
        ],
        render: |c, i| render_sandbox(&c.sandbox, i.state),
    },
    PartSpec {
        id: "model",
        props: &[
            prop("format", PropKind::Format(&["symbol", "model", "style"])),
            prop("symbol", PropKind::Symbol),
            prop("style", PropKind::Style),
            DISABLED,
        ],
        render: |c, i| render_model(&c.model, i.state),
    },
    PartSpec {
        id: "context",
        props: &[
            prop(
                "format",
                PropKind::Format(&["symbol", "percentage", "pct_style", "tokens", "tok_style"]),
            ),
            prop("symbol", PropKind::Symbol),
            prop("tok_style", PropKind::Style),
            prop("threshold_warning", PropKind::Threshold),
            prop("threshold_critical", PropKind::Threshold),
            prop("style_normal", PropKind::Style),
            prop("style_warning", PropKind::Style),
            prop("style_critical", PropKind::Style),
            DISABLED,
        ],
        render: |c, i| render_context(&c.context, i.state),
    },
    PartSpec {
        id: "agent_state",
        props: &[
            prop("format", PropKind::Format(&["symbol", "state", "style"])),
            prop("symbol", PropKind::Symbol),
            prop("style", PropKind::Style),
            DISABLED,
        ],
        render: |c, i| render_agent_state(&c.agent_state, i.state),
    },
    PartSpec {
        id: "plan",
        props: &[
            prop("format", PropKind::Format(&["tier", "style"])),
            prop("style", PropKind::Style),
            DISABLED,
        ],
        render: |c, i| render_plan(&c.plan, i.state),
    },
    PartSpec {
        id: "quota",
        props: &[
            prop(
                "format",
                PropKind::Format(&["symbol", "label", "percentage", "style", "reset_time"]),
            ),
            prop("symbol", PropKind::Symbol),
            prop("separator", PropKind::Separator),
            prop("threshold_warning", PropKind::Threshold),
            prop("threshold_critical", PropKind::Threshold),
            prop("style_normal", PropKind::Style),
            prop("style_warning", PropKind::Style),
            prop("style_critical", PropKind::Style),
            prop("hide_zero", PropKind::Bool),
            DISABLED,
        ],
        render: |c, i| render_quota(&c.quota, i.state),
    },
    PartSpec {
        id: "review",
        props: &[
            prop("format", PropKind::Format(&["symbol", "items", "style"])),
            prop("symbol", PropKind::Symbol),
            prop("active_format", PropKind::Format(&["pr", "round", "agent"])),
            prop(
                "waiting_format",
                PropKind::Format(&["pr", "round", "reason"]),
            ),
            prop("separator", PropKind::Separator),
            prop("style", PropKind::Style),
            DISABLED,
        ],
        render: render_review_part,
    },
];

/// Squirrel Notifier 連携が無効のときは、何も出力しない
fn render_review_part(config: &Config, input: &PartInput) -> String {
    if !config.integrations.squirrel_notifier.enabled {
        return String::new();
    }
    let active: Vec<&ActiveReview> = input.reviews.active.iter().collect();
    let waiting: Vec<&QueuedReview> = input.reviews.waiting.iter().collect();
    render_review(&config.review, &active, &waiting)
}

pub fn find(id: &str) -> Option<&'static PartSpec> {
    PARTS.iter().find(|part| part.id == id)
}

/// トップレベルの `format` で使える部品 ID
pub fn part_ids() -> Vec<&'static str> {
    PARTS.iter().map(|part| part.id).collect()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use toml_edit::{DocumentMut, Item};

    use super::*;
    use crate::engine::config::DEFAULT_FORMAT;
    use crate::engine::template::{Token, tokenize};
    use crate::modules::directory::is_valid_max_length;

    /// 未知の変数（`$` が固定テキストに残るもの）を含まないこと
    fn unknown_variable(format: &str, placeholders: &[&str]) -> Option<String> {
        tokenize(format, placeholders)
            .into_iter()
            .find_map(|t| match t {
                Token::Text(text) if text.contains('$') => Some(text.to_string()),
                _ => None,
            })
    }

    fn default_document() -> DocumentMut {
        toml_edit::ser::to_string(&Config::default())
            .unwrap()
            .parse()
            .unwrap()
    }

    #[test]
    fn test_part_ids_are_unique_and_not_prefixes() {
        let ids = part_ids();
        for (i, a) in ids.iter().enumerate() {
            for (j, b) in ids.iter().enumerate() {
                if i != j {
                    assert!(!b.starts_with(a), "`{a}` は `{b}` の接頭辞");
                }
            }
        }
    }

    #[test]
    fn test_registry_matches_default_config_tables() {
        let doc = default_document();
        let config_tables: BTreeSet<&str> = doc
            .iter()
            .filter(|(key, item)| item.is_table_like() && *key != "integrations")
            .map(|(key, _)| key)
            .collect();
        let part_tables: BTreeSet<&str> = part_ids().into_iter().collect();
        assert_eq!(part_tables, config_tables, "部品とテーブルの対応");

        for part in &PARTS {
            let table = doc[part.id].as_table_like().unwrap();
            let config_keys: BTreeSet<&str> = table.iter().map(|(key, _)| key).collect();
            let prop_keys: BTreeSet<&str> = part.props.iter().map(|p| p.key).collect();
            assert_eq!(prop_keys, config_keys, "{} のキー", part.id);
            assert_eq!(
                prop_keys.len(),
                part.props.len(),
                "{} のキーの重複",
                part.id
            );

            for p in part.props {
                let item = table.get(p.key).unwrap();
                let ok = match p.kind {
                    PropKind::Bool => item.is_bool(),
                    PropKind::Threshold => {
                        item.as_integer().is_some_and(|v| (0..=100).contains(&v))
                    }
                    PropKind::Length => item.as_integer().is_some_and(is_valid_max_length),
                    _ => item.is_str(),
                };
                assert!(ok, "{}.{} の型: {item:?}", part.id, p.key);
            }
        }
    }

    #[test]
    fn test_default_choice_values_are_valid_choices() {
        let doc = default_document();
        for part in &PARTS {
            for p in part.props {
                let PropKind::Choice(choices) = p.kind else {
                    continue;
                };
                let value = doc[part.id][p.key].as_str().unwrap();
                assert!(choices.contains(&value), "{}.{}: {value}", part.id, p.key);
            }
        }
    }

    #[test]
    fn test_default_formats_use_only_declared_placeholders() {
        let doc = default_document();
        for part in &PARTS {
            for p in part.props {
                let PropKind::Format(placeholders) = p.kind else {
                    continue;
                };
                let format = match &doc[part.id][p.key] {
                    Item::Value(v) => v.as_str().unwrap(),
                    other => panic!("{}.{} が文字列でない: {other:?}", part.id, p.key),
                };
                assert_eq!(
                    unknown_variable(format, placeholders),
                    None,
                    "{}.{}: {format:?}",
                    part.id,
                    p.key
                );
            }
        }
    }

    #[test]
    fn test_default_layout_uses_only_registered_parts() {
        assert_eq!(unknown_variable(DEFAULT_FORMAT, &part_ids()), None);
    }

    #[test]
    fn test_find() {
        assert!(find("review").is_some());
        assert!(find("review_x").is_none());
    }
}
