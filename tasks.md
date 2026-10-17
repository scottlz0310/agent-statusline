# タスク管理 (tasks.md)

## Phase 1: プロジェクト基盤 & コア render PoC
- [x] プロジェクト構想仕様書の更新 ([agent-statusline-spec.md](docs/agent-statusline-spec.md))
- [x] プロジェクト初期ドキュメントの作成 ([README.md](README.md), [tasks.md](tasks.md), [CHANGELOG.md](CHANGELOG.md))
- [x] プロジェクトスキャフォールドの構築 (`Cargo.toml`, `.gitignore`, `renovate.json`, `.github/workflows/ci.yml`, `src/`)
- [x] 内部共通データモデルの実装 (`StatuslineState`, `RatelimitStatus`)
- [x] `StatuslineAdapter` トレイトの定義
- [x] `AntigravityAdapter` (`agy`) の実装
- [x] `ClaudeAdapter` (`claude`) の実装
- [x] `CopilotAdapter` (`copilot`) の実装
- [x] `gix` によるインプロセス Git 状態検出モジュールの実装
- [x] クォータ計算および Squirrel Notifier 原子的書き出しモジュールの実装
- [x] デフォルト 3 行 ANSI レンダリングエンジンの実装
- [x] `render` コマンドの実装と実行レイテンシ検証 (#35 で再計測。既定レイアウトの中央値は Linux 約 2ms、Windows 約 11〜16ms)

## Phase 2: クライアント設定自動化 (`install` / `uninstall` / `status`)
- [x] 各クライアントの設定ファイルパス解決 (`client.rs`: `AgentConfigTarget`)
- [x] 設定ファイルの安全なパッチ機構の実装 (`patcher.rs`: バックアップ、既存設定保持、コメント保持、アトミック書き込み)
- [x] `install` サブコマンドの実装 (`--agent`, `--all`, `--dry-run`)
- [x] `uninstall` サブコマンドの実装 (`--agent`, `--all`, `--dry-run`)
- [x] `status` サブコマンドの実装 (各クライアントの登録状態・整合性検証)
- [x] 一時ディレクトリを用いたインストーラーの単体・結合テスト

## Phase 3: Starship ライクな TOML 設定エンジン (`config.toml`)
- [x] TOML 設定スキーマの実装 (`config.rs`)
- [x] テンプレート変数展開エンジン (`formatter.rs`)
- [x] インラインスタイル構文 (`[text](style)`) の ANSI 変換エンジン (`style.rs`)
- [x] ユーザー設定ファイルの読み込みとゼロコンフィグフォールバック

## Phase 4: CI/CD・自動配布・品質保証
- [x] GitHub Actions ワークフロー (`ci.yml`: fmt, clippy, test, llvm-cov)
- [x] Codecov によるテストカバレッジ計測・レポート設定 (`codecov.yml` 初期配置: Informative モード) (#9)
- [x] エントリポイントコードビハインド行数ラチェット監視の実装 (`scripts/check-code-behind-size.ps1`) (#9)
- [x] Lefthook Git フックの設定 (`lefthook.yml`: fmt, clippy, code-behind-size, test)
- [x] Renovate 設定 (`renovate.json`) の整備
- [x] `cargo-dist` の設定とリリースクロスビルドワークフロー (`release.yml`)
- [x] 自己更新サブコマンド (`update`) & バックグラウンド更新チェックの実装
- [x] 未使用スケルトン (`Commands::Init`, `--shell`, `ShellKind`) の完全削除とコードビハインド行数ラチェット引き締め
- [x] カバレッジ目標閾値（target / threshold）の確定と品質ゲートの有効化 (`codecov.yml`: target 80%, threshold 2%/5%, informational: false) (#9)
- [x] README・設計仕様書の現状同期と初回リリース手順の整備

## 配布方式の移行 (#22)

- [x] 自己更新と Windows / Linux installer の ZIP checksum 検証、旧インストールからの移行手順 (#25)
- [x] 3 ターゲットの ZIP / checksum / installer 添付と公開ゲート (#24)
- [x] 最初の新方式 Release の実環境検証 (#26)

## 不具合修正

- [x] Claude Code のレートリミット出力 ID を Squirrel Notifier の契約（`claude-code`）に合わせる (#28)
- [x] v0.2.0 リリース（#24 完了後）
- [x] v0.3.1 リリース（#35 の Windows 描画時間の改善）
- [x] v0.4.0 リリース（設定エディタ `agent-statusline config`）

## Squirrel Notifier 双方向連携

- [x] レビュー依頼・キュー状態サマリ（`statusline-summary.json`）をカレントリポジトリ分だけ `$review` で表示 (#23)
- [x] v0.3.0 リリース

## レートリミット表示の改善

- [x] Antigravity の Argon クォータ (`gemini-argon-wtu-weekly`) の表示順序とラベルマッピング (`Argon Wk`) を追加 (#71)
- [x] `[quota] hide_zero` 設定の実装（使用率 0% のクォータ項目を非表示化）と設定 TUI・バリデーション連携 (#71)
- [x] v0.4.1 リリース

## 依存関係・セキュリティ

- [x] `zip` v9 への更新（RUSTSEC-2026-0367 / 0368 / 0369 対応）と `ZipFile::name()` の `Result` 対応 (#73, #74)
- [x] v0.4.2 リリース

## 設定 TUI (`agent-statusline config`)

基本設計: [docs/agent-statusline-config-tui-design.md](docs/agent-statusline-config-tui-design.md)

- [x] 基本設計書の作成
- [x] 0-1: 描画を「収集」と「合成（純関数 `compose`）」に分離し、ゴールデンテストで出力一致を担保
- [x] 0-2: 部品レジストリ（`PartSpec` / `PropSpec`）の導入と整合テスト、`[directory] symbol` の追加
- [x] 0-3a: 編集サービス（設定パスの解決、厳密な読み込み `load_config_strict`、配置モデル `Layout`）
- [x] 0-3b: 編集サービス（TOML の部分更新と保存前の検証）
- [x] 0-3c: 編集サービス（リンク経由の保存、外部変更検知、履歴）
- [x] `[directory] truncation_mode`（`left` / `middle`）の実装（描画で未使用だった設定キーの実装。0-4 の前提）
- [x] 0-4a: プレビュー基盤の純ロジック（シナリオ、幅・折り返しの模擬、部品ごとのサンプル描画）
- [x] 0-4b: ratatui の PoC（TUI での ANSI 描画、絵文字の幅、折り返し、幅変更。`tui` feature の要否の計測）
- [x] 1-1: 配置の編集（選択・移動・削除・改行・行の追加・元に戻す、配置領域の表示、プレビューとの連動）
- [x] 1-2: 部品一覧と仮挿入プレビュー（プレビューと部品の表示例は生 ANSI で出し、ratatui 方式は削除）
- [x] 1-3a: 編集中の設定（`Draft`: 文書・設定・配置・検証結果・時系列の元に戻す）
- [x] 1-3b: プロパティ編集の画面（記号・スタイル・閾値・選択肢・真偽値・`format` のテキスト入力、検証結果の表示）
- [x] 1-4a: 保存（`s`、未保存の表示と終了の確認、競合時の再読み込み・別名で保存、`config` の編集化）
- [x] 1-4b: 絵文字の候補選択と幅の表示（Windows では絵文字を入力できないため必須）、README
- [x] 2: 仕上げ（トラッカー #84。全項目を実装する。順序は #84 の「実装順序」）
  - [x] #83 `[directory]` のパス省略を端末の幅に追従させる（`COLUMNS` の利用、下限 40 の撤廃、`max_length` の追加）
  - [x] #82 未認識キーの警告表示（検証結果の一覧 `v`）
  - [x] #81 部品一覧のカテゴリ切替と検索
  - [x] #76 TOML 表示と差分ビュー（競合時の差分の確認を含む）
  - [x] #77 履歴からの復元
  - [x] #80 配置領域の縦・横スクロール
  - [x] #78 スタイル（色・装飾）のトグル選択
  - [x] #79 マウス操作への対応
  - [x] 実機確認（Windows。Windows Terminal・VS Code。結果は `docs/config-tui-verification-windows.md`。IME の表示位置は #94）
  - [x] 実機確認（Ubuntu。Windows で確認できなかった項目。Xvfb 上の `xterm`。結果は `docs/config-tui-verification-ubuntu.md`。GNOME Terminal 固有の見え方は未確認）
  - [x] #94 日本語 IME の変換ウィンドウを入力欄の位置に出す（入力中の端末のカーソルの位置）
