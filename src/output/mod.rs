//! 番組出力。スロットの映像とミキサーのマスター音声を、配信・録画へ渡す。
//!
//! エンジンスレッドが [`Taps`] にスロットごとの最新フレームを置き、ミキサーが合成済みの音声を配る。
//! 出力(配信 [`rtmp`]・録画 [`record`])はそれぞれ自分のスレッドから一定の間隔で取り出し、
//! 共通の処理([`pipeline`])でエンコードする。

mod convert;
mod encode;
mod flv;
mod h264;
mod mov;
mod pipeline;
pub mod record;
pub mod rtmp;

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

use crate::mixer::{Bus, Fanout};
use crate::source::video::Frame;
use crate::switcher::Slot;

/// 出力の映像のフレームレート。
pub const FRAME_RATE: u32 = 30;

/// スロットの映像と音声の受け渡し口。`Clone` すると同じ受け渡し口を指す。
#[derive(Clone, Default)]
pub struct Taps {
    frames: Arc<Mutex<HashMap<Slot, Arc<Frame>>>>,
    audio: Fanout,
}

impl Taps {
    /// ミキサーの番組用の音声 `audio` を配る受け渡し口を作る。
    pub fn new(audio: Fanout) -> Self {
        Self {
            frames: Arc::default(),
            audio,
        }
    }

    /// 各スロットにいま出ているフレームを置き換える。載っていないスロットは空とみなす。
    pub fn set_frames(&self, frames: impl IntoIterator<Item = (Slot, Arc<Frame>)>) {
        *self.frames.lock().unwrap_or_else(PoisonError::into_inner) = frames.into_iter().collect();
    }

    /// `slot` にいま出ているフレーム。
    pub fn frame(&self, slot: Slot) -> Option<Arc<Frame>> {
        self.frames
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&slot)
            .cloned()
    }

    /// 合成済みの音声(48kHz・ステレオのinterleaved)を受け取り始める。
    pub fn subscribe_audio(&self) -> Bus {
        self.audio.subscribe()
    }
}
