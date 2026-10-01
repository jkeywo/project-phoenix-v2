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
mod tests {
    use super::*;
    use bridge_media::DeviceAvailability;

    fn catalogue(names: &[Option<&str>]) -> DeviceCatalogue<usize> {
        DeviceCatalogue::new(names.iter().enumerate().map(|(handle, name)| {
            (
                RawMediaDevice {
                    kind: MediaKind::Output,
                    name: name.map(String::from),
                    hardware_id: None,
                    default: false,
                    availability: DeviceAvailability::Available,
                },
                handle,
            )
        }))
    }

    #[test]
    fn resolves_handles_by_identity_after_enumeration_changes() {
        assert_eq!(
            catalogue(&[Some("A"), Some("B")])
                .resolve("output:A")
                .unwrap()
                .handle,
            0
        );
        assert_eq!(
            catalogue(&[Some("B"), Some("A")])
                .resolve("output:A")
                .unwrap()
                .handle,
            1
        );
    }

    #[test]
    fn refuses_missing_duplicate_and_unnamed_devices() {
        let devices = catalogue(&[Some("A"), Some("A"), None]);
        assert!(matches!(
            devices.resolve("output:missing"),
            Err(SelectionError::Missing(_))
        ));
        for entry in devices.entries() {
            assert!(matches!(
                devices.resolve(entry.device.identity.as_str()),
                Err(SelectionError::Ambiguous(_))
            ));
        }
    }

    #[test]
    fn preflights_all_assignments_and_keeps_authored_order() {
        let profile: BridgeProfile =
            toml::from_str("version=1\n[[media]]\nsurface='comms'\noutput=['output:B','output:A']")
                .unwrap();
        let devices = catalogue(&[Some("A"), Some("B")]);
        assert_eq!(
            devices
                .surface(&profile, "comms", MediaKind::Output)
                .unwrap()
                .iter()
                .map(|e| e.handle)
                .collect::<Vec<_>>(),
            [1, 0]
        );
        assert!(matches!(
            catalogue(&[Some("B")]).surface(&profile, "comms", MediaKind::Output),
            Err(SelectionError::Missing(_))
        ));
        assert!(matches!(
            devices.surface(&profile, "absent", MediaKind::Output),
            Err(SelectionError::UnknownSurface)
        ));
        assert!(matches!(
            devices.surface(&profile, "comms", MediaKind::Microphone),
            Err(SelectionError::Unassigned)
        ));
    }
}
