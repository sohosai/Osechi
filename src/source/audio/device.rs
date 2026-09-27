//! OSの音声入力デバイス(cpal)。

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample};

use super::{Chunk, FEED_CAPACITY, Kind};
use crate::error::{Context, Error, Result};
use crate::source::{Feed, Origin, Producer, Source, SourceId};

/// 接続されている音声入力デバイスを探す。
pub(super) fn scan() -> Vec<Source<Kind>> {
    let Ok(devices) = cpal::default_host().input_devices() else {
        return Vec::new();
    };
    devices
        .enumerate()
        .filter_map(|(index, device)| {
            let id = device.id().ok()?;
            let name = device
                .description()
                .map(|description| description.name().to_string())
                .unwrap_or_else(|_| format!("Unknown Input Device {index}"));
            Some(Source {
                id: SourceId::new("mic", &id),
                name,
                kind: Kind::Device(id),
                origin: Origin::Scanned,
            })
        })
        .collect()
}

/// デバイスの既定設定で入力ストリームを開始する。ストリームは返した [`Feed`] が保持する。
pub(super) fn open(id: &cpal::DeviceId) -> Result<Feed<Chunk>> {
    let device = cpal::default_host()
        .device_by_id(id)
        .context(format!("audio input device not found: {id}"))?;
    let supported = device
        .default_input_config()
        .context("failed to get default input config")?;
    let config = supported.config();

    let (producer, feed) = Feed::new(FEED_CAPACITY);
    let stream = match supported.sample_format() {
        SampleFormat::F32 => build::<f32>(&device, &config, producer),
        SampleFormat::I16 => build::<i16>(&device, &config, producer),
        SampleFormat::U16 => build::<u16>(&device, &config, producer),
        other => Err(Error::new(format!(
            "unsupported input sample format: {other:?}"
        ))),
    }?;
    stream
        .play()
        .context("failed to start audio input stream")?;
    Ok(feed.with_guard(stream))
}

/// サンプル型 `T` の入力ストリームを作り、届いた音声を f32 に揃えて `producer` へ送る。
fn build<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    producer: Producer<Chunk>,
) -> Result<cpal::Stream>
where
    T: SizedSample,
    f32: FromSample<T>,
{
    let (sample_rate, channels) = (config.sample_rate, config.channels);
    let on_error = producer.clone();
    device
        .build_input_stream(
            config,
            move |data: &[T], _| {
                producer.send(Ok(Chunk {
                    samples: data.iter().map(|sample| sample.to_sample()).collect(),
                    sample_rate,
                    channels,
                }));
            },
            move |err| {
                on_error.send(Err(err).context("audio input stream error"));
            },
            None,
        )
        .context("failed to build audio input stream")
}
