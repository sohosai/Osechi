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
                    self.video_source_manager.web_camera_scan();
                }
                ui.separator();

                let available_sources = self.video_source_manager.web_camera_list();

                for idx in 0..8 {
                    ui.horizontal(|ui| {
                        ui.label(format!("Input {}:", idx + 1));

                        crate::ui::utils::draw_source_combo_box(
                            ui,
                            &format!("input_select_{}", idx),
                            &mut self.inputs[idx],
                            available_sources,
                        );
                    });
                }
            });
        self.show_input_settings = show_settings;
    }
}
