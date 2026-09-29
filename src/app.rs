//! アプリ全体の状態と、処理の順序。
//!
//! 状態([`App`])はUIスレッドとエンジンスレッドで共有する。ソースの開閉・音声の合成・配信や録画への
//! 受け渡しはエンジンスレッドが一定間隔で進めるので、ウインドウの最小化やドラッグで描画が止まっても
//! 音声・配信・録画は止まらない。UIスレッドは描画の間だけ状態をロックする。

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::thread;
use std::time::{Duration, Instant};

use eframe::egui;

use crate::config::Config;
use crate::mixer::{Mixer, Pick};
use crate::output::{Taps, record, rtmp};
use crate::source::audio::{self, aes67};
use crate::source::{Catalog, Live, Origin, SourceId, video};
use crate::switcher::{Slot, Switcher};
use crate::ui;

/// エンジンスレッドが [`App::tick`] を呼ぶ間隔。
///
/// AES67は1msごとにチャンクが届き、ソースごとに64個まで溜められる(`audio::FEED_CAPACITY`)ので、
/// それより十分短くする。
const TICK_INTERVAL: Duration = Duration::from_millis(10);

/// 開いている映像ソースの最新フレーム。
#[derive(Clone)]
pub struct Latest {
    pub frame: Arc<video::Frame>,
    /// 届くたびに増える番号。UIがテクスチャを更新すべきかの判定に使う。
    pub serial: u64,
}

pub struct App {
    /// 利用可能な映像ソース
    pub video: Catalog<video::Kind>,
    /// 利用可能な音声ソース
    pub audio: Catalog<audio::Kind>,
    /// 開いている映像ソース(スイッチャーに割り当てられているもの)
    pub frames: Live<video::Frame>,
    /// 開いている音声ソース(ミキサーに入っているもの・ソース一覧でチャンネルを広げているもの)
    pub chunks: Live<audio::Chunk>,
    /// 開いている音声ソースごとの、最後に届いた音声のチャンネル数。
    /// AES67フローはチャンネル数が送り手の設定次第で変わるので、実際に届いた値でチャンネルを振り分ける。
    channel_counts: HashMap<SourceId, u16>,
    /// 開いている映像ソースごとの最新フレーム
    pub latest: HashMap<SourceId, Latest>,
    /// UIに表示するテクスチャと、その元になった [`Latest::serial`]
    pub textures: HashMap<SourceId, (u64, egui::TextureHandle)>,
    pub switcher: Switcher,
    pub mixer: Mixer,
    /// 配信・録画へ渡すスロットの映像とマスター音声
    pub taps: Taps,
    /// RTMP配信
    pub rtmp: rtmp::Rtmp,
    /// 録画
    pub recorder: record::Recorder,
    pub ui: ui::State,
    /// SAPによるAES67フローの自動検出(SAPのポートが使えなければ無効)
    discovery: Option<aes67::Discovery>,
    next_serial: u64,
}

impl App {
    pub fn new(ctx: &egui::Context, config: Config) -> Self {
        ui::theme::install(ctx);

        let mixer = Mixer::new();
        let taps = Taps::new(mixer.program_audio());
        let mut ui = ui::State::default();
        if let Some(url) = config.rtmp_url {
            ui.rtmp.url = url;
        }

        let mut app = Self {
            video: Catalog::default(),
            audio: Catalog::default(),
            frames: Live::default(),
            chunks: Live::default(),
            channel_counts: HashMap::new(),
            latest: HashMap::new(),
            textures: HashMap::new(),
            switcher: Switcher::default(),
            mixer,
            taps,
            rtmp: rtmp::Rtmp::default(),
            recorder: record::Recorder::default(),
            ui,
            discovery: aes67::Discovery::start(),
            next_serial: 0,
        };
        app.rescan();

        for (name, flow) in config.aes67_flows {
            let source = flow.into_source(&name, Origin::Manual);
            app.mixer.add(source.id.clone(), Pick::All);
            app.audio.insert(source);
        }
        if config.demo_mixer
            && let Some(device) = app
                .audio
                .iter()
                .find(|source| matches!(source.kind, audio::Kind::Device(_)))
        {
            app.mixer.add(device.id.clone(), Pick::All);
        }

        crate::api::spawn(app.mixer.remote_mute());
        app
    }

    /// 接続されているデバイスを探し直す。手動追加・自動検出したソースはそのまま残る。
    pub fn rescan(&mut self) {
        self.video.sync(Origin::Scanned, video::scan());
        self.audio.sync(Origin::Scanned, audio::scan());
    }

    /// 入出力を1回分進める。エンジンスレッドが [`TICK_INTERVAL`] ごとに呼ぶ。
    /// ソース一覧の更新 → 使われているソースの開閉 → 映像の取り込み → 音声の合成 → 配信・録画への受け渡しの順。
    fn tick(&mut self) {
        if let Some(discovery) = &mut self.discovery {
            self.audio.sync(Origin::Discovered, discovery.sources());
        }
        self.frames.sync(&self.video, self.switcher.sources());
        self.chunks.sync(
            &self.audio,
            self.mixer.sources().chain(&self.ui.expanded_audio),
        );
        self.collect_frames();

        let received = self.chunks.drain_all();
        self.channel_counts
            .retain(|id, _| received.contains_key(id));
        for (id, chunks) in &received {
            if let Some(chunk) = chunks.last() {
                self.channel_counts.insert(id.clone(), chunk.channels);
            }
        }
        self.mixer.process(&received);

        self.taps.set_frames(Slot::all().filter_map(|slot| {
            let latest = self.latest.get(self.switcher[slot].as_ref()?)?;
            Some((slot, Arc::clone(&latest.frame)))
        }));
    }

    /// 音声ソースのチャンネル数。開いていて音声が届いていればその値、まだならソースの設定から分かる値
    /// (AES67フローの設定のチャンネル数)。どちらも無ければ(開いていない入力デバイスなど) `None`。
    pub fn channel_count(&self, id: &SourceId) -> Option<u16> {
        self.received_channel_count(id)
            .or_else(|| self.audio.get(id)?.kind.channels())
    }

    /// 開いている音声ソースに実際に届いている音声のチャンネル数。まだ届いていなければ `None`。
    pub fn received_channel_count(&self, id: &SourceId) -> Option<u16> {
        self.channel_counts.get(id).copied()
    }

    /// 開いている映像ソースごとに、届いた最新のフレームを取っておく。
    fn collect_frames(&mut self) {
        self.latest.retain(|id, _| self.frames.contains(id));
        for id in self.frames.ids() {
            if let Some(frame) = self.frames.drain(id).last() {
                self.next_serial += 1;
                let latest = Latest {
                    frame: Arc::new(frame),
                    serial: self.next_serial,
                };
                self.latest.insert(id.clone(), latest);
            }
        }
    }

    /// 最新フレームが変わったソースのテクスチャを更新する。UIスレッドから呼ぶ。
    fn upload_textures(&mut self, ctx: &egui::Context) {
        self.textures.retain(|id, _| self.latest.contains_key(id));
        for (id, latest) in &self.latest {
            if self
                .textures
                .get(id)
                .is_some_and(|(serial, _)| *serial == latest.serial)
            {
                continue;
            }
            let frame = &latest.frame;
            let size = [frame.width() as usize, frame.height() as usize];
            let image = egui::ColorImage::from_rgb(size, frame.as_raw());
            match self.textures.get_mut(id) {
                Some((serial, texture)) => {
                    texture.set(image, egui::TextureOptions::LINEAR);
                    *serial = latest.serial;
                }
                None => {
                    let texture = ctx.load_texture(
                        format!("source:{id}"),
                        image,
                        egui::TextureOptions::LINEAR,
                    );
                    self.textures.insert(id.clone(), (latest.serial, texture));
                }
            }
        }
    }
}

fn lock(app: &Mutex<App>) -> MutexGuard<'_, App> {
    app.lock().unwrap_or_else(PoisonError::into_inner)
}

/// eframeから呼ばれる窓口。状態を共有し、エンジンスレッドを動かす。
pub struct Shell {
    app: Arc<Mutex<App>>,
}

impl Shell {
    pub fn new(app: App) -> Self {
        let app = Arc::new(Mutex::new(app));
        let weak = Arc::downgrade(&app);
        thread::Builder::new()
            .name("engine".to_string())
            .spawn(move || run_engine(weak))
            .expect("failed to spawn engine thread");
        Self { app }
    }
}

/// [`TICK_INTERVAL`] ごとに [`App::tick`] を呼び続ける。状態が破棄されたら終わる。
fn run_engine(app: Weak<Mutex<App>>) {
    let mut next = Instant::now();
    loop {
        let Some(app) = app.upgrade() else {
            return;
        };
        lock(&app).tick();
        drop(app);

        next += TICK_INTERVAL;
        let now = Instant::now();
        match next.checked_duration_since(now) {
            Some(wait) => thread::sleep(wait),
            // 大きく遅れたら追いつこうとせず、今から数え直す
            None => next = now,
        }
    }
}

/// 終了時は、録画中のファイルを仕上げ終えるまで待つ(途中で終わると普通の .mov にならない)。
impl Drop for Shell {
    fn drop(&mut self) {
        lock(&self.app).recorder.finish();
    }
}

impl eframe::App for Shell {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let mut app = lock(&self.app);
        app.upload_textures(ui.ctx());
        ui::show(&mut app, ui);
        ui.ctx().request_repaint();
    }
}
