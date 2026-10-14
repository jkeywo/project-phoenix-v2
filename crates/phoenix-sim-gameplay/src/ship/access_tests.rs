use super::*;
fn owner() -> InteriorAccess {
    let ship:ShipConfig=serde_json::from_value(serde_json::json!({"system":[{
        "id":"hatch","kind":"access-gate",
        "component":[{"kind":"access_boundary","id":"door","sealed_channel":"sealed","transition_secs":1.0}],
        "channel":[{"name":"sealed","kind":"boolean"}]
    }]})).unwrap();
    super::super::composition::validate_composition(&ship.systems[0]).unwrap();
    InteriorAccess::from_ship(&ship)
}
fn hatch() -> SystemId {
    SystemId("hatch".into())
}
#[test]
fn seal_waits_for_committed_crossings_and_ai_cannot_clear_request() {
    let mut access = owner();
    assert!(access.set(&hatch(), true, true));
    access.set(&hatch(), false, false);
    access.tick(10.0, &BTreeSet::from([hatch()]));
    assert_eq!(access.statuses()[0].phase, BoundaryPhase::Sealing);
    assert_eq!(access.statuses()[0].elapsed, 0.0);
    access.tick(0.5, &BTreeSet::new());
    let saved = access.statuses();
    let mut resumed = owner();
    resumed.restore(&saved).unwrap();
    resumed.tick(0.5, &BTreeSet::new());
    assert_eq!(resumed.statuses()[0].phase, BoundaryPhase::Sealed);
    resumed.set(&hatch(), false, true);
    resumed.tick(1.0, &BTreeSet::new());
    assert_eq!(resumed.statuses()[0].phase, BoundaryPhase::Open);
}
#[test]
fn invalid_boundary_restore_is_transactional() {
    let mut access = owner();
    let before = access.statuses();
    let mut malformed = before.clone();
    malformed[0].elapsed = f32::NAN;
    assert!(access.restore(&malformed).is_err());
    assert_eq!(access.statuses(), before);
    malformed = before.clone();
    malformed.push(before[0].clone());
    assert!(access.restore(&malformed).is_err());
    assert_eq!(access.statuses(), before);
}
#[test]
fn unavailable_boundary_retains_opening_progress_and_intent_until_recovery() {
    let mut access = owner();
    access.set(&hatch(), true, true);
    access.tick(1.0, &BTreeSet::new());
    access.set(&hatch(), false, true);
    access.tick(0.25, &BTreeSet::new());
    let before = access.statuses();
    assert_eq!(before[0].phase, BoundaryPhase::Opening);
    access.tick_available(10.0, &BTreeSet::new(), &BTreeSet::from([hatch()]));
    assert_eq!(access.statuses(), before);
    access.tick_available(0.75, &BTreeSet::new(), &BTreeSet::new());
    assert_eq!(access.statuses()[0].phase, BoundaryPhase::Open);
}
