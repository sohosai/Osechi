use eframe::egui::{self, Color32, CornerRadius, Stroke};

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

/// アプリ全体のダークテーマを適用する。
/// egui標準のダークテーマをベースに、パネルの配色・アクセントカラー・
/// ウィジェットの角丸を統一のトークンで上書きする。
pub fn apply(ctx: &egui::Context) {
    let mut visuals = egui::Visuals::dark();

    visuals.panel_fill = BG_PANEL;
    visuals.window_fill = BG_PANEL_HEADER;
    visuals.window_stroke = Stroke::new(1.0, BORDER);
    visuals.hyperlink_color = ACCENT_SELECT;
    visuals.selection.bg_fill = ACCENT_SELECT;
    visuals.selection.stroke = Stroke::new(1.0, Color32::BLACK);

    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, BORDER);
    visuals.widgets.inactive.weak_bg_fill = BG_PANEL_HEADER;
    visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, BORDER);
    visuals.widgets.hovered.weak_bg_fill = BG_ROW_HOVER;
    visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, ACCENT_SELECT);
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

/// 現在の割り当て先を示す小さな角丸バッジ(PVW/PGM/IN n/MIX など)を描画する。
pub fn badge(ui: &mut egui::Ui, text: &str, color: Color32) {
    egui::Frame::new()
        .fill(Color32::from_rgba_unmultiplied(
            color.r(),
            color.g(),
            color.b(),
            40,
        ))
        .corner_radius(3.0)
        .inner_margin(egui::Margin::symmetric(6, 1))
        .show(ui, |ui| {
            ui.label(egui::RichText::new(text).small().strong().color(color));
        });
}

/// バッジの文言(PVW/PGM/IN n/MIX)から対応するアクセントカラーを返す。
pub fn badge_color(text: &str) -> Color32 {
    match text {
        "PVW" => ACCENT_PREVIEW,
        "PGM" => ACCENT_PROGRAM,
        "MIX" => CHIP_AUDIO_FG,
        _ => ACCENT_SELECT,
    }
}

/// ソース種別を示す小さなアイコンチップ(CAM/SCR/MICなど)を描画する。
pub fn icon_chip(ui: &mut egui::Ui, text: &str, bg: Color32, fg: Color32) -> egui::Response {
    egui::Frame::new()
        .fill(bg)
        .corner_radius(4.0)
        .inner_margin(egui::Margin::symmetric(5, 2))
        .show(ui, |ui| {
            ui.label(
                egui::RichText::new(text)
                    .small()
                    .strong()
                    .color(fg)
                    .monospace(),
            );
        })
        .response
}
