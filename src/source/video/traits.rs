use std::sync::Arc;

use nokhwa::utils::CameraIndex;

/// 映像ソースを一意に識別するID
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum VideoSourceId {
    WebCamera(String),
    // Todo:将来的にここに伝送,画面キャプチャ,WEB View,etc...が足されていく
}

impl VideoSourceId {
    // ほぼ一意なIDを発行する
    pub fn as_string(&self) -> String {
        match self {
            Self::WebCamera(id) => format!("web_camera_{}", id),
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
pub enum VideoSourceKind {
    /// WEBカメラ
    WebCamera { index: CameraIndex },
    // Todo:将来的にここに伝送,画面キャプチャ,WEB View,etc...が足されていく
}

/// 映像ソースの設計図。
///
/// ID・名前・接続パラメータなど、ストリームを開かなくても取得できる情報を保持する。
/// `Clone` 可能で軽量なため、UI表示やソース一覧の保持に適している。
///
/// 実際にフレームを取得するには [`open`](VideoSourceDescriptor::open) で
/// [`VideoStream`] を生成する。
#[derive(Debug, Clone)]
pub struct VideoSourceDescriptor {
    pub id: VideoSourceId,
    pub name: String,
    pub kind: VideoSourceKind,
}

/// アクティブな映像ストリーム。
///
/// [`VideoSourceDescriptor::open`] から生成され、
/// OSのハードウェアデバイスへの接続を保持する。
/// フレームの取得のみに専念するトレイト。
///
/// # ライフサイクル
/// - drop されると、内部のハードウェア接続が自動的に切断される。
pub trait VideoStream: Send {
    /// 次のフレームを取得する。
    ///
    /// - `Ok(Some(frame))` — 新しいフレームが利用可能
    /// - `Ok(None)` — まだ新しいフレームが届いていない（ノンブロッキング）
    /// - `Err(e)` — ストリームエラーが発生
    fn get_frame(&mut self) -> Result<Option<FrameData>, crate::error::AppError>;

    // Todo:将来的には色の情報、解像度の情報、etc...が足されていく
    // 必要であればこの映像を参照しているプログラムが呼び出す
}
