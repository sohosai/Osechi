pub mod video;

use nokhwa::{native_api_backend, query};

use crate::error::AppError;
use crate::source::video::traits::{
    VideoSourceDescriptor, VideoSourceId, VideoSourceKind, VideoStream,
};
use crate::source::video::web_camera::WebCameraStream;

impl VideoSourceDescriptor {
    /// この設計図から実際の映像ストリームを開く。
    ///
    /// 内部で OS のハードウェアデバイスへの接続が行われる。
    pub fn open(&self) -> Result<Box<dyn VideoStream>, AppError> {
        match &self.kind {
            VideoSourceKind::WebCamera { index } => {
                let mut stream = WebCameraStream::new(index.clone());
                stream.open_stream()?;
                Ok(Box::new(stream))
            }
        }
    }
}

/// あらゆる映像ソースの検出と管理を行うマネージャー。
///
/// スキャンによって検出されたソースの一覧を [`VideoSourceDescriptor`] として保持し、
/// 指定されたソースの [`VideoStream`] を生成する。
#[derive(Default)]
pub struct VideoSourceManager {
    web_cameras: Vec<VideoSourceDescriptor>,
    // Todo:将来的にここに伝送,画面キャプチャ,WEB View,etc...が足されていく
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

            let descriptor = VideoSourceDescriptor {
                id,
                name,
                kind: VideoSourceKind::WebCamera {
                    index: info.index().clone(),
                },
            };
            self.web_cameras.push(descriptor);
        }
    }

    /// 最新のスキャン結果のWEBカメラの一覧を返す
    pub fn web_camera_list(&self) -> &[VideoSourceDescriptor] {
        &self.web_cameras
    }

    /// 指定されたIDの映像ソースを開き、[`VideoStream`] を取得する
    pub fn open(&self, video_source_id: &VideoSourceId) -> Result<Box<dyn VideoStream>, AppError> {
        let descriptor = self
            .web_cameras
            .iter()
            .find(|d| d.id == *video_source_id)
            .ok_or_else(|| {
                AppError::Other(format!("VideoSource {:?} not found", video_source_id))
            })?;
        descriptor.open()
    }
}
