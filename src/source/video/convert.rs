use image::RgbImage;

use super::Frame;

/// NV12の変換で使う作業バッファ。取得スレッド内で使い回す。
pub(super) struct Nv12Converter {
    nv12: Vec<u8>,
}

impl Nv12Converter {
    pub(super) fn new() -> Self {
        Self { nv12: Vec::new() }
    }

    pub(super) fn convert(&mut self, data: &[u8], width: u32, height: u32) -> Option<Frame> {
        self.nv12.clear();
        self.nv12.extend_from_slice(data);
        nv12_to_rgb(&self.nv12, width, height)
    }
}

fn nv12_to_rgb(data: &[u8], width: u32, height: u32) -> Option<Frame> {
    let width_px = usize::try_from(width).ok()?;
    let height_px = usize::try_from(height).ok()?;
    let y_size = plane_size(width_px, height_px)?;
    if data.len() < y_size.checked_mul(3)? / 2 {
        return None;
    }

    let y_plane = &data[..y_size];
    let uv_plane = &data[y_size..];
    let rgb_size = y_size.checked_mul(3)?;
    // 1画素を4バイトで書き、末尾のはみ出し分は最後に truncate する。
    let mut rgb = vec![0u8; rgb_size + 1];
    let row_bytes = width_px * 3;

    for pair in 0..height_px / 2 {
        let y0 = &y_plane[pair * 2 * width_px..][..width_px];
        let y1 = &y_plane[(pair * 2 + 1) * width_px..][..width_px];
        let uv = &uv_plane[pair * width_px..][..width_px];
        let base0 = pair * 2 * row_bytes;
        let base1 = base0 + row_bytes;

        convert_row(&mut rgb, base0, y0, uv);
        convert_row(&mut rgb, base1, y1, uv);
    }

    rgb.truncate(rgb_size);
    RgbImage::from_raw(width, height, rgb)
}

fn convert_row(rgb: &mut [u8], base: usize, y: &[u8], uv: &[u8]) {
    for x in (0..y.len()).step_by(2) {
        let u = i32::from(uv[x]) - 128;
        let v = i32::from(uv[x + 1]) - 128;
        store(rgb, base + x * 3, yuv_to_rgb(i32::from(y[x]), u, v));
        store(
            rgb,
            base + (x + 1) * 3,
            yuv_to_rgb(i32::from(y[x + 1]), u, v),
        );
    }
}

fn plane_size(width: usize, height: usize) -> Option<usize> {
    if width == 0 || height == 0 || !width.is_multiple_of(2) || !height.is_multiple_of(2) {
        return None;
    }
    width.checked_mul(height)
}

fn store(rgb: &mut [u8], base: usize, (r, g, b): (u8, u8, u8)) {
    rgb[base..base + 4].copy_from_slice(&[r, g, b, 0]);
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

    fn convert(data: &[u8], width: u32, height: u32) -> Option<Frame> {
        Nv12Converter::new().convert(data, width, height)
    }

    #[test]
    fn converts_limited_range_black_and_white() {
        let black = [16, 16, 16, 16, 128, 128];
        let rgb = convert(&black, 2, 2).unwrap();
        assert_eq!(rgb.as_raw(), &[0; 12]);

        let white = [235, 235, 235, 235, 128, 128];
        let rgb = convert(&white, 2, 2).unwrap();
        assert_eq!(rgb.as_raw(), &[255; 12]);
    }

    #[test]
    fn converts_bt709_primary_colors() {
        // output/convert.rs が RGB(255, 0, 0) から作る BT.709 リミテッドレンジの値
        let red = [63, 63, 63, 63, 102, 240];
        let rgb = convert(&red, 2, 2).unwrap();
        for pixel in rgb.pixels() {
            assert!(pixel[0] > 250, "red: {pixel:?}");
            assert!(pixel[1] < 5, "green: {pixel:?}");
            assert!(pixel[2] < 5, "blue: {pixel:?}");
        }
    }

    #[test]
    fn writes_every_pixel_without_shifting() {
        let data = [
            16, 235, 16, 235, // Y 1行目
            235, 16, 235, 16, // Y 2行目
            128, 128, 128, 128, // UV
        ];
        let rgb = convert(&data, 4, 2).unwrap();
        let expected: Vec<u8> = [0, 255, 0, 255, 255, 0, 255, 0]
            .into_iter()
            .flat_map(|value| [value; 3])
            .collect();
        assert_eq!(rgb.as_raw(), &expected);
    }

    #[test]
    fn rejects_short_or_odd_buffers() {
        assert!(convert(&[0; 5], 2, 2).is_none());
        assert!(convert(&[0; 6], 3, 2).is_none());
    }
}
