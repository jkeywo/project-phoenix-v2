use super::*;
#[test]
fn microphone_preflight_resolves_only_the_named_surface_and_all_members() {
    let profile: BridgeProfile = toml::from_str(
        r#"version = 1
[[media]]
surface = "comms"
microphone = ["mic:Headset", "mic:Desk"]
"#,
    )
    .unwrap();
    let catalogue = |names: &[&str]| {
        DeviceCatalogue::new(names.iter().enumerate().map(|(index, name)| {
            (
                RawMediaDevice {
                    kind: MediaKind::Microphone,
                    name: Some((*name).into()),
                    hardware_id: None,
                    default: false,
                    availability: DeviceAvailability::Available,
                },
                index,
            )
        }))
    };
    let devices = catalogue(&["Headset", "Desk"]);
    assert_eq!(
        selected_microphones(&profile, "comms", &devices)
            .unwrap()
            .iter()
            .map(|entry| entry.handle)
            .collect::<Vec<_>>(),
        vec![0, 1]
    );
    assert!(
        selected_microphones(&profile, "comms", &catalogue(&["Headset"]))
            .err()
            .unwrap()
            .contains("missing")
    );
    let duplicates = catalogue(&["Headset", "Headset"]);
    let duplicate_id = duplicates
        .entries()
        .next()
        .unwrap()
        .device
        .identity
        .to_string();
    let duplicate_profile: BridgeProfile = toml::from_str(&format!(
        "version=1\n[[media]]\nsurface='comms'\nmicrophone=['{duplicate_id}']"
    ))
    .unwrap();
    assert!(
        selected_microphones(&duplicate_profile, "comms", &duplicates)
            .err()
            .unwrap()
            .contains("unique")
    );
    assert!(selected_microphones(&profile, "viewscreen", &devices).is_err());
}

#[test]
fn metering_clamps_signal_without_retaining_or_inventing_invalid_levels() {
    assert_eq!(finite_level(-0.75), 0.75);
    assert_eq!(finite_level(2.0), 1.0);
    assert_eq!(finite_level(f32::NAN), 0.0);
    assert_eq!(finite_level(f32::INFINITY), 0.0);
}
