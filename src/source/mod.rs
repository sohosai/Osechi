pub mod video;

use nokhwa::{native_api_backend, query};
use std::collections::HashMap;

use crate::error::AppError;
use crate::source::video::traits::{SourceInfo, VideoSource, VideoSourceId};
use crate::source::video::web_camera::WebCamera;

/// 複数の映像ソースを統括するマネージャー
pub struct SourceManager {
    sources: HashMap<VideoSourceId, Box<dyn VideoSource>>,
    source_infos: Vec<SourceInfo>,
}

impl SourceManager {
    pub fn new() -> Self {
        let backend = native_api_backend().unwrap_or(nokhwa::utils::ApiBackend::Auto);
        let cameras = query(backend).unwrap_or_default();

        let mut source_infos = Vec::new();

        for (i, info) in cameras.into_iter().enumerate() {
            let id = VideoSourceId(i);
            let name = info.human_name();

            source_infos.push(SourceInfo {
                id,
                name,
                index: info.index().clone(),
            });
        }

        Self {
            sources: HashMap::new(),
            source_infos,
        }
    }

    pub fn available_sources(&self) -> &[SourceInfo] {
        &self.source_infos
    }

    pub fn open_source(&mut self, source_id: VideoSourceId) -> Result<(), AppError> {
        if self.sources.contains_key(&source_id) {
            return Ok(());
        }

        let info = self
            .source_infos
            .iter()
            .find(|s| s.id == source_id)
            .ok_or_else(|| AppError::Other(format!("Source {:?} not found", source_id)))?;

        let source = WebCamera::new(info.id, info.index.clone())?;
        self.sources.insert(source_id, Box::new(source));
        Ok(())
    }

    pub fn get_frame(
        &mut self,
        source_id: VideoSourceId,
    ) -> Result<Option<crate::source::video::traits::FrameData>, AppError> {
        let source = self
            .sources
            .get_mut(&source_id)
            .ok_or_else(|| AppError::Other(format!("Source {:?} not open", source_id)))?;
        source.get_frame()
    }
}

impl Default for SourceManager {
    fn default() -> Self {
        Self::new()
    }
}
