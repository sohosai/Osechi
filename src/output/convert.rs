//! 映像フレームを、エンコーダに渡す I420(YUV 4:2:0)に変換する。
//!
//! 出力の大きさは固定で、元の縦横比を保って収まる最大の大きさに(双線形補間で)拡大縮小し、
//! 余白は黒で埋める。色はBT.709・リミテッドレンジ(HD映像の標準)にする。

use std::thread;

use crate::source::video::Frame;

/// 変換に使うスレッドの上限。1080pなら4本で1フレーム数msに収まる。
const MAX_THREADS: usize = 4;

/// I420形式の画像。Y は全画素、U・V は縦横半分。
pub struct I420 {
    width: usize,
    height: usize,
    y: Vec<u8>,
    u: Vec<u8>,
    v: Vec<u8>,
}

impl I420 {
    /// 黒で埋めた画像を作る。幅・高さは偶数であること。
    pub fn new(width: usize, height: usize) -> Self {
        assert!(
            width.is_multiple_of(2) && height.is_multiple_of(2),
            "I420 needs even dimensions"
        );
        let chroma = width / 2 * height / 2;
        Self {
            width,
            height,
            y: vec![BLACK_Y; width * height],
            u: vec![NEUTRAL_UV; chroma],
            v: vec![NEUTRAL_UV; chroma],
        }
    }

    pub fn width(&self) -> usize {
        self.width
    }

    pub fn height(&self) -> usize {
        self.height
    }

    pub fn planes(&self) -> (&[u8], &[u8], &[u8]) {
        (&self.y, &self.u, &self.v)
    }

    /// 全面を黒にする。
    pub fn fill_black(&mut self) {
        self.y.fill(BLACK_Y);
        self.u.fill(NEUTRAL_UV);
        self.v.fill(NEUTRAL_UV);
    }

    /// `frame` を縦横比を保って中央に収め、余白を黒にして書き込む。
    pub fn fill_from(&mut self, frame: &Frame) {
        let (src_w, src_h) = (frame.width() as usize, frame.height() as usize);
        if src_w == 0 || src_h == 0 {
            self.fill_black();
            return;
        }
        let (width, height) = (self.width, self.height);
        let (inner_w, inner_h) = fit(src_w, src_h, width, height);
        let xs = taps(src_w, width, inner_w);
        let ys = taps(src_h, height, inner_h);
        let rgb = frame.as_raw().as_slice();

        let chroma_width = width / 2;
        let pairs = height / 2;
        let threads = thread::available_parallelism()
            .map_or(1, |n| n.get())
            .clamp(1, MAX_THREADS);
        let pairs_per_band = pairs.div_ceil(threads);

        thread::scope(|scope| {
            let bands = self
                .y
                .chunks_mut(pairs_per_band * 2 * width)
                .zip(self.u.chunks_mut(pairs_per_band * chroma_width))
                .zip(self.v.chunks_mut(pairs_per_band * chroma_width))
                .enumerate();
            for (band, ((y, u), v)) in bands {
                let (xs, ys) = (&xs, &ys);
                scope.spawn(move || {
                    let first_pair = band * pairs_per_band;
                    let source = Source {
                        rgb,
                        stride: src_w * 3,
                    };
                    for (i, (u_row, v_row)) in u
                        .chunks_mut(chroma_width)
                        .zip(v.chunks_mut(chroma_width))
                        .enumerate()
                    {
                        let row = (first_pair + i) * 2;
                        let (top, bottom) =
                            y[i * 2 * width..(i + 1) * 2 * width].split_at_mut(width);
                        convert_row_pair(
                            &source,
                            xs,
                            (&ys[row], &ys[row + 1]),
                            (top, bottom),
                            (u_row, v_row),
                        );
                    }
                });
            }
        });
    }
}

/// 黒の Y 値(リミテッドレンジの下端)。
const BLACK_Y: u8 = 16;
/// 無彩色の U・V 値。
const NEUTRAL_UV: u8 = 128;

/// 出力の1画素が、元画像のどこを参照するか。`None` は余白(黒)。
type Tap = Option<Sample>;

/// 双線形補間の参照位置。`weight` は `next` 側の重み(0-256)。
#[derive(Debug, Clone, Copy, PartialEq)]
struct Sample {
    index: usize,
    next: usize,
    weight: u32,
}

/// 元画像 `src_w`x`src_h` を `width`x`height` に縦横比を保って収めたときの (幅, 高さ)。偶数に丸める。
fn fit(src_w: usize, src_h: usize, width: usize, height: usize) -> (usize, usize) {
    // 幅に合わせると高さがはみ出すなら、高さに合わせる
    if src_w * height > src_h * width {
        let scaled = (src_h * width / src_w).min(height);
        (width, scaled / 2 * 2)
    } else {
        let scaled = (src_w * height / src_h).min(width);
        (scaled / 2 * 2, height)
    }
}

/// 出力の1軸(長さ `out`)について、中央に長さ `inner` で元画像(長さ `src`)を置いたときの参照位置。
fn taps(src: usize, out: usize, inner: usize) -> Vec<Tap> {
    let start = (out - inner) / 2;
    let scale = src as f32 / inner as f32;
    (0..out)
        .map(|i| {
            if i < start || i >= start + inner {
                return None;
            }
            let position = ((i - start) as f32 + 0.5) * scale - 0.5;
            let position = position.clamp(0.0, (src - 1) as f32);
            let index = position.floor() as usize;
            Some(Sample {
                index,
                next: (index + 1).min(src - 1),
                weight: ((position - index as f32) * 256.0).round() as u32,
            })
        })
        .collect()
}

/// RGB8 の元画像。
struct Source<'a> {
    rgb: &'a [u8],
    stride: usize,
}

impl Source<'_> {
    /// 出力の1画素の RGB。余白なら黒。
    fn pixel(&self, x: &Tap, y: &Tap) -> [i32; 3] {
        let (Some(x), Some(y)) = (x, y) else {
            return [0; 3];
        };
        let at = |row: usize, col: usize| row * self.stride + col * 3;
        let (a, b) = (at(y.index, x.index), at(y.index, x.next));
        let (c, d) = (at(y.next, x.index), at(y.next, x.next));
        let (wx, wy) = (x.weight, y.weight);
        let mut out = [0; 3];
        for (ch, value) in out.iter_mut().enumerate() {
            let p = |offset: usize| u32::from(self.rgb[offset + ch]);
            let top = p(a) * (256 - wx) + p(b) * wx;
            let bottom = p(c) * (256 - wx) + p(d) * wx;
            *value = ((top * (256 - wy) + bottom * wy + (1 << 15)) >> 16) as i32;
        }
        out
    }
}

/// 出力の2行分(Y の2行と、U・V の1行)を書く。
fn convert_row_pair(
    source: &Source,
    xs: &[Tap],
    (y0, y1): (&Tap, &Tap),
    (top, bottom): (&mut [u8], &mut [u8]),
    (u_row, v_row): (&mut [u8], &mut [u8]),
) {
    for (cx, (u, v)) in u_row.iter_mut().zip(v_row.iter_mut()).enumerate() {
        let x = cx * 2;
        let pixels = [
            source.pixel(&xs[x], y0),
            source.pixel(&xs[x + 1], y0),
            source.pixel(&xs[x], y1),
            source.pixel(&xs[x + 1], y1),
        ];
        top[x] = luma(pixels[0]);
        top[x + 1] = luma(pixels[1]);
        bottom[x] = luma(pixels[2]);
        bottom[x + 1] = luma(pixels[3]);

        let mut sum = [0; 3];
        for pixel in pixels {
            for ch in 0..3 {
                sum[ch] += pixel[ch];
            }
        }
        let average = sum.map(|total| (total + 2) >> 2);
        (*u, *v) = chroma(average);
    }
}

/// BT.709・リミテッドレンジの輝度(16-235)。係数は 256 倍の整数。
fn luma([r, g, b]: [i32; 3]) -> u8 {
    (((47 * r + 157 * g + 16 * b + 128) >> 8) + 16).clamp(16, 235) as u8
}

/// BT.709・リミテッドレンジの色差(16-240)。
fn chroma([r, g, b]: [i32; 3]) -> (u8, u8) {
    let u = ((-26 * r - 86 * g + 112 * b + 128) >> 8) + 128;
    let v = ((112 * r - 102 * g - 10 * b + 128) >> 8) + 128;
    (u.clamp(16, 240) as u8, v.clamp(16, 240) as u8)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(width: u32, height: u32, rgb: [u8; 3]) -> Frame {
        Frame::from_pixel(width, height, image::Rgb(rgb))
    }

    #[test]
    fn primary_colors_follow_bt709_limited_range() {
        assert_eq!(luma([0, 0, 0]), 16);
        assert_eq!(luma([255, 255, 255]), 235);
        assert_eq!(luma([255, 0, 0]), 63);
        assert_eq!(chroma([255, 255, 255]), (128, 128));
        assert_eq!(chroma([255, 0, 0]), (102, 240));
        assert_eq!(chroma([0, 0, 255]), (240, 118));
    }

    #[test]
    fn same_size_frame_fills_everything() {
        let mut image = I420::new(64, 36);
        image.fill_from(&solid(64, 36, [255, 255, 255]));
        let (y, u, v) = image.planes();
        assert!(y.iter().all(|&p| p == 235));
        assert!(u.iter().chain(v).all(|&p| p == 128));
    }

    #[test]
    fn narrower_frame_is_pillarboxed() {
        // 4:3 を 16:9 に収めると左右に黒帯が付く
        let mut image = I420::new(64, 36);
        image.fill_from(&solid(40, 30, [255, 255, 255]));
        let (y, _, _) = image.planes();
        let row = &y[18 * 64..19 * 64];
        assert_eq!(row[0], BLACK_Y);
        assert_eq!(row[63], BLACK_Y);
        assert_eq!(row[32], 235);
        assert_eq!(fit(40, 30, 64, 36), (48, 36));
    }

    #[test]
    fn wider_frame_is_letterboxed() {
        assert_eq!(fit(210, 90, 64, 36), (64, 26));
        let mut image = I420::new(64, 36);
        image.fill_from(&solid(210, 90, [255, 255, 255]));
        let (y, _, _) = image.planes();
        assert_eq!(y[0], BLACK_Y);
        assert_eq!(y[18 * 64 + 32], 235);
    }

    #[test]
    fn upscales_720p_to_1080p() {
        let mut image = I420::new(1920, 1080);
        image.fill_from(&solid(1280, 720, [0, 0, 255]));
        let (y, u, v) = image.planes();
        assert!(y.iter().all(|&p| p == luma([0, 0, 255])));
        assert!(u.iter().all(|&p| p == 240));
        assert!(v.iter().all(|&p| p == 118));
    }

    #[test]
    fn taps_interpolate_between_neighbors() {
        // 2倍に拡大すると、出力の画素は元の画素の間を 1/4・3/4 で補間する
        let taps = taps(2, 4, 4);
        assert_eq!(
            taps[1],
            Some(Sample {
                index: 0,
                next: 1,
                weight: 64
            })
        );
        assert_eq!(taps[3].unwrap().index, 1);
    }
}
