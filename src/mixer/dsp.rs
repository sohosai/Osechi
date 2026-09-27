//! 音声信号の計算。状態を持たない関数だけを置く。

/// フェーダーの下端のdB値。フェーダー位置 0.0-1.0 を -60dB〜0dB に線形に割り当てる。
const FLOOR_DB: f32 = -60.0;

/// フェーダー位置(0.0-1.0)をdBにする。0.0 は -∞dB(無音)。
pub fn gain_to_db(gain: f32) -> f32 {
    if gain <= 0.0 {
        f32::NEG_INFINITY
    } else {
        FLOOR_DB * (1.0 - gain)
    }
}

/// フェーダー位置(0.0-1.0)を、信号に掛ける線形係数にする。
pub fn gain_to_linear(gain: f32) -> f32 {
    db_to_linear(gain_to_db(gain))
}

fn db_to_linear(db: f32) -> f32 {
    if db.is_infinite() {
        0.0
    } else {
        10f32.powf(db / 20.0)
    }
}

/// ピーク振幅(線形, 0.0-1.0)を、フェーダーと同じ -60dB〜0dB の目盛りでメーター上の位置(0.0-1.0)にする。
///
/// 振幅をそのまま線形に目盛ると -40dB を下回るような静かな信号がほとんど点かないため、dBに揃える。
pub fn level_to_meter(level: f32) -> f32 {
    if level <= 0.0 {
        return 0.0;
    }
    ((20.0 * level.log10() - FLOOR_DB) / -FLOOR_DB).clamp(0.0, 1.0)
}

/// 絶対値の最大。
pub fn peak(samples: &[f32]) -> f32 {
    samples.iter().fold(0.0, |peak, s| peak.max(s.abs()))
}

/// interleavedの任意チャンネル数の音声をステレオにする。
/// 1ch: L/Rに複製。2ch: そのまま。3ch以上: 全チャンネルの平均をL/Rに複製する。
pub fn downmix_to_stereo(samples: &[f32], channels: u16) -> Vec<f32> {
    if channels == 0 {
        return Vec::new();
    }
    let channels = channels as usize;
    let mut out = Vec::with_capacity(samples.len() / channels * 2);
    for frame in samples.chunks_exact(channels) {
        let (l, r) = match frame {
            [mono] => (*mono, *mono),
            [l, r] => (*l, *r),
            _ => {
                let avg = frame.iter().sum::<f32>() / channels as f32;
                (avg, avg)
            }
        };
        out.extend([l, r]);
    }
    out
}

/// interleavedステレオ音声のサンプルレートを変換する。
/// チャンク単位で完結する線形補間で、チャンク境界をまたぐ位相は持ち越さない簡易実装。
pub fn resample_stereo(samples: &[f32], from_rate: u32, to_rate: u32) -> Vec<f32> {
    if from_rate == to_rate || samples.is_empty() {
        return samples.to_vec();
    }
    let frames_in = samples.len() / 2;
    if frames_in == 0 {
        return Vec::new();
    }
    let frames_out = (frames_in as u64 * to_rate as u64 / from_rate as u64).max(1) as usize;
    let ratio = from_rate as f64 / to_rate as f64;

    let mut out = Vec::with_capacity(frames_out * 2);
    for i in 0..frames_out {
        let position = i as f64 * ratio;
        let i0 = (position.floor() as usize).min(frames_in - 1);
        let i1 = (i0 + 1).min(frames_in - 1);
        let frac = (position - i0 as f64) as f32;
        for ch in 0..2 {
            let (a, b) = (samples[i0 * 2 + ch], samples[i1 * 2 + ch]);
            out.push(a + (b - a) * frac);
        }
    }
    out
}

/// `src` に `factor` を掛けて `dst` に足し込む。`dst` が短ければ無音で伸ばす。
pub fn add_scaled(dst: &mut Vec<f32>, src: &[f32], factor: f32) {
    if dst.len() < src.len() {
        dst.resize(src.len(), 0.0);
    }
    for (d, s) in dst.iter_mut().zip(src) {
        *d += s * factor;
    }
}

/// tanhによる簡易ソフトクリップ。合算が -1.0..=1.0 を超えても急に歪まず滑らかに飽和させる。
pub fn soft_clip(sample: f32) -> f32 {
    sample.tanh()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gain_zero_is_silent() {
        assert_eq!(gain_to_db(0.0), f32::NEG_INFINITY);
        assert_eq!(gain_to_linear(0.0), 0.0);
    }

    #[test]
    fn gain_one_is_unity() {
        assert!(gain_to_db(1.0).abs() < 1e-4);
        assert!((gain_to_linear(1.0) - 1.0).abs() < 1e-4);
    }

    #[test]
    fn default_fader_position_is_minus_15_db() {
        assert!((gain_to_db(0.75) + 15.0).abs() < 1e-4);
    }

    #[test]
    fn meter_uses_db_scale() {
        assert_eq!(level_to_meter(0.0), 0.0);
        assert!((level_to_meter(1.0) - 1.0).abs() < 1e-6);
        // -54dB 付近の小さな信号でも目盛りの1割ほどは点く
        assert!((level_to_meter(0.002) - 0.1).abs() < 0.01);
        assert_eq!(level_to_meter(1e-6), 0.0);
    }

    #[test]
    fn downmix_mono_duplicates_to_both_channels() {
        assert_eq!(
            downmix_to_stereo(&[0.5, -0.25], 1),
            [0.5, 0.5, -0.25, -0.25]
        );
    }

    #[test]
    fn downmix_stereo_passes_through() {
        assert_eq!(
            downmix_to_stereo(&[0.1, 0.2, 0.3, 0.4], 2),
            [0.1, 0.2, 0.3, 0.4]
        );
    }

    #[test]
    fn downmix_multichannel_averages() {
        assert_eq!(downmix_to_stereo(&[0.0, 1.0, 0.0, -1.0], 4), [0.0, 0.0]);
    }

    #[test]
    fn resample_identity_when_rates_match() {
        let samples = [0.1, 0.2, 0.3, 0.4];
        assert_eq!(resample_stereo(&samples, 48_000, 48_000), samples);
    }

    #[test]
    fn resample_doubles_frame_count_when_rate_doubles() {
        assert_eq!(
            resample_stereo(&[0.0, 0.0, 1.0, 1.0], 24_000, 48_000).len(),
            8
        );
    }

    #[test]
    fn add_scaled_extends_and_accumulates() {
        let mut mix = vec![1.0];
        add_scaled(&mut mix, &[1.0, 2.0], 0.5);
        assert_eq!(mix, [1.5, 1.0]);
    }

    #[test]
    fn soft_clip_keeps_small_values_almost_unchanged() {
        assert!((soft_clip(0.1) - 0.1).abs() < 0.01);
    }

    #[test]
    fn soft_clip_bounds_large_values() {
        // tanh(10) は理論上 1.0 未満だが f32 では 1.0 に丸まりうるので境界値は許容する
        assert!(soft_clip(10.0) <= 1.0);
        assert!(soft_clip(-10.0) >= -1.0);
    }
}
