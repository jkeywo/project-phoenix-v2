//! Real snapshot coverage for paid shield work and private AI continuation.
use super::*;
use crate::ai::state_tree_runtime::{ShipAiTreePrograms, ShipAiTreeRuntime};
use crate::core::messages::AdmittedCommands;
use crate::ship::power::ShipPowerSystem;
use crate::ship::shields::{ShieldsDamageHistory, ShipShields};
use crate::ship_plugin::{ShipConfigComponent, ShipSystemControlSources};
use bevy::ecs::system::RunSystemOnce;
use std::collections::BTreeMap;

fn fixture() -> (App, Entity, SystemId, SystemId, SystemId) {
    let content = crate::entities::include_resolve::load_entity_config(
        "assets/entities/alliance_cruiser.toml",
    )
    .unwrap();
    let ship = content.ship_config.unwrap();
    let hardware = content.energy.unwrap();
    let trees =
        crate::ai::state_tree_runtime::compile_ship_trees_with_energy(&ship, Some(&hardware))
            .unwrap();
    let arcs: Vec<_> = content
        .shield_arcs
        .iter()
        .map(|arc| arc.to_runtime())
        .collect();
    let shields = crate::weapons::shield::ShieldSystem::from_arcs(
        &arcs,
        &content.shields_console.unwrap().base.unwrap().to_runtime(),
    );
    let mut power =
        crate::modifiers::power_system::PowerSystem::from_energy(hardware, ship.clone()).unwrap();
    let donor = crate::ship::system_registry::shield_arc_system_id("aft").unwrap();
    let recipient = crate::ship::system_registry::shield_arc_system_id("fore").unwrap();
    let partial = crate::ship::system_registry::shield_arc_system_id("port").unwrap();
    for id in [&recipient, &partial] {
        power.set_fine_flow(id, Some(0.0)).unwrap();
        power.network_mut().consumers.get_mut(id).unwrap().energy = 0.0;
    }
    let mut sources = ShipSystemControlSources::default();
    sources.0.set(
        donor.clone(),
        crate::ship::control_source::ControlSource::Ai,
    );
    let mut app = App::new();
    app.init_resource::<crate::sim_tick::SimTick>();
    app.init_resource::<crate::world::server::WorldContentRuntime>();
    app.init_resource::<crate::effect_queue::EffectQueue<crate::infrastructure::CapacityAdjustment>>();
    app.init_resource::<crate::effect_queue::EffectQueue<crate::infrastructure::ConditionAdjustment>>();
    app.insert_resource(crate::world::config::WorldConfig::default());
    let mut time = Time::<()>::default();
    time.advance_by(std::time::Duration::from_millis(100));
    app.insert_resource(time);
    use crate::sim_rng::InstallSimRng;
    app.insert_sim_rng(crate::sim_rng::SimRng::new(
        417,
        crate::sim_rng::SeedSource::World,
    ));
    // Preserve the production capture registration gates while keeping this
    // fixture limited to the actual shield, resource and private-tree owners.
    let world = app.world_mut();
    world.register_component::<ShipPhysics>();
    world.register_component::<EntitySystemHull>();
    world.register_component::<crate::console::command::server::ShipStationStances>();
    world.register_component::<ActiveBeam>();
    world.register_component::<PhaserCooldown>();
    world.register_component::<TorpedoSystemResource>();
    world.register_component::<EntityShipArcHull>();
    world.register_component::<ShipRepairTeams>();
    world.register_component::<RepairRequestQueue>();
    world.register_component::<RepairHumanAlerted>();
    world.register_component::<crate::ai::server::ObjectiveCursors>();
    world.register_component::<crate::console::weapons::blaster::BlasterSystemResource>();
    world.register_component::<crate::ship::shields::PendingShieldsThreatBearing>();
    let entity = world
        .spawn((
            EntityUuid("snapshot-shields".into()),
            ShipRedAlert(false),
            ShipConfigComponent(ship),
            // The accepted Cruiser declares weapon machinery. Production's
            // assignments::install always seeds its retained intent owner,
            // even when no targets are selected in this shield-only rig.
            crate::console::weapons::assignments::ShipWeaponTargets::default(),
            sources,
            // Capture the real crew projection required by this configured
            // ship. This sparse owner rig has no occupied human Stations.
            crate::ship_plugin::ActiveStationRatings::default(),
            crate::lobby::crew_replication::ShipCrewPresence::default(),
            AdmittedCommands::default(),
            ShipPowerSystem(power),
            ShipShields(shields, 0.5),
            ShieldsDamageHistory::default(),
            crate::server_app::ShipSystemBlackboards::default(),
            crate::resources::server::ResourceServices(content.resource_services),
            crate::infrastructure::InfrastructureCondition(
                crate::infrastructure::InfrastructureState::from_config(
                    content.infrastructure.as_ref().unwrap(),
                ),
            ),
            trees,
        ))
        .id();
    (app, entity, donor, recipient, partial)
}

fn evidence(app: &mut App) {
    app.world_mut()
        .run_system_once(crate::ship::shields::track_shield_damage_history)
        .unwrap();
    app.world_mut()
        .run_system_once(crate::ship::shields::publish_shields_blackboard)
        .unwrap();
}
fn paid_step(app: &mut App) {
    app.world_mut()
        .run_system_once(crate::ship::power::tick_power_system)
        .unwrap();
    app.world_mut()
        .run_system_once(crate::ship::shields::tick_shields)
        .unwrap();
    app.world_mut()
        .run_system_once(crate::infrastructure::server::tick_infrastructure_condition)
        .unwrap();
    evidence(app);
    app.world_mut().resource_mut::<crate::sim_tick::SimTick>().0 += 1;
}
fn execute_authored_tree(app: &mut App, entity: Entity, donor: &SystemId, recipient: &SystemId) {
    use phoenix_sim_gameplay::ai::host::{decide_tree_with_numbers, TreeHostOutcome};
    let targets = app
        .world()
        .get::<ShipShields>(entity)
        .unwrap()
        .0
        .facings
        .iter()
        .find(|arc| arc.id == "aft")
        .unwrap()
        .energy_config
        .recovery_targets
        .clone();
    let (booleans, numeric) = phoenix_sim_gameplay::ship::shield_ai::bindings(&targets);
    let mut facts: BTreeMap<_, _> = booleans
        .facts
        .iter()
        .map(|name| (name.clone(), false))
        .collect();
    for prefix in ["target-visible-", "target-preferred-", "target-ready-"] {
        facts.insert(format!("{prefix}{}", recipient.0), true);
    }
    let mut numbers: BTreeMap<_, _> = numeric
        .facts
        .iter()
        .map(|name| (name.clone(), 0.0))
        .collect();
    numbers.insert("strength_fraction".into(), 1.0);
    let channels: BTreeMap<_, _> = booleans
        .channels
        .iter()
        .map(|name| (name.clone(), false))
        .collect();
    let tick = app.world().resource::<crate::sim_tick::SimTick>().0;
    let mut query = app.world_mut().query::<(
        &ShipAiTreePrograms,
        &ShipSystemControlSources,
        &mut ShipAiTreeRuntime,
    )>();
    let (programs, sources, mut runtime) = query.get_mut(app.world_mut(), entity).unwrap();
    let TreeHostOutcome::Evaluated(result) = decide_tree_with_numbers(
        &sources.0,
        donor,
        programs.0.get(donor),
        &facts,
        &numbers,
        &channels,
        runtime.0.get_mut(donor).unwrap(),
        tick,
    )
    .unwrap() else {
        panic!("actual authored shield tree must execute")
    };
    assert_eq!(
        result.intents.get(&format!("recover-{}", recipient.0)),
        Some(&true)
    );
}
fn arm_mid_work(
    app: &mut App,
    entity: Entity,
    donor: &SystemId,
    recipient: &SystemId,
    partial: &SystemId,
) {
    evidence(app);
    app.world_mut()
        .run_system_once(crate::ship::power::tick_power_system)
        .unwrap();
    let upkeep = app
        .world()
        .get::<ShipShields>(entity)
        .unwrap()
        .0
        .facings
        .iter()
        .find(|arc| arc.id == "port")
        .unwrap()
        .energy_config
        .upkeep_per_sec;
    app.world_mut()
        .get_mut::<ShipPowerSystem>(entity)
        .unwrap()
        .0
        .network_mut()
        .consumers
        .get_mut(partial)
        .unwrap()
        .energy = upkeep * 0.1 * 0.5;
    let mut shields = app.world_mut().get_mut::<ShipShields>(entity).unwrap();
    for arc in &mut shields.0.facings {
        if arc.id == "aft" {
            arc.raised = false;
            arc.recovery_target = Some(recipient.clone());
        }
        if arc.id == "fore" {
            arc.hp = 20;
        }
    }
    drop(shields);
    app.world_mut()
        .run_system_once(crate::ship::shields::tick_shields)
        .unwrap();
    app.world_mut()
        .run_system_once(crate::infrastructure::server::tick_infrastructure_condition)
        .unwrap();
    let mut shields = app.world_mut().get_mut::<ShipShields>(entity).unwrap();
    let port = shields
        .0
        .facings
        .iter_mut()
        .find(|arc| arc.id == "port")
        .unwrap();
    assert_eq!(port.protection_fraction, 0.5);
    assert_eq!(port.apply_damage(1), 1);
    assert_eq!(port.interception_remainder, 0.5);
    let aft = shields
        .0
        .facings
        .iter()
        .find(|arc| arc.id == "aft")
        .unwrap();
    assert!(aft.stored_strength() < aft.max_hp as f64);
    assert_eq!(aft.recovery_target.as_ref(), Some(recipient));
    drop(shields);
    evidence(app);
    execute_authored_tree(app, entity, donor, recipient);
}

#[test]
fn shield_snapshot_resumes_mid_recovery_partial_interception_and_private_tree_execution() {
    let (mut live, entity, donor, recipient, partial) = fixture();
    arm_mid_work(&mut live, entity, &donor, &recipient, &partial);
    let saved: PhoenixSnapshot =
        serde_json::from_slice(&serde_json::to_vec(&capture(live.world())).unwrap()).unwrap();
    assert_eq!(saved.entities.len(), 1);
    assert!(saved.entities[0]
        .weapon_targets
        .as_ref()
        .unwrap()
        .0
        .is_empty());
    assert!(saved.entities[0]
        .weapons
        .as_ref()
        .unwrap()
        .shield_operations
        .iter()
        .any(|arc| arc.interception_remainder == 0.5));
    assert_eq!(
        saved.entities[0].ai_trees.as_ref().unwrap().0[&donor].last_tick,
        Some(0)
    );
    let (mut resumed, restored, _, _, _) = fixture();
    let report = restore(resumed.world_mut(), &saved);
    assert!(report.is_complete(), "{report:?}");
    assert_eq!(report.entities_restored, 1);
    assert_eq!(
        serde_json::to_value(capture(resumed.world())).unwrap(),
        serde_json::to_value(&saved).unwrap()
    );
    // The next hit resolves the saved fractional debt, rather than restarting
    // partial interception with an empty accumulator.
    for (app, entity) in [(&mut live, entity), (&mut resumed, restored)] {
        let mut shields = app.world_mut().get_mut::<ShipShields>(entity).unwrap();
        let port = shields
            .0
            .facings
            .iter_mut()
            .find(|arc| arc.id == "port")
            .unwrap();
        assert_eq!(port.apply_damage(1), 0);
        assert_eq!(port.interception_remainder, 0.0);
    }
    let initial_donor_strength = live
        .world()
        .get::<ShipShields>(entity)
        .unwrap()
        .0
        .facings
        .iter()
        .find(|arc| arc.id == "aft")
        .unwrap()
        .stored_strength();
    for _ in 0..3 {
        paid_step(&mut live);
        paid_step(&mut resumed);
        execute_authored_tree(&mut live, entity, &donor, &recipient);
        execute_authored_tree(&mut resumed, restored, &donor, &recipient);
        assert_eq!(
            serde_json::to_value(capture(live.world())).unwrap(),
            serde_json::to_value(capture(resumed.world())).unwrap()
        );
    }
    assert!(
        live.world()
            .get::<ShipShields>(entity)
            .unwrap()
            .0
            .facings
            .iter()
            .find(|arc| arc.id == "aft")
            .unwrap()
            .stored_strength()
            < initial_donor_strength,
        "continued equality must include real recovery progress, not two stalled worlds"
    );
    assert_eq!(
        live.world().get::<ShipAiTreeRuntime>(entity).unwrap().0[&donor].last_tick,
        Some(3)
    );
}

#[test]
fn shield_snapshot_rejects_invalid_recovery_before_writing_any_entity_or_clock() {
    let (mut source, entity, donor, recipient, partial) = fixture();
    arm_mid_work(&mut source, entity, &donor, &recipient, &partial);
    let mut invalid = capture(source.world());
    invalid.tick = 999;
    invalid.entities[0].red_alert = Some(true);
    invalid.entities[0]
        .weapons
        .as_mut()
        .unwrap()
        .shield_operations[0]
        .recovery_target = Some(SystemId("not-an-installed-battery".into()));
    let (mut target, _, _, _, _) = fixture();
    let before = serde_json::to_value(capture(target.world())).unwrap();
    let report = restore(target.world_mut(), &invalid);
    assert!(
        report
            .gaps
            .iter()
            .any(|gap| matches!(gap, RestoreGap::ResourceStateInvalid { .. })),
        "{report:?}"
    );
    assert_eq!(report.entities_restored, 0);
    assert_eq!(
        serde_json::to_value(capture(target.world())).unwrap(),
        before,
        "preflight must reject before alert, clock, reserves, charge or AI memory changes"
    );
}

#[test]
fn interior_snapshot_rejects_missing_team_owner_before_writing_any_entity_or_clock() {
    fn with_teams() -> (App, Entity) {
        let (mut app, entity, _, _, _) = fixture();
        let content = crate::entities::include_resolve::load_entity_config(
            "assets/entities/alliance_cruiser.toml",
        )
        .unwrap();
        let count = content.repair.as_ref().unwrap().repair_team_count as usize;
        let graph = phoenix_sim_gameplay::ship::interior::InteriorGraph::compile(
            content.interior.as_ref().unwrap(),
            content.ship_config.as_ref().unwrap(),
            count,
        )
        .unwrap();
        let mut teams = crate::modifiers::repair_teams::RepairTeams::new(count);
        teams.install_layout(graph);
        app.world_mut()
            .entity_mut(entity)
            .insert(ShipRepairTeams(teams));
        (app, entity)
    }
    let (source, _) = with_teams();
    let mut invalid = capture(source.world());
    assert!(invalid.entities[0]
        .repair
        .as_ref()
        .unwrap()
        .jobs
        .as_ref()
        .unwrap()
        .navigation
        .is_some());
    invalid.entities[0].repair = None;
    invalid.entities[0].red_alert = Some(true);
    invalid.tick = 999;
    let (mut target, _) = with_teams();
    let before = serde_json::to_value(capture(target.world())).unwrap();
    let report = restore(target.world_mut(), &invalid);
    assert!(
        report
            .gaps
            .iter()
            .any(|gap| matches!(gap, RestoreGap::RepairJobsInvalid { .. })),
        "{report:?}"
    );
    assert_eq!(report.entities_restored, 0);
    assert_eq!(
        serde_json::to_value(capture(target.world())).unwrap(),
        before
    );
}
