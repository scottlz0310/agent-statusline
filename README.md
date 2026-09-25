# agent-statusline

Antigravity CLI、Claude Code、GitHub Copilot CLI 向けの Rust 製ステータスライン CLI。

---

## 主な特徴

- ⚡ **外部 CLI に依存しない描画**:
  - Git 状態・JSON・時刻の処理に `git`、`jq`、`date` などの外部 CLI を使わず、Rust のライブラリで処理。
  - `render --bench` で実行環境ごとの描画時間を計測可能。開発時の実測は 4.2ms。
- 🤝 **3 大 AI CLI クライアント対応**:
  - **Antigravity CLI (`agy`)**
  - **Claude Code (`claude`)**
  - **GitHub Copilot CLI (`copilot`)**
- 🛠️ **設定自動化 (`install` / `uninstall` / `status`)**:
  - `Mcp-Docker` の設計モデルを踏襲。各クライアントの設定ファイル（`settings.json` 等）への statusline コマンド登録・解除をワンコマンドで自動実行。
- 🐿️ **Squirrel Notifier 連携**:
  - 各クライアントのレートリミット状態（5時間枠・週次枠・クォータ残量）を共通スキーマ（`schemaVersion: 1`）に集約し、ローカルへ原子的（Atomic rename）に出力。
- 🔄 **自動更新 (`agent-statusline update`)**:
  - GitHub Release から更新し、バイナリを置き換える。描画時の更新確認は 24 時間ごとにバックグラウンドで行う。
- 🎨 **Starship ライクな TOML 設定**:
  - `$directory$git_branch...` や `[text](style)` による柔軟な見た目のカスタマイズ（ゼロコンフィグでも洗練された 3 行表示を提供）。

---

## クイックスタート

### GitHub Release からインストール

Windows では、最新の GitHub Release に含まれる PowerShell インストーラーを次のコマンドで実行できます。

```powershell
irm https://github.com/scottlz0310/agent-statusline/releases/latest/download/agent-statusline-installer.ps1 | iex
```

#### Windows Defender に関する注意

Windows の検証環境で上記コマンドを実行した際、Microsoft Defender が `Trojan:Win32/Commando!ml` を検出しました。検出対象として表示されたのは PowerShell のコマンド実行です。実行ファイル自体が検出対象だったとは確認されておらず、誤検知かどうかも判断していません。

同じ環境では、Rust toolchain を使ってソースからインストールした場合、この検出は発生せず、Antigravity CLI、Claude Code、GitHub Copilot CLI の3クライアントで動作を確認しました。この結果は検証した環境でのものです。Cargo の利用には Rust toolchain が必要です。

```bash
cargo install --git https://github.com/scottlz0310/agent-statusline --tag v0.1.0 --locked
```

### リリース公開後のバイナリ更新

```bash
# 最新の GitHub Release へ更新
agent-statusline update
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

## アンインストール

最初に、対応クライアントの設定から statusline 登録を解除します。以前の独自 statusline 設定がバックアップされている場合は復元されます。

```bash
agent-statusline uninstall --all
```

このコマンドはクライアント設定を変更しますが、バイナリは削除しません。バイナリはインストール方法に応じて削除してください。

### GitHub Release からインストールした場合

Windows では、インストール先のディレクトリを削除します。

```powershell
$installDir = Join-Path $env:LOCALAPPDATA 'Programs\agent-statusline'
$appDataDir = Join-Path $env:LOCALAPPDATA 'agent-statusline'

$userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
if ($userPath) {
    $pathEntries = $userPath -split ';' | Where-Object {
        $_.TrimEnd('\') -ine $installDir.TrimEnd('\')
    }
    [Environment]::SetEnvironmentVariable('Path', ($pathEntries -join ';'), 'User')
}

if (Test-Path -LiteralPath $installDir) {
    Remove-Item -LiteralPath $installDir -Recurse -Force
}

foreach ($fileName in 'agent-statusline-receipt.json', 'update_check.json') {
    $file = Join-Path $appDataDir $fileName
    if (Test-Path -LiteralPath $file) {
        Remove-Item -LiteralPath $file -Force
    }
}

if ((Test-Path -LiteralPath $appDataDir) -and -not (Get-ChildItem -LiteralPath $appDataDir -Force | Select-Object -First 1)) {
    Remove-Item -LiteralPath $appDataDir -Force
}
```

この処理はユーザー環境変数 `Path` からインストール先だけを取り除き、cargo-dist の receipt と更新確認キャッシュを削除します。反映を確認するには PowerShell を開き直してください。`%APPDATA%\agent-statusline\config.toml` のユーザー設定と、各クライアント設定の `.bak` は確認後に必要に応じて削除してください。

Linux では、インストール先のバイナリを削除します。インストーラーがシェル設定ファイルに `~/.local/bin` の PATH 設定を追加した場合は、その設定も削除してください。

```bash
rm ~/.local/bin/agent-statusline
```

### `cargo install` でインストールした場合

```bash
cargo uninstall agent-statusline
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

リリース手順と現在の配布ターゲットも仕様書に記載しています。

---

## 開発

### 前提条件と Git フックのセットアップ

本リポジトリではコミット前・プッシュ前の品質検査に [Lefthook](https://github.com/evilmartians/lefthook) を採用しています。初回クローン時にフックを有効化してください。

```bash
# Lefthook CLI のインストール
# Windows (winget)
winget install evilmartians.lefthook
# macOS / Linux (Homebrew)
brew install lefthook
# または Go
go install github.com/evilmartians/lefthook/v2@latest

# Git フックの登録
lefthook install
```

### ローカルでの検証

```bash
# Lefthook によるフック検証
lefthook run pre-commit --all-files
lefthook run pre-push --all-files

# 手動での個別チェック
cargo fmt --check
cargo clippy -- -D warnings
cargo test
pwsh -File scripts/check-code-behind-size.ps1
```

描画時間はエージェントから渡される JSON を stdin に送り、`render --bench` で計測できます。`--bench` の計測値は実行環境や入力によって変わります。

## ライセンス

MIT
