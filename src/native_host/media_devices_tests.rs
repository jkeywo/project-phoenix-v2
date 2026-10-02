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
