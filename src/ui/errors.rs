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
                            let name = self
                                .video_source_manager
                                .web_camera_list()
                                .iter()
                                .find(|s| s.id == *id)
                                .map(|s| s.name.clone())
                                .unwrap_or_else(|| id.as_string());
                            ui.colored_label(egui::Color32::RED, format!("{}: {}", name, err));
                        }
                    }
                });
        }
    }
}
