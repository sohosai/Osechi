//! 中央のマルチビュー。上段にPreview・Program、下段にInput 1..8を並べる。
//!
//! 映像ソースをドロップするとそのスロットに割り当て、Inputをクリックするとその映像をPreviewに出す。

use eframe::egui::{self, Color32, Painter, Rect, Sense, Stroke, StrokeKind, Ui, Vec2, vec2};

use super::{Drag, theme};
use crate::app::App;
use crate::switcher::{INPUTS, Slot};

/// 映像の縦横比。
const ASPECT: f32 = 16.0 / 9.0;
/// Inputを並べる列数。
const COLUMNS: usize = 4;

pub(super) fn show(app: &mut App, ui: &mut Ui) {
    let frame = egui::Frame::central_panel(&ui.ctx().global_style()).inner_margin(0.0);
    egui::CentralPanel::default()
        .frame(frame)
        .show_inside(ui, |ui| {
            let (response, painter) = ui.allocate_painter(ui.available_size(), Sense::hover());
            let canvas = fit(response.rect);
            painter.rect_filled(canvas, 0.0, Color32::BLACK);

            for (slot, rect) in layout(canvas) {
                let drop = accept_drop(app, ui, slot, rect);
                if paint_cell(app, ui, &painter, slot, rect, drop) {
                    app.switcher[slot] = None;
                }
            }
        });
}

/// `area` の中央に、16:9 で収まる最大のキャンバスを置く。
/// 幅は4の倍数・高さは2の倍数に丸め、Inputのセルがピクセル境界に揃うようにする。
fn fit(area: Rect) -> Rect {
    let mut width = (area.width() - 2.0).max(16.0);
    let mut height = (area.height() - 2.0).max(16.0);
    if width / height > ASPECT {
        width = height * ASPECT;
    } else {
        height = width / ASPECT;
    }
    let width = ((width as usize).max(16) / 4 * 4) as f32;
    let height = ((height as usize).max(16) / 2 * 2) as f32;
    Rect::from_center_size(area.center(), vec2(width, height))
}

/// 各スロットのセルの位置。上半分を Preview | Program、下半分を Input の格子にする。
fn layout(canvas: Rect) -> impl Iterator<Item = (Slot, Rect)> {
    let half = vec2(canvas.width() / 2.0, canvas.height() / 2.0);
    let cell = vec2(
        canvas.width() / COLUMNS as f32,
        half.y / (INPUTS / COLUMNS) as f32,
    );
    let top = [
        (Slot::Preview, Rect::from_min_size(canvas.min, half)),
        (
            Slot::Program,
            Rect::from_min_size(canvas.min + vec2(half.x, 0.0), half),
        ),
    ];
    let inputs = (0..INPUTS).map(move |i| {
        let offset = vec2(
            (i % COLUMNS) as f32 * cell.x,
            half.y + (i / COLUMNS) as f32 * cell.y,
        );
        (
            Slot::Input(i),
            Rect::from_min_size(canvas.min + offset, cell),
        )
    });
    top.into_iter().chain(inputs)
}

/// セルをドロップ先として登録し、落とされた映像ソースをそのスロットに割り当てる。
/// Inputのセルはクリックで、割り当てられている映像をPreviewに出す。
///
/// 戻り値は、いまドラッグ中のものをこのセルに落とせるか(セル上でドラッグ中でなければ `None`)。
fn accept_drop(app: &mut App, ui: &mut Ui, slot: Slot, rect: Rect) -> Option<bool> {
    let sense = match slot {
        Slot::Input(_) => Sense::click(),
        Slot::Preview | Slot::Program => Sense::hover(),
    };
    let response = ui.interact(rect, egui::Id::new(("multiview", slot)), sense);

    if let Some(payload) = response.dnd_release_payload::<Drag>()
        && let Drag::Video(id) = &*payload
    {
        app.switcher[slot] = Some(id.clone());
    }
    if response.clicked()
        && let Some(id) = app.switcher[slot].clone()
    {
        app.switcher[Slot::Preview] = Some(id);
    }

    let dragging = egui::DragAndDrop::payload::<Drag>(ui.ctx())?;
    response
        .contains_pointer()
        .then(|| matches!(*dragging, Drag::Video(_)))
}

/// セルを描く。Inputの割り当て解除ボタンが押されたら `true` を返す。
fn paint_cell(
    app: &App,
    ui: &mut Ui,
    painter: &Painter,
    slot: Slot,
    rect: Rect,
    drop: Option<bool>,
) -> bool {
    let source = app.switcher[slot].as_ref();
    let on = |target: Slot| source.is_some() && app.switcher[target].as_ref() == source;
    let (on_preview, on_program) = (on(Slot::Preview), on(Slot::Program));

    // 枠: Preview/Programは固定色、InputはPGM/PVWに出ているかで色を変える(タリー)
    let (mut color, mut width) = match slot {
        Slot::Preview | Slot::Program => (theme::slot_color(slot), 3.0),
        Slot::Input(_) if on_program => (theme::ACCENT_PROGRAM, 3.0),
        Slot::Input(_) if on_preview => (theme::ACCENT_PREVIEW, 3.0),
        Slot::Input(_) => (theme::BORDER, 1.0),
    };
    if let Some(accepts) = drop {
        color = if accepts {
            theme::ACCENT_SELECT
        } else {
            theme::ACCENT_PROGRAM
        };
        width = 3.0;
        painter.rect_filled(rect, 2.0, color.gamma_multiply(0.12));
    }

    if let Some(texture) = source.and_then(|id| app.textures.get(id)) {
        painter.image(
            texture.id(),
            rect,
            crop_to_aspect(texture.size_vec2()),
            Color32::WHITE,
        );
    }

    painter.rect_stroke(
        rect.shrink(width / 2.0),
        0.0,
        Stroke::new(width, color),
        StrokeKind::Inside,
    );
    if matches!(slot, Slot::Input(_)) && on_program && on_preview {
        painter.rect_stroke(
            rect.shrink(4.5),
            0.0,
            Stroke::new(3.0_f32, theme::ACCENT_PREVIEW),
            StrokeKind::Inside,
        );
    }

    let label = match slot {
        Slot::Preview | Slot::Program => {
            paint_tag(painter, rect, slot);
            source
                .map_or("No Source", |id| app.video.name(id))
                .to_string()
        }
        Slot::Input(index) => format!("Input {}", index + 1),
    };
    if app.ui.show_labels {
        paint_label(painter, rect, label);
    }

    matches!(slot, Slot::Input(_))
        && source.is_some()
        && ui
            .put(
                Rect::from_min_size(rect.right_top() + vec2(-20.0, 4.0), vec2(16.0, 16.0)),
                egui::Button::new("x").small(),
            )
            .on_hover_text("Unassign")
            .clicked()
}

/// 画像を歪めずセルいっぱいに出すため、はみ出す側を中央で切り取るUV範囲を返す。
fn crop_to_aspect(size: Vec2) -> Rect {
    let aspect = size.x / size.y;
    let full = Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
    if aspect < ASPECT {
        let margin = (1.0 - aspect / ASPECT) / 2.0;
        full.shrink2(vec2(0.0, margin))
    } else if aspect > ASPECT {
        let margin = (1.0 - ASPECT / aspect) / 2.0;
        full.shrink2(vec2(margin, 0.0))
    } else {
        full
    }
}

/// セルの左上に、役割を示すタグ(PREVIEW / PROGRAM)を描く。
fn paint_tag(painter: &Painter, rect: Rect, slot: Slot) {
    let text = match slot {
        Slot::Preview => "PREVIEW",
        _ => "PROGRAM",
    };
    let ink = Color32::from_black_alpha(230);
    let galley = painter.layout_no_wrap(text.to_string(), egui::FontId::proportional(10.0), ink);
    let padding = vec2(6.0, 2.0);
    let tag = Rect::from_min_size(
        rect.left_top() + vec2(8.0, 8.0),
        galley.size() + padding * 2.0,
    );
    painter.rect_filled(tag, 2.0, theme::slot_color(slot));
    painter.galley(tag.min + padding, galley, ink);
}

/// セルの下中央に、半透明の背景付きでラベルを描く。
fn paint_label(painter: &Painter, rect: Rect, text: String) {
    let galley = painter.layout_no_wrap(text, egui::FontId::proportional(16.0), Color32::WHITE);
    let size = galley.size();
    let pos = egui::pos2(rect.center().x - size.x / 2.0, rect.max.y - size.y - 8.0);
    painter.rect_filled(
        Rect::from_min_size(pos - vec2(6.0, 2.0), size + vec2(12.0, 4.0)),
        4.0,
        Color32::from_black_alpha(160),
    );
    painter.galley(pos, galley, Color32::WHITE);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fit_keeps_16_9_and_pixel_alignment() {
        let area = Rect::from_min_size(egui::pos2(0.0, 0.0), vec2(1003.0, 900.0));
        let canvas = fit(area);
        assert_eq!(canvas.width() as usize % 4, 0);
        assert_eq!(canvas.height() as usize % 2, 0);
        assert!((canvas.width() / canvas.height() - ASPECT).abs() < 0.01);
        assert_eq!(canvas.center(), area.center());
    }

    #[test]
    fn layout_covers_every_slot_once() {
        let canvas = Rect::from_min_size(egui::pos2(0.0, 0.0), vec2(1600.0, 900.0));
        let slots: Vec<Slot> = layout(canvas).map(|(slot, _)| slot).collect();
        assert_eq!(slots, Slot::all().collect::<Vec<_>>());
    }

    #[test]
    fn crop_trims_the_overflowing_side() {
        // 4:3 は上下を切る
        let uv = crop_to_aspect(vec2(4.0, 3.0));
        assert_eq!((uv.min.x, uv.max.x), (0.0, 1.0));
        assert!(uv.min.y > 0.0 && uv.max.y < 1.0);
        // 21:9 は左右を切る
        let uv = crop_to_aspect(vec2(21.0, 9.0));
        assert!(uv.min.x > 0.0 && uv.max.x < 1.0);
    }
}
