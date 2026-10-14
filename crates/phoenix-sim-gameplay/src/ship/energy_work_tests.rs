use super::*;

fn power() -> (PowerSystem, crate::core::messages::SystemId) {
    let power = PowerSystem::default();
    let mut ship = power.ship_config().clone();
    let hardware = power.config().clone();
    ship.systems[0].kind = "helm_thrust".into();
    let id = ship.systems[0].id.clone();
    (PowerSystem::from_energy(hardware, ship).unwrap(), id)
}

#[test]
fn incoming_flow_cut_still_allows_charged_actuator_then_partial_and_no_work() {
    let (mut power, id) = power();
    let group = power.ship_config().power_group_for(&id).unwrap().clone();
    power.set_flow(&group, 0.0).unwrap();
    power.network_mut().consumers.get_mut(&id).unwrap().energy = 1.25;
    assert_eq!(
        continuous_for_kind(Some(&mut power), "helm_thrust", 1.0, 1.0),
        1.0
    );
    assert_eq!(
        continuous_for_kind(Some(&mut power), "helm_thrust", 1.0, 1.0),
        0.25
    );
    assert_eq!(
        continuous_for_kind(Some(&mut power), "helm_thrust", 1.0, 1.0),
        0.0
    );
    assert_eq!(power.flow_for(&group), Some(0.0));
    assert!(power.network().consumers[&id].heat > 0.0);
}

#[test]
fn idle_missing_and_invalid_work_cannot_create_free_actuation_or_deplete_reserves() {
    let (mut power, _) = power();
    let before = power.capture_continuation();
    assert_eq!(continuous_for_kind(None, "helm_thrust", 1.0, 1.0), 0.0);
    assert_eq!(
        continuous_for_kind(Some(&mut power), "absent", 1.0, 1.0),
        0.0
    );
    assert_eq!(
        continuous_for_kind(Some(&mut power), "helm_thrust", f32::NAN, 1.0),
        0.0
    );
    continuous_for_kind(Some(&mut power), "helm_thrust", 1.0, 0.0);
    assert_eq!(power.capture_continuation(), before);
}
