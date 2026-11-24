# agent-statusline

Antigravity CLI、Claude Code、GitHub Copilot CLI、Codex CLI 向けの Rust 製ステータスライン CLI。

---

## 主な特徴

- ⚡ **外部 CLI に依存しない描画**:
  - Git 状態・JSON・時刻の処理に `git`、`jq`、`date` などの外部 CLI を使わず、Rust のライブラリで処理。ただし Windows では、システムの Git 設定ファイルの場所を調べるために、24 時間に 1 回だけ `git` を起動する。
  - `render --bench` で実行環境ごとの描画時間を計測可能。Git リポジトリ内での既定レイアウトの実測（中央値）は、Linux で約 2ms、Windows で約 11〜16ms（計測条件は [#35](https://github.com/scottlz0310/agent-statusline/issues/35)）。
- 🤝 **4 大 AI CLI クライアント対応**:
  - **Antigravity CLI (`agy`)**
  - **Claude Code (`claude`)**
  - **GitHub Copilot CLI (`copilot`)**
  - **Codex CLI (`codex`)**: Codex CLI `0.171.0` 以降の実験機能 Custom status line（stdio MCP provider + 構造化 `UiTree` JSON、[openai/codex#17827](https://github.com/openai/codex/issues/17827)）に対応。常駐 stdio MCP サーバー（`agent-statusline mcp codex`）として動作する。
- 🛠️ **設定自動化 (`install` / `uninstall` / `status`)**:
  - `Mcp-Docker` の設計モデルを踏襲。各クライアントの設定ファイル（`settings.json` / Codex の `$CODEX_HOME/config.toml`）への statusline 登録・解除・整合診断をワンコマンドで自動実行。
- 🐿️ **Squirrel Notifier 連携**:
  - Antigravity CLI / Claude Code / GitHub Copilot CLI のレートリミット状態（5時間枠・週次枠・クォータ残量）を共通スキーマ（`schemaVersion: 1`）に集約し、ローカルへ原子的（Atomic rename）に出力（Codex CLI は Squirrel Notifier が `codex app-server` から直接取得するため statusline 側からの書き出しは行わない）。
  - Squirrel Notifier が出力する公開状態（`review-status.json` を優先し、不在時は `statusline-summary.json` へフォールバック）から、カレントリポジトリの PR のうちレビュー実行中（🔍・経過時間）と reviewer 起動待ち（⏳・保留理由 `(CI)` / `(busy#N)` / `(pause N%)` / `(manual)`）、および購読異常・鮮度切れ警告（`⚠auth` / `⚠err` / `⚠stopped` / `⚠stale`）を `$review` に表示（例: `🐿 🔍#31 r2 3m ⏳#32 r1 (CI)`、Codex CLI を含む全クライアント共通）。Codex CLI では PR 番号（`#31` 等）が GitHub PR へのクリック可能リンク（`UiTree` `link` ノード）として描画される。Squirrel Notifier が起動していない場合やリポジトリ外では何も表示しない。
- 🔄 **自動更新 (`agent-statusline update`)**:
  - GitHub Release から更新し、バイナリを置き換える。描画時の更新確認は 24 時間ごとにバックグラウンドで行う。
- 🎨 **Starship ライクな TOML 設定**:
  - `$directory$git_branch...` や `[text](style)` による柔軟な見た目のカスタマイズ（ゼロコンフィグでも洗練された 3 行表示を提供）。
- 🧩 **対話的な設定エディタ (`agent-statusline config`)**:
  - 部品の並べ替え・追加・削除、色・記号・しきい値などの設定を、プレビューを見ながらキーで編集して保存する（[設定エディタ](#設定エディタ-agent-statusline-config)）。

---

## クイックスタート

### GitHub Release からインストール

Windows では、最新の GitHub Release に含まれる PowerShell インストーラーを次のコマンドで実行できます。

```powershell
irm https://github.com/scottlz0310/agent-statusline/releases/latest/download/agent-statusline-installer.ps1 | iex
```

Linux x86_64 / aarch64 では shell installer を使用できます。ZIP と同じ Release の `.zip.sha256` を検証し、`~/.local/bin/agent-statusline` に配置します。`~/.local/bin` が `PATH` にない場合、installer が設定方法を案内します。

```bash
curl -fsSLO https://github.com/scottlz0310/agent-statusline/releases/latest/download/agent-statusline-installer.sh
sh agent-statusline-installer.sh
```

v0.2.0 以降の Release では、このリポジトリ内の installer を正式な asset として添付します。v0.1.0 の installer は cargo-dist の生成物です。

#### Windows Defender に関する注意

Windows の検証環境で上記コマンドを実行した際、Microsoft Defender が `Trojan:Win32/Commando!ml` を検出しました。検出対象として表示されたのは PowerShell のコマンド実行です。実行ファイル自体が検出対象だったとは確認されておらず、誤検知かどうかも判断していません。

同じ環境では、Rust toolchain を使ってソースからインストールした場合、この検出は発生せず、Antigravity CLI、Claude Code、GitHub Copilot CLI の3クライアントで動作を確認しました。この結果は検証した環境でのものです。Cargo の利用には Rust toolchain が必要です。

```bash
cargo install --git https://github.com/scottlz0310/agent-statusline --tag v0.7.0 --locked
```

> [!NOTE]
> 既に GitHub Release 版（Windows: `%LOCALAPPDATA%\Programs\agent-statusline`、Linux: `~/.local/bin`）を導入済みの場合、そのディレクトリが Cargo の `bin`（Windows: `%USERPROFILE%\.cargo\bin`、Linux: `~/.cargo/bin`）より先に `PATH` にあると、`install --agent codex` だけでなく Codex CLI が起動する MCP サーバー（`command = "agent-statusline"`）も旧版を起動して失敗します。旧版と同居させる場合は、登録時・Codex 起動時の両方で新版が先に解決されるよう永続的な `PATH` で Cargo の `bin` を公開版より前に配置する（または後述のアンインストール手順で旧公開版バイナリを削除、もしくは `agent-statusline update` で公開版側を `v0.6.0` 以降へ更新する）ようにし、新しいシェルで次を確認してください。
>
> ```bash
> # 先頭に意図した新版バイナリが解決されることを確認（Windows PowerShell は where.exe agent-statusline）
> which -a agent-statusline
>
> # 解決されたバイナリのバージョンと Codex 対応を確認
> agent-statusline --version
> agent-statusline mcp codex --help
> ```

### リリース公開後のバイナリ更新

```bash
# 最新の GitHub Release へ更新
agent-statusline update
```

更新時は対象 ZIP と同じ Release の `<ZIP名>.sha256` を取得し、ZIP 全体を検証してから現在のバイナリを置き換えます。checksum の欠落・不一致や ZIP 内のバイナリ欠落時は旧バイナリを保持します。

### v0.1.0 から新配布方式へ移行する場合

新方式の Release 公開後、既存の `v0.1.0` から `agent-statusline update` を実行します。旧 updater からの更新は最初の新方式 Release の実環境検証（[#26](https://github.com/scottlz0310/agent-statusline/issues/26)）で確認します。更新後は `agent-statusline --version` と `agent-statusline status` で動作を確認してください。

上書き更新では cargo-dist が作成した receipt や Linux の `~/.config/agent-statusline/env`、profile 内の読み込み行を削除しません。Linux では次を確認し、不要と判断したものだけ手動で整理してください。

```bash
ls -l ~/.config/agent-statusline/agent-statusline-receipt.json ~/.config/agent-statusline/env 2>/dev/null || true
grep -n 'agent-statusline/env\|\.local/bin' ~/.profile ~/.bash_profile ~/.bashrc ~/.zprofile 2>/dev/null || true
```

新 installer は `env` helper や receipt を作らず、profile を変更しません。既存の PATH 行が必要なら残してください。Windows の旧 receipt と PATH も自動削除しません。後述のアンインストール手順で整理できます。

### クライアント設定の自動登録

```bash
# 全クライアント（agy, claude, copilot, codex）に一括登録
agent-statusline install --all

# 特定クライアントのみ登録
agent-statusline install --agent agy,claude
agent-statusline install --agent codex

# 設定状態の診断
agent-statusline status
```

Codex CLI（`0.171.0` 以降）への登録では、`$CODEX_HOME/config.toml`（未設定時は `~/.codex/config.toml`）のコメントや既存設定を保持したまま、`[features] tui_extensions_statusline = true`、`[mcp_servers.agent-statusline]`、`[tui] status_line_provider` の 3 箇所を設定します。同名の別 MCP サーバーが既に登録されている場合は上書きせずエラーで保護します。

## アンインストール

最初に、対応クライアントの設定から statusline 登録を解除します。以前の独自 statusline 設定（Codex の独自 `status_line_provider` を含む）が退避されている場合は復元されます。

```bash
agent-statusline uninstall --all
```

Codex CLI では、`install` 時に退避した元の独自プロバイダー情報（`config.toml.agent-statusline-origin.json`）がアンインストール完了後に自動削除されます。ロールバック用の `.bak` ファイルは確認後に必要に応じて削除してください。

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

foreach ($fileName in 'agent-statusline-receipt.json', 'update_check.json', 'git_system_config.json') {
    $file = Join-Path $appDataDir $fileName
    if (Test-Path -LiteralPath $file) {
        Remove-Item -LiteralPath $file -Force
    }
}

if ((Test-Path -LiteralPath $appDataDir) -and -not (Get-ChildItem -LiteralPath $appDataDir -Force | Select-Object -First 1)) {
    Remove-Item -LiteralPath $appDataDir -Force
}
```

この処理はユーザー環境変数 `Path` からインストール先だけを取り除き、cargo-dist の receipt、更新確認キャッシュ、Git 設定の場所のキャッシュを削除します。反映を確認するには PowerShell を開き直してください。`%APPDATA%\agent-statusline\config.toml` のユーザー設定と、各クライアント設定の `.bak` は確認後に必要に応じて削除してください。

Linux では、インストール先のバイナリ、cargo-dist の install receipt、更新確認キャッシュを削除します。cargo-dist 0.33.0 は flat install の場合に `env` ヘルパーも設定ディレクトリへ配置します。`~/.profile` などのシェル設定ファイルからこのヘルパーを読み込む行（`. <env-path>` 形式）と、インストーラーが追加した `~/.local/bin` の PATH 設定行を削除してください。

```bash
rm -f ~/.local/bin/agent-statusline
rm -f ~/.config/agent-statusline/agent-statusline-receipt.json
rm -f ~/.config/agent-statusline/env
rm -f ~/.cache/agent-statusline/update_check.json
```

上記は XDG の標準保存先を使う場合のパスです。`XDG_CONFIG_HOME` または `XDG_CACHE_HOME` を設定している場合は、それぞれの保存先に読み替えてください。`~/.config/agent-statusline/config.toml` など独自設定は削除されません。

### `cargo install` でインストールした場合

```bash
cargo uninstall agent-statusline
```

### ステータスラインの実行 (エージェントから自動呼出)

```bash
# Antigravity CLI（ワンショット CLI）
agent-statusline render --agent agy

# Claude Code（ワンショット CLI）
agent-statusline render --agent claude

# GitHub Copilot CLI（ワンショット CLI）
agent-statusline render --agent copilot

# Codex CLI（常駐 stdio MCP サーバー）
agent-statusline mcp codex
```

---

## 設定エディタ (`agent-statusline config`)

TOML を直接書かずに、ステータスラインの見た目を、プレビューを見ながら編集して保存する対話的な画面（TUI）です。対話的な端末でなければ、すぐに失敗します。

```bash
# 標準の設定ファイル（なければ、保存すると作成される）を編集する
agent-statusline config

# 設定ファイルを指定する
agent-statusline config --config ./my-statusline.toml

# マウスを使わない（端末のテキスト選択を、そのまま使える）
agent-statusline config --no-mouse
```

- **画面**: 上から、プレビュー（エージェントの状態を想定したサンプルで描画）、配置、部品一覧、プロパティ、状態行（検証の結果）、キーの一覧。`Tab` で、配置 → プロパティ → 部品一覧 → プレビューの順に操作の対象を移す。
- **配置**: `↑↓←→` で選び、`Shift+矢印` で動かし、`d` で削除、`n` で選択位置の前に改行、`a` で行を追加する。 行数が高さを、行の長さが幅を超えるときは、選択中のトークンが見える位置まで縦・横にスクロールし（横は全行で共有。行番号の列は固定）、続きがある方向（`↑ ↓ ‹ ›`）を枠の下に示す。
- **部品一覧**: `↑↓` で部品・区切り・改行を選ぶと、配置のカーソルの後ろに**仮に入れた状態**がプレビューに出る。`Enter` で後ろに、`i` で前に追加する。`←` `→` でカテゴリ（すべて / 作業場所・Git / エージェント / レビュー / レイアウト）を切り替え、`/` で名前を検索して絞り込む（入力のたびに絞り込み、`Enter` で確定、`Esc` で検索を解除）。
- **プロパティ**: 選んだ部品の設定キーを、現在の値と並べる。真偽値は `Enter` / `Space` で反転、選択肢は `Enter` / `←` `→` で切り替え、しきい値は `←` `→` で増減（`Shift` で 10 ずつ）、区切り・`format` などの文字列は `Enter` で入力する。スタイル（`style` `folder_style` など）は `Enter` で**色と装飾の選択画面**が開く（`↑↓` で行を選び、前景・背景の色は `←` `→` で「なし → black → … → gray → なし」と巡回、装飾（bold・dimmed・italic・underline・inverted）は `Space` でオン・オフ。選んでいる間もプレビューに反映され、`Enter` で決定、`Esc` で取り消し、`e` で従来のテキスト入力に切り替える）。画面で触らなかったトークン（認識できないもの、別名の `purple`、複数の前景色など）は、元の表記のまま残る。入力中の内容は、確定前でもプレビューに反映される。しきい値の大小の誤りや、認識できないスタイル名など、**新しいエラーになる値は受け付けられない**（理由が状態行に出る）。
- **記号**: 記号（`symbol` など）は、`Enter` で候補の一覧が開く。部品ごとの候補、共通の記号、ASCII だけの代替から選べる。選んでいる間も、プレビューに反映され、**表示幅**と、端末によって幅が変わりうる文字（異体字セレクタ、East Asian Ambiguous、🐿 のような幅 1 と数える絵文字など）の注意が出る。一覧で `e` を押すと、自由に入力できる。
- **検証結果**: 状態行には先頭の 1 件だけが出る。`v` で、エラーと警告（綴りの誤りなどで認識できないキーを含む）の全件を一覧にする。`↑↓` で選び（長い項目は、一覧の高さを超える分を 1 行ずつスクロールする）、`Esc` / `v` で閉じる。
- **マウス**: 既定で有効です（`--no-mouse` で無効にできます。有効の間、端末のテキスト選択には `Shift` が要ります）。左クリックで、枠へフォーカスを移し、配置のトークン・部品一覧の項目・プロパティの行を選びます。**選択済みの項目をもう一度クリック**すると `Enter` 相当の実行です（部品を配置に追加、真偽値の反転、記号やスタイルの選択画面を開く、など）。ホイールは、指している枠の `↑` `↓` です（プレビューの上では何も起きません）。検証結果・差分・履歴・記号の候補・スタイル編集では、行のクリックで選び（もう一度クリックで決定）、ホイールで動かせます。ドラッグ＆ドロップは提供しません。
- **差分**: `t` で、最後に保存（または読み込み）した内容と今の下書きの差分を、TOML のまま見る（`-` が保存済みにだけある行、`+` が下書きにある行）。`↑↓` `PageUp` `PageDown` `Home` `End` でスクロールし、`f` で変更箇所と全文を切り替え、`Esc` / `t` で閉じる。保存しようとしたら設定ファイルが外部で変更・削除されていたときは、`d` でディスク上の今の内容と下書きの差分を見てから、再読み込み（編集を破棄）か別名で保存かを選べる。
- **元に戻す**: `u` で、配置とプロパティの変更を、操作の逆順に戻す。
- **履歴**: `H` で、保存前の内容の履歴（新しい順。日時と、今の下書きからその世代へ変わる行数）を開く。`↑↓` で選び、`Enter` で今の下書きとの差分（復元の確認）を見て、もう一度 `Enter`（または `y`）で、その世代の内容に下書きを置き換える（保存はしないので、`s` で保存する）。復元の直後なら `u` で復元前の下書きに戻せる。`Esc` で差分から一覧へ、一覧から画面へ戻る。
- **保存**: `s` で、読み込んだ設定ファイルへ保存する。コメントや未知のキーは残り、保存前の内容は履歴（ローカルのデータディレクトリ）に残る。エラーが残っている間は保存できない。未保存の変更があるまま `q` / `Esc` で終了するときは、確認する。保存しようとしたらファイルが外部で変更されていたときは、上書きせず、`r`（再読み込み。今の編集は破棄）か `a`（別名で保存）を選ぶ。

### 既知の制限

- **Windows では、絵文字（BMP 外の文字）を、キー入力・貼り付けで入力できません。** 依存する端末ライブラリ crossterm の既知の不具合（[#1072](https://github.com/crossterm-rs/crossterm/issues/1072)、[#848](https://github.com/crossterm-rs/crossterm/issues/848)）によるものです。記号は、候補の一覧から選んでください（`★` など BMP 内の文字は入力できます）。
- プレビューは、この画面を動かしている端末での表示です。エージェント CLI がステータスラインを表示する領域（幅、フォント、折り返し）とは、異なることがあります。
- TOML の直接編集は未対応です。

---

## Codex CLI 利用時の制限・注意点

- **Codex CLI `0.171.0` 以降が必要（実験機能依存）**: `tui_extensions_statusline` は Codex 側の Experimental 機能（`/experimental` の Custom status line）に依存しています。上流のスキーマやフラグ仕様が変更された場合は追従が必要になることがあります。
- **ユーザー設定必須**: Codex CLI はプロジェクト層の `mcp_servers` 定義をステータスラインプロバイダーとして拒否するため、必ずユーザー設定（`$CODEX_HOME/config.toml`、既定 `~/.codex/config.toml`）へ登録してください（`agent-statusline install --agent codex` が自動設定します）。
- **端末幅の未提供**: Codex のスナップショットには端末幅が含まれないため、`[directory] max_length = 0`（自動）時は 120 列相当を基準にパスを省略します。固定長にしたい場合は `[directory] max_length` で文字数を指定してください。
- **一部スタイルの制約**: Codex `UiTree` に直接対応しない `dimmed`・`gray`・`grey` は `#808080` に近似し、`bg:*`（背景色）と `inverted` は無視されます。フッター高さの上限により、3 行を超える行は末尾 `…` でクリップされます。
- **Windows 常駐中の `agent-statusline update`**: Windows で Codex CLI が常駐 MCP サーバーとして `agent-statusline.exe` を起動している間に `agent-statusline update` を実行した場合、バイナリの置換自体は完了しますが、稼働中のプロセスは旧バージョンのまま動作し続けます。新バージョンを反映するには Codex CLI を再起動（または MCP 再接続）してください。

---

## アーキテクチャ

詳細な仕様および設計思想については [docs/agent-statusline-spec.md](docs/agent-statusline-spec.md) をご覧ください。

リリース手順と現在の配布ターゲットも仕様書に記載しています。

設定を対話的に編集・プレビューする `agent-statusline config`（TUI）の基本設計は [docs/agent-statusline-config-tui-design.md](docs/agent-statusline-config-tui-design.md) にあります（未実装）。

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
cargo clippy --all-targets -- -D warnings
cargo test
pwsh -File scripts/check-code-behind-size.ps1
```

描画時間はエージェントから渡される JSON を stdin に送り、`render --bench` で計測できます。`--bench` の計測値は実行環境や入力によって変わります。

## ライセンス

MIT
