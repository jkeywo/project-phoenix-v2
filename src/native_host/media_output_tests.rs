use super::*;
#[test]
fn selection_refuses_ambiguous_and_missing_members_before_any_playback() {
    let profile: BridgeProfile = toml::from_str(
        r#"
version = 1
[[media]]
surface = "comms"
output = ["output:Headset", "output:Speakers"]
"#,
    )
    .unwrap();
    let catalogue = |names: &[&str]| {
        DeviceCatalogue::new(names.iter().enumerate().map(|(index, name)| {
            (
                RawMediaDevice {
                    kind: MediaKind::Output,
                    name: Some((*name).into()),
                    hardware_id: None,
                    default: false,
                    availability: DeviceAvailability::Available,
                },
                index,
            )
        }))
    };
    let devices = catalogue(&["Headset", "Speakers"]);
    assert_eq!(
        selected_outputs(&profile, "comms", &devices)
            .unwrap()
            .iter()
            .map(|entry| entry.handle)
            .collect::<Vec<_>>(),
        vec![0, 1]
    );
    assert!(
        selected_outputs(&profile, "comms", &catalogue(&["Headset"]))
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
        "version=1\n[[media]]\nsurface='comms'\noutput=['{duplicate_id}']"
    ))
    .unwrap();
    assert!(selected_outputs(&duplicate_profile, "comms", &duplicates)
        .err()
        .unwrap()
        .contains("unique"));
    assert!(selected_outputs(&profile, "viewscreen", &devices).is_err());
}

#[test]
fn the_test_tone_is_quiet_faded_and_finite_with_a_silent_tail() {
    for rate in [8_000, 44_100, 48_000, 192_000] {
        assert_eq!(tone_sample(0, rate), 0.0);
        assert!((0..rate).all(|frame| tone_sample(frame, rate).abs() <= 0.040001));
        assert_eq!(tone_sample(rate, rate), 0.0);
        assert_eq!(tone_sample(u32::MAX, rate), 0.0);
    }
}
