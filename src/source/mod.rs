pub mod video;

use nokhwa::{native_api_backend, query};

use crate::error::AppError;
use crate::source::video::traits::{VideoSource, VideoSourceId};
use crate::source::video::web_camera::WebCamera;

/// あらゆる映像ソースを管理する型
pub struct VideoSourceManager {
    web_cameras: Vec<Box<dyn VideoSource>>,
}

impl VideoSourceManager {
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
            let id = VideoSourceId::WebCamera(id_str);
            let name = format!("📷 {}", info.human_name());

            let camera = WebCamera::new(id, name, info.index().clone());
            self.web_cameras.push(Box::new(camera));
        }
    }

    /// 最新のスキャン結果のWEBカメラのリストを返す
    pub fn web_camera_list(&self) -> &[Box<dyn VideoSource>] {
        &self.web_cameras
    }

    /// 映像ソースを開いて、[VideoSource]を取得する
    pub fn open(&self, video_source_id: &VideoSourceId) -> Result<Box<dyn VideoSource>, AppError> {
        let source = self
            .web_cameras
            .iter()
            .find(|s| s.id() == *video_source_id)
            .ok_or_else(|| {
                AppError::Other(format!("VideoSource {:?} not found", video_source_id))
            })?;
        
        // clone_box() で遅延評価される新しいインスタンスを返す
        Ok(source.clone_box())
    }
}

impl Default for VideoSourceManager {
    fn default() -> Self {
        Self::new()
    }
}
