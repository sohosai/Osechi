use osechi::app::{INITIAL_HEIGHT, INITIAL_WIDTH, OsechiApp};
use osechi::dev::DevOptions;

fn main() -> eframe::Result {
    let _log_guard = osechi::init::log();
    let dev_options = DevOptions::from_args();

    let (width, height) = dev_options
        .window_size
        .unwrap_or((INITIAL_WIDTH as f32, INITIAL_HEIGHT as f32 + 40.0));

    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default().with_inner_size([width, height]),
        ..Default::default()
    };

    eframe::run_native(
        // ウインドウにバージョンを入れる
        concat!("Osechi v", env!("CARGO_PKG_VERSION")),
        options,
        Box::new(|cc| Ok(Box::new(OsechiApp::new(&cc.egui_ctx, dev_options)))),
    )
}
