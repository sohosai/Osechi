//! SDP(Session Description Protocol, RFC 4566)。

use std::net::Ipv4Addr;
use std::str::FromStr;

use crate::error::Error;

/// SDPのうち、単一の音声RTPストリームを受信するのに必要な項目。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Session {
    /// `s=` のセッション名(空のこともある)。
    pub name: String,
    /// `c=` の宛先アドレス。
    pub addr: Ipv4Addr,
    /// 最初の `m=audio` のポート。
    pub port: u16,
    /// 最初の `m=audio` のRTPペイロードタイプ。
    pub payload_type: u8,
    /// `a=rtpmap` のエンコーディング名(`L24` など)。
    pub encoding: String,
    /// `a=rtpmap` のクロックレート(音声ではサンプルレート)。
    pub clock_rate: u32,
    /// `a=rtpmap` のチャンネル数(省略時は1)。
    pub channels: u16,
}

impl FromStr for Session {
    type Err = Error;

    /// 最初の `m=` 行だけを対象にし、それが音声でなければエラーにする。
    fn from_str(sdp: &str) -> Result<Self, Error> {
        let missing = |field: &str| Error::new(format!("SDP has no usable {field}"));

        let mut name = String::new();
        let mut addr = None;
        let mut media: Option<(u16, Option<u8>)> = None;
        // (payload type, エンコーディング名, クロックレート, チャンネル数)
        let mut rtpmaps: Vec<(u8, &str, u32, u16)> = Vec::new();

        for line in sdp.lines() {
            let Some((kind, value)) = line.trim().split_once('=') else {
                continue;
            };
            match kind {
                "s" => name = value.trim().to_string(),
                // 例: "IN IP4 239.1.1.1/32"
                "c" => {
                    if let ["IN", "IP4", target, ..] =
                        value.split_whitespace().collect::<Vec<_>>()[..]
                    {
                        addr = target.split('/').next().and_then(|a| a.parse().ok());
                    }
                }
                // 例: "audio 5004 RTP/AVP 97"
                "m" if media.is_none() => {
                    let mut parts = value.split_whitespace();
                    if parts.next() != Some("audio") {
                        return Err(Error::new("first SDP media is not audio"));
                    }
                    let port = parts
                        .next()
                        .and_then(|p| p.parse().ok())
                        .ok_or_else(|| missing("media port"))?;
                    let payload_type = parts
                        .next()
                        .filter(|proto| proto.eq_ignore_ascii_case("RTP/AVP"))
                        .and_then(|_| parts.next())
                        .and_then(|pt| pt.parse().ok());
                    media = Some((port, payload_type));
                }
                // 例: "rtpmap:97 L24/48000/2"
                "a" => {
                    if let Some(rtpmap) = value.strip_prefix("rtpmap:").and_then(parse_rtpmap) {
                        rtpmaps.push(rtpmap);
                    }
                }
                _ => {}
            }
        }

        let (port, payload_type) = media.ok_or_else(|| missing("m=audio line"))?;
        let payload_type = payload_type.ok_or_else(|| missing("RTP/AVP payload type"))?;
        let (_, encoding, clock_rate, channels) = rtpmaps
            .into_iter()
            .find(|(pt, ..)| *pt == payload_type)
            .ok_or_else(|| missing("rtpmap for the media payload type"))?;

        Ok(Self {
            name,
            addr: addr.ok_or_else(|| missing("c=IN IP4 address"))?,
            port,
            payload_type,
            encoding: encoding.to_string(),
            clock_rate,
            channels,
        })
    }
}

/// `97 L24/48000/2` を分解する。チャンネル数は省略時1。
fn parse_rtpmap(value: &str) -> Option<(u8, &str, u32, u16)> {
    let (payload_type, encoding) = value.split_once(char::is_whitespace)?;
    let mut fields = encoding.split('/');
    let name = fields.next()?;
    let clock_rate = fields.next()?.parse().ok()?;
    let channels = fields.next().map_or(Some(1), |c| c.parse().ok())?;
    Some((payload_type.parse().ok()?, name, clock_rate, channels))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::sap::tests::SDP;

    #[test]
    fn parses_audio_session() {
        let session: Session = SDP.parse().unwrap();
        assert_eq!(
            session,
            Session {
                name: "Console Out L/R".to_string(),
                addr: Ipv4Addr::new(239, 1, 1, 1),
                port: 5004,
                payload_type: 97,
                encoding: "L24".to_string(),
                clock_rate: 48_000,
                channels: 2,
            }
        );
    }

    #[test]
    fn picks_rtpmap_matching_media_payload_type() {
        let sdp = SDP.replace("a=rtpmap:97", "a=rtpmap:96 L16/44100\r\na=rtpmap:97");
        let session: Session = sdp.parse().unwrap();
        assert_eq!(session.encoding, "L24");
    }

    #[test]
    fn defaults_to_one_channel() {
        let session: Session = SDP.replace("L24/48000/2", "L24/48000").parse().unwrap();
        assert_eq!(session.channels, 1);
    }

    #[test]
    fn rejects_non_audio_media() {
        assert!(
            SDP.replace("m=audio", "m=video")
                .parse::<Session>()
                .is_err()
        );
    }

    #[test]
    fn rejects_missing_rtpmap() {
        assert!(
            SDP.replace("a=rtpmap:97", "a=rtpmap:98")
                .parse::<Session>()
                .is_err()
        );
    }
}
