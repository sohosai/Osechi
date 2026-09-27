//! 番組出力。PGMの映像とミキサーのマスター音声を、配信などの出力先へ渡す。
//!
//! エンジンスレッドが [`Program`] に最新のPGMフレームと合成済みの音声を置き、
//! 出力先(いまは [`rtmp`] のみ)はそれぞれ自分のスレッドから一定の間隔で取り出す。

mod convert;
mod encode;
mod flv;
pub mod rtmp;

use std::sync::{Arc, Mutex, PoisonError};

use crate::mixer::Bus;
use crate::source::video::Frame;

/// 出力の映像の大きさ(1080p)。
pub const WIDTH: usize = 1920;
pub const HEIGHT: usize = 1080;
/// 出力の映像のフレームレート。
pub const FRAME_RATE: u32 = 30;

/// 番組(PGM)の映像と音声の受け渡し口。`Clone` すると同じ受け渡し口を指す。
#[derive(Clone)]
pub struct Program {
    frame: Arc<Mutex<Option<Arc<Frame>>>>,
    audio: Bus,
}

impl Program {
    /// ミキサーの番組用の音声 `audio` を受け渡す口を作る。
    pub fn new(audio: Bus) -> Self {
        Self {
            frame: Arc::default(),
            audio,
        }
    }

    /// いまPGMに出ているフレームを置く。PGMが空なら `None`。
    pub fn set_frame(&self, frame: Option<Arc<Frame>>) {
        *self.frame.lock().unwrap_or_else(PoisonError::into_inner) = frame;
    }

    /// いまPGMに出ているフレーム。
    pub fn frame(&self) -> Option<Arc<Frame>> {
        self.frame
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// 合成済みの音声(48kHz・ステレオのinterleaved)。
    pub fn audio(&self) -> &Bus {
        &self.audio
    }
}
