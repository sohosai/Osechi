use eframe::egui::{self, Color32, Rect, Sense, Stroke, StrokeKind, vec2};

use crate::app::{DragPayload, OsechiApp};
use crate::mixer;
use crate::source::audio::{self, SourceId};
use crate::ui::theme;

const CHANNEL_WIDTH: f32 = 108.0;
const TRACK_WIDTH: f32 = 30.0;
const FADER_HEIGHT: f32 = 148.0;
const MUTE_HEIGHT: f32 = 28.0;

impl OsechiApp {
    /// オーディオミキサーのパネル。ミキサーに追加された音声ソースごとに
    /// フェーダー・グラデーションのレベルメーター・ミュートボタンを表示する。
    pub(crate) fn draw_audio_mixer_dock(&mut self, ui: &mut egui::Ui) {
        egui::Panel::bottom("audio_mixer_dock")
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
                        let ids: Vec<SourceId> = self
                            .mixer_channels
                            .iter()
                            .map(|c| c.source_id.clone())
                            .collect();

                        let mut remove_id = None;
                        for id in &ids {
                            if self.draw_mixer_strip(ui, id) {
                                remove_id = Some(id.clone());
                            }
                        }
                        if let Some(id) = remove_id {
                            self.remove_mixer_channel(&id);
                        }

                        self.draw_mixer_add_slot(ui);
                        ui.add_space(10.0);
                        ui.separator();
                        ui.add_space(10.0);
                        self.draw_master_strip(ui);
                    });
                });
            });
    }

    /// マスターストリップ。全チャンネルを合成した最終ミックスの音量調整・
    /// ミュートと、モニター出力デバイスの選択を行う。
    fn draw_master_strip(&mut self, ui: &mut egui::Ui) {
        egui::Frame::new()
            .fill(theme::BG_PANEL_HEADER)
            .stroke(Stroke::new(1.0, theme::ACCENT_SELECT.gamma_multiply(0.6)))
            .corner_radius(6.0)
            .inner_margin(egui::Margin::symmetric(7, 8))
            .show(ui, |ui| {
                ui.vertical(|ui| {
                    ui.set_width(CHANNEL_WIDTH);

                    ui.label(
                        egui::RichText::new("MASTER")
                            .small()
                            .strong()
                            .color(theme::ACCENT_SELECT),
                    );
                    ui.add_space(4.0);

                    let current_name = self
                        .monitor_device_id
                        .as_ref()
                        .and_then(|id| self.output_devices.iter().find(|(did, _)| did == id))
                        .map(|(_, name)| name.clone())
                        .unwrap_or_else(|| "(none)".to_string());

                    egui::ComboBox::from_id_salt("monitor_output_device")
                        .selected_text(egui::RichText::new(current_name).small())
                        .width(CHANNEL_WIDTH - 14.0)
                        .show_ui(ui, |ui| {
                            if ui
                                .selectable_label(self.monitor_device_id.is_none(), "(none)")
                                .clicked()
                            {
                                self.set_monitor_device(None);
                            }
                            let devices = self.output_devices.clone();
                            for (id, name) in &devices {
                                let selected = self.monitor_device_id.as_ref() == Some(id);
                                if ui.selectable_label(selected, name).clicked() {
                                    self.set_monitor_device(Some(id.clone()));
                                }
                            }
                        })
                        .response
                        .on_hover_text("Monitor output device");
                    ui.add_space(6.0);

                    ui.vertical_centered(|ui| {
                        fader_track(
                            ui,
                            vec2(TRACK_WIDTH, FADER_HEIGHT),
                            self.master_level,
                            &mut self.master_gain,
                            self.master_muted,
                        );
                    });
                    ui.add_space(6.0);

                    ui.vertical_centered(|ui| {
                        ui.label(
                            egui::RichText::new(format!(
                                "{:+.1} dB",
                                mixer::gain_to_db(self.master_gain)
                            ))
                            .small()
                            .monospace()
                            .color(ui.visuals().weak_text_color()),
                        );
                    });
                    ui.add_space(6.0);

                    if mute_button(ui, CHANNEL_WIDTH - 14.0, self.master_muted).clicked() {
                        self.master_muted = !self.master_muted;
                    }
                });
            });
    }

    /// 1チャンネル分のストリップをカード状の枠に入れて描画する。ミュート状態は
    /// メーターにも反映される(プリフェーダーの信号自体は流れているが出力は無い、
    /// という見た目にするため)。削除ボタンが押されたら `true` を返す。
    fn draw_mixer_strip(&mut self, ui: &mut egui::Ui, id: &SourceId) -> bool {
        let name = crate::ui::utils::audio_source_name(id, self.audio_source_manager.list());
        let is_aes67 = matches!(
            self.audio_source_manager
                .list()
                .iter()
                .find(|d| &d.id == id)
                .map(|d| &d.kind),
            Some(audio::SourceKind::Aes67 { .. })
        );
        let accent = if is_aes67 {
            theme::CHIP_DANTE_FG
        } else {
            theme::CHIP_AUDIO_FG
        };
        let mut remove = false;

        egui::Frame::new()
            .fill(theme::BG_PANEL_HEADER)
            .stroke(Stroke::new(1.0, theme::BORDER))
            .corner_radius(6.0)
            .inner_margin(egui::Margin::symmetric(7, 8))
            .show(ui, |ui| {
                ui.vertical(|ui| {
                    ui.set_width(CHANNEL_WIDTH);

                    ui.horizontal(|ui| {
                        let (dot_rect, _) = ui.allocate_exact_size(vec2(8.0, 8.0), Sense::hover());
                        ui.painter().circle_filled(dot_rect.center(), 3.0, accent);
                        ui.add_space(3.0);
                        ui.label(egui::RichText::new(&name).small().strong());
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if close_icon_button(ui)
                                .on_hover_text("Remove channel")
                                .clicked()
                            {
                                remove = true;
                            }
                        });
                    });
                    ui.add_space(6.0);

                    let Some(channel) = self.mixer_channels.iter_mut().find(|c| &c.source_id == id)
                    else {
                        return;
                    };

                    ui.vertical_centered(|ui| {
                        fader_track(
                            ui,
                            vec2(TRACK_WIDTH, FADER_HEIGHT),
                            channel.level,
                            &mut channel.gain,
                            channel.muted,
                        );
                    });
                    ui.add_space(6.0);

                    ui.vertical_centered(|ui| {
                        ui.label(
                            egui::RichText::new(format!(
                                "{:+.1} dB",
                                mixer::gain_to_db(channel.gain)
                            ))
                            .small()
                            .monospace()
                            .color(ui.visuals().weak_text_color()),
                        );
                    });
                    ui.add_space(6.0);

                    if mute_button(ui, CHANNEL_WIDTH - 14.0, channel.muted).clicked() {
                        channel.muted = !channel.muted;
                    }
                });
            });

        remove
    }

    /// 音声ソースをドラッグしてミキサーに追加するためのドロップ枠。
    fn draw_mixer_add_slot(&mut self, ui: &mut egui::Ui) {
        let (rect, response) =
            ui.allocate_exact_size(vec2(CHANNEL_WIDTH, FADER_HEIGHT + 90.0), Sense::hover());

        let dragging = egui::DragAndDrop::payload::<DragPayload>(ui.ctx());
        let accept = response.contains_pointer()
            && matches!(dragging.as_deref(), Some(DragPayload::Audio(_)));

        let stroke_color = if accept {
            theme::ACCENT_SELECT
        } else {
            ui.visuals().weak_text_color()
        };
        ui.painter().rect_stroke(
            rect.shrink(1.0),
            5.0,
            Stroke::new(1.5, stroke_color),
            StrokeKind::Inside,
        );
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            "Drop audio\nsource here",
            egui::FontId::proportional(11.0),
            stroke_color,
        );

        if let Some(payload) = response.dnd_release_payload::<DragPayload>()
            && let DragPayload::Audio(source_id) = &*payload
        {
            self.add_mixer_channel(source_id.clone());
        }
    }
}

/// 円形のホバー反応する削除アイコンボタン(バツ印を自前描画する)。
fn close_icon_button(ui: &mut egui::Ui) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::click());
    let hovered = response.hovered();

    if hovered {
        ui.painter().circle_filled(
            rect.center(),
            9.0,
            theme::ACCENT_PROGRAM.gamma_multiply(0.22),
        );
    }
    let color = if hovered {
        theme::ACCENT_PROGRAM
    } else {
        ui.visuals().weak_text_color()
    };
    let pad = 4.5;
    let stroke = Stroke::new(1.3, color);
    ui.painter().line_segment(
        [rect.min + vec2(pad, pad), rect.max - vec2(pad, pad)],
        stroke,
    );
    ui.painter().line_segment(
        [
            egui::pos2(rect.min.x + pad, rect.max.y - pad),
            egui::pos2(rect.max.x - pad, rect.min.y + pad),
        ],
        stroke,
    );

    response
}

/// LEDインジケータ付きのミュートボタン。ミュート中は塗りつぶし+グロー、
/// 通常時はゴースト調にすることで、一目で状態が分かるようにしている。
fn mute_button(ui: &mut egui::Ui, width: f32, muted: bool) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(vec2(width, MUTE_HEIGHT), Sense::click());
    let hovered = response.hovered();
    let painter = ui.painter();

    let (bg, border, text_color, led_color) = if muted {
        (
            theme::ACCENT_PROGRAM,
            theme::ACCENT_PROGRAM,
            Color32::WHITE,
            Color32::WHITE,
        )
    } else if hovered {
        (
            theme::BG_ROW_HOVER,
            theme::ACCENT_PROGRAM.gamma_multiply(0.6),
            ui.visuals().text_color(),
            theme::ACCENT_PROGRAM.gamma_multiply(0.7),
        )
    } else {
        (
            theme::BG_PANEL_HEADER,
            theme::BORDER,
            ui.visuals().weak_text_color(),
            Color32::from_rgb(0x4a, 0x2e, 0x2e),
        )
    };

    if muted {
        painter.rect_stroke(
            rect.expand(2.0),
            7.0,
            Stroke::new(1.0, theme::ACCENT_PROGRAM.gamma_multiply(0.35)),
            StrokeKind::Outside,
        );
    }
    painter.rect_filled(rect, 5.0, bg);
    painter.rect_stroke(rect, 5.0, Stroke::new(1.0, border), StrokeKind::Inside);

    let led_center = rect.left_center() + vec2(15.0, 0.0);
    painter.circle_filled(led_center, 4.0, led_color);
    if muted {
        painter.circle_stroke(
            led_center,
            6.5,
            Stroke::new(1.0, Color32::from_rgba_unmultiplied(255, 255, 255, 70)),
        );
    }

    painter.text(
        led_center + vec2(10.0, 0.0),
        egui::Align2::LEFT_CENTER,
        "MUTE",
        egui::FontId::new(12.0, egui::FontFamily::Proportional),
        text_color,
    );

    response.on_hover_text(if muted { "Unmute" } else { "Mute" })
}

/// グラデーションのレベルメーターとフェーダーを一体化したトラックを描画する。
/// クリック・ドラッグでゲイン(0.0-1.0)を直接操作できる。
fn fader_track(ui: &mut egui::Ui, size: egui::Vec2, level: f32, gain: &mut f32, muted: bool) {
    let (rect, response) = ui.allocate_exact_size(size, Sense::click_and_drag());

    if (response.dragged() || response.clicked())
        && let Some(pos) = response.interact_pointer_pos()
    {
        let ratio = 1.0 - ((pos.y - rect.top()) / rect.height()).clamp(0.0, 1.0);
        *gain = ratio;
    }

    let displayed_level = if muted { 0.0 } else { level };
    paint_fader_track(ui.painter(), rect, displayed_level, *gain, muted);
}

fn paint_fader_track(painter: &egui::Painter, rect: Rect, level: f32, gain: f32, muted: bool) {
    const TRACK_BG: Color32 = Color32::from_rgb(0x0a, 0x0a, 0x0c);
    const BANDS: usize = 32;

    painter.rect_filled(rect, 5.0, TRACK_BG);

    let level = level.clamp(0.0, 1.0);
    let band_height = rect.height() / BANDS as f32;
    let lit_bands = (level * BANDS as f32).round() as usize;

    for i in 0..lit_bands {
        let t = i as f32 / (BANDS - 1) as f32;
        let band_rect = Rect::from_min_size(
            egui::pos2(
                rect.left() + 2.0,
                rect.bottom() - (i + 1) as f32 * band_height,
            ),
            vec2(rect.width() - 4.0, band_height - 1.0),
        );
        painter.rect_filled(band_rect, 1.0, gradient_color(t));
    }

    painter.rect_stroke(
        rect,
        5.0,
        Stroke::new(1.0, theme::BORDER),
        StrokeKind::Inside,
    );

    // フェーダーのつまみ
    let thumb_y = rect.bottom() - gain.clamp(0.0, 1.0) * rect.height();
    let thumb_rect = Rect::from_center_size(
        egui::pos2(rect.center().x, thumb_y),
        vec2(rect.width() + 10.0, 9.0),
    );
    let thumb_color = if muted {
        Color32::from_rgb(0x8a, 0x8a, 0x90)
    } else {
        Color32::from_rgb(0xf2, 0xf2, 0xf4)
    };
    painter.rect_filled(thumb_rect, 2.5, thumb_color);
    painter.rect_stroke(
        thumb_rect,
        2.5,
        Stroke::new(1.0, Color32::from_rgb(0x50, 0x50, 0x56)),
        StrokeKind::Outside,
    );
    painter.line_segment(
        [
            egui::pos2(thumb_rect.left() + 5.0, thumb_rect.center().y),
            egui::pos2(thumb_rect.right() - 5.0, thumb_rect.center().y),
        ],
        Stroke::new(1.0, Color32::from_rgb(0x8a, 0x8a, 0x90)),
    );
}

/// t=0.0(静か)は緑、t=1.0(大音量)は赤になるグラデーション色を返す。
fn gradient_color(t: f32) -> Color32 {
    const GREEN: Color32 = Color32::from_rgb(0x3e, 0xcf, 0x5e);
    const YELLOW: Color32 = Color32::from_rgb(0xf4, 0xc4, 0x30);
    const RED: Color32 = Color32::from_rgb(0xef, 0x44, 0x44);
    const YELLOW_AT: f32 = 0.75;

    if t < YELLOW_AT {
        lerp_color(GREEN, YELLOW, t / YELLOW_AT)
    } else {
        lerp_color(YELLOW, RED, (t - YELLOW_AT) / (1.0 - YELLOW_AT))
    }
}

fn lerp_color(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let lerp = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * t).round() as u8;
    Color32::from_rgb(lerp(a.r(), b.r()), lerp(a.g(), b.g()), lerp(a.b(), b.b()))
}
