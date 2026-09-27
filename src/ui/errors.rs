//! 開けない・受信に失敗しているソースの一覧(右下に出るウインドウ)。問題が無ければ出さない。

use eframe::egui;

use crate::app::App;

pub(super) fn show(app: &mut App, ui: &mut egui::Ui) {
    let video = app
        .frames
        .errors()
        .map(|(id, err)| (app.video.name(id), err));
    let audio = app
        .chunks
        .errors()
        .map(|(id, err)| (app.audio.name(id), err));
    let errors: Vec<(&str, String)> = video.chain(audio).collect();
    if errors.is_empty() {
        return;
    }

    egui::Window::new("Source Errors")
        .anchor(egui::Align2::RIGHT_BOTTOM, egui::vec2(-10.0, -10.0))
        .collapsible(true)
        .show(ui.ctx(), |ui| {
            for (name, err) in errors {
                ui.colored_label(egui::Color32::RED, format!("{name}: {err}"));
            }
        });
}
