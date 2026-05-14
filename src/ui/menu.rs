use crate::app::OsechiApp;
use eframe::egui;

impl OsechiApp {
    pub(crate) fn draw_menu(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("menu_bar").show(ctx, |ui| {
            egui::MenuBar::new().ui(ui, |ui| {
                ui.menu_button("File", |ui| {
                    if ui.button("Exit").clicked() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
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
                    let source_names: Vec<String> = self
                        .video_source_manager
                        .web_camera_list()
                        .iter()
                        .map(|s| s.name())
                        .collect();

                    // Programソースの選択
                    ui.menu_button("Program Source", |ui| {
                        let selected_idx = self.selected_source_id.as_ref().and_then(|id| {
                            self.video_source_manager
                                .web_camera_list()
                                .iter()
                                .position(|s| s.id() == *id)
                        });

                        let mut new_selected = selected_idx;
                        for (i, name) in source_names.iter().enumerate() {
                            ui.radio_value(&mut new_selected, Some(i), name);
                        }

                        if new_selected != selected_idx
                            && let Some(idx) = new_selected
                            && idx < self.video_source_manager.web_camera_list().len()
                        {
                            let new_id = self.video_source_manager.web_camera_list()[idx].id();
                            self.selected_source_id = Some(new_id);
                        }
                    });

                    // Previewソースの選択
                    ui.menu_button("Preview Source", |ui| {
                        let preview_idx = self.preview_source_id.as_ref().and_then(|id| {
                            self.video_source_manager
                                .web_camera_list()
                                .iter()
                                .position(|s| s.id() == *id)
                        });

                        let mut new_preview = preview_idx;
                        for (i, name) in source_names.iter().enumerate() {
                            ui.radio_value(&mut new_preview, Some(i), name);
                        }

                        if new_preview != preview_idx
                            && let Some(idx) = new_preview
                            && idx < self.video_source_manager.web_camera_list().len()
                        {
                            let new_id = self.video_source_manager.web_camera_list()[idx].id();
                            self.preview_source_id = Some(new_id);
                        }
                    });
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
