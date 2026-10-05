//! Content file JSON decoding, kept at one codec seam.
pub fn from_json_bytes<T: serde::de::DeserializeOwned>(
    bytes: &[u8],
) -> Result<T, serde_json::Error> {
    serde_json::from_slice(bytes)
}
