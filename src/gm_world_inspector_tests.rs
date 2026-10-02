use super::*;
#[test]
fn projection_has_no_generic_mutation_and_flags_are_derived() {
    let world = crate::world::config::parse_world(
        "[global]\ntitle='Probe'\n[[deadline]]\nid='end'\ndue_secs=10\n",
    )
    .unwrap();
    let projection = projection(Some(&world), None, None, None, Some(false));
    assert!(projection
        .fields
        .iter()
        .any(|f| f.id == "global.sim_tick_hz"));
    assert!(projection
        .fields
        .iter()
        .any(|f| f.id == "global.description"));
    let flags = projection
        .fields
        .iter()
        .find(|f| f.id == "flag[].value")
        .unwrap();
    assert_eq!(flags.descriptor.live_mutability, LiveMutability::Derived);
    assert!(projection
        .fields
        .iter()
        .filter(|f| f.descriptor.live_mutability == LiveMutability::NamedAction)
        .all(|f| f.action_panel.is_some()));
}

#[test]
fn active_supporting_layer_keeps_its_path_provenance_and_derived_flags() {
    let world = crate::world::config::parse_world("[global]\ntitle='Probe'\n").unwrap();
    let layer_config = crate::world::config::parse_world(
        "[global]\ntitle='Reinforcements'\nsim_tick_hz=120\nai_tick_hz=20\nai_snapshot_hz=10\n",
    )
    .unwrap();
    let mut layer = crate::world::server::WorldRuntime {
        inspection_config: Some(Box::new(layer_config)),
        is_active: true,
        ..Default::default()
    };
    layer.flags.set_flag_value("wave", 3);
    let layers = crate::world::server::WorldLayerMap(std::collections::HashMap::from([(
        "assets/worlds/reinforcements.toml".to_owned(),
        layer,
    )]));

    let projection = projection(Some(&world), None, None, Some(&layers), None);
    let reading = projection
        .readings
        .get("assets/worlds/reinforcements.toml")
        .unwrap();
    assert_eq!(
        reading.origin_layer.as_deref(),
        Some("assets/worlds/reinforcements.toml")
    );
    assert_eq!(reading.values["flag[wave].value"], "3");
    assert_eq!(reading.values["global.title"], "Reinforcements");
    assert_eq!(reading.values["global.sim_tick_hz"], "120.0");
}
