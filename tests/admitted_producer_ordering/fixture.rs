//! Prepared source state for the real Navigation and internal Repair producers.
//! The owning test controls cadence and executes the ordinary plugin graph.
use bevy::prelude::*;
use project_phoenix::{
    console::{
        navigation::{NavigationTargetSelector, NavigationWaypoint, WaypointMode},
        repair::{RepairQueueEntry, RepairRequestQueue, RepairTargetSelector, ShipRepairTeams},
    },
    core::messages::{StationId, SystemId, TeamSlot},
    entities::{
        config::DoctrineObjective,
        spawner::{BehaviourSection, EntitySystemHull, EntityUuid},
    },
    server_app::Ship,
    ship::{damage::DamageTier, state::ShipPhysics},
    ship_plugin::ShipConfigComponent,
    world::config::WorldConfig,
};
use serde::Serialize;

#[derive(Clone, Debug, Serialize)]
pub struct NavigationRepairExpectation {
    pub uuid: String,
    pub anchor: String,
    pub navigation_x: f32,
    pub navigation_z: f32,
    pub generation_before: u64,
    pub repair_system: SystemId,
    pub repair_station: StationId,
    pub initial_hp: f32,
    pub max_hp: f32,
    pub team_count: usize,
    pub travel_duration: f32,
    pub repair_rate_hp_per_sec: f32,
}

/// Call while all four producers are Human-controlled, before allowing their
/// ordinary decision tick. Main must let PublishAggregate publish the new
/// standing doctrine; writing a one-tick Viewscreen blackboard would be erased.
///
/// This deliberately prepares an already-reported internal Repair request.
/// It does not claim to exercise the upstream damage-notification/lag route.
/// Team counts, timings, selectors and authored system ownership are retained.
pub fn prepare_navigation_and_repair(
    app: &mut App,
    ships: &[Entity],
) -> Vec<NavigationRepairExpectation> {
    assert_eq!(
        ships.len(),
        2,
        "the producer fixture uses two authored cruisers"
    );
    assert_ne!(ships[0], ships[1]);
    let mut expectations = Vec::new();
    for (index, &ship) in ships.iter().enumerate() {
        assert!(app.world().get::<Ship>(ship).is_some());
        assert!(app.world().get::<NavigationTargetSelector>(ship).is_some());
        assert!(app.world().get::<RepairTargetSelector>(ship).is_some());
        let uuid = app.world().get::<EntityUuid>(ship).unwrap().0.clone();
        assert!(!uuid.is_empty());
        let physics = app.world().get::<ShipPhysics>(ship).unwrap();
        let navigation_x = physics.x + 300.0 + 100.0 * index as f32;
        let navigation_z = physics.z + 200.0;
        assert!(navigation_x.is_finite() && navigation_z.is_finite());
        let anchor = format!("producer-fixture-navigation-{index}");
        assert!(!app
            .world()
            .resource::<WorldConfig>()
            .anchors
            .contains_key(&anchor));

        let config = &app.world().get::<ShipConfigComponent>(ship).unwrap().0;
        let hull = &app.world().get::<EntitySystemHull>(ship).unwrap().0;
        // Every test target is a real fine system, with station ownership from
        // the actual cruiser config. Distinct rows also witness per-ship routing.
        let mut targets: Vec<_> = config
            .systems
            .iter()
            .filter(|system| system.station.as_ref().is_some_and(|s| s.0 == "helm"))
            .filter_map(|system| {
                let row = hull.get(&system.id)?;
                (row.max.is_finite() && row.max > 0.0).then_some((
                    system.id.clone(),
                    system.station.clone().unwrap(),
                    row.current,
                    row.max,
                ))
            })
            .collect();
        targets.sort_by(|a, b| a.0.cmp(&b.0));
        assert!(
            targets.len() >= ships.len(),
            "authored distinct Helm hull rows"
        );
        assert!(targets.iter().all(|(_, _, hp, max)| hp == max));
        let (repair_system, repair_station, _, max_hp) = targets[index].clone();
        let tiers = hull.get(&repair_system).unwrap().tier_config;
        assert!(
            tiers.disabled_threshold_pct.is_finite()
                && tiers.damaged_threshold_pct.is_finite()
                && tiers.disabled_threshold_pct >= 0.0
                && tiers.disabled_threshold_pct < tiers.damaged_threshold_pct
                && tiers.damaged_threshold_pct <= 1.0,
            "the real authored system must have a nonempty Damaged band"
        );
        // Operational includes equality at the authored damaged threshold.
        // In particular, the cruiser's dock row remains Operational
        // at 50% HP. Choose the interior of this row's actual Damaged band,
        // retaining its thresholds and avoiding Disabled/Destroyed effects.
        let damaged_ratio = tiers.disabled_threshold_pct
            + (tiers.damaged_threshold_pct - tiers.disabled_threshold_pct) * 0.5;
        let initial_hp = max_hp * damaged_ratio;
        assert!(initial_hp > 0.0 && initial_hp < max_hp);
        let teams = &app.world().get::<ShipRepairTeams>(ship).unwrap().0;
        let team_count = teams.slots().len();
        assert!(team_count > 0);
        assert!(teams
            .slots()
            .iter()
            .all(|slot| matches!(slot, TeamSlot::Idle)));
        let timings = teams.timings();
        assert!(timings.travel_duration.is_finite() && timings.travel_duration > 0.0);
        assert!(timings.repair_rate_hp_per_sec.is_finite() && timings.repair_rate_hp_per_sec > 0.0);
        assert!(app
            .world()
            .get::<RepairRequestQueue>(ship)
            .unwrap()
            .entries
            .is_empty());

        let world = app.world_mut();
        world
            .resource_mut::<WorldConfig>()
            .anchors
            .insert(anchor.clone(), [navigation_x, 0.0, navigation_z]);
        // Use the real doctrine aggregator and selector. Default construction
        // respects DoctrineObjective's private authoring-validation field.
        let mut doctrine = DoctrineObjective::default();
        doctrine.id = anchor.clone();
        doctrine.text = format!("Prepared navigation destination {index}");
        doctrine.directive_kind = Some("Reach".into());
        doctrine.directive_anchor = Some(anchor.clone());
        doctrine.base_priority = 100.0;
        // The tracer proves waypoint application and clearance without asking
        // the Helm to translate the hull during other producers' observations.
        doctrine.target_speed = 0.0;
        doctrine.use_impulse = Some(false);
        world.get_mut::<BehaviourSection>(ship).unwrap().0.doctrine = vec![doctrine];
        let generation_before = {
            let mut waypoint = world.get_mut::<NavigationWaypoint>(ship).unwrap();
            waypoint.clear();
            waypoint.generation()
        };
        let tier = {
            let mut hull = world.get_mut::<EntitySystemHull>(ship).unwrap();
            hull.0.set_hp(&repair_system, initial_hp);
            assert_eq!(hull.0.get(&repair_system).unwrap().current, initial_hp);
            let tier = hull.0.tier_for(&repair_system);
            assert_ne!(tier, DamageTier::Operational);
            assert_eq!(tier, DamageTier::Damaged);
            tier
        };
        world
            .get_mut::<RepairRequestQueue>(ship)
            .unwrap()
            .push_or_merge(RepairQueueEntry {
                station_id: repair_station.0.clone(),
                station_label: repair_station.0.clone(),
                tier,
                deficit: max_hp - initial_hp,
            });
        expectations.push(NavigationRepairExpectation {
            uuid,
            anchor,
            navigation_x,
            navigation_z,
            generation_before,
            repair_system,
            repair_station,
            initial_hp,
            max_hp,
            team_count,
            travel_duration: timings.travel_duration,
            repair_rate_hp_per_sec: timings.repair_rate_hp_per_sec,
        });
    }
    assert_ne!(expectations[0].uuid, expectations[1].uuid);
    assert_ne!(expectations[0].repair_system, expectations[1].repair_system);
    expectations
}

/// With repaired=false, call after the first actual AI decision/application
/// step and before the authored travel duration expires. With repaired=true,
/// call again after ordinary travel plus positive repair work. Main separately
/// witnesses the AI command identities and the actual RepairApplied messages.
pub fn assert_navigation_and_repair(
    app: &App,
    ships: &[Entity],
    expected: &[NavigationRepairExpectation],
    repaired: bool,
) {
    assert_eq!(ships.len(), 2);
    assert_eq!(ships.len(), expected.len());
    for (&ship, expected) in ships.iter().zip(expected) {
        assert_eq!(
            app.world().get::<EntityUuid>(ship).unwrap().0,
            expected.uuid
        );
        assert_eq!(
            app.world().resource::<WorldConfig>().anchors[&expected.anchor],
            [expected.navigation_x, 0.0, expected.navigation_z]
        );
        let waypoint = app.world().get::<NavigationWaypoint>(ship).unwrap();
        assert_eq!(
            waypoint.mode(),
            Some(&WaypointMode::Free {
                x: expected.navigation_x,
                z: expected.navigation_z,
            }),
            "the real Navigation applier must install this ship's selected goal"
        );
        assert_eq!(
            waypoint.generation(),
            expected.generation_before.wrapping_add(1),
            "stable doctrine must not re-issue an unchanged waypoint"
        );
        let config = &app.world().get::<ShipConfigComponent>(ship).unwrap().0;
        assert_eq!(
            config
                .system(&expected.repair_system)
                .unwrap()
                .station
                .as_ref(),
            Some(&expected.repair_station)
        );
        let teams = &app.world().get::<ShipRepairTeams>(ship).unwrap().0;
        assert_eq!(teams.slots().len(), expected.team_count);
        assert_eq!(teams.timings().travel_duration, expected.travel_duration);
        assert_eq!(
            teams.timings().repair_rate_hp_per_sec,
            expected.repair_rate_hp_per_sec
        );
        let hull = &app.world().get::<EntitySystemHull>(ship).unwrap().0;
        let row = hull.get(&expected.repair_system).unwrap();
        assert_eq!(row.max, expected.max_hp);
        if repaired {
            assert!(
                row.current > expected.initial_hp,
                "ordinary team work must restore HP"
            );
            assert!(row.current <= expected.max_hp);
        } else {
            assert_eq!(
                row.current, expected.initial_hp,
                "travel must precede healing"
            );
            let TeamSlot::Travelling {
                system_id, elapsed, ..
            } = &teams.slots()[0]
            else {
                panic!("the real Repair applier must dispatch the first free team");
            };
            assert_eq!(system_id.as_ref(), Some(&expected.repair_system));
            assert!(*elapsed > 0.0 && *elapsed < expected.travel_duration);
            assert!(
                teams.slots()[1..]
                    .iter()
                    .all(|slot| matches!(slot, TeamSlot::Idle)),
                "one station request must not allocate duplicate teams"
            );
        }
    }
}
