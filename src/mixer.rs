use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use cpal::Sample;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

use crate::error::AppError;

/// ミックスバスの内部フォーマット。AES67の必須レートであり、一般的な
/// オーディオデバイスの既定レートでもあるため固定値として採用する。
pub const MIX_SAMPLE_RATE: u32 = 48_000;
pub const MIX_CHANNELS: u16 = 2;

/// ミックスバスに溜め込む上限(約0.5秒分)。出力デバイスが無い/遅い場合に
/// 無限に溜まらないよう、古い方から捨てて遅延を抑える。
const MIX_BUS_MAX_SAMPLES: usize = (MIX_SAMPLE_RATE as usize) * (MIX_CHANNELS as usize) / 2;

/// フェーダー位置(0.0-1.0)を、表示用・音声処理用で共通のdBに変換する。
/// 出力ミキシングとUI表示(`ui/audio_mixer.rs`)の両方から呼ばれる、
/// この変換の唯一の定義。
pub fn gain_to_db(gain: f32) -> f32 {
    if gain <= 0.0 {
        f32::NEG_INFINITY
    } else {
        gain * 60.0 - 60.0
    }
}

/// フェーダー位置(0.0-1.0)を、実際の音量に掛ける線形係数に変換する。
pub fn gain_to_linear(gain: f32) -> f32 {
    let db = gain_to_db(gain);
    if db.is_infinite() {
        0.0
    } else {
        10f32.powf(db / 20.0)
    }
}

/// interleavedの任意チャンネル数の音声をステレオ(L, R)にダウンミックスする。
/// 1ch: 同じ値をL/Rに複製。2ch: そのまま。3ch以上: 平均してL/Rに複製する
/// (複数チャンネルを束ねたAES67フローの簡易フォールバック)。
pub fn downmix_to_stereo(samples: &[f32], channels: u16) -> Vec<f32> {
    if channels == 0 {
        return Vec::new();
    }
    let channels = channels as usize;
    let mut out = Vec::with_capacity((samples.len() / channels) * 2);

    for frame in samples.chunks_exact(channels) {
        match channels {
            1 => {
                out.push(frame[0]);
                out.push(frame[0]);
            }
            2 => {
                out.push(frame[0]);
                out.push(frame[1]);
            }
            _ => {
                let avg = frame.iter().sum::<f32>() / channels as f32;
                out.push(avg);
                out.push(avg);
            }
        }
    }

    out
}

/// interleavedステレオ音声のサンプルレートを変換する。
/// チャンク単位でステートレスな線形補間で行う簡易実装で、チャンク境界を
/// またぐ位相は持ち越さない(詳細はplanのドキュメント参照)。
pub fn resample_stereo(samples: &[f32], from_rate: u32, to_rate: u32) -> Vec<f32> {
    if from_rate == to_rate || samples.is_empty() {
        return samples.to_vec();
    }

    let frames_in = samples.len() / 2;
    if frames_in == 0 {
        return Vec::new();
    }
    let frames_out = ((frames_in as u64) * (to_rate as u64) / (from_rate as u64)).max(1) as usize;
    let ratio = from_rate as f64 / to_rate as f64;

    let mut out = Vec::with_capacity(frames_out * 2);
    for i in 0..frames_out {
        let src_pos = i as f64 * ratio;
        let idx0 = (src_pos.floor() as usize).min(frames_in - 1);
        let idx1 = (idx0 + 1).min(frames_in - 1);
        let frac = (src_pos - idx0 as f64) as f32;

        for ch in 0..2 {
            let a = samples[idx0 * 2 + ch];
            let b = samples[idx1 * 2 + ch];
            out.push(a + (b - a) * frac);
        }
    }

    out
}

/// tanhベースの簡易ソフトクリップ。複数チャンネルを合算した結果が
/// -1.0..=1.0 を超えても、急激な歪みではなく滑らかに飽和させる。
pub fn soft_clip(sample: f32) -> f32 {
    sample.tanh()
}

/// 合成済みのステレオ音声(`MIX_SAMPLE_RATE`Hz / `MIX_CHANNELS`ch interleaved)
/// を保持する共有リングバッファ。`OsechiApp::capture_all_audio`(UIスレッド)
/// が書き込み、出力デバイスのコールバック(別スレッド)が読み出す。
#[derive(Clone)]
pub struct MixBus {
    buffer: Arc<Mutex<VecDeque<f32>>>,
}

impl MixBus {
    pub fn new() -> Self {
        Self {
            buffer: Arc::new(Mutex::new(VecDeque::with_capacity(MIX_BUS_MAX_SAMPLES))),
        }
    }

    /// このフレームで合成したサンプル列を追記する。溜まりすぎている場合は
    /// 古い方から捨てて遅延を抑える。
    pub fn push_frame(&self, samples: &[f32]) {
        let Ok(mut buffer) = self.buffer.lock() else {
            return;
        };
        buffer.extend(samples.iter().copied());
        while buffer.len() > MIX_BUS_MAX_SAMPLES {
            buffer.pop_front();
        }
    }

    /// `out` を必要なサンプル数だけ埋める。足りない分は無音(0.0)で埋める。
    pub fn pull_into(&self, out: &mut [f32]) {
        let Ok(mut buffer) = self.buffer.lock() else {
            out.fill(0.0);
            return;
        };
        for slot in out.iter_mut() {
            *slot = buffer.pop_front().unwrap_or(0.0);
        }
    }
}

impl Default for MixBus {
    fn default() -> Self {
        Self::new()
    }
}

/// 選択されたモニター出力デバイスのcpalストリームを保持する。
pub struct MonitorOutput {
    stream: Option<cpal::Stream>,
}

impl MonitorOutput {
    pub fn new() -> Self {
        Self { stream: None }
    }

    /// 出力先を切り替える。`device` が `None` の場合は出力を止める
    /// (モニターOFF)。
    pub fn set_device(
        &mut self,
        device: Option<cpal::Device>,
        mix_bus: MixBus,
    ) -> Result<(), AppError> {
        // 先に既存のストリームを破棄して再生を止める。
        self.stream = None;

        let Some(device) = device else {
            return Ok(());
        };

        let supported_config = device
            .default_output_config()
            .map_err(|e| AppError::Other(format!("Failed to get default output config: {}", e)))?;
        let sample_format = supported_config.sample_format();
        let config = supported_config.config();
        let device_sample_rate = config.sample_rate;
        let device_channels = config.channels;

        let err_fn = |err| tracing::error!("Audio output stream error: {}", err);

        let stream = match sample_format {
            cpal::SampleFormat::F32 => build_output_stream::<f32>(
                &device,
                &config,
                device_sample_rate,
                device_channels,
                mix_bus,
                err_fn,
                |s| s,
            ),
            cpal::SampleFormat::I16 => build_output_stream::<i16>(
                &device,
                &config,
                device_sample_rate,
                device_channels,
                mix_bus,
                err_fn,
                |s| s.to_sample::<i16>(),
            ),
            cpal::SampleFormat::U16 => build_output_stream::<u16>(
                &device,
                &config,
                device_sample_rate,
                device_channels,
                mix_bus,
                err_fn,
                |s| s.to_sample::<u16>(),
            ),
            other => {
                return Err(AppError::Other(format!(
                    "Unsupported audio output sample format: {:?}",
                    other
                )));
            }
        }?;

        stream
            .play()
            .map_err(|e| AppError::Other(format!("Failed to start audio output stream: {}", e)))?;

        self.stream = Some(stream);
        Ok(())
    }
}

impl Default for MonitorOutput {
    fn default() -> Self {
        Self::new()
    }
}

/// OSの音声出力デバイスを列挙する(デバイスIDと表示名のペア)。
pub fn scan_output_devices() -> Vec<(cpal::DeviceId, String)> {
    let host = cpal::default_host();
    let Ok(devices) = host.output_devices() else {
        return Vec::new();
    };

    devices
        .enumerate()
        .filter_map(|(index, device)| {
            let name = device
                .description()
                .map(|description| description.name().to_string())
                .unwrap_or_else(|_| format!("Unknown Output Device {}", index));
            let id = device.id().ok()?;
            Some((id, name))
        })
        .collect()
}

/// 指定したIDの出力デバイスを取得する。
pub fn output_device_by_id(id: &cpal::DeviceId) -> Option<cpal::Device> {
    cpal::default_host().device_by_id(id)
}

/// `mix_bus` からミックスバスのフォーマットを読み出し、デバイスの実際の
/// サンプルレート/チャンネル数に変換して出力ストリームへ書き込む。
fn build_output_stream<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    device_sample_rate: u32,
    device_channels: u16,
    mix_bus: MixBus,
    err_fn: impl FnMut(cpal::StreamError) + Send + 'static,
    convert_sample: fn(f32) -> T,
) -> Result<cpal::Stream, AppError>
where
    T: cpal::SizedSample + Send + 'static,
{
    let channels_usize = device_channels as usize;
    let mut mix_scratch: Vec<f32> = Vec::new();

    device
        .build_output_stream(
            config,
            move |data: &mut [T], _| {
                if channels_usize == 0 {
                    return;
                }
                let frames = data.len() / channels_usize;
                let mix_frames_needed = ((frames as u64) * (MIX_SAMPLE_RATE as u64)
                    / (device_sample_rate as u64).max(1))
                .max(1) as usize;

                mix_scratch.resize(mix_frames_needed * MIX_CHANNELS as usize, 0.0);
                mix_bus.pull_into(&mut mix_scratch);
                let resampled = resample_stereo(&mix_scratch, MIX_SAMPLE_RATE, device_sample_rate);

                for (i, out_frame) in data.chunks_mut(channels_usize).enumerate() {
                    let l = resampled.get(i * 2).copied().unwrap_or(0.0);
                    let r = resampled.get(i * 2 + 1).copied().unwrap_or(0.0);
                    for (ch, sample) in out_frame.iter_mut().enumerate() {
                        let value = if channels_usize == 1 {
                            (l + r) * 0.5
                        } else if ch == 0 {
                            l
                        } else {
                            r
                        };
                        *sample = convert_sample(value);
                    }
                }
            },
            err_fn,
            None,
        )
        .map_err(|e| AppError::Other(format!("Failed to build audio output stream: {}", e)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gain_zero_is_silent() {
        assert_eq!(gain_to_db(0.0), f32::NEG_INFINITY);
        assert_eq!(gain_to_linear(0.0), 0.0);
    }

    #[test]
    fn gain_one_is_unity() {
        assert!((gain_to_db(1.0) - 0.0).abs() < 1e-4);
        assert!((gain_to_linear(1.0) - 1.0).abs() < 1e-4);
    }

    #[test]
    fn downmix_mono_duplicates_to_both_channels() {
        let out = downmix_to_stereo(&[0.5, -0.25], 1);
        assert_eq!(out, vec![0.5, 0.5, -0.25, -0.25]);
    }

    #[test]
    fn downmix_stereo_passes_through() {
        let out = downmix_to_stereo(&[0.1, 0.2, 0.3, 0.4], 2);
        assert_eq!(out, vec![0.1, 0.2, 0.3, 0.4]);
    }

    #[test]
    fn downmix_multichannel_averages() {
        // 4ch, 1フレーム: 平均は (0.0+1.0+0.0+ -1.0)/4 = 0.0
        let out = downmix_to_stereo(&[0.0, 1.0, 0.0, -1.0], 4);
        assert_eq!(out, vec![0.0, 0.0]);
    }

    #[test]
    fn resample_identity_when_rates_match() {
        let samples = vec![0.1, 0.2, 0.3, 0.4];
        assert_eq!(resample_stereo(&samples, 48_000, 48_000), samples);
    }

    #[test]
    fn resample_doubles_frame_count_when_rate_doubles() {
        // 2フレーム(L,R x2) @ 24000Hz -> 48000Hzなら概ね2倍のフレーム数になる
        let samples = vec![0.0, 0.0, 1.0, 1.0];
        let out = resample_stereo(&samples, 24_000, 48_000);
        assert_eq!(out.len(), 8);
    }

    #[test]
    fn soft_clip_keeps_small_values_almost_unchanged() {
        assert!((soft_clip(0.1) - 0.1).abs() < 0.01);
    }

    #[test]
    fn soft_clip_bounds_large_values() {
        // tanh(10) は理論上 1.0 未満だが f32 精度では 1.0 に丸まりうるため、
        // "範囲を超えない" ことを検証する(境界値は許容する)。
        assert!(soft_clip(10.0) <= 1.0);
        assert!(soft_clip(-10.0) >= -1.0);
        assert!(soft_clip(100.0) <= 1.0);
    }

    #[test]
    fn mix_bus_pulls_back_pushed_samples() {
        let bus = MixBus::new();
        bus.push_frame(&[0.1, 0.2, 0.3, 0.4]);

        let mut out = [0.0f32; 4];
        bus.pull_into(&mut out);
        assert_eq!(out, [0.1, 0.2, 0.3, 0.4]);
    }

    #[test]
    fn mix_bus_pads_with_silence_when_underrun() {
        let bus = MixBus::new();
        bus.push_frame(&[0.5, 0.5]);

        let mut out = [0.0f32; 4];
        bus.pull_into(&mut out);
        assert_eq!(out, [0.5, 0.5, 0.0, 0.0]);
    }
}
