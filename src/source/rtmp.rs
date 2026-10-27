//! RTMP 入力。OBS などから送られてくる FLV を RTMP サーバーとして受け取り、
//! H.264 を RGB に、AAC を PCM にデコードしてソースとして公開する。

use std::collections::{HashMap, VecDeque};
use std::fmt;
use std::io::{ErrorKind, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use fdk_aac::dec::{Decoder as AacDecoder, Transport};
use openh264::decoder::{Decoder as H264Decoder, DecoderConfig, Flush};
use openh264::formats::YUVSource;
use openh264::{OpenH264API, nal_units};
use rml_rtmp::handshake::{Handshake, HandshakeProcessResult, PeerType};
use rml_rtmp::sessions::{
    ServerSession, ServerSessionConfig, ServerSessionEvent, ServerSessionResult,
};

use super::audio::{Chunk, FEED_CAPACITY as AUDIO_CAPACITY};
use super::video::Frame;
use super::{Feed, Origin, Source, SourceId, audio, video};
use crate::error::{Context, Error, Result};

const VIDEO_CAPACITY: usize = 2;
const INACTIVE_RETENTION: Duration = Duration::from_secs(30);

/// 同じ配信キーに接続している1本の RTMP ストリーム。
#[derive(Clone)]
pub struct Stream {
    app: String,
    key: String,
    inner: Arc<Inner>,
}

struct Inner {
    video: Mutex<Option<Frame>>,
    audio: Mutex<VecDeque<Chunk>>,
    error: Mutex<Option<String>>,
    active: AtomicBool,
    last_active: Mutex<Instant>,
}

impl fmt::Debug for Stream {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RtmpStream")
            .field("id", &self.id())
            .field("active", &self.is_active())
            .finish()
    }
}

impl Stream {
    fn new(app: &str, key: &str) -> Self {
        Self {
            app: app.to_string(),
            key: key.to_string(),
            inner: Arc::new(Inner {
                video: Mutex::new(None),
                audio: Mutex::new(VecDeque::with_capacity(AUDIO_CAPACITY)),
                error: Mutex::new(None),
                active: AtomicBool::new(false),
                last_active: Mutex::new(Instant::now()),
            }),
        }
    }

    fn id(&self) -> String {
        format!("{}/{}", self.app, self.key)
    }

    fn is_active(&self) -> bool {
        self.inner.active.load(Ordering::Relaxed)
    }

    fn reset(&self) {
        *lock(&self.inner.video) = None;
        lock(&self.inner.audio).clear();
        *lock(&self.inner.error) = None;
        self.inner.active.store(true, Ordering::Relaxed);
        *lock(&self.inner.last_active) = Instant::now();
    }

    fn stop(&self) {
        self.inner.active.store(false, Ordering::Relaxed);
        *lock(&self.inner.last_active) = Instant::now();
        *lock(&self.inner.error) = Some("RTMP publisher disconnected".to_string());
    }

    fn clear_error(&self) {
        *lock(&self.inner.error) = None;
    }

    fn fail(&self, message: impl Into<String>) {
        *lock(&self.inner.error) = Some(message.into());
    }

    fn push_video(&self, frame: Frame) {
        let first = {
            let mut video = lock(&self.inner.video);
            let first = video.is_none();
            *video = Some(frame);
            first
        };
        if first {
            tracing::debug!("RTMP stream {}: first video frame decoded", self.id());
        }
        self.clear_error();
    }

    fn push_audio(&self, chunk: Chunk) {
        let mut audio = lock(&self.inner.audio);
        if audio.len() >= AUDIO_CAPACITY {
            audio.pop_front();
        }
        audio.push_back(chunk);
        drop(audio);
        self.clear_error();
    }

    fn video_source(&self) -> Source<video::Kind> {
        Source {
            id: SourceId::new("rtmp", self.id()),
            name: format!("RTMP {}", self.id()),
            kind: video::Kind::Rtmp(self.clone()),
            origin: Origin::Rtmp,
        }
    }

    fn audio_source(&self) -> Source<audio::Kind> {
        Source {
            id: SourceId::new("rtmp-audio", self.id()),
            name: format!("RTMP {} (Audio)", self.id()),
            kind: audio::Kind::Rtmp(self.clone()),
            origin: Origin::Rtmp,
        }
    }

    pub fn open_video(&self) -> Result<Feed<Frame>> {
        let inner = Arc::clone(&self.inner);
        Ok(Feed::spawn(VIDEO_CAPACITY, move |producer| {
            while producer.is_open() {
                if let Some(frame) = lock(&inner.video).take() {
                    if !producer.send(Ok(frame)) {
                        break;
                    }
                    continue;
                }
                if let Some(err) = lock(&inner.error).take() {
                    producer.send(Err(Error::new(err)));
                }
                thread::sleep(Duration::from_millis(5));
            }
        }))
    }

    pub fn open_audio(&self) -> Result<Feed<Chunk>> {
        let inner = Arc::clone(&self.inner);
        Ok(Feed::spawn(AUDIO_CAPACITY, move |producer| {
            while producer.is_open() {
                let chunks: Vec<Chunk> = lock(&inner.audio).drain(..).collect();
                if !chunks.is_empty() {
                    for chunk in chunks {
                        if !producer.send(Ok(chunk)) {
                            return;
                        }
                    }
                    continue;
                }
                if let Some(err) = lock(&inner.error).take() {
                    producer.send(Err(Error::new(err)));
                }
                thread::sleep(Duration::from_millis(5));
            }
        }))
    }
}

/// RTMP 入力サーバー。
pub struct Ingest {
    addr: SocketAddr,
    shared: Arc<Shared>,
    shutdown: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

struct Shared {
    streams: Mutex<HashMap<String, Stream>>,
    error: Mutex<Option<String>>,
    shutdown: Arc<AtomicBool>,
}

impl Ingest {
    pub fn start(addr: SocketAddr) -> Result<Self> {
        let listener = TcpListener::bind(addr).context(format!("failed to listen on {addr}"))?;
        listener
            .set_nonblocking(true)
            .context("failed to configure RTMP listener")?;
        let addr = listener
            .local_addr()
            .context("failed to get RTMP address")?;

        let shutdown = Arc::new(AtomicBool::new(false));
        let shared = Arc::new(Shared {
            streams: Mutex::new(HashMap::new()),
            error: Mutex::new(None),
            shutdown: Arc::clone(&shutdown),
        });
        let thread = {
            let shared = Arc::clone(&shared);
            let shutdown = Arc::clone(&shutdown);
            thread::Builder::new()
                .name("rtmp-ingest".to_string())
                .spawn(move || accept_loop(listener, shared, shutdown))
                .context("failed to spawn RTMP ingest thread")?
        };

        Ok(Self {
            addr,
            shared,
            shutdown,
            thread: Some(thread),
        })
    }

    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    pub fn error(&self) -> Option<String> {
        lock(&self.shared.error).clone()
    }

    pub fn video_sources(&self) -> Vec<Source<video::Kind>> {
        self.streams()
            .into_iter()
            .map(|stream| stream.video_source())
            .collect()
    }

    pub fn audio_sources(&self) -> Vec<Source<audio::Kind>> {
        self.streams()
            .into_iter()
            .map(|stream| stream.audio_source())
            .collect()
    }

    fn streams(&self) -> Vec<Stream> {
        self.shared.prune();
        lock(&self.shared.streams)
            .values()
            .filter(|stream| stream.is_active())
            .cloned()
            .collect()
    }
}

impl Drop for Ingest {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Shared {
    fn begin_publish(&self, app: &str, key: &str) -> Option<Stream> {
        let id = format!("{app}/{key}");
        let mut streams = lock(&self.streams);
        if streams.get(&id).is_some_and(Stream::is_active) {
            return None;
        }
        let stream = streams
            .entry(id)
            .or_insert_with(|| Stream::new(app, key))
            .clone();
        stream.reset();
        Some(stream)
    }

    fn end_publish(&self, stream: &Stream) {
        if let Some(current) = lock(&self.streams).get(&stream.id())
            && Arc::ptr_eq(&current.inner, &stream.inner)
        {
            current.stop();
        }
    }

    fn prune(&self) {
        let mut streams = lock(&self.streams);
        streams.retain(|_, stream| {
            stream.is_active() || lock(&stream.inner.last_active).elapsed() < INACTIVE_RETENTION
        });
    }
}

fn accept_loop(listener: TcpListener, shared: Arc<Shared>, shutdown: Arc<AtomicBool>) {
    while !shutdown.load(Ordering::Relaxed) {
        match listener.accept() {
            Ok((socket, _)) => {
                *lock(&shared.error) = None;
                let shared = Arc::clone(&shared);
                let _ = thread::Builder::new()
                    .name("rtmp-client".to_string())
                    .spawn(move || {
                        if let Err(err) = serve(socket, &shared) {
                            tracing::warn!("RTMP client disconnected with error: {err:#}");
                        }
                    });
            }
            Err(err) if err.kind() == ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(25));
            }
            Err(err) => {
                *lock(&shared.error) = Some(format!("RTMP accept failed: {err}"));
                thread::sleep(Duration::from_secs(1));
            }
        }
    }
}

#[derive(Default)]
struct ConnState {
    stream: Option<Stream>,
    video: Option<H264Decoder>,
    video_length_size: usize,
    audio: Option<AacDecoder>,
}

impl ConnState {
    fn new() -> Self {
        Self {
            video_length_size: 4,
            ..Default::default()
        }
    }

    fn reset_decoders(&mut self) {
        self.video = None;
        self.video_length_size = 4;
        self.audio = None;
    }

    fn push_video(&mut self, data: &[u8]) {
        let Some(stream) = self.stream.clone() else {
            return;
        };
        if data.len() < 5 || data[0] & 0x0f != 7 {
            return;
        }

        match data[1] {
            0 => match parse_avc_config(&data[5..]) {
                Ok(config) => {
                    self.video_length_size = config.length_size;
                    if let Some(decoder) = self.video_decoder() {
                        for nal in nal_units(&config.parameter_sets) {
                            let _ = decoder.decode(nal);
                        }
                    }
                    stream.clear_error();
                }
                Err(err) => stream.fail(format!("RTMP: {err:#}")),
            },
            1 => {
                let annex_b = match annex_b_from_avcc(&data[5..], self.video_length_size) {
                    Ok(annex_b) => annex_b,
                    Err(err) => {
                        stream.fail(format!("RTMP: {err:#}"));
                        return;
                    }
                };
                if annex_b.is_empty() {
                    return;
                }
                let Some(decoder) = self.video_decoder() else {
                    return;
                };
                match decoder.decode(&annex_b) {
                    Ok(Some(yuv)) => match yuv_to_rgb(&yuv) {
                        Ok(frame) => stream.push_video(frame),
                        Err(err) => stream.fail(format!("RTMP: {err:#}")),
                    },
                    Ok(None) => {}
                    Err(err) => {
                        tracing::debug!("RTMP video decode failed: {err}");
                        stream.fail(format!("RTMP video decode failed: {err}"));
                    }
                }
            }
            _ => {}
        }
    }

    fn video_decoder(&mut self) -> Option<&mut H264Decoder> {
        if self.video.is_none() {
            let config = DecoderConfig::new().flush_after_decode(Flush::NoFlush);
            match H264Decoder::with_api_config(OpenH264API::from_source(), config) {
                Ok(decoder) => self.video = Some(decoder),
                Err(err) => {
                    if let Some(stream) = &self.stream {
                        stream.fail(format!("RTMP H.264 decoder failed: {err}"));
                    }
                    return None;
                }
            }
        }
        self.video.as_mut()
    }

    fn push_audio(&mut self, data: &[u8]) {
        let Some(stream) = self.stream.clone() else {
            return;
        };
        if data.len() < 2 || data[0] >> 4 != 10 {
            return;
        }

        match data[1] {
            0 => {
                let mut decoder = AacDecoder::new(Transport::Raw);
                match decoder.config_raw(&data[2..]) {
                    Ok(()) => {
                        self.audio = Some(decoder);
                        stream.clear_error();
                    }
                    Err(err) => stream.fail(format!("RTMP AAC config failed: {err}")),
                }
            }
            1 => {
                let Some(decoder) = &mut self.audio else {
                    return;
                };
                if decoder.fill(&data[2..]).is_err() {
                    return;
                }
                let info = decoder.stream_info();
                let channels = info.numChannels.max(0) as usize;
                if channels == 0 {
                    return;
                }
                let mut pcm = vec![0_i16; 2048 * channels];
                if decoder.decode_frame(&mut pcm).is_err() {
                    return;
                }

                let info = decoder.stream_info();
                let channels = info.numChannels.max(0) as usize;
                let frames = info.frameSize.max(0) as usize;
                let sample_rate = info.sampleRate.max(0) as u32;
                if channels == 0 || frames == 0 || sample_rate == 0 {
                    return;
                }
                let total = channels * frames;
                if pcm.len() < total {
                    return;
                }
                let planar = channels > 1 && info.sampleRate == info.aacSampleRate;
                let mut samples = Vec::with_capacity(total);
                for frame in 0..frames {
                    for channel in 0..channels {
                        let index = if planar {
                            channel * frames + frame
                        } else {
                            frame * channels + channel
                        };
                        samples.push(pcm[index] as f32 / 32_768.0);
                    }
                }
                stream.push_audio(Chunk {
                    samples,
                    sample_rate,
                    channels: channels as u16,
                });
            }
            _ => {}
        }
    }
}

fn serve(mut socket: TcpStream, shared: &Shared) -> Result<()> {
    socket
        .set_nodelay(true)
        .context("failed to configure RTMP socket")?;
    socket
        .set_read_timeout(Some(Duration::from_millis(500)))
        .context("failed to configure RTMP read timeout")?;
    socket
        .set_write_timeout(Some(Duration::from_secs(5)))
        .context("failed to configure RTMP write timeout")?;

    let mut handshake = Handshake::new(PeerType::Server);
    let mut session: Option<ServerSession> = None;
    let mut state = ConnState::new();
    let mut buffer = [0_u8; 8192];

    loop {
        if shared.shutdown.load(Ordering::Relaxed) {
            break;
        }
        let count = match socket.read(&mut buffer) {
            Ok(0) => break,
            Ok(count) => count,
            Err(err) => match err.kind() {
                ErrorKind::WouldBlock | ErrorKind::TimedOut => continue,
                ErrorKind::ConnectionReset | ErrorKind::ConnectionAborted => break,
                _ => return Err(err).context("RTMP read failed"),
            },
        };

        if let Some(session) = session.as_mut() {
            handle_bytes(session, &buffer[..count], shared, &mut state, &mut socket)?;
            continue;
        }

        match handshake
            .process_bytes(&buffer[..count])
            .context("RTMP handshake failed")?
        {
            HandshakeProcessResult::InProgress { response_bytes } => {
                socket
                    .write_all(&response_bytes)
                    .context("RTMP handshake write failed")?;
            }
            HandshakeProcessResult::Completed {
                response_bytes,
                remaining_bytes,
            } => {
                socket
                    .write_all(&response_bytes)
                    .context("RTMP handshake write failed")?;
                let (mut new_session, initial) = ServerSession::new(ServerSessionConfig::new())
                    .context("failed to create RTMP session")?;
                process_results(&mut new_session, initial, shared, &mut state, &mut socket)?;
                if !remaining_bytes.is_empty() {
                    handle_bytes(
                        &mut new_session,
                        &remaining_bytes,
                        shared,
                        &mut state,
                        &mut socket,
                    )?;
                }
                session = Some(new_session);
            }
        }
    }

    if let Some(stream) = state.stream.take() {
        shared.end_publish(&stream);
    }
    Ok(())
}

fn handle_bytes(
    session: &mut ServerSession,
    bytes: &[u8],
    shared: &Shared,
    state: &mut ConnState,
    socket: &mut TcpStream,
) -> Result<()> {
    let results = session.handle_input(bytes).context("RTMP session error")?;
    process_results(session, results, shared, state, socket)
}

fn process_results(
    session: &mut ServerSession,
    results: Vec<ServerSessionResult>,
    shared: &Shared,
    state: &mut ConnState,
    socket: &mut TcpStream,
) -> Result<()> {
    let mut queue: VecDeque<ServerSessionResult> = results.into();
    while let Some(result) = queue.pop_front() {
        match result {
            ServerSessionResult::OutboundResponse(packet) => {
                socket
                    .write_all(&packet.bytes)
                    .context("RTMP write failed")?;
            }
            ServerSessionResult::RaisedEvent(event) => {
                let more = handle_event(session, event, shared, state)?;
                queue.extend(more);
            }
            ServerSessionResult::UnhandleableMessageReceived(_) => {}
        }
    }
    Ok(())
}

fn handle_event(
    session: &mut ServerSession,
    event: ServerSessionEvent,
    shared: &Shared,
    state: &mut ConnState,
) -> Result<Vec<ServerSessionResult>> {
    match event {
        ServerSessionEvent::ConnectionRequested { request_id, .. } => session
            .accept_request(request_id)
            .context("failed to accept RTMP connection"),
        ServerSessionEvent::PublishStreamRequested {
            request_id,
            app_name,
            stream_key,
            ..
        } => {
            if state.stream.is_some() {
                return session
                    .reject_request(
                        request_id,
                        "NetStream.Publish.BadName",
                        "This connection is already publishing",
                    )
                    .context("failed to reject RTMP publish");
            }
            let Some(stream) = shared.begin_publish(&app_name, &stream_key) else {
                return session
                    .reject_request(
                        request_id,
                        "NetStream.Publish.BadName",
                        "Stream is already being published",
                    )
                    .context("failed to reject RTMP publish");
            };
            state.reset_decoders();
            state.stream = Some(stream);
            session
                .accept_request(request_id)
                .context("failed to accept RTMP publish")
        }
        ServerSessionEvent::PublishStreamFinished { .. } => {
            if let Some(stream) = state.stream.take() {
                shared.end_publish(&stream);
            }
            Ok(Vec::new())
        }
        ServerSessionEvent::VideoDataReceived { data, .. } => {
            state.push_video(&data);
            Ok(Vec::new())
        }
        ServerSessionEvent::AudioDataReceived { data, .. } => {
            state.push_audio(&data);
            Ok(Vec::new())
        }
        _ => Ok(Vec::new()),
    }
}

struct AvcConfig {
    length_size: usize,
    parameter_sets: Vec<u8>,
}

fn parse_avc_config(record: &[u8]) -> Result<AvcConfig> {
    if record.len() < 7 || record[0] != 1 {
        return Err(Error::new("invalid AVC decoder configuration"));
    }
    let length_size = usize::from(record[4] & 0x03) + 1;
    let sps_count = usize::from(record[5] & 0x1f);
    let mut offset = 6;
    let mut parameter_sets = Vec::new();

    for _ in 0..sps_count {
        let len = read_u16(record, offset).context("truncated AVC SPS")?;
        offset += 2;
        let end = offset
            .checked_add(len)
            .filter(|&end| end <= record.len())
            .context("truncated AVC SPS")?;
        parameter_sets.extend_from_slice(&[0, 0, 0, 1]);
        parameter_sets.extend_from_slice(&record[offset..end]);
        offset = end;
    }

    let pps_count = *record.get(offset).context("truncated AVC PPS count")?;
    offset += 1;
    for _ in 0..pps_count {
        let len = read_u16(record, offset).context("truncated AVC PPS")?;
        offset += 2;
        let end = offset
            .checked_add(len)
            .filter(|&end| end <= record.len())
            .context("truncated AVC PPS")?;
        parameter_sets.extend_from_slice(&[0, 0, 0, 1]);
        parameter_sets.extend_from_slice(&record[offset..end]);
        offset = end;
    }

    Ok(AvcConfig {
        length_size,
        parameter_sets,
    })
}

fn annex_b_from_avcc(payload: &[u8], length_size: usize) -> Result<Vec<u8>> {
    if !(1..=4).contains(&length_size) {
        return Err(Error::new("invalid AVC NAL length size"));
    }
    let mut annex_b = Vec::with_capacity(payload.len() + 16);
    let mut offset = 0;
    while offset < payload.len() {
        if offset + length_size > payload.len() {
            return Err(Error::new("truncated AVC NAL length"));
        }
        let mut len = 0_usize;
        for byte in &payload[offset..offset + length_size] {
            len = (len << 8) | usize::from(*byte);
        }
        offset += length_size;
        let end = offset
            .checked_add(len)
            .filter(|&end| end <= payload.len())
            .context("truncated AVC NAL unit")?;
        if len > 0 {
            // AUD・SEI・filler などはデコードに不要。NVENC は filler を混ぜるため、
            // デコーダに渡す前に取り除いておく。
            let nal_type = payload[offset] & 0x1f;
            if matches!(nal_type, 1..=5 | 7 | 8) {
                annex_b.extend_from_slice(&[0, 0, 0, 1]);
                annex_b.extend_from_slice(&payload[offset..end]);
            }
        }
        offset = end;
    }
    Ok(annex_b)
}

fn read_u16(data: &[u8], offset: usize) -> Option<usize> {
    let bytes = data.get(offset..offset + 2)?;
    Some(usize::from(u16::from_be_bytes([bytes[0], bytes[1]])))
}

fn yuv_to_rgb(yuv: &openh264::decoder::DecodedYUV<'_>) -> Result<Frame> {
    let (width, height) = yuv.dimensions();
    if width == 0 || height == 0 {
        return Err(Error::new("invalid H.264 frame size"));
    }
    let mut rgb = vec![0_u8; yuv.rgb8_len()];
    yuv.write_rgb8(&mut rgb);
    image::RgbImage::from_raw(width as u32, height as u32, rgb)
        .ok_or_else(|| Error::new("failed to build H.264 frame"))
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SPS: [u8; 5] = [0x67, 0x42, 0xc0, 0x28, 0x01];
    const PPS: [u8; 2] = [0x68, 0xce];

    fn avc_config() -> Vec<u8> {
        let mut record = vec![1, 0x42, 0xc0, 0x28, 0xff, 0xe1];
        record.extend((SPS.len() as u16).to_be_bytes());
        record.extend(SPS);
        record.push(1);
        record.extend((PPS.len() as u16).to_be_bytes());
        record.extend(PPS);
        record
    }

    #[test]
    fn parses_avc_config_and_builds_parameter_sets() {
        let config = parse_avc_config(&avc_config()).unwrap();
        assert_eq!(config.length_size, 4);
        assert_eq!(
            config.parameter_sets,
            [
                0, 0, 0, 1, 0x67, 0x42, 0xc0, 0x28, 0x01, 0, 0, 0, 1, 0x68, 0xce
            ]
        );
    }

    #[test]
    fn rejects_truncated_avc_config() {
        assert!(parse_avc_config(&[1, 0x42, 0xc0]).is_err());
        let mut record = avc_config();
        record.truncate(record.len() - 1);
        assert!(parse_avc_config(&record).is_err());
    }

    #[test]
    fn converts_avcc_to_annex_b() {
        let payload = [0, 0, 0, 2, 0x65, 0x88, 0, 0, 0, 1, 0x41];
        assert_eq!(
            annex_b_from_avcc(&payload, 4).unwrap(),
            [0, 0, 0, 1, 0x65, 0x88, 0, 0, 0, 1, 0x41]
        );
    }

    #[test]
    fn drops_sei_and_filler_nals() {
        let payload = [
            0, 0, 0, 1, 0x06, // SEI
            0, 0, 0, 3, 0x0c, 0xff, 0xff, // filler
            0, 0, 0, 2, 0x65, 0x88, // IDR
        ];
        assert_eq!(
            annex_b_from_avcc(&payload, 4).unwrap(),
            [0, 0, 0, 1, 0x65, 0x88]
        );
    }

    #[test]
    fn rejects_truncated_avcc() {
        assert!(annex_b_from_avcc(&[0, 0, 0], 4).is_err());
        assert!(annex_b_from_avcc(&[0, 0, 0, 9, 1], 4).is_err());
    }
}
