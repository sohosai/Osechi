//! 映像(H.264, OpenH264)と音声(AAC-LC, fdk-aac)のエンコード。
//!
//! どちらもソースからビルドして組み込むので、利用者が別にエンコーダを入れる必要はない。

use openh264::OpenH264API;
use openh264::encoder::{
    BitRate, Complexity, Encoder, EncoderConfig, FrameRate, FrameType, IntraFramePeriod,
    RateControlMode, UsageType, VuiConfig,
};
use openh264::formats::YUVSlices;

use super::convert::I420;
use crate::error::{Context, Error, Result};
use crate::mixer::{CHANNELS, SAMPLE_RATE};

/// 1回のエンコードで出てきた映像。
pub struct EncodedVideo {
    /// IDRフレーム(ここから再生を始められる)か
    pub keyframe: bool,
    /// NALユニット(スタートコードを除いたもの)
    pub nals: Vec<Vec<u8>>,
}

/// H.264 エンコーダ。キーフレームは呼び出し側が [`VideoEncoder::encode`] の引数で決める。
pub struct VideoEncoder {
    encoder: Encoder,
}

impl VideoEncoder {
    /// 目標ビットレート `bitrate`(bps)・`frame_rate` fps のエンコーダを作る。
    pub fn new(bitrate: u32, frame_rate: u32) -> Result<Self> {
        let config = EncoderConfig::new()
            .bitrate(BitRate::from_bps(bitrate))
            .max_frame_rate(FrameRate::from_hz(frame_rate as f32))
            .rate_control_mode(RateControlMode::Bitrate)
            .usage_type(UsageType::CameraVideoRealTime)
            .complexity(Complexity::Medium)
            // フレームを飛ばすとタイムスタンプが一定間隔でなくなるので飛ばさない
            .skip_frames(false)
            // キーフレームの位置は呼び出し側で揃えるので、エンコーダの判断では入れない
            .intra_frame_period(IntraFramePeriod::auto())
            .scene_change_detect(false)
            .vui(VuiConfig::bt709());
        let encoder = Encoder::with_api_config(OpenH264API::from_source(), config)
            .context("failed to create H.264 encoder")?;
        Ok(Self { encoder })
    }

    /// 1フレームをエンコードする。`keyframe` なら IDR フレームにする。
    pub fn encode(&mut self, image: &I420, keyframe: bool) -> Result<EncodedVideo> {
        if keyframe {
            self.encoder.force_intra_frame();
        }
        let (y, u, v) = image.planes();
        let (width, height) = (image.width(), image.height());
        let source = YUVSlices::new((y, u, v), (width, height), (width, width / 2, width / 2));
        let bitstream = self
            .encoder
            .encode(&source)
            .context("failed to encode video frame")?;

        let mut nals = Vec::new();
        for layer in (0..bitstream.num_layers()).filter_map(|i| bitstream.layer(i)) {
            for nal in (0..layer.nal_count()).filter_map(|i| layer.nal_unit(i)) {
                nals.push(strip_start_code(nal).to_vec());
            }
        }
        Ok(EncodedVideo {
            keyframe: bitstream.frame_type() == FrameType::IDR,
            nals,
        })
    }
}

/// Annex B のスタートコード(`00 00 01` / `00 00 00 01`)を取り除く。
fn strip_start_code(nal: &[u8]) -> &[u8] {
    nal.strip_prefix(&[0, 0, 0, 1])
        .or_else(|| nal.strip_prefix(&[0, 0, 1]))
        .unwrap_or(nal)
}

/// AAC 1フレームあたりのサンプル数(1チャンネル分)。
pub const AAC_FRAME_SIZE: usize = 1024;

/// AAC-LC エンコーダ(48kHz・ステレオ)。
pub struct AudioEncoder {
    encoder: fdk_aac::enc::Encoder,
    output: Vec<u8>,
}

impl AudioEncoder {
    /// 目標ビットレート `bitrate`(bps)のエンコーダを作る。
    pub fn new(bitrate: u32) -> Result<Self> {
        let params = fdk_aac::enc::EncoderParams {
            bit_rate: fdk_aac::enc::BitRate::Cbr(bitrate),
            sample_rate: SAMPLE_RATE,
            channels: fdk_aac::enc::ChannelMode::Stereo,
            // RTMP(FLV)ではADTSヘッダを付けない生のAACフレームを送る
            transport: fdk_aac::enc::Transport::Raw,
            audio_object_type: fdk_aac::enc::AudioObjectType::Mpeg4LowComplexity,
        };
        let encoder = fdk_aac::enc::Encoder::new(params)
            .map_err(|err| Error::new(format!("failed to create AAC encoder: {err}")))?;
        Ok(Self {
            encoder,
            output: vec![0; 8192],
        })
    }

    /// 1フレーム分([`AAC_FRAME_SIZE`] × 2ch)の interleaved PCM をエンコードする。
    /// エンコーダの遅延のため、最初の数回は何も出てこない(`None`)。
    pub fn encode(&mut self, pcm: &[i16]) -> Result<Option<Vec<u8>>> {
        debug_assert_eq!(pcm.len(), AAC_FRAME_SIZE * CHANNELS as usize);
        let info = self
            .encoder
            .encode(pcm, &mut self.output)
            .map_err(|err| Error::new(format!("failed to encode audio frame: {err}")))?;
        Ok((info.output_size > 0).then(|| self.output[..info.output_size].to_vec()))
    }
}

/// -1.0..=1.0 の f32 を 16bit PCM にする。
pub fn to_i16(sample: f32) -> i16 {
    (sample.clamp(-1.0, 1.0) * f32::from(i16::MAX)) as i16
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_both_start_code_lengths() {
        assert_eq!(strip_start_code(&[0, 0, 0, 1, 0x67, 1]), [0x67, 1]);
        assert_eq!(strip_start_code(&[0, 0, 1, 0x68]), [0x68]);
        assert_eq!(strip_start_code(&[0x65, 2]), [0x65, 2]);
    }

    #[test]
    fn converts_float_to_i16() {
        assert_eq!(to_i16(0.0), 0);
        assert_eq!(to_i16(1.0), i16::MAX);
        assert_eq!(to_i16(2.0), i16::MAX);
        assert_eq!(to_i16(-1.0), -i16::MAX);
    }

    #[test]
    fn encodes_keyframe_with_parameter_sets() {
        let mut encoder = VideoEncoder::new(2_000_000, 30).unwrap();
        let image = I420::new(320, 180);
        let first = encoder.encode(&image, true).unwrap();
        assert!(first.keyframe);
        let types: Vec<u8> = first.nals.iter().map(|nal| nal[0] & 0x1f).collect();
        assert!(types.contains(&7), "SPS missing: {types:?}");
        assert!(types.contains(&8), "PPS missing: {types:?}");
        assert!(types.contains(&5), "IDR slice missing: {types:?}");

        let second = encoder.encode(&image, false).unwrap();
        assert!(!second.keyframe);
        let third = encoder.encode(&image, true).unwrap();
        assert!(third.keyframe);
    }

    #[test]
    fn audio_encoder_eventually_outputs_frames() {
        let mut encoder = AudioEncoder::new(128_000).unwrap();
        let silence = vec![0i16; AAC_FRAME_SIZE * 2];
        let outputs = (0..8)
            .filter_map(|_| encoder.encode(&silence).unwrap())
            .count();
        assert!(outputs > 0);
    }
}
