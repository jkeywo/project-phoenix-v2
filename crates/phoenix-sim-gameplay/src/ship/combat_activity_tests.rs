use super::*;
use crate::core::messages::SystemId;
use crate::server_app::{Ship, ShipAttackedThisTick, WeaponFiredThisTick};
use crate::ship::damage::SystemHull;

fn app_with_hull(hull: SystemHull) -> (App, Entity) {
    let mut app = App::new();
    app.add_plugins(bevy::time::TimePlugin)
        .add_systems(Update, update_combat_activity);
    let ship = app
        .world_mut()
        .spawn((
            Ship,
            crate::entities::spawner::EntitySystemHull(hull),
            RecentCombatActivity::default(),
            WeaponFiredThisTick::default(),
            ShipAttackedThisTick::default(),
        ))
        .id();
    (app, ship)
}

fn activity_for(app: &mut App, ship: Entity) -> RecentCombatActivity {
    app.world()
        .get::<RecentCombatActivity>(ship)
        .unwrap()
        .clone()
}

fn test_hull(current_damage: f32) -> SystemHull {
    let mut hull = SystemHull::from_config(&[(SystemId("captain".into()), 100.0)]);
    if current_damage > 0.0 {
        let mut rng = crate::sim_rng::unseeded_test_rng();
        hull.apply_damage(current_damage, &mut rng);
    }
    hull
}

#[test]
fn first_update_with_full_hull_does_not_record_damage() {
    let (mut app, ship) = app_with_hull(test_hull(0.0));

    app.update();

    assert_eq!(activity_for(&mut app, ship).last_damage_taken, None);
}

#[test]
fn first_update_with_damaged_hull_records_damage() {
    let (mut app, ship) = app_with_hull(test_hull(10.0));

    app.update();

    assert_eq!(activity_for(&mut app, ship).last_damage_taken, Some(0.0));
}

#[test]
fn weapon_fire_records_activity_and_resets_flag() {
    let (mut app, ship) = app_with_hull(test_hull(0.0));
    app.world_mut()
        .entity_mut(ship)
        .get_mut::<WeaponFiredThisTick>()
        .unwrap()
        .0 = true;

    app.update();

    assert_eq!(activity_for(&mut app, ship).last_weapon_fired, Some(0.0));
    assert!(!app.world().get::<WeaponFiredThisTick>(ship).unwrap().0);
}

#[test]
fn hostile_fire_records_activity_and_resets_flag() {
    let (mut app, ship) = app_with_hull(test_hull(0.0));
    app.world_mut()
        .entity_mut(ship)
        .get_mut::<ShipAttackedThisTick>()
        .unwrap()
        .0 = true;

    app.update();

    assert_eq!(
        activity_for(&mut app, ship).last_hostile_fire_taken,
        Some(0.0)
    );
    assert!(!app.world().get::<ShipAttackedThisTick>(ship).unwrap().0);
}

#[test]
fn actual_combat_writer_uses_logical_tick_despite_a_different_host_epoch() {
    let (mut app, ship) = app_with_hull(test_hull(10.0));
    let period = phoenix_sim_contracts::sim_tick::sim_tick_period(60.0);
    app.insert_resource(Time::<Fixed>::from_duration(period));
    app.insert_resource(phoenix_sim_contracts::sim_tick::SimTick(71));
    app.world_mut()
        .get_mut::<ShipAttackedThisTick>(ship)
        .unwrap()
        .0 = true;
    app.world_mut()
        .get_mut::<WeaponFiredThisTick>(ship)
        .unwrap()
        .0 = true;
    app.update();
    let expected = (71.0 * period.as_secs_f64()) as f32;
    let activity = activity_for(&mut app, ship);
    assert_eq!(activity.last_damage_taken, Some(expected));
    assert_eq!(activity.last_hostile_fire_taken, Some(expected));
    assert_eq!(activity.last_weapon_fired, Some(expected));
    assert_ne!(expected, app.world().resource::<Time>().elapsed_secs());
}
