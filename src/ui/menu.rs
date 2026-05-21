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
                    if ui.button("⚙ Manage Inputs").clicked() {
                        self.show_input_settings = !self.show_input_settings;
                    }
                    ui.separator();
                    ui.checkbox(&mut self.show_labels, "Show Labels");
                });

                ui.menu_button("Sources", |ui| {
                    let sources = self.video_source_manager.web_camera_list();
                    crate::ui::utils::draw_source_radio_menu(
                        ui,
                        "Program Source",
                        &mut self.selected_source_id,
                        sources,
                    );
                    crate::ui::utils::draw_source_radio_menu(
                        ui,
                        "Preview Source",
                        &mut self.preview_source_id,
                        sources,
                    );
                });

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(format!(
                        "📹 Sources: {}",
                        self.video_source_manager.web_camera_list().len()
                    ));

                    ui.separator();
                });
            });
        });
    }
}
