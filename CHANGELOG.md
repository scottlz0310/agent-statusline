# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- 設定を対話的に編集・プレビューする TUI（`agent-statusline config`）の基本設計書 `docs/agent-statusline-config-tui-design.md` を追加。実装は含まない。
- `[directory] symbol`（既定 `"📁 "`）を追加。既定の `directory.format` は `$symbol[$folder]($folder_style) …` になったが、既定の出力は変わらない。絵文字を `format` に直書きしている既存の設定も、そのまま動く。
- 設定の編集サービス `editor` の土台を追加（設定 TUI フェーズ 0-3a）。設定ファイルの厳密な読み込み（構文エラーは既定値で置き換えず、元の内容とエラー位置を返す。未作成のファイルやリンク切れは保存先つきで扱う）と、トップレベル `format` を行と部品に分解して損失なく戻せる配置モデルを含む。まだ `config` サブコマンドからは呼ばれない。
- 設定の編集サービス `editor` に、TOML の部分更新（`set_prop` / `set_format`。変更したキーだけを書き込み、コメント・並び・未知のキー・`format` の書き方を保つ。既定値と同じ値はキーがなければ書かない）と、保存前の検証（`validate`。構文・型・閾値の範囲・スタイル名はエラー、未認識のキーや `format` の未知の変数・括弧の不整合は警告）を追加（設定 TUI フェーズ 0-3b）。まだ `config` サブコマンドからは呼ばれない。
- 設定の編集サービス `editor` に、設定ファイルの安全な保存（`save`）と履歴（`history`）を追加（設定 TUI フェーズ 0-3c）。保存前に検証し、既存のファイルはリンクを解決した実ファイルへ同じディレクトリの一時ファイル経由で書き込み（失敗時は旧内容のまま）、保存直前に内容ハッシュを再確認して外部の変更・削除があれば上書きしない。保存前の内容はローカルのデータディレクトリに 20 世代まで残す。未作成のファイルは、他で作られていれば上書きせず、リンク切れのリンク先の親ディレクトリがなければ作成しない。まだ `config` サブコマンドからは呼ばれない。
- `[directory] truncation_mode` を実装。`middle`（既定。中間を省略: `/foo/.../bar`）に加えて、`left`（先頭側を省略: `.../foo/bar`）を選べる。これまでは設定に存在するだけで、値にかかわらず `middle` として描画されていた。認識できない値でも描画は止めず `middle` として扱う。
- 設定のプレビュー基盤 `preview` を追加（設定 TUI フェーズ 0-4a）。実環境から独立したシナリオ（通常、長いパス・ブランチ、コンテキスト警告・上限間近、クォータ逼迫、レビュー待機・実行中、エージェント待機）を `compose` へ渡して描画し、端末幅に合わせて折り返す・切り詰める（ANSI の装飾を保ち、表示幅は `unicode-width` で計算する）。部品ごとの表示例も描画できる。まだ `config` サブコマンドからは呼ばれない。
- `agent-statusline config` サブコマンドを追加（設定 TUI フェーズ 0-4b の試作）。現状は、設定のプレビュー専用のビューア（読み取り専用）で、シナリオ・幅（auto / 80 / 120 / 160）・折り返し / 切り詰め・描画方式（ratatui / 生の ANSI）をキーで切り替え、部品ごとの表示例を並べる。端末での絵文字・折り返し・幅変更の確認に使う。対話的な端末でなければ、すぐ失敗する。`NO_COLOR` が設定されていても、プレビューは色つきで表示する（ステータスライン本体の `render` が `NO_COLOR` に関係なく色つきで出力するため。実機の確認で、`NO_COLOR=1` の端末では色が出ない問題を見つけて対処）。依存に `ratatui` を追加（リリースバイナリは約 +4.9%、`render` の起動時間への影響は計測上なし）。
- `agent-statusline config` で、配置（`format`）を編集できるようにした（設定 TUI フェーズ 1-1）。配置領域に行ごとの部品と固定テキストを並べ、選択（矢印）、移動（Shift+矢印。行の端では隣の行へ移る）、削除（`d`）、選択位置の前への改行（`n`）、末尾への行の追加（`a`）、元に戻す（`u`）ができ、プレビューは編集中の配置を即座に描く。何も表示しない部品は薄く示し、出力されない行にはその旨を添える。`Tab` でプレビューと配置のフォーカスを切り替える（シナリオ・幅などはプレビューにフォーカスがあるときのキー）。まだ設定ファイルへは保存できず、部品の追加は後続のフェーズで入る。
- `agent-statusline config` に部品一覧を追加した（設定 TUI フェーズ 1-2）。`Tab` で、配置 → 部品一覧 → プレビューの順にフォーカスを切り替える。部品一覧には、10 個の部品、定型の固定テキスト（空白と `|` `/` `-` の区切り）、改行を並べ、各部品に選択中のシナリオでの表示例を添える。カーソルを動かすと、選択中の項目を配置のカーソルの後ろに仮に入れた状態を、配置領域とプレビューに出す（確定するまで設定は変わらない）。`Enter` で後ろへ、`i` で前へ追加し、`u` で元に戻せる。
- 設定の編集サービス `editor` に、編集中の設定（下書き）`Draft` を追加（設定 TUI フェーズ 1-3a）。元の TOML 文書（コメント・未知のキーを保つ）、そこから導いた設定、配置の編集状態、検証の結果を持つ。プロパティの変更は、新しいエラーになる値（閾値の大小・範囲、認識できないスタイル名・選択肢、型の不一致など）を受け付けず、下書きを変えない（警告は受け付けて表示用に保持する）。読み込んだ設定が意味的なエラーを複数持っていても、1 つずつ直せる。配置とプロパティの変更は、時系列で 1 本の履歴に積み、`undo` で操作の逆順に戻す。保存用の TOML は、配置を `format` として書き込む（変えていないときは書かない）。まだ `config` サブコマンドからは呼ばれない。

### Changed
- `agent-statusline config` のプレビューと部品の表示例を、生の ANSI で端末へ直接出すようにした。ratatui の色の指定は 256 色の指標になり、太字の明るさが `render` の出力と食い違っていたため（設定 TUI フェーズ 1-2）。比較用だった描画方式の切り替え（`r`）は削除した。
- 部品（`directory`、`git_branch` など 10 個）の ID・設定キー・`format` で使える変数・描画関数を、部品レジストリ `engine/parts.rs` に集約した。描画の分岐はレジストリ起点になり、既定の設定との整合を検査するテストを追加した。出力は変わらない（設定 TUI フェーズ 0-2）。
- 描画を、副作用のある「収集」（Git の読み取り、サマリ JSON の読み込み）と、収集済みの入力だけから合成する純関数 `compose` に分離した。`render` の出力は変わらない。設定 TUI のプレビューが Git やサマリに触れずに描画できるようにするための前提整備（設定 TUI フェーズ 0-1）。

### Fixed
- `format` の置換を 1 パスにした。これまでは逐次置換だったため、ブランチ名・モデル名などの値に `$model` のような変数名が含まれると、後続の置換で再展開されていた。値はそのまま表示される。`$` に続く文字列は、その文脈で既知の変数名のうち最長一致として読み、直後の文字列は固定テキストとして保つ（既定の `$symbolCtx` は従来どおり `🧠 Ctx` と表示される）。

## [0.3.1] - 2026-09-29

### Changed
- CI と lefthook の clippy を `cargo clippy --all-targets -- -D warnings` に変更し、テストコードも検査対象にした。既存のテストコードで検出された 2 件（`format_push_string`、`too_many_lines`）を解消 (#38)。
- README と tasks.md の描画時間を、計測条件付きの実測値に更新 (#35)。
- README の `cargo install` 例を最新 Release の `v0.3.1` に更新。

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
