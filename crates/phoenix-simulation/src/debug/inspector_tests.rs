use super::*;

fn ent(name: &str, x: f32, z: f32) -> InspectorEntityInput {
    InspectorEntityInput {
        name: name.to_string(),
        tags: vec![],
        x,
        z,
        faction: None,
        hull_current: None,
        hull_max: None,
        comms_range: None,
        ai_target: None,
    }
}

#[test]
fn no_player_still_lists_entities_versioned() {
    let payload = project_inspector(None, vec![ent("scout", 3.0, 4.0)]);
    assert_eq!(payload.schema_version, DEBUG_SCHEMA_VERSION);
    assert!(payload.player.is_none());
    assert_eq!(payload.entities.len(), 1);
    // Distance from the origin fallback: sqrt(3^2 + 4^2) = 5.
    assert!((payload.entities[0].distance - 5.0).abs() < 1e-4);
}

#[test]
fn distance_is_measured_from_the_player_and_sorts_the_list() {
    let player = InspectorPlayerInput {
        x: 10.0,
        z: 0.0,
        hull: vec![InspectorHullEntry {
            system: "core".into(),
            current: 50.0,
            max: 100.0,
        }],
        shields: vec![InspectorShieldFacing {
            label: "Fore".into(),
            hp: 20,
            max_hp: 40,
            offline: false,
            focused: true,
        }],
    };
    // "far" is 20u away, "near" is 5u away — expect near first after sorting.
    let payload = project_inspector(
        Some(player),
        vec![ent("far", 30.0, 0.0), ent("near", 15.0, 0.0)],
    );
    let p = payload.player.expect("player present");
    assert_eq!(p.hull.len(), 1);
    assert_eq!(p.shields[0].label, "Fore");
    assert!(p.shields[0].focused);
    assert_eq!(payload.entities.len(), 2);
    assert_eq!(payload.entities[0].name, "near");
    assert!((payload.entities[0].distance - 5.0).abs() < 1e-4);
    assert_eq!(payload.entities[1].name, "far");
    assert!((payload.entities[1].distance - 20.0).abs() < 1e-4);
}

#[test]
fn comms_range_derives_hailable_and_in_range() {
    let mut in_range = ent("friendly", 3.0, 0.0);
    in_range.comms_range = Some(10.0); // 3u away, within 10u range
    let mut out_of_range = ent("distant", 50.0, 0.0);
    out_of_range.comms_range = Some(10.0); // 50u away, outside range
    let payload = project_inspector(None, vec![in_range, out_of_range]);
    // Sorted by distance: friendly (3u) first.
    let friendly = &payload.entities[0];
    assert_eq!(friendly.name, "friendly");
    assert_eq!(friendly.comms_hailable, Some(true));
    assert_eq!(friendly.comms_in_range, Some(true));
    let distant = &payload.entities[1];
    assert_eq!(distant.comms_hailable, Some(true));
    assert_eq!(distant.comms_in_range, Some(false));
}

#[test]
fn no_comms_component_leaves_comms_fields_absent() {
    let payload = project_inspector(None, vec![ent("rock-adjacent", 1.0, 1.0)]);
    let e = &payload.entities[0];
    assert_eq!(e.comms_hailable, None);
    assert_eq!(e.comms_in_range, None);
    assert_eq!(e.comms_range, None);
}
