#![allow(non_snake_case)]

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use windows::Win32::Foundation::RPC_E_CHANGED_MODE;
use windows::Win32::Media::DirectShow::{
    IAMStreamConfig, IBaseFilter, ICreateDevEnum, IEnumPins, IFilterGraph2, IGraphBuilder,
    IMediaControl, IMediaFilter, IPin, PIN_DIRECTION, PINDIR_INPUT, PINDIR_OUTPUT,
};
use windows::Win32::Media::IReferenceClock;
use windows::Win32::Media::MediaFoundation::{
    AM_MEDIA_TYPE, CLSID_FilterGraph, CLSID_SystemDeviceEnum, CLSID_VideoInputDeviceCategory,
    FORMAT_VideoInfo, FORMAT_VideoInfo2, MEDIASUBTYPE_RGB24, MEDIATYPE_Video, VIDEOINFOHEADER,
    VIDEOINFOHEADER2,
};
use windows::Win32::System::Com::StructuredStorage::IPropertyBag;
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoTaskMemFree,
    CoUninitialize, IBindCtx, IEnumMoniker, IMoniker,
};
use windows::Win32::System::Variant::{VARIANT, VariantClear, VariantToStringAlloc};
use windows::core::{GUID, HRESULT, HSTRING, IUnknown, IUnknown_Vtbl, Interface, PCWSTR};

use super::convert;
use super::{FEED_CAPACITY, Frame, Kind};
use crate::error::{Context, Error, Result};
use crate::source::{Feed, Origin, Producer, Source, SourceId};

const CLSID_SAMPLE_GRABBER: GUID = GUID::from_u128(0xC1F400A0_3F08_11D3_9F0B_006008039E37);
const CLSID_NULL_RENDERER: GUID = GUID::from_u128(0xC1F400A4_3F08_11D3_9F0B_006008039E37);
const MEDIASUBTYPE_YUY2: GUID = GUID::from_u128(0x32595559_0000_0010_8000_00AA00389B71);
const MEDIASUBTYPE_NV12: GUID = GUID::from_u128(0x3231564E_0000_0010_8000_00AA00389B71);

#[derive(Debug, Clone)]
pub(super) struct Device {
    pub name: String,
    pub path: String,
}

#[derive(Clone, Copy)]
enum PixelFormat {
    Bgr24 { stride: usize, bottom_up: bool },
    Yuy2,
    Nv12,
    Unsupported(GUID),
}

pub(super) fn scan() -> Vec<Source<Kind>> {
    match enumerate() {
        Ok(devices) => devices.into_iter().map(source).collect(),
        Err(err) => {
            tracing::debug!("DirectShow scan failed: {err:#}");
            Vec::new()
        }
    }
}

pub(super) fn open(device: Device) -> Result<Feed<Frame>> {
    Ok(Feed::spawn(FEED_CAPACITY, move |producer| {
        let result = producer.clone();
        if let Err(err) = run(device, producer) {
            tracing::error!("DirectShow capture failed: {err:#}");
            result.send(Err(err));
        }
    }))
}

pub(super) fn is_obs_virtual_camera(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    name.contains("obs") && name.contains("virtual")
}

fn source(device: Device) -> Source<Kind> {
    let key = if device.path.is_empty() {
        device.name.clone()
    } else {
        device.path.clone()
    };
    Source {
        id: SourceId::new("camera-dshow", key),
        name: device.name.clone(),
        kind: Kind::DirectShow {
            friendly_name: device.name,
            device_path: device.path,
        },
        origin: Origin::Scanned,
    }
}

fn enumerate() -> Result<Vec<Device>> {
    let _com = ComGuard::init()?;
    unsafe {
        let dev_enum: ICreateDevEnum =
            CoCreateInstance(&CLSID_SystemDeviceEnum, None, CLSCTX_INPROC_SERVER)
                .context("failed to create DirectShow device enumerator")?;
        let mut enum_moniker: Option<IEnumMoniker> = None;
        dev_enum
            .CreateClassEnumerator(&CLSID_VideoInputDeviceCategory, &mut enum_moniker, 0)
            .context("failed to enumerate DirectShow video devices")?;

        let Some(enum_moniker) = enum_moniker else {
            return Ok(Vec::new());
        };

        let mut devices = Vec::new();
        let mut item = [None; 1];
        loop {
            let hr = enum_moniker.Next(&mut item, None);
            if hr.is_err() {
                break;
            }
            let Some(moniker) = item[0].take() else {
                break;
            };
            let Ok(bag): std::result::Result<IPropertyBag, _> =
                moniker.BindToStorage(None::<&IBindCtx>, None::<&IMoniker>)
            else {
                continue;
            };
            let name = read_property(&bag, "FriendlyName").unwrap_or_default();
            if name.is_empty() {
                continue;
            }
            let path = read_property(&bag, "DevicePath").unwrap_or_default();
            devices.push(Device { name, path });
        }
        Ok(devices)
    }
}

unsafe fn read_property(bag: &IPropertyBag, name: &str) -> Option<String> {
    unsafe {
        let name = HSTRING::from(name);
        let mut variant = VARIANT::default();
        bag.Read(PCWSTR(name.as_ptr()), &mut variant, None).ok()?;

        let allocated = VariantToStringAlloc(&variant).ok();
        let string = allocated.as_ref().and_then(|value| value.to_string().ok());
        if let Some(value) = allocated {
            CoTaskMemFree(Some(value.0.cast()));
        }
        let _ = VariantClear(&mut variant);
        string
    }
}

fn run(device: Device, producer: Producer<Frame>) -> Result<()> {
    let _com = ComGuard::init()?;
    let obs = is_obs_virtual_camera(&device.name);

    unsafe {
        let graph: IGraphBuilder = CoCreateInstance(&CLSID_FilterGraph, None, CLSCTX_INPROC_SERVER)
            .context("failed to create DirectShow filter graph")?;
        let graph: IFilterGraph2 = graph.cast().context("failed to get IFilterGraph2")?;

        let source = find_filter(&device)?;
        graph
            .AddFilter(&source, windows::core::w!("Source"))
            .context("failed to add DirectShow source filter")?;

        let grabber_filter: IBaseFilter =
            CoCreateInstance(&CLSID_SAMPLE_GRABBER, None, CLSCTX_INPROC_SERVER)
                .context("failed to create SampleGrabber")?;
        graph
            .AddFilter(&grabber_filter, windows::core::w!("SampleGrabber"))
            .context("failed to add SampleGrabber")?;
        let grabber: ISampleGrabber = grabber_filter
            .cast()
            .context("failed to query ISampleGrabber")?;

        check(grabber.SetOneShot(0), "SetOneShot failed")?;
        check(grabber.SetBufferSamples(0), "SetBufferSamples failed")?;

        let null_renderer: IBaseFilter =
            CoCreateInstance(&CLSID_NULL_RENDERER, None, CLSCTX_INPROC_SERVER)
                .context("failed to create NullRenderer")?;
        graph
            .AddFilter(&null_renderer, windows::core::w!("NullRenderer"))
            .context("failed to add NullRenderer")?;

        let source_out = find_pin(&source, PINDIR_OUTPUT)?;
        let grabber_in = find_pin(&grabber_filter, PINDIR_INPUT)?;

        if obs {
            if let Err(err) = force_nv12(&source) {
                tracing::warn!("could not force NV12 on DirectShow source: {err:#}");
            }
            set_media_type(&grabber, MEDIASUBTYPE_NV12, FORMAT_VideoInfo)?;
            if let Err(err) = graph.Connect(&source_out, &grabber_in) {
                tracing::warn!("NV12 connect failed ({err}), retrying with any subtype");
                let _ = graph.Disconnect(&source_out);
                let _ = graph.Disconnect(&grabber_in);
                set_media_type(&grabber, GUID::zeroed(), GUID::zeroed())?;
                graph
                    .Connect(&source_out, &grabber_in)
                    .context("failed to connect DirectShow source to SampleGrabber")?;
            }
        } else {
            set_media_type(&grabber, MEDIASUBTYPE_RGB24, GUID::zeroed())?;
            if let Err(err) = graph.Connect(&source_out, &grabber_in) {
                tracing::warn!("RGB24 connect failed ({err}), retrying with any subtype");
                let _ = graph.Disconnect(&source_out);
                let _ = graph.Disconnect(&grabber_in);
                set_media_type(&grabber, GUID::zeroed(), GUID::zeroed())?;
                graph
                    .Connect(&source_out, &grabber_in)
                    .context("failed to connect DirectShow source to SampleGrabber")?;
            }
        }

        let grabber_out = find_pin(&grabber_filter, PINDIR_OUTPUT)?;
        let null_in = find_pin(&null_renderer, PINDIR_INPUT)?;
        graph
            .Connect(&grabber_out, &null_in)
            .context("failed to connect SampleGrabber to NullRenderer")?;

        let mut media_type = AM_MEDIA_TYPE::default();
        check(
            grabber.GetConnectedMediaType(&mut media_type),
            "failed to read connected media type",
        )?;
        let Some((width, height, format)) = describe(&media_type) else {
            free_media_type(&media_type);
            return Err(Error::new("unsupported DirectShow video format"));
        };
        free_media_type(&media_type);

        let running = Arc::new(AtomicBool::new(true));
        let callback = CallbackGuard::new(GrabberCallback::new(
            producer.clone(),
            Arc::clone(&running),
            width,
            height,
            format,
        ));
        check(grabber.SetCallback(callback.raw(), 1), "SetCallback failed")?;

        let control: IMediaControl = graph.cast().context("failed to get IMediaControl")?;
        if obs {
            let filter: IMediaFilter = graph.cast().context("failed to get IMediaFilter")?;
            filter
                .SetSyncSource(None::<&IReferenceClock>)
                .context("SetSyncSource failed")?;
        }

        control.Run().context("failed to run DirectShow graph")?;
        tracing::info!(
            "DirectShow capture started: {} ({}x{}, {})",
            device.name,
            width,
            height,
            format_label(format)
        );

        while running.load(Ordering::Relaxed) && producer.is_open() {
            std::thread::sleep(std::time::Duration::from_millis(50));
        }

        running.store(false, Ordering::Relaxed);
        if let Err(err) = control.Stop() {
            tracing::warn!("IMediaControl::Stop failed: {err}");
        }
        drop(callback);
    }

    Ok(())
}

unsafe fn find_filter(device: &Device) -> Result<IBaseFilter> {
    unsafe {
        let dev_enum: ICreateDevEnum =
            CoCreateInstance(&CLSID_SystemDeviceEnum, None, CLSCTX_INPROC_SERVER)
                .context("failed to create DirectShow device enumerator")?;
        let mut enum_moniker: Option<IEnumMoniker> = None;
        dev_enum
            .CreateClassEnumerator(&CLSID_VideoInputDeviceCategory, &mut enum_moniker, 0)
            .context("failed to enumerate DirectShow video devices")?;

        let Some(enum_moniker) = enum_moniker else {
            return Err(Error::new("no DirectShow video devices"));
        };

        let mut item = [None; 1];
        loop {
            let hr = enum_moniker.Next(&mut item, None);
            if hr.is_err() {
                break;
            }
            let Some(moniker) = item[0].take() else {
                break;
            };
            let Ok(bag): std::result::Result<IPropertyBag, _> =
                moniker.BindToStorage(None::<&IBindCtx>, None::<&IMoniker>)
            else {
                continue;
            };
            let name = read_property(&bag, "FriendlyName").unwrap_or_default();
            let path = read_property(&bag, "DevicePath").unwrap_or_default();
            let matched = if device.path.is_empty() {
                name == device.name
            } else {
                path == device.path
            };
            if matched {
                return moniker
                    .BindToObject(None::<&IBindCtx>, None::<&IMoniker>)
                    .context("failed to open DirectShow device");
            }
        }

        Err(Error::new(format!(
            "DirectShow device not found: {}",
            device.name
        )))
    }
}

unsafe fn find_pin(filter: &IBaseFilter, direction: PIN_DIRECTION) -> Result<IPin> {
    unsafe {
        let pins: IEnumPins = filter.EnumPins().context("EnumPins failed")?;
        let mut item = [None; 1];
        loop {
            let hr = pins.Next(&mut item, None);
            if hr.is_err() {
                break;
            }
            let Some(pin) = item[0].take() else {
                break;
            };
            let matches = pin
                .QueryDirection()
                .is_ok_and(|pin_direction| pin_direction.0 == direction.0);
            if matches && pin.ConnectedTo().is_err() {
                return Ok(pin);
            }
        }
        Err(Error::new("DirectShow pin not found"))
    }
}

unsafe fn force_nv12(source: &IBaseFilter) -> Result<()> {
    unsafe {
        let pins: IEnumPins = source.EnumPins().context("EnumPins failed")?;
        let mut item = [None; 1];
        loop {
            let hr = pins.Next(&mut item, None);
            if hr.is_err() {
                break;
            }
            let Some(pin) = item[0].take() else {
                break;
            };
            if !pin
                .QueryDirection()
                .is_ok_and(|pin_direction| pin_direction.0 == PINDIR_OUTPUT.0)
            {
                continue;
            }
            let Ok(config) = pin.cast::<IAMStreamConfig>() else {
                continue;
            };
            let mut count = 0;
            let mut size = 0;
            if config
                .GetNumberOfCapabilities(&mut count, &mut size)
                .is_err()
                || size <= 0
            {
                continue;
            }

            for index in 0..count {
                let mut caps = vec![0u8; size as usize];
                let mut media_type: *mut AM_MEDIA_TYPE = std::ptr::null_mut();
                if config
                    .GetStreamCaps(index, &mut media_type, caps.as_mut_ptr())
                    .is_err()
                    || media_type.is_null()
                {
                    continue;
                }
                let media_type_ref = &*media_type;
                let is_nv12 = media_type_ref.subtype == MEDIASUBTYPE_NV12
                    && (media_type_ref.formattype == FORMAT_VideoInfo
                        || media_type_ref.formattype == FORMAT_VideoInfo2);
                if is_nv12 {
                    let result = config.SetFormat(media_type);
                    free_stream_caps(media_type_ref);
                    if result.is_ok() {
                        return Ok(());
                    }
                } else {
                    free_stream_caps(media_type_ref);
                }
            }
        }
        Err(Error::new("no NV12 DirectShow stream capability"))
    }
}

unsafe fn set_media_type(grabber: &ISampleGrabber, subtype: GUID, format_type: GUID) -> Result<()> {
    unsafe {
        let media_type = AM_MEDIA_TYPE {
            majortype: MEDIATYPE_Video,
            subtype,
            formattype: format_type,
            ..AM_MEDIA_TYPE::default()
        };
        check(
            grabber.SetMediaType(&media_type),
            "failed to set SampleGrabber media type",
        )
    }
}

fn describe(media_type: &AM_MEDIA_TYPE) -> Option<(u32, u32, PixelFormat)> {
    let (width, height, bottom_up, stride) = dimensions(media_type)?;
    let format = if media_type.subtype == MEDIASUBTYPE_RGB24 {
        PixelFormat::Bgr24 { stride, bottom_up }
    } else if media_type.subtype == MEDIASUBTYPE_YUY2 {
        PixelFormat::Yuy2
    } else if media_type.subtype == MEDIASUBTYPE_NV12 {
        PixelFormat::Nv12
    } else {
        PixelFormat::Unsupported(media_type.subtype)
    };
    Some((width, height, format))
}

fn dimensions(media_type: &AM_MEDIA_TYPE) -> Option<(u32, u32, bool, usize)> {
    if media_type.pbFormat.is_null() {
        return None;
    }
    let (width, height, bottom_up, image_size) = if media_type.formattype == FORMAT_VideoInfo
        && media_type.cbFormat as usize >= std::mem::size_of::<VIDEOINFOHEADER>()
    {
        let header = unsafe { &*(media_type.pbFormat.cast::<VIDEOINFOHEADER>()) };
        let bitmap = header.bmiHeader;
        (
            bitmap.biWidth.unsigned_abs(),
            bitmap.biHeight.unsigned_abs(),
            bitmap.biHeight > 0,
            bitmap.biSizeImage as usize,
        )
    } else if media_type.formattype == FORMAT_VideoInfo2
        && media_type.cbFormat as usize >= std::mem::size_of::<VIDEOINFOHEADER2>()
    {
        let header = unsafe { &*(media_type.pbFormat.cast::<VIDEOINFOHEADER2>()) };
        let bitmap = header.bmiHeader;
        (
            bitmap.biWidth.unsigned_abs(),
            bitmap.biHeight.unsigned_abs(),
            bitmap.biHeight > 0,
            bitmap.biSizeImage as usize,
        )
    } else {
        return None;
    };

    if width == 0 || height == 0 {
        return None;
    }
    let width = width as usize;
    let height = height as usize;
    let row_bytes = width.checked_mul(3)?;
    let stride = if image_size >= row_bytes.checked_mul(height)? {
        image_size / height
    } else {
        (row_bytes + 3) & !3
    };
    Some((width as u32, height as u32, bottom_up, stride))
}

fn check(hr: HRESULT, message: &str) -> Result<()> {
    hr.ok().context(message.to_string())
}

unsafe fn free_media_type(media_type: &AM_MEDIA_TYPE) {
    unsafe {
        if !media_type.pbFormat.is_null() {
            CoTaskMemFree(Some(media_type.pbFormat.cast()));
        }
    }
}

unsafe fn free_stream_caps(media_type: &AM_MEDIA_TYPE) {
    unsafe {
        free_media_type(media_type);
        CoTaskMemFree(Some((media_type as *const AM_MEDIA_TYPE).cast()));
    }
}

fn format_label(format: PixelFormat) -> &'static str {
    match format {
        PixelFormat::Bgr24 { .. } => "RGB24",
        PixelFormat::Yuy2 => "YUY2",
        PixelFormat::Nv12 => "NV12",
        PixelFormat::Unsupported(_) => "unsupported",
    }
}

struct ComGuard {
    owned: bool,
}

impl ComGuard {
    fn init() -> Result<Self> {
        let hr = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        if hr.is_ok() {
            return Ok(Self { owned: true });
        }
        if hr == RPC_E_CHANGED_MODE {
            return Ok(Self { owned: false });
        }
        Err(Error::new(format!("CoInitializeEx failed: {hr:?}")))
    }
}

impl Drop for ComGuard {
    fn drop(&mut self) {
        if self.owned {
            unsafe { CoUninitialize() };
        }
    }
}

#[windows::core::interface("6B652FFF-11FE-4FCE-92AD-0266B5D7C78F")]
unsafe trait ISampleGrabber: IUnknown {
    unsafe fn SetOneShot(&self, one_shot: i32) -> HRESULT;
    unsafe fn SetMediaType(&self, media_type: *const AM_MEDIA_TYPE) -> HRESULT;
    unsafe fn GetConnectedMediaType(&self, media_type: *mut AM_MEDIA_TYPE) -> HRESULT;
    unsafe fn SetBufferSamples(&self, buffer: i32) -> HRESULT;
    unsafe fn GetCurrentBuffer(&self, buffer_size: *mut i32, buffer: *mut i32) -> HRESULT;
    unsafe fn GetCurrentSample(&self, sample: *mut *mut core::ffi::c_void) -> HRESULT;
    unsafe fn SetCallback(&self, callback: *const core::ffi::c_void, method: i32) -> HRESULT;
}

const IID_IUNKNOWN: GUID = GUID::from_u128(0x00000000_0000_0000_C000_000000000046);
const IID_ISAMPLEGRABBER_CB: GUID = GUID::from_u128(0x0579154A_2B53_4994_B0D0_E773148EFF85);
const E_NOINTERFACE: HRESULT = HRESULT(0x80004002u32 as i32);

#[repr(C)]
struct ISampleGrabberCBVtbl {
    query_interface: unsafe extern "system" fn(
        *mut core::ffi::c_void,
        *const GUID,
        *mut *mut core::ffi::c_void,
    ) -> HRESULT,
    add_ref: unsafe extern "system" fn(*mut core::ffi::c_void) -> u32,
    release: unsafe extern "system" fn(*mut core::ffi::c_void) -> u32,
    sample_cb:
        unsafe extern "system" fn(*mut core::ffi::c_void, f64, *mut core::ffi::c_void) -> HRESULT,
    buffer_cb: unsafe extern "system" fn(*mut core::ffi::c_void, f64, *mut u8, i32) -> HRESULT,
}

static SAMPLE_GRABBER_CB_VTBL: ISampleGrabberCBVtbl = ISampleGrabberCBVtbl {
    query_interface: grabber_cb_query_interface,
    add_ref: grabber_cb_add_ref,
    release: grabber_cb_release,
    sample_cb: grabber_cb_sample,
    buffer_cb: grabber_cb_buffer,
};

#[repr(C)]
struct GrabberCallback {
    vtbl: *const ISampleGrabberCBVtbl,
    ref_count: AtomicU32,
    producer: Producer<Frame>,
    running: Arc<AtomicBool>,
    width: u32,
    height: u32,
    format: PixelFormat,
    error_reported: Arc<AtomicBool>,
}

impl GrabberCallback {
    fn new(
        producer: Producer<Frame>,
        running: Arc<AtomicBool>,
        width: u32,
        height: u32,
        format: PixelFormat,
    ) -> Self {
        Self {
            vtbl: &SAMPLE_GRABBER_CB_VTBL,
            ref_count: AtomicU32::new(1),
            producer,
            running,
            width,
            height,
            format,
            error_reported: Arc::new(AtomicBool::new(false)),
        }
    }
}

struct CallbackGuard(*mut core::ffi::c_void);

impl CallbackGuard {
    fn new(callback: GrabberCallback) -> Self {
        Self(Box::into_raw(Box::new(callback)).cast())
    }

    fn raw(&self) -> *mut core::ffi::c_void {
        self.0
    }
}

impl Drop for CallbackGuard {
    fn drop(&mut self) {
        unsafe { grabber_cb_release(self.0) };
    }
}

unsafe extern "system" fn grabber_cb_query_interface(
    this: *mut core::ffi::c_void,
    riid: *const GUID,
    result: *mut *mut core::ffi::c_void,
) -> HRESULT {
    unsafe {
        if result.is_null() {
            return E_NOINTERFACE;
        }
        if !riid.is_null() && (*riid == IID_IUNKNOWN || *riid == IID_ISAMPLEGRABBER_CB) {
            *result = this;
            grabber_cb_add_ref(this);
            HRESULT(0)
        } else {
            *result = std::ptr::null_mut();
            E_NOINTERFACE
        }
    }
}

unsafe extern "system" fn grabber_cb_add_ref(this: *mut core::ffi::c_void) -> u32 {
    unsafe {
        let callback = &*(this as *const GrabberCallback);
        callback
            .ref_count
            .fetch_add(1, Ordering::Relaxed)
            .wrapping_add(1)
    }
}

unsafe extern "system" fn grabber_cb_release(this: *mut core::ffi::c_void) -> u32 {
    unsafe {
        if this.is_null() {
            return 0;
        }
        let callback = &*(this as *const GrabberCallback);
        let previous = callback.ref_count.fetch_sub(1, Ordering::AcqRel);
        if previous == 1 {
            drop(Box::from_raw(this as *mut GrabberCallback));
            0
        } else {
            previous - 1
        }
    }
}

unsafe extern "system" fn grabber_cb_sample(
    _this: *mut core::ffi::c_void,
    _sample_time: f64,
    _sample: *mut core::ffi::c_void,
) -> HRESULT {
    HRESULT(0)
}

unsafe extern "system" fn grabber_cb_buffer(
    this: *mut core::ffi::c_void,
    _sample_time: f64,
    buffer: *mut u8,
    buffer_len: i32,
) -> HRESULT {
    unsafe {
        let callback = &*(this as *const GrabberCallback);
        if !callback.running.load(Ordering::Relaxed) || !callback.producer.is_open() {
            return HRESULT(0);
        }
        let Some(len) = usize::try_from(buffer_len).ok() else {
            return HRESULT(0);
        };
        if buffer.is_null() || len == 0 {
            return HRESULT(0);
        }

        let data = std::slice::from_raw_parts(buffer, len);
        let frame = match callback.format {
            PixelFormat::Bgr24 { stride, bottom_up } => {
                convert::bgr_to_rgb(data, callback.width, callback.height, stride, bottom_up)
            }
            PixelFormat::Yuy2 => convert::yuy2_to_rgb(data, callback.width, callback.height),
            PixelFormat::Nv12 => convert::nv12_to_rgb(data, callback.width, callback.height),
            PixelFormat::Unsupported(subtype) => {
                if !callback.error_reported.swap(true, Ordering::Relaxed) {
                    callback.producer.send(Err(Error::new(format!(
                        "unsupported DirectShow video format: {subtype:?}"
                    ))));
                }
                return HRESULT(0);
            }
        };

        match frame {
            Some(frame) => {
                if !callback.producer.send(Ok(frame)) {
                    callback.running.store(false, Ordering::Relaxed);
                }
            }
            None => {
                if !callback.error_reported.swap(true, Ordering::Relaxed) {
                    callback
                        .producer
                        .send(Err(Error::new("failed to convert DirectShow video frame")));
                }
            }
        }
        HRESULT(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_obs_virtual_camera() {
        assert!(is_obs_virtual_camera("OBS Virtual Camera"));
        assert!(is_obs_virtual_camera("obs virtual camera"));
        assert!(!is_obs_virtual_camera("Integrated Camera"));
    }

    #[test]
    fn formats_have_labels() {
        assert_eq!(format_label(PixelFormat::Nv12), "NV12");
        assert_eq!(format_label(PixelFormat::Yuy2), "YUY2");
    }
}
