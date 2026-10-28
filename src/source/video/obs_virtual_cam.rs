use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::System::Memory::{
    FILE_MAP_READ, MEMORY_BASIC_INFORMATION, MEMORY_MAPPED_VIEW_ADDRESS, MapViewOfFile,
    OpenFileMappingW, UnmapViewOfFile, VirtualQuery,
};
use windows::core::{HSTRING, PCWSTR};

use super::convert;
use super::{FEED_CAPACITY, Frame, Kind};
use crate::error::{Context, Error, Result};
use crate::source::{Feed, Origin, Producer, Source, SourceId};

const MAPPING_NAME: &str = "OBSVirtualCamVideo";
const FRAME_HEADER_BYTES: usize = 32;
const SLOT_COUNT: usize = 3;
const STATE_READY: u32 = 2;
const STATE_STOPPING: u32 = 3;
/// 新しいフレームが届かないままこの時間が過ぎたら、OBS側が終了したとみなす。
const FRAME_TIMEOUT: Duration = Duration::from_secs(2);

#[repr(C)]
struct QueueHeader {
    write_idx: AtomicU32,
    read_idx: AtomicU32,
    state: AtomicU32,
    offsets: [u32; SLOT_COUNT],
    queue_type: u32,
    cx: u32,
    cy: u32,
    interval: u64,
    reserved: [u32; 8],
}

const _: () = assert!(std::mem::size_of::<QueueHeader>() == 80);

struct Mapping {
    handle: HANDLE,
    view: MEMORY_MAPPED_VIEW_ADDRESS,
}

unsafe impl Send for Mapping {}

impl Mapping {
    fn open() -> Result<Self> {
        unsafe {
            let name = HSTRING::from(MAPPING_NAME);
            let handle = OpenFileMappingW(FILE_MAP_READ.0, false, PCWSTR(name.as_ptr()))
                .context("OBS Virtual Camera is not running")?;
            let view = MapViewOfFile(handle, FILE_MAP_READ, 0, 0, 0);
            if view.Value.is_null() {
                let _ = CloseHandle(handle);
                return Err(Error::new("failed to map OBS Virtual Camera memory"));
            }
            Ok(Self { handle, view })
        }
    }

    fn header(&self) -> &QueueHeader {
        unsafe { &*self.view.Value.cast::<QueueHeader>() }
    }

    fn bytes(&self) -> *const u8 {
        self.view.Value.cast()
    }

    fn len(&self) -> usize {
        let mut info = MEMORY_BASIC_INFORMATION::default();
        let written = unsafe {
            VirtualQuery(
                Some(self.view.Value.cast_const()),
                &mut info,
                std::mem::size_of::<MEMORY_BASIC_INFORMATION>(),
            )
        };
        if written == 0 { 0 } else { info.RegionSize }
    }
}

impl Drop for Mapping {
    fn drop(&mut self) {
        unsafe {
            let _ = UnmapViewOfFile(self.view);
            let _ = CloseHandle(self.handle);
        }
    }
}

pub(super) fn scan() -> Vec<Source<Kind>> {
    if is_available() {
        vec![source()]
    } else {
        Vec::new()
    }
}

pub(super) fn open() -> Result<Feed<Frame>> {
    Ok(Feed::spawn(FEED_CAPACITY, move |producer| {
        let result = producer.clone();
        if let Err(err) = run(producer) {
            tracing::error!("OBS Virtual Camera capture failed: {err:#}");
            result.send(Err(err));
        }
    }))
}

fn source() -> Source<Kind> {
    Source {
        id: SourceId::new("obs-virtualcam", "default"),
        name: "OBS Virtual Camera".to_string(),
        kind: Kind::ObsVirtualCam,
        origin: Origin::Scanned,
    }
}

fn is_available() -> bool {
    unsafe {
        let name = HSTRING::from(MAPPING_NAME);
        match OpenFileMappingW(FILE_MAP_READ.0, false, PCWSTR(name.as_ptr())) {
            Ok(handle) => {
                let _ = CloseHandle(handle);
                true
            }
            Err(_) => false,
        }
    }
}

fn run(producer: Producer<Frame>) -> Result<()> {
    let mapping = Mapping::open()?;
    let mapped_len = mapping.len();
    if mapped_len < std::mem::size_of::<QueueHeader>() {
        return Err(Error::new("OBS Virtual Camera shared memory is too small"));
    }
    let header = mapping.header();
    let mut last = u32::MAX;
    let mut last_frame = Instant::now();
    let mut warned = false;

    while producer.is_open() {
        let state = header.state.load(Ordering::Acquire);
        if state == STATE_STOPPING {
            return Err(Error::new("OBS Virtual Camera stopped"));
        }
        if state != STATE_READY || header.queue_type != 0 {
            if last_frame.elapsed() >= FRAME_TIMEOUT {
                return Err(Error::new("OBS Virtual Camera stopped sending frames"));
            }
            std::thread::sleep(Duration::from_millis(5));
            continue;
        }

        let index = header.read_idx.load(Ordering::Acquire);
        if index == last {
            if last_frame.elapsed() >= FRAME_TIMEOUT {
                return Err(Error::new("OBS Virtual Camera stopped sending frames"));
            }
            std::thread::sleep(Duration::from_millis(5));
            continue;
        }

        let width = header.cx;
        let height = header.cy;
        let Some(size) = frame_size(width, height) else {
            if !warned {
                tracing::warn!("OBS Virtual Camera returned invalid size: {width}x{height}");
                warned = true;
            }
            std::thread::sleep(Duration::from_millis(5));
            continue;
        };
        let offset = header.offsets[index as usize % SLOT_COUNT] as usize;
        let end = offset
            .checked_add(FRAME_HEADER_BYTES)
            .and_then(|start| start.checked_add(size));
        match end {
            Some(end) if end <= mapped_len => {}
            _ => return Err(Error::new("OBS Virtual Camera frame is out of bounds")),
        }
        let data = unsafe {
            std::slice::from_raw_parts(mapping.bytes().add(offset + FRAME_HEADER_BYTES), size)
        };
        let nv12 = data.to_vec();

        if let Some(frame) = convert::nv12_to_rgb(&nv12, width, height) {
            if !producer.send(Ok(frame)) {
                return Ok(());
            }
            last = index;
            last_frame = Instant::now();
        } else if !warned {
            tracing::warn!("failed to convert OBS Virtual Camera frame");
            warned = true;
        }
    }

    Ok(())
}

fn frame_size(width: u32, height: u32) -> Option<usize> {
    if width == 0 || height == 0 || !width.is_multiple_of(2) || !height.is_multiple_of(2) {
        return None;
    }
    (width as usize)
        .checked_mul(height as usize)?
        .checked_mul(3)?
        .checked_div(2)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_layout_matches_obs() {
        assert_eq!(std::mem::size_of::<QueueHeader>(), 80);
    }

    #[test]
    fn frame_size_requires_even_dimensions() {
        assert_eq!(frame_size(1920, 1080), Some(1920 * 1080 * 3 / 2));
        assert_eq!(frame_size(1921, 1080), None);
        assert_eq!(frame_size(1920, 1081), None);
        assert_eq!(frame_size(0, 1080), None);
    }

    #[test]
    fn slot_wraps() {
        for index in 0..10 {
            assert_eq!(index as usize % SLOT_COUNT, (index % 3) as usize);
        }
    }
}
