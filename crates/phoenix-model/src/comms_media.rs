//! Closed human media vocabulary. Devices and packets are never saved game state.
use serde::{Deserialize, Serialize};

pub const MAX_MEDIA_JSON_BYTES: usize = 20 * 1024;
pub const PCM_SAMPLES: usize = 1280;
pub const PCM_BYTES: usize = PCM_SAMPLES * 2;
pub const MAX_JPEG_BYTES: usize = 12 * 1024;
pub const MAX_JPEG_WIDTH: u16 = 320;
pub const MAX_JPEG_HEIGHT: u16 = 180;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MediaMode {
    Audio,
    AudioVideo,
}
impl MediaMode {
    pub fn supports(self, requested: Self) -> bool {
        self == Self::AudioVideo || requested == Self::Audio
    }
    pub fn permits(self, kind: MediaPacketKind) -> bool {
        kind == MediaPacketKind::Pcm16Mono16k || self == Self::AudioVideo
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MediaPacketKind {
    Pcm16Mono16k,
    Jpeg,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MediaPacket {
    pub call_origin_id: String,
    pub lease_generation: u32,
    pub sequence: u32,
    pub capture_ms: u32,
    pub kind: MediaPacketKind,
    /// Canonical standard base64, not arbitrary MIME, URLs or HTML.
    pub data: String,
}
impl MediaPacket {
    /// Cheap closed-shape checks before allocating a decoded copy. Full JSON
    /// bytes are bounded by the actual wire codec before general ingress.
    pub fn validate(&self) -> Result<(), &'static str> {
        if !crate::comms_channel::identity(&self.call_origin_id)
            || self.lease_generation == 0
            || self.sequence == 0
        {
            return Err("media packet lacks its actual call/lease/sequence");
        }
        let decoded = base64_length(&self.data).ok_or("invalid bounded canonical media base64")?;
        match self.kind {
            MediaPacketKind::Pcm16Mono16k if decoded != PCM_BYTES => {
                Err("media PCM requires exactly 1280 mono little-endian samples")
            }
            MediaPacketKind::Jpeg if !(12..=MAX_JPEG_BYTES).contains(&decoded) => {
                Err("media JPEG exceeds its independent still byte bound")
            }
            _ => Ok(()),
        }
    }
    pub fn validate_decoded(&self, bytes: &[u8]) -> Result<(), &'static str> {
        self.validate()?;
        if Some(bytes.len()) != base64_length(&self.data) {
            return Err("decoded media disagrees with its encoded length");
        }
        if self.kind == MediaPacketKind::Jpeg && jpeg_dimensions(bytes).is_none() {
            return Err("media JPEG has unsupported or excessive decoded dimensions");
        }
        Ok(())
    }
}

fn base64_length(value: &str) -> Option<usize> {
    if value.is_empty() || value.len() > 16384 || !value.len().is_multiple_of(4) {
        return None;
    }
    let bytes = value.as_bytes();
    let padding = bytes.iter().rev().take_while(|byte| **byte == b'=').count();
    if padding > 2 {
        return None;
    }
    let payload = &bytes[..bytes.len() - padding];
    let sextet = |byte: u8| match byte {
        b'A'..=b'Z' => Some(byte - b'A'),
        b'a'..=b'z' => Some(byte - b'a' + 26),
        b'0'..=b'9' => Some(byte - b'0' + 52),
        b'+' => Some(62),
        b'/' => Some(63),
        _ => None,
    };
    if !payload.iter().all(|byte| sextet(*byte).is_some()) {
        return None;
    }
    let last = sextet(*payload.last()?)?;
    if (padding == 2 && last & 15 != 0) || (padding == 1 && last & 3 != 0) {
        return None;
    }
    Some(value.len() / 4 * 3 - padding)
}

/// Inspect the closed baseline JPEG surface before any image decoder allocation.
pub fn jpeg_dimensions(bytes: &[u8]) -> Option<(u16, u16)> {
    if !(12..=MAX_JPEG_BYTES).contains(&bytes.len())
        || bytes[..2] != [255, 216]
        || bytes[bytes.len() - 2..] != [255, 217]
    {
        return None;
    }
    let mut offset = 2;
    let mut dimensions = None;
    while offset + 4 <= bytes.len() {
        if bytes[offset] != 255 {
            return None;
        }
        offset += 1;
        while bytes.get(offset) == Some(&255) {
            offset += 1;
        }
        let marker = *bytes.get(offset)?;
        offset += 1;
        let length = u16::from_be_bytes([*bytes.get(offset)?, *bytes.get(offset + 1)?]) as usize;
        if length < 2 || offset + length > bytes.len() || marker == 0xdc {
            return None;
        }
        if (0xc0..=0xcf).contains(&marker) && ![0xc4, 0xc8, 0xcc].contains(&marker) {
            if marker != 0xc0 || dimensions.is_some() || length < 8 || bytes[offset + 2] != 8 {
                return None;
            }
            let height = u16::from_be_bytes([bytes[offset + 3], bytes[offset + 4]]);
            let width = u16::from_be_bytes([bytes[offset + 5], bytes[offset + 6]]);
            if width == 0 || height == 0 || width > MAX_JPEG_WIDTH || height > MAX_JPEG_HEIGHT {
                return None;
            }
            dimensions = Some((width, height));
        }
        offset += length;
        if marker == 0xda {
            dimensions?;
            while offset < bytes.len() - 2 {
                if bytes[offset] != 255 {
                    offset += 1;
                    continue;
                }
                offset += 1;
                let marker = *bytes.get(offset)?;
                offset += 1;
                if marker != 0 && !(0xd0..=0xd7).contains(&marker) {
                    return None;
                }
            }
            return (offset == bytes.len() - 2).then_some(dimensions).flatten();
        }
    }
    (offset == bytes.len() - 2).then_some(dimensions).flatten()
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MediaStart {
    pub call_origin_id: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MediaStop {
    pub lease_generation: u32,
}
/// Private current-operator presentation only; absent generations never open capture.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MediaSessionView {
    pub call_origin_id: Option<String>,
    pub mode: Option<MediaMode>,
    pub eligible: bool,
    pub lease_generation: Option<u32>,
    pub peer_lease_generation: Option<u32>,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn canonical_base64_and_fixed_pcm_are_bounded_before_decode() {
        assert_eq!(base64_length("AA=="), Some(1));
        assert_eq!(base64_length("AAA="), Some(2));
        for value in ["", "AB==", "AAB=", "A===", "AAA", "A=AA"] {
            assert_eq!(base64_length(value), None, "{value}");
        }
        let mut packet = MediaPacket {
            call_origin_id: "actual-call".into(),
            lease_generation: 1,
            sequence: 1,
            capture_ms: 80,
            kind: MediaPacketKind::Pcm16Mono16k,
            data: format!("{}AA==", "A".repeat(3412)),
        };
        assert_eq!(packet.validate_decoded(&vec![0; PCM_BYTES]), Ok(()));
        packet.sequence = 0;
        assert!(packet.validate().is_err());
    }
    #[test]
    fn image_extent_is_checked_without_decoding() {
        let mut bytes = vec![
            255, 216, 255, 192, 0, 11, 8, 0, 180, 1, 64, 1, 1, 17, 0, 255, 217,
        ];
        assert_eq!(jpeg_dimensions(&bytes), Some((320, 180)));
        bytes[9] = 255;
        assert_eq!(jpeg_dimensions(&bytes), None);
    }
}
