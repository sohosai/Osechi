use osechi::app::{INITIAL_HEIGHT, INITIAL_WIDTH, OsechiApp};

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_inner_size([INITIAL_WIDTH as f32, INITIAL_HEIGHT as f32 + 40.0]),
        ..Default::default()
    };

    eframe::run_native(
        // ウインドウにバージョンを入れる
        concat!("Osechi v", env!("CARGO_PKG_VERSION")),
        options,
        Box::new(|cc| Ok(Box::new(OsechiApp::new(&cc.egui_ctx)))),
    )
}
