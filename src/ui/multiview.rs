use crate::app::OsechiApp;
use crate::source::video::SourceId;
use eframe::egui;

impl OsechiApp {
    /// 映像のPreviewのUI
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

                let bg_rect = egui::Rect::from_min_size(
                    response.rect.min + egui::vec2(x_offset, y_offset),
                    egui::vec2(canvas_width as f32, canvas_height as f32),
                );
                painter.rect_filled(bg_rect, 0.0, egui::Color32::BLACK);

                let draw_cam =
                    |source_id: Option<SourceId>,
                     rect: egui::Rect,
                     label_text: &str,
                     border_override: Option<egui::Color32>| {
                        let mut is_preview = false;
                        let mut is_program = false;

                        if let Some(id) = &source_id {
                            is_preview = Some(id) == self.preview_source_id.as_ref();
                            is_program = Some(id) == self.selected_source_id.as_ref();
                        }

                        let mut stroke_color = egui::Color32::DARK_GRAY;
                        let mut stroke_width = 1.0;

                        if let Some(c) = border_override {
                            stroke_color = c;
                            stroke_width = 3.0;
                        } else if is_program {
                            stroke_color = egui::Color32::RED;
                            stroke_width = 3.0;
                        } else if is_preview {
                            stroke_color = egui::Color32::GREEN;
                            stroke_width = 3.0;
                        }

                        if let Some(id) = &source_id
                            && let Some(active) = self.active_sources.get(id)
                            && let Some(tex) = &active.texture
                        {
                            let img_aspect = tex.width as f32 / tex.height as f32;
                            let target_aspect = 16.0 / 9.0;

                            let mut uv = egui::Rect::from_min_max(
                                egui::pos2(0.0, 0.0),
                                egui::pos2(1.0, 1.0),
                            );

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
                                egui::Stroke::new(3.0, egui::Color32::GREEN),
                                egui::StrokeKind::Inside,
                            );
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
                    };

                let base_pos = response.rect.min + egui::vec2(x_offset, y_offset);

                let preview_rect = egui::Rect::from_min_size(
                    base_pos,
                    egui::vec2(top_view_width as f32, top_height as f32),
                );
                draw_cam(
                    self.preview_source_id.clone(),
                    preview_rect,
                    "Preview",
                    Some(egui::Color32::GREEN),
                );

                let program_rect = egui::Rect::from_min_size(
                    base_pos + egui::vec2(top_view_width as f32, 0.0),
                    egui::vec2(top_view_width as f32, top_height as f32),
                );
                draw_cam(
                    self.selected_source_id.clone(),
                    program_rect,
                    "Program",
                    Some(egui::Color32::RED),
                );

                let mut view_idx = 0;
                let cols = 4;
                let rows = 2;
                let cell_width = canvas_width as f32 / cols as f32;
                let cell_height = bottom_height as f32 / rows as f32;

                for r in 0..rows {
                    for c in 0..cols {
                        let rect = egui::Rect::from_min_size(
                            base_pos
                                + egui::vec2(
                                    c as f32 * cell_width,
                                    top_height as f32 + r as f32 * cell_height,
                                ),
                            egui::vec2(cell_width, cell_height),
                        );

                        let (source_id, source_label) = if view_idx < 8 {
                            (
                                self.inputs[view_idx].clone(),
                                format!("Input {}", view_idx + 1),
                            )
                        } else {
                            (None, String::new())
                        };

                        draw_cam(source_id, rect, &source_label, None);
                        view_idx += 1;
                    }
                }
            });
    }
}
