//! 設定ファイルの編集サービス（設定 TUI フェーズ 0-3）。画面に依存しない。
//!
//! フェーズ 1 の TUI が呼ぶまで、非テストビルドでは未使用の項目がある。
#![allow(dead_code)]

pub mod diff;
pub mod draft;
pub mod history;
pub mod layout;
pub mod layout_edit;
pub mod load;
pub mod save;
pub mod style_spec;
pub mod update;
pub mod validate;
