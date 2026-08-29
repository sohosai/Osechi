pub mod manager;
pub mod screen_capture;
pub mod web_camera;

use crate::error::AppError;
use crate::source::video::screen_capture::ScreenCaptureStream;
use crate::source::video::web_camera::WebCameraStream;
use nokhwa::utils::CameraIndex;
use std::sync::Arc;

/// 映像ソースを一意に識別するID
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum SourceId {
    WebCamera(String),
    ScreenCapture(u32),
    // Todo:将来的にここに伝送,WEB View,etc...が足されていく
}

impl std::fmt::Display for SourceId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::WebCamera(id) => write!(f, "web_camera_{}", id),
            Self::ScreenCapture(id) => write!(f, "screen_capture_{}", id),
        }
    }
}

/// 映像の1フレームのデータを表す型
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrameData {
    pub pixels: Arc<Vec<u8>>,
    pub width: u32,
    pub height: u32,
}

/// 映像ソースの種別ごとの接続パラメータ。
///
/// ストリームを開くために必要なハードウェア固有の情報を保持する。
#[derive(Debug, Clone)]
pub enum SourceKind {
    /// WEBカメラ
    WebCamera { index: CameraIndex },
    /// 画面共有(モニター単位のキャプチャ)
    ScreenCapture { monitor_id: u32 },
    // Todo:将来的にここに伝送,WEB View,etc...が足されていく
}

/// ID・名前・接続パラメータなど、ストリームを開かなくても取得できる情報を保持する。
/// `Clone` 可能で軽量なため、UI表示やソース一覧の保持に適している。
///
/// 実際にフレームを取得するには [`open`](Descriptor::open) で
/// [`Stream`] を生成する。
#[derive(Debug, Clone)]
pub struct Descriptor {
    pub id: SourceId,
    pub name: String,
    pub kind: SourceKind,
}

impl Descriptor {
    /// 実際の映像ストリームを開く。
    /// 内部で OS のハードウェアデバイスへの接続が行われる。
    pub fn open(&self) -> Result<Box<dyn Stream>, AppError> {
        match &self.kind {
            SourceKind::WebCamera { index } => {
                let mut stream = WebCameraStream::new(index.clone());
                stream.open_stream()?;
                Ok(Box::new(stream))
            }
            SourceKind::ScreenCapture { monitor_id } => {
                let mut stream = ScreenCaptureStream::new(*monitor_id);
                stream.open_stream()?;
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
    /// 次のフレームを取得する。
    ///
    /// - `Ok(Some(frame))` — 新しいフレームが利用可能
    /// - `Ok(None)` — まだ新しいフレームが届いていない（ノンブロッキング）
    /// - `Err(e)` — ストリームエラーが発生
    fn get_frame(&mut self) -> Result<Option<FrameData>, AppError>;

    // Todo:将来的には色の情報、解像度の情報、etc...が足されていく
    // 必要であればこの映像を参照しているプログラムが呼び出す
}
