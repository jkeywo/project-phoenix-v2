//! Fixtures fit independent real instruments before the ordinary spawn path.
pub(super) fn clone_instrument(
    config: &mut crate::entities::config::EntityConfig,
    from: &str,
    to: &str,
) {
    use crate::core::messages::SystemId;
    let ship = config.ship_config.as_mut().unwrap();
    let mut system = ship.system(&SystemId(from.into())).unwrap().clone();
    system.id = SystemId(to.into());
    ship.systems.push(system);
    let energy = config.energy.as_mut().unwrap();
    let mut consumer = energy
        .consumers
        .iter()
        .find(|row| row.system.0 == from)
        .unwrap()
        .clone();
    consumer.system = SystemId(to.into());
    energy.consumers.push(consumer);
    let layout = config.interior.as_mut().unwrap();
    layout.placements.insert(
        SystemId(to.into()),
        layout.placements[&SystemId(from.into())].clone(),
    );
    let sensing = config.sensing.as_mut().unwrap();
    if let Some(mut profile) = sensing
        .focused
        .iter()
        .find(|row| row.system.0 == from)
        .cloned()
    {
        profile.system = SystemId(to.into());
        sensing.focused.push(profile);
    }
    if let Some(mut profile) = sensing
        .passive
        .iter()
        .find(|row| row.system.0 == from)
        .cloned()
    {
        profile.system = SystemId(to.into());
        sensing.passive.push(profile);
    }
}
