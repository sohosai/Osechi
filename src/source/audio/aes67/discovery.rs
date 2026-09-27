//! SAPで告知されるAES67フローの自動検出。

use std::collections::HashMap;
use std::time::{Duration, Instant};

use super::Config;
use crate::net::{multicast, sap, sdp};
use crate::source::audio::Kind;
use crate::source::{Feed, Origin, Source};

/// この時間だけ再告知が無ければフローを取り除く。SAPの典型的な再告知間隔
/// (数十秒〜数分)に対して十分な余裕を持たせる。
const SESSION_TIMEOUT: Duration = Duration::from_secs(5 * 60);

/// 未処理のSAPメッセージを溜めておく数。
const FEED_CAPACITY: usize = 64;

const RECV_BUFFER_SIZE: usize = 2048;

/// SAPの告知をバックグラウンドで受信し続け、いま告知されているAES67フローを把握する。
pub struct Discovery {
    messages: Feed<sap::Message>,
    sessions: HashMap<sap::Key, (Source<Kind>, Instant)>,
}

impl Discovery {
    /// 受信を始める。SAPのポートが使えない環境では `None`(手動追加は引き続き使える)。
    pub fn start() -> Option<Self> {
        let socket = multicast::receiver(sap::ADDR, sap::PORT).ok()?;
        let messages = Feed::spawn(FEED_CAPACITY, move |producer| {
            let mut buf = [0u8; RECV_BUFFER_SIZE];
            while producer.is_open() {
                if let Ok(Some(len)) = multicast::recv(&socket, &mut buf)
                    && let Ok(message) = sap::Message::try_from(&buf[..len])
                {
                    producer.send(Ok(message));
                }
            }
        });
        Some(Self {
            messages,
            sessions: HashMap::new(),
        })
    }

    /// 届いた告知を反映し、いま有効なフローを返す。毎フレーム呼ぶことを想定している。
    pub fn sources(&mut self) -> impl Iterator<Item = Source<Kind>> + '_ {
        for message in self.messages.try_iter() {
            match message {
                sap::Message::Announce { key, sdp } => {
                    if let Some(source) = parse(&sdp) {
                        self.sessions.insert(key, (source, Instant::now()));
                    }
                }
                sap::Message::Delete { key } => {
                    self.sessions.remove(&key);
                }
            }
        }
        self.sessions
            .retain(|_, (_, last_seen)| last_seen.elapsed() < SESSION_TIMEOUT);
        self.sessions.values().map(|(source, _)| source.clone())
    }
}

/// AES67として受信できるSDPならソースにする。
fn parse(sdp: &str) -> Option<Source<Kind>> {
    let session: sdp::Session = sdp.parse().ok()?;
    let config = Config::try_from(&session).ok()?;
    Some(config.into_source(&session.name, Origin::Discovered))
}

#[cfg(test)]
mod tests {
    use std::net::UdpSocket;
    use std::thread;

    use super::*;
    use crate::net::sap::tests::{SDP, packet};

    #[test]
    fn parses_announced_flow() {
        let source = parse(SDP).expect("valid AES67 SDP");
        assert_eq!(source.name, "Console Out L/R");
        assert_eq!(source.origin, Origin::Discovered);
        assert!(matches!(
            source.kind,
            Kind::Aes67(Config { channels: 2, .. })
        ));
    }

    #[test]
    fn ignores_unusable_sdp() {
        assert!(parse(&SDP.replace("L24", "OPUS")).is_none());
        assert!(parse(&SDP.replace("239.1.1.1", "192.168.1.1")).is_none());
    }

    /// SAPの既定アドレスへ実際に告知を送り、検出できるか確認する。
    /// ネットワーク環境に依存するため `cargo test -- --ignored` で実行する。
    #[test]
    #[ignore = "requires multicast networking on this machine"]
    fn discovers_flow_over_real_multicast_announce() {
        let mut discovery = Discovery::start().expect("SAP port 9875 should be free");
        let sender = UdpSocket::bind("0.0.0.0:0").expect("bind sender socket");
        let bytes = packet(false, SDP.as_bytes());

        let found = (0..50).any(|_| {
            sender
                .send_to(&bytes, (sap::ADDR, sap::PORT))
                .expect("send SAP packet");
            thread::sleep(Duration::from_millis(50));
            discovery
                .sources()
                .any(|source| source.name == "Console Out L/R")
        });
        assert!(found, "no flow discovered within the timeout");
    }
}
