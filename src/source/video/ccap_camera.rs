use ccap::{Convert, FrameOrientation, PixelFormat, PropertyName, Provider, VideoFrame};
use image::RgbImage;

use super::{FEED_CAPACITY, Frame, Kind};
use crate::error::{Error, Result};
use crate::source::{Feed, Origin, Producer, Source, SourceId};

pub(super) fn scan() -> Vec<Source<Kind>> {
    let result = Provider::new().and_then(|provider| provider.list_devices());
    match result {
        Ok(devices) => devices.into_iter().map(source).collect(),
        Err(err) => {
            tracing::debug!("ccap scan failed: {err}");
            Vec::new()
        }
    }
}

pub(super) fn open(name: String) -> Result<Feed<Frame>> {
    Ok(Feed::spawn(FEED_CAPACITY, move |producer| {
        let result = producer.clone();
        if let Err(err) = run(&name, producer) {
            tracing::error!("ccap capture failed: {err:#}");
            result.send(Err(err));
        }
    }))
}

fn source(name: String) -> Source<Kind> {
    Source {
        id: SourceId::new("camera-ccap", &name),
        name: name.clone(),
        kind: Kind::Ccap { name },
        origin: Origin::Scanned,
    }
}

fn run(name: &str, producer: Producer<Frame>) -> Result<()> {
    let mut provider = Provider::with_device_name(name)
        .map_err(|err| Error::new(format!("failed to open {name}: {err}")))?;
    provider
        .set_property(
            PropertyName::PixelFormatOutput,
            PixelFormat::Rgb24.to_c_enum() as f64,
        )
        .map_err(|err| Error::new(format!("failed to set output format: {err}")))?;
    provider
        .start()
        .map_err(|err| Error::new(format!("failed to start {name}: {err}")))?;

    while producer.is_open() {
        match provider.grab_frame(1000) {
            Ok(Some(frame)) => {
                let Some(frame) = frame_to_rgb(&frame) else {
                    tracing::warn!("ccap returned an unsupported frame");
                    continue;
                };
                if !producer.send(Ok(frame)) {
                    break;
                }
            }
            Ok(None) => {}
            Err(err) => return Err(Error::new(format!("failed to capture frame: {err}"))),
        }
    }

    let _ = provider.stop();
    Ok(())
}

fn frame_to_rgb(frame: &VideoFrame) -> Option<Frame> {
    let info = frame.info().ok()?;
    let width = info.width;
    let height = info.height;
    let bottom_up = info.orientation == FrameOrientation::BottomToTop;
    match info.pixel_format {
        PixelFormat::Rgb24 => {
            let data = info.data_planes[0]?;
            copy_rgb_rows(data, info.strides[0] as usize, width, height, bottom_up)
        }
        PixelFormat::Bgr24 => {
            let data = info.data_planes[0]?;
            copy_rows(
                data,
                info.strides[0] as usize,
                width,
                height,
                bottom_up,
                3,
                |p| [p[2], p[1], p[0]],
            )
        }
        PixelFormat::Bgra32 => {
            let data = info.data_planes[0]?;
            copy_rows(
                data,
                info.strides[0] as usize,
                width,
                height,
                bottom_up,
                4,
                |p| [p[2], p[1], p[0]],
            )
        }
        PixelFormat::Rgba32 => {
            let data = info.data_planes[0]?;
            copy_rows(
                data,
                info.strides[0] as usize,
                width,
                height,
                bottom_up,
                4,
                |p| [p[0], p[1], p[2]],
            )
        }
        PixelFormat::Nv12 => {
            let y = info.data_planes[0]?;
            let uv = info.data_planes[1]?;
            let rgb = Convert::nv12_to_rgb24(
                y,
                info.strides[0] as usize,
                uv,
                info.strides[1] as usize,
                width,
                height,
            )
            .ok()?;
            flip_if_needed(RgbImage::from_raw(width, height, rgb)?, bottom_up)
        }
        PixelFormat::I420 => {
            let y = info.data_planes[0]?;
            let u = info.data_planes[1]?;
            let v = info.data_planes[2]?;
            let rgb = Convert::i420_to_rgb24(
                y,
                info.strides[0] as usize,
                u,
                info.strides[1] as usize,
                v,
                info.strides[2] as usize,
                width,
                height,
            )
            .ok()?;
            flip_if_needed(RgbImage::from_raw(width, height, rgb)?, bottom_up)
        }
        _ => None,
    }
}

fn copy_rows(
    data: &[u8],
    stride: usize,
    width: u32,
    height: u32,
    bottom_up: bool,
    bytes_per_pixel: usize,
    pixel: impl Fn(&[u8]) -> [u8; 3],
) -> Option<Frame> {
    let width_px = width as usize;
    let height_px = height as usize;
    let row_bytes = width_px.checked_mul(3)?;
    let stride = stride.max(width_px.checked_mul(bytes_per_pixel)?);
    if data.len() < stride.checked_mul(height_px)? {
        return None;
    }
    let mut rgb = vec![0u8; row_bytes.checked_mul(height_px)?];
    for row in 0..height_px {
        let source_row = if bottom_up { height_px - 1 - row } else { row };
        let src = &data[source_row * stride..source_row * stride + width_px * bytes_per_pixel];
        let dst = &mut rgb[row * row_bytes..(row + 1) * row_bytes];
        for col in 0..width_px {
            let p = &src[col * bytes_per_pixel..(col + 1) * bytes_per_pixel];
            dst[col * 3..col * 3 + 3].copy_from_slice(&pixel(p));
        }
    }
    RgbImage::from_raw(width, height, rgb)
}

fn copy_rgb_rows(
    data: &[u8],
    stride: usize,
    width: u32,
    height: u32,
    bottom_up: bool,
) -> Option<Frame> {
    let width_px = width as usize;
    let height_px = height as usize;
    let row_bytes = width_px.checked_mul(3)?;
    let stride = stride.max(row_bytes);
    if data.len() < stride.checked_mul(height_px)? {
        return None;
    }
    let mut rgb = vec![0u8; row_bytes.checked_mul(height_px)?];
    for row in 0..height_px {
        let source_row = if bottom_up { height_px - 1 - row } else { row };
        rgb[row * row_bytes..(row + 1) * row_bytes]
            .copy_from_slice(&data[source_row * stride..source_row * stride + row_bytes]);
    }
    RgbImage::from_raw(width, height, rgb)
}

fn flip_if_needed(frame: Frame, bottom_up: bool) -> Option<Frame> {
    if !bottom_up {
        return Some(frame);
    }
    let width = frame.width() as usize;
    let height = frame.height() as usize;
    let row_bytes = width * 3;
    let mut rgb = vec![0u8; row_bytes * height];
    for row in 0..height {
        let source_row = height - 1 - row;
        rgb[row * row_bytes..(row + 1) * row_bytes]
            .copy_from_slice(&frame.as_raw()[source_row * row_bytes..(source_row + 1) * row_bytes]);
    }
    RgbImage::from_raw(width as u32, height as u32, rgb)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copies_rgb_rows_with_stride() {
        let data = [
            1, 2, 3, 4, 5, 6, 99, 99, //
            7, 8, 9, 10, 11, 12, 99, 99,
        ];
        let frame = copy_rows(&data, 8, 2, 2, false, 3, |p| [p[0], p[1], p[2]]).unwrap();
        assert_eq!(frame.as_raw(), &[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12]);
    }

    #[test]
    fn copies_rgb_rows_bottom_up() {
        let data = [1, 2, 3, 4, 5, 6, 99, 99, 7, 8, 9, 10, 11, 12, 99, 99];
        let frame = copy_rgb_rows(&data, 8, 2, 2, true).unwrap();
        assert_eq!(frame.as_raw(), &[7, 8, 9, 10, 11, 12, 1, 2, 3, 4, 5, 6]);
    }

    #[test]
    fn converts_bgr_and_bgra() {
        let bgr = copy_rows(&[1, 2, 3], 3, 1, 1, false, 3, |p| [p[2], p[1], p[0]]).unwrap();
        assert_eq!(bgr.as_raw(), &[3, 2, 1]);

        let bgra = copy_rows(&[1, 2, 3, 255], 4, 1, 1, false, 4, |p| [p[2], p[1], p[0]]).unwrap();
        assert_eq!(bgra.as_raw(), &[3, 2, 1]);
    }

    #[test]
    fn rejects_short_rows() {
        assert!(copy_rows(&[0; 5], 6, 2, 1, false, 3, |p| [p[0], p[1], p[2]]).is_none());
    }
}
