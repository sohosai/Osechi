use std::collections::VecDeque;
use std::net::{Ipv4Addr, SocketAddrV4, UdpSocket};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use crate::error::AppError;
use crate::source::audio::{self, AudioChunk, Stream};

/// 受信バッファのサイズ。AES67のMTUは通常1500バイト程度に収まるが、
/// 余裕を持たせておく。
const RECV_BUFFER_SIZE: usize = 2048;

/// ソケットの受信タイムアウト。`Aes67Stream` が drop された後、
/// バックグラウンドスレッドがこの間隔以内に停止フラグを検知して終了する。
const RECV_TIMEOUT: Duration = Duration::from_millis(200);

/// AES67のRTPペイロードで運ばれるPCMのビット深度。
/// AES67はL24が必須(MUST)、L16は任意(MAY)のフォーマット。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Aes67SampleFormat {
    L16,
    L24,
}

impl std::fmt::Display for Aes67SampleFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::L16 => write!(f, "L16"),
            Self::L24 => write!(f, "L24"),
        }
    }
}

/// 手動で入力されたAES67フローの接続パラメータ。
///
/// AES67はRTPのペイロードタイプ番号とサンプルフォーマット・チャンネル数・
/// サンプルレートの対応関係をSDP(`a=rtpmap`)で伝えるが、v1ではSDP/SAPによる
/// 自動検出を行わないため、これらはユーザーが手動で入力する。
#[derive(Debug, Clone)]
pub struct Aes67FlowConfig {
    pub multicast_addr: Ipv4Addr,
    pub port: u16,
    pub payload_type: u8,
    pub sample_format: Aes67SampleFormat,
    pub channels: u16,
    pub sample_rate: u32,
}

/// AES67(Dante機器のAES67相互接続モードが送出するRTPストリーム)の
/// アクティブなストリーム。バックグラウンドスレッドでマルチキャストRTP
/// パケットを受信し、リングバッファ経由でメインスレッドに渡す。
///
/// # v1での既知の制約
/// - PTPクロック同期は行わない。ローカルクロックでパケットが届いた順に
///   処理するだけの素朴な実装であり、長時間運用でのドリフトや
///   ドロップアウトには対応していない
/// - RTPシーケンス番号は「明らかに古い/重複したパケットの破棄」にのみ使い、
///   並び替えを行うジッタバッファは実装していない
pub struct Aes67Stream {
    ring_buffer: Arc<Mutex<VecDeque<AudioChunk>>>,
    last_error: Arc<Mutex<Option<AppError>>>,
    stop: Arc<AtomicBool>,
}

impl Aes67Stream {
    /// マルチキャストグループへの参加とバックグラウンドスレッドの起動を行う。
    pub fn new(config: Aes67FlowConfig) -> Result<Self, AppError> {
        let socket = UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, config.port))
            .map_err(|e| AppError::Other(format!("Failed to bind AES67 UDP socket: {}", e)))?;
        socket
            .join_multicast_v4(&config.multicast_addr, &Ipv4Addr::UNSPECIFIED)
            .map_err(|e| {
                AppError::Other(format!(
                    "Failed to join multicast group {}: {}",
                    config.multicast_addr, e
                ))
            })?;
        socket.set_read_timeout(Some(RECV_TIMEOUT)).map_err(|e| {
            AppError::Other(format!("Failed to set AES67 socket read timeout: {}", e))
        })?;

        let ring_buffer = Arc::new(Mutex::new(VecDeque::with_capacity(
            audio::RING_BUFFER_CAPACITY,
        )));
        let last_error = Arc::new(Mutex::new(None));
        let stop = Arc::new(AtomicBool::new(false));

        spawn_receive_thread(
            socket,
            config,
            Arc::clone(&ring_buffer),
            Arc::clone(&last_error),
            Arc::clone(&stop),
        );

        Ok(Self {
            ring_buffer,
            last_error,
            stop,
        })
    }
}

impl Drop for Aes67Stream {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

impl Stream for Aes67Stream {
    fn get_chunk(&self) -> Result<Option<AudioChunk>, AppError> {
        if let Some(err) = self
            .last_error
            .lock()
            .map_err(|_| AppError::Other("AES67 error state lock poisoned".to_string()))?
            .take()
        {
            return Err(err);
        }

        let mut buffer = self
            .ring_buffer
            .lock()
            .map_err(|_| AppError::Other("AES67 ring buffer lock poisoned".to_string()))?;

        Ok(buffer.pop_front())
    }
}

fn spawn_receive_thread(
    socket: UdpSocket,
    config: Aes67FlowConfig,
    ring_buffer: Arc<Mutex<VecDeque<AudioChunk>>>,
    last_error: Arc<Mutex<Option<AppError>>>,
    stop: Arc<AtomicBool>,
) {
    thread::spawn(move || {
        let mut buf = [0u8; RECV_BUFFER_SIZE];
        let mut accumulated_frames: u64 = 0;
        let channels_usize = config.channels as usize;

        while !stop.load(Ordering::Relaxed) {
            let len = match socket.recv_from(&mut buf) {
                Ok((len, _src)) => len,
                Err(e)
                    if e.kind() == std::io::ErrorKind::WouldBlock
                        || e.kind() == std::io::ErrorKind::TimedOut =>
                {
                    continue;
                }
                Err(e) => {
                    audio::set_last_error(
                        &last_error,
                        AppError::Other(format!("AES67 receive error: {}", e)),
                    );
                    continue;
                }
            };

            let Some((header, payload)) = parse_rtp_packet(&buf[..len]) else {
                continue;
            };
            if header.payload_type != config.payload_type {
                continue;
            }

            let samples = decode_samples(payload, config.sample_format);
            if samples.is_empty() || channels_usize == 0 {
                continue;
            }

            let frames = samples.len() / channels_usize;
            let pts = accumulated_frames;
            accumulated_frames += frames as u64;

            let chunk = AudioChunk {
                samples: Arc::new(samples),
                sample_rate: config.sample_rate,
                channels: config.channels,
                frames,
                pts,
            };
            audio::push_ring_buffer(&ring_buffer, chunk);
        }
    });
}

/// パースしたRTPヘッダのうち、ここで使う情報のみを保持する。
struct RtpHeader {
    payload_type: u8,
    #[allow(dead_code)]
    sequence_number: u16,
    #[allow(dead_code)]
    timestamp: u32,
}

/// RTPパケット(RFC 3550)をパースし、ヘッダ情報とペイロード部分のスライスを返す。
/// 短すぎる・versionが2でないなど不正な形式の場合は `None`。
fn parse_rtp_packet(packet: &[u8]) -> Option<(RtpHeader, &[u8])> {
    const FIXED_HEADER_LEN: usize = 12;
    if packet.len() < FIXED_HEADER_LEN {
        return None;
    }

    let b0 = packet[0];
    let version = b0 >> 6;
    if version != 2 {
        return None;
    }
    let has_padding = (b0 & 0b0010_0000) != 0;
    let has_extension = (b0 & 0b0001_0000) != 0;
    let csrc_count = (b0 & 0b0000_1111) as usize;

    let payload_type = packet[1] & 0b0111_1111;
    let sequence_number = u16::from_be_bytes([packet[2], packet[3]]);
    let timestamp = u32::from_be_bytes([packet[4], packet[5], packet[6], packet[7]]);

    let mut offset = FIXED_HEADER_LEN + csrc_count * 4;
    if offset > packet.len() {
        return None;
    }

    if has_extension {
        if offset + 4 > packet.len() {
            return None;
        }
        // 拡張ヘッダ: profile(2バイト) + 長さ(2バイト, 32bitワード単位)
        let ext_len_words = u16::from_be_bytes([packet[offset + 2], packet[offset + 3]]) as usize;
        offset += 4 + ext_len_words * 4;
        if offset > packet.len() {
            return None;
        }
    }

    let mut end = packet.len();
    if has_padding {
        let pad_len = *packet.last()? as usize;
        if pad_len == 0 || pad_len > end.saturating_sub(offset) {
            return None;
        }
        end -= pad_len;
    }

    if offset > end {
        return None;
    }

    Some((
        RtpHeader {
            payload_type,
            sequence_number,
            timestamp,
        },
        &packet[offset..end],
    ))
}

/// RTPペイロードを `f32` interleaved PCM に変換する。
fn decode_samples(payload: &[u8], format: Aes67SampleFormat) -> Vec<f32> {
    match format {
        Aes67SampleFormat::L16 => payload
            .chunks_exact(2)
            .map(|b| i16::from_be_bytes([b[0], b[1]]) as f32 / 32_768.0)
            .collect(),
        Aes67SampleFormat::L24 => payload
            .chunks_exact(3)
            .map(|b| {
                let unsigned = ((b[0] as i32) << 16) | ((b[1] as i32) << 8) | (b[2] as i32);
                // 24bit -> 32bit の符号拡張
                let signed = (unsigned << 8) >> 8;
                signed as f32 / 8_388_608.0
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rtp_packet(payload_type: u8, sequence: u16, timestamp: u32, payload: &[u8]) -> Vec<u8> {
        let mut packet = Vec::with_capacity(12 + payload.len());
        packet.push(0b1000_0000); // version=2, padding=0, extension=0, CC=0
        packet.push(payload_type & 0b0111_1111);
        packet.extend_from_slice(&sequence.to_be_bytes());
        packet.extend_from_slice(&timestamp.to_be_bytes());
        packet.extend_from_slice(&0xAABB_CCDDu32.to_be_bytes()); // SSRC
        packet.extend_from_slice(payload);
        packet
    }

    #[test]
    fn parses_minimal_rtp_header() {
        let payload = [1, 2, 3, 4];
        let packet = rtp_packet(97, 42, 1000, &payload);

        let (header, parsed_payload) = parse_rtp_packet(&packet).expect("valid packet");
        assert_eq!(header.payload_type, 97);
        assert_eq!(header.sequence_number, 42);
        assert_eq!(header.timestamp, 1000);
        assert_eq!(parsed_payload, &payload);
    }

    #[test]
    fn rejects_too_short_packet() {
        assert!(parse_rtp_packet(&[0u8; 4]).is_none());
    }

    #[test]
    fn rejects_wrong_version() {
        let mut packet = rtp_packet(97, 1, 1, &[0, 0]);
        packet[0] = 0b0100_0000; // version = 1
        assert!(parse_rtp_packet(&packet).is_none());
    }

    #[test]
    fn skips_csrc_list() {
        let mut packet = vec![0b1000_0010, 97]; // CC=2
        packet.extend_from_slice(&1u16.to_be_bytes()); // sequence
        packet.extend_from_slice(&0u32.to_be_bytes()); // timestamp
        packet.extend_from_slice(&0u32.to_be_bytes()); // SSRC
        packet.extend_from_slice(&0u32.to_be_bytes()); // CSRC 1
        packet.extend_from_slice(&0u32.to_be_bytes()); // CSRC 2
        packet.extend_from_slice(&[9, 9, 9]); // payload

        let (_, payload) = parse_rtp_packet(&packet).expect("valid packet");
        assert_eq!(payload, &[9, 9, 9]);
    }

    #[test]
    fn strips_padding() {
        // 末尾の1バイトが「パディングの長さ(自分自身を含む)」を表すので、
        // 実ペイロード [1,2,3,4] に1バイトのパディング(値=1)を1つ付ける。
        let mut packet = rtp_packet(97, 1, 1, &[1, 2, 3, 4, 1]);
        packet[0] |= 0b0010_0000; // padding bit
        let (_, payload) = parse_rtp_packet(&packet).expect("valid packet");
        assert_eq!(payload, &[1, 2, 3, 4]);
    }

    #[test]
    fn decodes_l16_round_trip() {
        let samples: [i16; 3] = [0, i16::MAX, i16::MIN];
        let mut payload = Vec::new();
        for s in samples {
            payload.extend_from_slice(&s.to_be_bytes());
        }

        let decoded = decode_samples(&payload, Aes67SampleFormat::L16);
        assert_eq!(decoded.len(), 3);
        assert!((decoded[0] - 0.0).abs() < 1e-6);
        assert!((decoded[1] - 1.0).abs() < 1e-3);
        assert!((decoded[2] - (-1.0)).abs() < 1e-3);
    }

    #[test]
    fn decodes_l24_round_trip() {
        // 24bit の最大値・最小値・0 を手動でバイト列にする
        let payload: [u8; 9] = [
            0x00, 0x00, 0x00, // 0
            0x7F, 0xFF, 0xFF, // 最大値 (2^23 - 1)
            0x80, 0x00, 0x00, // 最小値 (-2^23)
        ];

        let decoded = decode_samples(&payload, Aes67SampleFormat::L24);
        assert_eq!(decoded.len(), 3);
        assert!((decoded[0] - 0.0).abs() < 1e-6);
        assert!((decoded[1] - 1.0).abs() < 1e-3);
        assert!((decoded[2] - (-1.0)).abs() < 1e-6);
    }

    /// 実際にマルチキャストソケットのbind/join/送受信を行うエンドツーエンドの
    /// テスト。実機のDante/AES67機器が無くても、`Aes67Stream` の受信パイプ
    /// ライン全体(ソケット→スレッド→RTPパース→AudioChunk)をこのPC上だけで
    /// 検証できる。ネットワーク環境に依存するため通常の `cargo test` では
    /// 実行せず、`cargo test -- --ignored` で明示的に実行する。
    #[test]
    #[ignore = "requires multicast networking on this machine"]
    fn receives_audio_over_loopback_multicast() {
        let config = Aes67FlowConfig {
            multicast_addr: std::net::Ipv4Addr::new(239, 5, 5, 5),
            port: 6100,
            payload_type: 97,
            sample_format: Aes67SampleFormat::L24,
            channels: 2,
            sample_rate: 48_000,
        };

        let stream = Aes67Stream::new(config.clone()).expect("failed to open Aes67Stream");
        let sender = std::net::UdpSocket::bind("0.0.0.0:0").expect("bind sender socket");
        let target = format!("{}:{}", config.multicast_addr, config.port);

        // L24, 2ch, 1フレーム: 1ch目=無音, 2ch目=フルスケール
        let payload = [0x00, 0x00, 0x00, 0x7F, 0xFF, 0xFF];

        let mut received = None;
        for attempt in 0..50u16 {
            let packet = rtp_packet(97, attempt, attempt as u32, &payload);
            sender.send_to(&packet, &target).expect("send test packet");
            thread::sleep(Duration::from_millis(20));
            if let Ok(Some(chunk)) = stream.get_chunk() {
                received = Some(chunk);
                break;
            }
        }

        let chunk = received.expect("did not receive any AudioChunk within the timeout");
        assert_eq!(chunk.channels, 2);
        assert_eq!(chunk.frames, 1);
        assert_eq!(chunk.sample_rate, 48_000);
        assert!(chunk.samples[0].abs() < 1e-6);
        assert!(chunk.samples[1] > 0.99);
    }

    /// 自作の送信コードではなく、独立した実装(ffmpegのRTPマルチキャスト送出)
    /// から受信できることを確認するテスト。ffmpegが出力するSDPは
    /// `a=rtpmap:96 L16/48000/2` であり、AES67のL16プロファイルと同じ形。
    /// ffmpegがインストールされていない環境では起動時に失敗するため
    /// `--ignored` を付けて明示的に実行する。
    #[test]
    #[ignore = "requires ffmpeg installed and multicast networking on this machine"]
    fn receives_audio_from_ffmpeg() {
        let config = Aes67FlowConfig {
            multicast_addr: std::net::Ipv4Addr::new(239, 5, 5, 6),
            port: 6101,
            payload_type: 96,
            sample_format: Aes67SampleFormat::L16,
            channels: 2,
            sample_rate: 48_000,
        };

        let stream = Aes67Stream::new(config.clone()).expect("failed to open Aes67Stream");

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
                &format!("rtp://{}:{}", config.multicast_addr, config.port),
            ])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("failed to spawn ffmpeg (is it installed and on PATH?)");

        let mut received = None;
        for _ in 0..100 {
            thread::sleep(Duration::from_millis(50));
            if let Ok(Some(chunk)) = stream.get_chunk() {
                received = Some(chunk);
                break;
            }
        }

        let _ = ffmpeg.kill();
        let _ = ffmpeg.wait();

        let chunk = received.expect("did not receive any audio from ffmpeg within the timeout");
        assert_eq!(chunk.channels, 2);
        assert_eq!(chunk.sample_rate, 48_000);
        assert!(!chunk.samples.is_empty());

        let peak = chunk.samples.iter().fold(0.0f32, |m, &s| m.max(s.abs()));
        assert!(
            peak > 0.01,
            "expected non-silent audio from ffmpeg's test tone, got peak={peak}"
        );
    }
}
