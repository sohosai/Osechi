//! 配色・フォント・余白。色は全てここの定数を使う。

use std::sync::Arc;

use eframe::egui::{self, Color32, CornerRadius, Stroke};

use crate::switcher::Slot;

pub const BG_PANEL: Color32 = Color32::from_rgb(0x1a, 0x1a, 0x1e);
pub const BG_PANEL_HEADER: Color32 = Color32::from_rgb(0x21, 0x21, 0x25);
pub const BG_ROW_HOVER: Color32 = Color32::from_rgb(0x28, 0x28, 0x2e);
pub const BORDER: Color32 = Color32::from_rgb(0x38, 0x38, 0x3e);

pub const ACCENT_SELECT: Color32 = Color32::from_rgb(0x5b, 0x9d, 0xff);
pub const ACCENT_PREVIEW: Color32 = Color32::from_rgb(0x3e, 0xcf, 0x5e);
pub const ACCENT_PROGRAM: Color32 = Color32::from_rgb(0xef, 0x44, 0x44);

pub const CHIP_VIDEO_BG: Color32 = Color32::from_rgb(0x1a, 0x2a, 0x3e);
pub const CHIP_VIDEO_FG: Color32 = Color32::from_rgb(0x79, 0xb1, 0xff);
pub const CHIP_AUDIO_BG: Color32 = Color32::from_rgb(0x28, 0x20, 0x38);
pub const CHIP_AUDIO_FG: Color32 = Color32::from_rgb(0xbd, 0x93, 0xff);
pub const CHIP_DANTE_BG: Color32 = Color32::from_rgb(0x2a, 0x22, 0x14);
pub const CHIP_DANTE_FG: Color32 = Color32::from_rgb(0xe0, 0xa8, 0x58);

/// スロットを表す色(PVW=緑, PGM=赤, Input=青)。
pub fn slot_color(slot: Slot) -> Color32 {
    match slot {
        Slot::Preview => ACCENT_PREVIEW,
        Slot::Program => ACCENT_PROGRAM,
        Slot::Input(_) => ACCENT_SELECT,
    }
}

/// フォント・ダークテーマ・余白をアプリ全体に適用する。
pub fn install(ctx: &egui::Context) {
    install_cjk_fallback(ctx);

    let mut visuals = egui::Visuals::dark();
    visuals.panel_fill = BG_PANEL;
    visuals.window_fill = BG_PANEL_HEADER;
    visuals.window_stroke = Stroke::new(1.0_f32, BORDER);
    visuals.hyperlink_color = ACCENT_SELECT;
    visuals.selection.bg_fill = ACCENT_SELECT;
    visuals.selection.stroke = Stroke::new(1.0_f32, Color32::BLACK);

    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, BORDER);
    visuals.widgets.inactive.weak_bg_fill = BG_PANEL_HEADER;
    visuals.widgets.inactive.bg_stroke = Stroke::new(1.0_f32, BORDER);
    visuals.widgets.hovered.weak_bg_fill = BG_ROW_HOVER;
    visuals.widgets.hovered.bg_stroke = Stroke::new(1.0_f32, ACCENT_SELECT);
    visuals.widgets.active.weak_bg_fill = BG_ROW_HOVER;
    for widget in [
        &mut visuals.widgets.noninteractive,
        &mut visuals.widgets.inactive,
        &mut visuals.widgets.hovered,
        &mut visuals.widgets.active,
        &mut visuals.widgets.open,
    ] {
        widget.corner_radius = CornerRadius::from(4);
    }
    ctx.set_visuals(visuals);

    let mut style = (*ctx.global_style()).clone();
    style.spacing.item_spacing = egui::vec2(8.0, 6.0);
    style.spacing.button_padding = egui::vec2(8.0, 3.0);
    ctx.set_global_style(style);
}

/// OSのCJKフォントを優先度の低いフォールバックとして追加し、デバイス名など
/// OSから渡ってくる日本語が豆腐(□)にならないようにする。見つからなければ既定のまま。
fn install_cjk_fallback(ctx: &egui::Context) {
    /// 探すフォント(パス, collection内のフェイス番号)。先に見つかったものを使う。
    const CANDIDATES: &[(&str, u32)] = &[
        // Windows
        (r"C:\Windows\Fonts\YuGothM.ttc", 0),
        (r"C:\Windows\Fonts\meiryo.ttc", 0),
        (r"C:\Windows\Fonts\msgothic.ttc", 0),
        // macOS
        ("/System/Library/Fonts/ヒラギノ角ゴシック W4.ttc", 0),
        ("/System/Library/Fonts/Supplemental/Arial Unicode.ttf", 0),
        // Linux
        ("/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc", 0),
        ("/usr/share/fonts/truetype/noto/NotoSansCJK-Regular.ttc", 0),
    ];
    const NAME: &str = "cjk_fallback";

    let Some((bytes, index)) = CANDIDATES
        .iter()
        .find_map(|&(path, index)| std::fs::read(path).ok().map(|bytes| (bytes, index)))
    else {
        return;
    };

    let mut fonts = egui::FontDefinitions::default();
    let mut font = egui::FontData::from_owned(bytes);
    font.index = index;
    fonts.font_data.insert(NAME.to_owned(), Arc::new(font));
    for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
        fonts
            .families
            .entry(family)
            .or_default()
            .push(NAME.to_owned());
    }
    ctx.set_fonts(fonts);
}
