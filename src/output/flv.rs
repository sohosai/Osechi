//! RTMPで送る映像・音声メッセージの中身(FLVのタグ本体)を組み立てる。
//!
//! 映像は H.264(AVC)、音声は AAC。どちらも最初に「シーケンスヘッダ」(デコーダの設定)を送り、
//! その後にフレームを送る。

use crate::mixer::{CHANNELS, SAMPLE_RATE};

/// FLVの映像コーデックID(AVC)。onMetaDataでも使う。
pub const VIDEO_CODEC_ID: u32 = 7;
/// FLVの音声コーデックID(AAC)。onMetaDataでも使う。
pub const AUDIO_CODEC_ID: u32 = 10;

/// 映像タグの先頭5バイト: フレーム種別 + コーデックID、パケット種別、composition time(0)。
fn video_header(keyframe: bool, sequence_header: bool) -> [u8; 5] {
    let frame_type = if keyframe { 1 } else { 2 };
    let packet_type = if sequence_header { 0 } else { 1 };
    [
        (frame_type << 4) | VIDEO_CODEC_ID as u8,
        packet_type,
        0,
        0,
        0,
    ]
}

/// AVCのシーケンスヘッダ。`config` は [`super::h264::decoder_config`] の結果。
pub fn avc_sequence_header(config: &[u8]) -> Vec<u8> {
    [&video_header(true, true)[..], config].concat()
}

/// AVCの1フレーム。`frame` は [`super::h264::length_prefixed`] の結果。
pub fn avc_frame(frame: &[u8], keyframe: bool) -> Vec<u8> {
    [&video_header(keyframe, false)[..], frame].concat()
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
    [&[AUDIO_HEADER, 1][..], data].concat()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn video_tags_carry_frame_and_packet_types() {
        assert_eq!(avc_sequence_header(&[1, 2]), [0x17, 0, 0, 0, 0, 1, 2]);
        assert_eq!(avc_frame(&[9], true), [0x17, 1, 0, 0, 0, 9]);
        assert_eq!(avc_frame(&[9], false)[0], 0x27);
    }

    #[test]
    fn aac_sequence_header_describes_48khz_stereo_lc() {
        assert_eq!(aac_sequence_header(), [0xaf, 0, 0x11, 0x90]);
        assert_eq!(aac_frame(&[1, 2]), [0xaf, 1, 1, 2]);
    }
}
