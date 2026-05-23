pub mod input_device;
pub mod manager;

use std::sync::Arc;

use crate::error::AppError;
use crate::source::audio::input_device::InputDeviceStream;

/// 音声ソースを一意に識別するID
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum SourceId {
    InputDevice(String),
    // TODO: 将来的にここに伝送, システム音声, ファイル再生, etc...が足されていく
}

impl std::fmt::Display for SourceId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InputDevice(id) => write!(f, "audio_input_device_{}", id),
        }
    }
}

/// 音声の一定時間分のデータを表す型
#[derive(Debug, Clone, PartialEq)]
pub struct AudioChunk {
    /// f32 の interleaved PCM サンプル列
    pub samples: Arc<Vec<f32>>,
    pub sample_rate: u32,
    pub channels: u16,
    pub frames: usize,
    /// プレゼンテーションタイムスタンプ（ストリーム開始からの累積フレーム数）
    pub pts: u64,
}

/// 音声ソースの種別ごとの接続パラメータ。
///
/// ストリームを開くために必要なハードウェア固有の情報を保持する。
#[derive(Debug, Clone)]
pub enum SourceKind {
    /// OS の音声入力デバイス
    InputDevice {
        device_id: cpal::DeviceId,
        name: String,
    },
    // Todo: 将来的にここに伝送, システム音声, ファイル再生, etc...が足されていく
}

/// ID・名前・接続パラメータなど、ストリームを開かなくても取得できる情報を保持する。
/// `Clone` 可能で軽量なため、UI表示やソース一覧の保持に適している。
///
/// 実際に音声を取得するには [`open`](Descriptor::open) で
/// [`Stream`] を生成する。
#[derive(Debug, Clone)]
pub struct Descriptor {
    pub id: SourceId,
    pub name: String,
    pub kind: SourceKind,
}

impl Descriptor {
    /// 実際の音声ストリームを開く。
    /// 内部で OS のハードウェアデバイスへの接続が行われる。
    pub fn open(&self) -> Result<Box<dyn Stream>, AppError> {
        match &self.kind {
            SourceKind::InputDevice { device_id, name } => {
                let stream = InputDeviceStream::new(device_id.clone(), name.clone())?;
                Ok(Box::new(stream))
            }
        }
    }
}

/// [`Descriptor::open`] から生成され、
/// OSのハードウェアデバイスへの接続を保持する。
///
/// # ライフサイクル
/// - drop されると、内部のハードウェア接続が自動的に切断される。
pub trait Stream: Send {
    /// 次の音声チャンクを取得する。
    ///
    /// - `Ok(Some(chunk))` — 新しい音声データが利用可能
    /// - `Ok(None)` — まだ新しい音声データが届いていない（ノンブロッキング）
    /// - `Err(e)` — ストリームエラーが発生
    fn get_chunk(&self) -> Result<Option<AudioChunk>, AppError>;

    // Todo: 将来的には音量, レイテンシ, デバイスフォーマット情報, etc...が足されていく
}
