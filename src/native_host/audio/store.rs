//! Hardware routing uses bridge-media vocabulary; mix is an endpoint preference.
use super::super::{
    bridge_media::{validate_media, MediaSurfaceEntry, ValidatedMedia},
    bridge_profile::BridgeProfile,
    layout_store::write_atomically,
};
use std::path::PathBuf;

#[derive(Clone, Debug)]
pub struct HardwareStore {
    dir: PathBuf,
}
impl HardwareStore {
    pub fn user() -> Option<Self> {
        Some(Self::at(
            directories::BaseDirs::new()?
                .data_dir()
                .join("ProjectPhoenix"),
        ))
    }
    pub fn at(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }
    pub fn path(&self) -> PathBuf {
        self.dir.join("bridge-media.toml")
    }
    pub fn load(&self) -> Result<BridgeProfile, String> {
        match std::fs::read_to_string(self.path()) {
            Ok(text) => {
                let profile = BridgeProfile::from_toml(&text).map_err(|e| e.to_string())?;
                validate_media(&profile.media).map_err(|e| e.to_string())?;
                Ok(profile)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(BridgeProfile::empty()),
            Err(e) => Err(e.to_string()),
        }
    }
    pub fn save(&self, profile: &BridgeProfile) -> Result<(), String> {
        validate_media(&profile.media).map_err(|e| e.to_string())?;
        std::fs::create_dir_all(&self.dir).map_err(|e| e.to_string())?;
        write_atomically(&self.path(), &profile.to_toml().map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())
    }
}
pub fn from_validated(media: &ValidatedMedia) -> BridgeProfile {
    let mut profile = BridgeProfile::empty();
    profile.media = media
        .surfaces
        .iter()
        .map(|surface| MediaSurfaceEntry {
            surface: surface.surface.clone(),
            camera: surface.camera.as_ref().map(ToString::to_string),
            microphones: surface
                .microphones
                .iter()
                .map(ToString::to_string)
                .collect(),
            outputs: surface.outputs.iter().map(ToString::to_string).collect(),
            allow_shared: {
                let mut ids: Vec<_> = surface.allow_shared.iter().cloned().collect();
                ids.sort();
                ids
            },
        })
        .collect();
    profile
}
/// Only an explicitly authored surface replaces its remembered assignment.
/// Validate the merged result at the consumer: two separately valid profiles
/// can otherwise introduce unconsented sharing when combined.
pub fn merge_media(saved: &BridgeProfile, authored: Option<&ValidatedMedia>) -> BridgeProfile {
    let mut merged = saved.clone();
    if let Some(authored) = authored {
        for entry in from_validated(authored).media {
            if let Some(remembered) = merged
                .media
                .iter_mut()
                .find(|old| old.surface == entry.surface)
            {
                *remembered = entry;
            } else {
                merged.media.push(entry);
            }
        }
    }
    merged
}
/// Empty viewscreen output means the explicitly labelled system-default route.
/// Explicit multi-output profiles are refused by A2 rather than silently picking
/// one or playing the room over every private assignment.
pub fn room_output(profile: &BridgeProfile) -> Result<Option<String>, String> {
    validate_media(&profile.media).map_err(|e| e.to_string())?;
    let outputs = profile
        .media
        .iter()
        .find(|entry| entry.surface == "viewscreen")
        .map(|entry| entry.outputs.as_slice())
        .unwrap_or_default();
    match outputs {
        [] => Ok(None),
        [one] => Ok(Some(one.clone())),
        _ => {
            Err("The Viewscreen needs one room output; select one output in Audio settings".into())
        }
    }
}
pub fn select_room(
    profile: &BridgeProfile,
    output: Option<String>,
) -> Result<BridgeProfile, String> {
    let mut next = profile.clone();
    if !next.media.iter().any(|entry| entry.surface == "viewscreen") {
        next.media.push(MediaSurfaceEntry {
            surface: "viewscreen".into(),
            camera: None,
            microphones: Vec::new(),
            outputs: Vec::new(),
            allow_shared: Vec::new(),
        });
    }
    next.media
        .iter_mut()
        .find(|entry| entry.surface == "viewscreen")
        .unwrap()
        .outputs = output.into_iter().collect();
    validate_media(&next.media).map_err(|e| e.to_string())?;
    Ok(next)
}
