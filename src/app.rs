//! アプリ全体の状態と、1フレームごとの処理の順序。

use std::collections::HashMap;

use eframe::egui;

use crate::config::Config;
use crate::mixer::Mixer;
use crate::source::audio::{self, aes67};
use crate::source::{Catalog, Live, Origin, SourceId, video};
use crate::switcher::Switcher;
use crate::ui;

pub struct App {
    /// 利用可能な映像ソース
    pub video: Catalog<video::Kind>,
    /// 利用可能な音声ソース
    pub audio: Catalog<audio::Kind>,
    /// 開いている映像ソース(スイッチャーに割り当てられているもの)
    pub frames: Live<video::Frame>,
    /// 開いている音声ソース(ミキサーに入っているもの)
    pub chunks: Live<audio::Chunk>,
    /// 開いている映像ソースの最新フレーム
    pub textures: HashMap<SourceId, egui::TextureHandle>,
    pub switcher: Switcher,
    pub mixer: Mixer,
    pub ui: ui::State,
    /// SAPによるAES67フローの自動検出(SAPのポートが使えなければ無効)
    discovery: Option<aes67::Discovery>,
}

impl App {
    pub fn new(ctx: &egui::Context, config: Config) -> Self {
        ui::theme::install(ctx);

        let mut app = Self {
            video: Catalog::default(),
            audio: Catalog::default(),
            frames: Live::default(),
            chunks: Live::default(),
            textures: HashMap::new(),
            switcher: Switcher::default(),
            mixer: Mixer::new(),
            ui: ui::State::default(),
            discovery: aes67::Discovery::start(),
        };
        app.rescan();

        for (name, flow) in config.aes67_flows {
            let source = flow.into_source(&name, Origin::Manual);
            app.mixer.add(source.id.clone());
            app.audio.insert(source);
        }
        if config.demo_mixer
            && let Some(device) = app
                .audio
                .iter()
                .find(|source| matches!(source.kind, audio::Kind::Device(_)))
        {
            app.mixer.add(device.id.clone());
        }

        crate::api::spawn(app.mixer.remote_mute());
        app
    }

    /// 接続されているデバイスを探し直す。手動追加・自動検出したソースはそのまま残る。
    pub fn rescan(&mut self) {
        self.video.sync(Origin::Scanned, video::scan());
        self.audio.sync(Origin::Scanned, audio::scan());
    }

    /// 1フレーム分の入出力を進める。
    /// ソース一覧の更新 → 使われているソースの開閉 → 映像の取り込み → 音声の合成の順。
    fn tick(&mut self, ctx: &egui::Context) {
        if let Some(discovery) = &mut self.discovery {
            self.audio.sync(Origin::Discovered, discovery.sources());
        }
        self.frames.sync(&self.video, self.switcher.sources());
        self.chunks.sync(&self.audio, self.mixer.sources());
        self.upload_frames(ctx);
        self.mixer.process(&self.chunks);
    }

    /// 開いている映像ソースごとに、届いた最新のフレームをテクスチャにする。
    fn upload_frames(&mut self, ctx: &egui::Context) {
        self.textures.retain(|id, _| self.frames.contains(id));
        for id in self.frames.ids() {
            let Some(frame) = self.frames.drain(id).last() else {
                continue;
            };
            let size = [frame.width() as usize, frame.height() as usize];
            let image = egui::ColorImage::from_rgb(size, frame.as_raw());
            match self.textures.get_mut(id) {
                Some(texture) => texture.set(image, egui::TextureOptions::LINEAR),
                None => {
                    let texture = ctx.load_texture(
                        format!("source:{id}"),
                        image,
                        egui::TextureOptions::LINEAR,
                    );
                    self.textures.insert(id.clone(), texture);
                }
            }
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.tick(ui.ctx());
        ui::show(self, ui);
        ui.ctx().request_repaint();
    }
}
