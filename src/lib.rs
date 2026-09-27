//! Osechi: 軽量な映像スイッチング・伝送ソフトウエア。
//!
//! モジュール構成と、コードを書くときの規約は `docs/architecture.md` を参照。

pub mod api;
pub mod app;
pub mod config;
pub mod error;
pub mod log;
pub mod mixer;
pub mod net;
pub mod output;
pub mod source;
pub mod switcher;
pub mod ui;
