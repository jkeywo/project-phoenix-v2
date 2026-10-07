use super::*;

#[test]
fn inventory_is_complete_classified_and_occupancy_uses_public_effects_only() {
    let inventory = fields();
    let by_id: BTreeMap<_, _> = inventory
        .iter()
        .map(|field| (field.id.as_str(), field))
        .collect();
    for required in [
        "identity.uuid",
        "identity.kind",
        "transform.rotation.w",
        "shape.sphere.radius",
        "shape.box.yaw",
        "shape.torus.outer_radius",
        "effects.damage_zone.shield_pierce",
        "effects.slow_zone.yaw_rate_modifier",
        "effects.blocks_impulse.present",
        "effects.radar_dampening.range_modifier",
        "effects.comms_jammed.present",
        "effects.sensor_blind.present",
        "effects.nebula_fog.density",
        "presentation.radar.region_colour.b",
        "runtime.occupant_count",
    ] {
        assert!(
            by_id.contains_key(required),
            "missing descriptor {required}"
        );
    }
    assert!(inventory
        .iter()
        .all(|field| field.descriptor.live_mutability != LiveMutability::NamedAction));
    assert!(inventory
        .iter()
        .filter(|field| field.descriptor.live_mutability == LiveMutability::RecreateRequired)
        .all(|field| field.descriptor.validation == ["inspector.region.recreate_explanation"]));

    let effects = RegionEffectsSection(vec![
        RegionEffectKind::SlowZone {
            thrust_modifier: Some(-1.0),
            yaw_rate_modifier: None,
        },
        RegionEffectKind::RadarDampening { multiplier: -1.0 },
        RegionEffectKind::CommsJam,
    ]);
    let region_uuid = EntityUuid("region".into());
    let ship_uuid = EntityUuid("ship".into());
    let shape = RegionShapeSection(crate::regions::shape::RegionShape::Sphere { radius: 10.0 });
    let transform = Transform::from_xyz(1.0, 2.0, 3.0);
    let reading = reading(RegionReadingInputs {
        uuid: &region_uuid,
        name: None,
        display_name: Some("Storm front"),
        id: None,
        template: None,
        layer: None,
        tags: None,
        transform: &transform,
        shape: &shape,
        effects: Some(&effects),
        radar: None,
        occupants: vec![(&ship_uuid, "Scout".into())],
    });
    assert_eq!(reading.values["shape.sphere.radius"], "10");
    assert_eq!(reading.values["identity.display_name"], "Storm front");
    assert_eq!(reading.label, "Storm front");
    assert_eq!(reading.values["shape.box.yaw"], "not-applicable");
    assert_eq!(reading.values["effects.damage_zone.present"], "false");
    assert_eq!(reading.values["effects.slow_zone.present"], "true");
    assert_eq!(reading.occupants[0].entity_id, "ship");
    assert_eq!(
        reading.occupants[0].consequences[0].values["thrust_multiplier"],
        "0.5"
    );
    assert_eq!(
        reading.occupants[0].consequences[1].values["radar_range_multiplier"],
        "0.5"
    );
}
