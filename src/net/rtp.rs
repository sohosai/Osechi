//! RTP(RFC 3550)。

use crate::error::Error;

/// RTPパケット。ペイロードは受信バッファを借用する。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Packet<'a> {
    pub payload_type: u8,
    pub sequence: u16,
    pub timestamp: u32,
    /// CSRC・拡張ヘッダ・パディングを除いたペイロード。
    pub payload: &'a [u8],
}

impl<'a> TryFrom<&'a [u8]> for Packet<'a> {
    type Error = Error;

    fn try_from(packet: &'a [u8]) -> Result<Self, Error> {
        const FIXED_HEADER_LEN: usize = 12;
        let invalid = |reason: &str| Error::new(format!("invalid RTP packet: {reason}"));

        if packet.len() < FIXED_HEADER_LEN {
            return Err(invalid("too short"));
        }
        let b0 = packet[0];
        if b0 >> 6 != 2 {
            return Err(invalid("version is not 2"));
        }
        let has_padding = b0 & 0b0010_0000 != 0;
        let has_extension = b0 & 0b0001_0000 != 0;
        let csrc_count = (b0 & 0b0000_1111) as usize;

        let mut start = FIXED_HEADER_LEN + csrc_count * 4;
        if has_extension {
            // 拡張ヘッダ: profile(2バイト) + 長さ(2バイト, 32bitワード単位)
            let len_bytes = packet
                .get(start + 2..start + 4)
                .ok_or_else(|| invalid("truncated extension header"))?;
            let words = u16::from_be_bytes([len_bytes[0], len_bytes[1]]) as usize;
            start += 4 + words * 4;
        }

        let mut end = packet.len();
        if has_padding {
            // 末尾1バイトがパディング長(自分自身を含む)
            let pad_len = packet[end - 1] as usize;
            if pad_len == 0 || pad_len > end.saturating_sub(start) {
                return Err(invalid("bad padding"));
            }
            end -= pad_len;
        }

        Ok(Self {
            payload_type: packet[1] & 0b0111_1111,
            sequence: u16::from_be_bytes([packet[2], packet[3]]),
            timestamp: u32::from_be_bytes([packet[4], packet[5], packet[6], packet[7]]),
            payload: packet
                .get(start..end)
                .ok_or_else(|| invalid("header longer than packet"))?,
        })
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// テスト用に、最小ヘッダ(CSRC・拡張・パディング無し)のRTPパケットを組み立てる。
    pub(crate) fn packet(
        payload_type: u8,
        sequence: u16,
        timestamp: u32,
        payload: &[u8],
    ) -> Vec<u8> {
        let mut packet = vec![0b1000_0000, payload_type & 0b0111_1111];
        packet.extend_from_slice(&sequence.to_be_bytes());
        packet.extend_from_slice(&timestamp.to_be_bytes());
        packet.extend_from_slice(&0xAABB_CCDDu32.to_be_bytes()); // SSRC
        packet.extend_from_slice(payload);
        packet
    }

    #[test]
    fn parses_minimal_header() {
        let bytes = packet(97, 42, 1000, &[1, 2, 3, 4]);
        let parsed = Packet::try_from(bytes.as_slice()).expect("valid packet");
        assert_eq!(parsed.payload_type, 97);
        assert_eq!(parsed.sequence, 42);
        assert_eq!(parsed.timestamp, 1000);
        assert_eq!(parsed.payload, &[1, 2, 3, 4]);
    }

    #[test]
    fn ignores_marker_bit() {
        let mut bytes = packet(96, 1, 1, &[0]);
        bytes[1] |= 0b1000_0000;
        assert_eq!(Packet::try_from(bytes.as_slice()).unwrap().payload_type, 96);
    }

    #[test]
    fn rejects_too_short_packet() {
        assert!(Packet::try_from([0u8; 4].as_slice()).is_err());
    }

    #[test]
    fn rejects_wrong_version() {
        let mut bytes = packet(97, 1, 1, &[0, 0]);
        bytes[0] = 0b0100_0000; // version = 1
        assert!(Packet::try_from(bytes.as_slice()).is_err());
    }

    #[test]
    fn skips_csrc_list() {
        let mut bytes = packet(97, 1, 0, &[]);
        bytes[0] |= 2; // CC=2
        bytes.extend_from_slice(&[0; 8]); // CSRC x2
        bytes.extend_from_slice(&[9, 9, 9]);
        assert_eq!(
            Packet::try_from(bytes.as_slice()).unwrap().payload,
            &[9, 9, 9]
        );
    }

    #[test]
    fn skips_extension_header() {
        let mut bytes = packet(97, 1, 0, &[]);
        bytes[0] |= 0b0001_0000;
        bytes.extend_from_slice(&[0xBE, 0xDE, 0, 1]); // profile, 長さ=1ワード
        bytes.extend_from_slice(&[0; 4]);
        bytes.extend_from_slice(&[7, 7]);
        assert_eq!(Packet::try_from(bytes.as_slice()).unwrap().payload, &[7, 7]);
    }

    #[test]
    fn strips_padding() {
        // 実ペイロード [1,2,3,4] に長さ1のパディングを付ける
        let mut bytes = packet(97, 1, 1, &[1, 2, 3, 4, 1]);
        bytes[0] |= 0b0010_0000;
        assert_eq!(
            Packet::try_from(bytes.as_slice()).unwrap().payload,
            &[1, 2, 3, 4]
        );
    }
}
