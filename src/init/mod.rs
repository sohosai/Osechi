/// ログに関連する内容の初期化
mod log;

/// フォントに関連する初期化
mod fonts;

pub use fonts::install_cjk_fallback;
pub use log::log;
