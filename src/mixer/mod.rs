//! オーディオミキサー。ミキサーに入っている音声ソースを合成し、モニター出力へ流す。
//!
//! 内部の音声は [`SAMPLE_RATE`]Hz・[`CHANNELS`]ch の f32 interleaved に揃える。

mod dsp;
mod output;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

pub use dsp::level_to_meter;

use crate::source::audio::Chunk;
use crate::source::{Live, SourceId};

/// 内部のサンプルレート。AES67の必須レートで、一般的なデバイスの既定値でもある。
pub const SAMPLE_RATE: u32 = 48_000;
/// 内部のチャンネル数(ステレオ)。
pub const CHANNELS: u16 = 2;

/// メーターの下降の速さ(1フレームごとに掛ける係数)。
const METER_RELEASE: f32 = 0.85;

/// フェーダー・ミュート・メーターを持つストリップ。チャンネルとマスターで共通。
#[derive(Debug, Clone, PartialEq)]
pub struct Strip {
    /// フェーダー位置(0.0-1.0)。0.0 が -∞dB、1.0 が 0dB。
    pub gain: f32,
    pub muted: bool,
    /// メーターの値(ピーク振幅, 0.0-1.0)。ミュートやフェーダーの影響を受けない入力レベル。
    pub level: f32,
}

impl Default for Strip {
    /// フェーダーは -15dB から始める。
    fn default() -> Self {
        Self {
            gain: 0.75,
            muted: false,
            level: 0.0,
        }
    }
}

impl Strip {
    /// フェーダー位置のdB値。
    pub fn db(&self) -> f32 {
        dsp::gain_to_db(self.gain)
    }

    /// 信号に掛ける係数。ミュート中は0。
    fn factor(&self) -> f32 {
        if self.muted {
            0.0
        } else {
            dsp::gain_to_linear(self.gain)
        }
    }

    /// このフレームのピーク(届いた音声が無ければ `None`)でメーターを更新する。
    /// 上昇は即座に、下降はなだらかに追従させ、実物のメーターに近い動きにする。
    fn meter(&mut self, peak: Option<f32>) {
        self.level = match peak {
            Some(peak) if peak > self.level => peak,
            _ => self.level * METER_RELEASE,
        }
        .clamp(0.0, 1.0);
    }
}

/// ミキサーの1チャンネル。
#[derive(Debug, Clone)]
pub struct Channel {
    pub source: SourceId,
    pub strip: Strip,
}

/// ミキサー全体。
pub struct Mixer {
    pub channels: Vec<Channel>,
    pub master: Strip,
    /// 外部API(ミュートAPI)と共有するマスターミュート。
    remote_mute: Arc<AtomicBool>,
    /// 前のフレームで `remote_mute` と揃えたときの値。どちらが変えたかの判定に使う。
    remote_mute_seen: bool,
    bus: output::Bus,
    monitor: output::Monitor,
    outputs: Vec<(cpal::DeviceId, String)>,
}

impl Default for Mixer {
    fn default() -> Self {
        Self::new()
    }
}

impl Mixer {
    /// 出力デバイスを列挙して、空のミキサーを作る(モニター出力はOFF)。
    pub fn new() -> Self {
        Self {
            channels: Vec::new(),
            master: Strip::default(),
            remote_mute: Arc::new(AtomicBool::new(false)),
            remote_mute_seen: false,
            bus: output::Bus::default(),
            monitor: output::Monitor::default(),
            outputs: output::devices(),
        }
    }

    /// チャンネルを追加する。既にあれば何もしない。
    pub fn add(&mut self, source: SourceId) {
        if !self.contains(&source) {
            self.channels.push(Channel {
                source,
                strip: Strip::default(),
            });
        }
    }

    pub fn remove(&mut self, source: &SourceId) {
        self.channels.retain(|channel| channel.source != *source);
    }

    pub fn contains(&self, source: &SourceId) -> bool {
        self.channels
            .iter()
            .any(|channel| channel.source == *source)
    }

    /// ミキサーに入っているソース。
    pub fn sources(&self) -> impl Iterator<Item = &SourceId> {
        self.channels.iter().map(|channel| &channel.source)
    }

    /// 外部APIと共有するマスターミュートの状態。
    pub fn remote_mute(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.remote_mute)
    }

    /// 選べる出力デバイス(ID・表示名)。
    pub fn outputs(&self) -> &[(cpal::DeviceId, String)] {
        &self.outputs
    }

    /// いまモニター出力に使っているデバイス。
    pub fn monitor(&self) -> Option<&cpal::DeviceId> {
        self.monitor.device()
    }

    /// モニター出力のデバイスを切り替える。`None` ならモニターを止める。
    pub fn set_monitor(&mut self, device: Option<cpal::DeviceId>) {
        if let Err(err) = self.monitor.set(device, &self.bus) {
            tracing::error!("failed to set monitor output device: {err:#}");
        }
    }

    /// このフレームに届いた音声を合成してモニターへ送り、メーターを更新する。
    pub fn process(&mut self, feeds: &Live<Chunk>) {
        self.sync_remote_mute();

        let mut mix = Vec::new();
        for channel in &mut self.channels {
            let chunks: Vec<Chunk> = feeds.drain(&channel.source).collect();
            let peak = chunks
                .iter()
                .map(|chunk| dsp::peak(&chunk.samples))
                .reduce(f32::max);
            channel.strip.meter(peak);
            if channel.strip.muted {
                continue;
            }

            let signal: Vec<f32> = chunks
                .iter()
                .flat_map(|chunk| {
                    let stereo = dsp::downmix_to_stereo(&chunk.samples, chunk.channels);
                    dsp::resample_stereo(&stereo, chunk.sample_rate, SAMPLE_RATE)
                })
                .collect();
            dsp::add_scaled(&mut mix, &signal, channel.strip.factor());
        }

        if mix.is_empty() {
            self.master.meter(None);
            return;
        }
        let factor = self.master.factor();
        for sample in &mut mix {
            *sample = dsp::soft_clip(*sample * factor);
        }
        self.master.meter(Some(dsp::peak(&mix)));
        self.bus.push(&mix);
    }

    /// UIでのマスターミュート操作と、外部APIからの操作を揃える。
    /// 前回揃えた値から変わった側を採用し、両方が変えていれば外部APIを優先する。
    fn sync_remote_mute(&mut self) {
        if let Err(remote) = self.remote_mute.compare_exchange(
            self.remote_mute_seen,
            self.master.muted,
            Ordering::AcqRel,
            Ordering::Acquire,
        ) {
            self.master.muted = remote;
        }
        self.remote_mute_seen = self.master.muted;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mixer() -> Mixer {
        Mixer {
            channels: Vec::new(),
            master: Strip::default(),
            remote_mute: Arc::new(AtomicBool::new(false)),
            remote_mute_seen: false,
            bus: output::Bus::default(),
            monitor: output::Monitor::default(),
            outputs: Vec::new(),
        }
    }

    #[test]
    fn meter_attacks_instantly_and_releases_slowly() {
        let mut strip = Strip::default();
        strip.meter(Some(0.5));
        assert_eq!(strip.level, 0.5);
        strip.meter(Some(0.1));
        assert!((strip.level - 0.5 * METER_RELEASE).abs() < 1e-6);
        strip.meter(None);
        assert!((strip.level - 0.5 * METER_RELEASE * METER_RELEASE).abs() < 1e-6);
    }

    #[test]
    fn add_ignores_duplicates() {
        let mut mixer = mixer();
        let id = SourceId::new("test", 1);
        mixer.add(id.clone());
        mixer.add(id.clone());
        assert_eq!(mixer.channels.len(), 1);
        mixer.remove(&id);
        assert!(!mixer.contains(&id));
    }

    #[test]
    fn remote_mute_reaches_master() {
        let mut mixer = mixer();
        let remote = mixer.remote_mute();
        remote.store(true, Ordering::Relaxed);
        mixer.sync_remote_mute();
        assert!(mixer.master.muted);
    }

    #[test]
    fn ui_mute_reaches_remote() {
        let mut mixer = mixer();
        mixer.master.muted = true;
        mixer.sync_remote_mute();
        assert!(mixer.remote_mute().load(Ordering::Relaxed));

        mixer.master.muted = false;
        mixer.sync_remote_mute();
        assert!(!mixer.remote_mute().load(Ordering::Relaxed));
    }

    #[test]
    fn remote_unmute_overrides_unchanged_ui() {
        let mut mixer = mixer();
        mixer.master.muted = true;
        mixer.sync_remote_mute();

        mixer.remote_mute().store(false, Ordering::Relaxed);
        mixer.sync_remote_mute();
        assert!(!mixer.master.muted);
    }
}
