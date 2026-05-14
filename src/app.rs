use eframe::egui;
use std::collections::{HashMap, HashSet};

use crate::source::VideoSourceManager;
use crate::source::video::traits::{VideoSource, VideoSourceId};

pub const INITIAL_WIDTH: usize = 1280;
pub const INITIAL_HEIGHT: usize = 720;

/// 各カメラごとのテクスチャ管理
pub struct CameraTexture {
    pub handle: egui::TextureHandle,
    pub width: u32,
    pub height: u32,
}

/// アクティブなカメラの状態（テクスチャとエラーをカプセル化）
pub struct ActiveSource {
    pub source: Option<Box<dyn VideoSource>>,
    pub texture: Option<CameraTexture>,
    pub last_error: Option<String>,
}

///　アプリ全体のステート
pub struct OsechiApp {
    pub video_source_manager: VideoSourceManager,
    pub inputs: [Option<VideoSourceId>; 8],
    pub active_sources: HashMap<VideoSourceId, ActiveSource>,
    pub selected_source_id: Option<VideoSourceId>,
    pub preview_source_id: Option<VideoSourceId>,
    pub show_input_settings: bool,
    pub show_labels: bool,
}

impl OsechiApp {
    /// アプリの初期化関数
    pub fn new(_ctx: &egui::Context) -> Self {
        nokhwa::nokhwa_initialize(|_| {});

        let video_source_manager = VideoSourceManager::new();
        let inputs = [None, None, None, None, None, None, None, None];

        Self {
            video_source_manager,
            inputs,
            active_sources: HashMap::new(),
            selected_source_id: None,
            preview_source_id: None,
            show_input_settings: false,
            show_labels: true,
        }
    }

    /// 現在のウインドウサイズを取得して、画面が崩れないように配置を再計算する関数。
    /// 毎フレーム読んで、UIが崩れないようにする。
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

    pub fn capture_all_frames(&mut self, ctx: &egui::Context) {
        let mut needed_sources = HashSet::new();

        if let Some(id) = &self.preview_source_id {
            needed_sources.insert(id.clone());
        }
        if let Some(id) = &self.selected_source_id {
            needed_sources.insert(id.clone());
        }
        for id in self.inputs.iter().flatten() {
            needed_sources.insert(id.clone());
        }

        self.active_sources
            .retain(|id, _| needed_sources.contains(id));

        for id in &needed_sources {
            if !self.active_sources.contains_key(id) {
                match self.video_source_manager.open_source(id) {
                    Ok(source) => {
                        self.active_sources.insert(
                            id.clone(),
                            ActiveSource {
                                source: Some(source),
                                texture: None,
                                last_error: None,
                            },
                        );
                    }
                    Err(e) => {
                        self.active_sources.insert(
                            id.clone(),
                            ActiveSource {
                                source: None,
                                texture: None,
                                last_error: Some(format!("open failed: {}", e)),
                            },
                        );
                    }
                }
            }
        }

        // アクティブな全ソースからフレームを取得してテクスチャを更新
        for (source_id, active) in self.active_sources.iter_mut() {
            if let Some(source) = &mut active.source {
                match source.get_frame() {
                    Ok(Some(frame_data)) => {
                        active.last_error = None;
                        let w = frame_data.width as usize;
                        let h = frame_data.height as usize;

                        let color_image = egui::ColorImage::from_rgb([w, h], &frame_data.pixels);

                        if let Some(tex) = &mut active.texture {
                            tex.handle.set(color_image, egui::TextureOptions::LINEAR);
                            tex.width = frame_data.width;
                            tex.height = frame_data.height;
                        } else {
                            let safe_name = source_id
                                .as_string()
                                .replace(|c: char| !c.is_alphanumeric(), "_");
                            let name = format!("source_tex_{}", safe_name);
                            let handle =
                                ctx.load_texture(&name, color_image, egui::TextureOptions::LINEAR);
                            active.texture = Some(CameraTexture {
                                handle,
                                width: frame_data.width,
                                height: frame_data.height,
                            });
                        }
                    }
                    Ok(None) => {}
                    Err(e) => {
                        active.last_error = Some(e.to_string());
                    }
                }
            }
        }
    }
}

impl eframe::App for OsechiApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.capture_all_frames(ctx);

        self.draw_menu(ctx);
        self.draw_errors_window(ctx);
        self.draw_inputs_window(ctx);
        self.draw_multiview(ctx);

        ctx.request_repaint();
    }
}
