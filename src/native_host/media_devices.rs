//! Explicit endpoint resolution. Handles never leave their owner's thread.
use super::bridge_media::{self, DiscoveredMediaDevice, MediaKind, RawMediaDevice};
use super::bridge_profile::BridgeProfile;
use std::collections::HashMap;

pub(crate) struct Entry<H> {
    pub device: DiscoveredMediaDevice,
    pub handle: H,
    pub ambiguous: bool,
}

pub(crate) struct DeviceCatalogue<H> {
    entries: Vec<Entry<H>>,
}

#[derive(Debug, PartialEq)]
pub(crate) enum SelectionError {
    InvalidProfile(String),
    UnknownSurface,
    Unassigned,
    Missing(String),
    Ambiguous(String),
}

impl<H> DeviceCatalogue<H> {
    pub fn new(devices: impl IntoIterator<Item = (RawMediaDevice, H)>) -> Self {
        let (raws, handles): (Vec<_>, Vec<_>) = devices.into_iter().unzip();
        let mut counts = HashMap::new();
        for raw in &raws {
            *counts.entry((raw.kind, raw.name.clone())).or_insert(0usize) += 1;
        }
        let entries = bridge_media::identify_media(&raws)
            .into_iter()
            .zip(handles)
            .zip(&raws)
            .map(|((device, handle), raw)| Entry {
                ambiguous: raw.name.is_none() || counts[&(raw.kind, raw.name.clone())] > 1,
                device,
                handle,
            })
            .collect();
        Self { entries }
    }

    pub fn entries(&self) -> impl Iterator<Item = &Entry<H>> {
        self.entries.iter()
    }

    pub fn resolve(&self, id: &str) -> Result<&Entry<H>, SelectionError> {
        let entry = self
            .entries
            .iter()
            .find(|e| e.device.identity.as_str() == id)
            .ok_or_else(|| SelectionError::Missing(id.into()))?;
        if entry.ambiguous {
            return Err(SelectionError::Ambiguous(id.into()));
        }
        Ok(entry)
    }

    /// Resolve the entire authored list before any adapter performs hardware I/O.
    pub fn surface(
        &self,
        profile: &BridgeProfile,
        surface: &str,
        kind: MediaKind,
    ) -> Result<Vec<&Entry<H>>, SelectionError> {
        let validated = bridge_media::validate_media(&profile.media)
            .map_err(|e| SelectionError::InvalidProfile(e.to_string()))?;
        let assigned = validated
            .surfaces
            .iter()
            .find(|entry| entry.surface == surface)
            .ok_or(SelectionError::UnknownSurface)?;
        let ids = match kind {
            MediaKind::Microphone => &assigned.microphones,
            MediaKind::Output => &assigned.outputs,
            MediaKind::Camera => unreachable!("audio catalogue only"),
        };
        if ids.is_empty() {
            return Err(SelectionError::Unassigned);
        }
        ids.iter().map(|id| self.resolve(id.as_str())).collect()
    }
}

#[cfg(test)]
#[path = "media_devices_tests.rs"]
mod tests;
