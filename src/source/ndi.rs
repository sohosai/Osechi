//! NDI の検出と受信。
//!
//! NDI SDK へのリンクは行わず、実行時に `Processing.NDI.Lib` を読み込む。
//! SDK が入っていない環境でも起動でき、その場合は検出エラーとして扱う。

use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int, c_uint, c_void};
use std::path::PathBuf;
use std::ptr::{null, null_mut};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use libloading::Library;

use super::audio::{Chunk, FEED_CAPACITY as AUDIO_CAPACITY};
use super::video::Frame;
use super::{Feed, Origin, Source, SourceId};
use crate::error::{Context, Error, Result};

const VIDEO_CAPACITY: usize = 2;

const FOURCC_RGBX: c_uint = 1_480_738_642;
const FOURCC_RGBA: c_uint = 1_094_862_674;
const FOURCC_BGRX: c_uint = 1_481_787_202;
const FOURCC_BGRA: c_uint = 1_095_911_234;
const FOURCC_FLTP: c_uint = 1_884_572_742;

const FRAME_TYPE_VIDEO: c_int = 1;
const FRAME_TYPE_AUDIO: c_int = 2;
const FRAME_TYPE_ERROR: c_int = 4;

const RECV_COLOR_RGBX_RGBA: c_uint = 2;
const RECV_BANDWIDTH_HIGHEST: c_int = 100;
const RECV_BANDWIDTH_AUDIO_ONLY: c_int = 10;

type FindInstance = *mut c_void;
type RecvInstance = *mut c_void;

#[repr(C)]
#[derive(Clone, Copy)]
struct NdiSource {
    p_ndi_name: *const c_char,
    p_url_address: *const c_char,
}

#[repr(C)]
struct FindSettings {
    show_local_sources: bool,
    p_groups: *const c_char,
    p_extra_ips: *const c_char,
}

#[repr(C)]
struct ReceiverSettings {
    source_to_connect_to: NdiSource,
    color_format: c_uint,
    bandwidth: c_int,
    allow_video_fields: bool,
    p_ndi_recv_name: *const c_char,
}

#[repr(C)]
#[derive(Default)]
struct VideoFrame {
    xres: c_int,
    yres: c_int,
    four_cc: c_uint,
    frame_rate_n: c_int,
    frame_rate_d: c_int,
    picture_aspect_ratio: f32,
    frame_format_type: c_int,
    timecode: i64,
    p_data: *mut u8,
    line_stride_in_bytes: c_int,
    p_metadata: *const c_char,
    timestamp: i64,
}

#[repr(C)]
#[derive(Default)]
struct AudioFrame {
    sample_rate: c_int,
    no_channels: c_int,
    no_samples: c_int,
    timecode: i64,
    four_cc: c_uint,
    p_data: *mut u8,
    channel_stride_in_bytes: c_int,
    p_metadata: *const c_char,
    timestamp: i64,
}

/// 実行時に読み込んだ NDI ランタイム。
struct Runtime {
    _library: Library,
    initialize: unsafe extern "C" fn() -> bool,
    find_create: unsafe extern "C" fn(*const FindSettings) -> FindInstance,
    find_get_current_sources: unsafe extern "C" fn(FindInstance, *mut u32) -> *const NdiSource,
    find_wait_for_sources: unsafe extern "C" fn(FindInstance, u32) -> bool,
    find_destroy: unsafe extern "C" fn(FindInstance),
    recv_create: unsafe extern "C" fn(*const ReceiverSettings) -> RecvInstance,
    recv_capture: unsafe extern "C" fn(
        RecvInstance,
        *mut VideoFrame,
        *mut AudioFrame,
        *mut c_void,
        u32,
    ) -> c_int,
    recv_free_video: unsafe extern "C" fn(RecvInstance, *const VideoFrame),
    recv_free_audio: unsafe extern "C" fn(RecvInstance, *const AudioFrame),
    recv_destroy: unsafe extern "C" fn(RecvInstance),
}

unsafe impl Send for Runtime {}
unsafe impl Sync for Runtime {}

static RUNTIME: OnceLock<std::result::Result<Arc<Runtime>, String>> = OnceLock::new();

impl Runtime {
    fn get() -> Result<Arc<Self>> {
        match RUNTIME.get_or_init(load_runtime) {
            Ok(runtime) => Ok(Arc::clone(runtime)),
            Err(err) => Err(Error::new(err.clone())),
        }
    }

    unsafe fn from_library(library: Library) -> std::result::Result<Self, String> {
        let initialize: unsafe extern "C" fn() -> bool =
            unsafe { symbol(&library, b"NDIlib_initialize\0")? };
        let find_create: unsafe extern "C" fn(*const FindSettings) -> FindInstance =
            unsafe { symbol(&library, b"NDIlib_find_create_v2\0")? };
        let find_get_current_sources: unsafe extern "C" fn(
            FindInstance,
            *mut u32,
        ) -> *const NdiSource = unsafe { symbol(&library, b"NDIlib_find_get_current_sources\0")? };
        let find_wait_for_sources: unsafe extern "C" fn(FindInstance, u32) -> bool =
            unsafe { symbol(&library, b"NDIlib_find_wait_for_sources\0")? };
        let find_destroy: unsafe extern "C" fn(FindInstance) =
            unsafe { symbol(&library, b"NDIlib_find_destroy\0")? };
        let recv_create: unsafe extern "C" fn(*const ReceiverSettings) -> RecvInstance =
            unsafe { symbol(&library, b"NDIlib_recv_create_v3\0")? };
        let recv_capture: unsafe extern "C" fn(
            RecvInstance,
            *mut VideoFrame,
            *mut AudioFrame,
            *mut c_void,
            u32,
        ) -> c_int = unsafe { symbol(&library, b"NDIlib_recv_capture_v3\0")? };
        let recv_free_video: unsafe extern "C" fn(RecvInstance, *const VideoFrame) =
            unsafe { symbol(&library, b"NDIlib_recv_free_video_v2\0")? };
        let recv_free_audio: unsafe extern "C" fn(RecvInstance, *const AudioFrame) =
            unsafe { symbol(&library, b"NDIlib_recv_free_audio_v3\0")? };
        let recv_destroy: unsafe extern "C" fn(RecvInstance) =
            unsafe { symbol(&library, b"NDIlib_recv_destroy\0")? };

        Ok(Self {
            _library: library,
            initialize,
            find_create,
            find_get_current_sources,
            find_wait_for_sources,
            find_destroy,
            recv_create,
            recv_capture,
            recv_free_video,
            recv_free_audio,
            recv_destroy,
        })
    }
}

unsafe fn symbol<T: Copy>(library: &Library, name: &[u8]) -> std::result::Result<T, String> {
    unsafe { library.get::<T>(name) }
        .map(|symbol| *symbol)
        .map_err(|err| format!("{}: {err}", String::from_utf8_lossy(name)))
}

fn load_runtime() -> std::result::Result<Arc<Runtime>, String> {
    let mut errors = Vec::new();
    for path in candidates() {
        let library = match unsafe { Library::new(&path) } {
            Ok(library) => library,
            Err(err) => {
                errors.push(format!("{}: {err}", path.display()));
                continue;
            }
        };
        match unsafe { Runtime::from_library(library) } {
            Ok(runtime) => {
                if unsafe { (runtime.initialize)() } {
                    return Ok(Arc::new(runtime));
                }
                errors.push(format!("{}: NDIlib_initialize failed", path.display()));
            }
            Err(err) => errors.push(format!("{}: {err}", path.display())),
        }
    }

    let detail = if errors.is_empty() {
        String::new()
    } else {
        format!(" ({})", errors.join("; "))
    };
    Err(format!("NDI runtime not found{detail}"))
}

fn candidates() -> Vec<PathBuf> {
    let mut paths = Vec::new();

    #[cfg(target_os = "windows")]
    {
        let name = if cfg!(target_pointer_width = "32") {
            "Processing.NDI.Lib.x86.dll"
        } else {
            "Processing.NDI.Lib.x64.dll"
        };
        for variable in ["NDI_RUNTIME_DIR_V6", "NDI_RUNTIME_DIR_V5"] {
            if let Ok(dir) = std::env::var(variable) {
                let dir = PathBuf::from(dir);
                paths.push(dir.join(name));
                paths.push(dir.join("lib").join(name));
            }
        }
        paths.push(PathBuf::from(r"C:\Program Files\NDI\NDI 6 Tools\Runtime").join(name));
        paths.push(PathBuf::from(r"C:\Program Files\NDI\NDI 6 SDK\Bin\x64").join(name));
        paths.push(PathBuf::from(name));
    }

    #[cfg(target_os = "linux")]
    {
        for name in ["libndi.so.6", "libndi.so.5", "libndi.so"] {
            paths.push(PathBuf::from("/usr/lib").join(name));
            paths.push(PathBuf::from("/usr/local/lib").join(name));
            paths.push(PathBuf::from(name));
        }
    }

    #[cfg(target_os = "macos")]
    {
        for dir in [
            "/Library/NDI 6 SDK/lib/macOS",
            "/Library/NDI SDK for macOS/lib/macOS",
            "/Library/NDI SDK for Apple/lib/macOS",
        ] {
            paths.push(PathBuf::from(dir).join("libndi.dylib"));
        }
        paths.push(PathBuf::from("libndi.dylib"));
    }

    paths
}

/// 検出した NDI ソース。接続に使う名前と URL を持つ。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceRef {
    pub name: String,
    pub url: String,
}

impl SourceRef {
    pub fn id(&self) -> SourceId {
        SourceId::new("ndi", &self.name)
    }

    pub fn audio_id(&self) -> SourceId {
        SourceId::new("ndi-audio", &self.name)
    }
}

/// NDI ソースの検出。バックグラウンドで一覧を更新する。
pub struct Discovery {
    sources: Arc<Mutex<Vec<SourceRef>>>,
    error: Arc<Mutex<Option<String>>>,
    shutdown: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Discovery {
    pub fn start() -> Self {
        let sources = Arc::new(Mutex::new(Vec::new()));
        let error = Arc::new(Mutex::new(None));
        let shutdown = Arc::new(AtomicBool::new(false));
        let thread = {
            let sources = Arc::clone(&sources);
            let error = Arc::clone(&error);
            let shutdown = Arc::clone(&shutdown);
            thread::Builder::new()
                .name("ndi-discovery".to_string())
                .spawn(move || discovery_loop(sources, error, shutdown))
                .ok()
        };
        Self {
            sources,
            error,
            shutdown,
            thread,
        }
    }

    pub fn sources(&self) -> Vec<SourceRef> {
        lock(&self.sources).clone()
    }

    pub fn error(&self) -> Option<String> {
        lock(&self.error).clone()
    }
}

impl Drop for Discovery {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn discovery_loop(
    sources: Arc<Mutex<Vec<SourceRef>>>,
    error: Arc<Mutex<Option<String>>>,
    shutdown: Arc<AtomicBool>,
) {
    if let Err(err) = discover(&sources, &shutdown) {
        let message = format!("{err:#}");
        tracing::warn!("NDI discovery failed: {message}");
        if !message.starts_with("NDI runtime not found") {
            *lock(&error) = Some(message);
        }
    }
}

fn discover(sources: &Mutex<Vec<SourceRef>>, shutdown: &AtomicBool) -> Result<()> {
    let runtime = Runtime::get()?;
    if !unsafe { (runtime.initialize)() } {
        return Err(Error::new("NDIlib_initialize failed"));
    }

    let settings = FindSettings {
        show_local_sources: true,
        p_groups: null(),
        p_extra_ips: null(),
    };
    let instance = unsafe { (runtime.find_create)(&settings) };
    if instance.is_null() {
        return Err(Error::new("failed to create NDI finder"));
    }
    let finder = Finder {
        runtime: Arc::clone(&runtime),
        instance,
    };

    while !shutdown.load(Ordering::Relaxed) {
        unsafe { (finder.runtime.find_wait_for_sources)(finder.instance, 500) };
        if shutdown.load(Ordering::Relaxed) {
            break;
        }

        let mut count = 0_u32;
        let ptr = unsafe { (finder.runtime.find_get_current_sources)(finder.instance, &mut count) };
        let mut found = Vec::new();
        if !ptr.is_null() {
            for index in 0..count as usize {
                let source = unsafe { &*ptr.add(index) };
                let Some(name) = c_string(source.p_ndi_name) else {
                    continue;
                };
                let url = c_string(source.p_url_address).unwrap_or_default();
                found.push(SourceRef { name, url });
            }
        }
        found.sort_by(|a, b| a.name.cmp(&b.name));
        found.dedup_by(|a, b| a.name == b.name);
        *lock(sources) = found;
    }

    Ok(())
}

struct Finder {
    runtime: Arc<Runtime>,
    instance: FindInstance,
}

impl Drop for Finder {
    fn drop(&mut self) {
        unsafe { (self.runtime.find_destroy)(self.instance) };
    }
}

struct Receiver {
    runtime: Arc<Runtime>,
    instance: RecvInstance,
}

impl Drop for Receiver {
    fn drop(&mut self) {
        unsafe { (self.runtime.recv_destroy)(self.instance) };
    }
}

struct SourceStrings {
    name: CString,
    url: CString,
}

impl SourceStrings {
    fn new(info: &SourceRef) -> Result<Self> {
        Ok(Self {
            name: CString::new(info.name.clone()).context("invalid NDI source name")?,
            url: CString::new(info.url.clone()).context("invalid NDI source URL")?,
        })
    }

    fn raw(&self) -> NdiSource {
        NdiSource {
            p_ndi_name: self.name.as_ptr(),
            p_url_address: self.url.as_ptr(),
        }
    }
}

fn connect(
    runtime: &Arc<Runtime>,
    info: &SourceRef,
    bandwidth: c_int,
    name: &str,
) -> Result<Receiver> {
    let source = SourceStrings::new(info)?;
    let receiver_name = CString::new(name).context("invalid NDI receiver name")?;
    let settings = ReceiverSettings {
        source_to_connect_to: source.raw(),
        color_format: RECV_COLOR_RGBX_RGBA,
        bandwidth,
        allow_video_fields: false,
        p_ndi_recv_name: receiver_name.as_ptr(),
    };
    let instance = unsafe { (runtime.recv_create)(&settings) };
    if instance.is_null() {
        return Err(Error::new(format!(
            "failed to connect to NDI source: {}",
            info.name
        )));
    }
    Ok(Receiver {
        runtime: Arc::clone(runtime),
        instance,
    })
}

pub fn open_video(info: &SourceRef) -> Result<Feed<Frame>> {
    let runtime = Runtime::get()?;
    let info = info.clone();
    Ok(Feed::spawn(VIDEO_CAPACITY, move |producer| {
        if let Err(err) = capture_video(runtime, &info, &producer) {
            producer.send(Err(err));
        }
    }))
}

fn capture_video(
    runtime: Arc<Runtime>,
    info: &SourceRef,
    producer: &super::Producer<Frame>,
) -> Result<()> {
    let receiver = connect(&runtime, info, RECV_BANDWIDTH_HIGHEST, "Osechi Video")?;

    while producer.is_open() {
        let mut frame = VideoFrame::default();
        let kind = unsafe {
            (receiver.runtime.recv_capture)(
                receiver.instance,
                &mut frame,
                null_mut(),
                null_mut(),
                100,
            )
        };
        match kind {
            FRAME_TYPE_VIDEO => {
                let image = unsafe { video_frame_to_rgb(&frame) };
                unsafe { (receiver.runtime.recv_free_video)(receiver.instance, &frame) };
                if !producer.send(image) {
                    break;
                }
            }
            FRAME_TYPE_ERROR => {
                producer.send(Err(Error::new(format!(
                    "NDI source disconnected: {}",
                    info.name
                ))));
                thread::sleep(Duration::from_millis(250));
            }
            _ => {}
        }
    }
    Ok(())
}

pub fn open_audio(info: &SourceRef) -> Result<Feed<Chunk>> {
    let runtime = Runtime::get()?;
    let info = info.clone();
    Ok(Feed::spawn(AUDIO_CAPACITY, move |producer| {
        if let Err(err) = capture_audio(runtime, &info, &producer) {
            producer.send(Err(err));
        }
    }))
}

fn capture_audio(
    runtime: Arc<Runtime>,
    info: &SourceRef,
    producer: &super::Producer<Chunk>,
) -> Result<()> {
    let receiver = connect(&runtime, info, RECV_BANDWIDTH_AUDIO_ONLY, "Osechi Audio")?;

    while producer.is_open() {
        let mut frame = AudioFrame::default();
        let kind = unsafe {
            (receiver.runtime.recv_capture)(
                receiver.instance,
                null_mut(),
                &mut frame,
                null_mut(),
                100,
            )
        };
        match kind {
            FRAME_TYPE_AUDIO => {
                let chunk = unsafe { audio_frame_to_chunk(&frame) };
                unsafe { (receiver.runtime.recv_free_audio)(receiver.instance, &frame) };
                if !producer.send(chunk) {
                    break;
                }
            }
            FRAME_TYPE_ERROR => {
                producer.send(Err(Error::new(format!(
                    "NDI source disconnected: {}",
                    info.name
                ))));
                thread::sleep(Duration::from_millis(250));
            }
            _ => {}
        }
    }
    Ok(())
}

unsafe fn video_frame_to_rgb(frame: &VideoFrame) -> Result<Frame> {
    let width = frame.xres.max(0) as u32;
    let height = frame.yres.max(0) as u32;
    if width == 0 || height == 0 || frame.p_data.is_null() {
        return Err(Error::new("invalid NDI video frame"));
    }

    let row = width as usize * 4;
    let stride = if frame.line_stride_in_bytes > 0 {
        frame.line_stride_in_bytes as usize
    } else {
        row
    };
    if stride < row {
        return Err(Error::new("invalid NDI video stride"));
    }
    let len = stride
        .checked_mul(height as usize - 1)
        .and_then(|len| len.checked_add(row))
        .ok_or_else(|| Error::new("NDI video frame too large"))?;
    let data = unsafe { std::slice::from_raw_parts(frame.p_data, len) };

    let mut rgba = Vec::with_capacity(row * height as usize);
    for row_index in 0..height as usize {
        let start = row_index * stride;
        rgba.extend_from_slice(&data[start..start + row]);
    }

    match frame.four_cc {
        FOURCC_RGBX | FOURCC_RGBA => {}
        FOURCC_BGRX | FOURCC_BGRA => {
            for pixel in rgba.as_chunks_mut::<4>().0 {
                pixel.swap(0, 2);
            }
        }
        other => {
            return Err(Error::new(format!("unsupported NDI video format: {other}")));
        }
    }

    let rgba = image::RgbaImage::from_raw(width, height, rgba)
        .ok_or_else(|| Error::new("failed to build NDI video frame"))?;
    Ok(image::DynamicImage::ImageRgba8(rgba).into_rgb8())
}

unsafe fn audio_frame_to_chunk(frame: &AudioFrame) -> Result<Chunk> {
    let channels = frame.no_channels.max(0) as usize;
    let samples = frame.no_samples.max(0) as usize;
    let sample_rate = frame.sample_rate.max(0) as u32;
    if channels == 0 || samples == 0 || sample_rate == 0 || frame.p_data.is_null() {
        return Err(Error::new("invalid NDI audio frame"));
    }
    if frame.four_cc != FOURCC_FLTP {
        return Err(Error::new(format!(
            "unsupported NDI audio format: {}",
            frame.four_cc
        )));
    }
    let stride = frame.channel_stride_in_bytes.max(0) as usize;
    if stride < samples * 4 || !stride.is_multiple_of(4) {
        return Err(Error::new("invalid NDI audio stride"));
    }
    let len = stride
        .checked_mul(channels - 1)
        .and_then(|len| len.checked_add(samples * 4))
        .ok_or_else(|| Error::new("NDI audio frame too large"))?;
    let data = unsafe { std::slice::from_raw_parts(frame.p_data, len) };

    let mut interleaved = Vec::with_capacity(channels * samples);
    for sample in 0..samples {
        for channel in 0..channels {
            let offset = channel * stride + sample * 4;
            let value = f32::from_ne_bytes([
                data[offset],
                data[offset + 1],
                data[offset + 2],
                data[offset + 3],
            ]);
            interleaved.push(value);
        }
    }

    Ok(Chunk {
        samples: interleaved,
        sample_rate,
        channels: channels as u16,
    })
}

fn c_string(ptr: *const c_char) -> Option<String> {
    if ptr.is_null() {
        return None;
    }
    Some(
        unsafe { CStr::from_ptr(ptr) }
            .to_string_lossy()
            .into_owned(),
    )
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

pub fn video_sources(info: &SourceRef) -> Source<super::video::Kind> {
    Source {
        id: info.id(),
        name: format!("NDI {}", info.name),
        kind: super::video::Kind::Ndi(info.clone()),
        origin: Origin::Ndi,
    }
}

pub fn audio_sources(info: &SourceRef) -> Source<super::audio::Kind> {
    Source {
        id: info.audio_id(),
        name: format!("NDI {} (Audio)", info.name),
        kind: super::audio::Kind::Ndi(info.clone()),
        origin: Origin::Ndi,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_ids_are_distinct_for_video_and_audio() {
        let info = SourceRef {
            name: "MACHINE (Camera)".to_string(),
            url: "192.168.1.10:5963".to_string(),
        };
        assert_eq!(info.id().to_string(), "ndi:MACHINE (Camera)");
        assert_eq!(info.audio_id().to_string(), "ndi-audio:MACHINE (Camera)");
        assert_ne!(info.id(), info.audio_id());
    }

    #[test]
    fn source_strings_reject_nul_bytes() {
        let info = SourceRef {
            name: "bad\0name".to_string(),
            url: String::new(),
        };
        assert!(SourceStrings::new(&info).is_err());
    }

    #[test]
    fn converts_rgbx_rows_with_stride() {
        let mut data = vec![0_u8; 3 * 8];
        data[..4].copy_from_slice(&[1, 2, 3, 255]);
        data[4..8].copy_from_slice(&[4, 5, 6, 255]);
        data[8..12].copy_from_slice(&[7, 8, 9, 255]);
        data[12..16].copy_from_slice(&[10, 11, 12, 255]);
        let frame = VideoFrame {
            xres: 2,
            yres: 2,
            four_cc: FOURCC_RGBX,
            p_data: data.as_mut_ptr(),
            line_stride_in_bytes: 8,
            ..Default::default()
        };

        let image = unsafe { video_frame_to_rgb(&frame) }.unwrap();
        assert_eq!(image.dimensions(), (2, 2));
        assert_eq!(image.as_raw(), &[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12]);
    }
}
