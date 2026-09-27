//! RTMPでの配信。edamame(の publisher)などのRTMPサーバーへ、番組(PGM)を H.264 + AAC で送る。
//!
//! edamame は 1080p を再エンコードせずにそのまま視聴者へ配る(`mode = "copy"`)ので、次の条件で送る。
//! - 映像: 1920x1080・30fps 固定・キーフレームは2秒ごと(セグメント長を割り切る間隔)
//! - 音声: AAC-LC・48kHz・ステレオ
//!
//! 送信は専用スレッドで行う。接続が切れたら [`RETRY_INTERVAL`] おいて繋ぎ直し、繋ぎ直すたびに
//! タイムスタンプとキーフレームの周期を0から数え直す(edamame 側も受け直すたびに0から数える)。

use std::fmt;
use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, TryRecvError};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use bytes::Bytes;
use rml_rtmp::handshake::{Handshake, HandshakeProcessResult, PeerType};
use rml_rtmp::sessions::{
    ClientSession, ClientSessionConfig, ClientSessionEvent, ClientSessionResult,
    PublishRequestType, StreamMetadata,
};
use rml_rtmp::time::RtmpTimestamp;

use super::encode::{AAC_FRAME_SIZE, AudioEncoder, to_i16};
use super::pipeline::{AudioPipeline, Clock, VideoPipeline};
use super::{FRAME_RATE, Taps, flv, h264};
use crate::error::{Context, Error, Result};
use crate::mixer::{CHANNELS, SAMPLE_RATE};
use crate::switcher::Slot;

/// RTMPの既定のポート。
const DEFAULT_PORT: u16 = 1935;
/// 送出する映像の大きさ。edamame の 1080p(copy)の設定と一致させる。
const SIZE: (usize, usize) = (1920, 1080);
/// 音声のビットレート(bps)。
const AUDIO_BITRATE: u32 = 128_000;
/// 接続が切れてから繋ぎ直すまでの間隔。
const RETRY_INTERVAL: Duration = Duration::from_secs(2);
/// 接続・送信・応答待ちのタイムアウト。
const NETWORK_TIMEOUT: Duration = Duration::from_secs(5);

/// 配信先。`rtmp://host[:port]/app/stream` を分解したもの。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    host: String,
    port: u16,
    app: String,
    /// ストリーム名(YouTubeなどではストリームキー)。ログには出さない。
    stream: String,
}

impl Target {
    /// RTMPの `connect` で送る tcUrl。
    fn tc_url(&self) -> String {
        format!("rtmp://{}:{}/{}", self.host, self.port, self.app)
    }
}

impl FromStr for Target {
    type Err = Error;

    fn from_str(url: &str) -> Result<Self> {
        let url = url.trim();
        if url.starts_with("rtmps://") {
            return Err(Error::new("rtmps:// is not supported; use rtmp://"));
        }
        let rest = url
            .strip_prefix("rtmp://")
            .context("URL must start with rtmp://")?;
        let (authority, path) = rest
            .split_once('/')
            .context("URL must be rtmp://host[:port]/app/stream")?;
        let (host, port) = match authority.rsplit_once(':') {
            Some((host, port)) => (
                host,
                port.parse()
                    .map_err(|_| Error::new(format!("invalid port: {port}")))?,
            ),
            None => (authority, DEFAULT_PORT),
        };
        let (app, stream) = path
            .split_once('/')
            .context("URL must include both app and stream (rtmp://host/app/stream)")?;
        if host.is_empty() || app.is_empty() || stream.is_empty() {
            return Err(Error::new(
                "URL must be rtmp://host[:port]/app/stream with non-empty parts",
            ));
        }
        Ok(Self {
            host: host.to_string(),
            port,
            app: app.to_string(),
            stream: stream.to_string(),
        })
    }
}

/// ストリーム名を伏せた表示(ログ用)。
impl fmt::Display for Target {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "rtmp://{}:{}/{}/***", self.host, self.port, self.app)
    }
}

/// 配信の状態。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum State {
    /// 配信していない
    #[default]
    Idle,
    /// 接続中
    Connecting,
    /// 送出中
    Live,
    /// 失敗したので繋ぎ直す前の待ち
    Retrying,
}

impl fmt::Display for State {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Idle => "Idle",
            Self::Connecting => "Connecting",
            Self::Live => "Live",
            Self::Retrying => "Retrying",
        })
    }
}

/// 配信の状態と統計。UIに出す。
#[derive(Debug, Clone, Default)]
pub struct Status {
    pub state: State,
    /// 今回の接続で送出を始めた時刻
    pub live_since: Option<Instant>,
    /// 直近1秒の送信ビットレート(bps)
    pub bitrate: f64,
    /// 直近1秒のエンコード速度(fps)
    pub fps: f64,
    /// 送信が間に合わず飛ばしたフレームの累計
    pub dropped_frames: u64,
    /// 音声が足りず無音で埋めた回数の累計
    pub audio_gaps: u64,
    /// 繋ぎ直した回数
    pub reconnects: u64,
    /// 直近の失敗の理由
    pub error: Option<String>,
}

type SharedStatus = Arc<Mutex<Status>>;

fn update(status: &SharedStatus, f: impl FnOnce(&mut Status)) {
    f(&mut status.lock().unwrap_or_else(PoisonError::into_inner));
}

/// 配信の設定。
#[derive(Debug, Clone)]
pub struct Settings {
    pub target: Target,
    /// 映像の目標ビットレート(bps)
    pub video_bitrate: u32,
}

/// 配信の操作口。[`Rtmp::start`] で送信スレッドを起動し、[`Rtmp::stop`] か drop で止める。
#[derive(Default)]
pub struct Rtmp {
    running: Option<Running>,
}

struct Running {
    stop: Arc<AtomicBool>,
    status: SharedStatus,
}

impl Drop for Running {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

impl Rtmp {
    /// PGM の配信を始める。既に配信中なら止めてから始め直す。
    pub fn start(&mut self, settings: Settings, taps: Taps) {
        self.stop();
        tracing::info!(
            "RTMP output starting: {} ({} kbps)",
            settings.target,
            settings.video_bitrate / 1000
        );
        let stop = Arc::new(AtomicBool::new(false));
        let status = SharedStatus::default();
        let (thread_stop, thread_status) = (Arc::clone(&stop), Arc::clone(&status));
        thread::Builder::new()
            .name("rtmp".to_string())
            .spawn(move || run(&settings, &taps, &thread_stop, &thread_status))
            .expect("failed to spawn RTMP thread");
        self.running = Some(Running { stop, status });
    }

    /// 配信を止める。送信スレッドは送信中の処理を終えてから抜ける(待たない)。
    pub fn stop(&mut self) {
        if self.running.take().is_some() {
            tracing::info!("RTMP output stopped");
        }
    }

    pub fn is_running(&self) -> bool {
        self.running.is_some()
    }

    /// いまの状態と統計。
    pub fn status(&self) -> Status {
        self.running
            .as_ref()
            .map_or_else(Status::default, |running| {
                running
                    .status
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .clone()
            })
    }
}

/// 止められるまで、接続 → 送出 → (失敗したら)待って繋ぎ直す、を繰り返す。
fn run(settings: &Settings, taps: &Taps, stop: &AtomicBool, status: &SharedStatus) {
    let mut attempts = 0u64;
    while !stop.load(Ordering::Relaxed) {
        update(status, |s| {
            s.state = State::Connecting;
            s.reconnects = attempts.saturating_sub(1);
        });
        attempts += 1;

        match publish(settings, taps, stop, status) {
            Ok(()) => break,
            Err(err) => {
                tracing::warn!("RTMP output to {} failed: {err:#}", settings.target);
                update(status, |s| {
                    s.state = State::Retrying;
                    s.live_since = None;
                    s.error = Some(format!("{err:#}"));
                });
            }
        }

        let retry_at = Instant::now() + RETRY_INTERVAL;
        while Instant::now() < retry_at && !stop.load(Ordering::Relaxed) {
            thread::sleep(Duration::from_millis(100));
        }
    }
}

/// 1回の接続で送出する。止められたら `Ok`、接続や送信に失敗したら `Err`。
fn publish(
    settings: &Settings,
    taps: &Taps,
    stop: &AtomicBool,
    status: &SharedStatus,
) -> Result<()> {
    let mut connection = Connection::open(&settings.target)?;
    connection.start_publishing(&settings.target)?;
    connection.send_metadata(settings.video_bitrate)?;
    tracing::info!("RTMP output is live: {}", settings.target);

    let mut encoder = Encoder::new(taps, settings.video_bitrate)?;
    update(status, |s| {
        s.state = State::Live;
        s.live_since = Some(Instant::now());
        s.error = None;
    });

    let mut clock = Clock::starting_at(Instant::now());
    let mut stats = Stats::new();
    while !stop.load(Ordering::Relaxed) {
        connection.receive()?;
        let index = clock.tick();
        for tag in encoder.encode(index)? {
            connection.send(tag)?;
        }
        stats.frames += 1;
        stats.report(
            connection.sent_bytes,
            clock.dropped,
            encoder.audio.gaps,
            status,
        );
    }

    connection.stop_publishing();
    Ok(())
}

/// 送信の統計(1秒ごとに [`Status`] へ反映する)。
struct Stats {
    since: Instant,
    bytes_at: u64,
    frames: u64,
}

impl Stats {
    fn new() -> Self {
        Self {
            since: Instant::now(),
            bytes_at: 0,
            frames: 0,
        }
    }

    fn report(&mut self, sent_bytes: u64, dropped: u64, audio_gaps: u64, status: &SharedStatus) {
        let elapsed = self.since.elapsed().as_secs_f64();
        if elapsed < 1.0 {
            return;
        }
        let bitrate = (sent_bytes - self.bytes_at) as f64 * 8.0 / elapsed;
        let fps = self.frames as f64 / elapsed;
        update(status, |s| {
            s.bitrate = bitrate;
            s.fps = fps;
            s.dropped_frames = dropped;
            s.audio_gaps = audio_gaps;
        });
        tracing::debug!("RTMP output: {:.0} kbps, {fps:.1} fps", bitrate / 1000.0);
        *self = Self {
            bytes_at: sent_bytes,
            ..Self::new()
        };
    }
}

/// 送るメッセージ。
enum Tag {
    Video {
        data: Vec<u8>,
        timestamp: u32,
        keyframe: bool,
    },
    Audio {
        data: Vec<u8>,
        timestamp: u32,
    },
}

/// PGM の映像・音声を取り出してエンコードし、送るメッセージにする。
struct Encoder {
    video: VideoPipeline,
    audio: AudioPipeline,
    aac: AudioEncoder,
    sent_video_header: bool,
    sent_audio_header: bool,
    /// AACエンコーダへ渡す前の 16bit PCM
    pcm: Vec<i16>,
    /// 出力したAACフレームの数
    aac_frames: u64,
}

impl Encoder {
    fn new(taps: &Taps, video_bitrate: u32) -> Result<Self> {
        Ok(Self {
            video: VideoPipeline::new(taps.clone(), Slot::Program, SIZE, video_bitrate)?,
            audio: AudioPipeline::new(taps),
            aac: AudioEncoder::new(AUDIO_BITRATE)?,
            sent_video_header: false,
            sent_audio_header: false,
            pcm: Vec::new(),
            aac_frames: 0,
        })
    }

    /// `index` 番目のフレームと、その時刻までの音声をエンコードする。
    fn encode(&mut self, index: u64) -> Result<Vec<Tag>> {
        let mut tags = self.encode_video(index)?;
        tags.extend(self.encode_audio(index + 1)?);
        Ok(tags)
    }

    fn encode_video(&mut self, index: u64) -> Result<Vec<Tag>> {
        let encoded = self.video.encode(index)?;
        let timestamp = timestamp_ms(index, u64::from(FRAME_RATE));

        let mut tags = Vec::new();
        if !self.sent_video_header {
            // 最初のキーフレームに付いてくる SPS・PPS からシーケンスヘッダを作る
            let Some((sps, pps)) = h264::parameter_sets(&encoded.nals) else {
                return Ok(tags);
            };
            tags.push(Tag::Video {
                data: flv::avc_sequence_header(&h264::decoder_config(sps, pps)),
                timestamp,
                keyframe: true,
            });
            self.sent_video_header = true;
        }
        if !encoded.nals.is_empty() {
            tags.push(Tag::Video {
                data: flv::avc_frame(&h264::length_prefixed(&encoded.nals), encoded.keyframe),
                timestamp,
                keyframe: encoded.keyframe,
            });
        }
        Ok(tags)
    }

    /// 映像の `frames` 枚目の時刻までの音声を AAC にする。
    fn encode_audio(&mut self, frames: u64) -> Result<Vec<Tag>> {
        self.pcm
            .extend(self.audio.take(frames).into_iter().map(to_i16));

        let mut tags = Vec::new();
        let frame_len = AAC_FRAME_SIZE * CHANNELS as usize;
        while self.pcm.len() >= frame_len {
            let chunk: Vec<i16> = self.pcm.drain(..frame_len).collect();
            let Some(aac) = self.aac.encode(&chunk)? else {
                continue;
            };
            let timestamp = timestamp_ms(
                self.aac_frames * AAC_FRAME_SIZE as u64,
                u64::from(SAMPLE_RATE),
            );
            if !self.sent_audio_header {
                tags.push(Tag::Audio {
                    data: flv::aac_sequence_header(),
                    timestamp,
                });
                self.sent_audio_header = true;
            }
            tags.push(Tag::Audio {
                data: flv::aac_frame(&aac),
                timestamp,
            });
            self.aac_frames += 1;
        }
        Ok(tags)
    }
}

/// `rate` 分の1秒単位の位置 `position` をミリ秒にする。
fn timestamp_ms(position: u64, rate: u64) -> u32 {
    (position * 1000 / rate) as u32
}

/// RTMPサーバーとの接続。
struct Connection {
    stream: TcpStream,
    session: ClientSession,
    /// 受信スレッドが読んだバイト列。受信スレッドが終わると切断される。
    incoming: Receiver<Vec<u8>>,
    sent_bytes: u64,
}

impl Connection {
    /// TCPで接続し、ハンドシェイクを済ませる。
    fn open(target: &Target) -> Result<Self> {
        let mut stream = connect(target)?;
        let leftover = handshake(&mut stream)?;

        let reader = stream.try_clone().context("failed to clone socket")?;
        let (sender, incoming) = mpsc::channel();
        thread::Builder::new()
            .name("rtmp-recv".to_string())
            .spawn(move || receive_loop(reader, &sender))
            .context("failed to spawn RTMP receive thread")?;

        let mut config = ClientSessionConfig::new();
        config.tc_url = Some(target.tc_url());
        let (session, results) = ClientSession::new(config).map_err(rtmp_error)?;
        let mut connection = Self {
            stream,
            session,
            incoming,
            sent_bytes: 0,
        };
        connection.apply(results)?;
        if !leftover.is_empty() {
            let results = connection
                .session
                .handle_input(&leftover)
                .map_err(rtmp_error)?;
            connection.apply(results)?;
        }
        Ok(connection)
    }

    /// `connect` と `publish` を送り、サーバーが受け入れるまで待つ。
    fn start_publishing(&mut self, target: &Target) -> Result<()> {
        let request = self
            .session
            .request_connection(target.app.clone())
            .map_err(rtmp_error)?;
        self.apply(vec![request])?;
        self.wait_for("connect", |event| {
            matches!(event, ClientSessionEvent::ConnectionRequestAccepted)
        })?;

        let request = self
            .session
            .request_publishing(target.stream.clone(), PublishRequestType::Live)
            .map_err(rtmp_error)?;
        self.apply(vec![request])?;
        self.wait_for("publish", |event| {
            matches!(event, ClientSessionEvent::PublishRequestAccepted)
        })
    }

    /// 映像・音声の形式を onMetaData で伝える。
    fn send_metadata(&mut self, video_bitrate: u32) -> Result<()> {
        let mut metadata = StreamMetadata::new();
        metadata.video_width = Some(SIZE.0 as u32);
        metadata.video_height = Some(SIZE.1 as u32);
        metadata.video_codec_id = Some(flv::VIDEO_CODEC_ID);
        metadata.video_frame_rate = Some(FRAME_RATE as f32);
        metadata.video_bitrate_kbps = Some(video_bitrate / 1000);
        metadata.audio_codec_id = Some(flv::AUDIO_CODEC_ID);
        metadata.audio_bitrate_kbps = Some(AUDIO_BITRATE / 1000);
        metadata.audio_sample_rate = Some(SAMPLE_RATE);
        metadata.audio_channels = Some(u32::from(CHANNELS));
        metadata.audio_is_stereo = Some(CHANNELS == 2);
        metadata.encoder = Some(concat!("Osechi v", env!("CARGO_PKG_VERSION")).to_string());
        let result = self
            .session
            .publish_metadata(&metadata)
            .map_err(rtmp_error)?;
        self.apply(vec![result])?;
        Ok(())
    }

    /// 映像・音声のメッセージを送る。
    fn send(&mut self, tag: Tag) -> Result<()> {
        let result = match tag {
            Tag::Video {
                data,
                timestamp,
                keyframe,
            } => self.session.publish_video_data(
                Bytes::from(data),
                RtmpTimestamp::new(timestamp),
                !keyframe,
            ),
            Tag::Audio { data, timestamp } => self.session.publish_audio_data(
                Bytes::from(data),
                RtmpTimestamp::new(timestamp),
                false,
            ),
        }
        .map_err(rtmp_error)?;
        self.apply(vec![result])?;
        Ok(())
    }

    /// 届いているメッセージを処理する(応答や確認応答を返す)。ブロックしない。
    fn receive(&mut self) -> Result<()> {
        loop {
            match self.incoming.try_recv() {
                Ok(bytes) => {
                    let results = self.session.handle_input(&bytes).map_err(rtmp_error)?;
                    self.check(results)?;
                }
                Err(TryRecvError::Empty) => return Ok(()),
                Err(TryRecvError::Disconnected) => {
                    return Err(Error::new("connection closed by server"));
                }
            }
        }
    }

    /// サーバーが `accepted` に当たる応答を返すまで待つ。
    fn wait_for(
        &mut self,
        what: &str,
        accepted: impl Fn(&ClientSessionEvent) -> bool,
    ) -> Result<()> {
        let deadline = Instant::now() + NETWORK_TIMEOUT;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            let bytes = match self.incoming.recv_timeout(remaining) {
                Ok(bytes) => bytes,
                Err(RecvTimeoutError::Timeout) => {
                    return Err(Error::new(format!(
                        "server did not respond to {what} request"
                    )));
                }
                Err(RecvTimeoutError::Disconnected) => {
                    return Err(Error::new(format!(
                        "connection closed by server during {what} request"
                    )));
                }
            };
            let results = self.session.handle_input(&bytes).map_err(rtmp_error)?;
            if self.check(results)?.iter().any(&accepted) {
                return Ok(());
            }
        }
    }

    /// 結果を処理し、拒否されていれば `Err` にする。
    fn check(&mut self, results: Vec<ClientSessionResult>) -> Result<Vec<ClientSessionEvent>> {
        let events = self.apply(results)?;
        for event in &events {
            match event {
                ClientSessionEvent::ConnectionRequestRejected { description } => {
                    return Err(Error::new(format!(
                        "server rejected connection: {description}"
                    )));
                }
                ClientSessionEvent::UnhandleableOnStatusCode { code }
                    if code.contains("Failed")
                        || code.contains("BadName")
                        || code.contains("Rejected") =>
                {
                    return Err(Error::new(format!("server rejected stream: {code}")));
                }
                _ => {}
            }
        }
        Ok(events)
    }

    /// 送るべきパケットを送り、発生したイベントを返す。
    fn apply(&mut self, results: Vec<ClientSessionResult>) -> Result<Vec<ClientSessionEvent>> {
        let mut events = Vec::new();
        for result in results {
            match result {
                ClientSessionResult::OutboundResponse(packet) => {
                    self.stream
                        .write_all(&packet.bytes)
                        .context("failed to send to server")?;
                    self.sent_bytes += packet.bytes.len() as u64;
                }
                ClientSessionResult::RaisedEvent(event) => events.push(event),
                ClientSessionResult::UnhandleableMessageReceived(_) => {}
            }
        }
        Ok(events)
    }

    /// 配信の終了を伝えて切断する(失敗しても構わない)。
    fn stop_publishing(mut self) {
        if let Ok(results) = self.session.stop_publishing() {
            let _ = self.apply(results);
        }
        let _ = self.stream.shutdown(std::net::Shutdown::Both);
    }
}

impl Drop for Connection {
    fn drop(&mut self) {
        // 受信スレッドを終わらせる
        let _ = self.stream.shutdown(std::net::Shutdown::Both);
    }
}

fn rtmp_error(err: impl fmt::Display) -> Error {
    Error::new(format!("RTMP protocol error: {err}"))
}

/// 配信先へTCPで接続する。
fn connect(target: &Target) -> Result<TcpStream> {
    let addrs = (target.host.as_str(), target.port)
        .to_socket_addrs()
        .context(format!("failed to resolve {}", target.host))?;
    let mut last_error = Error::new(format!("no address found for {}", target.host));
    for addr in addrs {
        match TcpStream::connect_timeout(&addr, NETWORK_TIMEOUT) {
            Ok(stream) => {
                stream
                    .set_nodelay(true)
                    .context("failed to set TCP_NODELAY")?;
                stream
                    .set_write_timeout(Some(NETWORK_TIMEOUT))
                    .context("failed to set write timeout")?;
                return Ok(stream);
            }
            Err(err) => last_error = Error::new(format!("failed to connect to {addr}: {err}")),
        }
    }
    Err(last_error)
}

/// RTMPのハンドシェイク。ハンドシェイクの後に続けて届いたバイト列を返す。
fn handshake(stream: &mut TcpStream) -> Result<Vec<u8>> {
    stream
        .set_read_timeout(Some(NETWORK_TIMEOUT))
        .context("failed to set read timeout")?;
    let mut handshake = Handshake::new(PeerType::Client);
    let hello = handshake
        .generate_outbound_p0_and_p1()
        .map_err(rtmp_error)?;
    stream
        .write_all(&hello)
        .context("failed to send handshake")?;

    let mut buf = [0u8; 4096];
    loop {
        let len = stream.read(&mut buf).context("handshake failed")?;
        if len == 0 {
            return Err(Error::new("connection closed during handshake"));
        }
        match handshake.process_bytes(&buf[..len]).map_err(rtmp_error)? {
            HandshakeProcessResult::InProgress { response_bytes } => {
                stream
                    .write_all(&response_bytes)
                    .context("failed to send handshake")?;
            }
            HandshakeProcessResult::Completed {
                response_bytes,
                remaining_bytes,
            } => {
                stream
                    .write_all(&response_bytes)
                    .context("failed to send handshake")?;
                stream
                    .set_read_timeout(None)
                    .context("failed to clear read timeout")?;
                return Ok(remaining_bytes);
            }
        }
    }
}

/// 受信スレッド。読めたバイト列を `sender` へ渡し、切断されたら終わる。
fn receive_loop(mut stream: TcpStream, sender: &mpsc::Sender<Vec<u8>>) {
    let mut buf = vec![0u8; 16 * 1024];
    loop {
        match stream.read(&mut buf) {
            Ok(0) | Err(_) => return,
            Ok(len) => {
                if sender.send(buf[..len].to_vec()).is_err() {
                    return;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_edamame_url() {
        let target: Target = "rtmp://192.168.1.10:1935/live/1A".parse().unwrap();
        assert_eq!(target.host, "192.168.1.10");
        assert_eq!(target.port, 1935);
        assert_eq!(target.app, "live");
        assert_eq!(target.stream, "1A");
        assert_eq!(target.tc_url(), "rtmp://192.168.1.10:1935/live");
    }

    #[test]
    fn default_port_and_nested_stream_name() {
        let target: Target = "rtmp://a.rtmp.youtube.com/live2/abcd-efgh".parse().unwrap();
        assert_eq!(target.port, DEFAULT_PORT);
        assert_eq!(target.app, "live2");
        assert_eq!(target.stream, "abcd-efgh");
    }

    #[test]
    fn display_hides_stream_name() {
        let target: Target = "rtmp://example.com/live/secret-key".parse().unwrap();
        assert_eq!(target.to_string(), "rtmp://example.com:1935/live/***");
    }

    #[test]
    fn rejects_invalid_urls() {
        for url in [
            "",
            "http://example.com/live/1A",
            "rtmps://example.com/live/1A",
            "rtmp://example.com",
            "rtmp://example.com/live",
            "rtmp://example.com/live/",
            "rtmp://example.com:port/live/1A",
        ] {
            assert!(url.parse::<Target>().is_err(), "{url} should be rejected");
        }
    }

    #[test]
    fn timestamps_are_in_milliseconds() {
        assert_eq!(timestamp_ms(60, 30), 2000);
        assert_eq!(timestamp_ms(1, 30), 33);
        assert_eq!(timestamp_ms(1024, 48_000), 21);
    }

    #[test]
    fn sends_sequence_header_then_keyframes_every_two_seconds() {
        let mut encoder = Encoder::new(&Taps::default(), 1_000_000).unwrap();
        let mut keyframes = Vec::new();
        for index in 0..130 {
            for tag in encoder.encode(index).unwrap() {
                if let Tag::Video {
                    keyframe: true,
                    timestamp,
                    data,
                } = tag
                    && data[1] == 1
                {
                    keyframes.push(timestamp);
                }
            }
        }
        assert_eq!(keyframes, [0, 2000, 4000]);
    }

    #[test]
    fn audio_keeps_pace_with_video() {
        let mut encoder = Encoder::new(&Taps::default(), 1_000_000).unwrap();
        let mut last_audio = 0;
        for index in 0..90 {
            for tag in encoder.encode(index).unwrap() {
                if let Tag::Audio { timestamp, .. } = tag {
                    last_audio = timestamp;
                }
            }
        }
        // 3秒分の映像に対し、音声も(溜めた分とエンコーダの遅延を除いて)ほぼ3秒分出ている
        assert!((2800..=3000).contains(&last_audio), "{last_audio}");
    }
}
