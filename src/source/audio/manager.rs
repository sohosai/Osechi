use crate::{
    error::AppError,
    source::audio::input_device,
    source::audio::{Descriptor, SourceId, Stream},
};

/// 音声ソースの検出と管理を行うマネージャー。
///
/// スキャンによって検出されたソースの一覧を [`Descriptor`] として保持し、
/// 指定されたソースの [`Stream`] を生成する。
#[derive(Default)]
pub struct SourceManager {
    input_devices: Vec<Descriptor>,
    // Todo: 将来的にここに伝送, システム音声, ファイル再生, etc...が足されていく
}

impl SourceManager {
    pub fn new() -> Self {
        Self {
            input_devices: Vec::new(),
        }
    }

    /// 音声入力デバイスを全てスキャンする
    pub fn input_device_scan(&mut self) {
        self.input_devices = input_device::scan();
    }

    /// 最新のスキャン結果の音声入力デバイス一覧を返す
    pub fn input_device_list(&self) -> &[Descriptor] {
        &self.input_devices
    }

    /// 指定されたIDの音声ソースを開き、[`Stream`] を取得する
    pub fn open(&self, source_id: &SourceId) -> Result<Box<dyn Stream>, AppError> {
        let descriptor = self
            .input_devices
            .iter()
            .find(|d| d.id == *source_id)
            .ok_or_else(|| AppError::Other(format!("Audio source {:?} not found", source_id)))?;
        descriptor.open()
    }
}
