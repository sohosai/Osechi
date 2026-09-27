//! 録画の設定と状態のウインドウ。スロットごとに録画するか・解像度・ビットレートを選び、REC で一斉に始める。
//! メニューバーには録画中の表示を出す。

use std::path::Path;

use eframe::egui::{self, RichText};

use super::output::format_duration;
use super::theme;
use crate::app::App;
use crate::output::record::{Resolution, SlotSettings, Status};
use crate::switcher::Slot;

/// 保存先の既定値(起動したフォルダからの相対パス)。
const DEFAULT_DIR: &str = "recordings";
/// 映像ビットレートの既定値(Mbps)。
const DEFAULT_BITRATE_MBPS: u32 = 12;

/// 録画設定の入力中の内容。
pub(crate) struct RecordForm {
    dir: String,
    slots: Vec<SlotForm>,
    error: Option<String>,
}

/// 1スロット分の設定。
struct SlotForm {
    slot: Slot,
    /// REC で録画を始める対象か
    armed: bool,
    resolution: Resolution,
    bitrate_mbps: u32,
}

impl SlotForm {
    fn settings(&self) -> SlotSettings {
        SlotSettings {
            resolution: self.resolution,
            bitrate: self.bitrate_mbps * 1_000_000,
        }
    }
}

impl Default for RecordForm {
    /// PGM だけを録画する。IN はカメラの大きさのまま、PVW・PGM は 1080p にする。
    fn default() -> Self {
        let slots = Slot::all()
            .map(|slot| SlotForm {
                slot,
                armed: slot == Slot::Program,
                resolution: match slot {
                    Slot::Input(_) => Resolution::Source,
                    Slot::Preview | Slot::Program => Resolution::Hd1080,
                },
                bitrate_mbps: DEFAULT_BITRATE_MBPS,
            })
            .collect();
        Self {
            dir: DEFAULT_DIR.to_string(),
            slots,
            error: None,
        }
    }
}

/// 開いていれば録画ウインドウを描く。
pub(super) fn show(app: &mut App, ctx: &egui::Context) {
    let mut open = app.ui.record_open;
    egui::Window::new("Recording")
        .collapsible(false)
        .resizable(false)
        .open(&mut open)
        .show(ctx, |ui| panel(app, ui));
    app.ui.record_open = open;
}

fn panel(app: &mut App, ui: &mut egui::Ui) {
    let recording = app.recorder.is_recording();
    ui.horizontal(|ui| {
        ui.label("Folder");
        ui.add_enabled(
            !recording,
            egui::TextEdit::singleline(&mut app.ui.recording.dir).desired_width(280.0),
        );
    });
    ui.add_space(6.0);

    egui::Grid::new("record_slots")
        .num_columns(5)
        .spacing([10.0, 6.0])
        .show(ui, |ui| {
            for header in ["", "Rec", "Resolution", "Bitrate", ""] {
                ui.label(RichText::new(header).small().weak());
            }
            ui.end_row();

            for form in &mut app.ui.recording.slots {
                let active = app.recorder.is_recording_slot(form.slot);
                ui.label(
                    RichText::new(form.slot.to_string())
                        .strong()
                        .color(theme::slot_color(form.slot)),
                );

                // 録画中に印を付け外しすると、そのスロットだけを始める・止める
                if ui.checkbox(&mut form.armed, "").changed() && recording {
                    if form.armed {
                        app.recorder.add(form.slot, form.settings(), &app.taps);
                    } else {
                        app.recorder.remove(form.slot);
                    }
                }

                ui.add_enabled_ui(!active, |ui| {
                    egui::ComboBox::from_id_salt(("record_resolution", form.slot))
                        .selected_text(form.resolution.to_string())
                        .width(80.0)
                        .show_ui(ui, |ui| {
                            for resolution in Resolution::ALL {
                                ui.selectable_value(
                                    &mut form.resolution,
                                    resolution,
                                    resolution.to_string(),
                                );
                            }
                        });
                });
                ui.add_enabled(
                    !active,
                    egui::DragValue::new(&mut form.bitrate_mbps)
                        .range(1..=50)
                        .suffix(" Mbps"),
                );
                match app.recorder.status(form.slot) {
                    Some(status) => status_label(ui, &status),
                    None => {
                        ui.label("");
                    }
                }
                ui.end_row();
            }
        });
    ui.label(
        RichText::new("30fps H.264 / 24bit 48kHz PCM (.mov), master audio in every file")
            .small()
            .weak(),
    );

    if let Some(err) = &app.ui.recording.error {
        ui.add_space(4.0);
        ui.colored_label(theme::ACCENT_PROGRAM, err);
    }

    ui.add_space(6.0);
    ui.horizontal(|ui| {
        if recording {
            if ui.button("■ Stop").clicked() {
                app.recorder.stop();
            }
            if let Some(elapsed) = app.recorder.elapsed() {
                ui.label(
                    RichText::new(format!("● REC {}", format_duration(elapsed)))
                        .color(theme::ACCENT_PROGRAM)
                        .monospace(),
                );
            }
        } else if ui.button("● Rec").clicked() {
            start(app);
        }
    });
}

/// 印の付いたスロットの録画を一斉に始める。
fn start(app: &mut App) {
    let form = &mut app.ui.recording;
    let slots: Vec<(Slot, SlotSettings)> = form
        .slots
        .iter()
        .filter(|form| form.armed)
        .map(|form| (form.slot, form.settings()))
        .collect();
    if slots.is_empty() {
        form.error = Some("Check at least one slot to record".to_string());
        return;
    }
    form.error = app
        .recorder
        .start(Path::new(form.dir.trim()), slots, &app.taps)
        .err()
        .map(|err| format!("{err:#}"));
}

/// 録画中のスロットの状態(書いた量・飛ばしたフレーム・失敗の理由)。
fn status_label(ui: &mut egui::Ui, status: &Status) {
    if let Some(err) = &status.error {
        ui.colored_label(theme::ACCENT_PROGRAM, "Failed")
            .on_hover_text(err);
        return;
    }
    let mut text = format!("● {}", format_size(status.bytes));
    if status.dropped_frames > 0 {
        text.push_str(&format!("  dropped {}", status.dropped_frames));
    }
    ui.label(RichText::new(text).color(theme::ACCENT_PROGRAM).monospace())
        .on_hover_text(status.path.display().to_string());
}

/// メニューバーに出す録画中の表示(録画していなければ何も出さない)。
pub(super) fn indicator(app: &App, ui: &mut egui::Ui) {
    if let Some(elapsed) = app.recorder.elapsed() {
        ui.label(
            RichText::new(format!("● REC {}", format_duration(elapsed)))
                .color(theme::ACCENT_PROGRAM)
                .monospace(),
        );
    }
}

/// `12.3 MB` `1.23 GB` の形にする。
fn format_size(bytes: u64) -> String {
    const MB: f64 = 1_000_000.0;
    let bytes = bytes as f64;
    if bytes < 1000.0 * MB {
        format!("{:.1} MB", bytes / MB)
    } else {
        format!("{:.2} GB", bytes / (1000.0 * MB))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_sizes() {
        assert_eq!(format_size(12_345_678), "12.3 MB");
        assert_eq!(format_size(1_234_567_890), "1.23 GB");
    }

    #[test]
    fn defaults_record_program_only() {
        let form = RecordForm::default();
        let armed: Vec<Slot> = form
            .slots
            .iter()
            .filter(|slot| slot.armed)
            .map(|slot| slot.slot)
            .collect();
        assert_eq!(armed, [Slot::Program]);
        assert_eq!(form.slots.len(), Slot::all().count());
    }
}
