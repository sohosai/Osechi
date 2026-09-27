//! QuickTime(.mov)の書き出し。映像は H.264、音声は 24bit のリニアPCM(`lpcm`)。
//!
//! 録画中にアプリが落ちても失わないよう、OBS の「Hybrid MP4」と同じ方式で書く。
//! - 録画中: 先頭に空の目次(`moov`)を置き、キーフレームごとに「断片」(`moof` + `mdat`)を追記する。
//!   途中で止まっても、そこまでの断片は再生できる(fragmented 形式)。
//! - 終了時: 全サンプルを指す目次を末尾に書き足し、先頭の目次と断片の見出し(`moof`)を `free` に
//!   書き換える。これで断片を使わない普通の .mov になり、どの編集ソフトでも扱える。

use std::fs::File;
use std::io::{Seek, SeekFrom, Write};
use std::path::Path;
use std::sync::mpsc::{self, SyncSender};
use std::thread;

use super::FRAME_RATE;
use crate::error::{Context, Result};
use crate::mixer::{CHANNELS, SAMPLE_RATE};

const VIDEO_TRACK: u32 = 1;
const AUDIO_TRACK: u32 = 2;
/// 目次全体の時間の単位(1/1000秒)。
const MOVIE_TIMESCALE: u32 = 1000;
/// 映像の時間の単位。1フレーム = 1000。
const VIDEO_TIMESCALE: u32 = FRAME_RATE * 1000;
/// 音声1フレーム(全チャンネル分)のバイト数。24bit × チャンネル数。
pub const AUDIO_FRAME_BYTES: usize = 3 * CHANNELS as usize;

/// `moof` の中の、サンプルごとの付加情報(依存関係)。
const SAMPLE_SYNC: u32 = 0x0200_0000;
const SAMPLE_NON_SYNC: u32 = 0x0101_0000;

/// 映像の1サンプル(フレーム)。
struct VideoSample {
    /// 録画を始めてからのフレーム番号
    frame: u64,
    keyframe: bool,
    data: Vec<u8>,
}

/// 書き終えた映像サンプルの情報(終了時の目次に使う)。
struct SampleInfo {
    size: u32,
    /// 長さ(フレーム数)。フレームを飛ばした直後は2以上になる。
    frames: u32,
    keyframe: bool,
}

/// ファイル内の連続したサンプルのまとまり(断片1つ分)。
struct Chunk {
    offset: u64,
    samples: u32,
}

/// .mov の書き出し。[`MovWriter::finish`] を呼ばずに drop した場合は fragmented 形式のまま残る。
pub struct MovWriter {
    file: File,
    /// 書いた内容のディスクへの確定を別スレッドに頼む口(確定は数百ms止まることがあり、
    /// エンコードを待たせないため)。drop するとそのスレッドは終わる。
    sync_requests: SyncSender<()>,
    /// 次に書く位置(=これまでに書いたバイト数)
    position: u64,
    width: u16,
    height: u16,
    /// H.264 のデコーダ設定(`avcC` の中身)
    decoder_config: Vec<u8>,
    /// 終了時に `free` に書き換える箱(先頭の目次と各断片の見出し)の位置
    placeholders: Vec<u64>,
    fragments: u32,
    pending_video: Vec<VideoSample>,
    pending_audio: Vec<u8>,
    video_samples: Vec<SampleInfo>,
    video_chunks: Vec<Chunk>,
    audio_frames: u64,
    audio_chunks: Vec<Chunk>,
}

impl MovWriter {
    /// `path` にファイルを作り、先頭の見出し(`ftyp` と空の目次)を書く。
    pub fn create(
        path: &Path,
        (width, height): (usize, usize),
        decoder_config: Vec<u8>,
    ) -> Result<Self> {
        let file = File::create(path).context(format!("failed to create {}", path.display()))?;
        let syncer = file.try_clone().context("failed to open recording")?;
        let (sync_requests, requests) = mpsc::sync_channel::<()>(1);
        thread::Builder::new()
            .name("record-sync".to_string())
            .spawn(move || {
                for () in requests {
                    if let Err(err) = syncer.sync_data() {
                        tracing::warn!("failed to flush recording to disk: {err}");
                    }
                }
            })
            .context("failed to spawn recording sync thread")?;
        let mut writer = Self {
            file,
            sync_requests,
            position: 0,
            width: width as u16,
            height: height as u16,
            decoder_config,
            placeholders: Vec::new(),
            fragments: 0,
            pending_video: Vec::new(),
            pending_audio: Vec::new(),
            video_samples: Vec::new(),
            video_chunks: Vec::new(),
            audio_frames: 0,
            audio_chunks: Vec::new(),
        };
        let ftyp = Atom::new()
            .bytes(b"qt  ")
            .u32(0x200)
            .bytes(b"qt  ")
            .finish(b"ftyp");
        writer.write(&ftyp)?;
        writer.placeholders.push(writer.position);
        let moov = writer.moov(true);
        writer.write(&moov)?;
        Ok(writer)
    }

    /// 映像の1フレーム(長さ付き NAL ユニット)を加える。キーフレームが来たらそれまでを断片として書き出す。
    pub fn push_video(&mut self, frame: u64, keyframe: bool, data: Vec<u8>) -> Result<()> {
        if keyframe && !self.pending_video.is_empty() {
            self.flush(Some(frame))?;
        }
        self.pending_video.push(VideoSample {
            frame,
            keyframe,
            data,
        });
        Ok(())
    }

    /// 音声(24bit ビッグエンディアンの interleaved PCM)を加える。
    pub fn push_audio(&mut self, pcm: &[u8]) {
        self.pending_audio.extend(pcm);
    }

    /// これまでに書いたバイト数。
    pub fn bytes_written(&self) -> u64 {
        self.position
    }

    /// 残りを書き出し、全体の目次を付けて普通の .mov に仕上げる。
    pub fn finish(mut self) -> Result<()> {
        self.flush(None)?;
        let moov = self.moov(false);
        self.write(&moov)?;
        for offset in std::mem::take(&mut self.placeholders) {
            self.file
                .seek(SeekFrom::Start(offset + 4))
                .and_then(|_| self.file.write_all(b"free"))
                .context("failed to finalize recording")?;
        }
        self.file.sync_all().context("failed to finalize recording")
    }

    fn write(&mut self, bytes: &[u8]) -> Result<()> {
        self.file
            .write_all(bytes)
            .context("failed to write recording")?;
        self.position += bytes.len() as u64;
        Ok(())
    }

    /// 溜まっているサンプルを断片(`moof` + `mdat`)として書き出す。
    /// `next_frame` は次の断片の最初のフレーム番号(最後の断片なら `None`)。
    fn flush(&mut self, next_frame: Option<u64>) -> Result<()> {
        if self.pending_video.is_empty() && self.pending_audio.is_empty() {
            return Ok(());
        }
        let video = std::mem::take(&mut self.pending_video);
        let audio = std::mem::take(&mut self.pending_audio);
        let audio_frames = (audio.len() / AUDIO_FRAME_BYTES) as u64;

        let infos: Vec<SampleInfo> = video
            .iter()
            .enumerate()
            .map(|(i, sample)| {
                let next = video
                    .get(i + 1)
                    .map(|next| next.frame)
                    .or(next_frame)
                    .unwrap_or(sample.frame + 1);
                SampleInfo {
                    size: sample.data.len() as u32,
                    frames: (next - sample.frame).max(1) as u32,
                    keyframe: sample.keyframe,
                }
            })
            .collect();
        let video_bytes: u64 = infos.iter().map(|info| u64::from(info.size)).sum();
        let first_frame = video.first().map_or(0, |sample| sample.frame);

        // データの位置は moof の大きさで決まるので、仮の値で一度組んで大きさを測る
        let moof = |data_start: u32| {
            self.moof(
                first_frame,
                &infos,
                data_start,
                data_start + video_bytes as u32,
                audio_frames,
            )
        };
        let moof_len = moof(0).len() as u32;
        let moof = moof(moof_len + 8);

        let data_start = self.position + u64::from(moof_len) + 8;
        self.placeholders.push(self.position);
        self.write(&moof)?;
        self.write(&((8 + video_bytes + audio.len() as u64) as u32).to_be_bytes())?;
        self.write(b"mdat")?;
        for sample in &video {
            self.write(&sample.data)?;
        }
        self.write(&audio)?;
        // 前の確定がまだ終わっていなければ、それに任せる
        let _ = self.sync_requests.try_send(());

        if !infos.is_empty() {
            self.video_chunks.push(Chunk {
                offset: data_start,
                samples: infos.len() as u32,
            });
        }
        if audio_frames > 0 {
            self.audio_chunks.push(Chunk {
                offset: data_start + video_bytes,
                samples: audio_frames as u32,
            });
        }
        self.video_samples.extend(infos);
        self.audio_frames += audio_frames;
        self.fragments += 1;
        Ok(())
    }

    /// 断片の見出し。`video_offset` `audio_offset` は moof の先頭からのデータの位置。
    fn moof(
        &self,
        first_frame: u64,
        video: &[SampleInfo],
        video_offset: u32,
        audio_offset: u32,
        audio_frames: u64,
    ) -> Vec<u8> {
        const DEFAULT_BASE_IS_MOOF: u32 = 0x02_0000;
        let mut moof = Atom::new().atom(Atom::full(0, 0).u32(self.fragments + 1).finish(b"mfhd"));

        if !video.is_empty() {
            let mut trun = Atom::full(0, 0x001 | 0x100 | 0x200 | 0x400)
                .u32(video.len() as u32)
                .u32(video_offset);
            for sample in video {
                let flags = if sample.keyframe {
                    SAMPLE_SYNC
                } else {
                    SAMPLE_NON_SYNC
                };
                trun = trun.u32(sample.frames * 1000).u32(sample.size).u32(flags);
            }
            moof = moof.atom(
                Atom::new()
                    .atom(
                        Atom::full(0, DEFAULT_BASE_IS_MOOF)
                            .u32(VIDEO_TRACK)
                            .finish(b"tfhd"),
                    )
                    .atom(Atom::full(1, 0).u64(first_frame * 1000).finish(b"tfdt"))
                    .atom(trun.finish(b"trun"))
                    .finish(b"traf"),
            );
        }
        if audio_frames > 0 {
            moof = moof.atom(
                Atom::new()
                    .atom(
                        // 既定の長さ・大きさ・付加情報を持つので、trun は数と位置だけ
                        Atom::full(0, DEFAULT_BASE_IS_MOOF | 0x08 | 0x10 | 0x20)
                            .u32(AUDIO_TRACK)
                            .u32(1)
                            .u32(AUDIO_FRAME_BYTES as u32)
                            .u32(SAMPLE_SYNC)
                            .finish(b"tfhd"),
                    )
                    .atom(Atom::full(1, 0).u64(self.audio_frames).finish(b"tfdt"))
                    .atom(
                        Atom::full(0, 0x001)
                            .u32(audio_frames as u32)
                            .u32(audio_offset)
                            .finish(b"trun"),
                    )
                    .finish(b"traf"),
            );
        }
        moof.finish(b"moof")
    }

    /// 目次。`fragmented` なら中身の無い目次に断片の既定値(`mvex`)を付ける(録画の開始時)。
    /// そうでなければ、書き終えた全サンプルを指す目次にする(終了時)。
    fn moov(&self, fragmented: bool) -> Vec<u8> {
        let video_frames: u64 = self.video_samples.iter().map(|s| u64::from(s.frames)).sum();
        let video_ms = video_frames * 1000 / u64::from(FRAME_RATE);
        let audio_ms = self.audio_frames * 1000 / u64::from(SAMPLE_RATE);

        let mut moov = Atom::new()
            .atom(mvhd(video_ms.max(audio_ms) as u32))
            .atom(self.video_trak(video_ms as u32, (video_frames * 1000) as u32))
            .atom(self.audio_trak(audio_ms as u32));
        if fragmented {
            let trex = |track: u32, duration: u32, size: u32| {
                Atom::full(0, 0)
                    .u32(track)
                    .u32(1)
                    .u32(duration)
                    .u32(size)
                    .u32(0)
                    .finish(b"trex")
            };
            moov = moov.atom(
                Atom::new()
                    .atom(trex(VIDEO_TRACK, 1000, 0))
                    .atom(trex(AUDIO_TRACK, 1, AUDIO_FRAME_BYTES as u32))
                    .finish(b"mvex"),
            );
        }
        moov.finish(b"moov")
    }

    fn video_trak(&self, duration_ms: u32, media_duration: u32) -> Vec<u8> {
        let avc1 = Atom::new()
            .zeros(6)
            .u16(1) // data reference index
            .zeros(16) // version, revision, vendor, temporal/spatial quality
            .u16(self.width)
            .u16(self.height)
            .u32(0x0048_0000) // 72dpi
            .u32(0x0048_0000)
            .u32(0)
            .u16(1) // frames per sample
            .pascal("H.264", 32)
            .u16(24) // depth
            .u16(0xffff) // color table id
            .atom(Atom::new().bytes(&self.decoder_config).finish(b"avcC"))
            // BT.709(エンコーダの色変換と揃える)
            .atom(
                Atom::new()
                    .bytes(b"nclc")
                    .u16(1)
                    .u16(1)
                    .u16(1)
                    .finish(b"colr"),
            )
            .finish(b"avc1");

        let samples = &self.video_samples;
        let mut durations: Vec<(u32, u32)> = Vec::new();
        for sample in samples {
            let delta = sample.frames * 1000;
            match durations.last_mut() {
                Some((count, last)) if *last == delta => *count += 1,
                _ => durations.push((1, delta)),
            }
        }
        let keyframes: Vec<u32> = (1..)
            .zip(samples)
            .filter(|(_, sample)| sample.keyframe)
            .map(|(number, _)| number)
            .collect();
        let stbl = Atom::new()
            .atom(stsd(avc1))
            .atom(stts(&durations))
            .atom(
                Atom::full(0, 0)
                    .u32(keyframes.len() as u32)
                    .u32s(&keyframes)
                    .finish(b"stss"),
            )
            .atom(stsc(&self.video_chunks))
            .atom(
                Atom::full(0, 0)
                    .u32(0)
                    .u32(samples.len() as u32)
                    .u32s(&samples.iter().map(|s| s.size).collect::<Vec<_>>())
                    .finish(b"stsz"),
            )
            .atom(co64(&self.video_chunks))
            .finish(b"stbl");

        trak(
            Atom::full(0, 0x3)
                .u32(0)
                .u32(0)
                .u32(VIDEO_TRACK)
                .u32(0)
                .u32(duration_ms)
                .zeros(8)
                .u16(0) // layer
                .u16(0) // alternate group
                .u16(0) // volume
                .u16(0)
                .matrix()
                .u32(u32::from(self.width) << 16)
                .u32(u32::from(self.height) << 16),
            mdhd(VIDEO_TIMESCALE, media_duration),
            b"vide",
            "VideoHandler",
            Atom::full(0, 1).u16(0).zeros(6).finish(b"vmhd"),
            stbl,
        )
    }

    fn audio_trak(&self, duration_ms: u32) -> Vec<u8> {
        /// 符号付き整数・詰めて並べる・ビッグエンディアン。
        const LPCM_FLAGS: u32 = 0x4 | 0x8 | 0x2;
        // SoundDescription バージョン2(Apple が LPCM に推奨する形)
        let lpcm = Atom::new()
            .zeros(6)
            .u16(1) // data reference index
            .u16(2) // version
            .u16(0)
            .u32(0)
            .u16(3)
            .u16(16)
            .u16(0xfffe)
            .u16(0)
            .u32(0x0001_0000)
            .u32(72) // sizeOfStructOnly
            .u64(f64::from(SAMPLE_RATE).to_bits())
            .u32(u32::from(CHANNELS))
            .u32(0x7f00_0000)
            .u32(24) // bits per channel
            .u32(LPCM_FLAGS)
            .u32(AUDIO_FRAME_BYTES as u32) // bytes per packet
            .u32(1) // frames per packet
            .finish(b"lpcm");

        let frames = self.audio_frames as u32;
        let durations = if frames > 0 {
            vec![(frames, 1)]
        } else {
            Vec::new()
        };
        let stbl = Atom::new()
            .atom(stsd(lpcm))
            .atom(stts(&durations))
            .atom(stsc(&self.audio_chunks))
            .atom(
                Atom::full(0, 0)
                    .u32(AUDIO_FRAME_BYTES as u32)
                    .u32(frames)
                    .finish(b"stsz"),
            )
            .atom(co64(&self.audio_chunks))
            .finish(b"stbl");

        trak(
            Atom::full(0, 0x3)
                .u32(0)
                .u32(0)
                .u32(AUDIO_TRACK)
                .u32(0)
                .u32(duration_ms)
                .zeros(8)
                .u16(0)
                .u16(0)
                .u16(0x0100) // volume 1.0
                .u16(0)
                .matrix()
                .u32(0)
                .u32(0),
            mdhd(SAMPLE_RATE, frames),
            b"soun",
            "SoundHandler",
            Atom::full(0, 0).u16(0).u16(0).finish(b"smhd"),
            stbl,
        )
    }
}

fn mvhd(duration_ms: u32) -> Vec<u8> {
    Atom::full(0, 0)
        .u32(0)
        .u32(0)
        .u32(MOVIE_TIMESCALE)
        .u32(duration_ms)
        .u32(0x0001_0000) // rate 1.0
        .u16(0x0100) // volume 1.0
        .zeros(10)
        .matrix()
        .zeros(24) // preview / poster / selection / current time
        .u32(AUDIO_TRACK + 1) // next track id
        .finish(b"mvhd")
}

fn mdhd(timescale: u32, duration: u32) -> Vec<u8> {
    Atom::full(0, 0)
        .u32(0)
        .u32(0)
        .u32(timescale)
        .u32(duration)
        .u16(0) // language
        .u16(0) // quality
        .finish(b"mdhd")
}

/// QuickTime の `hdlr`。名前は長さ付きの文字列。
fn hdlr(component: &[u8; 4], subtype: &[u8; 4], name: &str) -> Vec<u8> {
    Atom::full(0, 0)
        .bytes(component)
        .bytes(subtype)
        .zeros(12) // manufacturer, flags, flags mask
        .pascal(name, name.len() + 1)
        .finish(b"hdlr")
}

fn trak(
    tkhd: Atom,
    mdhd: Vec<u8>,
    kind: &[u8; 4],
    name: &str,
    media_header: Vec<u8>,
    stbl: Vec<u8>,
) -> Vec<u8> {
    let dinf = Atom::new()
        .atom(
            Atom::full(0, 0)
                .u32(1)
                .atom(Atom::full(0, 1).finish(b"alis"))
                .finish(b"dref"),
        )
        .finish(b"dinf");
    let minf = Atom::new()
        .atom(media_header)
        .atom(hdlr(b"dhlr", b"alis", "DataHandler"))
        .atom(dinf)
        .atom(stbl)
        .finish(b"minf");
    let mdia = Atom::new()
        .atom(mdhd)
        .atom(hdlr(b"mhlr", kind, name))
        .atom(minf)
        .finish(b"mdia");
    Atom::new()
        .atom(tkhd.finish(b"tkhd"))
        .atom(mdia)
        .finish(b"trak")
}

fn stsd(entry: Vec<u8>) -> Vec<u8> {
    Atom::full(0, 0).u32(1).atom(entry).finish(b"stsd")
}

/// サンプルの長さ(数, 長さ)の並び。
fn stts(durations: &[(u32, u32)]) -> Vec<u8> {
    let mut atom = Atom::full(0, 0).u32(durations.len() as u32);
    for &(count, delta) in durations {
        atom = atom.u32(count).u32(delta);
    }
    atom.finish(b"stts")
}

/// まとまりごとのサンプル数。同じ数が続く間は1つにまとめる。
fn stsc(chunks: &[Chunk]) -> Vec<u8> {
    let mut runs: Vec<(u32, u32)> = Vec::new();
    for (number, chunk) in (1..).zip(chunks) {
        if runs
            .last()
            .is_none_or(|&(_, samples)| samples != chunk.samples)
        {
            runs.push((number, chunk.samples));
        }
    }
    let mut atom = Atom::full(0, 0).u32(runs.len() as u32);
    for (first, samples) in runs {
        atom = atom.u32(first).u32(samples).u32(1);
    }
    atom.finish(b"stsc")
}

/// まとまりの位置(64bit)。
fn co64(chunks: &[Chunk]) -> Vec<u8> {
    let mut atom = Atom::full(0, 0).u32(chunks.len() as u32);
    for chunk in chunks {
        atom = atom.u64(chunk.offset);
    }
    atom.finish(b"co64")
}

/// 箱(atom)の中身を組み立てる。[`Atom::finish`] で大きさと種類を付ける。
#[derive(Default)]
struct Atom(Vec<u8>);

impl Atom {
    fn new() -> Self {
        Self::default()
    }

    /// version と flags で始まる箱。
    fn full(version: u8, flags: u32) -> Self {
        Self::new().u32((u32::from(version) << 24) | flags)
    }

    fn bytes(mut self, bytes: &[u8]) -> Self {
        self.0.extend(bytes);
        self
    }

    fn zeros(self, count: usize) -> Self {
        self.bytes(&vec![0; count])
    }

    fn u16(self, value: u16) -> Self {
        self.bytes(&value.to_be_bytes())
    }

    fn u32(self, value: u32) -> Self {
        self.bytes(&value.to_be_bytes())
    }

    fn u32s(mut self, values: &[u32]) -> Self {
        for value in values {
            self = self.u32(*value);
        }
        self
    }

    fn u64(self, value: u64) -> Self {
        self.bytes(&value.to_be_bytes())
    }

    fn atom(self, atom: Vec<u8>) -> Self {
        self.bytes(&atom)
    }

    /// 長さ付きの文字列を、長さのバイトを含めて `width` バイトに詰める。
    fn pascal(self, text: &str, width: usize) -> Self {
        let text = &text.as_bytes()[..text.len().min(width - 1)];
        self.bytes(&[text.len() as u8])
            .bytes(text)
            .zeros(width - 1 - text.len())
    }

    /// 回転なしの変換行列。
    fn matrix(self) -> Self {
        self.u32s(&[0x0001_0000, 0, 0, 0, 0x0001_0000, 0, 0, 0, 0x4000_0000])
    }

    fn finish(self, kind: &[u8; 4]) -> Vec<u8> {
        let mut atom = Vec::with_capacity(8 + self.0.len());
        atom.extend(((8 + self.0.len()) as u32).to_be_bytes());
        atom.extend(kind);
        atom.extend(self.0);
        atom
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::output::h264::decoder_config;
    use crate::output::h264::tests::{PPS, SPS};

    /// 箱の並び(種類, 位置, 大きさ)を最上位だけ読む。
    fn top_level(bytes: &[u8]) -> Vec<(String, usize, usize)> {
        let mut boxes = Vec::new();
        let mut offset = 0;
        while offset + 8 <= bytes.len() {
            let size = u32::from_be_bytes(bytes[offset..offset + 4].try_into().unwrap()) as usize;
            let kind = String::from_utf8_lossy(&bytes[offset + 4..offset + 8]).to_string();
            boxes.push((kind, offset, size));
            offset += size;
        }
        assert_eq!(offset, bytes.len(), "boxes must tile the file exactly");
        boxes
    }

    fn write_sample_file(path: &Path, finish: bool) -> Vec<u8> {
        let mut writer = MovWriter::create(path, (64, 36), decoder_config(&SPS, &PPS)).unwrap();
        for frame in 0..130u64 {
            writer
                .push_video(frame, frame % 60 == 0, vec![0, 0, 0, 1, 0x65])
                .unwrap();
            writer.push_audio(&[0; 1600 * AUDIO_FRAME_BYTES]);
        }
        if finish {
            writer.finish().unwrap();
        } else {
            drop(writer);
        }
        std::fs::read(path).unwrap()
    }

    fn temp_path(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("osechi-mov-test-{}-{name}.mov", std::process::id()))
    }

    #[test]
    fn unfinished_file_is_fragmented() {
        let path = temp_path("fragmented");
        let bytes = write_sample_file(&path, false);
        let kinds: Vec<String> = top_level(&bytes).into_iter().map(|(k, _, _)| k).collect();
        // 最後の断片はまだ書き出されていない(キーフレームが来ていない)
        assert_eq!(kinds, ["ftyp", "moov", "moof", "mdat", "moof", "mdat"]);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn finished_file_has_single_index_at_the_end() {
        let path = temp_path("finished");
        let bytes = write_sample_file(&path, true);
        let boxes = top_level(&bytes);
        let kinds: Vec<&str> = boxes.iter().map(|(k, _, _)| k.as_str()).collect();
        assert_eq!(
            kinds,
            [
                "ftyp", "free", "free", "mdat", "free", "mdat", "free", "mdat", "moov"
            ]
        );

        // 目次の映像は130フレーム・キーフレーム3つ、音声は130フレーム分
        let moov = &bytes[boxes.last().unwrap().1..];
        let find = |kind: &[u8]| {
            moov.windows(4)
                .position(|w| w == kind)
                .map(|at| u32::from_be_bytes(moov[at + 8..at + 12].try_into().unwrap()))
        };
        assert_eq!(find(b"stss"), Some(3));
        assert_eq!(find(b"co64"), Some(3));
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn pascal_strings_are_padded() {
        let atom = Atom::new().pascal("H.264", 8).finish(b"test");
        assert_eq!(&atom[8..], [5, b'H', b'.', b'2', b'6', b'4', 0, 0]);
    }

    #[test]
    fn stsc_merges_equal_runs() {
        let chunk = |samples| Chunk { offset: 0, samples };
        let atom = stsc(&[chunk(60), chunk(60), chunk(10)]);
        // entry count 2: (1, 60, 1), (3, 10, 1)
        assert_eq!(&atom[12..16], 2u32.to_be_bytes());
        assert_eq!(&atom[16..28], [0, 0, 0, 1, 0, 0, 0, 60, 0, 0, 0, 1]);
        assert_eq!(&atom[28..32], 3u32.to_be_bytes());
    }
}
