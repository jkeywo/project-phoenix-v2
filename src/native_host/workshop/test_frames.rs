//! Bounded presentation-only records on the disposable child's inherited pipe.
//! Frames never enter a crew transport, simulation snapshot or operator profile.
use crate::delivery::serve::HostedDocuments;
use std::io::{Read, Write};

pub const PREFIX: &str = "PHOENIX_WORKSHOP_FRAME ";
pub const WIDTH: u32 = 960;
pub const HEIGHT: u32 = 540;
pub const MAX_BYTES: usize = 4 * 1024 * 1024;

/// Owns one process generation's private HTTP resource. The reader gets a
/// publisher, not ownership: dropping the process retires the route after its
/// reader has joined, so a late frame cannot publish it again.
pub struct FrameRoute {
    documents: HostedDocuments,
    path: String,
    pub url: String,
    pub presentation_url: String,
    presentation_path: String,
}
impl FrameRoute {
    #[allow(clippy::disallowed_methods)] // Private capability, never simulation identity.
    pub fn new(documents: HostedDocuments, origin: &str) -> Self {
        let path = format!("/workshop-test-frame/{}/view.png", uuid::Uuid::new_v4());
        let presentation_path = path.replace("view.png", "presentation.json");
        Self {
            documents,
            url: format!("{}{path}", origin.trim_end_matches('/')),
            presentation_url: format!("{}{presentation_path}", origin.trim_end_matches('/')),
            path,
            presentation_path,
        }
    }
    pub fn presentation_publisher(&self) -> impl Fn(Vec<u8>) + Send + 'static {
        let documents = self.documents.clone();
        let path = self.presentation_path.clone();
        move |bytes| documents.publish_bytes(path.clone(), bytes, "application/json", false)
    }
    pub fn publisher(&self) -> impl Fn(Vec<u8>) + Send + 'static {
        let documents = self.documents.clone();
        let path = self.path.clone();
        move |bytes| documents.publish_bytes(path.clone(), bytes, "image/png", false)
    }
}
impl Drop for FrameRoute {
    fn drop(&mut self) {
        self.documents.withdraw(&self.path);
        self.documents.withdraw(&self.presentation_path);
    }
}

/// The PNG IHDR is checked before publication. The bounded body is read exactly;
/// a truncated frame closes this child's pipe instead of interpreting pixels as
/// subsequent control acknowledgements.
pub fn read_frame(header: &str, reader: &mut impl Read) -> Result<Vec<u8>, String> {
    let fields: Vec<_> = header
        .strip_prefix(PREFIX)
        .ok_or("Invalid Test frame prefix")?
        .split_whitespace()
        .collect();
    if fields.len() != 3 {
        return Err("Invalid Test frame header".into());
    }
    let width: u32 = fields[0].parse().map_err(|_| "Invalid Test frame width")?;
    let height: u32 = fields[1].parse().map_err(|_| "Invalid Test frame height")?;
    let length: usize = fields[2].parse().map_err(|_| "Invalid Test frame length")?;
    if width != WIDTH || height != HEIGHT || !(33..=MAX_BYTES).contains(&length) {
        return Err("Test frame exceeds its presentation bounds".into());
    }
    let mut bytes = vec![0; length];
    reader
        .read_exact(&mut bytes)
        .map_err(|error| error.to_string())?;
    if &bytes[..8] != b"\x89PNG\r\n\x1a\n"
        || &bytes[12..16] != b"IHDR"
        || bytes[16..20] != width.to_be_bytes()
        || bytes[20..24] != height.to_be_bytes()
    {
        return Err("Invalid Test frame image".into());
    }
    Ok(bytes)
}

pub fn write_frame(output: &mut impl Write, bytes: &[u8]) -> std::io::Result<()> {
    if !(33..=MAX_BYTES).contains(&bytes.len()) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "Invalid Test frame size",
        ));
    }
    writeln!(output, "{PREFIX}{WIDTH} {HEIGHT} {}", bytes.len())?;
    output.write_all(bytes)?;
    output.flush()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, Cursor};
    fn image() -> Vec<u8> {
        let mut bytes = vec![0; 33];
        bytes[..8].copy_from_slice(b"\x89PNG\r\n\x1a\n");
        bytes[12..16].copy_from_slice(b"IHDR");
        bytes[16..20].copy_from_slice(&WIDTH.to_be_bytes());
        bytes[20..24].copy_from_slice(&HEIGHT.to_be_bytes());
        bytes
    }
    #[test]
    fn frames_are_bounded_and_leave_the_following_status_record_intact() {
        let pixels = image();
        let mut wire = Vec::new();
        write_frame(&mut wire, &pixels).unwrap();
        wire.extend_from_slice(b"PHOENIX_WORKSHOP_TEST next\n");
        let mut input = Cursor::new(wire);
        let mut header = String::new();
        input.read_line(&mut header).unwrap();
        assert_eq!(read_frame(&header, &mut input).unwrap(), pixels);
        let mut status = String::new();
        input.read_line(&mut status).unwrap();
        assert_eq!(status, "PHOENIX_WORKSHOP_TEST next\n");
        for header in [
            format!("{PREFIX}{WIDTH} {HEIGHT} {}", MAX_BYTES + 1),
            format!("{PREFIX}0 {HEIGHT} 33"),
            format!("{PREFIX}{WIDTH} {HEIGHT} -1"),
            format!("{PREFIX}{WIDTH} {HEIGHT} 33 extra"),
        ] {
            assert!(read_frame(&header, &mut Cursor::new(&pixels)).is_err());
        }
        assert!(read_frame(
            &format!("{PREFIX}{WIDTH} {HEIGHT} 33"),
            &mut Cursor::new(&pixels[..32])
        )
        .is_err());
        assert!(read_frame(
            &format!("{PREFIX}{WIDTH} {HEIGHT} 33"),
            &mut Cursor::new(vec![0; 33])
        )
        .is_err());
    }
    #[test]
    fn presentation_round_trip_refuses_unknown_authority_channels_and_large_records() {
        use crate::workshop::test_protocol::TestPresentation;
        let mut payload = TestPresentation {
            sequence: 3,
            role_presets: "[]".into(),
            ..Default::default()
        };
        payload
            .channels
            .insert("gm_entity".into(), "{\"entities\":[]}".into());
        let json = crate::core::codec::encode_workshop_test_presentation(&payload).unwrap();
        let mut wire = Vec::new();
        write_presentation(&mut wire, &json).unwrap();
        let mut input = Cursor::new(wire);
        let mut header = String::new();
        input.read_line(&mut header).unwrap();
        let bytes = read_presentation(&header, &mut input).unwrap();
        assert_eq!(
            crate::core::codec::decode_workshop_test_presentation(&bytes).unwrap(),
            payload
        );
        payload
            .channels
            .insert("execute_script".into(), "bad".into());
        let json = crate::core::codec::encode_workshop_test_presentation(&payload).unwrap();
        assert!(read_presentation(
            &format!("{PRESENTATION_PREFIX}{}", json.len()),
            &mut Cursor::new(json.as_bytes())
        )
        .is_err());
        assert!(read_presentation(
            &format!("{PRESENTATION_PREFIX}{}", MAX_BYTES + 1),
            &mut Cursor::new([])
        )
        .is_err());
        assert!(read_presentation(
            &format!("{PRESENTATION_PREFIX}20"),
            &mut Cursor::new(b"short")
        )
        .is_err());
    }
    #[test]
    fn latest_frame_replaces_one_resource_and_drop_retires_the_generation() {
        let documents = HostedDocuments::default();
        let first = FrameRoute::new(documents.clone(), "http://127.0.0.1:7");
        let publish = first.publisher();
        publish(vec![1]);
        publish(vec![2]);
        assert_eq!(documents.len(), 1);
        assert_eq!(documents.resource(&first.path).unwrap().body.as_ref(), &[2]);
        assert!(!documents.resource(&first.path).unwrap().immutable);
        let second = FrameRoute::new(documents.clone(), "http://127.0.0.1:7");
        assert_ne!(first.url, second.url);
        drop(publish);
        drop(first);
        assert_eq!(documents.len(), 0);
    }
}

pub const PRESENTATION_PREFIX: &str = "PHOENIX_WORKSHOP_PRESENTATION ";
pub const CHANNELS: &[&str] = &[
    "hud",
    "gm_entity",
    "gm_activity",
    "gm_station",
    "gm_session",
    "gm_mission",
    "gm_spawn",
    "gm_comms",
    "gm_attention",
    "gm_health",
    "gm_workload",
];

pub fn read_presentation(header: &str, reader: &mut impl Read) -> Result<Vec<u8>, String> {
    let length: usize = header
        .strip_prefix(PRESENTATION_PREFIX)
        .ok_or("Invalid presentation prefix")?
        .trim()
        .parse()
        .map_err(|_| "Invalid presentation length")?;
    if !(1..=MAX_BYTES).contains(&length) {
        return Err("Test presentation exceeds its bounds".into());
    }
    let mut bytes = vec![0; length];
    reader.read_exact(&mut bytes).map_err(|e| e.to_string())?;
    let payload = crate::core::codec::decode_workshop_test_presentation(&bytes)?;
    if payload
        .channels
        .keys()
        .any(|name| !CHANNELS.contains(&name.as_str()))
    {
        return Err("Invalid Test presentation channel".into());
    }
    Ok(bytes)
}
pub fn write_presentation(output: &mut impl Write, json: &str) -> std::io::Result<()> {
    if json.len() > MAX_BYTES {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "Test presentation exceeds its bounds",
        ));
    }
    writeln!(output, "{PRESENTATION_PREFIX}{}", json.len())?;
    output.write_all(json.as_bytes())?;
    output.flush()
}

/// Presentation failure reaches Test status and causes the parent to retire the
/// child. Rendering errors never leave a document labelled as a healthy run.
#[derive(bevy::prelude::Resource, Clone, Default)]
pub struct PresentationFailure(std::sync::Arc<std::sync::Mutex<Option<String>>>);
impl PresentationFailure {
    pub fn fail(&self, error: String) {
        *self.0.lock().unwrap_or_else(|e| e.into_inner()) = Some(error);
    }
    pub fn error(&self) -> Option<String> {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
}
