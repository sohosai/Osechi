use std::sync::Arc;

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

/// あらゆる映像ソースに共通で定義できる性質
pub trait VideoSource: Send {
    fn id(&self) -> VideoSourceId;

    /// ユーザーに表示される[VideoSource]の名称
    fn name(&self) -> String;

    /// この[VideoSource]から次のフレームを取得する関数
    fn get_frame(&mut self) -> Result<Option<FrameData>, crate::error::AppError>;
    fn clone_box(&self) -> Box<dyn VideoSource>;

    // Todo:将来的には色の情報、解像度の情報、etc...が足されていく
    // 必要であればこの映像を参照しているプログラムが呼び出す
}
