use crate::app::CameraApp;
use eframe::egui;

impl CameraApp {
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
                        .source_manager
                        .available_sources()
                        .iter()
                        .map(|s| s.name.clone())
                        .collect();

                    // Programソースの選択
                    ui.menu_button("Program Source", |ui| {
                        let selected_idx = self
                            .selected_source_id
                            .and_then(|id| {
                                self.source_manager
                                    .available_sources()
                                    .iter()
                                    .position(|s| s.id == id)
                            })
                            .unwrap_or(0);

                        let mut new_selected = selected_idx;
                        for (i, name) in source_names.iter().enumerate() {
                            ui.radio_value(&mut new_selected, i, name);
                        }

                        if new_selected != selected_idx
                            && new_selected < self.source_manager.available_sources().len()
                        {
                            let new_source = &self.source_manager.available_sources()[new_selected];
                            let new_id = new_source.id;
                            match self.source_manager.open_source(new_id) {
                                Ok(_) => {
                                    self.selected_source_id = Some(new_id);
                                    self.source_errors.remove(&new_id);
                                }
                                Err(e) => {
                                    self.source_errors
                                        .insert(new_id, format!("open failed: {}", e));
                                }
                            }
                        }
                    });

                    // Previewソースの選択
                    ui.menu_button("Preview Source", |ui| {
                        let preview_idx = self
                            .preview_source_id
                            .and_then(|id| {
                                self.source_manager
                                    .available_sources()
                                    .iter()
                                    .position(|s| s.id == id)
                            })
                            .unwrap_or(0);

                        let mut new_preview = preview_idx;
                        for (i, name) in source_names.iter().enumerate() {
                            ui.radio_value(&mut new_preview, i, name);
                        }

                        if new_preview != preview_idx
                            && new_preview < self.source_manager.available_sources().len()
                        {
                            let new_source = &self.source_manager.available_sources()[new_preview];
                            let new_id = new_source.id;
                            match self.source_manager.open_source(new_id) {
                                Ok(_) => {
                                    self.preview_source_id = Some(new_id);
                                    self.source_errors.remove(&new_id);
                                }
                                Err(e) => {
                                    self.source_errors
                                        .insert(new_id, format!("open failed: {}", e));
                                }
                            }
                        }
                    });
                });

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(format!(
                        "📹 Sources: {}",
                        self.source_manager.available_sources().len()
                    ));

                    ui.separator();
                });
            });
        });
    }
}
