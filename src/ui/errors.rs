use crate::app::OsechiApp;
use eframe::egui;

impl OsechiApp {
    pub(crate) fn draw_errors_window(&mut self, ctx: &egui::Context) {
        let has_errors = self.active_sources.values().any(|a| a.last_error.is_some());

        if has_errors {
            egui::Window::new("Source Errors")
                .anchor(egui::Align2::RIGHT_BOTTOM, egui::Vec2::new(-10.0, -10.0))
                .collapsible(true)
                .show(ctx, |ui| {
                    for (id, active) in &self.active_sources {
                        if let Some(err) = &active.last_error {
                            let name = crate::ui::utils::get_source_name(
                                id,
                                self.video_source_manager.web_camera_list(),
                            );
                            ui.colored_label(egui::Color32::RED, format!("{}: {}", name, err));
                        }
                    }
                });
        }
    }
}
