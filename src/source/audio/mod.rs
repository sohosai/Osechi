//! 音声ソース。音声は [`Chunk`](f32のinterleaved PCM)として流れる。

pub mod aes67;
mod device;

use super::{Feed, Open, Source};
use crate::error::Result;

/// 一定時間分の音声。
#[derive(Debug, Clone, PartialEq)]
pub struct Chunk {
    /// f32 の interleaved PCM(-1.0..=1.0)。長さはチャンネル数の倍数とは限らない。
    pub samples: Vec<f32>,
    pub sample_rate: u32,
    pub channels: u16,
}

/// 音声ソースの種別と、開くのに必要な情報。
#[derive(Debug, Clone)]
pub enum Kind {
    /// OSの音声入力デバイス
    Device(cpal::DeviceId),
    /// AES67のRTPマルチキャストフロー
    Aes67(aes67::Config),
}

impl Open for Kind {
    type Output = Chunk;

    fn open(&self) -> Result<Feed<Chunk>> {
        match self {
            Self::Device(id) => device::open(id),
            Self::Aes67(config) => aes67::open(config),
        }
    }
}

/// 接続されている音声入力デバイスを全て探す。AES67フローは [`aes67::Discovery`] で見つける。
pub fn scan() -> Vec<Source<Kind>> {
    device::scan()
}

/// チャンクを溜めておく数。
///
/// エンジンスレッドが取り出すのは約10msに1回(ロックの待ちで延びることもある)で、AES67はptime=1msごとに
/// 1チャンク届く。取り出す間隔+余裕を吸収できないと、取り出す前に上書きされて音が周期的に
/// 欠け、ノイズになる。OSの入力デバイスは1チャンクがずっと長いのでこれで十分足りる。
const FEED_CAPACITY: usize = 64;
