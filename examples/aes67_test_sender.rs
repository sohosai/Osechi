//! AES67のテスト用RTP送信ツール。
//!
//! 実機のDante/AES67機器がまだ無い開発サイクルで、Osechi側のAES67受信
//! パイプライン(ソケット受信→RTPパース→AudioChunk→ミキサーのメーター)を
//! 1台のPC上で確認するための開発用ツール。合成サイン波をL24 PCMの
//! RTPパケットに詰めてマルチキャスト送信するだけで、実際のDante/AES67
//! プロトコル(PTP・ルーティング制御)は一切扱わない。
//!
//! 使い方:
//!   cargo run --example aes67_test_sender -- 239.1.1.1:5004 [チャンネル数]
//!
//! チャンネル数は省略時2。チャンネルごとに周波数を変える(1ch目 440Hz, 2ch目 880Hz, ...)ので、
//! 多チャンネル受信でチャンネルが正しく分かれているかを耳とメーターで確かめられる。
//!
//! Osechi側は Sources dock の「+ Add AES67 Source」で、このツールの
//! 設定(指定したチャンネル数 / 48000Hz / L24 / Payload Type 97)に合わせて同じ
//! マルチキャストアドレス・ポートを追加する。

use std::env;
use std::f32::consts::PI;
use std::net::UdpSocket;
use std::thread;
use std::time::{Duration, Instant};

const SAMPLE_RATE: u32 = 48_000;
const DEFAULT_CHANNELS: usize = 2;
const PAYLOAD_TYPE: u8 = 97;
/// 1パケットあたりのフレーム数(48kHzで1ms分。AES67の標準のptime)。
/// 多チャンネルでも1パケットがMTU(1500バイト)に収まるよう短くしている(L24・8chで1164バイト)。
const FRAMES_PER_PACKET: usize = 48;
/// 1ch目の周波数。nch目はこのn倍にする。
const BASE_TONE_HZ: f32 = 440.0;
/// 約 -12dBFS。耳やメーターの確認用途では十分な大きさで、割れない。
const AMPLITUDE: f32 = 0.25;

fn main() {
    let mut args = env::args().skip(1);
    let Some(target) = args.next() else {
        eprintln!("Usage: aes67_test_sender <multicast_ip>:<port> [channels]");
        eprintln!("Example: aes67_test_sender 239.1.1.1:5004 8");
        std::process::exit(1);
    };
    let channels = match args.next() {
        None => DEFAULT_CHANNELS,
        Some(arg) => match arg.parse() {
            Ok(n) if n >= 1 => n,
            _ => {
                eprintln!("channels must be a positive integer: {arg}");
                std::process::exit(1);
            }
        },
    };

    let socket = UdpSocket::bind("0.0.0.0:0").expect("failed to bind UDP socket");
    socket
        .set_multicast_ttl_v4(4)
        .expect("failed to set multicast TTL");

    println!(
        "Sending test tones ({channels}ch / {SAMPLE_RATE}Hz / L24 / PT={PAYLOAD_TYPE}) to {target}"
    );
    for ch in 0..channels {
        println!("  Ch {}: {}Hz", ch + 1, tone_hz(ch));
    }
    println!("Add it in Osechi's Sources dock with matching settings. Press Ctrl+C to stop.");

    let mut sequence: u16 = 0;
    let mut timestamp: u32 = 0;
    let mut phases = vec![0.0f32; channels];
    let packet_interval =
        Duration::from_micros(FRAMES_PER_PACKET as u64 * 1_000_000 / SAMPLE_RATE as u64);

    // sleepの精度(特にWindows)では1msごとに正確には起きられないので、送るべき時刻を積み上げ、
    // 遅れた分はまとめて送って平均の送出レートを実時間に合わせる。
    let mut next = Instant::now();
    loop {
        let packet = build_packet(sequence, timestamp, &mut phases);
        if let Err(e) = socket.send_to(&packet, &target) {
            eprintln!("send failed: {e}");
        }

        sequence = sequence.wrapping_add(1);
        timestamp = timestamp.wrapping_add(FRAMES_PER_PACKET as u32);
        next += packet_interval;
        if let Some(wait) = next.checked_duration_since(Instant::now()) {
            thread::sleep(wait);
        }
    }
}

/// `ch` チャンネル目(0始まり)のテストトーンの周波数。
fn tone_hz(ch: usize) -> f32 {
    BASE_TONE_HZ * (ch + 1) as f32
}

/// RTPヘッダ(12バイト) + L24 interleaved PCM のパケットを1つ組み立てる。
/// `phases` はチャンネルごとのサイン波の位相で、チャンネル数もその長さで決まる。
fn build_packet(sequence: u16, timestamp: u32, phases: &mut [f32]) -> Vec<u8> {
    let mut packet = Vec::with_capacity(12 + FRAMES_PER_PACKET * phases.len() * 3);
    packet.push(0b1000_0000); // version=2, padding=0, extension=0, CSRC count=0
    packet.push(PAYLOAD_TYPE & 0b0111_1111);
    packet.extend_from_slice(&sequence.to_be_bytes());
    packet.extend_from_slice(&timestamp.to_be_bytes());
    packet.extend_from_slice(&0x0AE5_67AAu32.to_be_bytes()); // SSRC(固定のダミー値)

    for _ in 0..FRAMES_PER_PACKET {
        for (ch, phase) in phases.iter_mut().enumerate() {
            let sample = (phase.sin() * AMPLITUDE * 8_388_607.0) as i32;
            *phase += 2.0 * PI * tone_hz(ch) / SAMPLE_RATE as f32;
            if *phase > 2.0 * PI {
                *phase -= 2.0 * PI;
            }

            packet.push(((sample >> 16) & 0xFF) as u8);
            packet.push(((sample >> 8) & 0xFF) as u8);
            packet.push((sample & 0xFF) as u8);
        }
    }

    packet
}
