use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct VideoSourceId(pub usize);

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

/// すべての映像ソースが実装すべき共通トレイト
pub trait VideoSource: Send {
    /// 次のフレームデータを取得する関数
    fn get_frame(&mut self) -> Result<Option<FrameData>, crate::error::AppError>;
}
