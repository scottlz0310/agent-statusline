//! 設定ファイルの編集サービス（設定 TUI フェーズ 0-3）。画面に依存しない。
//!
//! フェーズ 1 の TUI が呼ぶまで、非テストビルドでは未使用の項目がある。
#![allow(dead_code)]

pub mod layout;
pub mod load;
