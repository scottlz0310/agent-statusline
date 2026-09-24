# agent-statusline

AI コーディングエージェント（Antigravity CLI, Claude Code, GitHub Copilot CLI）向けの超高速・ゼロプロセス Rust 製ステータスライン。

---

## 主な特徴

- ⚡ **超高速・ゼロプロセス (Zero Fork/Exec)**:
  - 外部コマンド（`git`, `jq`, `date` 等）を一切呼び出さず、すべて Rust のインプロセス（`gix`, `serde_json`, `chrono`）で完結。
  - 実行レイテンシ **1〜3ms**（従来のシェルスクリプト方式の 2〜16 秒から 1000 倍以上高速化）。タイムアウトによるエラーや自動無効化を完全撲滅。
- 🤝 **3 大 AI CLI クライアント対応**:
  - **Antigravity CLI (`agy`)**
  - **Claude Code (`claude`)**
  - **GitHub Copilot CLI (`copilot`)**
- 🛠️ **設定自動化 (`install` / `uninstall` / `status`)**:
  - `Mcp-Docker` の設計モデルを踏襲。各クライアントの設定ファイル（`settings.json` 等）への statusline コマンド登録・解除をワンコマンドで自動実行。
- 🐿️ **Squirrel Notifier 連携**:
  - 各クライアントのレートリミット状態（5時間枠・週次枠・クォータ残量）を共通スキーマ（`schemaVersion: 1`）に集約し、ローカルへ原子的（Atomic rename）に出力。
- 🎨 **Starship ライクな TOML 設定**:
  - `$directory$git_branch...` や `[text](style)` による柔軟な見た目のカスタマイズ（ゼロコンフィグでも洗練された 3 行表示を提供）。

---

## クイックスタート

### インストール (バイナリ配布)

```powershell
# Windows (PowerShell)
irm https://github.com/scottlz0310/agent-statusline/releases/latest/download/agent-statusline-installer.ps1 | iex
```

### クライアント設定の自動登録

```bash
# 全クライアントに一括登録
agent-statusline install --all

# 特定クライアントのみ登録
agent-statusline install --agent agy,claude

# 設定状態の診断
agent-statusline status
```

### ステータスラインの実行 (エージェントから自動呼出)

```bash
# Antigravity CLI
agent-statusline render --agent agy

# Claude Code
agent-statusline render --agent claude

# GitHub Copilot CLI
agent-statusline render --agent copilot
```

---

## アーキテクチャ

詳細な仕様および設計思想については [docs/agent-statusline-spec.md](docs/agent-statusline-spec.md) をご覧ください。

---

## 開発

```bash
# フォーマットチェック
cargo fmt --check

# リント
cargo clippy -- -D warnings

# テスト実行
cargo test
```

## ライセンス

MIT
