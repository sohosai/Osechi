pub mod video;

use nokhwa::{native_api_backend, query};

use crate::error::AppError;
use crate::source::video::traits::{SourceInfo, VideoSource, VideoSourceId};
use crate::source::video::web_camera::WebCamera;

/// あらゆる映像ソースを管理する型
pub struct VideoSourceManager {
    web_camera_infos: Vec<SourceInfo>,
}

impl VideoSourceManager {
    pub fn new() -> Self {
        Self {
            web_camera_infos: Vec::new(),
        }
    }

    /// WEBカメラを全てスキャンする
    pub fn web_camera_scan(&mut self) {
        let backend = native_api_backend().unwrap_or(nokhwa::utils::ApiBackend::Auto);
        let cameras = query(backend).unwrap_or_default();

        self.web_camera_infos.clear();
        for info in cameras.into_iter() {
            let id_str = format!("{}_{}", info.human_name(), info.index());
            let id = VideoSourceId::WebCamera(id_str);
            let name = format!("📷 {}", info.human_name());

            self.web_camera_infos.push(SourceInfo {
                id,
                name,
                index: info.index().clone(),
            });
        }
    }

    /// 最新のスキャン結果のWEBカメラのリストを返す
    pub fn web_camera_list(&self) -> &[SourceInfo] {
        &self.web_camera_infos
    }

    /// 映像ソースを開いて、[VideoSource]を取得する
    pub fn open_source(&self, source_id: &VideoSourceId) -> Result<Box<dyn VideoSource>, AppError> {
        match source_id {
            VideoSourceId::WebCamera(_) => {
                let info = self
                    .web_camera_infos
                    .iter()
                    .find(|s| s.id == *source_id)
                    .ok_or_else(|| {
                        AppError::Other(format!("WebCamera {:?} not found", source_id))
                    })?;
                let source = WebCamera::new(info.index.clone())?;
                Ok(Box::new(source))
            }
            VideoSourceId::Desktop(_) => Err(AppError::Other(
                "Desktop capture not implemented".to_string(),
            )),
            VideoSourceId::Ndi(_) => {
                Err(AppError::Other("NDI capture not implemented".to_string()))
            }
        }
    }
}

impl Default for VideoSourceManager {
    fn default() -> Self {
        Self::new()
    }
}
