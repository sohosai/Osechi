//! SAP(Session Announcement Protocol, RFC 2974)。セッション(SDP)をマルチキャストで告知する。

use std::net::Ipv4Addr;

use crate::error::Error;

/// SAPの既定のマルチキャストアドレス。
pub const ADDR: Ipv4Addr = Ipv4Addr::new(224, 2, 127, 254);
/// SAPの既定のポート。
pub const PORT: u16 = 9875;

/// 告知されたセッションを一意に識別するキー(発信元とメッセージIDハッシュ)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Key {
    pub origin: Ipv4Addr,
    pub hash: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Message {
    /// セッションの告知。`sdp` はセッション記述の本文。
    Announce { key: Key, sdp: String },
    /// セッションの削除。
    Delete { key: Key },
}

impl TryFrom<&[u8]> for Message {
    type Error = Error;

    /// IPv6発信元・暗号化・圧縮されたパケットは非対応としてエラーにする。
    fn try_from(packet: &[u8]) -> Result<Self, Error> {
        let invalid = |reason: &str| Error::new(format!("unsupported SAP packet: {reason}"));

        if packet.len() < 8 {
            return Err(invalid("too short"));
        }
        let b0 = packet[0];
        if b0 >> 5 != 1 {
            return Err(invalid("version is not 1"));
        }
        let is_ipv6 = b0 & 0b0001_0000 != 0;
        let is_delete = b0 & 0b0000_0100 != 0;
        let is_encrypted = b0 & 0b0000_0010 != 0;
        let is_compressed = b0 & 0b0000_0001 != 0;
        if is_ipv6 || is_encrypted || is_compressed {
            return Err(invalid("IPv6, encrypted or compressed"));
        }

        let key = Key {
            origin: Ipv4Addr::new(packet[4], packet[5], packet[6], packet[7]),
            hash: u16::from_be_bytes([packet[2], packet[3]]),
        };
        if is_delete {
            return Ok(Self::Delete { key });
        }

        // ヘッダ4バイト + IPv4発信元4バイト + 認証データ(32bitワード単位)
        let auth_len = packet[1] as usize * 4;
        let rest = packet
            .get(8 + auth_len..)
            .ok_or_else(|| invalid("truncated authentication data"))?;

        // payload type(MIMEタイプのNUL終端文字列)は省略する実装も多い。"v=" で始まれば
        // SDP本文が直接始まっているとみなし、そうでなければ最初のNUL終端文字列を読み飛ばす。
        let body = if rest.starts_with(b"v=") {
            rest
        } else {
            let nul = rest
                .iter()
                .position(|&b| b == 0)
                .ok_or_else(|| invalid("missing payload type terminator"))?;
            &rest[nul + 1..]
        };

        let sdp = String::from_utf8_lossy(body).into_owned();
        if !sdp.trim_start().starts_with("v=") {
            return Err(invalid("payload is not SDP"));
        }
        Ok(Self::Announce { key, sdp })
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) const SDP: &str = "v=0\r\no=- 123 1 IN IP4 192.0.2.1\r\ns=Console Out L/R\r\nc=IN IP4 239.1.1.1/32\r\nt=0 0\r\nm=audio 5004 RTP/AVP 97\r\na=rtpmap:97 L24/48000/2";

    /// テスト用に、認証データ無しのSAPv1パケットを組み立てる。
    pub(crate) fn packet(delete: bool, payload: &[u8]) -> Vec<u8> {
        let flags = 0b0010_0000 | if delete { 0b0000_0100 } else { 0 };
        let mut packet = vec![flags, 0];
        packet.extend_from_slice(&0x1234u16.to_be_bytes());
        packet.extend_from_slice(&[192, 0, 2, 1]);
        packet.extend_from_slice(payload);
        packet
    }

    const KEY: Key = Key {
        origin: Ipv4Addr::new(192, 0, 2, 1),
        hash: 0x1234,
    };

    #[test]
    fn parses_announce_with_omitted_payload_type() {
        let message = Message::try_from(packet(false, SDP.as_bytes()).as_slice()).unwrap();
        assert_eq!(
            message,
            Message::Announce {
                key: KEY,
                sdp: SDP.to_string()
            }
        );
    }

    #[test]
    fn parses_announce_with_explicit_payload_type() {
        let mut payload = b"application/sdp\0".to_vec();
        payload.extend_from_slice(SDP.as_bytes());
        let message = Message::try_from(packet(false, &payload).as_slice()).unwrap();
        assert!(matches!(message, Message::Announce { sdp, .. } if sdp == SDP));
    }

    #[test]
    fn parses_delete_message() {
        let message = Message::try_from(packet(true, SDP.as_bytes()).as_slice()).unwrap();
        assert_eq!(message, Message::Delete { key: KEY });
    }

    #[test]
    fn rejects_ipv6_packets() {
        let mut bytes = packet(false, SDP.as_bytes());
        bytes[0] |= 0b0001_0000;
        assert!(Message::try_from(bytes.as_slice()).is_err());
    }

    #[test]
    fn rejects_non_sdp_payload() {
        assert!(Message::try_from(packet(false, b"hello").as_slice()).is_err());
    }
}
