use osechi::app::{App, Shell};
use osechi::config::Config;

fn main() -> eframe::Result {
    let _log_guard = osechi::log::init();
    let config = Config::from_args(std::env::args().skip(1));

    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default().with_inner_size(config.window_size),
        ..Default::default()
    };

    eframe::run_native(
        // ウインドウのタイトルにバージョンを入れる
        concat!("Osechi v", env!("CARGO_PKG_VERSION")),
        options,
        Box::new(|cc| Ok(Box::new(Shell::new(App::new(&cc.egui_ctx, config))))),
    )
}
