use osechi::app::{CameraApp, INITIAL_HEIGHT, INITIAL_WIDTH};

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_inner_size([INITIAL_WIDTH as f32, INITIAL_HEIGHT as f32 + 40.0]),
        ..Default::default()
    };

    eframe::run_native(
        "Oseti",
        options,
        Box::new(|cc| Ok(Box::new(CameraApp::new(&cc.egui_ctx)))),
    )
}
