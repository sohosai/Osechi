//! 映像ソース。映像は [`Frame`](RGB8の画像)として流れる。

mod camera;
#[cfg(any(windows, test))]
mod convert;
#[cfg(windows)]
mod directshow;
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
    /// DirectShowの映像デバイス(仮想カメラなど)
    #[cfg(windows)]
    DirectShow {
        friendly_name: String,
        device_path: String,
    },
}

impl Open for Kind {
    type Output = Frame;

    fn open(&self) -> Result<Feed<Frame>> {
        match self {
            Self::Camera(index) => camera::open(index.clone()),
            Self::Screen(monitor_id) => screen::open(*monitor_id),
            #[cfg(windows)]
            Self::DirectShow {
                friendly_name,
                device_path,
            } => directshow::open(directshow::Device {
                name: friendly_name.clone(),
                path: device_path.clone(),
            }),
        }
    }
}

/// 接続されている映像ソースを全て探す。
pub fn scan() -> Vec<Source<Kind>> {
    let mut sources: Vec<_> = camera::scan().into_iter().chain(screen::scan()).collect();
    #[cfg(windows)]
    {
        let mut names: std::collections::HashSet<String> = sources
            .iter()
            .map(|source| source.name.trim().to_lowercase())
            .collect();
        for source in directshow::scan() {
            if names.insert(source.name.trim().to_lowercase()) {
                sources.push(source);
            }
        }
    }
    sources
}

/// フレームを溜めておく数。表示には最新の1枚しか使わないので最小限にする。
const FEED_CAPACITY: usize = 2;
