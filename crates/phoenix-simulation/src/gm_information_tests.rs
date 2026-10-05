use super::*;
#[test]
fn spatial_cues_follow_the_effective_sensors_picture_without_inventing_delayed_audio() {
    use crate::core::messages::ViewMode;
    let mut state = ContactInformation::default();
    reports::set(
        &mut state.reports,
        "a",
        "b",
        Some(reports::ReportPolicy {
            delay_ticks: 4,
            position_step_mm: 0,
            hide_identity: false,
        }),
    );
    let mut overrides = Default::default();
    for view in [ViewMode::SensorsRadar, ViewMode::ScienceRadar] {
        assert!(suppresses_spatial_cue(&state, &overrides, "a", "b", &view));
        assert!(!suppresses_spatial_cue(
            &state, &overrides, "other", "b", &view
        ));
    }
    assert!(!suppresses_spatial_cue(
        &state,
        &overrides,
        "a",
        "b",
        &ViewMode::Camera(Default::default())
    ));
    state.reports.clear();
    assert!(!suppresses_spatial_cue(
        &state,
        &overrides,
        "a",
        "b",
        &ViewMode::SensorsRadar
    ));
    crate::gm_contact::set(
        &mut overrides,
        "a",
        "b",
        crate::gm_contact::ContactMode::Conceal,
    );
    assert!(suppresses_spatial_cue(
        &state,
        &overrides,
        "a",
        "b",
        &ViewMode::SensorsRadar
    ));
}
#[test]
fn ghost_changes_have_a_binary_roundtrip_and_strict_authored_shape() {
    let change = ContactInformationChange::SetGhost {
        id: "echo".into(),
        palette: "cargo".into(),
        position_mm: [1000, 0, -2000],
    };
    let codec = vellum_digest::ShareCodec::new("GM-INFORMATION-TEST-");
    let bytes = codec.encode(&change).unwrap();
    assert_eq!(
        codec.decode::<ContactInformationChange>(&bytes).unwrap(),
        change
    );
    let raw: crate::world::config::RawActionEntry = toml::from_str(r#"
            type = "set_contact_information"
            entity = "observer"
            contact_information = { set_ghost = { id = "echo", palette = "cargo", position_mm = [1000, 0, -2000] } }
        "#).unwrap();
    assert_eq!(
        crate::world::config::parse_action_entry(&raw).unwrap(),
        crate::world::config::TriggerAction::SetContactInformation {
            ship: "observer".into(),
            change
        }
    );
    assert!(toml::from_str::<ContactInformationChange>(
        r#"set_ghost = { id = "echo", palette = "cargo", position_mm = [1,2,3], physical = true }"#
    )
    .is_err());
}
