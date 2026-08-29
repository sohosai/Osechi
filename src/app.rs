use eframe::egui;
use std::collections::{HashMap, HashSet};

use crate::mixer;
use crate::source::audio;
use crate::source::video;

pub const INITIAL_WIDTH: usize = 1280;
pub const INITIAL_HEIGHT: usize = 720;

/// ドラッグ&ドロップでSourcesパネルからやり取りされるペイロード
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DragPayload {
    Video(video::SourceId),
    Audio(audio::SourceId),
}

/// 各カメラごとのテクスチャ管理
pub struct CameraTexture {
    pub handle: egui::TextureHandle,
    pub width: u32,
    pub height: u32,
}

/// アクティブな映像ソースの状態（ストリームとテクスチャをカプセル化）
pub struct ActiveSource {
    pub stream: Option<Box<dyn video::Stream>>,
    pub texture: Option<CameraTexture>,
    pub last_error: Option<String>,
}

/// アクティブな音声ソースの状態(ミキサーに追加されている間だけ開かれる)
pub struct ActiveAudioSource {
    pub stream: Option<Box<dyn audio::Stream>>,
    pub last_error: Option<String>,
}

/// オーディオミキサーの1チャンネル
pub struct MixerChannel {
    pub source_id: audio::SourceId,
    /// フェーダー位置(0.0-1.0)。dB表示に変換して使う。
    pub gain: f32,
    pub muted: bool,
    /// レベルメーター用に平滑化された現在の音量(0.0-1.0)
    pub level: f32,
}

///　アプリ全体のステート
pub struct OsechiApp {
    pub video_source_manager: video::manager::SourceManager,
    pub audio_source_manager: audio::manager::SourceManager,
    pub inputs: [Option<video::SourceId>; 8],
    pub active_sources: HashMap<video::SourceId, ActiveSource>,
    pub selected_source_id: Option<video::SourceId>,
    pub preview_source_id: Option<video::SourceId>,
    pub active_audio_sources: HashMap<audio::SourceId, ActiveAudioSource>,
    pub mixer_channels: Vec<MixerChannel>,
    pub show_labels: bool,
    /// 「Add AES67 Source」フォームが開いている間、入力中の値を保持する。
    pub add_aes67_form: Option<crate::ui::sources_dock::Aes67FormDraft>,

    /// 合成済みのステレオ音声を保持する共有リングバッファ。
    pub mix_bus: mixer::MixBus,
    /// 選択されているモニター出力デバイスのcpalストリームを保持する。
    pub monitor: mixer::MonitorOutput,
    /// 現在選択されているモニター出力デバイス(未選択ならモニターOFF)。
    pub monitor_device_id: Option<cpal::DeviceId>,
    /// 起動時に列挙した音声出力デバイスの一覧(ID・表示名)。
    pub output_devices: Vec<(cpal::DeviceId, String)>,
    /// マスターフェーダー位置(0.0-1.0)とミュート状態。
    pub master_gain: f32,
    pub master_muted: bool,
    /// マスター(最終ミックス)のレベルメーター用に平滑化された音量(0.0-1.0)。
    pub master_level: f32,

    /// 開発用: 指定されていればレイアウト安定後にスクリーンショットを撮り
    /// アプリを終了する(`docs/dev-tools.md` 参照)。通常はNone。
    dev_screenshot: Option<crate::dev::ScreenshotRequester>,
}

impl OsechiApp {
    /// アプリの初期化関数
    pub fn new(ctx: &egui::Context, dev_options: crate::dev::DevOptions) -> Self {
        nokhwa::nokhwa_initialize(|_| {});

        crate::init::install_cjk_fallback(ctx);
        crate::ui::theme::apply(ctx);

        let mut video_source_manager = video::manager::SourceManager::new();
        video_source_manager.scan();

        let mut audio_source_manager = audio::manager::SourceManager::new();
        audio_source_manager.input_device_scan();

        let mut mixer_channels = Vec::new();
        if dev_options.demo_mixer
            && let Some(desc) = audio_source_manager
                .list()
                .iter()
                .find(|d| matches!(d.kind, audio::SourceKind::InputDevice { .. }))
        {
            mixer_channels.push(MixerChannel {
                source_id: desc.id.clone(),
                gain: 0.75,
                muted: false,
                level: 0.0,
            });
        }

        Self {
            video_source_manager,
            audio_source_manager,
            inputs: [None, None, None, None, None, None, None, None],
            active_sources: HashMap::new(),
            selected_source_id: None,
            preview_source_id: None,
            active_audio_sources: HashMap::new(),
            mixer_channels,
            show_labels: true,
            add_aes67_form: None,
            mix_bus: mixer::MixBus::new(),
            monitor: mixer::MonitorOutput::new(),
            monitor_device_id: None,
            output_devices: mixer::scan_output_devices(),
            master_gain: 0.75,
            master_muted: false,
            master_level: 0.0,
            dev_screenshot: dev_options
                .screenshot_path
                .map(crate::dev::ScreenshotRequester::new),
        }
    }

    /// モニター出力デバイスを切り替える。`device_id` が `None` ならモニターを止める。
    pub fn set_monitor_device(&mut self, device_id: Option<cpal::DeviceId>) {
        self.monitor_device_id = device_id.clone();
        let device = device_id.and_then(|id| mixer::output_device_by_id(&id));
        if let Err(e) = self.monitor.set_device(device, self.mix_bus.clone()) {
            tracing::error!("failed to set monitor output device: {e}");
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

    /// 現在この映像ソースが割り当てられている場所を表す短いラベル(PVW/PGM/IN n)を返す
    pub fn video_badge_for(&self, id: &video::SourceId) -> Option<String> {
        if self.preview_source_id.as_ref() == Some(id) {
            return Some("PVW".to_string());
        }
        if self.selected_source_id.as_ref() == Some(id) {
            return Some("PGM".to_string());
        }
        self.inputs
            .iter()
            .position(|slot| slot.as_ref() == Some(id))
            .map(|idx| format!("IN {}", idx + 1))
    }

    /// この音声ソースがミキサーに追加済みかどうかの短いラベルを返す
    pub fn audio_badge_for(&self, id: &audio::SourceId) -> Option<String> {
        self.mixer_channels
            .iter()
            .any(|channel| &channel.source_id == id)
            .then(|| "MIX".to_string())
    }

    pub fn assign_preview(&mut self, id: video::SourceId) {
        self.preview_source_id = Some(id);
    }

    pub fn assign_program(&mut self, id: video::SourceId) {
        self.selected_source_id = Some(id);
    }

    pub fn assign_input(&mut self, idx: usize, id: video::SourceId) {
        self.inputs[idx] = Some(id);
    }

    pub fn clear_input(&mut self, idx: usize) {
        self.inputs[idx] = None;
    }

    /// 音声ソースをミキサーに追加する。既に追加済みなら何もしない。
    pub fn add_mixer_channel(&mut self, id: audio::SourceId) {
        if self.mixer_channels.iter().any(|c| c.source_id == id) {
            return;
        }
        self.mixer_channels.push(MixerChannel {
            source_id: id,
            gain: 0.75,
            muted: false,
            level: 0.0,
        });
    }

    pub fn remove_mixer_channel(&mut self, id: &audio::SourceId) {
        self.mixer_channels.retain(|c| &c.source_id != id);
        self.active_audio_sources.remove(id);
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
                match self.video_source_manager.open(id) {
                    Ok(stream) => {
                        self.active_sources.insert(
                            id.clone(),
                            ActiveSource {
                                stream: Some(stream),
                                texture: None,
                                last_error: None,
                            },
                        );
                    }
                    Err(e) => {
                        self.active_sources.insert(
                            id.clone(),
                            ActiveSource {
                                stream: None,
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
            if let Some(stream) = &mut active.stream {
                match stream.get_frame() {
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
                                .to_string()
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

    /// ミキサーに追加されている音声ソースのストリームを開閉し、
    /// レベルメーター用の音量を更新する。
    pub fn capture_all_audio(&mut self) {
        let needed: HashSet<audio::SourceId> = self
            .mixer_channels
            .iter()
            .map(|c| c.source_id.clone())
            .collect();
        self.active_audio_sources
            .retain(|id, _| needed.contains(id));

        for id in &needed {
            if !self.active_audio_sources.contains_key(id) {
                let active = match self.audio_source_manager.open(id) {
                    Ok(stream) => ActiveAudioSource {
                        stream: Some(stream),
                        last_error: None,
                    },
                    Err(e) => ActiveAudioSource {
                        stream: None,
                        last_error: Some(format!("open failed: {}", e)),
                    },
                };
                self.active_audio_sources.insert(id.clone(), active);
            }
        }

        // このフレームで合成するステレオ音声(48kHz/2ch)。全チャンネル分を
        // 加算してからマスターゲイン・ソフトクリップをかけてmix busへ流す。
        let mut frame_mix: Vec<f32> = Vec::new();

        let active_audio_sources = &mut self.active_audio_sources;
        for channel in self.mixer_channels.iter_mut() {
            let Some(active) = active_audio_sources.get_mut(&channel.source_id) else {
                continue;
            };
            let Some(stream) = &mut active.stream else {
                continue;
            };

            let mut peak: f32 = 0.0;
            let mut received_any = false;
            let mut channel_mix: Vec<f32> = Vec::new();
            let gain = mixer::gain_to_linear(channel.gain);

            loop {
                match stream.get_chunk() {
                    Ok(Some(chunk)) => {
                        received_any = true;
                        active.last_error = None;
                        for &sample in chunk.samples.iter() {
                            peak = peak.max(sample.abs());
                        }

                        // メーターはミュート/フェーダーの影響を受けない(常に
                        // 生の入力レベルを見せる)が、実際の合成はここでスキップする。
                        if !channel.muted {
                            let stereo = mixer::downmix_to_stereo(&chunk.samples, chunk.channels);
                            let resampled = mixer::resample_stereo(
                                &stereo,
                                chunk.sample_rate,
                                mixer::MIX_SAMPLE_RATE,
                            );
                            channel_mix.extend(resampled.iter().map(|s| s * gain));
                        }
                    }
                    Ok(None) => break,
                    Err(e) => {
                        active.last_error = Some(e.to_string());
                        break;
                    }
                }
            }

            // アタック(音量上昇)は即座に反映し、リリース(下降)はなだらかに
            // 減衰させることで、実際の VU メーターに近い動きにする。
            channel.level = if received_any && peak > channel.level {
                peak
            } else {
                channel.level * 0.85
            };
            channel.level = channel.level.clamp(0.0, 1.0);

            if frame_mix.len() < channel_mix.len() {
                frame_mix.resize(channel_mix.len(), 0.0);
            }
            for (dst, src) in frame_mix.iter_mut().zip(channel_mix.iter()) {
                *dst += src;
            }
        }

        if !frame_mix.is_empty() {
            let master_gain = if self.master_muted {
                0.0
            } else {
                mixer::gain_to_linear(self.master_gain)
            };

            let mut master_peak: f32 = 0.0;
            for sample in frame_mix.iter_mut() {
                *sample = mixer::soft_clip(*sample * master_gain);
                master_peak = master_peak.max(sample.abs());
            }

            self.master_level = if master_peak > self.master_level {
                master_peak
            } else {
                self.master_level * 0.85
            };
            self.master_level = self.master_level.clamp(0.0, 1.0);

            self.mix_bus.push_frame(&frame_mix);
        } else {
            self.master_level *= 0.85;
        }
    }
}

impl eframe::App for OsechiApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();

        self.audio_source_manager.sync_sap_discoveries();
        self.capture_all_frames(&ctx);
        self.capture_all_audio();

        self.draw_menu(ui);
        self.draw_errors_window(&ctx);
        self.draw_sources_dock(ui);
        self.draw_audio_mixer_dock(ui);
        self.draw_multiview(ui);

        if let Some(requester) = &mut self.dev_screenshot {
            requester.tick(&ctx);
        }

        ctx.request_repaint();
    }
}
