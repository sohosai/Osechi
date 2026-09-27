//! 配信・録画で共通のエンコード処理。
//!
//! どの出力も、スロットの映像を一定のフレームレート([`FRAME_RATE`])で取り出して H.264 にし、
//! 同じ時刻までのマスター音声を取り出す。時刻は実時間ではなくフレーム番号で数えるので、
//! 映像と音声は常に揃い、タイムスタンプは一定間隔になる。

use std::fmt;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use super::convert::I420;
use super::encode::{EncodedVideo, VideoEncoder};
use super::{FRAME_RATE, Taps};
use crate::error::Result;
use crate::mixer::{Bus, CHANNELS, SAMPLE_RATE};
use crate::source::video::Frame;
use crate::switcher::Slot;

/// キーフレームの間隔(秒)。edamame のセグメント長(6秒)を割り切る値にする。録画ではファイルへ書き出す単位になる。
pub const KEYFRAME_INTERVAL_SECS: u64 = 2;
/// 映像の1フレームあたりの音声のフレーム数。
pub const AUDIO_FRAMES_PER_VIDEO_FRAME: u64 = SAMPLE_RATE as u64 / FRAME_RATE as u64;
/// この数のフレームより遅れたら、追いつくのを諦めて飛ばす。
const MAX_LAG_FRAMES: u64 = 3;
/// 音声の揺らぎを吸収するために溜めておく量(フレーム数)。約60ms。
const AUDIO_CUSHION: u64 = SAMPLE_RATE as u64 * 60 / 1000;
/// 溜まった音声がこれ(フレーム数)を超えたら [`AUDIO_CUSHION`] まで捨てて遅延を詰める。約200ms。
const AUDIO_BACKLOG_LIMIT: u64 = SAMPLE_RATE as u64 * 200 / 1000;

/// [`FRAME_RATE`] の間隔でフレーム番号を刻む時計。
pub struct Clock {
    start: Instant,
    next: u64,
    /// 間に合わずに飛ばしたフレームの累計
    pub dropped: u64,
}

impl Clock {
    /// `start` の時刻を0番目のフレームとする。複数の出力で番号を揃えるときは同じ `start` を渡す。
    pub fn starting_at(start: Instant) -> Self {
        Self {
            start,
            next: 0,
            dropped: 0,
        }
    }

    /// 次のフレームの時刻まで待ち、その番号を返す。大きく遅れていたら今に追いつくよう番号を飛ばす。
    pub fn tick(&mut self) -> u64 {
        let interval = Duration::from_secs(1) / FRAME_RATE;
        let due = self.start + interval * self.next as u32;
        if let Some(wait) = due.checked_duration_since(Instant::now()) {
            thread::sleep(wait);
        }
        let current = (self.start.elapsed().as_nanos() / interval.as_nanos()) as u64;
        if current > self.next + MAX_LAG_FRAMES {
            self.dropped += current - self.next;
            self.next = current;
        }
        self.next += 1;
        self.next - 1
    }
}

/// 出力する映像の大きさ。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resolution {
    /// 始めたときのソースの大きさのまま(ソースが無ければ1080p)
    Source,
    Hd1080,
    Hd720,
}

impl Resolution {
    pub const ALL: [Self; 3] = [Self::Source, Self::Hd1080, Self::Hd720];

    /// 実際の大きさ(幅, 高さ)。I420 にするため偶数に丸める。
    pub fn size(self, frame: Option<&Frame>) -> (usize, usize) {
        match (self, frame) {
            (Self::Source, Some(frame)) => (
                (frame.width() as usize / 2 * 2).max(2),
                (frame.height() as usize / 2 * 2).max(2),
            ),
            (Self::Source | Self::Hd1080, _) => (1920, 1080),
            (Self::Hd720, _) => (1280, 720),
        }
    }
}

impl fmt::Display for Resolution {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Source => "Source",
            Self::Hd1080 => "1080p",
            Self::Hd720 => "720p",
        })
    }
}

/// スロットの映像を取り出して H.264 にする。
pub struct VideoPipeline {
    taps: Taps,
    slot: Slot,
    encoder: VideoEncoder,
    image: I420,
    /// `image` の元になったフレーム(同じなら変換を省く)
    shown: Option<Arc<Frame>>,
    /// 最後にキーフレームを入れたGOPの番号
    keyframe_group: Option<u64>,
}

impl VideoPipeline {
    /// `slot` の映像を `size` の大きさ・`bitrate`(bps)でエンコードする。
    pub fn new(
        taps: Taps,
        slot: Slot,
        (width, height): (usize, usize),
        bitrate: u32,
    ) -> Result<Self> {
        Ok(Self {
            taps,
            slot,
            encoder: VideoEncoder::new(bitrate, FRAME_RATE)?,
            image: I420::new(width, height),
            shown: None,
            keyframe_group: None,
        })
    }

    /// 出力の大きさ(幅, 高さ)。
    pub fn size(&self) -> (usize, usize) {
        (self.image.width(), self.image.height())
    }

    /// `index` 番目のフレームをエンコードする。キーフレームは [`KEYFRAME_INTERVAL_SECS`] ごとの位置に入れる。
    pub fn encode(&mut self, index: u64) -> Result<EncodedVideo> {
        let frame = self.taps.frame(self.slot);
        let changed = match (&frame, &self.shown) {
            (Some(new), Some(old)) => !Arc::ptr_eq(new, old),
            (None, None) => false,
            _ => true,
        };
        if changed {
            match &frame {
                Some(frame) => self.image.fill_from(frame),
                None => self.image.fill_black(),
            }
            self.shown = frame;
        }

        let group = index / (KEYFRAME_INTERVAL_SECS * u64::from(FRAME_RATE));
        let encoded = self
            .encoder
            .encode(&self.image, self.keyframe_group != Some(group))?;
        if encoded.keyframe {
            self.keyframe_group = Some(group);
        }
        Ok(encoded)
    }
}

/// マスター音声を、映像のフレーム番号に合わせて取り出す。
pub struct AudioPipeline {
    bus: Bus,
    /// 取り出し終えた音声のフレーム数(先頭の無音を含む)
    position: u64,
    /// 最初に返す無音(揺らぎを吸収するための溜め)のフレーム数
    silence: u64,
    /// 音声が足りず無音で埋めた回数
    pub gaps: u64,
}

impl AudioPipeline {
    /// 音声を受け取り始める。
    pub fn new(taps: &Taps) -> Self {
        Self {
            bus: taps.subscribe_audio(),
            position: AUDIO_CUSHION,
            silence: AUDIO_CUSHION,
            gaps: 0,
        }
    }

    /// 映像の `frames` 枚目の時刻までの音声(interleaved)を取り出す。
    ///
    /// 最初は約60ms分の無音から始め、その分だけ溜まった音声から取り出すので、ミキサーが積む間隔の
    /// 揺らぎで途切れない。溜まりすぎたら捨てて遅延を詰める。
    pub fn take(&mut self, frames: u64) -> Vec<f32> {
        let channels = u64::from(CHANNELS);
        let need = (frames * AUDIO_FRAMES_PER_VIDEO_FRAME).saturating_sub(self.position);
        let buffered = self.bus.len() as u64 / channels;
        if buffered > need + AUDIO_BACKLOG_LIMIT {
            self.bus
                .skip(((buffered - need - AUDIO_CUSHION) * channels) as usize);
        } else if buffered < need {
            self.gaps += 1;
        }

        let silence = std::mem::take(&mut self.silence);
        let mut samples = vec![0.0; ((silence + need) * channels) as usize];
        self.bus.pull(&mut samples[(silence * channels) as usize..]);
        self.position += need;
        samples
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolution_follows_source_size() {
        let frame = Frame::new(1281, 721);
        assert_eq!(Resolution::Source.size(Some(&frame)), (1280, 720));
        assert_eq!(Resolution::Source.size(None), (1920, 1080));
        assert_eq!(Resolution::Hd720.size(Some(&frame)), (1280, 720));
        assert_eq!(Resolution::Hd1080.size(None), (1920, 1080));
    }

    #[test]
    fn keyframes_follow_the_fixed_interval() {
        let mut video =
            VideoPipeline::new(Taps::default(), Slot::Program, (64, 36), 500_000).unwrap();
        let keyframes: Vec<u64> = (0..130)
            .filter(|&index| video.encode(index).unwrap().keyframe)
            .collect();
        assert_eq!(keyframes, [0, 60, 120]);
    }

    #[test]
    fn audio_starts_with_cushion_and_keeps_pace() {
        let taps = Taps::default();
        let mut audio = AudioPipeline::new(&taps);
        let channels = CHANNELS as usize;
        // 最初の1フレーム分は溜めの無音だけで足りる
        assert_eq!(audio.take(1).len(), AUDIO_CUSHION as usize * channels);
        // 以後は1フレームにつき 1600 フレーム分ずつ出てくる
        let total: usize = (2..=90).map(|frames| audio.take(frames).len()).sum();
        assert_eq!(
            AUDIO_CUSHION as usize * channels + total,
            90 * AUDIO_FRAMES_PER_VIDEO_FRAME as usize * channels
        );
    }
}
