# agent-statusline 設計・機能仕様書

## 1. 背景と課題意識

* **現状の課題**: シェルスクリプト方式では、描画のたびに git や JSON 処理などの外部コマンドを起動するため、環境によって表示遅延やクライアント側のタイムアウトにつながる。
* **解決策**:  
  * 全処理（標準入力 JSON パース、Git 状態検出、時刻・クォータ計算、ローカル連携ファイル出力、ANSI 文字列生成）を **単一の Rust ネイティブバイナリ** に集約。  
  * 描画処理で `git` などの外部 CLI を起動せず、起動コストを抑える。実行時間は環境に依存するため、`render --bench` で計測する。
  * **3 大主要 CLI クライアント（Antigravity CLI, Claude Code, GitHub Copilot CLI）** を第一級市民として標準サポート。  
  * `Mcp-Docker` の設計モデルを踏襲した **クライアント設定自動インストーラー（`install` / `uninstall` / `status`）** を備え、ユーザー環境への安全な導入を自動化する。  
  * `cargo-dist` を用いた自動クロスコンパイルとワンライナーインストーラーにより、複数マシンへの配布・更新を自動化する。  
  * Starship ライクな TOML 設定とエージェント抽象化レイヤーを導入し、パワーユーザー向けの拡張性を確保する。

---

## 2. コア設計方針

1. **描画経路で外部 CLI を起動しない**:
   * Git 操作は `gix`、時刻計算は `chrono`、JSON 操作は `serde_json` で行う。24 時間ごとのバックグラウンド更新確認では同じ実行ファイルを起動する（6.3 節）。
2. **3 大クライアントの統合サポート**:  
   * Antigravity CLI (`agy`)、Claude Code (`claude`)、GitHub Copilot CLI (`copilot`) の入力 JSON スキーマの違いを `Adapter` 層で吸収し、単一バイナリでシームレスに処理。
3. **Mcp-Docker 方式の安全なクライアント設定自動化**:  
   * 各エージェントの設定ファイル（`settings.json` / `config.json` 等）のパスを自動解決し、既存設定やコメントを破壊せずに statusline 登録・解除・診断を行う。
4. **複数 OS のネイティブ実行**:
   * Windows、Linux 向けのネイティブバイナリを配布する。シェル固有のプロンプト幅制御は実装せず、端末向け ANSI 出力を行う。
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
├── adapter/       # agy、claude、copilot の入力変換
├── model/         # 共通状態と Squirrel Notifier スキーマ
├── modules/       # context、directory、git、quota
├── engine/        # TOML 設定、テンプレート、ANSI スタイル
├── installer/     # クライアント設定の install / uninstall / status
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
  │      ├─ gix による Git ブランチ・変更状態の取得
  │      ├─ chrono / 内部算術によるクォータ reset_time 差分計算
  │      └─ terminal_size によるターミナル幅取得
  ├─ 5. Sink 処理 (原子的ファイル書き出し)
  │      └─ OS のローカルデータディレクトリ/SquirrelNotifier/ratelimit-status/<agent>.json
  │         (一時ファイル書き出し + 原子的 rename)
  ├─ 6. Format Engine
  │      └─ Starship 風テンプレートに変数をバインドし、ANSI 装飾を付与
  └─ 7. stdout へ出力
```

更新確認の期限を過ぎている場合、描画処理は同じ実行ファイルの `update --background` を起動して確認を委譲します。通常の描画時間は `render --bench` で利用環境ごとに測定します。

---

## 4. クライアント仕様とインストーラー設計

### 4.1 対象クライアントと設定パス

| クライアント名 | 対象設定ファイル | statusLine 登録設定 |
| :--- | :--- | :--- |
| **`agy`** | `~/.gemini/antigravity-cli/settings.json` | `"statusLine": { "type": "command", "command": "agent-statusline render --agent agy", "enabled": true }` |
| **`claude`** | `~/.claude/settings.json` | `"statusLine": { "type": "command", "command": "agent-statusline render --agent claude" }` |
| **`copilot`** | `~/.copilot/settings.json` | `"statusLine": { "type": "command", "command": "agent-statusline render --agent copilot" }` |

### 4.2 インストーラーコマンド体系

`Mcp-Docker` の設計モデルに準拠：

```bash
# 全クライアントに一括登録
agent-statusline install --all

# 特定クライアントのみ登録
agent-statusline install --agent agy,claude

# 登録解除
agent-statusline uninstall --agent agy

# 現在の登録状態とバイナリ整合性を診断
agent-statusline status
```

---

## 5. Starship 風設定ファイル設計 (`config.toml`)

### 5.1 デフォルト構成イメージ

設定ファイル未指定時は、以下の構成がデフォルトとしてメモリ内で展開されます。

```toml
# ~/.config/agent-statusline/config.toml (または %APPDATA%/agent-statusline/config.toml)

format = """
$directory$git_branch$git_status$sandbox
$model$context$agent_state$plan
$quota
"""

[directory]
format = "📁 [$folder]($folder_style) [$path]($path_style) "
folder_style = "bold cyan"
path_style = "dimmed"
truncation_mode = "middle" # 中間省略 (/foo/.../bar)

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

[integrations.squirrel_notifier]
enabled = true
output_dir = "%LOCALAPPDATA%/SquirrelNotifier/ratelimit-status"
```

---

## 6. 自動配布・リリースパイプライン

### 6.1 現在のビルド・配布構成 (`cargo-dist`)

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

### 6.3 自己更新・自動アップデート機構 (`agent-statusline update`)

GitHub Release からバイナリを取得し、インストール済みバイナリを更新します。公開 Release がない間は、更新確認と更新操作は利用できません。

1. **実行中バイナリの安全な置換 (Windows 対応)**:
   - Windows では実行中の `.exe` ファイルは直接上書き・削除できませんが、**リネーム（移動）は許可**されています。
   - `update` 実行時、現在のバイナリ（`agent-statusline.exe`）を一時ファイル（`agent-statusline.exe.old`）にリネームした上で、GitHub Releases からダウンロードした最新バイナリを `agent-statusline.exe` として配置します。
   - 現在のプロセスは無停止で終了し、**次回エージェントが statusline を呼び出した瞬間から新バージョンが即座に起動**します（`.old` は次回起動時に自動クリーンアップ）。
2. **24時間ごとのバックグラウンドチェック**:
   - `render` 実行時は OS の cache directory 内にある `agent-statusline/update_check.json` のタイムスタンプを確認します。
   - 24 時間以上経過している場合のみ、バックグラウンドで非同期プロセスをスポーンして GitHub Releases API に最新タグを問い合わせます。
   - 更新チェックの通信はバックグラウンド処理で行い、描画処理とは分離します。

---

## 7. 実装状況と初回リリース準備

コア機能、3 クライアント対応、設定自動化、TOML 設定、CI、配布ワークフローは実装済みです。タスク一覧は [tasks.md](../tasks.md) を参照してください。

### 7.1 初回リリース手順

現時点では Git tag と GitHub Release はまだありません。初回リリース時は次の手順を行います。

1. `Cargo.toml` のバージョンと `CHANGELOG.md` のリリース内容・日付を確定する。
2. `Cargo.lock` を更新し、変更を `main` にマージする。
3. `Cargo.toml` と一致する SemVer タグ（例: `v0.1.0`）を push する。
4. `Release` workflow の完了後、GitHub Release に Windows x64、Linux x64 musl、Linux ARM64 musl の ZIP、チェックサム、installer があることを確認する。
5. Windows installer で初回バージョンをインストールした後、`agent-statusline update --force` を実行して、同一バージョンの Release asset を取得・置換できることを確認してから、初回リリースを利用者へ案内する。
