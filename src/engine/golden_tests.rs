//! 描画の分離（設定 TUI フェーズ 0-1）の前後で出力が変わらないことを担保するゴールデンテスト。
//! Git・サマリに依存しない範囲を、`render_template` の公開動作として固定する。

use std::path::PathBuf;

use crate::engine::config::Config;
use crate::engine::formatter::render_template;
use crate::model::state::{QuotaItem, StatuslineState};

fn quota(label: &str, pct: Option<u8>, reset: Option<i64>) -> QuotaItem {
    QuotaItem {
        id: label.to_string(),
        label: label.to_string(),
        used_percentage: pct,
        reset_in_seconds: reset,
        reset_time_iso: None,
    }
}

fn full_state() -> StatuslineState {
    StatuslineState {
        cwd: PathBuf::from("/home/dev/src/myproject"),
        model: Some("Claude Opus".to_string()),
        context_used_percentage: Some(72),
        total_input_tokens: Some(50_000),
        total_output_tokens: Some(12_000),
        agent_state: Some("working".to_string()),
        sandbox_enabled: true,
        plan_tier: Some("Pro".to_string()),
        quotas: vec![
            quota("5h", Some(5), Some(3600)),
            quota("7d", Some(30), Some(183_600)),
        ],
    }
}

fn config(toml: &str) -> Config {
    toml_edit::de::from_str(toml).unwrap()
}

fn state_with(f: impl FnOnce(&mut StatuslineState)) -> StatuslineState {
    let mut state = full_state();
    f(&mut state);
    state
}

fn state_full() -> StatuslineState {
    full_state()
}

fn state_empty() -> StatuslineState {
    StatuslineState {
        cwd: PathBuf::from("/home/dev/src/myproject"),
        ..Default::default()
    }
}

fn state_long_cwd() -> StatuslineState {
    state_with(|s| {
        s.cwd =
            PathBuf::from("/home/dev/src/very/deeply/nested/workspace/directory/for/the/project");
    })
}

fn state_warning() -> StatuslineState {
    state_with(|s| {
        s.context_used_percentage = Some(75);
        s.quotas = vec![quota("5h", Some(75), Some(60))];
    })
}

fn state_critical() -> StatuslineState {
    state_with(|s| {
        s.context_used_percentage = Some(95);
        s.quotas = vec![quota("5h", Some(95), None), quota("7d", None, Some(10))];
    })
}

fn state_idle() -> StatuslineState {
    state_with(|s| s.agent_state = Some("idle".to_string()))
}

fn cfg_default() -> Config {
    Config::default()
}

fn cfg_all_parts() -> Config {
    config(
        "format = \"$directory $git_branch $git_status $sandbox $review\\n$model $context $agent_state $plan\\n$quota\"",
    )
}

fn cfg_disabled() -> Config {
    config(
        "[directory]\ndisabled = true\n[model]\ndisabled = true\n[quota]\ndisabled = true\n[sandbox]\ndisabled = true\n",
    )
}

fn cfg_custom_layout() -> Config {
    config(
        r#"
format = "$model|$plan\n\n$directory   \n$context_x"
[model]
format = "<[$symbol$model]($style)>"
symbol = "M:"
style = "bold magenta"
[plan]
format = "[$tier]($style)"
[directory]
format = "[$folder_style]($folder_style)[$folderX]($path_style)[$pathY]"
[context]
format = "[$tokensX $symbolCtx $percentage $unknown]($pct_style) [$tok_style]($tok_style)"
"#,
    )
}

fn cfg_custom_quota() -> Config {
    config(
        r#"
format = "$quota"
[quota]
format = "[$label=$percentage rst=$reset_time]($style)"
separator = " ; "
"#,
    )
}

fn cfg_thresholds() -> Config {
    config(
        r#"
format = "$context $quota"
[context]
threshold_warning = 50
threshold_critical = 70
[quota]
threshold_warning = 10
threshold_critical = 20
"#,
    )
}

/// ケース名から (設定, 状態, 端末幅) を引く
fn inputs(name: &str) -> (Config, StatuslineState, usize) {
    match name {
        "default-full" => (cfg_default(), state_full(), 120),
        "default-empty" => (cfg_default(), state_empty(), 80),
        "default-long-path" => (cfg_default(), state_long_cwd(), 80),
        "default-long-path-wide" => (cfg_default(), state_long_cwd(), 200),
        "default-warning" => (cfg_default(), state_warning(), 120),
        "default-critical" => (cfg_default(), state_critical(), 120),
        "default-idle" => (cfg_default(), state_idle(), 120),
        "all-parts" => (cfg_all_parts(), state_full(), 120),
        "disabled" => (cfg_disabled(), state_full(), 120),
        "custom-layout" => (cfg_custom_layout(), state_full(), 120),
        "custom-quota" => (cfg_custom_quota(), state_critical(), 120),
        "thresholds" => (cfg_thresholds(), state_warning(), 120),
        _ => panic!("unknown golden case: {name}"),
    }
}

/// 分離前の実装で採取した出力
const GOLDEN: [(&str, &str); 12] = [
    (
        "default-full",
        "📁 \u{1b}[1;36mmyproject\u{1b}[0m \u{1b}[2m[/home/dev/src/myproject]\u{1b}[0m \u{1b}[33m🔒 sandbox\u{1b}[0m\n\u{1b}[35m🤖 Claude Opus\u{1b}[0m \u{1b}[33m🧠 Ctx: 72%\u{1b}[0m \u{1b}[2m(62k tok)\u{1b}[0m \u{1b}[33m⚡ working\u{1b}[0m \u{1b}[34mPro\u{1b}[0m\n\u{1b}[32m⏳ 5h: 5% (rst 1h0m)\u{1b}[0m   |  \u{1b}[32m⏳ 7d: 30% (rst 2d3h)\u{1b}[0m",
    ),
    (
        "default-empty",
        "📁 \u{1b}[1;36mmyproject\u{1b}[0m \u{1b}[2m[/home/dev/src/myproject]\u{1b}[0m",
    ),
    (
        "default-long-path",
        "📁 \u{1b}[1;36mproject\u{1b}[0m \u{1b}[2m[/home/dev/src/very/.../y/for/the/project]\u{1b}[0m \u{1b}[33m🔒 sandbox\u{1b}[0m\n\u{1b}[35m🤖 Claude Opus\u{1b}[0m \u{1b}[33m🧠 Ctx: 72%\u{1b}[0m \u{1b}[2m(62k tok)\u{1b}[0m \u{1b}[33m⚡ working\u{1b}[0m \u{1b}[34mPro\u{1b}[0m\n\u{1b}[32m⏳ 5h: 5% (rst 1h0m)\u{1b}[0m   |  \u{1b}[32m⏳ 7d: 30% (rst 2d3h)\u{1b}[0m",
    ),
    (
        "default-long-path-wide",
        "📁 \u{1b}[1;36mproject\u{1b}[0m \u{1b}[2m[/home/dev/src/very/deeply/nested/workspace/directory/for/the/project]\u{1b}[0m \u{1b}[33m🔒 sandbox\u{1b}[0m\n\u{1b}[35m🤖 Claude Opus\u{1b}[0m \u{1b}[33m🧠 Ctx: 72%\u{1b}[0m \u{1b}[2m(62k tok)\u{1b}[0m \u{1b}[33m⚡ working\u{1b}[0m \u{1b}[34mPro\u{1b}[0m\n\u{1b}[32m⏳ 5h: 5% (rst 1h0m)\u{1b}[0m   |  \u{1b}[32m⏳ 7d: 30% (rst 2d3h)\u{1b}[0m",
    ),
    (
        "default-warning",
        "📁 \u{1b}[1;36mmyproject\u{1b}[0m \u{1b}[2m[/home/dev/src/myproject]\u{1b}[0m \u{1b}[33m🔒 sandbox\u{1b}[0m\n\u{1b}[35m🤖 Claude Opus\u{1b}[0m \u{1b}[33m🧠 Ctx: 75%\u{1b}[0m \u{1b}[2m(62k tok)\u{1b}[0m \u{1b}[33m⚡ working\u{1b}[0m \u{1b}[34mPro\u{1b}[0m\n\u{1b}[33m⏳ 5h: 75% (rst 1m)\u{1b}[0m",
    ),
    (
        "default-critical",
        "📁 \u{1b}[1;36mmyproject\u{1b}[0m \u{1b}[2m[/home/dev/src/myproject]\u{1b}[0m \u{1b}[33m🔒 sandbox\u{1b}[0m\n\u{1b}[35m🤖 Claude Opus\u{1b}[0m \u{1b}[31m🧠 Ctx: 95%\u{1b}[0m \u{1b}[2m(62k tok)\u{1b}[0m \u{1b}[33m⚡ working\u{1b}[0m \u{1b}[34mPro\u{1b}[0m\n\u{1b}[31m⏳ 5h: 95%\u{1b}[0m",
    ),
    (
        "default-idle",
        "📁 \u{1b}[1;36mmyproject\u{1b}[0m \u{1b}[2m[/home/dev/src/myproject]\u{1b}[0m \u{1b}[33m🔒 sandbox\u{1b}[0m\n\u{1b}[35m🤖 Claude Opus\u{1b}[0m \u{1b}[33m🧠 Ctx: 72%\u{1b}[0m \u{1b}[2m(62k tok)\u{1b}[0m \u{1b}[34mPro\u{1b}[0m\n\u{1b}[32m⏳ 5h: 5% (rst 1h0m)\u{1b}[0m   |  \u{1b}[32m⏳ 7d: 30% (rst 2d3h)\u{1b}[0m",
    ),
    (
        "all-parts",
        "📁 \u{1b}[1;36mmyproject\u{1b}[0m \u{1b}[2m[/home/dev/src/myproject]\u{1b}[0m    \u{1b}[33m🔒 sandbox\u{1b}[0m\n\u{1b}[35m🤖 Claude Opus\u{1b}[0m  \u{1b}[33m🧠 Ctx: 72%\u{1b}[0m \u{1b}[2m(62k tok)\u{1b}[0m  \u{1b}[33m⚡ working\u{1b}[0m  \u{1b}[34mPro\u{1b}[0m\n\u{1b}[32m⏳ 5h: 5% (rst 1h0m)\u{1b}[0m   |  \u{1b}[32m⏳ 7d: 30% (rst 2d3h)\u{1b}[0m",
    ),
    (
        "disabled",
        "\u{1b}[33m🧠 Ctx: 72%\u{1b}[0m \u{1b}[2m(62k tok)\u{1b}[0m \u{1b}[33m⚡ working\u{1b}[0m \u{1b}[34mPro\u{1b}[0m",
    ),
    (
        "custom-layout",
        "<\u{1b}[1;35mM:Claude Opus\u{1b}[0m>|\u{1b}[34mPro\u{1b}[0m\n\u{1b}[1;36mbold cyan\u{1b}[0m\u{1b}[2mmyprojectX\u{1b}[0m[/home/dev/src/myprojectY]\n\u{1b}[33m62kX 🧠 Ctx 72% $unknown\u{1b}[0m \u{1b}[2mdimmed\u{1b}[0m_x",
    ),
    ("custom-quota", "\u{1b}[31m5h=95% rst=\u{1b}[0m"),
    (
        "thresholds",
        "\u{1b}[31m🧠 Ctx: 75%\u{1b}[0m \u{1b}[2m(62k tok)\u{1b}[0m  \u{1b}[31m⏳ 5h: 75% (rst 1m)\u{1b}[0m",
    ),
];

#[test]
fn test_golden_render_template() {
    for (name, expected) in GOLDEN {
        let (cfg, state, width) = inputs(name);
        assert_eq!(render_template(&cfg, &state, width), expected, "{name}");
    }
}
