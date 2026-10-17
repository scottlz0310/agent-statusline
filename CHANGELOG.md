# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- 設定エディタ（`agent-statusline config`）に、検証結果の一覧（`v`）を追加。状態行には先頭の 1 件しか出なかったエラー・警告（綴りの誤りなどで認識できないキーを含む）を、全件確認できる。`↑↓` で選び、`Esc` / `v` で閉じる。状態行には、ほかにもあるときに「v: 全件を表示」を添える (#82)。
- `[directory] max_length` を追加。パスの最大文字数を固定する。`0`（既定）は端末の幅の半分（自動）、`10`〜`1000` はその文字数で固定し、端末の幅の取得に左右されない。設定エディタ（`agent-statusline config`）では `←` `→`（`Shift` で ±10）で増減でき、`0` と `10` の間の値は受け付けない。保存前の検証も同じ範囲を要求する (#83)。

### Fixed
- Claude Code など、標準入出力がすべてパイプになるエージェントから実行されると、端末の幅が取れず常に 80 として描画していた。端末の幅が取れないときは、環境変数 `COLUMNS` を使う（端末の幅が取れるときはそちらが優先。`COLUMNS` が 0 や数値でないときは無視して 80）。`[directory]` のパス省略がウィンドウの幅に追従する (#83)。

### Changed
- `[directory]` のパスの上限を `(幅 / 2)` にし、下限 `40` を撤廃した。幅 80 では従来どおり 40 文字だが、幅が 80 未満なら上限も小さくなる。また、`COLUMNS` を読むようになったため、Claude Code の幅の広いウィンドウ（例: 118 列）では、長いパスの上限が 40 から 59 文字に増える。従来の長さに固定するには `max_length = 40` を設定する (#83)。

## [0.4.2] - 2026-10-17

### Security
- `zip` crate を v9 に更新 (RUSTSEC-2026-0367 / 0368 / 0369)。v9 で `ZipFile::name()` が `Result` を返すようになったため、自己更新の ZIP 展開で不正なエントリ名を `InvalidData` エラーとして伝播する (#73)。`Cargo.toml` の下限も修正済みの 9.0.4 に引き上げ (#74)。

### Changed
- README の `cargo install` 例を、このリリースの `v0.4.2` に更新。

## [0.4.1] - 2026-10-17

### Added
- `[quota] hide_zero` オプションを追加。`true` に設定すると使用率が 0% のクォータ項目を非表示にする。設定エディタ（`agent-statusline config`）でのトグル編集およびバリデーションにも対応 (#71)。
- Antigravity CLI の Argon クォータ（`gemini-argon-wtu-weekly`）のラベル短縮（`Argon Wk`）と優先表示順序の定義を追加 (#71)。

### Changed
- README の `cargo install` 例を、このリリースの `v0.4.1` に更新。

## [0.4.0] - 2026-10-13

### Added
- **対話的な設定エディタ `agent-statusline config`** を追加。TOML を直接書かずに、プレビューを見ながら、配置・部品・設定を編集して保存できる。設計は [docs/agent-statusline-config-tui-design.md](docs/agent-statusline-config-tui-design.md)、使い方は README の「設定エディタ」を参照。
  - **プレビュー**: 実環境から独立したシナリオ（通常、長いパス・ブランチ、コンテキスト警告・上限間近、クォータ逼迫、レビュー待機・実行中、エージェント待機）で描画する。幅（auto / 80 / 120 / 160）、折り返し / 切り詰めを切り替えられる。プレビューと部品の表示例は、生の ANSI で端末へ直接出し、実際の `render` の出力と同じ色になる。`NO_COLOR` が設定されていても色つきで表示する。
  - **配置**: 部品と固定テキストを、行ごとに選択・移動（Shift+矢印。行の端では隣の行へ）・削除・改行・行の追加ができる。何も表示しない部品は薄く示し、出力されない行にはその旨を添える。
  - **部品一覧**: 10 個の部品、定型の区切り、改行を並べ、各部品の表示例を添える。カーソルを動かすと、配置のカーソルの後ろに**仮に入れた状態**をプレビューに出す（確定するまで設定は変わらない）。`Enter` で後ろに、`i` で前に追加する。
  - **プロパティ**: 選んだ部品の設定キーを、現在の値と並べる（既定値と異なる値には既定値を添える）。真偽値は反転、選択肢は巡回、閾値は ±1（`Shift` で ±10）、記号・スタイル・区切り・`format` はテキスト入力。入力中の内容は、確定前でもプレビューに反映する。閾値の大小や認識できないスタイル名など、新しいエラーになる値は受け付けない。
  - **記号の候補**: 記号（`symbol` など）は、部品ごとの候補・共通の記号・ASCII の代替の一覧から選べる。選んでいる間もプレビューに反映し、表示幅と、端末によって幅が変わりうる文字（異体字セレクタ、ZWJ、East Asian Ambiguous、🐿 のように幅 1 と数える絵文字）の注意を出す。
  - **元に戻す**: `u` で、配置とプロパティの変更を、操作の逆順に戻す。
  - **保存**: `s` で、読み込んだ設定ファイルへ保存する（リンクは解決した実ファイルへ、一時ファイル経由。コメント・未知のキー・`format` の書き方を保つ。保存前の内容は、ローカルのデータディレクトリに 20 世代まで残す）。エラーが残る間は保存しない。未保存の変更があるまま終了するときは確認する。保存しようとしたらファイルが外部で変更・削除・作成されていたときは、上書きせず、再読み込み（今の編集は破棄）か別名で保存を選ぶ。
  - **既知の制限**: Windows では、絵文字（BMP 外の文字）をキー入力・貼り付けで入力できない（依存する crossterm の既知の不具合。crossterm-rs/crossterm の #1072、#848）。記号は候補の一覧から選ぶ。差分の確認、TOML の表示、履歴からの復元、マウス操作は未対応。
- `[directory] symbol`（既定 `"📁 "`）を追加。既定の `directory.format` は `$symbol[$folder]($folder_style) …` になったが、既定の出力は変わらない。絵文字を `format` に直書きしている既存の設定も、そのまま動く。
- `[directory] truncation_mode` を実装。`middle`（既定。中間を省略: `/foo/.../bar`）に加えて、`left`（先頭側を省略: `.../foo/bar`）を選べる。これまでは設定に存在するだけで、値にかかわらず `middle` として描画されていた。認識できない値でも描画は止めず `middle` として扱う。

### Changed
- README の `cargo install` 例を、このリリースの `v0.4.0` に更新。
- 部品（`directory`、`git_branch` など 10 個）の ID・設定キー・`format` で使える変数・描画関数を、部品レジストリ `engine/parts.rs` に集約した。描画を、副作用のある「収集」（Git の読み取り、サマリ JSON の読み込み）と、収集済みの入力だけから合成する純関数 `compose` に分離した。いずれも `render` の出力は変わらない（設定エディタの前提整備）。
- 設定エディタのため、依存に `ratatui`（crossterm を含む）、`unicode-width`、`toml_edit` などを追加した。Windows のリリースバイナリは約 5.0MB から約 5.5MB（約 +10%）になった。`render` の起動時間は、追加前と同等（Windows、交互に測って中央値 約 30.7ms → 31.5ms。ノイズの範囲）。

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
