use crate::app::OsechiApp;
use eframe::egui;

impl OsechiApp {
    pub(crate) fn draw_menu(&mut self, ui: &mut egui::Ui) {
        egui::Panel::top("menu_bar").show_inside(ui, |ui| {
            egui::MenuBar::new().ui(ui, |ui| {
                ui.menu_button("File", |ui| {
                    if ui.button("Exit").clicked() {
                        ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                });

                ui.menu_button("Settings", |ui| {
                    ui.checkbox(&mut self.show_labels, "Show Labels");
                });

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(format!("Audio: {}", self.audio_source_manager.list().len()));
                    ui.label(format!("Video: {}", self.video_source_manager.list().len()));
                    ui.separator();
                });
            });
        });
    }
}
