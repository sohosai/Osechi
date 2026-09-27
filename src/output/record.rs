//! 録画。スロット(PVW・PGM・IN 1..8)ごとに、映像(H.264)とマスター音声(24bit PCM)を .mov に書き出す。
//!
//! 録画するスロットは一斉に始める。同じ時刻を0番目のフレームとして数えるので、ファイル同士の時刻が揃う。
//! 録画中にもスロットごとに止めたり加えたりできる。スロットごとに専用のスレッドでエンコード・書き出しを行う。

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use super::mov::{AUDIO_FRAME_BYTES, MovWriter};
pub use super::pipeline::Resolution;

use super::pipeline::{AudioPipeline, Clock, VideoPipeline};
use super::{Taps, h264};
use crate::error::{Context, Result};
use crate::switcher::Slot;

/// スロットごとの録画の設定。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SlotSettings {
    pub resolution: Resolution,
    /// 映像の目標ビットレート(bps)
    pub bitrate: u32,
}

/// 録画の状態と統計。UIに出す。
#[derive(Debug, Clone, Default)]
pub struct Status {
    /// 書き出し先のファイル
    pub path: PathBuf,
    /// これまでに書いたバイト数
    pub bytes: u64,
    /// エンコードや書き込みが間に合わず飛ばしたフレームの累計
    pub dropped_frames: u64,
    /// 音声が足りず無音で埋めた回数の累計
    pub audio_gaps: u64,
    /// 失敗して止まった理由
    pub error: Option<String>,
}

type SharedStatus = Arc<Mutex<Status>>;

fn update(status: &SharedStatus, f: impl FnOnce(&mut Status)) {
    f(&mut status.lock().unwrap_or_else(PoisonError::into_inner));
}

/// 録画の操作口。
#[derive(Default)]
pub struct Recorder {
    take: Option<Take>,
    sessions: HashMap<Slot, Session>,
    /// 止めた後、ファイルを仕上げている最中のスレッド
    finishing: Vec<JoinHandle<()>>,
}

/// 一斉に始めた1回分の録画。
struct Take {
    dir: PathBuf,
    /// ファイル名の先頭に付ける開始時刻
    stamp: String,
    start: Instant,
}

/// 1つのスロットの書き出し。
struct Session {
    stop: Arc<AtomicBool>,
    status: SharedStatus,
    thread: JoinHandle<()>,
}

impl Recorder {
    /// `dir` に、`slots` の各スロットの録画を一斉に始める。既に録画中なら止めてから始め直す。
    pub fn start(
        &mut self,
        dir: &Path,
        slots: impl IntoIterator<Item = (Slot, SlotSettings)>,
        taps: &Taps,
    ) -> Result<()> {
        self.stop();
        fs::create_dir_all(dir).context(format!("failed to create {}", dir.display()))?;
        let take = Take {
            dir: dir.to_path_buf(),
            stamp: chrono::Local::now().format("%Y-%m-%d_%H%M%S").to_string(),
            start: Instant::now(),
        };
        tracing::info!("recording started: {}/{}_*", dir.display(), take.stamp);
        for (slot, settings) in slots {
            let session = Session::spawn(&take, slot, settings, taps, take.start);
            self.sessions.insert(slot, session);
        }
        self.take = Some(take);
        Ok(())
    }

    /// 録画中に `slot` の録画を加える。録画中でなければ何もしない。
    pub fn add(&mut self, slot: Slot, settings: SlotSettings, taps: &Taps) {
        if let Some(take) = &self.take
            && !self.sessions.contains_key(&slot)
        {
            let session = Session::spawn(take, slot, settings, taps, Instant::now());
            self.sessions.insert(slot, session);
        }
    }

    /// `slot` の録画だけを止める。
    pub fn remove(&mut self, slot: Slot) {
        if let Some(session) = self.sessions.remove(&slot) {
            session.stop.store(true, Ordering::Relaxed);
            self.finishing.push(session.thread);
        }
    }

    /// 全ての録画を止める。ファイルの仕上げは各スレッドが続ける(待たない)。
    pub fn stop(&mut self) {
        let slots: Vec<Slot> = self.sessions.keys().copied().collect();
        for slot in slots {
            self.remove(slot);
        }
        if self.take.take().is_some() {
            tracing::info!("recording stopped");
        }
        self.finishing.retain(|thread| !thread.is_finished());
    }

    /// 全ての録画を止め、ファイルを仕上げ終えるまで待つ。アプリの終了時に呼ぶ。
    pub fn finish(&mut self) {
        self.stop();
        for thread in self.finishing.drain(..) {
            let _ = thread.join();
        }
    }

    pub fn is_recording(&self) -> bool {
        self.take.is_some()
    }

    /// 一斉に始めてからの時間。
    pub fn elapsed(&self) -> Option<Duration> {
        self.take.as_ref().map(|take| take.start.elapsed())
    }

    /// `slot` を録画しているか。
    pub fn is_recording_slot(&self, slot: Slot) -> bool {
        self.sessions.contains_key(&slot)
    }

    /// `slot` の録画の状態(録画していなければ `None`)。
    pub fn status(&self, slot: Slot) -> Option<Status> {
        self.sessions.get(&slot).map(|session| {
            session
                .status
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone()
        })
    }
}

impl Session {
    /// `slot` の書き出しスレッドを起動する。`start` の時刻を0番目のフレームとする。
    fn spawn(take: &Take, slot: Slot, settings: SlotSettings, taps: &Taps, start: Instant) -> Self {
        let clock = Clock::starting_at(start);
        let path = unique_path(&take.dir, &format!("{}_{}", take.stamp, file_label(slot)));
        let stop = Arc::new(AtomicBool::new(false));
        let status = Arc::new(Mutex::new(Status {
            path: path.clone(),
            ..Status::default()
        }));
        let (thread_stop, thread_status, taps) =
            (Arc::clone(&stop), Arc::clone(&status), taps.clone());
        let thread = thread::Builder::new()
            .name(format!("record-{}", file_label(slot)))
            .spawn(move || {
                let result = record(
                    &path,
                    slot,
                    settings,
                    &taps,
                    clock,
                    &thread_stop,
                    &thread_status,
                );
                match result {
                    Ok(()) => tracing::info!("recording saved: {}", path.display()),
                    Err(err) => {
                        tracing::error!("recording {} failed: {err:#}", path.display());
                        update(&thread_status, |s| s.error = Some(format!("{err:#}")));
                    }
                }
            })
            .expect("failed to spawn recording thread");
        Self {
            stop,
            status,
            thread,
        }
    }
}

/// ファイル名に使うスロット名(`PVW` `PGM` `IN1` ...)。
fn file_label(slot: Slot) -> String {
    slot.to_string().replace(' ', "")
}

/// `dir/name.mov`。同じ名前のファイルがあれば `_2` `_3` ... を付ける。
fn unique_path(dir: &Path, name: &str) -> PathBuf {
    (1..)
        .map(|n| match n {
            1 => dir.join(format!("{name}.mov")),
            n => dir.join(format!("{name}_{n}.mov")),
        })
        .find(|path| !path.exists())
        .expect("some file name is free")
}

/// 止められるまで `slot` を `path` に書き出し、最後にファイルを仕上げる。
fn record(
    path: &Path,
    slot: Slot,
    settings: SlotSettings,
    taps: &Taps,
    mut clock: Clock,
    stop: &AtomicBool,
    status: &SharedStatus,
) -> Result<()> {
    let size = settings.resolution.size(taps.frame(slot).as_deref());
    let mut video = VideoPipeline::new(taps.clone(), slot, size, settings.bitrate)?;
    let mut audio = AudioPipeline::new(taps);
    let mut writer: Option<MovWriter> = None;
    let mut first = None;

    while !stop.load(Ordering::Relaxed) {
        let index = clock.tick();
        let frame = index - *first.get_or_insert(index);
        let encoded = video.encode(index)?;
        let pcm = to_pcm24(&audio.take(frame + 1));

        if writer.is_none() {
            // 最初のキーフレームに付いてくる SPS・PPS でファイルを作る
            if let Some((sps, pps)) = h264::parameter_sets(&encoded.nals) {
                writer = Some(MovWriter::create(
                    path,
                    video.size(),
                    h264::decoder_config(sps, pps),
                )?);
            }
        }
        let Some(writer) = &mut writer else {
            continue;
        };
        if !encoded.nals.is_empty() {
            writer.push_video(
                frame,
                encoded.keyframe,
                h264::length_prefixed(&encoded.nals),
            )?;
        }
        writer.push_audio(&pcm);
        update(status, |s| {
            s.bytes = writer.bytes_written();
            s.dropped_frames = clock.dropped;
            s.audio_gaps = audio.gaps;
        });
    }

    match writer {
        Some(writer) => writer.finish(),
        None => Ok(()),
    }
}

/// -1.0..=1.0 の f32 を 24bit ビッグエンディアンの PCM にする。
fn to_pcm24(samples: &[f32]) -> Vec<u8> {
    let mut pcm = Vec::with_capacity(samples.len() / 2 * AUDIO_FRAME_BYTES);
    for sample in samples {
        let value = (sample.clamp(-1.0, 1.0) * 8_388_607.0).round() as i32;
        pcm.extend(&value.to_be_bytes()[1..]);
    }
    pcm
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_to_24bit_big_endian() {
        assert_eq!(
            to_pcm24(&[0.0, 1.0, -1.0]),
            [0, 0, 0, 0x7f, 0xff, 0xff, 0x80, 0x00, 0x01]
        );
    }

    #[test]
    fn file_labels_have_no_spaces() {
        assert_eq!(file_label(Slot::Program), "PGM");
        assert_eq!(file_label(Slot::Input(0)), "IN1");
    }

    #[test]
    fn unique_path_avoids_existing_files() {
        let dir = std::env::temp_dir().join(format!("osechi-record-test-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let first = unique_path(&dir, "take_PGM");
        assert_eq!(first.file_name().unwrap(), "take_PGM.mov");
        fs::write(&first, b"").unwrap();
        assert_eq!(
            unique_path(&dir, "take_PGM").file_name().unwrap(),
            "take_PGM_2.mov"
        );
        fs::remove_dir_all(dir).unwrap();
    }

    /// 実時間で3秒録画し、仕上がったファイルの箱の並びを確かめる。
    #[test]
    #[ignore = "records for 3 seconds in real time"]
    fn records_a_finished_mov() {
        let dir = std::env::temp_dir().join(format!("osechi-record-e2e-{}", std::process::id()));
        let taps = Taps::default();
        let mut recorder = Recorder::default();
        let settings = SlotSettings {
            resolution: Resolution::Hd720,
            bitrate: 2_000_000,
        };
        recorder
            .start(&dir, [(Slot::Program, settings)], &taps)
            .unwrap();
        thread::sleep(Duration::from_secs(3));
        let path = recorder.status(Slot::Program).unwrap().path;
        recorder.finish();
        let bytes = fs::read(&path).unwrap();
        assert!(bytes.windows(4).any(|w| w == b"moov"));
        fs::remove_dir_all(dir).unwrap();
    }
}
