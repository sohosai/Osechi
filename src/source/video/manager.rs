use nokhwa::{native_api_backend, query};

use crate::{
    error::AppError,
    source::video::{Descriptor, SourceId, SourceKind, Stream},
};

/// 映像ソースの検出と管理を行うマネージャー。
///
/// スキャンによって検出されたソースの一覧を [`Descriptor`] として保持し、
/// 指定されたソースの [`Stream`] を生成する。
#[derive(Default)]
pub struct SourceManager {
    web_cameras: Vec<Descriptor>,
    // Todo:将来的にここに伝送,画面キャプチャ,WEB View,etc...が足されていく
}

impl SourceManager {
    pub fn new() -> Self {
        Self {
            web_cameras: Vec::new(),
        }
    }

    /// WEBカメラを全てスキャンする
    pub fn web_camera_scan(&mut self) {
        let backend = native_api_backend().unwrap_or(nokhwa::utils::ApiBackend::Auto);
        let cameras = query(backend).unwrap_or_default();

        self.web_cameras.clear();
        for info in cameras.into_iter() {
            let id_str = format!("{}_{}", info.human_name(), info.index());
            let id = SourceId::WebCamera(id_str);
            let name = info.human_name().to_string();

            let descriptor = Descriptor {
                id,
                name,
                kind: SourceKind::WebCamera {
                    index: info.index().clone(),
                },
            };
            self.web_cameras.push(descriptor);
        }
    }

    /// 最新のスキャン結果のWEBカメラの一覧を返す
    pub fn web_camera_list(&self) -> &[Descriptor] {
        &self.web_cameras
    }

    /// 指定されたIDの映像ソースを開き、[`Stream`] を取得する
    pub fn open(&self, source_id: &SourceId) -> Result<Box<dyn Stream>, AppError> {
        let descriptor = self
            .web_cameras
            .iter()
            .find(|d| d.id == *source_id)
            .ok_or_else(|| AppError::Other(format!("Video source {:?} not found", source_id)))?;
        descriptor.open()
    }
}
