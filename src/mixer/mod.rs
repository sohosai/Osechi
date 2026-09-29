//! オーディオミキサー。ミキサーに入っている音声ソースを合成し、モニター出力へ流す。
//!
//! 内部の音声は [`SAMPLE_RATE`]Hz・[`CHANNELS`]ch の f32 interleaved に揃える。

mod dsp;
mod output;

use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

pub use dsp::level_to_meter;
pub use output::{Bus, Fanout};

use crate::source::SourceId;
use crate::source::audio::Chunk;

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

/// ソースのどのチャンネルをミキサーの1チャンネルに流すか。チャンネル番号は0始まり。
///
/// 多チャンネルのAES67フローを、1つのフローのままチャンネルごと(またはペアごと)に
/// 別のストリップへ振り分けるのに使う。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum Pick {
    /// 全チャンネル。1ch: L/Rに複製、2ch: そのまま、3ch以上: 平均をL/Rに複製。
    #[default]
    All,
    /// 1チャンネルをL/Rに複製する。
    Mono(u16),
    /// 2チャンネルをそれぞれL/Rにする。
    Stereo(u16, u16),
}

impl Pick {
    /// `channels` チャンネルのソースで選べるもの。
    /// 全チャンネル、各チャンネル単独、隣り合う2チャンネルのペア(1-2, 3-4, ...)の順。
    /// 1ch・2chのソースでは全チャンネルと同じになるものは含めない。
    pub fn options(channels: u16) -> Vec<Self> {
        let mut options = vec![Self::All];
        if channels > 1 {
            options.extend((0..channels).map(Self::Mono));
        }
        if channels > 2 {
            options.extend((0..channels - 1).step_by(2).map(|l| Self::Stereo(l, l + 1)));
        }
        options
    }

    /// `channels` チャンネルのソースに、選んでいるチャンネルが全てあるか。
    pub fn fits(self, channels: u16) -> bool {
        match self {
            Self::All => channels > 0,
            Self::Mono(ch) => ch < channels,
            Self::Stereo(l, r) => l.max(r) < channels,
        }
    }
}

/// UIに出す表記。チャンネル番号は1始まりにする(`Ch 3`, `Ch 1-2`)。
impl fmt::Display for Pick {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::All => f.write_str("All"),
            Self::Mono(ch) => write!(f, "Ch {}", ch + 1),
            Self::Stereo(l, r) => write!(f, "Ch {}-{}", l + 1, r + 1),
        }
    }
}

/// ミキサーの1チャンネル。
#[derive(Debug, Clone)]
pub struct Channel {
    pub source: SourceId,
    /// ソースのどのチャンネルを使うか
    pub pick: Pick,
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
    /// モニター出力へ渡す合成結果
    bus: output::Bus,
    /// 番組出力(配信・録画)へ配る合成結果
    program: Fanout,
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
            program: Fanout::default(),
            monitor: output::Monitor::default(),
            outputs: output::devices(),
        }
    }

    /// `source` の `pick` を流すチャンネルを追加する。同じ組み合わせが既にあれば何もしない。
    /// 同じソースでも `pick` が違えば別のチャンネルとして並べられる。
    pub fn add(&mut self, source: SourceId, pick: Pick) {
        if !self.contains_pick(&source, pick) {
            self.channels.push(Channel {
                source,
                pick,
                strip: Strip::default(),
            });
        }
    }

    /// `index` 番目のチャンネルを外す。
    pub fn remove_at(&mut self, index: usize) {
        if index < self.channels.len() {
            self.channels.remove(index);
        }
    }

    /// `source` を使っているチャンネルを全て外す。
    pub fn remove_source(&mut self, source: &SourceId) {
        self.channels.retain(|channel| channel.source != *source);
    }

    /// `source` を使っているチャンネルが1つでもあるか。
    pub fn contains(&self, source: &SourceId) -> bool {
        self.channels
            .iter()
            .any(|channel| channel.source == *source)
    }

    /// `source` の `pick` を流すチャンネルがあるか。
    pub fn contains_pick(&self, source: &SourceId, pick: Pick) -> bool {
        self.channels
            .iter()
            .any(|channel| channel.source == *source && channel.pick == pick)
    }

    /// ミキサーに入っているソース。同じソースを複数のチャンネルで使っていれば重複して返す。
    pub fn sources(&self) -> impl Iterator<Item = &SourceId> {
        self.channels.iter().map(|channel| &channel.source)
    }

    /// 外部APIと共有するマスターミュートの状態。
    pub fn remote_mute(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.remote_mute)
    }

    /// 番組出力(配信・録画)へ配る合成結果。マスターのフェーダー・ミュートを通した後の音声。
    pub fn program_audio(&self) -> Fanout {
        self.program.clone()
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

    /// 前回から届いた音声(ソースごと)を合成してモニターと番組出力へ送り、メーターを更新する。
    ///
    /// 同じソースを複数のチャンネルで使うことがあるので、呼ぶ側がソースごとに1回だけ取り出して渡し、
    /// ここでチャンネル同士で分け合う(取り出すとキューから消えるため)。
    pub fn process(&mut self, received: &HashMap<SourceId, Vec<Chunk>>) {
        self.sync_remote_mute();

        let mut mix = Vec::new();
        for channel in &mut self.channels {
            // (選んだチャンネルをステレオにしたもの, サンプルレート)
            let picked: Vec<(Vec<f32>, u32)> = received
                .get(&channel.source)
                .into_iter()
                .flatten()
                .map(|chunk| {
                    let stereo = dsp::pick_stereo(&chunk.samples, chunk.channels, channel.pick);
                    (stereo, chunk.sample_rate)
                })
                .collect();
            let peak = picked
                .iter()
                .map(|(stereo, _)| dsp::peak(stereo))
                .reduce(f32::max);
            channel.strip.meter(peak);
            if channel.strip.muted {
                continue;
            }

            let signal: Vec<f32> = picked
                .iter()
                .flat_map(|(stereo, rate)| dsp::resample_stereo(stereo, *rate, SAMPLE_RATE))
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
        self.program.push(&mix);
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
            program: Fanout::default(),
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
        mixer.add(id.clone(), Pick::All);
        mixer.add(id.clone(), Pick::All);
        assert_eq!(mixer.channels.len(), 1);
        mixer.remove_source(&id);
        assert!(!mixer.contains(&id));
    }

    #[test]
    fn same_source_can_be_added_with_different_picks() {
        let mut mixer = mixer();
        let id = SourceId::new("test", 1);
        mixer.add(id.clone(), Pick::Mono(0));
        mixer.add(id.clone(), Pick::Mono(1));
        assert_eq!(mixer.channels.len(), 2);
        assert!(mixer.contains_pick(&id, Pick::Mono(1)));
        assert!(!mixer.contains_pick(&id, Pick::All));

        mixer.remove_at(0);
        assert!(!mixer.contains_pick(&id, Pick::Mono(0)));
        assert!(mixer.contains(&id));
        mixer.remove_at(5);
        assert_eq!(mixer.channels.len(), 1);
    }

    #[test]
    fn pick_options_follow_channel_count() {
        assert_eq!(Pick::options(1), [Pick::All]);
        assert_eq!(Pick::options(2), [Pick::All, Pick::Mono(0), Pick::Mono(1)]);
        assert_eq!(
            Pick::options(3),
            [
                Pick::All,
                Pick::Mono(0),
                Pick::Mono(1),
                Pick::Mono(2),
                Pick::Stereo(0, 1),
            ]
        );
        assert_eq!(Pick::options(8).len(), 1 + 8 + 4);
    }

    #[test]
    fn pick_fits_only_existing_channels() {
        assert!(Pick::Mono(1).fits(2));
        assert!(!Pick::Mono(2).fits(2));
        assert!(Pick::Stereo(6, 7).fits(8));
        assert!(!Pick::Stereo(6, 7).fits(4));
        assert!(Pick::All.fits(1));
        assert!(!Pick::All.fits(0));
    }

    #[test]
    fn pick_labels_are_one_based() {
        assert_eq!(Pick::All.to_string(), "All");
        assert_eq!(Pick::Mono(2).to_string(), "Ch 3");
        assert_eq!(Pick::Stereo(0, 1).to_string(), "Ch 1-2");
    }

    fn levels(mixer: &Mixer) -> Vec<f32> {
        mixer.channels.iter().map(|c| c.strip.level).collect()
    }

    fn assert_levels(mixer: &Mixer, expected: &[f32]) {
        let levels = levels(mixer);
        assert_eq!(levels.len(), expected.len());
        for (level, expected) in levels.iter().zip(expected) {
            assert!((level - expected).abs() < 1e-6, "levels: {levels:?}");
        }
    }

    #[test]
    fn channels_sharing_a_source_each_get_their_pick() {
        let id = SourceId::new("test", "4ch");
        // 4ch x 2フレーム。各チャンネルのピークは 0.1, 0.2, 0.3, 0.4
        let chunk = Chunk {
            samples: vec![0.1, -0.2, 0.3, 0.0, 0.0, 0.2, -0.3, 0.4],
            sample_rate: SAMPLE_RATE,
            channels: 4,
        };

        let mut mixer = mixer();
        for ch in 0..4 {
            mixer.add(id.clone(), Pick::Mono(ch));
        }
        mixer.add(id.clone(), Pick::Stereo(0, 3));
        mixer.process(&HashMap::from([(id, vec![chunk])]));

        assert_levels(&mixer, &[0.1, 0.2, 0.3, 0.4, 0.4]);
    }

    #[test]
    fn channel_missing_from_the_stream_is_silent() {
        let id = SourceId::new("test", "flow");
        let mut mixer = mixer();
        mixer.add(id.clone(), Pick::Mono(1));
        mixer.add(id.clone(), Pick::Mono(5));

        // 途中で8chから2chに減った
        let chunks = vec![
            Chunk {
                samples: vec![0.0, 0.5, 0.0, 0.0, 0.0, 0.8, 0.0, 0.0],
                sample_rate: SAMPLE_RATE,
                channels: 8,
            },
            Chunk {
                samples: vec![0.0, 0.3],
                sample_rate: SAMPLE_RATE,
                channels: 2,
            },
        ];
        mixer.process(&HashMap::from([(id.clone(), chunks)]));
        assert_levels(&mixer, &[0.5, 0.8]);

        mixer.process(&HashMap::from([(
            id,
            vec![Chunk {
                samples: vec![0.0, 0.3],
                sample_rate: SAMPLE_RATE,
                channels: 2,
            }],
        )]));
        // Ch 2は届いた値、Ch 6は無音なのでメーターは下がっていくだけ
        assert_levels(&mixer, &[0.5 * METER_RELEASE, 0.8 * METER_RELEASE]);
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
