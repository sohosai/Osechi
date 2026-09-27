//! RTMP配信のテスト用送信ツール。
//!
//! カメラやミキサーを使わずに、番組出力(`output::rtmp`)から先だけを確認するための開発用ツール。
//! 動くカラーバー(1280x720、出力時に1920x1080へ拡大される)と 440Hz のテストトーンを番組として置き、
//! Osechi本体と同じ経路でRTMPサーバーへ送る。1秒ごとに送信の状態を表示する。
//!
//! 使い方:
//!   cargo run --release --example rtmp_test_sender -- rtmp://127.0.0.1:1935/live/1A [秒数]
//!
//! 受け側の例(ffmpegでファイルに保存する):
//!   ffmpeg -listen 1 -i rtmp://127.0.0.1:1935/live/1A -c copy out.flv

use std::f32::consts::PI;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use osechi::mixer::{Bus, CHANNELS, SAMPLE_RATE};
use osechi::output::Program;
use osechi::output::rtmp::{Rtmp, Settings};
use osechi::source::video::Frame;

const WIDTH: u32 = 1280;
const HEIGHT: u32 = 720;
const TONE_HZ: f32 = 440.0;
/// 約 -12dBFS。
const AMPLITUDE: f32 = 0.25;
const VIDEO_BITRATE: u32 = 8_000_000;

fn main() {
    let mut args = std::env::args().skip(1);
    let Some(url) = args.next() else {
        eprintln!("Usage: rtmp_test_sender <rtmp://host:port/app/stream> [seconds]");
        std::process::exit(1);
    };
    let seconds: u64 = args.next().and_then(|s| s.parse().ok()).unwrap_or(30);
    let target = match url.parse() {
        Ok(target) => target,
        Err(err) => {
            eprintln!("Invalid URL: {err:#}");
            std::process::exit(1);
        }
    };

    let bus = Bus::default();
    let program = Program::new(bus.clone());
    let done = Arc::new(AtomicBool::new(false));
    let video = thread::spawn({
        let (program, done) = (program.clone(), Arc::clone(&done));
        move || generate_video(&program, &done)
    });
    let audio = thread::spawn({
        let done = Arc::clone(&done);
        move || generate_audio(&bus, &done)
    });

    let mut rtmp = Rtmp::default();
    rtmp.start(
        Settings {
            target,
            video_bitrate: VIDEO_BITRATE,
        },
        program,
    );
    for _ in 0..seconds {
        thread::sleep(Duration::from_secs(1));
        let status = rtmp.status();
        println!(
            "{:<10} {:>6.2} Mbps {:>5.1} fps  dropped={} gaps={} reconnects={} {}",
            status.state.to_string(),
            status.bitrate / 1e6,
            status.fps,
            status.dropped_frames,
            status.audio_gaps,
            status.reconnects,
            status.error.unwrap_or_default(),
        );
    }
    rtmp.stop();
    done.store(true, Ordering::Relaxed);
    let _ = video.join();
    let _ = audio.join();
    // 送信スレッドが終了を伝えて切断するのを待つ
    thread::sleep(Duration::from_millis(500));
}

/// 右へ流れるカラーバーを30fpsで番組に置き続ける。
fn generate_video(program: &Program, done: &AtomicBool) {
    const BARS: [[u8; 3]; 7] = [
        [192, 192, 192],
        [192, 192, 0],
        [0, 192, 192],
        [0, 192, 0],
        [192, 0, 192],
        [192, 0, 0],
        [0, 0, 192],
    ];
    let start = Instant::now();
    let mut index = 0u32;
    while !done.load(Ordering::Relaxed) {
        let offset = index * 8;
        let frame = Frame::from_fn(WIDTH, HEIGHT, |x, _| {
            let bar = ((x + offset) % WIDTH) as usize * BARS.len() / WIDTH as usize;
            image::Rgb(BARS[bar])
        });
        program.set_frame(Some(Arc::new(frame)));
        index += 1;
        let due = start + Duration::from_secs(1) * index / 30;
        if let Some(wait) = due.checked_duration_since(Instant::now()) {
            thread::sleep(wait);
        }
    }
}

/// 10msごとに 440Hz のトーンを積む(ミキサーが合成結果を積むのと同じ間隔)。
fn generate_audio(bus: &Bus, done: &AtomicBool) {
    let per_tick = SAMPLE_RATE as usize / 100;
    let start = Instant::now();
    let mut position = 0usize;
    let mut tick = 0u32;
    while !done.load(Ordering::Relaxed) {
        let mut samples = Vec::with_capacity(per_tick * CHANNELS as usize);
        for i in 0..per_tick {
            let t = (position + i) as f32 / SAMPLE_RATE as f32;
            let value = (2.0 * PI * TONE_HZ * t).sin() * AMPLITUDE;
            samples.extend([value; CHANNELS as usize]);
        }
        bus.push(&samples);
        position += per_tick;
        tick += 1;
        let due = start + Duration::from_millis(10) * tick;
        if let Some(wait) = due.checked_duration_since(Instant::now()) {
            thread::sleep(wait);
        }
    }
}
