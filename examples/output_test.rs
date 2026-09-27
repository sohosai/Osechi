//! 番組出力(配信・録画)のテスト用ツール。
//!
//! カメラやミキサーを使わずに、出力(`output::rtmp` / `output::record`)から先だけを確認するための開発用ツール。
//! テスト用の映像と 440Hz のトーンをスロットに置き、Osechi本体と同じ経路で配信・録画する。
//! 1秒ごとに状態を表示する。
//!
//! - PGM: 右へ流れるカラーバー(1280x720)
//! - IN 1: 左へ流れるカラーバー(640x480。Source 解像度・左右の黒帯の確認用)
//! - IN 2..8・PVW: 流れるカラーバー(1280x720。一般的なカメラ相当の負荷を掛ける用)
//!
//! 使い方:
//!   cargo run --release --example output_test -- rtmp rtmp://127.0.0.1:1935/live/1A [秒数]
//!   cargo run --release --example output_test -- record recordings [秒数]
//!   cargo run --release --example output_test -- record-all recordings [秒数]
//!
//! 配信の受け側の例(ffmpegでファイルに保存する):
//!   ffmpeg -listen 1 -i rtmp://127.0.0.1:1935/live/1A -c copy out.flv
//! `record` は PGM(1080p)・IN 1(Source)・PVW(720p)の3本、`record-all` は全10スロット
//! (PVW・PGM は1080p、IN は Source)を書き出す。

use std::f32::consts::PI;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use osechi::mixer::{CHANNELS, Fanout, SAMPLE_RATE};
use osechi::output::Taps;
use osechi::output::record::{Recorder, Resolution, SlotSettings};
use osechi::output::rtmp::{Rtmp, Settings};
use osechi::source::video::Frame;
use osechi::switcher::Slot;

const TONE_HZ: f32 = 440.0;
/// 約 -12dBFS。
const AMPLITUDE: f32 = 0.25;
const BITRATE: u32 = 8_000_000;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (Some(mode), Some(target)) = (args.first(), args.get(1)) else {
        usage();
    };
    let seconds: u64 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(30);

    let audio = Fanout::default();
    let taps = Taps::new(audio.clone());
    let done = Arc::new(AtomicBool::new(false));
    let generators = [
        thread::spawn({
            let (taps, done) = (taps.clone(), Arc::clone(&done));
            move || generate_video(&taps, &done)
        }),
        thread::spawn({
            let done = Arc::clone(&done);
            move || generate_audio(&audio, &done)
        }),
    ];

    match mode.as_str() {
        "rtmp" => run_rtmp(target, &taps, seconds),
        "record" => {
            let slots = [
                (Slot::Program, Resolution::Hd1080),
                (Slot::Input(0), Resolution::Source),
                (Slot::Preview, Resolution::Hd720),
            ];
            run_record(Path::new(target), &taps, &slots, seconds);
        }
        "record-all" => {
            let slots: Vec<(Slot, Resolution)> = Slot::all()
                .map(|slot| match slot {
                    Slot::Input(_) => (slot, Resolution::Source),
                    _ => (slot, Resolution::Hd1080),
                })
                .collect();
            run_record(Path::new(target), &taps, &slots, seconds);
        }
        _ => usage(),
    }

    done.store(true, Ordering::Relaxed);
    for generator in generators {
        let _ = generator.join();
    }
}

fn usage() -> ! {
    eprintln!("Usage: output_test rtmp <rtmp://host:port/app/stream> [seconds]");
    eprintln!("       output_test record <directory> [seconds]");
    eprintln!("       output_test record-all <directory> [seconds]");
    std::process::exit(1);
}

fn run_rtmp(url: &str, taps: &Taps, seconds: u64) {
    let target = url.parse().unwrap_or_else(|err| {
        eprintln!("Invalid URL: {err:#}");
        std::process::exit(1);
    });
    let mut rtmp = Rtmp::default();
    rtmp.start(
        Settings {
            target,
            video_bitrate: BITRATE,
        },
        taps.clone(),
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
    // 送信スレッドが終了を伝えて切断するのを待つ
    thread::sleep(Duration::from_millis(500));
}

fn run_record(dir: &Path, taps: &Taps, slots: &[(Slot, Resolution)], seconds: u64) {
    let slots: Vec<(Slot, SlotSettings)> = slots
        .iter()
        .map(|&(slot, resolution)| {
            let settings = SlotSettings {
                resolution,
                bitrate: BITRATE,
            };
            (slot, settings)
        })
        .collect();
    // 映像が届いてから始める(Source は始めたときのソースの大きさで決まる)
    thread::sleep(Duration::from_millis(200));
    let mut recorder = Recorder::default();
    if let Err(err) = recorder.start(dir, slots.iter().copied(), taps) {
        eprintln!("Failed to start recording: {err:#}");
        std::process::exit(1);
    }
    for _ in 0..seconds {
        thread::sleep(Duration::from_secs(1));
        for &(slot, _) in &slots {
            let status = recorder.status(slot).unwrap_or_default();
            print!(
                "{slot}: {:.1} MB dropped={} gaps={} {}  ",
                status.bytes as f64 / 1e6,
                status.dropped_frames,
                status.audio_gaps,
                status.error.unwrap_or_default()
            );
        }
        println!();
    }
    let paths: Vec<_> = slots
        .iter()
        .filter_map(|(slot, _)| recorder.status(*slot))
        .map(|status| status.path)
        .collect();
    recorder.finish();
    for path in paths {
        println!("saved {}", path.display());
    }
}

/// カラーバーを30fpsで流し続ける(PGM は右へ、IN 1 は左へ、IN 2..8・PVW は用意した絵を順に)。
fn generate_video(taps: &Taps, done: &AtomicBool) {
    // IN 2..8・PVW の分は毎回作ると間に合わないので、動く絵を先に用意して使い回す
    let ring: Vec<Arc<Frame>> = (0..16)
        .map(|i| Arc::new(color_bars(1280, 720, i * 16)))
        .collect();
    let start = Instant::now();
    let mut index = 0u32;
    while !done.load(Ordering::Relaxed) {
        let offset = index * 8;
        let others = (1..8).map(Slot::Input).chain([Slot::Preview]);
        let frames = [
            (Slot::Program, Arc::new(color_bars(1280, 720, offset))),
            (
                Slot::Input(0),
                Arc::new(color_bars(640, 480, 640 * 64 - offset)),
            ),
        ]
        .into_iter()
        .chain(others.enumerate().map(|(i, slot)| {
            (
                slot,
                Arc::clone(&ring[(index as usize + i * 2) % ring.len()]),
            )
        }));
        taps.set_frames(frames);
        index += 1;
        let due = start + Duration::from_secs(1) * index / 30;
        if let Some(wait) = due.checked_duration_since(Instant::now()) {
            thread::sleep(wait);
        }
    }
}

/// 横に `offset` ずらしたカラーバー。
fn color_bars(width: u32, height: u32, offset: u32) -> Frame {
    const BARS: [[u8; 3]; 7] = [
        [192, 192, 192],
        [192, 192, 0],
        [0, 192, 192],
        [0, 192, 0],
        [192, 0, 192],
        [192, 0, 0],
        [0, 0, 192],
    ];
    Frame::from_fn(width, height, |x, _| {
        let bar = ((x + offset) % width) as usize * BARS.len() / width as usize;
        image::Rgb(BARS[bar])
    })
}

/// 10msごとに 440Hz のトーンを配る(ミキサーが合成結果を配るのと同じ間隔)。
fn generate_audio(audio: &Fanout, done: &AtomicBool) {
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
        audio.push(&samples);
        position += per_tick;
        tick += 1;
        let due = start + Duration::from_millis(10) * tick;
        if let Some(wait) = due.checked_duration_since(Instant::now()) {
            thread::sleep(wait);
        }
    }
}
