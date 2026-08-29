use std::net::Ipv4Addr;

use eframe::egui;

use crate::app::{DragPayload, OsechiApp};
use crate::source::audio::{self, aes67::Aes67SampleFormat};
use crate::source::video;
use crate::ui::theme;

/// 「Add AES67 Source」フォームの入力中の状態。
/// 実機を使わずにフォーム経由で手動設定するため、各フィールドは文字列で保持し
/// 「Add」クリック時にまとめてパース・検証する。
pub struct Aes67FormDraft {
    pub name: String,
    pub multicast_addr: String,
    pub port: String,
    pub payload_type: String,
    pub sample_format: Aes67SampleFormat,
    pub channels: String,
    pub sample_rate: String,
    pub error: Option<String>,
}

impl Default for Aes67FormDraft {
    fn default() -> Self {
        Self {
            name: String::new(),
            multicast_addr: "239.1.1.1".to_string(),
            port: "5004".to_string(),
            payload_type: "97".to_string(),
            sample_format: Aes67SampleFormat::L24,
            channels: "2".to_string(),
            sample_rate: "48000".to_string(),
            error: None,
        }
    }
}

impl OsechiApp {
    /// 映像・音声ソースの一覧を表示するパネル。
    /// 各行をドラッグして Preview/Program/Input やオーディオミキサーへ割り当てる。
    pub(crate) fn draw_sources_dock(&mut self, ui: &mut egui::Ui) {
        egui::Panel::left("sources_dock")
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
                            self.video_source_manager.scan();
                            self.audio_source_manager.input_device_scan();
                        }
                        ui.add_space(2.0);
                    });
                });
                ui.add_space(4.0);
                ui.separator();

                let mut remove_audio_id = None;

                egui::ScrollArea::vertical().show(ui, |ui| {
                    ui.add_space(4.0);
                    section_label(ui, "VIDEO");
                    let video_sources = self.video_source_manager.list().to_vec();
                    for desc in &video_sources {
                        self.draw_video_source_row(ui, desc);
                    }

                    ui.add_space(10.0);
                    section_label(ui, "AUDIO");
                    let audio_sources = self.audio_source_manager.list().to_vec();
                    for desc in &audio_sources {
                        if self.draw_audio_source_row(ui, desc) {
                            remove_audio_id = Some(desc.id.clone());
                        }
                    }

                    ui.add_space(2.0);
                    if ui.small_button("+ Add AES67 Source").clicked() {
                        self.add_aes67_form = Some(Aes67FormDraft::default());
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

                if let Some(id) = remove_audio_id {
                    self.audio_source_manager.remove(&id);
                    self.mixer_channels.retain(|c| c.source_id != id);
                    self.active_audio_sources.remove(&id);
                }
            });

        self.draw_add_aes67_dialog(ui.ctx());
    }

    fn draw_video_source_row(&self, ui: &mut egui::Ui, desc: &video::Descriptor) {
        let (chip_text, chip_bg, chip_fg) = match &desc.kind {
            video::SourceKind::WebCamera { .. } => {
                ("CAM", theme::CHIP_VIDEO_BG, theme::CHIP_VIDEO_FG)
            }
            video::SourceKind::ScreenCapture { .. } => {
                ("SCR", theme::CHIP_VIDEO_BG, theme::CHIP_VIDEO_FG)
            }
        };
        let badge = self.video_badge_for(&desc.id);
        let drag_id = egui::Id::new(("video_source_row", &desc.id));

        source_row(ui, drag_id, DragPayload::Video(desc.id.clone()), |ui| {
            theme::icon_chip(ui, chip_text, chip_bg, chip_fg);
            ui.label(&desc.name);
            if let Some(badge_text) = &badge {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    theme::badge(ui, badge_text, theme::badge_color(badge_text));
                });
            }
        });
    }

    /// 音声ソース1行を描画する。手動追加されたAES67ソースには削除ボタンを
    /// 出し、クリックされたら `true` を返す。自動検出のOS入力デバイスと、
    /// SAPで自動検出されたAES67ソースは、アナウンスの有無で自動的に管理
    /// されるため削除ボタンは出さない。
    fn draw_audio_source_row(&self, ui: &mut egui::Ui, desc: &audio::Descriptor) -> bool {
        let is_aes67 = matches!(desc.kind, audio::SourceKind::Aes67 { .. });
        let is_sap_discovered = self.audio_source_manager.is_sap_discovered(&desc.id);
        let (chip_text, chip_bg, chip_fg) = if is_aes67 {
            ("DANTE", theme::CHIP_DANTE_BG, theme::CHIP_DANTE_FG)
        } else {
            ("MIC", theme::CHIP_AUDIO_BG, theme::CHIP_AUDIO_FG)
        };
        let badge = self.audio_badge_for(&desc.id);
        let drag_id = egui::Id::new(("audio_source_row", &desc.id));
        let mut remove_clicked = false;

        source_row(ui, drag_id, DragPayload::Audio(desc.id.clone()), |ui| {
            let chip = theme::icon_chip(ui, chip_text, chip_bg, chip_fg);
            if is_aes67 {
                chip.on_hover_text(if is_sap_discovered {
                    "Discovered automatically via SAP"
                } else {
                    "Manually added AES67 source"
                });
            }
            ui.label(&desc.name);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if is_aes67
                    && !is_sap_discovered
                    && ui
                        .small_button("x")
                        .on_hover_text("Remove source")
                        .clicked()
                {
                    remove_clicked = true;
                }
                if let Some(badge_text) = &badge {
                    theme::badge(ui, badge_text, theme::badge_color(badge_text));
                }
            });
        });

        remove_clicked
    }

    /// 「Add AES67 Source」フォームが開いていれば描画する。
    fn draw_add_aes67_dialog(&mut self, ctx: &egui::Context) {
        let Some(draft) = self.add_aes67_form.as_mut() else {
            return;
        };
        let mut open = true;
        let mut submit_clicked = false;
        let mut cancel_clicked = false;

        egui::Window::new("Add AES67 Source")
            .collapsible(false)
            .resizable(false)
            .open(&mut open)
            .show(ctx, |ui| {
                egui::Grid::new("aes67_form_grid")
                    .num_columns(2)
                    .spacing([8.0, 6.0])
                    .show(ui, |ui| {
                        ui.label("Name");
                        ui.text_edit_singleline(&mut draft.name);
                        ui.end_row();

                        ui.label("Multicast IP");
                        ui.text_edit_singleline(&mut draft.multicast_addr);
                        ui.end_row();

                        ui.label("Port");
                        ui.text_edit_singleline(&mut draft.port);
                        ui.end_row();

                        ui.label("Payload Type");
                        ui.text_edit_singleline(&mut draft.payload_type);
                        ui.end_row();

                        ui.label("Sample Format");
                        egui::ComboBox::from_id_salt("aes67_sample_format")
                            .selected_text(draft.sample_format.to_string())
                            .show_ui(ui, |ui| {
                                ui.selectable_value(
                                    &mut draft.sample_format,
                                    Aes67SampleFormat::L24,
                                    "L24",
                                );
                                ui.selectable_value(
                                    &mut draft.sample_format,
                                    Aes67SampleFormat::L16,
                                    "L16",
                                );
                            });
                        ui.end_row();

                        ui.label("Channels");
                        ui.text_edit_singleline(&mut draft.channels);
                        ui.end_row();

                        ui.label("Sample Rate");
                        ui.text_edit_singleline(&mut draft.sample_rate);
                        ui.end_row();
                    });

                ui.add_space(4.0);
                if let Some(err) = &draft.error {
                    ui.colored_label(theme::ACCENT_PROGRAM, err);
                    ui.add_space(4.0);
                }

                ui.horizontal(|ui| {
                    if ui.button("Add").clicked() {
                        submit_clicked = true;
                    }
                    if ui.button("Cancel").clicked() {
                        cancel_clicked = true;
                    }
                });
            });

        let submitted_descriptor = if submit_clicked {
            match build_aes67_descriptor(draft) {
                Ok(descriptor) => Some(descriptor),
                Err(message) => {
                    draft.error = Some(message);
                    None
                }
            }
        } else {
            None
        };
        // ここで `draft`(`self.add_aes67_form` の可変借用)の最後の使用が終わる。

        if let Some(descriptor) = submitted_descriptor {
            self.audio_source_manager.add_aes67_flow(descriptor);
            self.add_aes67_form = None;
        } else if cancel_clicked || !open {
            self.add_aes67_form = None;
        }
    }
}

/// フォームの入力文字列をパース・検証し、AES67の [`audio::Descriptor`] を組み立てる。
fn build_aes67_descriptor(draft: &Aes67FormDraft) -> Result<audio::Descriptor, String> {
    let multicast_addr: Ipv4Addr = draft
        .multicast_addr
        .trim()
        .parse()
        .map_err(|_| "Invalid multicast IP address".to_string())?;
    if !multicast_addr.is_multicast() {
        return Err("Address must be a multicast address (224.0.0.0-239.255.255.255)".to_string());
    }

    let port: u16 = draft
        .port
        .trim()
        .parse()
        .map_err(|_| "Invalid port (0-65535)".to_string())?;

    let payload_type: u8 = draft
        .payload_type
        .trim()
        .parse()
        .map_err(|_| "Invalid payload type".to_string())?;
    if payload_type > 127 {
        return Err("Payload type must be 0-127".to_string());
    }

    let channels: u16 = draft
        .channels
        .trim()
        .parse()
        .map_err(|_| "Invalid channel count".to_string())?;
    if channels == 0 {
        return Err("Channel count must be at least 1".to_string());
    }

    let sample_rate: u32 = draft
        .sample_rate
        .trim()
        .parse()
        .map_err(|_| "Invalid sample rate".to_string())?;
    if sample_rate == 0 {
        return Err("Sample rate must be greater than 0".to_string());
    }

    let id_key = format!("{}:{}", multicast_addr, port);
    let name = if draft.name.trim().is_empty() {
        format!("AES67 {}", id_key)
    } else {
        draft.name.trim().to_string()
    };

    Ok(audio::Descriptor {
        id: audio::SourceId::Aes67(id_key),
        name,
        kind: audio::SourceKind::Aes67 {
            config: audio::aes67::Aes67FlowConfig {
                multicast_addr,
                port,
                payload_type,
                sample_format: draft.sample_format,
                channels,
                sample_rate,
            },
        },
    })
}

/// セクション見出し(VIDEO/AUDIOなど)を描画する。
fn section_label(ui: &mut egui::Ui, text: &str) {
    ui.label(egui::RichText::new(text).small().weak());
}

/// ドラッグ可能な1行を描画する。ホバー中は枠線でハイライトする。
fn source_row(
    ui: &mut egui::Ui,
    drag_id: egui::Id,
    payload: DragPayload,
    add_contents: impl FnOnce(&mut egui::Ui),
) {
    let inner = ui.dnd_drag_source(drag_id, payload, |ui| {
        ui.horizontal(|ui| {
            ui.add_space(2.0);
            add_contents(ui);
        });
    });

    if inner.response.hovered() {
        ui.painter().rect_stroke(
            inner.response.rect.expand(2.0),
            3.0,
            egui::Stroke::new(1.0, theme::ACCENT_SELECT),
            egui::StrokeKind::Outside,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_descriptor_from_valid_defaults() {
        let draft = Aes67FormDraft::default();
        let descriptor = build_aes67_descriptor(&draft).expect("defaults should be valid");

        assert_eq!(
            descriptor.id,
            audio::SourceId::Aes67("239.1.1.1:5004".to_string())
        );
        assert_eq!(descriptor.name, "AES67 239.1.1.1:5004");
        let audio::SourceKind::Aes67 { config } = descriptor.kind else {
            panic!("expected Aes67 kind");
        };
        assert_eq!(config.port, 5004);
        assert_eq!(config.payload_type, 97);
        assert_eq!(config.channels, 2);
        assert_eq!(config.sample_rate, 48_000);
    }

    #[test]
    fn uses_custom_name_when_given() {
        let draft = Aes67FormDraft {
            name: "Stage Left".to_string(),
            ..Aes67FormDraft::default()
        };
        let descriptor = build_aes67_descriptor(&draft).expect("should be valid");
        assert_eq!(descriptor.name, "Stage Left");
    }

    #[test]
    fn rejects_non_multicast_address() {
        let draft = Aes67FormDraft {
            multicast_addr: "192.168.1.1".to_string(),
            ..Aes67FormDraft::default()
        };
        assert!(build_aes67_descriptor(&draft).is_err());
    }

    #[test]
    fn rejects_invalid_ip() {
        let draft = Aes67FormDraft {
            multicast_addr: "not-an-ip".to_string(),
            ..Aes67FormDraft::default()
        };
        assert!(build_aes67_descriptor(&draft).is_err());
    }

    #[test]
    fn rejects_payload_type_out_of_range() {
        let draft = Aes67FormDraft {
            payload_type: "128".to_string(),
            ..Aes67FormDraft::default()
        };
        assert!(build_aes67_descriptor(&draft).is_err());
    }

    #[test]
    fn rejects_zero_channels() {
        let draft = Aes67FormDraft {
            channels: "0".to_string(),
            ..Aes67FormDraft::default()
        };
        assert!(build_aes67_descriptor(&draft).is_err());
    }

    #[test]
    fn rejects_zero_sample_rate() {
        let draft = Aes67FormDraft {
            sample_rate: "0".to_string(),
            ..Aes67FormDraft::default()
        };
        assert!(build_aes67_descriptor(&draft).is_err());
    }
}
