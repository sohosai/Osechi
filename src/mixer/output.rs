//! 合成した音声の出力。UIスレッドが積んだミックスを [`Bus`] 経由で出力デバイスへ渡す。

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, PoisonError};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample};

use super::{CHANNELS, SAMPLE_RATE, dsp};
use crate::error::{Context, Error, Result};

/// バスに溜める上限(約0.5秒分)。出力が無い・遅いときに遅延が積み上がらないよう古い方から捨てる。
const BUS_CAPACITY: usize = SAMPLE_RATE as usize * CHANNELS as usize / 2;

/// 合成済みの音声([`SAMPLE_RATE`]Hz・ステレオのinterleaved)を、UIスレッドから出力デバイスの
/// コールバック(別スレッド)へ渡すバッファ。
#[derive(Clone, Default)]
pub struct Bus {
    samples: Arc<Mutex<VecDeque<f32>>>,
}

impl Bus {
    /// 追記する。溜まりすぎたら古い方から捨てる。
    pub fn push(&self, samples: &[f32]) {
        let mut buffer = self.samples.lock().unwrap_or_else(PoisonError::into_inner);
        buffer.extend(samples);
        let excess = buffer.len().saturating_sub(BUS_CAPACITY);
        buffer.drain(..excess);
    }

    /// `out` を埋める。足りない分は無音にする。
    pub fn pull(&self, out: &mut [f32]) {
        let mut buffer = self.samples.lock().unwrap_or_else(PoisonError::into_inner);
        for slot in out {
            *slot = buffer.pop_front().unwrap_or(0.0);
        }
    }
}

/// 出力デバイスの一覧(ID・表示名)。
pub fn devices() -> Vec<(cpal::DeviceId, String)> {
    let Ok(devices) = cpal::default_host().output_devices() else {
        return Vec::new();
    };
    devices
        .enumerate()
        .filter_map(|(index, device)| {
            let name = device
                .description()
                .map(|description| description.name().to_string())
                .unwrap_or_else(|_| format!("Unknown Output Device {index}"));
            Some((device.id().ok()?, name))
        })
        .collect()
}

/// モニター出力。選んだデバイスへ [`Bus`] の音声を流し続ける。
#[derive(Default)]
pub struct Monitor {
    device: Option<cpal::DeviceId>,
    stream: Option<cpal::Stream>,
}

impl Monitor {
    pub fn device(&self) -> Option<&cpal::DeviceId> {
        self.device.as_ref()
    }

    /// 出力先を切り替える。`None` なら止める。開けなくても選択は記録する。
    pub fn set(&mut self, device: Option<cpal::DeviceId>, bus: &Bus) -> Result<()> {
        self.stream = None;
        self.device = device;
        let Some(id) = &self.device else {
            return Ok(());
        };
        let device = cpal::default_host()
            .device_by_id(id)
            .context(format!("output device not found: {id}"))?;
        let supported = device
            .default_output_config()
            .context("failed to get default output config")?;
        let config = supported.config();

        let stream = match supported.sample_format() {
            SampleFormat::F32 => build::<f32>(&device, &config, bus.clone()),
            SampleFormat::I16 => build::<i16>(&device, &config, bus.clone()),
            SampleFormat::U16 => build::<u16>(&device, &config, bus.clone()),
            other => Err(Error::new(format!(
                "unsupported output sample format: {other:?}"
            ))),
        }?;
        stream
            .play()
            .context("failed to start audio output stream")?;
        self.stream = Some(stream);
        Ok(())
    }
}

/// `bus` の音声をデバイスのサンプルレート・チャンネル数・サンプル型に変換して書き込むストリームを作る。
fn build<T>(device: &cpal::Device, config: &cpal::StreamConfig, bus: Bus) -> Result<cpal::Stream>
where
    T: SizedSample + FromSample<f32>,
{
    let (rate, channels) = (config.sample_rate, config.channels as usize);
    let mut scratch: Vec<f32> = Vec::new();

    device
        .build_output_stream(
            config,
            move |data: &mut [T], _| {
                if channels == 0 {
                    return;
                }
                let frames = data.len() / channels;
                let needed =
                    (frames as u64 * SAMPLE_RATE as u64 / (rate as u64).max(1)).max(1) as usize;
                scratch.resize(needed * CHANNELS as usize, 0.0);
                bus.pull(&mut scratch);
                let stereo = dsp::resample_stereo(&scratch, SAMPLE_RATE, rate);

                for (i, frame) in data.chunks_mut(channels).enumerate() {
                    let l = stereo.get(i * 2).copied().unwrap_or(0.0);
                    let r = stereo.get(i * 2 + 1).copied().unwrap_or(0.0);
                    for (ch, sample) in frame.iter_mut().enumerate() {
                        let value = match (channels, ch) {
                            (1, _) => (l + r) * 0.5,
                            (_, 0) => l,
                            _ => r,
                        };
                        *sample = T::from_sample(value);
                    }
                }
            },
            |err| tracing::error!("audio output stream error: {err}"),
            None,
        )
        .context("failed to build audio output stream")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bus_returns_pushed_samples() {
        let bus = Bus::default();
        bus.push(&[0.1, 0.2, 0.3, 0.4]);
        let mut out = [0.0; 4];
        bus.pull(&mut out);
        assert_eq!(out, [0.1, 0.2, 0.3, 0.4]);
    }

    #[test]
    fn bus_pads_with_silence_when_underrun() {
        let bus = Bus::default();
        bus.push(&[0.5, 0.5]);
        let mut out = [1.0; 4];
        bus.pull(&mut out);
        assert_eq!(out, [0.5, 0.5, 0.0, 0.0]);
    }

    #[test]
    fn bus_drops_oldest_beyond_capacity() {
        let bus = Bus::default();
        bus.push(&vec![0.0; BUS_CAPACITY]);
        bus.push(&[1.0]);
        let mut out = vec![0.0; BUS_CAPACITY];
        bus.pull(&mut out);
        assert_eq!(out.last(), Some(&1.0));
    }
}
