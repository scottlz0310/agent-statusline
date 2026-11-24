# agent-statusline 設計・機能仕様書

## 1. 背景と課題意識

* **現状の課題**: シェルスクリプト方式では、描画のたびに git や JSON 処理などの外部コマンドを起動するため、環境によって表示遅延やクライアント側のタイムアウトにつながる。
* **解決策**:  
  * 全処理（標準入力 JSON パース、Git 状態検出、時刻・クォータ計算、ローカル連携ファイル出力、ANSI 文字列生成）を **単一の Rust ネイティブバイナリ** に集約。  
  * 描画処理で `git` などの外部 CLI を起動せず、起動コストを抑える。実行時間は環境に依存するため、`render --bench` で計測する。
  * **4 大主要 CLI クライアント（Antigravity CLI, Claude Code, GitHub Copilot CLI, Codex CLI）** を第一級市民として標準サポート。
  * `Mcp-Docker` の設計モデルを踏襲した **クライアント設定自動インストーラー（`install` / `uninstall` / `status`）** を備え、ユーザー環境への安全な導入を自動化する。  
* Windows / Linux 向け ZIP と checksum を使い、複数マシンへの配布・更新を自動化する。
  * Starship ライクな TOML 設定とエージェント抽象化レイヤーを導入し、パワーユーザー向けの拡張性を確保する。

---

## 2. コア設計方針

1. **描画経路で外部 CLI を起動しない**:
   * Git 操作は `gix`、時刻計算は `chrono`、JSON 操作は `serde_json` で行う。24 時間ごとのバックグラウンド更新確認では同じ実行ファイルを起動する（6.3 節）。
   * 例外として Windows では、gix がシステムの gitconfig の場所を調べるために `git` を起動する（1 回約 40ms）。求めたパスは OS のキャッシュディレクトリの `agent-statusline/git_system_config.json` に保存し、24 時間再利用する。保存できない場合は描画のたびに `git` が起動するため、保存先とエラー内容を標準エラー出力に警告する。同じ理由で `git` が起動するため、Windows ではシステムの gitattributes を読み込まない（Git for Windows 既定の内容は diff ドライバの指定だけで、変更状態の判定に影響しない）。
2. **4 大クライアントの統合サポート**:
   * Antigravity CLI (`agy`)、Claude Code (`claude`)、GitHub Copilot CLI (`copilot`) のワンショット stdin JSON と、Codex CLI (`codex`) の常駐 stdio MCP スナップショット（`com.openai/codex-statusline-state`）の違いを `Adapter` 層とスタイル中間表現（`StyledSpan`）で吸収し、単一バイナリでシームレスに処理。
3. **Mcp-Docker 方式の安全なクライアント設定自動化**:  
   * 各エージェントの設定ファイル（`settings.json` / Codex の `$CODEX_HOME/config.toml`）のパスを自動解決し、既存設定やコメントを破壊せずに statusline 登録・解除・診断を行う。
4. **複数 OS のネイティブ実行**:
   * Windows、Linux 向けのネイティブバイナリを配布する。ワンショットクライアント向けには端末向け ANSI 出力を行い、Codex CLI 向けには構造化 `UiTree` JSON を出力する。
5. **Starship ライクなメンタルモデル**:  
   * TOML による宣言的レイアウト設定（`format = "$directory$git_branch..."`）とインラインスタイル指定記法（`[text](style)`）を採用し、既存ツールからの学習コストを最小化。  
6. **ゼロコンフィグで動作**:  
   * 設定ファイルが存在しない場合でも、デフォルトで洗練されたマルチライン（3行）表示を提供する。

---

## 3. システムアーキテクチャ

### 3.1 実装済みモジュール構成

実際のソース構成は次のとおりです。テストは各モジュール内に配置しています。

```text
src/
├── main.rs
├── cli.rs
├── adapter/       # agy、claude、copilot、codex の入力変換
├── model/         # 共通状態と Squirrel Notifier スキーマ
├── modules/       # context、directory、git、quota、review、terminal
├── engine/        # TOML 設定、テンプレート、スタイル IR（ANSI / Codex UiTree）
├── mcp/           # Codex CLI 向け常駐 stdio MCP サーバーと非ブロッキングキャッシュ
├── editor/        # 設定 TUI 用の厳密読み込み・部分更新・履歴・保存
├── preview/       # プレビュー基盤（シナリオ・表示幅・折り返し）
├── tui/           # 対話的設定エディタ (`agent-statusline config`)
├── installer/     # クライアント設定の install / uninstall / status（JSONC / Codex TOML）
├── sink/          # Squirrel Notifier JSON の原子的書き出し
└── updater/       # GitHub Release 確認とバイナリ更新
```
### 3.2 処理シーケンス (`render` コマンド実行時)

```text
[Agent (agy / claude / copilot)] 
       │ (JSON via stdin)
       ▼
[agent-statusline render --agent <name>]
  ├─ 1. stdin を一括読み込み (read_to_string)
  ├─ 2. 設定読み込み (config.toml / デフォルトフォールバック)
  ├─ 3. Adapter::parse() で内部共通型 StatuslineState へデシリアライズ
  ├─ 4. Rust ライブラリによる情報取得
  │      ├─ gix による Git ブランチ・変更状態・リモート (`owner/repo`) の取得
  │      ├─ Squirrel Notifier のレビュー・キュー状態サマリの読み込み (`$review` 使用時)
  │      ├─ chrono / 内部算術によるクォータ reset_time 差分計算
  │      └─ terminal_size によるターミナル幅取得（取れないとき〔パイプ実行〕は環境変数 `COLUMNS`、それも無ければ 80）
  ├─ 5. Sink 処理 (原子的ファイル書き出し)
  │      └─ OS のローカルデータディレクトリ/SquirrelNotifier/ratelimit-status/<agentId>.json (`claude` は `claude-code`、他は `--agent` と同じ)
  │         (一時ファイル書き出し + 原子的 rename)
  ├─ 6. Format Engine
  │      └─ Starship 風テンプレートに変数をバインドし、`StyledSpan` IR 経由で ANSI 装飾を付与
  └─ 7. stdout へ出力
```

更新確認の期限を過ぎている場合、描画処理は同じ実行ファイルの `update --background` を起動して確認を委譲します。通常の描画時間は `render --bench` で利用環境ごとに測定します。

### 3.3 処理シーケンス (`mcp codex` コマンド実行時)

Codex CLI（`0.171.0` 以降）の Custom status line は常駐 stdio MCP サーバーとして動作し、成功後約 1 秒間隔で `resources/read` が呼ばれます。1 回でも 1 秒タイムアウトを超過すると Codex 側のポーリングが停止するため、以下のキャッシュと非ブロッキング構成を採用しています。

```text
[Codex CLI (>= 0.171.0)]
       │ (改行区切り JSON-RPC 2.0 over stdio)
       ▼
[agent-statusline mcp codex]
  ├─ 起動時: 24 時間バックグラウンド更新チェックを 1 回起動
  ├─ initialize / ping / tools/list / tools/call / resources/list / resources/templates/list
  │    └─ `_meta["openai/ui"].entrypoints = [{ type: "statusline", resourceUri: "ui://agent-statusline/footer" }]` を広告
  └─ resources/read (`params._meta["com.openai/codex-statusline-state"]`)
       ├─ 1. `ConfigCache`（mtime・サイズ監視、一時読み取り失敗時は直前キャッシュを保持し次回再読込）
       ├─ 2. `CodexAdapter::parse_value()` で `StatuslineState` へ変換（`RatelimitPayload` は `None`）
       ├─ 3. `GitStatusCache`（バックグラウンドワーカー + TTL 3 秒、キャッシュミス時は最大 80ms のみ待機）
       ├─ 4. `ReviewSummaryCache`（mtime・サイズ監視、一時読み取り失敗時は直前サマリを保持）
       ├─ 5. `formatter::compose_lines()`（基準幅 120 列）→ `engine::ui_tree` で `UiTree` JSON 化
       │      └─ 64 KiB JSON / 16 KiB リーフ / 256 ノード / 深さ 16 の上限ガード付き
       └─ 6. `mimeType: "application/json"` の単一テキストリソースとして stdout へ返却
```

---

## 4. クライアント仕様とインストーラー設計

### 4.1 対象クライアントと設定パス

| クライアント名 | 対象設定ファイル | 登録内容 |
| :--- | :--- | :--- |
| **`agy`** | `~/.gemini/antigravity-cli/settings.json` | `"statusLine": { "type": "command", "command": "agent-statusline render --agent agy", "enabled": true }` |
| **`claude`** | `~/.claude/settings.json` | `"statusLine": { "type": "command", "command": "agent-statusline render --agent claude" }` |
| **`copilot`** | `$COPILOT_HOME/settings.json`（既定 `~/.copilot/settings.json`） | `"statusLine": { "type": "command", "command": "agent-statusline render --agent copilot" }` および `"footer": { "showCustom": true }` |
| **`codex`** | `$CODEX_HOME/config.toml`（既定 `~/.codex/config.toml`） | `[features] tui_extensions_statusline = true`、`[mcp_servers.agent-statusline]`（`command = "agent-statusline"`, `args = ["mcp", "codex"]`）、`[tui] status_line_provider = { server = "agent-statusline", tool = "statusline" }` |

### 4.2 インストーラーコマンド体系

`Mcp-Docker` の設計モデルに準拠：

```bash
# 全クライアント（agy, claude, copilot, codex）に一括登録
agent-statusline install --all

# 特定クライアントのみ登録
agent-statusline install --agent agy,claude
agent-statusline install --agent codex

# 登録解除（独自設定が退避されていれば復元）
agent-statusline uninstall --agent codex

# 現在の登録状態と整合性（Codex の 3 設定項目を含む）を診断
agent-statusline status
```

---

## 5. Starship 風設定ファイル設計 (`config.toml`)

### 5.1 デフォルト構成イメージ

設定ファイル未指定時は、以下の構成がデフォルトとしてメモリ内で展開されます。

```toml
# ~/.config/agent-statusline/config.toml (または %APPDATA%/agent-statusline/config.toml)

format = """
$directory$git_branch$git_status$sandbox$review
$model$context$agent_state$plan
$quota
"""

[directory]
symbol = "📁 "
format = "$symbol[$folder]($folder_style) [$path]($path_style) "
folder_style = "bold cyan"
path_style = "dimmed"
truncation_mode = "middle" # "middle": 中間省略 (/foo/.../bar) / "left": 先頭省略 (.../foo/bar)
max_length = 0             # パスの最大文字数。0: 端末の幅の半分（自動） / 10〜1000: その文字数で固定

[git_branch]
symbol = "🌿 "
format = "[$symbol$branch]($style) "
style = "green"

[git_status]
modified = "✗"
format = "[$status]($style) "
style = "red"

[sandbox]
symbol = "🔒 sandbox"
format = "[$symbol]($style) "
style = "yellow"

[model]
symbol = "🤖 "
format = "[$symbol$model]($style) "
style = "magenta"

[context]
symbol = "🧠 "
format = "[$symbol$percentage]($pct_style) [($tokens tok)]($tok_style) "
tok_style = "dimmed"
threshold_warning = 50
threshold_critical = 80
style_normal = "green"
style_warning = "yellow"
style_critical = "red"

[quota]
symbol = "⏳ "
format = "[$symbol$label: $percentage (rst $reset_time)]($style) "
separator = "  |  "
hide_zero = false                    # true にすると使用率 0% のクォータ項目を非表示

[review]
symbol = "🐿 "
format = "[$symbol$items]($style) "
active_format = "🔍#$pr r$round $elapsed"   # $pr $round $agent $elapsed
waiting_format = "⏳#$pr r$round $hold"     # $pr $round $reason $hold
separator = " "
style = "yellow"

[integrations.squirrel_notifier]
enabled = true
output_dir = "%LOCALAPPDATA%/SquirrelNotifier/ratelimit-status"
summary_path = "%LOCALAPPDATA%/SquirrelNotifier/statusline-summary.json"
status_path = "%LOCALAPPDATA%/SquirrelNotifier/review-status.json"
```

### 5.2 レビュー・キュー状態 (`$review`)

Squirrel Notifier が出力する公開状態 `review-status.json`（`schemaVersion: 1`、契約は squirrel-notifier の `docs/review-status-contract.md`）を優先して読み込み、不在・破損・未対応スキーマ時は従来の `statusline-summary.json`（`schemaVersion: 1`）へフォールバックする（`summary_path` のみ明示指定されている場合は既存設定との互換を保ち `summary_path` のみを使用）。カレントリポジトリの PR のうち、reviewer 実行中（`running` / `activeReviews`）と起動待ち（`waiting` / `queue.items`）をこの順で表示する。

* `review-status.json` 読み込み時の追加情報:
  * 実行中 PR の `startedAt` から経過時間（`$elapsed`: 例 `45s`, `3m`, `1h2m`）を算出して表示する（`statusline-summary.json` フォールバック時は空文字となり余白も除去される）。
  * 待機中 PR の `holdReason` / `queuePosition` / `autoPause.usedPercentage` から待機理由バッジ（`$hold`: `(CI)` / `(busy#N)` / `(pause N%)` / `(manual)` 等）を表示する。
  * `subscription.state` が `authRequired` / `error` / `stopped` の場合、または `updatedAt` が 3 分（180 秒）超古い場合（異常終了による鮮度切れ）は、警告（`⚠auth` / `⚠err` / `⚠stopped` / `⚠stale`）を表示する（鮮度切れ時は古い PR 一覧を表示しない）。
  * Codex CLI の `UiTree` 描画では、`$review` 内の PR 番号（`#N`）を GitHub PR（`https://github.com/<owner>/<repo>/pull/<N>`）への `link` ノードとして出力する（ANSI 出力は不変）。

* カレントリポジトリは、gix で解決した fetch 用の既定リモートの URL から `owner/repo` を取り出して特定する。既定リモートは、現在のブランチの upstream、`origin`、唯一のリモートの順で決まる。サマリの `repository` とは大文字小文字を区別せずに比較する。
* サマリの `repository` は GitHub の `owner/repo` なので、ホストが `github.com`（SSH over 443 の `ssh.github.com` を含む）のリモートだけを対象にする。GitLab など別ホストの同名リポジトリは一致させない。`~/.ssh/config` のホスト別名は gix が解決しないため対象外。
* 次の場合は何も表示しない。
  * `[integrations.squirrel_notifier] enabled = false`、`[review] disabled = true`、またはフォーマットに `$review` がない
  * Git リポジトリの外、リモートがない、またはリモートが GitHub ではない
  * サマリがない（Squirrel Notifier 未起動）、JSON が壊れている、`schemaVersion` が 1 ではない
  * 該当する PR がない
* サマリにブランチ情報はないため、表示はリポジトリ単位になる。
* Squirrel Notifier が異常終了するとサマリが残り、次に起動するまで古い状態を表示する。サマリは状態が変わったときにだけ書き出されるため、`updatedAt` の古さでは古い状態かどうかを判定できない。

---

## 6. 自動配布・リリースパイプライン

### 6.1 ビルド・配布ターゲット

* **ビルドターゲット**:  
  * `x86_64-pc-windows-msvc` (Windows / pwsh 用ネイティブ exe)  
  * `x86_64-unknown-linux-musl` (Linux / bash 用静的リンクバイナリ)  
  * `aarch64-unknown-linux-musl` (ARM Linux 用)

### 6.2 インストール経路

* **Windows (PowerShell)**:  
  ```powershell
  irm https://github.com/scottlz0310/agent-statusline/releases/latest/download/agent-statusline-installer.ps1 | iex
  ```
  `%LOCALAPPDATA%\Programs\agent-statusline` に配置され、`PATH` へ自動登録。
* **Linux (shell)**: `agent-statusline-installer.sh` が `~/.local/bin/agent-statusline` に配置する。PATH に含まれない場合は案内し、profile や cargo-dist receipt、`env` helper を作成しない。

両 installer は対象 ZIP と同じ Release の `<ZIP名>.sha256` を照合してからバイナリを配置する。v0.2.0 以降の Release にはリポジトリ内の installer を添付する。

### 6.3 自己更新・自動アップデート機構 (`agent-statusline update`)

GitHub Release からバイナリを取得し、インストール済みバイナリを更新します。公開 Release がない間は、更新確認と更新操作は利用できません。

更新対象は Windows x64、Linux x64 musl、Linux ARM64 musl。未対応 OS/arch は明示的にエラーとする。ZIP と checksum を同じ Release から取得し、ZIP 全体の SHA-256 と checksum に記載された ZIP basename を検証した後に展開・置換する。検証失敗時は旧バイナリを保持する。

1. **実行中バイナリの安全な置換 (Windows 対応)**:
   - Windows では実行中の `.exe` ファイルは直接上書き・削除できませんが、**リネーム（移動）は許可**されています。
   - `update` 実行時、現在のバイナリ（`agent-statusline.exe`）を一時ファイル（`agent-statusline.exe.old`）にリネームした上で、GitHub Releases からダウンロードした最新バイナリを `agent-statusline.exe` として配置します。
   - 現在のプロセスは無停止で終了し、**次回エージェントが statusline を呼び出した瞬間から新バージョンが即座に起動**します（`.old` は次回起動時に自動クリーンアップ）。なお Codex CLI の常駐 MCP サーバーとして稼働中に更新した場合、稼働中のプロセスは旧バイナリのまま動作を継続するため、新バージョンの反映には Codex CLI の再起動（または MCP 再接続）が必要です。
2. **24時間ごとのバックグラウンドチェック**:
   - `render` 実行時は OS の cache directory 内にある `agent-statusline/update_check.json` のタイムスタンプを確認します。
   - 24 時間以上経過している場合のみ、バックグラウンドで非同期プロセスをスポーンして GitHub Releases API に最新タグを問い合わせます。
   - 更新チェックの通信はバックグラウンド処理で行い、描画処理とは分離します。

---

## 7. リリース手順

v0.1.0 は cargo-dist 方式で公開した。v0.2.0 以降は `release-automate` と taiki-e の Rust Actions を組み合わせた `.github/workflows/release.yml` で公開する（設計: [#22](https://github.com/scottlz0310/agent-statusline/issues/22)）。

### 7.1 配布 asset

| asset | 内容 |
| --- | --- |
| `agent-statusline-<target>.zip` | ルートに `agent-statusline[.exe]` を 1 個だけ置いた ZIP（3 ターゲット） |
| `agent-statusline-<target>.zip.sha256` | `<hash> *<ZIP名>` 形式の SHA-256 |
| `agent-statusline-installer.ps1` / `.sh` | `scripts/` 内の installer |

### 7.2 公開の流れ

1. Actions の **Prepare Release** を `target_version` 付きで実行し、`Cargo.toml` / `Cargo.lock` / `CHANGELOG.md` を更新する準備 PR を作る。レビュー後に squash merge する。
2. `main` のリリースコミット（`chore(release): vX.Y.Z`）を受けた **Release** workflow が、同じ run の中でタグを固定して draft Release を作る。タグの push イベントには依存しない。
3. 同じ SHA から 3 ターゲットをビルドし、checkout とタグの SHA が一致することを確認してから、ZIP・checksum・installer を draft に添付する。
4. draft から asset を再取得し、`scripts/verify-release-assets.sh` で asset 名の完全一致、非ゼロサイズ、checksum、ZIP 内の単一バイナリを検証する。Windows x64 / Linux x64 / Linux ARM64 の各 runner で展開したバイナリを実行し、`--version` がタグと一致することも確認する。
5. すべて成功した場合だけ `release-automate` の finalize で公開する。

途中で失敗した場合は draft のまま残る。同じ run を再実行すると、同じタグ/SHA の draft に限り asset を置き換えて検証をやり直す。異なる SHA のタグや公開済み Release は書き換えない。

PR では公開権限を使わず、3 ターゲットのビルドと asset 構造・起動を同じスクリプトで検証する（`release.yml`・`scripts/`・`Cargo.toml`・`Cargo.lock` を変更した場合）。

最初の新方式 Release での既存版からの更新と新規インストールの確認は [#26](https://github.com/scottlz0310/agent-statusline/issues/26) で扱う。実リリースには別途明示的な許可を要する。
