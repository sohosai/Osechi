//! AES67のテスト用RTP送信ツール。
//!
//! 実機のDante/AES67機器がまだ無い開発サイクルで、Osechi側のAES67受信
//! パイプライン(ソケット受信→RTPパース→AudioChunk→ミキサーのメーター)を
//! 1台のPC上で確認するための開発用ツール。合成サイン波をL24 PCMの
//! RTPパケットに詰めてマルチキャスト送信するだけで、実際のDante/AES67
//! プロトコル(PTP・ルーティング制御)は一切扱わない。
//!
//! 使い方:
//!   cargo run --example aes67_test_sender -- 239.1.1.1:5004
//!
//! Osechi側は Sources dock の「+ Add AES67 Source」で、このツールの
//! デフォルト値(2ch / 48000Hz / L24 / Payload Type 97)に合わせて同じ
//! マルチキャストアドレス・ポートを追加する。

use std::env;
use std::f32::consts::PI;
use std::net::UdpSocket;
use std::thread;
use std::time::Duration;

const SAMPLE_RATE: u32 = 48_000;
const CHANNELS: u16 = 2;
const PAYLOAD_TYPE: u8 = 97;
/// 1パケットあたりのフレーム数(48kHzで4ms分)。
const FRAMES_PER_PACKET: usize = 192;
const TONE_HZ: f32 = 440.0;
/// 約 -12dBFS。耳やメーターの確認用途では十分な大きさで、割れない。
const AMPLITUDE: f32 = 0.25;

fn main() {
    let target = match env::args().nth(1) {
        Some(target) => target,
        None => {
            eprintln!("Usage: aes67_test_sender <multicast_ip>:<port>");
            eprintln!("Example: aes67_test_sender 239.1.1.1:5004");
            std::process::exit(1);
        }
    };

    let socket = UdpSocket::bind("0.0.0.0:0").expect("failed to bind UDP socket");
    socket
        .set_multicast_ttl_v4(4)
        .expect("failed to set multicast TTL");

    println!(
        "Sending {TONE_HZ}Hz test tone ({CHANNELS}ch / {SAMPLE_RATE}Hz / L24 / PT={PAYLOAD_TYPE}) to {target}"
    );
    println!("Add it in Osechi's Sources dock with matching settings. Press Ctrl+C to stop.");

    let mut sequence: u16 = 0;
    let mut timestamp: u32 = 0;
    let mut phase: f32 = 0.0;
    let phase_step = 2.0 * PI * TONE_HZ / SAMPLE_RATE as f32;
    let packet_interval =
        Duration::from_micros(FRAMES_PER_PACKET as u64 * 1_000_000 / SAMPLE_RATE as u64);

    loop {
        let packet = build_packet(sequence, timestamp, &mut phase, phase_step);
        if let Err(e) = socket.send_to(&packet, &target) {
            eprintln!("send failed: {e}");
        }

        sequence = sequence.wrapping_add(1);
        timestamp = timestamp.wrapping_add(FRAMES_PER_PACKET as u32);
        thread::sleep(packet_interval);
    }
}

/// RTPヘッダ(12バイト) + L24 interleaved PCM のパケットを1つ組み立てる。
fn build_packet(sequence: u16, timestamp: u32, phase: &mut f32, phase_step: f32) -> Vec<u8> {
    let mut packet = Vec::with_capacity(12 + FRAMES_PER_PACKET * CHANNELS as usize * 3);
    packet.push(0b1000_0000); // version=2, padding=0, extension=0, CSRC count=0
    packet.push(PAYLOAD_TYPE & 0b0111_1111);
    packet.extend_from_slice(&sequence.to_be_bytes());
    packet.extend_from_slice(&timestamp.to_be_bytes());
    packet.extend_from_slice(&0x0AE5_67AAu32.to_be_bytes()); // SSRC(固定のダミー値)

    for _ in 0..FRAMES_PER_PACKET {
        let sample = (phase.sin() * AMPLITUDE * 8_388_607.0) as i32;
        *phase += phase_step;
        if *phase > 2.0 * PI {
            *phase -= 2.0 * PI;
        }

        for _ in 0..CHANNELS {
            packet.push(((sample >> 16) & 0xFF) as u8);
            packet.push(((sample >> 8) & 0xFF) as u8);
            packet.push((sample & 0xFF) as u8);
        }
    }

    packet
}
