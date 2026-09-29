# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.3.1] - 2026-09-29

### Changed
- CI と lefthook の clippy を `cargo clippy --all-targets -- -D warnings` に変更し、テストコードも検査対象にした。既存のテストコードで検出された 2 件（`format_push_string`、`too_many_lines`）を解消 (#38)。
- README と tasks.md の描画時間を、計測条件付きの実測値に更新 (#35)。

### Fixed
- Windows で Git モジュールを使うと、描画のたびに gix がシステムの Git 設定の場所を調べるため `git` を 2 回起動し、既定レイアウトの描画に約 130ms かかっていた問題を修正。システムの gitconfig のパスを 24 時間キャッシュして gix に渡し、システムの gitattributes は読み込まないようにした。既定レイアウトの描画時間は約 11〜16ms になった (#35)。

## [0.3.0] - 2026-09-27

### Added
- `$review` モジュールを追加。Squirrel Notifier の `statusline-summary.json` から、カレントリポジトリの PR のうちレビュー実行中と reviewer 起動待ちのものを表示する（例: `🐿 🔍#31 r2 ⏳#32 r1`）。既定レイアウトの 1 行目に含め、`[review]` と `[integrations.squirrel_notifier] summary_path` で変更できる。Squirrel Notifier 未起動時やリポジトリ外では何も表示しない (#23)。

### Changed
- `$git_status` を使わないレイアウトでは、Git の変更状態の判定を省略するようにした。
- README の `cargo install` 例を最新 Release の `v0.3.0` に更新。
- Release / Prepare Release workflow が参照する `release-automate` の reusable workflow を v2.0.0（`fd3e676`）に更新し、Renovate が追跡できるようバージョンコメントを付与。

### Fixed
- Release workflow の初回実行で、draft Release の作成直後に一覧 API への反映が遅れて `draft / publish` が失敗する問題を、`release-automate` v2.0.0 の再試行で解消（v0.2.0 リリース時に発生、scottlz0310/release-automate#22）。

## [0.2.0] - 2026-09-27

### Added
- Linux shell installer を追加し、ZIP の SHA-256 を検証して `~/.local/bin` に配置。旧 cargo-dist の receipt、`env` helper、profile 設定は保持。
- v0.1.0 からの更新、旧インストールの残存物確認と手動整理を README に記載。
- README に GitHub Release と `cargo install` それぞれのアンインストール手順を追加し、Linux の receipt、env helper の読み込み行、更新確認キャッシュの削除を記載。
- README に Windows での PowerShell インストーラーの Defender 検出事例と、Cargo インストールでの検証結果を追記。

### Changed
- cargo-dist の生成 workflow を撤廃し、`release-automate` の draft Release と taiki-e の Rust Actions による配布 workflow に置き換え。3 ターゲットの ZIP・`.zip.sha256`・リポジトリ内 installer を draft に添付し、asset 名・checksum・ZIP 構造・各 runner でのバイナリ起動を検証した後にだけ公開する。PR では公開権限なしで同じビルドと検証を行う。`dist-workspace.toml` と `profile.dist` を削除 (#24)。
- 自己更新と Windows installer で Release ZIP の SHA-256 を照合し、検証失敗時は既存バイナリを保持。未対応 OS/arch は明示的にエラーとする。

### Fixed
- `render --agent claude` のレートリミット出力を Squirrel Notifier の契約に合わせ、`ratelimit-status/claude-code.json`（`agentId: "claude-code"`）へ書き出すよう修正。CLI 引数の `claude` は変更なし。v0.1.0 が書き出していた `ratelimit-status/claude.json` は参照されないため手動で削除してよい (#28)。

## [0.1.0] - 2026-09-26

### Changed
- README と設計仕様書を現在の実装に合わせ、初回リリース時の `agent-statusline update --force` による asset 取得確認を明記。

### Added
- Self-update subcommand (`agent-statusline update`) with `--check` and `--force` flags, supporting zero-downtime in-process binary replacement with atomic staging (`.new`), `.old` backup, and rollback protection on failure.
- `render` は 24 時間間隔で更新キャッシュを確認し、期限切れ時は同じ実行ファイルのバックグラウンド更新確認へ委譲。
- `cargo-dist` release pipeline configuration (`dist-workspace.toml`, `Cargo.toml` profile.dist, `.github/workflows/release.yml`) generating cross-compilation release builds, checksums, and shell/PowerShell installers for Windows (`x86_64-pc-windows-msvc`), Linux musl (`x86_64-unknown-linux-musl`), and ARM Linux musl (`aarch64-unknown-linux-musl`).
- Windows standalone PowerShell installer script (`scripts/agent-statusline-installer.ps1`) targeting `%LOCALAPPDATA%\Programs\agent-statusline` and configuring User `PATH`.
- Lefthook Git hooks configuration (`lefthook.yml`) executing `rustfmt`, `clippy`, code-behind size ratchet, and tests locally on pre-commit and pre-push, with installation and hook registration documented in `README.md`.
- Renovate configuration (`renovate.json`) integrating shared presets from `scottlz0310/renovate-config` for Rust, PowerShell, Lefthook, automerge, schedule, and security.
- Codecov coverage measurement pipeline and quality gate configuration (`codecov.yml`: target 80%, threshold 2% project / 5% patch, `informational: false`) protecting domain logic while excluding ratchet-guarded code-behind entrypoints (#9).
- Code-behind line count ratchet guard script (`scripts/check-code-behind-size.ps1`) to strictly prevent logic creep in untested entrypoints (#9).
- Starship-like TOML configuration engine (`config.toml`) supporting declarative statusline layouts and module customization (`src/engine/config.rs`).
- Inline style syntax parser (`[text](style)`) translating styles, modifiers, and foreground/background colors into ANSI escape sequences (`src/engine/style.rs`).
- Dynamic template variable expansion engine for all statusline modules with zero-configuration fallback (`src/engine/formatter.rs`).
- `--config <PATH>` CLI option on `render` subcommand and `AGENT_STATUSLINE_CONFIG` environment variable resolution.
- Automated client installer (`install`), uninstaller (`uninstall`), and configuration diagnostics (`status`) for Antigravity, Claude Code, and GitHub Copilot CLI.

### Removed
- Remove unused skeleton `init` subcommand, `--shell` CLI option on `render`, and `ShellKind` enum, tightening code-behind ratchet limits to 300 lines (`src/main.rs`) and 114 lines (`src/cli.rs`).

### Fixed
- Unify release distribution archives to ZIP across Windows and Linux musl targets in `cargo-dist` (`dist-workspace.toml`), ensuring reliable zero-dependency extraction in the self-update engine (`src/updater/`).
- Prevent runaway background update check spawns during offline conditions or GitHub API outages by immediately recording check attempt timestamps and ensuring cache update upon API failures.
- Expand environment variables (`%VAR%`, `$VAR`, `${VAR}`) in `output_dir` for custom Squirrel Notifier ratelimit output.
- Eliminate redundant in-process Git status lookups and evaluate modules conditionally in template renderer.
- Prevent false positives during uninstall and diagnostics when custom wrapper commands include `agent-statusline` in their name by strictly verifying command basename and arguments.
- Safe JSON configuration patcher preserving existing settings, atomic file replacement, automated backup creation (`.bak`), and JSONC comment preservation.
- Dry-run mode (`--dry-run`) with formatted JSON preview for safe configuration inspection.
- Core rendering engine (`render`) supporting zero-fork, sub-5ms latency statusline generation.
- Client adapters for Antigravity CLI (`agy`), Claude Code (`claude`), and GitHub Copilot CLI (`copilot`, supporting both `context_window` and legacy `context` schemas with standalone context token accounting).
- In-process Git branch and dirty state detector powered by `gix`.
- Atomic Squirrel Notifier ratelimit status writer (`%LOCALAPPDATA%/SquirrelNotifier/ratelimit-status/<agent>.json`).
- Benchmarking flag (`--bench`) for microsecond-precision latency inspection.
- Comprehensive unit tests covering all adapters, modules, rendering, and atomic file sinks.
- Initial project scaffolding and comprehensive design specifications (`docs/agent-statusline-spec.md`).
- Multi-client architecture design for Antigravity CLI, Claude Code, and GitHub Copilot CLI.
- Zero-downtime self-update design (`agent-statusline update` and background release check).
- Task management and tracking document (`tasks.md`).
