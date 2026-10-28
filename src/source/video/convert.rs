use image::RgbImage;

use super::Frame;

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

/// BT.709・リミテッドレンジ(16-235)のYUVをフルレンジRGBに変換する。
/// 係数は 256 倍の整数。
fn yuv_to_rgb(y: i32, u: i32, v: i32) -> (u8, u8, u8) {
    let c = y - 16;
    let r = (298 * c + 459 * v + 128) >> 8;
    let g = (298 * c - 55 * u - 136 * v + 128) >> 8;
    let b = (298 * c + 541 * u + 128) >> 8;
    (clamp(r), clamp(g), clamp(b))
}

fn clamp(value: i32) -> u8 {
    value.clamp(0, 255) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_limited_range_black_and_white() {
        let black = [16, 16, 16, 16, 128, 128];
        let rgb = nv12_to_rgb(&black, 2, 2).unwrap();
        assert_eq!(rgb.as_raw(), &[0; 12]);

        let white = [235, 235, 235, 235, 128, 128];
        let rgb = nv12_to_rgb(&white, 2, 2).unwrap();
        assert_eq!(rgb.as_raw(), &[255; 12]);
    }

    #[test]
    fn converts_bt709_primary_colors() {
        // output/convert.rs が RGB(255, 0, 0) から作る BT.709 リミテッドレンジの値
        let red = [63, 63, 63, 63, 102, 240];
        let rgb = nv12_to_rgb(&red, 2, 2).unwrap();
        for pixel in rgb.pixels() {
            assert!(pixel[0] > 250, "red: {pixel:?}");
            assert!(pixel[1] < 5, "green: {pixel:?}");
            assert!(pixel[2] < 5, "blue: {pixel:?}");
        }
    }

    #[test]
    fn rejects_short_or_odd_buffers() {
        assert!(nv12_to_rgb(&[0; 5], 2, 2).is_none());
        assert!(nv12_to_rgb(&[0; 6], 3, 2).is_none());
    }
}
