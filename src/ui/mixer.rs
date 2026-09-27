//! 下端のオーディオミキサー。チャンネルごとのストリップ、ドロップ枠、マスターを横に並べる。

use eframe::egui::{self, Sense, Stroke, StrokeKind, Ui, vec2};

use super::widget::{Chip, CloseButton, Fader, MuteButton};
use super::{Drag, theme};
use crate::app::App;
use crate::mixer::Strip;

const STRIP_WIDTH: f32 = 108.0;

pub(super) fn show(app: &mut App, ui: &mut Ui) {
    egui::Panel::bottom("mixer")
        .resizable(false)
        .min_size(220.0)
        .show_inside(ui, |ui| {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.add_space(4.0);
                ui.label(egui::RichText::new("AUDIO MIXER").strong().size(13.0));
            });
            ui.add_space(4.0);
            ui.separator();

            egui::ScrollArea::horizontal().show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.add_space(6.0);
                    let mut removed = None;
                    for channel in &mut app.mixer.channels {
                        let source = app.audio.get(&channel.source);
                        let name = app.audio.name(&channel.source);
                        let accent =
                            source.map_or(theme::CHIP_AUDIO_FG, |s| Chip::audio(&s.kind).accent());
                        if channel_strip(ui, name, accent, &mut channel.strip) {
                            removed = Some(channel.source.clone());
                        }
                    }
                    if let Some(id) = removed {
                        app.mixer.remove(&id);
                    }

                    drop_slot(app, ui);
                    ui.add_space(10.0);
                    ui.separator();
                    ui.add_space(10.0);
                    master_strip(app, ui);
                });
            });
        });
}

/// 1チャンネル分のカード。削除ボタンが押されたら `true` を返す。
fn channel_strip(ui: &mut Ui, name: &str, accent: egui::Color32, strip: &mut Strip) -> bool {
    let mut remove = false;
    card(ui, theme::BORDER, |ui| {
        ui.horizontal(|ui| {
            let (dot, _) = ui.allocate_exact_size(vec2(8.0, 8.0), Sense::hover());
            ui.painter().circle_filled(dot.center(), 3.0, accent);
            ui.add_space(3.0);
            ui.label(egui::RichText::new(name).small().strong());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                remove = ui
                    .add(CloseButton)
                    .on_hover_text("Remove channel")
                    .clicked();
            });
        });
        ui.add_space(6.0);
        strip_controls(ui, strip);
    });
    remove
}

/// マスターのカード。最終ミックスの音量・ミュートと、モニター出力先の選択。
fn master_strip(app: &mut App, ui: &mut Ui) {
    card(ui, theme::ACCENT_SELECT.gamma_multiply(0.6), |ui| {
        ui.label(
            egui::RichText::new("MASTER")
                .small()
                .strong()
                .color(theme::ACCENT_SELECT),
        );
        ui.add_space(4.0);

        let mixer = &app.mixer;
        let current = mixer
            .monitor()
            .and_then(|id| mixer.outputs().iter().find(|(output, _)| output == id))
            .map_or("(none)", |(_, name)| name.as_str());
        let mut choice = None;
        egui::ComboBox::from_id_salt("monitor_output")
            .selected_text(egui::RichText::new(current).small())
            .width(STRIP_WIDTH - 14.0)
            .show_ui(ui, |ui| {
                if ui
                    .selectable_label(mixer.monitor().is_none(), "(none)")
                    .clicked()
                {
                    choice = Some(None);
                }
                for (id, name) in mixer.outputs() {
                    if ui
                        .selectable_label(mixer.monitor() == Some(id), name)
                        .clicked()
                    {
                        choice = Some(Some(id.clone()));
                    }
                }
            })
            .response
            .on_hover_text("Monitor output device");
        if let Some(device) = choice {
            app.mixer.set_monitor(device);
        }
        ui.add_space(6.0);

        strip_controls(ui, &mut app.mixer.master);
    });
}

/// フェーダー(メーター付き)・dB表示・ミュートボタン。チャンネルとマスターで共通。
fn strip_controls(ui: &mut Ui, strip: &mut Strip) {
    ui.vertical_centered(|ui| {
        ui.add(Fader(strip));
    });
    ui.add_space(6.0);
    ui.vertical_centered(|ui| {
        ui.label(
            egui::RichText::new(format!("{:+.1} dB", strip.db()))
                .small()
                .monospace()
                .color(ui.visuals().weak_text_color()),
        );
    });
    ui.add_space(6.0);
    ui.add(MuteButton::new(&mut strip.muted, STRIP_WIDTH - 14.0));
}

/// ストリップを入れる角丸のカード。
fn card(ui: &mut Ui, border: egui::Color32, contents: impl FnOnce(&mut Ui)) {
    egui::Frame::new()
        .fill(theme::BG_PANEL_HEADER)
        .stroke(Stroke::new(1.0_f32, border))
        .corner_radius(6.0)
        .inner_margin(egui::Margin::symmetric(7, 8))
        .show(ui, |ui| {
            ui.vertical(|ui| {
                ui.set_width(STRIP_WIDTH);
                contents(ui);
            });
        });
}

/// 音声ソースをドロップしてミキサーに加える枠。
fn drop_slot(app: &mut App, ui: &mut Ui) {
    let (rect, response) = ui.allocate_exact_size(vec2(STRIP_WIDTH, 238.0), Sense::hover());
    let dragging = egui::DragAndDrop::payload::<Drag>(ui.ctx());
    let accepts =
        response.contains_pointer() && matches!(dragging.as_deref(), Some(Drag::Audio(_)));
    let color = if accepts {
        theme::ACCENT_SELECT
    } else {
        ui.visuals().weak_text_color()
    };
    ui.painter().rect_stroke(
        rect.shrink(1.0),
        5.0,
        Stroke::new(1.5_f32, color),
        StrokeKind::Inside,
    );
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        "Drop audio\nsource here",
        egui::FontId::proportional(11.0),
        color,
    );

    if let Some(payload) = response.dnd_release_payload::<Drag>()
        && let Drag::Audio(id) = &*payload
    {
        app.mixer.add(id.clone());
    }
}
