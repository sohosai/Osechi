use std::collections::HashMap;
use std::net::{Ipv4Addr, SocketAddrV4, UdpSocket};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use crate::source::audio::aes67::{Aes67FlowConfig, Aes67SampleFormat};
use crate::source::audio::{Descriptor, SourceId, SourceKind};

/// SAP(Session Announcement Protocol, RFC 2974)のデフォルトのマルチキャストアドレス/ポート。
const SAP_ADDR: Ipv4Addr = Ipv4Addr::new(224, 2, 127, 254);
const SAP_PORT: u16 = 9875;

/// この時間だけ再アナウンスが無ければセッションを取り除く。
/// SAPの典型的な再アナウンス間隔(数十秒〜数分)に対して十分な余裕を持たせる。
const SESSION_TIMEOUT: Duration = Duration::from_secs(5 * 60);

const RECV_TIMEOUT: Duration = Duration::from_millis(500);
const RECV_BUFFER_SIZE: usize = 2048;

/// SAPアナウンスのセッションを一意に識別するキー(RFC 2974)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct SessionKey {
    origin: Ipv4Addr,
    msg_id_hash: u16,
}

struct TrackedSession {
    descriptor: Descriptor,
    last_seen: Instant,
}

/// SAPアナウンスをバックグラウンドで購読し続け、AES67の音声フロー一覧を
/// 自動的に更新するリスナー。
///
/// マルチキャストソケットが使えない環境(ポートが既に使われている等)では
/// [`SapListener::start`] が `None` を返す。SAP自動検出はあくまで付加機能
/// であり、それが無くても手動でのAES67ソース追加は引き続き使えるため、
/// 致命的エラーにはしない。
pub struct SapListener {
    sessions: Arc<Mutex<HashMap<SessionKey, TrackedSession>>>,
    stop: Arc<AtomicBool>,
}

impl SapListener {
    pub fn start() -> Option<Self> {
        let socket = UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, SAP_PORT)).ok()?;
        socket
            .join_multicast_v4(&SAP_ADDR, &Ipv4Addr::UNSPECIFIED)
            .ok()?;
        socket.set_read_timeout(Some(RECV_TIMEOUT)).ok()?;

        let sessions = Arc::new(Mutex::new(HashMap::new()));
        let stop = Arc::new(AtomicBool::new(false));

        spawn_receive_thread(socket, Arc::clone(&sessions), Arc::clone(&stop));

        Some(Self { sessions, stop })
    }

    /// 現時点で有効な(タイムアウトしていない)SAP由来のディスクリプタ一覧を返す。
    pub fn discovered(&self) -> Vec<Descriptor> {
        let Ok(mut sessions) = self.sessions.lock() else {
            return Vec::new();
        };
        sessions.retain(|_, session| session.last_seen.elapsed() < SESSION_TIMEOUT);
        sessions
            .values()
            .map(|session| session.descriptor.clone())
            .collect()
    }
}

impl Drop for SapListener {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

fn spawn_receive_thread(
    socket: UdpSocket,
    sessions: Arc<Mutex<HashMap<SessionKey, TrackedSession>>>,
    stop: Arc<AtomicBool>,
) {
    thread::spawn(move || {
        let mut buf = [0u8; RECV_BUFFER_SIZE];

        while !stop.load(Ordering::Relaxed) {
            let len = match socket.recv_from(&mut buf) {
                Ok((len, _src)) => len,
                Err(e)
                    if e.kind() == std::io::ErrorKind::WouldBlock
                        || e.kind() == std::io::ErrorKind::TimedOut =>
                {
                    continue;
                }
                Err(_) => continue,
            };

            let Some(message) = parse_sap_packet(&buf[..len]) else {
                continue;
            };

            let Ok(mut sessions) = sessions.lock() else {
                continue;
            };
            match message {
                SapMessage::Delete(key) => {
                    sessions.remove(&key);
                }
                SapMessage::Announce(key, sdp) => {
                    if let Some(descriptor) = parse_sdp_for_aes67(&sdp) {
                        sessions.insert(
                            key,
                            TrackedSession {
                                descriptor,
                                last_seen: Instant::now(),
                            },
                        );
                    }
                }
            }
        }
    });
}

enum SapMessage {
    Announce(SessionKey, String),
    Delete(SessionKey),
}

/// SAPパケット(RFC 2974)をパースする。IPv6発信元・暗号化・圧縮された
/// パケットは(v1では)非対応としてスキップする。
fn parse_sap_packet(packet: &[u8]) -> Option<SapMessage> {
    if packet.len() < 8 {
        return None;
    }

    let b0 = packet[0];
    let version = b0 >> 5;
    if version != 1 {
        return None;
    }
    let is_ipv6 = (b0 & 0b0001_0000) != 0;
    let is_delete = (b0 & 0b0000_0100) != 0;
    let is_encrypted = (b0 & 0b0000_0010) != 0;
    let is_compressed = (b0 & 0b0000_0001) != 0;
    if is_ipv6 || is_encrypted || is_compressed {
        return None;
    }

    let auth_len_words = packet[1] as usize;
    let msg_id_hash = u16::from_be_bytes([packet[2], packet[3]]);

    let mut offset = 4 + 4; // ヘッダ4バイト + IPv4発信元4バイト
    if offset > packet.len() {
        return None;
    }
    let origin = Ipv4Addr::new(packet[4], packet[5], packet[6], packet[7]);

    offset += auth_len_words * 4;
    if offset > packet.len() {
        return None;
    }

    let key = SessionKey {
        origin,
        msg_id_hash,
    };

    if is_delete {
        return Some(SapMessage::Delete(key));
    }

    // payload type(MIMEタイプのNUL終端文字列)は省略される実装も多い。
    // "v=" で始まっていればSDPが直接始まっているとみなし、そうでなければ
    // 最初のNUL終端文字列を読み飛ばしてSDPとして扱う。
    let rest = &packet[offset..];
    let sdp_bytes = if rest.starts_with(b"v=") {
        rest
    } else if let Some(nul_pos) = rest.iter().position(|&b| b == 0) {
        &rest[nul_pos + 1..]
    } else {
        return None;
    };

    let sdp = String::from_utf8_lossy(sdp_bytes).into_owned();
    if !sdp.trim_start().starts_with("v=") {
        return None;
    }

    Some(SapMessage::Announce(key, sdp))
}

/// SDPから、単一のAES67(L16/L24)音声フローとして解釈できる情報を抽出する。
/// 複数メディアのSDP・音声以外・非対応フォーマットは `None` を返す。
fn parse_sdp_for_aes67(sdp: &str) -> Option<Descriptor> {
    let mut session_name: Option<&str> = None;
    let mut connection_addr: Option<Ipv4Addr> = None;
    let mut media_port: Option<u16> = None;
    let mut media_payload_type: Option<u8> = None;
    // (payload type, encoding名, クロックレート, チャンネル数)
    let mut rtpmap: Option<(u8, String, u32, u16)> = None;

    for line in sdp.lines() {
        let line = line.trim();
        let Some((kind, value)) = line.split_once('=') else {
            continue;
        };

        match kind {
            "s" => session_name = Some(value.trim()),
            "c" => {
                // 例: "IN IP4 239.1.1.1/32"
                let mut parts = value.split_whitespace();
                if parts.next() != Some("IN") || parts.next() != Some("IP4") {
                    continue;
                }
                let Some(addr_part) = parts.next() else {
                    continue;
                };
                let addr_str = addr_part.split('/').next().unwrap_or(addr_part);
                connection_addr = addr_str.parse().ok();
            }
            "m" if media_port.is_none() => {
                // 例: "audio 5004 RTP/AVP 97"(最初のm=行のみを対象とする)
                let mut parts = value.split_whitespace();
                if parts.next() != Some("audio") {
                    // 音声以外のメディア行が最初に来るSDPは非対応として扱う
                    return None;
                }
                media_port = parts.next().and_then(|p| p.parse().ok());
                if parts
                    .next()
                    .is_some_and(|p| p.eq_ignore_ascii_case("RTP/AVP"))
                {
                    media_payload_type = parts.next().and_then(|p| p.parse().ok());
                }
            }
            "a" => {
                if let Some(rest) = value.strip_prefix("rtpmap:") {
                    let mut parts = rest.splitn(2, char::is_whitespace);
                    let Some(pt) = parts.next().and_then(|p| p.parse::<u8>().ok()) else {
                        continue;
                    };
                    let Some(encoding_part) = parts.next() else {
                        continue;
                    };
                    let mut enc_fields = encoding_part.split('/');
                    let Some(encoding) = enc_fields.next() else {
                        continue;
                    };
                    let Some(clock_rate) = enc_fields.next().and_then(|c| c.parse().ok()) else {
                        continue;
                    };
                    let channels = enc_fields.next().and_then(|c| c.parse().ok()).unwrap_or(1);
                    rtpmap = Some((pt, encoding.to_string(), clock_rate, channels));
                }
            }
            _ => {}
        }
    }

    let multicast_addr = connection_addr.filter(|a| a.is_multicast())?;
    let port = media_port?;
    let payload_type = media_payload_type?;
    let (rtpmap_pt, encoding, clock_rate, channels) = rtpmap?;
    if rtpmap_pt != payload_type {
        return None;
    }

    let sample_format = match encoding.as_str() {
        "L16" => Aes67SampleFormat::L16,
        "L24" => Aes67SampleFormat::L24,
        _ => return None,
    };

    let id_key = format!("{}:{}", multicast_addr, port);
    let name = session_name
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .unwrap_or_else(|| format!("AES67 {}", id_key));

    Some(Descriptor {
        id: SourceId::Aes67(id_key),
        name,
        kind: SourceKind::Aes67 {
            config: Aes67FlowConfig {
                multicast_addr,
                port,
                payload_type,
                sample_format,
                channels,
                sample_rate: clock_rate,
            },
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_sdp() -> String {
        [
            "v=0",
            "o=- 123 1 IN IP4 192.0.2.1",
            "s=Console Out L/R",
            "c=IN IP4 239.1.1.1/32",
            "t=0 0",
            "m=audio 5004 RTP/AVP 97",
            "a=rtpmap:97 L24/48000/2",
        ]
        .join("\r\n")
    }

    fn sap_packet(delete: bool, payload: &[u8]) -> Vec<u8> {
        let mut flags = 0b0010_0000; // version=1
        if delete {
            flags |= 0b0000_0100;
        }
        let mut packet = vec![flags, 0]; // auth len = 0
        packet.extend_from_slice(&0x1234u16.to_be_bytes()); // msg id hash
        packet.extend_from_slice(&[192, 0, 2, 1]); // origin
        packet.extend_from_slice(payload);
        packet
    }

    #[test]
    fn parses_announce_with_omitted_payload_type() {
        let sdp = sample_sdp();
        let packet = sap_packet(false, sdp.as_bytes());

        let message = parse_sap_packet(&packet).expect("valid announce packet");
        let SapMessage::Announce(key, parsed_sdp) = message else {
            panic!("expected announce message");
        };
        assert_eq!(key.msg_id_hash, 0x1234);
        assert_eq!(parsed_sdp, sdp);
    }

    #[test]
    fn parses_announce_with_explicit_payload_type() {
        let sdp = sample_sdp();
        let mut payload = b"application/sdp".to_vec();
        payload.push(0);
        payload.extend_from_slice(sdp.as_bytes());
        let packet = sap_packet(false, &payload);

        let message = parse_sap_packet(&packet).expect("valid announce packet");
        let SapMessage::Announce(_, parsed_sdp) = message else {
            panic!("expected announce message");
        };
        assert_eq!(parsed_sdp, sdp);
    }

    #[test]
    fn parses_delete_message() {
        let packet = sap_packet(true, sample_sdp().as_bytes());
        let message = parse_sap_packet(&packet).expect("valid delete packet");
        assert!(matches!(message, SapMessage::Delete(_)));
    }

    #[test]
    fn rejects_ipv6_packets() {
        let mut packet = sap_packet(false, sample_sdp().as_bytes());
        packet[0] |= 0b0001_0000; // A bit = IPv6
        assert!(parse_sap_packet(&packet).is_none());
    }

    #[test]
    fn extracts_aes67_flow_from_sdp() {
        let descriptor = parse_sdp_for_aes67(&sample_sdp()).expect("should parse");
        assert_eq!(descriptor.name, "Console Out L/R");
        assert_eq!(descriptor.id, SourceId::Aes67("239.1.1.1:5004".to_string()));
        let SourceKind::Aes67 { config } = descriptor.kind else {
            panic!("expected Aes67 kind");
        };
        assert_eq!(config.payload_type, 97);
        assert_eq!(config.channels, 2);
        assert_eq!(config.sample_rate, 48_000);
        assert!(matches!(config.sample_format, Aes67SampleFormat::L24));
    }

    #[test]
    fn rejects_non_audio_media() {
        let sdp = sample_sdp().replace("m=audio", "m=video");
        assert!(parse_sdp_for_aes67(&sdp).is_none());
    }

    #[test]
    fn rejects_unsupported_encoding() {
        let sdp = sample_sdp().replace("L24/48000/2", "OPUS/48000/2");
        assert!(parse_sdp_for_aes67(&sdp).is_none());
    }

    #[test]
    fn rejects_non_multicast_connection_address() {
        let sdp = sample_sdp().replace("239.1.1.1", "192.168.1.1");
        assert!(parse_sdp_for_aes67(&sdp).is_none());
    }

    /// 実際にSAPの標準マルチキャストアドレス(224.2.127.254:9875)へ
    /// アナウンスを送信し、`SapListener` がそれを検出できることを確認する
    /// エンドツーエンドのテスト。実機のDante/AES67機器が無くても、
    /// SAP受信パイプライン全体をこのPC上だけで検証できる。
    /// ネットワーク環境に依存するため、通常の `cargo test` では実行せず
    /// `cargo test -- --ignored` で明示的に実行する。
    #[test]
    #[ignore = "requires multicast networking on this machine"]
    fn discovers_flow_over_real_multicast_announce() {
        let listener =
            SapListener::start().expect("failed to start SapListener (is port 9875 free?)");

        let sender = std::net::UdpSocket::bind("0.0.0.0:0").expect("bind sender socket");
        let packet = sap_packet(false, sample_sdp().as_bytes());
        let target = format!("{SAP_ADDR}:{SAP_PORT}");

        let mut found = false;
        for _ in 0..50 {
            sender.send_to(&packet, &target).expect("send SAP packet");
            thread::sleep(Duration::from_millis(50));
            if listener
                .discovered()
                .iter()
                .any(|d| d.name == "Console Out L/R")
            {
                found = true;
                break;
            }
        }

        assert!(
            found,
            "did not discover the announced AES67 flow via SAP within the timeout"
        );
    }
}
