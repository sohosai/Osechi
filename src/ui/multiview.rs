use crate::app::{DragPayload, OsechiApp};
use crate::source::video::SourceId;
use crate::ui::theme;
use eframe::egui;

const GRID_COLS: usize = 4;
const GRID_ROWS: usize = 2;

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum DropTarget {
    Preview,
    Program,
    Input(usize),
}

/// ドロップ受付の見た目の状態(accept=映像ソースを受け付けられる, reject=受け付けられない)
#[derive(Clone, Copy, Default)]
struct DropVisualState {
    accept: bool,
    reject: bool,
}

impl OsechiApp {
    /// 映像のPreview/Program/Multiview のUI
    pub(crate) fn draw_multiview(&mut self, ui: &mut egui::Ui) {
        let frame = egui::Frame::central_panel(&ui.ctx().global_style()).inner_margin(0.0);
        egui::CentralPanel::default()
            .frame(frame)
            .show_inside(ui, |ui| {
                let available = ui.available_size();
                let (canvas_width, canvas_height) = Self::fit_canvas_size(available);
                let top_height = canvas_height / 2;
                let bottom_height = canvas_height - top_height;
                let top_view_width = canvas_width / 2;

                let x_offset = ((available.x - canvas_width as f32) / 2.0).max(0.0);
                let y_offset = ((available.y - canvas_height as f32) / 2.0).max(0.0);

                let (response, painter) = ui.allocate_painter(available, egui::Sense::hover());
                let base_pos = response.rect.min + egui::vec2(x_offset, y_offset);

                let bg_rect = egui::Rect::from_min_size(
                    base_pos,
                    egui::vec2(canvas_width as f32, canvas_height as f32),
                );
                painter.rect_filled(bg_rect, 0.0, egui::Color32::BLACK);

                let preview_rect = egui::Rect::from_min_size(
                    base_pos,
                    egui::vec2(top_view_width as f32, top_height as f32),
                );
                let program_rect = egui::Rect::from_min_size(
                    base_pos + egui::vec2(top_view_width as f32, 0.0),
                    egui::vec2(top_view_width as f32, top_height as f32),
                );

                let cell_width = canvas_width as f32 / GRID_COLS as f32;
                let cell_height = bottom_height as f32 / GRID_ROWS as f32;
                let input_rects: Vec<egui::Rect> = (0..GRID_ROWS)
                    .flat_map(|r| (0..GRID_COLS).map(move |c| (r, c)))
                    .map(|(r, c)| {
                        egui::Rect::from_min_size(
                            base_pos
                                + egui::vec2(
                                    c as f32 * cell_width,
                                    top_height as f32 + r as f32 * cell_height,
                                ),
                            egui::vec2(cell_width, cell_height),
                        )
                    })
                    .collect();

                // --- インタラクション(ドラッグ&ドロップの受付・クリック)を先に処理する ---
                let preview_state = self.handle_drop_target(ui, preview_rect, DropTarget::Preview);
                let program_state = self.handle_drop_target(ui, program_rect, DropTarget::Program);
                let input_states: Vec<DropVisualState> = input_rects
                    .iter()
                    .enumerate()
                    .map(|(idx, &rect)| self.handle_drop_target(ui, rect, DropTarget::Input(idx)))
                    .collect();

                let preview_label = self
                    .preview_source_id
                    .as_ref()
                    .map(|id| {
                        crate::ui::utils::video_source_name(id, self.video_source_manager.list())
                    })
                    .unwrap_or_else(|| "No Source".to_string());
                let program_label = self
                    .selected_source_id
                    .as_ref()
                    .map(|id| {
                        crate::ui::utils::video_source_name(id, self.video_source_manager.list())
                    })
                    .unwrap_or_else(|| "No Source".to_string());

                // --- 描画(以降は読み取りのみ) ---
                let draw_cam = |ui: &mut egui::Ui,
                                source_id: Option<SourceId>,
                                rect: egui::Rect,
                                label_text: &str,
                                border_override: Option<egui::Color32>,
                                drop_state: DropVisualState,
                                input_idx: Option<usize>|
                 -> bool {
                    let mut is_preview = false;
                    let mut is_program = false;

                    if let Some(id) = &source_id {
                        is_preview = Some(id) == self.preview_source_id.as_ref();
                        is_program = Some(id) == self.selected_source_id.as_ref();
                    }

                    let mut stroke_color = theme::BORDER;
                    let mut stroke_width = 1.0;

                    if let Some(c) = border_override {
                        stroke_color = c;
                        stroke_width = 3.0;
                    } else if is_program {
                        stroke_color = theme::ACCENT_PROGRAM;
                        stroke_width = 3.0;
                    } else if is_preview {
                        stroke_color = theme::ACCENT_PREVIEW;
                        stroke_width = 3.0;
                    }

                    if drop_state.accept {
                        stroke_color = theme::ACCENT_SELECT;
                        stroke_width = 3.0;
                        painter.rect_filled(rect, 2.0, theme::ACCENT_SELECT.gamma_multiply(0.12));
                    } else if drop_state.reject {
                        stroke_color = theme::ACCENT_PROGRAM;
                        stroke_width = 3.0;
                        painter.rect_filled(rect, 2.0, theme::ACCENT_PROGRAM.gamma_multiply(0.12));
                    }

                    if let Some(id) = &source_id
                        && let Some(active) = self.active_sources.get(id)
                        && let Some(tex) = &active.texture
                    {
                        let img_aspect = tex.width as f32 / tex.height as f32;
                        let target_aspect = 16.0 / 9.0;

                        let mut uv =
                            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));

                        if img_aspect < target_aspect {
                            let crop_ratio = img_aspect / target_aspect;
                            let offset = (1.0 - crop_ratio) / 2.0;
                            uv = egui::Rect::from_min_max(
                                egui::pos2(0.0, offset),
                                egui::pos2(1.0, 1.0 - offset),
                            );
                        } else if img_aspect > target_aspect {
                            let crop_ratio = target_aspect / img_aspect;
                            let offset = (1.0 - crop_ratio) / 2.0;
                            uv = egui::Rect::from_min_max(
                                egui::pos2(offset, 0.0),
                                egui::pos2(1.0 - offset, 1.0),
                            );
                        }

                        painter.image(tex.handle.id(), rect, uv, egui::Color32::WHITE);
                    }

                    painter.rect_stroke(
                        rect.shrink(stroke_width / 2.0),
                        0.0,
                        egui::Stroke::new(stroke_width, stroke_color),
                        egui::StrokeKind::Inside,
                    );

                    if border_override.is_none() && is_program && is_preview {
                        painter.rect_stroke(
                            rect.shrink(3.0 + 3.0 / 2.0),
                            0.0,
                            egui::Stroke::new(3.0, theme::ACCENT_PREVIEW),
                            egui::StrokeKind::Inside,
                        );
                    }

                    if let Some(tag_color) = border_override {
                        let tag_text = if tag_color == theme::ACCENT_PREVIEW {
                            "PREVIEW"
                        } else {
                            "PROGRAM"
                        };
                        paint_corner_tag(&painter, rect, tag_text, tag_color);
                    }

                    if self.show_labels && !label_text.is_empty() {
                        let text_color = egui::Color32::WHITE;
                        let bg_color = egui::Color32::from_black_alpha(160);
                        let font_id = egui::FontId::proportional(16.0);

                        let galley =
                            painter.layout_no_wrap(label_text.to_string(), font_id, text_color);

                        let text_size = galley.size();
                        let text_pos = egui::pos2(
                            rect.center().x - text_size.x / 2.0,
                            rect.max.y - text_size.y - 8.0,
                        );

                        let bg_rect = egui::Rect::from_min_size(
                            text_pos - egui::vec2(6.0, 2.0),
                            text_size + egui::vec2(12.0, 4.0),
                        );

                        painter.rect_filled(bg_rect, 4.0, bg_color);
                        painter.galley(text_pos, galley, egui::Color32::WHITE);
                    }

                    if input_idx.is_some() && source_id.is_some() {
                        let clear_rect = egui::Rect::from_min_size(
                            rect.right_top() + egui::vec2(-20.0, 4.0),
                            egui::vec2(16.0, 16.0),
                        );
                        ui.put(clear_rect, egui::Button::new("x").small())
                            .on_hover_text("Unassign")
                            .clicked()
                    } else {
                        false
                    }
                };

                let mut clear_input_idx = None;

                draw_cam(
                    ui,
                    self.preview_source_id.clone(),
                    preview_rect,
                    &preview_label,
                    Some(theme::ACCENT_PREVIEW),
                    preview_state,
                    None,
                );
                draw_cam(
                    ui,
                    self.selected_source_id.clone(),
                    program_rect,
                    &program_label,
                    Some(theme::ACCENT_PROGRAM),
                    program_state,
                    None,
                );

                for (idx, &rect) in input_rects.iter().enumerate() {
                    let source_id = self.inputs[idx].clone();
                    let label = format!("Input {}", idx + 1);
                    let clicked_clear = draw_cam(
                        ui,
                        source_id,
                        rect,
                        &label,
                        None,
                        input_states[idx],
                        Some(idx),
                    );
                    if clicked_clear {
                        clear_input_idx = Some(idx);
                    }
                }

                if let Some(idx) = clear_input_idx {
                    self.clear_input(idx);
                }
            });
    }

    /// 指定した矩形をドロップ対象として登録し、映像ソースの受付判定と
    /// 実際の割り当て・クリックでのPreview読み込みを行う。
    fn handle_drop_target(
        &mut self,
        ui: &mut egui::Ui,
        rect: egui::Rect,
        target: DropTarget,
    ) -> DropVisualState {
        let sense = match target {
            DropTarget::Input(_) => egui::Sense::click(),
            DropTarget::Preview | DropTarget::Program => egui::Sense::hover(),
        };
        let id = egui::Id::new(("multiview_drop_target", target));
        let response = ui.interact(rect, id, sense);

        let dragging = egui::DragAndDrop::payload::<DragPayload>(ui.ctx());
        let mut state = DropVisualState::default();
        if response.contains_pointer()
            && let Some(payload) = &dragging
        {
            let is_video = matches!(**payload, DragPayload::Video(_));
            state.accept = is_video;
            state.reject = !is_video;
        }

        if let Some(payload) = response.dnd_release_payload::<DragPayload>()
            && let DragPayload::Video(source_id) = &*payload
        {
            match target {
                DropTarget::Preview => self.assign_preview(source_id.clone()),
                DropTarget::Program => self.assign_program(source_id.clone()),
                DropTarget::Input(idx) => self.assign_input(idx, source_id.clone()),
            }
        }

        if let DropTarget::Input(idx) = target
            && response.clicked()
            && let Some(source_id) = self.inputs[idx].clone()
        {
            self.assign_preview(source_id);
        }

        state
    }
}

/// セルの左上に役割を示す小さなタグ(PREVIEW/PROGRAM)を描画する。
fn paint_corner_tag(painter: &egui::Painter, rect: egui::Rect, text: &str, color: egui::Color32) {
    let font_id = egui::FontId::new(10.0, egui::FontFamily::Proportional);
    let galley = painter.layout_no_wrap(
        text.to_string(),
        font_id,
        egui::Color32::from_black_alpha(230),
    );
    let padding = egui::vec2(6.0, 2.0);
    let tag_rect = egui::Rect::from_min_size(
        rect.left_top() + egui::vec2(8.0, 8.0),
        galley.size() + padding * 2.0,
    );
    painter.rect_filled(tag_rect, 2.0, color);
    painter.galley(
        tag_rect.min + padding,
        galley,
        egui::Color32::from_black_alpha(230),
    );
}
