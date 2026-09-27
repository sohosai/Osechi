//! 左端のソース一覧。各行をドラッグしてマルチビューやミキサーへ割り当てる。
//! AES67フローを手動で追加するダイアログもここに置く。

use std::net::Ipv4Addr;

use eframe::egui;

use super::widget::{Badge, Chip};
use super::{Drag, theme};
use crate::app::App;
use crate::source::audio::{self, aes67};
use crate::source::{Origin, Source, video};

pub(super) fn show(app: &mut App, ui: &mut egui::Ui) {
    egui::Panel::left("sources")
        .resizable(false)
        .default_size(230.0)
        .show_inside(ui, |ui| {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.add_space(4.0);
                ui.label(egui::RichText::new("SOURCES").strong().size(13.0));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .small_button("Rescan")
                        .on_hover_text("Re-scan for video and audio devices")
                        .clicked()
                    {
                        app.rescan();
                    }
                    ui.add_space(2.0);
                });
            });
            ui.add_space(4.0);
            ui.separator();

            let mut removed = None;
            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.add_space(4.0);
                section_label(ui, "VIDEO");
                for source in &app.video {
                    video_row(app, ui, source);
                }

                ui.add_space(10.0);
                section_label(ui, "AUDIO");
                for source in &app.audio {
                    if audio_row(app, ui, source) {
                        removed = Some(source.id.clone());
                    }
                }

                ui.add_space(2.0);
                if ui.small_button("+ Add AES67 Source").clicked() {
                    app.ui.aes67_form = Some(Aes67Form::default());
                }
                ui.add_space(4.0);
            });

            ui.separator();
            ui.add_space(4.0);
            ui.label(
                egui::RichText::new("Drag onto Preview / Program / Input / Mixer")
                    .small()
                    .weak(),
            );
            ui.add_space(4.0);

            if let Some(id) = removed {
                app.audio.remove(&id);
                app.mixer.remove(&id);
            }
        });

    aes67_dialog(app, ui.ctx());
}

fn section_label(ui: &mut egui::Ui, text: &str) {
    ui.label(egui::RichText::new(text).small().weak());
}

fn video_row(app: &App, ui: &mut egui::Ui, source: &Source<video::Kind>) {
    let slot = app.switcher.slot_of(&source.id);
    draggable_row(ui, Drag::Video(source.id.clone()), |ui| {
        ui.add(Chip::video(&source.kind));
        ui.label(&source.name);
        if let Some(slot) = slot {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add(Badge::new(slot, theme::slot_color(slot)));
            });
        }
    });
}

/// 音声ソース1行。手動追加したソースには削除ボタンを出し、押されたら `true` を返す。
/// スキャンやSAPで見つけたものは自動で出入りするので削除ボタンは出さない。
fn audio_row(app: &App, ui: &mut egui::Ui, source: &Source<audio::Kind>) -> bool {
    let in_mixer = app.mixer.contains(&source.id);
    let mut remove = false;
    draggable_row(ui, Drag::Audio(source.id.clone()), |ui| {
        let chip = ui.add(Chip::audio(&source.kind));
        if matches!(source.kind, audio::Kind::Aes67(_)) {
            chip.on_hover_text(match source.origin {
                Origin::Discovered => "Discovered automatically via SAP",
                _ => "Manually added AES67 source",
            });
        }
        ui.label(&source.name);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            remove = source.origin == Origin::Manual
                && ui
                    .small_button("x")
                    .on_hover_text("Remove source")
                    .clicked();
            if in_mixer {
                ui.add(Badge::new("MIX", theme::CHIP_AUDIO_FG));
            }
        });
    });
    remove
}

/// ドラッグで `payload` を運べる1行。ホバー中は枠線で強調する。
fn draggable_row(ui: &mut egui::Ui, payload: Drag, contents: impl FnOnce(&mut egui::Ui)) {
    let id = egui::Id::new(("source_row", &payload));
    let inner = ui.dnd_drag_source(id, payload, |ui| {
        ui.horizontal(|ui| {
            ui.add_space(2.0);
            contents(ui);
        });
    });
    if inner.response.hovered() {
        ui.painter().rect_stroke(
            inner.response.rect.expand(2.0),
            3.0,
            egui::Stroke::new(1.0_f32, theme::ACCENT_SELECT),
            egui::StrokeKind::Outside,
        );
    }
}

/// 「Add AES67 Source」ダイアログの入力中の内容。
/// 自由に打てるよう文字列のまま持ち、「Add」を押したときにまとめて検証する。
pub(super) struct Aes67Form {
    name: String,
    addr: String,
    port: String,
    payload_type: String,
    format: aes67::Format,
    channels: String,
    sample_rate: String,
    error: Option<String>,
}

impl Default for Aes67Form {
    fn default() -> Self {
        Self {
            name: String::new(),
            addr: "239.1.1.1".to_string(),
            port: "5004".to_string(),
            payload_type: "97".to_string(),
            format: aes67::Format::L24,
            channels: "2".to_string(),
            sample_rate: "48000".to_string(),
            error: None,
        }
    }
}

impl Aes67Form {
    /// 入力を検証し、手動追加のソースにする。不正なら利用者向けのメッセージを返す。
    fn parse(&self) -> Result<Source<audio::Kind>, String> {
        let addr: Ipv4Addr = self
            .addr
            .trim()
            .parse()
            .map_err(|_| "Invalid multicast IP address")?;
        if !addr.is_multicast() {
            return Err("Address must be a multicast address (224.0.0.0-239.255.255.255)".into());
        }
        let port = self
            .port
            .trim()
            .parse()
            .map_err(|_| "Invalid port (0-65535)")?;
        let payload_type = self
            .payload_type
            .trim()
            .parse()
            .ok()
            .filter(|&pt: &u8| pt <= 127)
            .ok_or("Payload type must be 0-127")?;
        let channels = self
            .channels
            .trim()
            .parse()
            .ok()
            .filter(|&n: &u16| n >= 1)
            .ok_or("Channel count must be at least 1")?;
        let sample_rate = self
            .sample_rate
            .trim()
            .parse()
            .ok()
            .filter(|&rate: &u32| rate > 0)
            .ok_or("Sample rate must be greater than 0")?;

        let config = aes67::Config {
            addr,
            port,
            payload_type,
            format: self.format,
            channels,
            sample_rate,
        };
        Ok(config.into_source(&self.name, Origin::Manual))
    }

    /// フォームを描く。
    fn ui(&mut self, ui: &mut egui::Ui) {
        egui::Grid::new("aes67_form")
            .num_columns(2)
            .spacing([8.0, 6.0])
            .show(ui, |ui| {
                for (label, value) in [
                    ("Name", &mut self.name),
                    ("Multicast IP", &mut self.addr),
                    ("Port", &mut self.port),
                    ("Payload Type", &mut self.payload_type),
                ] {
                    ui.label(label);
                    ui.text_edit_singleline(value);
                    ui.end_row();
                }

                ui.label("Sample Format");
                egui::ComboBox::from_id_salt("aes67_format")
                    .selected_text(self.format.to_string())
                    .show_ui(ui, |ui| {
                        for format in aes67::Format::ALL {
                            ui.selectable_value(&mut self.format, format, format.to_string());
                        }
                    });
                ui.end_row();

                for (label, value) in [
                    ("Channels", &mut self.channels),
                    ("Sample Rate", &mut self.sample_rate),
                ] {
                    ui.label(label);
                    ui.text_edit_singleline(value);
                    ui.end_row();
                }
            });

        if let Some(err) = &self.error {
            ui.add_space(4.0);
            ui.colored_label(theme::ACCENT_PROGRAM, err);
        }
    }
}

/// ダイアログが開いていれば描き、「Add」で検証を通ったフローをソース一覧に加える。
fn aes67_dialog(app: &mut App, ctx: &egui::Context) {
    let Some(form) = &mut app.ui.aes67_form else {
        return;
    };
    let mut open = true;
    let (mut add, mut cancel) = (false, false);
    egui::Window::new("Add AES67 Source")
        .collapsible(false)
        .resizable(false)
        .open(&mut open)
        .show(ctx, |ui| {
            form.ui(ui);
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                add = ui.button("Add").clicked();
                cancel = ui.button("Cancel").clicked();
            });
        });

    if add {
        match form.parse() {
            Ok(source) => {
                app.audio.insert(source);
                app.ui.aes67_form = None;
            }
            Err(err) => form.error = Some(err),
        }
    } else if cancel || !open {
        app.ui.aes67_form = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::SourceId;

    fn form(edit: impl FnOnce(&mut Aes67Form)) -> Aes67Form {
        let mut form = Aes67Form::default();
        edit(&mut form);
        form
    }

    #[test]
    fn defaults_are_valid() {
        let source = Aes67Form::default()
            .parse()
            .expect("defaults should be valid");
        assert_eq!(source.id, SourceId::new("aes67", "239.1.1.1:5004"));
        assert_eq!(source.name, "AES67 239.1.1.1:5004");
        assert_eq!(source.origin, Origin::Manual);
        let audio::Kind::Aes67(config) = source.kind else {
            panic!("expected an AES67 source");
        };
        assert_eq!(config.port, 5004);
        assert_eq!(config.payload_type, 97);
        assert_eq!(config.channels, 2);
        assert_eq!(config.sample_rate, 48_000);
    }

    #[test]
    fn uses_custom_name() {
        let source = form(|f| f.name = "Stage Left".into()).parse().unwrap();
        assert_eq!(source.name, "Stage Left");
    }

    #[test]
    fn rejects_invalid_input() {
        let cases: [fn(&mut Aes67Form); 6] = [
            |f| f.addr = "192.168.1.1".into(),
            |f| f.addr = "not-an-ip".into(),
            |f| f.port = "70000".into(),
            |f| f.payload_type = "128".into(),
            |f| f.channels = "0".into(),
            |f| f.sample_rate = "0".into(),
        ];
        for edit in cases {
            assert!(form(edit).parse().is_err());
        }
    }
}
