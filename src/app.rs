use eframe::egui;
use std::collections::HashMap;

use crate::source::SourceManager;
use crate::source::video::traits::VideoSourceId;

pub const INITIAL_WIDTH: usize = 1280;
pub const INITIAL_HEIGHT: usize = 720;

/// 各カメラごとのテクスチャ管理
pub struct CameraTexture {
    pub handle: egui::TextureHandle,
    pub width: u32,
    pub height: u32,
}

/// メインアプリケーション状態
pub struct CameraApp {
    pub source_manager: SourceManager,
    pub inputs: [Option<VideoSourceId>; 8],
    pub source_textures: HashMap<VideoSourceId, CameraTexture>,
    pub selected_source_id: Option<VideoSourceId>,
    pub preview_source_id: Option<VideoSourceId>,
    pub source_errors: HashMap<VideoSourceId, String>,
    pub show_input_settings: bool,
    pub show_labels: bool,
}

impl CameraApp {
    pub fn fit_canvas_size(available: egui::Vec2) -> (usize, usize) {
        let target_aspect = 16.0f32 / 9.0f32;

        let mut width = (available.x - 2.0).max(16.0);
        let mut height = (available.y - 2.0).max(16.0);

        if width / height > target_aspect {
            width = height * target_aspect;
        } else {
            height = width / target_aspect;
        }

        let width_px = ((width.floor() as usize).max(16) / 4) * 4;
        let height_px = ((height.floor() as usize).max(16) / 2) * 2;

        (width_px, height_px)
    }

    pub fn new(_ctx: &egui::Context) -> Self {
        nokhwa::nokhwa_initialize(|_| {});

        let mut source_manager = SourceManager::new();
        let mut inputs = [None; 8];

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

    pub fn capture_all_frames(&mut self, ctx: &egui::Context) {
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

                    let color_image = egui::ColorImage::from_rgb([w, h], &frame_data.pixels);

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
                Ok(None) => {}
                Err(e) => {
                    self.source_errors.insert(source_id, e.to_string());
                }
            }
        }
    }
}

impl eframe::App for CameraApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.capture_all_frames(ctx);

        self.draw_menu(ctx);
        self.draw_errors_window(ctx);
        self.draw_inputs_window(ctx);
        self.draw_multiview(ctx);

        ctx.request_repaint();
    }
}
