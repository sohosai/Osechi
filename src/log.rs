//! ログ出力の初期化。標準出力と `./logs/` 以下の日次ローテーションファイルの両方へ出す
//! (詳細は `docs/log.md`)。

use tracing_appender::non_blocking::WorkerGuard;
use tracing_appender::rolling::{RollingFileAppender, Rotation};
use tracing_subscriber::{EnvFilter, Registry, fmt, prelude::*};

/// ログ出力を初期化する。返り値を drop するとファイルへの書き出しが止まるので、
/// `main` の終わりまで保持すること。
pub fn init() -> WorkerGuard {
    let file_appender = RollingFileAppender::builder()
        .rotation(Rotation::DAILY)
        .max_log_files(5)
        .filename_prefix("sys_log")
        .filename_suffix("log")
        .build("./logs")
        .expect("Failed to initialize rolling file appender");
    let (file_writer, guard) = tracing_appender::non_blocking(file_appender);

    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info"))
        .add_directive("osechi=trace".parse().expect("valid directive"));

    Registry::default()
        .with(filter)
        .with(fmt::layer().with_writer(std::io::stdout))
        .with(fmt::layer().with_ansi(false).with_writer(file_writer))
        .init();

    guard
}
