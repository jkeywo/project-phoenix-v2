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
#[path = "test_frames_tests.rs"]
mod tests;

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
    let payload: crate::workshop::test_protocol::TestPresentation =
        crate::core::codec::from_json_bytes(&bytes).map_err(|e| e.to_string())?;
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
