use crate::app::OsechiApp;
use eframe::egui;
use tracing::info;

impl OsechiApp {
    pub(crate) fn draw_inputs_window(&mut self, ctx: &egui::Context) {
        let mut show_settings = self.show_input_settings;
        egui::Window::new("Manage Inputs")
            .open(&mut show_settings)
            .resizable(false)
            .collapsible(false)
            .show(ctx, |ui| {
                if ui.button("🔄 Rescan Camera Devices").clicked() {
                    info!("Rescan Camera Devices");
                    self.video_source_manager.web_camera_scan();
                }
                ui.separator();

                let available_sources = self.video_source_manager.web_camera_list();

                for idx in 0..8 {
                    ui.horizontal(|ui| {
                        ui.label(format!("Input {}:", idx + 1));

                        let previous_id = self.inputs[idx].clone();
                        crate::ui::utils::draw_source_combo_box(
                            ui,
                            &format!("input_select_{}", idx),
                            &mut self.inputs[idx],
                            available_sources,
                        );

                        if self.inputs[idx] != previous_id {
                            let old_name = previous_id
                                .as_ref()
                                .map(|id| crate::ui::utils::get_source_name(id, available_sources))
                                .unwrap_or_else(|| "None".to_string());
                            let new_name = self.inputs[idx]
                                .as_ref()
                                .map(|id| crate::ui::utils::get_source_name(id, available_sources))
                                .unwrap_or_else(|| "None".to_string());
                            tracing::info!(
                                "Input {} changed: {} -> {}",
                                idx + 1,
                                old_name,
                                new_name
                            );
                        }
                    });
                }
            });
        self.show_input_settings = show_settings;
    }
}
