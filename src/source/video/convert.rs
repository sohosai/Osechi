use image::RgbImage;

use super::Frame;

pub(super) fn bgr_to_rgb(
    data: &[u8],
    width: u32,
    height: u32,
    stride: usize,
    bottom_up: bool,
) -> Option<Frame> {
    let width_px = usize::try_from(width).ok()?;
    let height_px = usize::try_from(height).ok()?;
    if width_px == 0 || height_px == 0 {
        return None;
    }

    let row_bytes = width_px.checked_mul(3)?;
    let stride = stride.max(row_bytes);
    if data.len() < stride.checked_mul(height_px)? {
        return None;
    }

    let mut rgb = vec![0u8; row_bytes.checked_mul(height_px)?];
    for y in 0..height_px {
        let src_y = if bottom_up { height_px - 1 - y } else { y };
        let src = &data[src_y * stride..src_y * stride + row_bytes];
        let dst = &mut rgb[y * row_bytes..(y + 1) * row_bytes];
        for x in 0..width_px {
            dst[x * 3] = src[x * 3 + 2];
            dst[x * 3 + 1] = src[x * 3 + 1];
            dst[x * 3 + 2] = src[x * 3];
        }
    }

    RgbImage::from_raw(width, height, rgb)
}

pub(super) fn yuy2_to_rgb(data: &[u8], width: u32, height: u32) -> Option<Frame> {
    let width_px = usize::try_from(width).ok()?;
    let height_px = usize::try_from(height).ok()?;
    if width_px == 0 || height_px == 0 || !width_px.is_multiple_of(2) {
        return None;
    }
    if data.len() < width_px.checked_mul(height_px)?.checked_mul(2)? {
        return None;
    }

    let mut rgb = vec![0u8; width_px.checked_mul(height_px)?.checked_mul(3)?];
    for i in 0..width_px * height_px / 2 {
        let y0 = i32::from(data[i * 4]);
        let u = i32::from(data[i * 4 + 1]) - 128;
        let y1 = i32::from(data[i * 4 + 2]);
        let v = i32::from(data[i * 4 + 3]) - 128;
        let base = i * 6;
        let (r0, g0, b0) = yuv_to_rgb(y0, u, v);
        let (r1, g1, b1) = yuv_to_rgb(y1, u, v);
        rgb[base..base + 6].copy_from_slice(&[r0, g0, b0, r1, g1, b1]);
    }

    RgbImage::from_raw(width, height, rgb)
}

pub(super) fn nv12_to_rgb(data: &[u8], width: u32, height: u32) -> Option<Frame> {
    let width_px = usize::try_from(width).ok()?;
    let height_px = usize::try_from(height).ok()?;
    if width_px == 0
        || height_px == 0
        || !width_px.is_multiple_of(2)
        || !height_px.is_multiple_of(2)
    {
        return None;
    }
    if data.len() < width_px.checked_mul(height_px)?.checked_mul(3)? / 2 {
        return None;
    }

    let y_plane = &data[..width_px * height_px];
    let uv_plane = &data[width_px * height_px..];
    let mut rgb = vec![0u8; width_px.checked_mul(height_px)?.checked_mul(3)?];

    for row in 0..height_px {
        for col in 0..width_px {
            let y = i32::from(y_plane[row * width_px + col]);
            let uv_index = (row / 2) * width_px + (col / 2) * 2;
            let u = i32::from(uv_plane[uv_index]) - 128;
            let v = i32::from(uv_plane[uv_index + 1]) - 128;
            let (r, g, b) = yuv_to_rgb(y, u, v);
            let base = (row * width_px + col) * 3;
            rgb[base..base + 3].copy_from_slice(&[r, g, b]);
        }
    }

    RgbImage::from_raw(width, height, rgb)
}

fn yuv_to_rgb(y: i32, u: i32, v: i32) -> (u8, u8, u8) {
    let r = ((y * 256 + 359 * v) >> 8).clamp(0, 255) as u8;
    let g = ((y * 256 - 88 * u - 183 * v) >> 8).clamp(0, 255) as u8;
    let b = ((y * 256 + 454 * u) >> 8).clamp(0, 255) as u8;
    (r, g, b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_bgr_bottom_up_to_rgb_top_down() {
        let mut bgr = vec![0u8; 2 * 3 * 2];
        bgr[0] = 255;
        bgr[3] = 255;
        bgr[8] = 255;
        bgr[11] = 255;

        let rgb = bgr_to_rgb(&bgr, 2, 2, 6, true).unwrap();

        assert_eq!(&rgb.as_raw()[0..3], &[255, 0, 0]);
        assert_eq!(&rgb.as_raw()[6..9], &[0, 0, 255]);
    }

    #[test]
    fn converts_bgr_top_down_to_rgb() {
        let bgr = [0, 0, 255, 0, 255, 0];
        let rgb = bgr_to_rgb(&bgr, 1, 2, 3, false).unwrap();
        assert_eq!(rgb.as_raw(), &[255, 0, 0, 0, 255, 0]);
    }

    #[test]
    fn converts_yuy2_black_and_white() {
        let yuy2 = [0, 128, 255, 128];
        let rgb = yuy2_to_rgb(&yuy2, 2, 1).unwrap();
        assert_eq!(rgb.as_raw(), &[0, 0, 0, 255, 255, 255]);
    }

    #[test]
    fn converts_nv12_black_and_white() {
        let black = [0, 0, 0, 0, 128, 128];
        let rgb = nv12_to_rgb(&black, 2, 2).unwrap();
        assert_eq!(rgb.as_raw(), &[0; 12]);

        let white = [255, 255, 255, 255, 128, 128];
        let rgb = nv12_to_rgb(&white, 2, 2).unwrap();
        assert_eq!(rgb.as_raw(), &[255; 12]);
    }

    #[test]
    fn rejects_short_or_odd_buffers() {
        assert!(bgr_to_rgb(&[0; 5], 2, 2, 6, false).is_none());
        assert!(yuy2_to_rgb(&[0; 4], 3, 1).is_none());
        assert!(nv12_to_rgb(&[0; 5], 2, 2).is_none());
        assert!(nv12_to_rgb(&[0; 6], 3, 2).is_none());
    }
}
