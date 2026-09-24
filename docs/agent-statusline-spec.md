# agent-statusline 設計構想仕様書

## 1. 背景と課題意識

* **現状の課題**:  
  * AIコーディングエージェント（Antigravity CLI, Claude Code, GitHub Copilot CLI 等）のステータスライン描画において、シェルスクリプト（特に Windows 環境での `Git Bash` / `sh` 経由）を用いると、1回の描画ごとに多数の外部プロセス（`sh`, `jq`, `git`, `date`, コアユーティリティ等）が直列起動される。  
  * Windows / PowerShell（`pwsh`）環境ではプロセス生成（fork/exec）コストが極めて高く、数十回のプロセス呼び出しが数百ミリ秒〜数秒（高負荷時十数秒）のレイテンシを引き起こし、エージェント側のタイムアウト制限（通常 1〜2 秒程度）を超過して強制終了・自動無効化エラーが頻発する。  
* **解決策**:  
  * 全処理（標準入力 JSON パース、Git 状態検出、時刻・クォータ計算、ローカル連携ファイル出力、ANSI 文字列生成）を **単一の Rust ネイティブバイナリ** に集約。  
  * 外部プロセスの生成回数を「ゼロ」にし、実行レイテンシを **1〜3ms 程度** へ短縮してタイムアウトを完全解消する。  
  * **3 大主要 CLI クライアント（Antigravity CLI, Claude Code, GitHub Copilot CLI）** を第一級市民として標準サポート。  
  * `Mcp-Docker` の設計モデルを踏襲した **クライアント設定自動インストーラー（`install` / `uninstall` / `status`）** を備え、ユーザー環境への安全な導入を自動化する。  
  * `cargo-dist` を用いた自動クロスコンパイルとワンライナーインストーラーにより、複数マシンへの配布・更新を自動化する。  
  * Starship ライクな TOML 設定とエージェント抽象化レイヤーを導入し、パワーユーザー向けの拡張性を確保する。

---

## 2. コア設計方針

1. **ゼロ・プロセス・オーバーヘッド (Zero Fork/Exec)**:  
   * 外部 CLI コマンド（`git.exe`, `jq.exe`, `date.exe` 等）は一切呼ばず、全て Rust クレートのインプロセス実行で完結させる（Git 操作は `gix`、時刻計算は `chrono`、JSON 操作は `serde_json`）。  
2. **3 大クライアントの統合サポート**:  
   * Antigravity CLI (`agy`)、Claude Code (`claude`)、GitHub Copilot CLI (`copilot`) の入力 JSON スキーマの違いを `Adapter` 層で吸収し、単一バイナリでシームレスに処理。
3. **Mcp-Docker 方式の安全なクライアント設定自動化**:  
   * 各エージェントの設定ファイル（`settings.json` / `config.json` 等）のパスを自動解決し、既存設定やコメントを破壊せずに statusline 登録・解除・診断を行う。
4. **pwsh ファースト ＋ bash サポート**:  
   * メイン環境である PowerShell 7（Windows）で最もストレスなく動くよう最適化しつつ、bash（WSL/Linux）向けのエスケープ文字幅対策（`\[...\]` / `\x01...\x02`）も切り替え可能とする。  
5. **Starship ライクなメンタルモデル**:  
   * TOML による宣言的レイアウト設定（`format = "$directory$git_branch..."`）とインラインスタイル指定記法（`[text](style)`）を採用し、既存ツールからの学習コストを最小化。  
6. **ゼロコンフィグで動作**:  
   * 設定ファイルが存在しない場合でも、デフォルトで洗練されたマルチライン（3行）表示を提供する。

---

## 3. システムアーキテクチャ

### 3.1 内部モジュール構成

```text
agent-statusline/
├── Cargo.toml
├── src/
│   ├── main.rs                  # CLI エントリポイント (clap: render, install, uninstall, status)
│   ├── cli.rs                   # コマンドライン引数定義
│   ├── adapter/                 # 各エージェント入力 JSON の内部共通型への変換
│   │   ├── mod.rs               # StatuslineAdapter トレイト
│   │   ├── agy.rs               # Antigravity CLI (agy) 用アダプター
│   │   ├── claude.rs            # Claude Code 用アダプター
│   │   └── copilot.rs           # GitHub Copilot CLI 用アダプター
│   ├── model/                   # 内部正規化データ構造
│   │   ├── state.rs             # ディレクトリ、モデル、コンテキスト、クォータ等
│   │   └── ratelimit.rs         # Squirrel Notifier 向けレートリミット外部通知スキーマ
│   ├── modules/                 # 各セグメントの情報抽出ロジック (インプロセス)
│   │   ├── directory.rs         # パス短縮 (中間省略ロジック)
│   │   ├── git.rs               # gix によるブランチ & dirty 状態検出
│   │   ├── model.rs             # 使用モデル名 & effort
│   │   ├── context.rs           # トークン数 (k短縮) & Ctx 使用率
│   │   ├── quota.rs             # レートリミット残時間・使用率計算
│   │   └── sandbox.rs           # サンドボックス状態表示
│   ├── engine/                  # Starship 風フォーマット展開 & スタイル適用
│   │   ├── config.rs            # TOML 設定パース & デフォルト設定定義
│   │   ├── formatter.rs         # テンプレート変数展開 ($directory 等)
│   │   └── style.rs             # ANSI カラー装飾 & シェルエスケープ
│   ├── installer/               # クライアント設定自動化 (Mcp-Docker モデル)
│   │   ├── mod.rs               # ClientInstaller トレイト
│   │   ├── client.rs            # クライアント識別・設定ファイルパス解決
│   │   └── patcher.rs           # JSON 設定ファイルの安全な更新・バックアップ
│   └── sink/                    # 出力先ハンドリング
│       ├── terminal.rs          # ANSI エスケープによる stdout 出力
│       └── notifier.rs          # Squirrel Notifier 用 ratelimit-status (JSON) の原子的書き出し
└── tests/                       # 統合テスト (実 JSON 入力による回帰テスト)
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
  ├─ 4. インプロセス情報取得 (ゼロ・外部プロセス)
  │      ├─ gix による .git 探索 & dirty 判定 (< 1ms)
  │      ├─ chrono / 内部算術によるクォータ reset_time 差分計算 (0ms)
  │      └─ terminal_size によるターミナル幅取得
  ├─ 5. Sink 処理 (原子的ファイル書き出し)
  │      └─ %LOCALAPPDATA%/SquirrelNotifier/ratelimit-status/<agent>.json
  │         (一時ファイル書き出し + 原子的 rename)
  ├─ 6. Format Engine
  │      └─ Starship 風テンプレートに変数をバインドし、ANSI 装飾を付与
  └─ 7. stdout へフラッシュ (全体で 1〜3ms 以内に終了)
```

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

### 6.1 ビルド・配布構成 (`cargo-dist`)

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

Claude Code と同等のゼロ・ダウンタイム自動更新機構を備えます。

1. **実行中バイナリの安全な置換 (Windows 対応)**:
   - Windows では実行中の `.exe` ファイルは直接上書き・削除できませんが、**リネーム（移動）は許可**されています。
   - `update` 実行時、現在のバイナリ（`agent-statusline.exe`）を一時ファイル（`agent-statusline.exe.old`）にリネームした上で、GitHub Releases からダウンロードした最新バイナリを `agent-statusline.exe` として配置します。
   - 現在のプロセスは無停止で終了し、**次回エージェントが statusline を呼び出した瞬間から新バージョンが即座に起動**します（`.old` は次回起動時に自動クリーンアップ）。
2. **24時間非同期バックグラウンドチェック (Zero-Latency)**:
   - `render` 実行時はキャッシュファイル（`~/.cache/agent-statusline/update_check.json`）のタイムスタンプのみをチェック（0ms）。
   - 24 時間以上経過している場合のみ、バックグラウンドで非同期プロセスをスポーンして GitHub Releases API に最新タグを問い合わせます。
   - `render` の描画速度（1〜3ms）には一切干渉しません。

---

## 7. 開発ロードマップ

- [ ] **Phase 1: コア機能 & 3 クライアント Adapter 実装 (PoC)**
  - [ ] プロジェクト基盤の初期化（`Cargo.toml`, `.gitignore`, `tasks.md`, `CHANGELOG.md`）
  - [ ] 共通モデル（`StatuslineState`, `RatelimitStatus`）と `StatuslineAdapter` トレイト
  - [ ] `agy`, `claude`, `copilot` 用アダプター実装
  - [ ] `gix` による Git ブランチ・dirty 状態の高速インプロセス検出
  - [ ] クォータ残時間計算（内部算術 / `chrono`）
  - [ ] Squirrel Notifier 向け原子的 JSON 出力
  - [ ] デフォルト 3 行 ANSI レンダリング（実行時間 1〜3ms の実証）
- [ ] **Phase 2: クライアント設定自動化（`install` / `uninstall` / `status`）**
  - [ ] 各エージェント設定ファイルパスの自動解決
  - [ ] `settings.json` / `config.json` の安全な更新（バックアップ、既存キー保護）
  - [ ] `agent-statusline install`, `uninstall`, `status` コマンドの実装
- [ ] **Phase 3: Starship ライクな TOML 設定エンジン (`config.toml`)**
  - [ ] TOML パーサーの実装
  - [ ] テンプレート変数展開エンジン（`$directory`, `$quota` 等）
  - [ ] インラインスタイル構文（`[text](style)`）の ANSI 変換器
- [ ] **Phase 4: CI/CD・自動配布・品質保証**
  - [ ] GitHub Actions ワークフロー（`cargo fmt`, `cargo clippy`, `cargo test`）
  - [ ] `cargo-dist` の導入と自動リリースパイプライン
  - [ ] 自己更新サブコマンド (`agent-statusline update`) & バックグラウンド更新チェックの実装
  - [ ] Renovate 連携（`scottlz0310/renovate-config`）