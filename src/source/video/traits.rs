use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum VideoSourceId {
    WebCamera(String),
    Desktop(String),
    Ndi(String),
}

impl VideoSourceId {
    pub fn as_string(&self) -> String {
        match self {
            Self::WebCamera(id) => format!("web_camera_{}", id),
            Self::Desktop(id) => format!("desktop_{}", id),
            Self::Ndi(id) => format!("ndi_{}", id),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceInfo {
    pub id: VideoSourceId,
    pub name: String,
    pub index: nokhwa::utils::CameraIndex,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrameData {
    pub pixels: Arc<Vec<u8>>,
    pub width: u32,
    pub height: u32,
}

pub trait VideoSource: Send {
    fn get_frame(&mut self) -> Result<Option<FrameData>, crate::error::AppError>;
}
