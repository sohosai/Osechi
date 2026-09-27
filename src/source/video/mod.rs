//! 映像ソース。映像は [`Frame`](RGB8の画像)として流れる。

mod camera;
mod screen;

use super::{Feed, Open, Source};
use crate::error::Result;

/// 映像の1フレーム。
pub type Frame = image::RgbImage;

/// 映像ソースの種別と、開くのに必要な情報。
#[derive(Debug, Clone)]
pub enum Kind {
    /// Webカメラ
    Camera(nokhwa::utils::CameraIndex),
    /// 画面キャプチャ(モニターID)
    Screen(u32),
}

impl Open for Kind {
    type Output = Frame;

    fn open(&self) -> Result<Feed<Frame>> {
        match self {
            Self::Camera(index) => camera::open(index.clone()),
            Self::Screen(monitor_id) => screen::open(*monitor_id),
        }
    }
}

/// 接続されている映像ソースを全て探す。
pub fn scan() -> Vec<Source<Kind>> {
    camera::scan().into_iter().chain(screen::scan()).collect()
}

/// フレームを溜めておく数。表示には最新の1枚しか使わないので最小限にする。
const FEED_CAPACITY: usize = 2;
