use crate::app::OsechiApp;
use eframe::egui;

impl OsechiApp {
    pub(crate) fn draw_inputs_window(&mut self, ctx: &egui::Context) {
        let mut show_settings = self.show_input_settings;
        egui::Window::new("Manage Inputs")
            .open(&mut show_settings)
            .resizable(false)
            .collapsible(false)
            .show(ctx, |ui| {
                if ui.button("🔄 Rescan Devices").clicked() {
                    self.source_manager.rescan();
                }
                ui.separator();

                let available_sources = self.source_manager.available_sources().to_vec();

                for idx in 0..8 {
                    ui.horizontal(|ui| {
                        ui.label(format!("Input {}:", idx + 1));

                        let current_source_id = self.inputs[idx].clone();

                        let selected_text = if let Some(id) = &current_source_id {
                            available_sources
                                .iter()
                                .find(|s| s.id == *id)
                                .map(|s| s.name.clone())
                                .unwrap_or_else(|| "Unknown / Disconnected".to_string())
                        } else {
                            "None".to_string()
                        };

                        egui::ComboBox::from_id_salt(format!("input_select_{}", idx))
                            .selected_text(selected_text)
                            .show_ui(ui, |ui| {
                                let mut is_none = current_source_id.is_none();
                                if ui.selectable_value(&mut is_none, true, "None").clicked() {
                                    self.inputs[idx] = None;
                                }

                                for source in &available_sources {
                                    let mut is_selected =
                                        current_source_id.as_ref() == Some(&source.id);
                                    if ui
                                        .selectable_value(&mut is_selected, true, &source.name)
                                        .clicked()
                                    {
                                        self.inputs[idx] = Some(source.id.clone());
                                    }
                                }
                            });
                    });
                }
            });
        self.show_input_settings = show_settings;
    }
}
