use tracing_appender::rolling::{RollingFileAppender, Rotation};
use tracing_subscriber::{EnvFilter, Registry, fmt, prelude::*};

/// ログに関連する設定を初期化する関数
pub fn log() -> tracing_appender::non_blocking::WorkerGuard {
    let file_appender = RollingFileAppender::builder()
        .rotation(Rotation::DAILY)
        .max_log_files(5)
        .filename_prefix("sys_log")
        .filename_suffix("log")
        .build("./logs")
        .expect("Failed to initialize rolling file appender");

    let (non_blocking_file, guard) = tracing_appender::non_blocking(file_appender);

    let env_filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info"))
        .add_directive("osechi=trace".parse().unwrap());

    let stdout_layer = fmt::layer().with_writer(std::io::stdout);

    let file_layer = fmt::layer().with_ansi(false).with_writer(non_blocking_file);

    Registry::default()
        .with(env_filter)
        .with(stdout_layer)
        .with(file_layer)
        .init();

    guard
}
