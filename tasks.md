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
- [x] `render` コマンドの実装と実行レイテンシ検証 (< 5ms 実測 4.2ms)

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
- [ ] カバレッジ目標閾値（target / threshold）の確定と品質ゲートの有効化
- [ ] `cargo-dist` の設定とリリースクロスビルドワークフロー (`release.yml`)
- [ ] 自己更新サブコマンド (`update`) & バックグラウンド更新チェックの実装
- [ ] Renovate 設定 (`renovate.json`) の整備
- [ ] ドキュメントの最終整備とリリース準備
