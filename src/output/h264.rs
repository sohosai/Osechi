//! H.264 のビットストリームを、コンテナ(FLV・MOV)に入れる形に整える。
//!
//! どちらのコンテナも、SPS・PPS を「デコーダ設定」(AVCDecoderConfigurationRecord)として別に持ち、
//! フレームは NAL ユニットを4バイトの長さ付きで並べた形(AVCC)で入れる。

const NAL_SPS: u8 = 7;
const NAL_PPS: u8 = 8;
const NAL_AUD: u8 = 9;

/// NALユニットの種類。
fn nal_type(nal: &[u8]) -> u8 {
    nal.first().map_or(0, |byte| byte & 0x1f)
}

/// NALユニットの中から SPS と PPS を探す。
pub fn parameter_sets(nals: &[Vec<u8>]) -> Option<(&[u8], &[u8])> {
    let find = |kind| {
        nals.iter()
            .find(|nal| nal_type(nal) == kind)
            .map(Vec::as_slice)
    };
    Some((find(NAL_SPS)?, find(NAL_PPS)?))
}

/// デコーダ設定(AVCDecoderConfigurationRecord)。FLV のシーケンスヘッダと MOV の `avcC` の中身。
pub fn decoder_config(sps: &[u8], pps: &[u8]) -> Vec<u8> {
    let mut record = vec![
        1,      // configurationVersion
        sps[1], // AVCProfileIndication
        sps[2], // profile_compatibility
        sps[3], // AVCLevelIndication
        0xff,   // NALユニットの長さは4バイト
        0xe1,   // SPSの数: 1
    ];
    record.extend((sps.len() as u16).to_be_bytes());
    record.extend(sps);
    record.push(1); // PPSの数
    record.extend((pps.len() as u16).to_be_bytes());
    record.extend(pps);
    record
}

/// 1フレーム分の NAL ユニットを4バイトの長さ付きで並べる。
/// SPS・PPS はデコーダ設定で渡し、AUD は不要なので入れない。
pub fn length_prefixed(nals: &[Vec<u8>]) -> Vec<u8> {
    let mut frame = Vec::new();
    for nal in nals {
        if nal.is_empty() || matches!(nal_type(nal), NAL_SPS | NAL_PPS | NAL_AUD) {
            continue;
        }
        frame.extend((nal.len() as u32).to_be_bytes());
        frame.extend(nal);
    }
    frame
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;

    pub const SPS: [u8; 4] = [0x67, 0x42, 0xc0, 0x28];
    pub const PPS: [u8; 2] = [0x68, 0xce];

    #[test]
    fn finds_parameter_sets() {
        let nals = vec![SPS.to_vec(), PPS.to_vec(), vec![0x65, 0x88]];
        assert_eq!(parameter_sets(&nals), Some((&SPS[..], &PPS[..])));
        assert_eq!(parameter_sets(&nals[2..]), None);
    }

    #[test]
    fn builds_decoder_config() {
        assert_eq!(
            decoder_config(&SPS, &PPS),
            [
                1, 0x42, 0xc0, 0x28, 0xff, 0xe1, // record header
                0, 4, 0x67, 0x42, 0xc0, 0x28, // SPS
                1, 0, 2, 0x68, 0xce, // PPS
            ]
        );
    }

    #[test]
    fn prefixes_lengths_and_skips_parameter_sets() {
        let nals = vec![SPS.to_vec(), PPS.to_vec(), vec![0x65, 0x88, 0x84]];
        assert_eq!(length_prefixed(&nals), [0, 0, 0, 3, 0x65, 0x88, 0x84]);
    }
}
