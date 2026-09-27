//! 複数のパネルで使う部品。どれも `ui.add(...)` で置ける `egui::Widget`。

use eframe::egui::{
    self, Align2, Color32, FontFamily, FontId, Rect, Response, Sense, Stroke, StrokeKind, Ui,
    Widget, pos2, vec2,
};

use super::theme;
use crate::mixer::{self, Strip};
use crate::source::{audio, video};

/// ソースの種別を示す小さなチップ(`CAM` `MIC` `DANTE` など)。
#[derive(Debug, Clone, Copy)]
pub struct Chip {
    text: &'static str,
    bg: Color32,
    fg: Color32,
}

impl Chip {
    pub fn video(kind: &video::Kind) -> Self {
        let text = match kind {
            video::Kind::Camera(_) => "CAM",
            video::Kind::Screen(_) => "SCR",
        };
        Self {
            text,
            bg: theme::CHIP_VIDEO_BG,
            fg: theme::CHIP_VIDEO_FG,
        }
    }

    pub fn audio(kind: &audio::Kind) -> Self {
        match kind {
            audio::Kind::Device(_) => Self {
                text: "MIC",
                bg: theme::CHIP_AUDIO_BG,
                fg: theme::CHIP_AUDIO_FG,
            },
            audio::Kind::Aes67(_) => Self {
                text: "DANTE",
                bg: theme::CHIP_DANTE_BG,
                fg: theme::CHIP_DANTE_FG,
            },
        }
    }

    /// 種別を表す色。チップ以外の場所で種別を示すのにも使う。
    pub fn accent(self) -> Color32 {
        self.fg
    }
}

impl Widget for Chip {
    fn ui(self, ui: &mut Ui) -> Response {
        egui::Frame::new()
            .fill(self.bg)
            .corner_radius(4.0)
            .inner_margin(egui::Margin::symmetric(5, 2))
            .show(ui, |ui| {
                ui.label(
                    egui::RichText::new(self.text)
                        .small()
                        .strong()
                        .color(self.fg)
                        .monospace(),
                );
            })
            .response
    }
}

/// 割り当て先を示す小さな角丸バッジ(`PVW` `PGM` `IN 1` `MIX` など)。
pub struct Badge {
    text: String,
    color: Color32,
}

impl Badge {
    pub fn new(text: impl ToString, color: Color32) -> Self {
        Self {
            text: text.to_string(),
            color,
        }
    }
}

impl Widget for Badge {
    fn ui(self, ui: &mut Ui) -> Response {
        let [r, g, b, _] = self.color.to_array();
        egui::Frame::new()
            .fill(Color32::from_rgba_unmultiplied(r, g, b, 40))
            .corner_radius(3.0)
            .inner_margin(egui::Margin::symmetric(6, 1))
            .show(ui, |ui| {
                ui.label(
                    egui::RichText::new(self.text)
                        .small()
                        .strong()
                        .color(self.color),
                );
            })
            .response
    }
}

/// レベルメーターと一体化したフェーダー。クリック・ドラッグでフェーダー位置を動かす。
/// メーターはフェーダーと同じdBの目盛りで描き、ミュート中は消す。
pub struct Fader<'a>(pub &'a mut Strip);

impl Fader<'_> {
    const SIZE: egui::Vec2 = vec2(30.0, 148.0);
    const BANDS: usize = 32;
}

impl Widget for Fader<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        let strip = self.0;
        let (rect, response) = ui.allocate_exact_size(Self::SIZE, Sense::click_and_drag());
        if (response.dragged() || response.clicked())
            && let Some(pos) = response.interact_pointer_pos()
        {
            strip.gain = 1.0 - ((pos.y - rect.top()) / rect.height()).clamp(0.0, 1.0);
        }

        let painter = ui.painter();
        painter.rect_filled(rect, 5.0, Color32::from_rgb(0x0a, 0x0a, 0x0c));

        // メーター: 下から点灯するバンド。緑→黄→赤のグラデーション。
        let meter = if strip.muted {
            0.0
        } else {
            mixer::level_to_meter(strip.level)
        };
        let band_height = rect.height() / Self::BANDS as f32;
        let lit = (meter * Self::BANDS as f32).round() as usize;
        for i in 0..lit {
            let band = Rect::from_min_size(
                pos2(
                    rect.left() + 2.0,
                    rect.bottom() - (i + 1) as f32 * band_height,
                ),
                vec2(rect.width() - 4.0, band_height - 1.0),
            );
            painter.rect_filled(band, 1.0, meter_color(i as f32 / (Self::BANDS - 1) as f32));
        }
        painter.rect_stroke(
            rect,
            5.0,
            Stroke::new(1.0_f32, theme::BORDER),
            StrokeKind::Inside,
        );

        // つまみ
        let thumb_y = rect.bottom() - strip.gain.clamp(0.0, 1.0) * rect.height();
        let thumb = Rect::from_center_size(
            pos2(rect.center().x, thumb_y),
            vec2(rect.width() + 10.0, 9.0),
        );
        let thumb_color = if strip.muted {
            Color32::from_rgb(0x8a, 0x8a, 0x90)
        } else {
            Color32::from_rgb(0xf2, 0xf2, 0xf4)
        };
        painter.rect_filled(thumb, 2.5, thumb_color);
        painter.rect_stroke(
            thumb,
            2.5,
            Stroke::new(1.0_f32, Color32::from_rgb(0x50, 0x50, 0x56)),
            StrokeKind::Outside,
        );
        painter.line_segment(
            [
                pos2(thumb.left() + 5.0, thumb.center().y),
                pos2(thumb.right() - 5.0, thumb.center().y),
            ],
            Stroke::new(1.0_f32, Color32::from_rgb(0x8a, 0x8a, 0x90)),
        );

        response
    }
}

/// メーターの `t`(0.0=下端, 1.0=上端)の位置の色。緑→(0.75で)黄→赤。
fn meter_color(t: f32) -> Color32 {
    const GREEN: Color32 = Color32::from_rgb(0x3e, 0xcf, 0x5e);
    const YELLOW: Color32 = Color32::from_rgb(0xf4, 0xc4, 0x30);
    const RED: Color32 = Color32::from_rgb(0xef, 0x44, 0x44);
    const YELLOW_AT: f32 = 0.75;

    if t < YELLOW_AT {
        lerp(GREEN, YELLOW, t / YELLOW_AT)
    } else {
        lerp(YELLOW, RED, (t - YELLOW_AT) / (1.0 - YELLOW_AT))
    }
}

fn lerp(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let mix = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * t).round() as u8;
    Color32::from_rgb(mix(a.r(), b.r()), mix(a.g(), b.g()), mix(a.b(), b.b()))
}

/// LEDインジケータ付きのミュートボタン。クリックで `muted` を切り替える。
/// ミュート中は塗りつぶし+グロー、通常時はゴースト調にして一目で状態が分かるようにする。
pub struct MuteButton<'a> {
    muted: &'a mut bool,
    width: f32,
}

impl<'a> MuteButton<'a> {
    pub fn new(muted: &'a mut bool, width: f32) -> Self {
        Self { muted, width }
    }
}

impl Widget for MuteButton<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        let (rect, response) = ui.allocate_exact_size(vec2(self.width, 28.0), Sense::click());
        if response.clicked() {
            *self.muted = !*self.muted;
        }
        let muted = *self.muted;

        let (bg, border, text_color, led_color) = if muted {
            (
                theme::ACCENT_PROGRAM,
                theme::ACCENT_PROGRAM,
                Color32::WHITE,
                Color32::WHITE,
            )
        } else if response.hovered() {
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

        let painter = ui.painter();
        if muted {
            painter.rect_stroke(
                rect.expand(2.0),
                7.0,
                Stroke::new(1.0_f32, theme::ACCENT_PROGRAM.gamma_multiply(0.35)),
                StrokeKind::Outside,
            );
        }
        painter.rect_filled(rect, 5.0, bg);
        painter.rect_stroke(rect, 5.0, Stroke::new(1.0_f32, border), StrokeKind::Inside);

        let led = rect.left_center() + vec2(15.0, 0.0);
        painter.circle_filled(led, 4.0, led_color);
        if muted {
            painter.circle_stroke(
                led,
                6.5,
                Stroke::new(1.0_f32, Color32::from_rgba_unmultiplied(255, 255, 255, 70)),
            );
        }
        painter.text(
            led + vec2(10.0, 0.0),
            Align2::LEFT_CENTER,
            "MUTE",
            FontId::new(12.0, FontFamily::Proportional),
            text_color,
        );

        response.on_hover_text(if muted { "Unmute" } else { "Mute" })
    }
}

/// ホバーで赤く反応する、円形の小さな削除ボタン(×)。
pub struct CloseButton;

impl Widget for CloseButton {
    fn ui(self, ui: &mut Ui) -> Response {
        let (rect, response) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::click());
        let color = if response.hovered() {
            ui.painter().circle_filled(
                rect.center(),
                9.0,
                theme::ACCENT_PROGRAM.gamma_multiply(0.22),
            );
            theme::ACCENT_PROGRAM
        } else {
            ui.visuals().weak_text_color()
        };

        let inner = rect.shrink(4.5);
        let stroke = Stroke::new(1.3_f32, color);
        ui.painter()
            .line_segment([inner.left_top(), inner.right_bottom()], stroke);
        ui.painter()
            .line_segment([inner.left_bottom(), inner.right_top()], stroke);
        response
    }
}
