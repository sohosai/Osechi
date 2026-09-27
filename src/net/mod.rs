//! ネットワークのプロトコル処理。パケットの解釈とソケットの準備だけを担い、
//! 映像・音声などのドメインの型には依存しない。

pub mod multicast;
pub mod rtp;
pub mod sap;
pub mod sdp;
