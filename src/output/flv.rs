//! RTMPで送る映像・音声メッセージの中身(FLVのタグ本体)を組み立てる。
//!
//! 映像は H.264(AVC)、音声は AAC。どちらも最初に「シーケンスヘッダ」(デコーダの設定)を送り、
//! その後にフレームを送る。

use crate::mixer::{CHANNELS, SAMPLE_RATE};

/// FLVの映像コーデックID(AVC)。onMetaDataでも使う。
pub const VIDEO_CODEC_ID: u32 = 7;
/// FLVの音声コーデックID(AAC)。onMetaDataでも使う。
pub const AUDIO_CODEC_ID: u32 = 10;

/// NALユニットの種類。
fn nal_type(nal: &[u8]) -> u8 {
    nal.first().map_or(0, |byte| byte & 0x1f)
}

const NAL_SPS: u8 = 7;
const NAL_PPS: u8 = 8;
const NAL_AUD: u8 = 9;

/// NALユニットの中から SPS と PPS を探す。
pub fn parameter_sets(nals: &[Vec<u8>]) -> Option<(&[u8], &[u8])> {
    let find = |kind| {
        nals.iter()
            .find(|nal| nal_type(nal) == kind)
            .map(Vec::as_slice)
    };
    Some((find(NAL_SPS)?, find(NAL_PPS)?))
}

/// 映像タグの先頭1バイト: フレーム種別(上位4bit) + コーデックID(下位4bit)。
fn video_header(keyframe: bool) -> u8 {
    let frame_type = if keyframe { 1 } else { 2 };
    (frame_type << 4) | VIDEO_CODEC_ID as u8
}

/// AVCのシーケンスヘッダ(AVCDecoderConfigurationRecord)。
pub fn avc_sequence_header(sps: &[u8], pps: &[u8]) -> Vec<u8> {
    let mut tag = vec![video_header(true), 0, 0, 0, 0];
    tag.extend([
        1,      // configurationVersion
        sps[1], // AVCProfileIndication
        sps[2], // profile_compatibility
        sps[3], // AVCLevelIndication
        0xff,   // NALユニットの長さは4バイト
        0xe1,   // SPSの数: 1
    ]);
    tag.extend((sps.len() as u16).to_be_bytes());
    tag.extend(sps);
    tag.push(1); // PPSの数
    tag.extend((pps.len() as u16).to_be_bytes());
    tag.extend(pps);
    tag
}

/// AVCの1フレーム。NALユニットを4バイトの長さ付きで並べる。
/// SPS・PPS はシーケンスヘッダで送り、AUD は不要なので入れない。
pub fn avc_frame(nals: &[Vec<u8>], keyframe: bool) -> Vec<u8> {
    let mut tag = vec![video_header(keyframe), 1, 0, 0, 0];
    for nal in nals {
        if matches!(nal_type(nal), NAL_SPS | NAL_PPS | NAL_AUD) || nal.is_empty() {
            continue;
        }
        tag.extend((nal.len() as u32).to_be_bytes());
        tag.extend(nal);
    }
    tag
}

/// 音声タグの先頭1バイト。AACでは形式上 44kHz・16bit・ステレオを指定する決まり
/// (実際のサンプルレート等はシーケンスヘッダで伝える)。
const AUDIO_HEADER: u8 = ((AUDIO_CODEC_ID as u8) << 4) | (3 << 2) | (1 << 1) | 1;

/// AACのシーケンスヘッダ(AudioSpecificConfig)。AAC-LC・48kHz・ステレオ。
pub fn aac_sequence_header() -> Vec<u8> {
    const AAC_LC: u16 = 2;
    let frequency_index: u16 = match SAMPLE_RATE {
        44_100 => 4,
        _ => 3, // 48kHz
    };
    let config = (AAC_LC << 11) | (frequency_index << 7) | (CHANNELS << 3);
    let [a, b] = config.to_be_bytes();
    vec![AUDIO_HEADER, 0, a, b]
}

/// AACの1フレーム。
pub fn aac_frame(data: &[u8]) -> Vec<u8> {
    let mut tag = vec![AUDIO_HEADER, 1];
    tag.extend(data);
    tag
}

#[cfg(test)]
mod tests {
    use super::*;

    const SPS: [u8; 4] = [0x67, 0x42, 0xc0, 0x28];
    const PPS: [u8; 2] = [0x68, 0xce];

    #[test]
    fn finds_parameter_sets() {
        let nals = vec![SPS.to_vec(), PPS.to_vec(), vec![0x65, 0x88]];
        assert_eq!(parameter_sets(&nals), Some((&SPS[..], &PPS[..])));
        assert_eq!(parameter_sets(&nals[2..]), None);
    }

    #[test]
    fn builds_avc_sequence_header() {
        let tag = avc_sequence_header(&SPS, &PPS);
        assert_eq!(
            tag,
            [
                0x17, 0, 0, 0, 0, // keyframe + AVC, sequence header, cts
                1, 0x42, 0xc0, 0x28, 0xff, 0xe1, // record header
                0, 4, 0x67, 0x42, 0xc0, 0x28, // SPS
                1, 0, 2, 0x68, 0xce, // PPS
            ]
        );
    }

    #[test]
    fn avc_frame_prefixes_lengths_and_skips_parameter_sets() {
        let nals = vec![SPS.to_vec(), PPS.to_vec(), vec![0x65, 0x88, 0x84]];
        let tag = avc_frame(&nals, true);
        assert_eq!(tag, [0x17, 1, 0, 0, 0, 0, 0, 0, 3, 0x65, 0x88, 0x84]);
        assert_eq!(avc_frame(&[vec![0x41, 0x9a]], false)[0], 0x27);
    }

    #[test]
    fn aac_sequence_header_describes_48khz_stereo_lc() {
        assert_eq!(aac_sequence_header(), [0xaf, 0, 0x11, 0x90]);
        assert_eq!(aac_frame(&[1, 2]), [0xaf, 1, 1, 2]);
    }
}
