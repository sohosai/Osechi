//! 上端のメニューバー。

use eframe::egui;

use crate::app::App;

pub(super) fn show(app: &mut App, ui: &mut egui::Ui) {
    egui::Panel::top("menu_bar").show_inside(ui, |ui| {
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button("File", |ui| {
                if ui.button("Exit").clicked() {
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                }
            });
            ui.menu_button("Output", |ui| {
                if ui.button("RTMP...").clicked() {
                    app.ui.output_open = true;
                }
            });
            ui.menu_button("Settings", |ui| {
                ui.checkbox(&mut app.ui.show_labels, "Show Labels");
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(format!("Audio: {}", app.audio.len()));
                ui.label(format!("Video: {}", app.video.len()));
                ui.separator();
                super::output::indicator(app, ui);
            });
        });
    });
}
