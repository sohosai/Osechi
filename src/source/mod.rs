pub mod video;

use nokhwa::{native_api_backend, query};

use crate::error::AppError;
use crate::source::video::traits::{SourceInfo, VideoSource, VideoSourceId};
use crate::source::video::web_camera::WebCamera;

/// 映像ソースの探索と生成を担当するファクトリーマネージャー
pub struct SourceManager {
    source_infos: Vec<SourceInfo>,
}

impl SourceManager {
    pub fn new() -> Self {
        let mut manager = Self {
            source_infos: Vec::new(),
        };
        manager.rescan();
        manager
    }

    pub fn rescan(&mut self) {
        let backend = native_api_backend().unwrap_or(nokhwa::utils::ApiBackend::Auto);
        let cameras = query(backend).unwrap_or_default();

        self.source_infos.clear();
        for info in cameras.into_iter() {
            // デバイス名とインデックスを組み合わせた不変のIDを生成
            let id_str = format!("{}_{}", info.human_name(), info.index());
            let id = VideoSourceId(id_str);
            let name = info.human_name();

            self.source_infos.push(SourceInfo {
                id,
                name,
                index: info.index().clone(),
            });
        }
    }

    pub fn available_sources(&self) -> &[SourceInfo] {
        &self.source_infos
    }

    pub fn open_source(&self, source_id: &VideoSourceId) -> Result<Box<dyn VideoSource>, AppError> {
        let info = self
            .source_infos
            .iter()
            .find(|s| s.id == *source_id)
            .ok_or_else(|| AppError::Other(format!("Source {:?} not found", source_id)))?;

        let source = WebCamera::new(info.index.clone())?;
        Ok(Box::new(source))
    }
}

impl Default for SourceManager {
    fn default() -> Self {
        Self::new()
    }
}
