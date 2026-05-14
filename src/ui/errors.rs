use crate::app::CameraApp;
use eframe::egui;

impl CameraApp {
    pub(crate) fn draw_errors_window(&mut self, ctx: &egui::Context) {
        if !self.source_errors.is_empty() {
            egui::Window::new("Source Errors")
                .anchor(egui::Align2::RIGHT_BOTTOM, egui::Vec2::new(-10.0, -10.0))
                .collapsible(true)
                .show(ctx, |ui| {
                    for source in self.source_manager.available_sources() {
                        if let Some(err) = self.source_errors.get(&source.id) {
                            ui.colored_label(
                                egui::Color32::RED,
                                format!("{}: {}", source.name, err),
                            );
                        }
                    }
                });
        }
    }
}
