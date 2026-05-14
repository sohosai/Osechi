mod error;
mod source;

use eframe::egui;
use source::SourceManager;
use source::video::traits::VideoSourceId;
use std::collections::HashMap;

const INITIAL_WIDTH: usize = 1280;
const INITIAL_HEIGHT: usize = 720;

/// 各カメラごとのテクスチャ管理
struct CameraTexture {
    handle: egui::TextureHandle,
    width: u32,
    height: u32,
}

/// メインアプリケーション状態
struct CameraApp {
    /// 映像ソース管理
    source_manager: SourceManager,
    /// 下段8枠の入力ソース
    inputs: [Option<VideoSourceId>; 8],
    /// ソースごとの描画用テクスチャ
    source_textures: HashMap<VideoSourceId, CameraTexture>,
    /// Programに選択中のソースID
    selected_source_id: Option<VideoSourceId>,
    /// Previewに選択中のソースID
    preview_source_id: Option<VideoSourceId>,
    /// ソースごとの最新エラー
    source_errors: HashMap<VideoSourceId, String>,
    /// 入力管理ウィンドウの表示状態
    show_input_settings: bool,
    /// カメラ名などのラベルを表示するかどうか
    show_labels: bool,
}

impl CameraApp {
    /// 利用可能領域に16:9キャンバスを収める
    fn fit_canvas_size(available: egui::Vec2) -> (usize, usize) {
        let target_aspect = 16.0f32 / 9.0f32;

        // 画面ピッタリだとスクロールバーが出現してガタつく原因になるため、2pxの余裕を持たせる
        let mut width = (available.x - 2.0).max(16.0);
        let mut height = (available.y - 2.0).max(16.0);

        if width / height > target_aspect {
            width = height * target_aspect;
        } else {
            height = width / target_aspect;
        }

        // 下段4x2分割・上段2分割で端数が出ないように丸める
        let width_px = ((width.floor() as usize).max(16) / 4) * 4;
        let height_px = ((height.floor() as usize).max(16) / 2) * 2;

        (width_px, height_px)
    }

    /// アプリケーションを初期化
    fn new(_ctx: &egui::Context) -> Self {
        nokhwa::nokhwa_initialize(|_| {});

        let mut source_manager = SourceManager::new();
        let mut inputs = [None; 8];

        // すべてのカメラを割り当て
        let available_sources: Vec<_> = source_manager.available_sources().to_vec();
        let mut source_errors = HashMap::new();
        for (i, source_info) in available_sources.iter().enumerate() {
            if i < 8 {
                match source_manager.open_source(source_info.id) {
                    Ok(_) => {
                        inputs[i] = Some(source_info.id);
                    }
                    Err(e) => {
                        source_errors.insert(source_info.id, format!("open failed: {}", e));
                    }
                }
            }
        }

        let preview_source_id = available_sources.first().map(|s| s.id);
        let selected_source_id = available_sources.get(1).map(|s| s.id).or(preview_source_id);

        if let Some(source_id) = preview_source_id
            && let Err(e) = source_manager.open_source(source_id)
        {
            source_errors.insert(source_id, format!("open failed: {}", e));
        }
        if let Some(source_id) = selected_source_id
            && let Err(e) = source_manager.open_source(source_id)
        {
            source_errors.insert(source_id, format!("open failed: {}", e));
        }

        Self {
            source_manager,
            inputs,
            source_textures: HashMap::new(),
            selected_source_id,
            preview_source_id,
            source_errors,
            show_input_settings: false,
            show_labels: true,
        }
    }

    /// フレームを受信しテクスチャを更新する
    fn capture_all_frames(&mut self, ctx: &egui::Context) {
        // 表示が必要な全ソースIDをリストアップ
        let mut needed_sources = std::collections::HashSet::new();

        if let Some(id) = self.preview_source_id {
            needed_sources.insert(id);
        }
        if let Some(id) = self.selected_source_id {
            needed_sources.insert(id);
        }
        for id in self.inputs.into_iter().flatten() {
            needed_sources.insert(id);
        }

        for source_id in needed_sources {
            match self.source_manager.get_frame(source_id) {
                Ok(Some(frame_data)) => {
                    self.source_errors.remove(&source_id);
                    let w = frame_data.width as usize;
                    let h = frame_data.height as usize;

                    // Arcに入っている生ピクセルデータをeguiのColorImageに変換
                    // Arcによりメモリコピーは発生しない（eguiロード時にRGBA変換される）
                    let color_image = egui::ColorImage::from_rgb([w, h], &frame_data.pixels);

                    // テクスチャを更新（なければ作成）
                    if let Some(tex) = self.source_textures.get_mut(&source_id) {
                        tex.handle.set(color_image, egui::TextureOptions::LINEAR);
                        tex.width = frame_data.width;
                        tex.height = frame_data.height;
                    } else {
                        let name = format!("source_tex_{}", source_id.0);
                        let handle =
                            ctx.load_texture(&name, color_image, egui::TextureOptions::LINEAR);
                        self.source_textures.insert(
                            source_id,
                            CameraTexture {
                                handle,
                                width: frame_data.width,
                                height: frame_data.height,
                            },
                        );
                    }
                }
                Ok(None) => {
                    // まだ新しいフレームが無いので何もしない (ノンブロッキング)
                }
                Err(e) => {
                    self.source_errors.insert(source_id, e.to_string());
                }
            }
        }
    }
}

impl eframe::App for CameraApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // フレームをキャプチャ
        self.capture_all_frames(ctx);

        // ===== トップメニューバー =====
        egui::TopBottomPanel::top("menu_bar").show(ctx, |ui| {
            egui::MenuBar::new().ui(ui, |ui| {
                ui.menu_button("File", |ui| {
                    if ui.button("Exit").clicked() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                });

                ui.menu_button("Settings", |ui| {
                    if ui.button("⚙ Manage Inputs").clicked() {
                        self.show_input_settings = !self.show_input_settings;
                    }
                    ui.separator();
                    ui.checkbox(&mut self.show_labels, "Show Labels");
                });

                ui.menu_button("Sources", |ui| {
                    let source_names: Vec<String> = self
                        .source_manager
                        .available_sources()
                        .iter()
                        .map(|s| s.name.clone())
                        .collect();

                    // Programソースの選択
                    ui.menu_button("Program Source", |ui| {
                        let selected_idx = self
                            .selected_source_id
                            .and_then(|id| {
                                self.source_manager
                                    .available_sources()
                                    .iter()
                                    .position(|s| s.id == id)
                            })
                            .unwrap_or(0);

                        let mut new_selected = selected_idx;
                        for (i, name) in source_names.iter().enumerate() {
                            ui.radio_value(&mut new_selected, i, name);
                        }

                        if new_selected != selected_idx
                            && new_selected < self.source_manager.available_sources().len()
                        {
                            let new_source = &self.source_manager.available_sources()[new_selected];
                            let new_id = new_source.id;
                            match self.source_manager.open_source(new_id) {
                                Ok(_) => {
                                    self.selected_source_id = Some(new_id);
                                    self.source_errors.remove(&new_id);
                                }
                                Err(e) => {
                                    self.source_errors
                                        .insert(new_id, format!("open failed: {}", e));
                                }
                            }
                        }
                    });

                    // Previewソースの選択
                    ui.menu_button("Preview Source", |ui| {
                        let preview_idx = self
                            .preview_source_id
                            .and_then(|id| {
                                self.source_manager
                                    .available_sources()
                                    .iter()
                                    .position(|s| s.id == id)
                            })
                            .unwrap_or(0);

                        let mut new_preview = preview_idx;
                        for (i, name) in source_names.iter().enumerate() {
                            ui.radio_value(&mut new_preview, i, name);
                        }

                        if new_preview != preview_idx
                            && new_preview < self.source_manager.available_sources().len()
                        {
                            let new_source = &self.source_manager.available_sources()[new_preview];
                            let new_id = new_source.id;
                            match self.source_manager.open_source(new_id) {
                                Ok(_) => {
                                    self.preview_source_id = Some(new_id);
                                    self.source_errors.remove(&new_id);
                                }
                                Err(e) => {
                                    self.source_errors
                                        .insert(new_id, format!("open failed: {}", e));
                                }
                            }
                        }
                    });
                });

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(format!(
                        "📹 Sources: {}",
                        self.source_manager.available_sources().len()
                    ));

                    ui.separator();
                });
            });
        });

        // ソースエラーの表示（別ウィンドウで表示）
        if !self.source_errors.is_empty() {
            egui::Window::new("Source Errors")
                .anchor(egui::Align2::RIGHT_BOTTOM, egui::Vec2::new(-10.0, -10.0))
                .collapsible(true)
                .show(ctx, |ui| {
                    for source in self.source_manager.available_sources() {
                        if let Some(err) = self.source_errors.get(&source.id) {
                            ui.colored_label(
                                egui::Color32::RED,
                                format!("{}: {}", source.name, err),
                            );
                        }
                    }
                });
        }

        // ===== 入力を管理 ウィンドウ =====
        let mut show_settings = self.show_input_settings;
        egui::Window::new("Manage Inputs")
            .open(&mut show_settings)
            .resizable(false)
            .collapsible(false)
            .show(ctx, |ui| {
                let available_sources = self.source_manager.available_sources().to_vec();

                for idx in 0..8 {
                    ui.horizontal(|ui| {
                        ui.label(format!("Input {}:", idx + 1));

                        let current_source_id = self.inputs[idx];

                        let selected_text = if let Some(id) = current_source_id {
                            available_sources
                                .iter()
                                .find(|s| s.id == id)
                                .map(|s| s.name.clone())
                                .unwrap_or_else(|| "Unknown".to_string())
                        } else {
                            "None".to_string()
                        };

                        egui::ComboBox::from_id_salt(format!("input_select_{}", idx))
                            .selected_text(selected_text)
                            .show_ui(ui, |ui| {
                                // "None" の選択肢
                                let mut is_none = current_source_id.is_none();
                                if ui.selectable_value(&mut is_none, true, "None").clicked() {
                                    self.inputs[idx] = None;
                                }

                                // 利用可能なソースの選択肢
                                for source in &available_sources {
                                    let mut is_selected = current_source_id == Some(source.id);
                                    if ui
                                        .selectable_value(&mut is_selected, true, &source.name)
                                        .clicked()
                                    {
                                        // 選択されたソースを開く
                                        if let Err(e) = self.source_manager.open_source(source.id) {
                                            self.source_errors
                                                .insert(source.id, format!("open failed: {}", e));
                                        } else {
                                            self.source_errors.remove(&source.id);
                                        }
                                        self.inputs[idx] = Some(source.id);
                                    }
                                }
                            });
                    });
                }
            });
        self.show_input_settings = show_settings;

        // ===== トップパネル（プレビュー＆プログラム）=====
        // CentralPanelの内部余白(margin)を0に設定する
        let frame = egui::Frame::central_panel(&ctx.style()).inner_margin(0.0);
        egui::CentralPanel::default().frame(frame).show(ctx, |ui| {
            let available = ui.available_size();
            let (canvas_width, canvas_height) = Self::fit_canvas_size(available);
            let top_height = canvas_height / 2;
            let bottom_height = canvas_height - top_height;
            let top_view_width = canvas_width / 2;

            // キャンバスを画面中央に配置するための開始座標（オフセット）を計算
            let x_offset = ((available.x - canvas_width as f32) / 2.0).max(0.0);
            let y_offset = ((available.y - canvas_height as f32) / 2.0).max(0.0);

            // eguiの自動レイアウトを無視して、画面全体を自由に描画できる領域(Painter)として確保
            let (response, painter) = ui.allocate_painter(available, egui::Sense::hover());

            // 背景を黒に塗りつぶす（空の入力枠などのため）
            let bg_rect = egui::Rect::from_min_size(
                response.rect.min + egui::vec2(x_offset, y_offset),
                egui::vec2(canvas_width as f32, canvas_height as f32),
            );
            painter.rect_filled(bg_rect, 0.0, egui::Color32::BLACK);

            // 画像のUVと描画をヘルパー関数で処理
            let draw_cam = |source_id: Option<VideoSourceId>,
                            rect: egui::Rect,
                            label_text: &str,
                            border_override: Option<egui::Color32>| {
                let mut is_preview = false;
                let mut is_program = false;

                if let Some(id) = source_id {
                    is_preview = Some(id) == self.preview_source_id;
                    is_program = Some(id) == self.selected_source_id;
                }

                let mut stroke_color = egui::Color32::DARK_GRAY;
                let mut stroke_width = 1.0;

                if let Some(c) = border_override {
                    stroke_color = c;
                    stroke_width = 3.0;
                } else if is_program {
                    stroke_color = egui::Color32::RED;
                    stroke_width = 3.0; // プログラムは赤枠
                } else if is_preview {
                    stroke_color = egui::Color32::GREEN;
                    stroke_width = 3.0; // プレビューは緑枠
                }

                if let Some(id) = source_id {
                    // テクスチャがあれば取得
                    if let Some(tex) = self.source_textures.get(&id) {
                        let img_aspect = tex.width as f32 / tex.height as f32;
                        let target_aspect = 16.0 / 9.0;

                        let mut uv =
                            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));

                        // 画像が縦長の場合（上下をカット）
                        if img_aspect < target_aspect {
                            let crop_ratio = img_aspect / target_aspect;
                            let offset = (1.0 - crop_ratio) / 2.0;
                            uv = egui::Rect::from_min_max(
                                egui::pos2(0.0, offset),
                                egui::pos2(1.0, 1.0 - offset),
                            );
                        }
                        // 画像が横長の場合（左右をカット）
                        else if img_aspect > target_aspect {
                            let crop_ratio = target_aspect / img_aspect;
                            let offset = (1.0 - crop_ratio) / 2.0;
                            uv = egui::Rect::from_min_max(
                                egui::pos2(offset, 0.0),
                                egui::pos2(1.0 - offset, 1.0),
                            );
                        }

                        painter.image(tex.handle.id(), rect, uv, egui::Color32::WHITE);
                    }
                }

                // 枠のボーダーを画像の上に描画
                painter.rect_stroke(
                    rect.shrink(stroke_width / 2.0),
                    0.0,
                    egui::Stroke::new(stroke_width, stroke_color),
                    egui::StrokeKind::Inside,
                );

                // もし「マルチビュー」で、かつプレビュー・プログラム両方に選ばれているなら
                // 赤枠の内側にさらに緑枠を書いて、両方選ばれていることが分かるようにする
                if border_override.is_none() && is_program && is_preview {
                    painter.rect_stroke(
                        rect.shrink(3.0 + 3.0 / 2.0),
                        0.0,
                        egui::Stroke::new(3.0, egui::Color32::GREEN),
                        egui::StrokeKind::Inside,
                    );
                }

                // 文字ラベルの描画
                if self.show_labels && !label_text.is_empty() {
                    let text_color = egui::Color32::WHITE;
                    let bg_color = egui::Color32::from_black_alpha(160); // 半透明の黒
                    let font_id = egui::FontId::proportional(16.0);

                    // フォントのレイアウトを計算するため、一時的に galley を作成
                    let galley =
                        painter.layout_no_wrap(label_text.to_string(), font_id, text_color);

                    let text_size = galley.size();
                    // 中央下部に配置。枠線から離して少し上(8px)に浮かせる
                    let text_pos = egui::pos2(
                        rect.center().x - text_size.x / 2.0,
                        rect.max.y - text_size.y - 8.0,
                    );

                    // 背景の矩形をテキストより少し大きめに描画（パディング2px）
                    let bg_rect = egui::Rect::from_min_size(
                        text_pos - egui::vec2(6.0, 2.0),
                        text_size + egui::vec2(12.0, 4.0),
                    );

                    painter.rect_filled(bg_rect, 4.0, bg_color); // 角丸4px
                    painter.galley(text_pos, galley, egui::Color32::WHITE);
                }
            };

            let base_pos = response.rect.min + egui::vec2(x_offset, y_offset);

            // ① プレビュー（左上）
            let preview_rect = egui::Rect::from_min_size(
                base_pos,
                egui::vec2(top_view_width as f32, top_height as f32),
            );
            draw_cam(
                self.preview_source_id,
                preview_rect,
                "Preview",
                Some(egui::Color32::GREEN),
            );

            // ② プログラム（右上）
            let program_rect = egui::Rect::from_min_size(
                base_pos + egui::vec2(top_view_width as f32, 0.0),
                egui::vec2(top_view_width as f32, top_height as f32),
            );
            draw_cam(
                self.selected_source_id,
                program_rect,
                "Program",
                Some(egui::Color32::RED),
            );

            // ③ マルチビュー（下段 4x2）
            let mut view_idx = 0;
            // self.layout_config.input_rows: usize, columns: usize があれば良いが
            // layout::LayoutConfig の仕様が不明な場合決め打ちでもOK
            // LayoutConfigに依存せず、常に 4x2 として描画
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

                    // レイアウト上のソースIDを取得
                    let (source_id, source_label) = if view_idx < 8 {
                        (self.inputs[view_idx], format!("Input {}", view_idx + 1))
                    } else {
                        (None, String::new())
                    };

                    draw_cam(source_id, rect, &source_label, None);
                    view_idx += 1;
                }
            }
        });

        // アニメーションを維持するために再描画リクエスト
        ctx.request_repaint();
    }
}

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([INITIAL_WIDTH as f32, INITIAL_HEIGHT as f32 + 40.0]),
        ..Default::default()
    };

    eframe::run_native(
        "Oseti Beta - OBS-style Multi-View",
        options,
        Box::new(|cc| Ok(Box::new(CameraApp::new(&cc.egui_ctx)))),
    )
}
