//! AES67(RTPマルチキャストで流れる非圧縮PCM)。Dante機器のAES67モードなどから受信する。
//!
//! フローの設定([`Config`])は手動で入力するか、SAPの告知から自動で見つける([`Discovery`])。
//!
//! # 既知の制約
//! - PTPによるクロック同期はしない。届いた順にそのまま流すため、長時間ではドリフトしうる。
//! - ジッタバッファは持たず、パケットの並び替えや欠落の補間はしない。

mod discovery;

use std::fmt;
use std::net::{Ipv4Addr, UdpSocket};
use std::str::FromStr;

pub use discovery::Discovery;

use super::{Chunk, FEED_CAPACITY, Kind};
use crate::error::{Context, Error, Result};
use crate::net::{multicast, rtp, sdp};
use crate::source::{Feed, Origin, Producer, Source, SourceId};

/// 受信バッファの大きさ。AES67のパケットは通常MTU(1500バイト)に収まるが余裕を持たせる。
const RECV_BUFFER_SIZE: usize = 2048;

/// RTPペイロードのPCM形式。AES67ではL24が必須、L16が任意。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    L16,
    L24,
}

impl Format {
    pub const ALL: [Self; 2] = [Self::L24, Self::L16];

    /// ビッグエンディアンの符号付き整数PCMを -1.0..=1.0 の f32 に変換する。
    /// 1サンプルに満たない末尾のバイトは無視する。
    pub fn decode(self, payload: &[u8]) -> Vec<f32> {
        match self {
            Self::L16 => payload
                .as_chunks::<2>()
                .0
                .iter()
                .map(|&bytes| f32::from(i16::from_be_bytes(bytes)) / 32_768.0)
                .collect(),
            Self::L24 => payload
                .as_chunks::<3>()
                .0
                .iter()
                // 下位にゼロを詰めて32bitとして読み、算術シフトで符号拡張する
                .map(|&[a, b, c]| (i32::from_be_bytes([a, b, c, 0]) >> 8) as f32 / 8_388_608.0)
                .collect(),
        }
    }
}

impl fmt::Display for Format {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::L16 => "L16",
            Self::L24 => "L24",
        })
    }
}

impl FromStr for Format {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        match s {
            "L16" => Ok(Self::L16),
            "L24" => Ok(Self::L24),
            other => Err(Error::new(format!("unsupported AES67 format: {other}"))),
        }
    }
}

/// AES67フローの受信設定。SDPの `c=` `m=` `a=rtpmap` に相当する。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    /// 宛先のマルチキャストアドレス
    pub addr: Ipv4Addr,
    pub port: u16,
    /// 受け付けるRTPペイロードタイプ。これ以外のパケットは捨てる。
    pub payload_type: u8,
    pub format: Format,
    pub channels: u16,
    pub sample_rate: u32,
}

impl Config {
    /// フローのID。同じ宛先(アドレス:ポート)は同じフローとみなす。
    pub fn id(&self) -> SourceId {
        SourceId::new("aes67", format_args!("{}:{}", self.addr, self.port))
    }

    /// ソース一覧に載せる形にする。`name` が空なら `AES67 <addr>:<port>` と名付ける。
    pub fn into_source(self, name: &str, origin: Origin) -> Source<Kind> {
        let name = match name.trim() {
            "" => format!("AES67 {}:{}", self.addr, self.port),
            name => name.to_string(),
        };
        Source {
            id: self.id(),
            name,
            kind: Kind::Aes67(self),
            origin,
        }
    }
}

impl TryFrom<&sdp::Session> for Config {
    type Error = Error;

    fn try_from(session: &sdp::Session) -> Result<Self> {
        if !session.addr.is_multicast() {
            return Err(Error::new(format!(
                "{} is not a multicast address",
                session.addr
            )));
        }
        if session.channels == 0 {
            return Err(Error::new("channel count is 0"));
        }
        Ok(Self {
            addr: session.addr,
            port: session.port,
            payload_type: session.payload_type,
            format: session.encoding.parse()?,
            channels: session.channels,
            sample_rate: session.clock_rate,
        })
    }
}

/// マルチキャストグループに参加し、専用スレッドで受信し続ける。
pub(super) fn open(config: &Config) -> Result<Feed<Chunk>> {
    let socket =
        multicast::receiver(config.addr, config.port).context("failed to open AES67 flow")?;
    let config = config.clone();
    Ok(Feed::spawn(FEED_CAPACITY, move |producer| {
        receive(&socket, &config, &producer)
    }))
}

fn receive(socket: &UdpSocket, config: &Config, producer: &Producer<Chunk>) {
    let mut buf = [0u8; RECV_BUFFER_SIZE];
    while producer.is_open() {
        match multicast::recv(socket, &mut buf) {
            Ok(Some(len)) => {
                if let Some(chunk) = decode(config, &buf[..len]) {
                    producer.send(Ok(chunk));
                }
            }
            Ok(None) => {}
            Err(err) => {
                producer.send(Err(err).context("AES67 receive error"));
            }
        }
    }
}

/// RTPパケットを音声に変換する。形式が違う・対象外のペイロードタイプ・空のパケットは `None`。
fn decode(config: &Config, packet: &[u8]) -> Option<Chunk> {
    let packet = rtp::Packet::try_from(packet)
        .ok()
        .filter(|packet| packet.payload_type == config.payload_type)?;
    let samples = config.format.decode(packet.payload);
    (!samples.is_empty()).then_some(Chunk {
        samples,
        sample_rate: config.sample_rate,
        channels: config.channels,
    })
}

#[cfg(test)]
mod tests {
    use std::thread;
    use std::time::Duration;

    use super::*;
    use crate::net::rtp::tests::packet;
    use crate::source::Open;

    fn config(addr: Ipv4Addr, port: u16, payload_type: u8, format: Format) -> Config {
        Config {
            addr,
            port,
            payload_type,
            format,
            channels: 2,
            sample_rate: 48_000,
        }
    }

    #[test]
    fn decodes_l16() {
        let payload: Vec<u8> = [0i16, i16::MAX, i16::MIN]
            .iter()
            .flat_map(|s| s.to_be_bytes())
            .collect();
        let decoded = Format::L16.decode(&payload);
        assert_eq!(decoded.len(), 3);
        assert!(decoded[0].abs() < 1e-6);
        assert!((decoded[1] - 1.0).abs() < 1e-3);
        assert!((decoded[2] + 1.0).abs() < 1e-3);
    }

    #[test]
    fn decodes_l24_with_sign_extension() {
        let payload = [
            0x00, 0x00, 0x00, // 0
            0x7F, 0xFF, 0xFF, // 最大値 (2^23 - 1)
            0x80, 0x00, 0x00, // 最小値 (-2^23)
            0xFF, 0xFF, 0xFF, // -1
        ];
        let decoded = Format::L24.decode(&payload);
        assert_eq!(decoded.len(), 4);
        assert!(decoded[0].abs() < 1e-6);
        assert!((decoded[1] - 1.0).abs() < 1e-3);
        assert!((decoded[2] + 1.0).abs() < 1e-6);
        assert!((decoded[3] + 1.0 / 8_388_608.0).abs() < 1e-9);
    }

    #[test]
    fn format_round_trips_through_text() {
        for format in Format::ALL {
            assert_eq!(format.to_string().parse::<Format>().unwrap(), format);
        }
        assert!("OPUS".parse::<Format>().is_err());
    }

    #[test]
    fn decode_filters_payload_type() {
        let config = config(Ipv4Addr::new(239, 1, 1, 1), 5004, 96, Format::L24);
        assert!(decode(&config, &packet(97, 0, 0, &[0x7F, 0xFF, 0xFF])).is_none());
        let chunk = decode(&config, &packet(96, 0, 0, &[0x7F, 0xFF, 0xFF])).unwrap();
        assert_eq!(chunk.channels, 2);
        assert_eq!(chunk.sample_rate, 48_000);
        assert_eq!(chunk.samples.len(), 1);
    }

    #[test]
    fn decode_skips_empty_payload() {
        let config = config(Ipv4Addr::new(239, 1, 1, 1), 5004, 96, Format::L24);
        assert!(decode(&config, &packet(96, 0, 0, &[])).is_none());
    }

    #[test]
    fn source_uses_address_as_id_and_default_name() {
        let source = config(Ipv4Addr::new(239, 1, 1, 1), 5004, 97, Format::L24)
            .into_source(" ", Origin::Manual);
        assert_eq!(source.id, SourceId::new("aes67", "239.1.1.1:5004"));
        assert_eq!(source.name, "AES67 239.1.1.1:5004");
    }

    #[test]
    fn config_from_sdp() {
        let session: sdp::Session = crate::net::sap::tests::SDP.parse().unwrap();
        let config = Config::try_from(&session).unwrap();
        assert_eq!(
            config,
            Config {
                addr: Ipv4Addr::new(239, 1, 1, 1),
                port: 5004,
                payload_type: 97,
                format: Format::L24,
                channels: 2,
                sample_rate: 48_000,
            }
        );
    }

    #[test]
    fn config_from_sdp_rejects_unicast_and_unsupported_format() {
        let mut session: sdp::Session = crate::net::sap::tests::SDP.parse().unwrap();
        session.addr = Ipv4Addr::new(192, 168, 1, 1);
        assert!(Config::try_from(&session).is_err());

        let mut session: sdp::Session = crate::net::sap::tests::SDP.parse().unwrap();
        session.encoding = "OPUS".to_string();
        assert!(Config::try_from(&session).is_err());
    }

    /// 実際にマルチキャストで送受信し、受信パイプライン全体(ソケット→スレッド→RTP解釈→Chunk)を
    /// このPCだけで確認する。ネットワーク環境に依存するため `cargo test -- --ignored` で実行する。
    #[test]
    #[ignore = "requires multicast networking on this machine"]
    fn receives_audio_over_loopback_multicast() {
        let config = config(Ipv4Addr::new(239, 5, 5, 5), 6100, 97, Format::L24);
        let feed = Kind::Aes67(config.clone()).open().expect("open AES67 flow");
        let sender = UdpSocket::bind("0.0.0.0:0").expect("bind sender socket");

        // L24, 2ch, 1フレーム: 1ch目=無音, 2ch目=フルスケール
        let payload = [0x00, 0x00, 0x00, 0x7F, 0xFF, 0xFF];
        let mut received = None;
        for sequence in 0..50u16 {
            let bytes = packet(97, sequence, sequence.into(), &payload);
            sender
                .send_to(&bytes, (config.addr, config.port))
                .expect("send test packet");
            thread::sleep(Duration::from_millis(20));
            received = feed.try_iter().next();
            if received.is_some() {
                break;
            }
        }

        let chunk = received.expect("no audio within the timeout");
        assert_eq!(chunk.channels, 2);
        assert_eq!(chunk.sample_rate, 48_000);
        assert!(chunk.samples[0].abs() < 1e-6);
        assert!(chunk.samples[1] > 0.99);
    }

    /// 独立した実装(ffmpegのRTPマルチキャスト送出, `a=rtpmap:96 L16/48000/2`)から受信できるか確認する。
    /// ffmpegが必要なため `cargo test -- --ignored` で実行する。
    #[test]
    #[ignore = "requires ffmpeg installed and multicast networking on this machine"]
    fn receives_audio_from_ffmpeg() {
        let config = config(Ipv4Addr::new(239, 5, 5, 6), 6101, 96, Format::L16);
        let feed = Kind::Aes67(config.clone()).open().expect("open AES67 flow");

        let mut ffmpeg = std::process::Command::new("ffmpeg")
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-re",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440:sample_rate=48000",
                "-ac",
                "2",
                "-acodec",
                "pcm_s16be",
                "-payload_type",
                "96",
                "-f",
                "rtp",
                &format!("rtp://{}:{}", config.addr, config.port),
            ])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("failed to spawn ffmpeg (is it installed and on PATH?)");

        let mut received = None;
        for _ in 0..100 {
            thread::sleep(Duration::from_millis(50));
            received = feed.try_iter().next();
            if received.is_some() {
                break;
            }
        }
        let _ = ffmpeg.kill();
        let _ = ffmpeg.wait();

        let chunk = received.expect("no audio from ffmpeg within the timeout");
        assert_eq!(chunk.channels, 2);
        assert_eq!(chunk.sample_rate, 48_000);
        let peak = chunk.samples.iter().fold(0.0f32, |m, &s| m.max(s.abs()));
        assert!(peak > 0.01, "expected a test tone, got peak={peak}");
    }
}
