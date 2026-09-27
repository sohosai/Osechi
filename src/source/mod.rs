//! 映像・音声の入力ソース。
//!
//! 映像も音声も同じ骨格で扱う。
//!
//! - [`Source`] … 開かなくても分かる情報(ID・表示名・種別・由来)。軽量で `Clone` できる。
//! - [`Catalog`] … 利用可能なソースの一覧。
//! - [`Open`] … 種別(`video::Kind` / `audio::Kind`)からデータの経路 [`Feed`] を開く。
//! - [`Live`] … いま開いているソースの集合。必要なものだけを開いた状態に保つ。
//!
//! 個々の入力方式(Webカメラ、AES67など)は `video/` `audio/` 以下に1方式1モジュールで置き、
//! それぞれが `scan()`(見つけられる方式のみ)と `open()` を提供する。

pub mod audio;
mod catalog;
mod feed;
mod live;
pub mod video;

use std::fmt;

pub use catalog::Catalog;
pub use feed::{Feed, Producer};
pub use live::Live;

use crate::error::Result;

/// ソースを一意に識別するID。`camera:...` のように方式ごとの接頭辞を持つ。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SourceId(String);

impl SourceId {
    /// 方式名 `scheme` と、その方式の中で一意なキー `key` からIDを作る。
    pub fn new(scheme: &str, key: impl fmt::Display) -> Self {
        Self(format!("{scheme}:{key}"))
    }
}

impl fmt::Display for SourceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// ソースが一覧に載った経緯。一覧のどの部分を誰が更新してよいかを決める。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    /// デバイスのスキャンで見つかったもの。再スキャンで入れ替わる。
    Scanned,
    /// ユーザーや起動設定が明示的に追加したもの。明示的に削除するまで残る。
    Manual,
    /// ネットワーク上の告知(SAP)で見つかったもの。告知が途絶えると消える。
    Discovered,
}

/// ソースの情報。`K` は映像なら `video::Kind`、音声なら `audio::Kind`。
#[derive(Debug, Clone)]
pub struct Source<K> {
    pub id: SourceId,
    pub name: String,
    pub kind: K,
    pub origin: Origin,
}

/// ソースの種別から、データの経路を開く。
///
/// 開くと取得処理(スレッドやOSのコールバック)が動き出し、返した [`Feed`] を
/// drop すると止まる。
pub trait Open {
    type Output;

    fn open(&self) -> Result<Feed<Self::Output>>;
}
