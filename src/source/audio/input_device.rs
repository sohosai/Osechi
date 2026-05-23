use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

use crate::error::AppError;
use crate::source::audio::{AudioChunk, Descriptor, SourceId, SourceKind, Stream};

const AUDIO_RING_BUFFER_CAPACITY: usize = 8;

/// OS の音声入力デバイスを検出し、軽量な [`Descriptor`] として返す。
pub fn scan() -> Vec<Descriptor> {
    let host = cpal::default_host();
    let devices = match host.input_devices() {
        Ok(devices) => devices,
        Err(_) => return Vec::new(),
    };

    devices
        .enumerate()
        .filter_map(|(index, device)| {
            let name = device
                .description()
                .map(|description| description.name().to_string())
                .unwrap_or_else(|_| format!("Unknown Input Device {}", index));
            let device_id = device.id().ok()?;
            let id = SourceId::InputDevice(device_id.to_string());

            Some(Descriptor {
                id,
                name: name.clone(),
                kind: SourceKind::InputDevice { device_id, name },
            })
        })
        .collect()
}

/// 音声入力デバイスのアクティブなストリーム。
/// OS の音声 callback で受け取ったサンプルをリングバッファ経由でメインスレッドに渡す。
pub struct InputDeviceStream {
    pub device_id: cpal::DeviceId,
    pub device_name: String,
    // TODO: rtrb などの lock-free なリングバッファを使うことも検討する
    ring_buffer: Arc<Mutex<VecDeque<AudioChunk>>>,
    last_error: Arc<Mutex<Option<AppError>>>,
    _cpal_stream: cpal::Stream,
}

impl InputDeviceStream {
    /// 音声入力ストリームを構築・開始する。
    /// callback から届く音声データは f32 interleaved PCM に揃えて送信する。
    pub fn new(device_id: cpal::DeviceId, device_name: String) -> Result<Self, AppError> {
        let host = cpal::default_host();
        let device = host.device_by_id(&device_id).ok_or_else(|| {
            AppError::Other(format!("Audio input device not found: {}", device_name))
        })?;

        let supported_config = device.default_input_config().map_err(|e| {
            AppError::Other(format!(
                "Failed to get default input config for {}: {}",
                device_name, e
            ))
        })?;
        let sample_format = supported_config.sample_format();
        let config = supported_config.config();
        let sample_rate = config.sample_rate;
        let channels = config.channels;
        let ring_buffer = Arc::new(Mutex::new(VecDeque::with_capacity(
            AUDIO_RING_BUFFER_CAPACITY,
        )));
        let last_error = Arc::new(Mutex::new(None));

        let err_state = Arc::clone(&last_error);
        let err_fn = move |err| {
            set_last_error(
                &err_state,
                AppError::Other(format!("Audio input stream error: {}", err)),
            );
        };

        let stream = match sample_format {
            cpal::SampleFormat::F32 => build_input_stream::<f32>(
                &device,
                &config,
                sample_rate,
                channels,
                Arc::clone(&ring_buffer),
                err_fn,
                |sample| sample,
            ),
            cpal::SampleFormat::I16 => build_input_stream::<i16>(
                &device,
                &config,
                sample_rate,
                channels,
                Arc::clone(&ring_buffer),
                err_fn,
                |sample| sample as f32 / i16::MAX as f32,
            ),
            cpal::SampleFormat::U16 => build_input_stream::<u16>(
                &device,
                &config,
                sample_rate,
                channels,
                Arc::clone(&ring_buffer),
                err_fn,
                |sample| sample as f32 / u16::MAX as f32 * 2.0 - 1.0,
            ),
            other => {
                return Err(AppError::Other(format!(
                    "Unsupported audio input sample format: {:?}",
                    other
                )));
            }
        }?;

        stream
            .play()
            .map_err(|e| AppError::Other(format!("Failed to start audio input stream: {}", e)))?;

        Ok(Self {
            device_id,
            device_name,
            ring_buffer,
            last_error,
            _cpal_stream: stream,
        })
    }
}

impl Stream for InputDeviceStream {
    fn get_chunk(&self) -> Result<Option<AudioChunk>, AppError> {
        if let Some(err) = self
            .last_error
            .lock()
            .map_err(|_| AppError::Other("Audio input error state lock poisoned".to_string()))?
            .take()
        {
            return Err(err);
        }

        let mut buffer = self
            .ring_buffer
            .lock()
            .map_err(|_| AppError::Other("Audio input ring buffer lock poisoned".to_string()))?;

        match buffer.pop_front() {
            Some(chunk) => Ok(Some(chunk)),
            None => Ok(None),
        }
    }
}

/// `cpal` の sample format ごとの具体型を `f32` interleaved PCM の
/// [`AudioChunk`] に揃える入力ストリームを構築する。
///
/// `cpal` の入力 callback に届く sample 型はデバイス設定によって変わるため、
/// `convert_sample` で `f32` に変換してからリングバッファへ渡す。
fn build_input_stream<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    sample_rate: u32,
    channels: u16,
    ring_buffer: Arc<Mutex<VecDeque<AudioChunk>>>,
    err_fn: impl FnMut(cpal::StreamError) + Send + 'static,
    convert_sample: fn(T) -> f32,
) -> Result<cpal::Stream, AppError>
where
    T: cpal::SizedSample + Copy + Send + 'static,
{
    let channels_usize = channels as usize;
    let mut accumulated_frames: u64 = 0;

    device
        .build_input_stream(
            config,
            move |data: &[T], _| {
                // TODO: Vec へのコピーを減らすため、リングバッファに直接書き込む方法も検討する
                // TODO: SIMD を使ってサンプル変換を高速化する方法も検討する
                let samples: Vec<f32> = data.iter().copied().map(convert_sample).collect();
                let frames = samples.len().checked_div(channels_usize).unwrap_or(0);
                let pts = accumulated_frames;
                accumulated_frames += frames as u64;
                let chunk = AudioChunk {
                    samples: Arc::new(samples),
                    sample_rate,
                    channels,
                    frames,
                    pts,
                };
                push_ring_buffer(&ring_buffer, chunk);
            },
            err_fn,
            None,
        )
        .map_err(|e| AppError::Other(format!("Failed to build audio input stream: {}", e)))
}

fn push_ring_buffer(ring_buffer: &Arc<Mutex<VecDeque<AudioChunk>>>, chunk: AudioChunk) {
    let Ok(mut buffer) = ring_buffer.try_lock() else {
        return;
    };

    if buffer.len() >= AUDIO_RING_BUFFER_CAPACITY {
        buffer.pop_front();
    }
    buffer.push_back(chunk);
}

fn set_last_error(last_error: &Mutex<Option<AppError>>, err: AppError) {
    let Ok(mut last_error) = last_error.try_lock() else {
        return;
    };
    *last_error = Some(err);
}
