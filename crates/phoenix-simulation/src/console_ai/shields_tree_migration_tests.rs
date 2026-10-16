//! Coverage migrated from the retired coarse controller to the actual arc host.
use super::*;
use crate::ship::control_source::ControlSource;
use crate::ship::power::ShipPowerSystem;
use crate::ship::shields::{ShieldsAiConfigResource, ShieldsDamageHistory, ShipShields};
use crate::ship_plugin::{ShipConfigComponent, ShipSystemControlSources};
use bevy::ecs::system::RunSystemOnce;

fn app() -> App {
    let mut app = App::new();
    crate::ai::host::register_ai_host_env(&mut app);
    app.init_resource::<crate::sim_tick::SimTick>();
    let mut world = crate::world::config::WorldConfig::default();
    world.global.sim_tick_hz = 60.0;
    app.insert_resource(world);
    let mut time = Time::<()>::default();
    time.advance_by(std::time::Duration::from_millis(100));
    app.insert_resource(time);
    app
}

fn actor(app: &mut App, uuid: &str) -> Entity {
    let content = crate::entities::include_resolve::load_entity_config(
        "assets/entities/alliance_cruiser.toml",
    )
    .unwrap();
    let ship = content.ship_config.unwrap();
    let hardware = content.energy.unwrap();
    let (programs, frames) =
        crate::ai::state_tree_runtime::compile_ship_trees_with_energy(&ship, Some(&hardware))
            .unwrap();
    let arcs: Vec<_> = content
        .shield_arcs
        .iter()
        .map(|arc| arc.to_runtime())
        .collect();
    let console = content.shields_console.unwrap();
    let mut shields =
        crate::weapons::shield::ShieldSystem::from_arcs(&arcs, &console.base.unwrap().to_runtime());
    shields.focus_config = crate::weapons::shield::ShieldFocusConfig {
        bonus_max_hp: console.focus_bonus_max_hp,
        bonus_regen: console.focus_bonus_regen,
        penalty_max_hp: console.focus_penalty_max_hp,
        penalty_regen: console.focus_penalty_regen,
        decay_rate: console.focus_decay_rate,
        focused_damage_multiplier: console.focus_focused_damage_multiplier,
        unfocused_damage_multiplier: console.focus_unfocused_damage_multiplier,
    };
    let tuning = console
        .ai
        .map(|config| ShieldsAiConfigResource {
            damage_window_secs: config.damage_window_secs,
            min_damage_window_secs: config.min_damage_window_secs,
            damage_pct_threshold: config.damage_pct_threshold,
            health_ratio_threshold: config.health_ratio_threshold,
            ..Default::default()
        })
        .unwrap_or_default();
    let mut sources = ShipSystemControlSources::default();
    for definition in ship
        .systems
        .iter()
        .filter(|system| system.kind == crate::ship::system_registry::SHIELD_ARC_KIND)
    {
        sources.0.set(definition.id.clone(), ControlSource::Ai);
    }
    let power =
        crate::modifiers::power_system::PowerSystem::from_energy(hardware, ship.clone()).unwrap();
    app.world_mut()
        .spawn((
            crate::server_app::Ship,
            crate::entities::spawner::EntityUuid(uuid.into()),
            ShipConfigComponent(ship),
            sources,
            ShipPowerSystem(power),
            ShipShields(shields, console.frequency),
            tuning,
            ShieldsDamageHistory::default(),
            crate::server_app::ShipSystemBlackboards::default(),
            crate::core::messages::AdmittedCommands::default(),
            programs,
            frames,
        ))
        .id()
}

fn index(app: &App, entity: Entity, id: &str) -> usize {
    app.world()
        .get::<ShipShields>(entity)
        .unwrap()
        .0
        .facings
        .iter()
        .position(|arc| arc.id == id)
        .unwrap()
}
fn focus(app: &App, entity: Entity) -> Option<String> {
    let shields = &app.world().get::<ShipShields>(entity).unwrap().0;
    shields
        .focused_facing
        .map(|index| shields.facings[index].id.clone())
}
fn observe(app: &mut App) {
    app.world_mut()
        .run_system_once(crate::ship::shields::track_shield_damage_history)
        .unwrap();
    app.world_mut()
        .run_system_once(crate::ship::shields::publish_shields_blackboard)
        .unwrap();
}
fn decide(app: &mut App) -> Vec<crate::core::messages::AdmittedCommand> {
    for mut admitted in app
        .world_mut()
        .query::<&mut crate::core::messages::AdmittedCommands>()
        .iter_mut(app.world_mut())
    {
        admitted.0.clear();
    }
    app.world_mut().run_system_once(ai_shields).unwrap();
    let commands = app
        .world_mut()
        .query::<&crate::core::messages::AdmittedCommands>()
        .iter(app.world())
        .flat_map(|admitted| admitted.0.clone())
        .collect();
    app.world_mut()
        .run_system_once(crate::ship::shields::handle_shields_messages)
        .unwrap();
    app.world_mut().resource_mut::<crate::sim_tick::SimTick>().0 += 1;
    commands
}
fn health(app: &mut App, entity: Entity, arc: &str, fraction: f32) {
    let index = index(app, entity, arc);
    let mut shields = app.world_mut().get_mut::<ShipShields>(entity).unwrap();
    shields.0.facings[index].hp = (shields.0.facings[index].max_hp as f32 * fraction) as i32;
}
fn injury(app: &mut App, entity: Entity, arc: &str, damage: i32) {
    let index = index(app, entity, arc);
    let mut shields = app.world_mut().get_mut::<ShipShields>(entity).unwrap();
    shields.0.facings[index].protection_fraction = 1.0;
    assert_eq!(shields.0.facings[index].apply_damage(damage), 0);
}
fn idle_tree() -> crate::ai::state_tree::AiTreeConfig {
    toml::from_str("root='idle'\n[[tree]]\nid='idle'\ninitial='hold'\n[[tree.state]]\nid='hold'\n")
        .unwrap()
}
fn set_trees(
    app: &mut App,
    entity: Entity,
    mut configure: impl FnMut(&SystemId) -> crate::ai::state_tree::AiTreeConfig,
) {
    let mut config = app
        .world()
        .get::<ShipConfigComponent>(entity)
        .unwrap()
        .0
        .clone();
    for system in config
        .systems
        .iter_mut()
        .filter(|system| system.kind == crate::ship::system_registry::SHIELD_ARC_KIND)
    {
        system
            .config
            .as_mut()
            .unwrap()
            .as_table_mut()
            .unwrap()
            .insert(
                "ai".into(),
                toml::Value::try_from(configure(&system.id)).unwrap(),
            );
    }
    let hardware = app
        .world()
        .get::<ShipPowerSystem>(entity)
        .unwrap()
        .0
        .config()
        .clone();
    let trees =
        crate::ai::state_tree_runtime::compile_ship_trees_with_energy(&config, Some(&hardware))
            .unwrap();
    app.world_mut()
        .entity_mut(entity)
        .insert((ShipConfigComponent(config), trees));
}

#[test]
fn separate_actors_use_their_own_observed_health_and_authored_tuning() {
    let mut app = app();
    let first = actor(&mut app, "first");
    let second = actor(&mut app, "second");
    assert!(
        app.world()
            .get::<crate::ai::server::AiHighFidelity>(first)
            .is_none(),
        "ordinary arc AI and evidence do not depend on the visual fidelity marker"
    );
    health(&mut app, first, "fore", 0.25);
    health(&mut app, second, "aft", 0.25);
    app.world_mut()
        .get_mut::<ShieldsAiConfigResource>(second)
        .unwrap()
        .health_ratio_threshold = 10.0;
    observe(&mut app);
    decide(&mut app);
    assert_eq!(focus(&app, first).as_deref(), Some("fore"));
    assert_eq!(
        focus(&app, second),
        None,
        "the second actor's own lower imbalance threshold holds"
    );
    app.world_mut()
        .get_mut::<ShieldsAiConfigResource>(second)
        .unwrap()
        .health_ratio_threshold = 90.0;
    observe(&mut app);
    decide(&mut app);
    assert_eq!(focus(&app, second).as_deref(), Some("aft"));
    assert_eq!(
        focus(&app, first).as_deref(),
        Some("fore"),
        "NPC focus changes never mutate another actor"
    );
}

#[test]
fn live_damage_cannot_drive_ai_until_the_permitted_observation_is_published() {
    let mut app = app();
    let entity = actor(&mut app, "published-damage");
    observe(&mut app);
    let starboard = index(&app, entity, "starboard");
    let damage = app.world().get::<ShipShields>(entity).unwrap().0.facings[starboard].hp * 3 / 4;
    injury(&mut app, entity, "starboard", damage);
    assert!(
        decide(&mut app).is_empty(),
        "the host does not inspect fresh live HP or private damage state"
    );
    observe(&mut app);
    let commands = decide(&mut app);
    let id = crate::ship::system_registry::shield_arc_system_id("starboard").unwrap();
    assert!(commands.iter().any(|command| command.target == id
        && command.payload == Control::SetShieldArcFocus { focused: true }));
    assert_eq!(focus(&app, entity).as_deref(), Some("starboard"));
}

#[test]
fn human_and_offline_arcs_reject_ai_actuation_without_discarding_observations() {
    let mut app = app();
    let entity = actor(&mut app, "human-arc");
    health(&mut app, entity, "fore", 0.1);
    let id = crate::ship::system_registry::shield_arc_system_id("fore").unwrap();
    app.world_mut()
        .get_mut::<ShipSystemControlSources>(entity)
        .unwrap()
        .0
        .set(id.clone(), ControlSource::Human);
    observe(&mut app);
    assert!(decide(&mut app).iter().all(|command| command.target != id));
    assert_eq!(focus(&app, entity), None);
    let mut sources = app
        .world_mut()
        .get_mut::<ShipSystemControlSources>(entity)
        .unwrap();
    sources.0.set(id.clone(), ControlSource::Ai);
    sources.0.set_offline(id.clone(), true);
    drop(sources);
    assert!(decide(&mut app).iter().all(|command| command.target != id));
    app.world_mut()
        .get_mut::<ShipSystemControlSources>(entity)
        .unwrap()
        .0
        .set_offline(id.clone(), false);
    observe(&mut app);
    assert!(decide(&mut app).iter().any(|command| command.target == id));
    assert_eq!(focus(&app, entity).as_deref(), Some("fore"));
}

#[test]
fn physical_history_accumulates_and_expires_at_the_authored_tick_boundary() {
    let mut app = app();
    let entity = actor(&mut app, "history");
    let mut tuning = app
        .world_mut()
        .get_mut::<ShieldsAiConfigResource>(entity)
        .unwrap();
    tuning.damage_window_secs = 0.6;
    tuning.min_damage_window_secs = 0.0;
    drop(tuning);
    observe(&mut app);
    app.world_mut().resource_mut::<crate::sim_tick::SimTick>().0 = 1;
    injury(&mut app, entity, "port", 10);
    observe(&mut app);
    // Keep both impacts unfocused so the test measures incoming physical
    // evidence, independently of the selected arc's damage reduction.
    app.world_mut().resource_mut::<crate::sim_tick::SimTick>().0 = 2;
    injury(&mut app, entity, "port", 5);
    observe(&mut app);
    decide(&mut app);
    let arc = index(&app, entity, "port");
    assert_eq!(
        app.world()
            .get::<ShieldsDamageHistory>(entity)
            .unwrap()
            .arcs[arc]
            .iter()
            .map(|record| record.amount)
            .sum::<i32>(),
        15
    );
    assert_eq!(focus(&app, entity).as_deref(), Some("port"));
    app.world_mut().resource_mut::<crate::sim_tick::SimTick>().0 = 37;
    observe(&mut app);
    assert_eq!(
        app.world()
            .get::<ShieldsDamageHistory>(entity)
            .unwrap()
            .arcs[arc]
            .iter()
            .map(|record| record.amount)
            .sum::<i32>(),
        5,
        "0.6f32 seconds is exactly 36 ticks; age 36 expires while age 35 remains"
    );
    app.world_mut().resource_mut::<crate::sim_tick::SimTick>().0 = 38;
    observe(&mut app);
    assert!(app
        .world()
        .get::<ShieldsDamageHistory>(entity)
        .unwrap()
        .arcs
        .iter()
        .all(Vec::is_empty));
}

#[test]
fn focus_decay_and_voluntary_recovery_are_not_reported_as_incoming_damage() {
    let mut app = app();
    let entity = actor(&mut app, "voluntary-work");
    observe(&mut app);
    let fore = index(&app, entity, "fore");
    let aft = index(&app, entity, "aft");
    // Supported authored focus variation exercises real decay even though the
    // shipped Cruiser currently authors no capacity penalty.
    let mut shields = app.world_mut().get_mut::<ShipShields>(entity).unwrap();
    shields.0.focus_config.penalty_max_hp = 20;
    shields.0.focus_config.decay_rate = 10.0;
    shields.0.set_focused_facing(Some(fore));
    drop(shields);
    let before = app.world().get::<ShipShields>(entity).unwrap().0.facings[aft].hp;
    app.world_mut()
        .run_system_once(crate::ship::shields::tick_shields)
        .unwrap();
    assert!(app.world().get::<ShipShields>(entity).unwrap().0.facings[aft].hp < before);
    observe(&mut app);
    assert!(app
        .world()
        .get::<ShieldsDamageHistory>(entity)
        .unwrap()
        .arcs
        .iter()
        .all(Vec::is_empty));
    let target = crate::ship::system_registry::shield_arc_system_id("fore").unwrap();
    app.world_mut()
        .get_mut::<ShipPowerSystem>(entity)
        .unwrap()
        .0
        .network_mut()
        .consumers
        .get_mut(&target)
        .unwrap()
        .energy = 0.0;
    let mut shields = app.world_mut().get_mut::<ShipShields>(entity).unwrap();
    shields.0.set_focused_facing(None);
    shields.0.facings[aft].raised = false;
    shields.0.facings[fore].raised = false;
    shields.0.facings[aft].recovery_target = Some(target);
    let before = shields.0.facings[aft].stored_strength();
    drop(shields);
    app.world_mut()
        .run_system_once(crate::ship::shields::tick_shields)
        .unwrap();
    assert!(
        app.world().get::<ShipShields>(entity).unwrap().0.facings[aft].stored_strength() < before
    );
    observe(&mut app);
    assert!(app
        .world()
        .get::<ShieldsDamageHistory>(entity)
        .unwrap()
        .arcs
        .iter()
        .all(Vec::is_empty));
    injury(&mut app, entity, "starboard", 7);
    observe(&mut app);
    let starboard = index(&app, entity, "starboard");
    assert_eq!(
        app.world()
            .get::<ShieldsDamageHistory>(entity)
            .unwrap()
            .arcs[starboard]
            .iter()
            .map(|record| record.amount)
            .sum::<i32>(),
        7
    );
}

#[test]
fn observed_threat_bearing_takes_precedence_and_an_authored_idle_tree_stays_idle() {
    let mut app = app();
    let entity = actor(&mut app, "threat");
    health(&mut app, entity, "fore", 0.1);
    observe(&mut app);
    app.world_mut()
        .entity_mut(entity)
        .insert(crate::ship::sensors::SensorsThreatState {
            last_threat_uuid: Some("hostile".into()),
            last_bearing_rad: Some(std::f32::consts::FRAC_PI_2),
            last_label: Some("closing".into()),
            last_distance: Some(100.0),
        });
    observe(&mut app);
    decide(&mut app);
    assert_eq!(focus(&app, entity).as_deref(), Some("starboard"));
    set_trees(&mut app, entity, |_| idle_tree());
    health(&mut app, entity, "aft", 0.05);
    observe(&mut app);
    assert!(decide(&mut app).is_empty());
    assert_eq!(focus(&app, entity).as_deref(), Some("starboard"));
}

#[test]
fn unobserved_world_flags_are_not_supported_arc_tree_inputs() {
    let mut app = app();
    let entity = actor(&mut app, "closed-input");
    let mut config = app
        .world()
        .get::<ShipConfigComponent>(entity)
        .unwrap()
        .0
        .clone();
    let system = config
        .systems
        .iter_mut()
        .find(|system| system.kind == crate::ship::system_registry::SHIELD_ARC_KIND)
        .unwrap();
    let mut ai = idle_tree();
    ai.tree[0].state[0]
        .task
        .push(crate::ai::state_tree::TaskDef {
            id: "private-world-flag".into(),
            when: Vec::new(),
            action: crate::ai::state_tree::TaskAction::Write {
                channel: "focus".into(),
                value: crate::ai::state_tree::BoolSource::Fact {
                    name: "brace_for_impact".into(),
                },
            },
        });
    system
        .config
        .as_mut()
        .unwrap()
        .as_table_mut()
        .unwrap()
        .insert("ai".into(), toml::Value::try_from(ai).unwrap());
    let hardware = app
        .world()
        .get::<ShipPowerSystem>(entity)
        .unwrap()
        .0
        .config()
        .clone();
    let error =
        crate::ai::state_tree_runtime::compile_ship_trees_with_energy(&config, Some(&hardware))
            .err()
            .expect("unobserved world input must be rejected");
    assert!(error.contains("brace_for_impact"), "{error}");
}

#[test]
fn private_tree_memory_and_shield_continuation_resume_after_damage_evidence_expires() {
    let mut live = app();
    let entity = actor(&mut live, "continuation");
    let fore = crate::ship::system_registry::shield_arc_system_id("fore").unwrap();
    let remembering: crate::ai::state_tree::AiTreeConfig = toml::from_str(r#"
root = "remember"
[[tree]]
id = "remember"
initial = "watch"
[tree.memory]
hit = false
[[tree.state]]
id = "watch"
[[tree.state.task]]
id = "retain-hit"
when = [{ value = { source = "number_compare", name = "recent_damage", comparison = "greater", value = 0.0 }, equals = true }]
action = { kind = "remember", key = "hit", value = { source = "literal", value = true } }
[[tree.state.task]]
id = "focus-remembered"
action = { kind = "write", channel = "focus", value = { source = "memory", name = "hit" } }
"#).unwrap();
    set_trees(&mut live, entity, |id| {
        if id == &fore {
            remembering.clone()
        } else {
            idle_tree()
        }
    });
    observe(&mut live);
    injury(&mut live, entity, "fore", 10);
    observe(&mut live);
    decide(&mut live);
    assert_eq!(focus(&live, entity).as_deref(), Some("fore"));
    assert_eq!(
        live.world()
            .get::<crate::ai::state_tree_runtime::ShipAiTreeRuntime>(entity)
            .unwrap()
            .0[&fore]
            .memory("hit"),
        Some(true)
    );
    let saved_tree: crate::ai::state_tree_runtime::ShipAiTreeRuntime = serde_json::from_str(
        &serde_json::to_string(
            live.world()
                .get::<crate::ai::state_tree_runtime::ShipAiTreeRuntime>(entity)
                .unwrap(),
        )
        .unwrap(),
    )
    .unwrap();
    let saved_power = live
        .world()
        .get::<ShipPowerSystem>(entity)
        .unwrap()
        .0
        .capture_continuation();
    let saved_history = live
        .world()
        .get::<ShieldsDamageHistory>(entity)
        .unwrap()
        .clone();
    let saved_operations: Vec<crate::weapons::shield::ShieldOperationState> = serde_json::from_str(
        &serde_json::to_string(
            &live
                .world()
                .get::<ShipShields>(entity)
                .unwrap()
                .0
                .capture_operations(),
        )
        .unwrap(),
    )
    .unwrap();
    let saved_charge: Vec<_> = live
        .world()
        .get::<ShipShields>(entity)
        .unwrap()
        .0
        .facings
        .iter()
        .map(|arc| {
            (
                arc.id.clone(),
                arc.hp,
                arc.hp_frac(),
                arc.offline_remaining,
                arc.is_focused,
            )
        })
        .collect();
    let mut resumed = app();
    let restored = actor(&mut resumed, "continuation");
    set_trees(&mut resumed, restored, |id| {
        if id == &fore {
            remembering.clone()
        } else {
            idle_tree()
        }
    });
    resumed
        .world_mut()
        .entity_mut(restored)
        .insert((saved_tree, saved_history));
    resumed
        .world_mut()
        .get_mut::<ShipPowerSystem>(restored)
        .unwrap()
        .0
        .restore_continuation(&saved_power)
        .unwrap();
    let mut shields = resumed
        .world_mut()
        .get_mut::<ShipShields>(restored)
        .unwrap();
    shields.0.restore_facings(&saved_charge);
    shields.0.restore_operations(&saved_operations).unwrap();
    drop(shields);
    let tuning = live.world().get::<ShieldsAiConfigResource>(entity).unwrap();
    let hz = live
        .world()
        .resource::<crate::world::config::WorldConfig>()
        .global
        .sim_tick_hz;
    let after_expiry =
        (tuning.damage_window_secs.max(tuning.min_damage_window_secs) * hz).ceil() as u64 + 10;
    for (world, actor) in [(&mut live, entity), (&mut resumed, restored)] {
        world
            .world_mut()
            .resource_mut::<crate::sim_tick::SimTick>()
            .0 = after_expiry;
        world
            .world_mut()
            .get_mut::<ShipShields>(actor)
            .unwrap()
            .0
            .set_focused_facing(None);
        observe(world);
        assert!(world
            .world()
            .get::<ShieldsDamageHistory>(actor)
            .unwrap()
            .arcs
            .iter()
            .all(Vec::is_empty));
    }
    let live_commands = decide(&mut live);
    let resumed_commands = decide(&mut resumed);
    assert_eq!(live_commands, resumed_commands);
    assert!(
        live_commands.iter().any(|command| command.target == fore
            && command.payload == Control::SetShieldArcFocus { focused: true }),
        "remembered state still acts when current damage facts are zero; a fresh tree would hold"
    );
    assert_eq!(focus(&live, entity), focus(&resumed, restored));
    assert_eq!(
        live.world()
            .get::<crate::ai::state_tree_runtime::ShipAiTreeRuntime>(entity),
        resumed
            .world()
            .get::<crate::ai::state_tree_runtime::ShipAiTreeRuntime>(restored)
    );
}

#[test]
fn authored_unavailable_arcs_do_not_emit_focus_commands() {
    let mut app = app();
    let entity = actor(&mut app, "focus-unavailable");
    health(&mut app, entity, "fore", 0.1);
    for arc in &mut app
        .world_mut()
        .get_mut::<ShipShields>(entity)
        .unwrap()
        .0
        .facings
    {
        arc.focus_available = false;
    }
    observe(&mut app);
    assert!(decide(&mut app).iter().all(|command| !matches!(
        command.payload,
        Control::SetShieldArcFocus { focused: true }
    )));
    assert_eq!(focus(&app, entity), None);
    for arc in &mut app
        .world_mut()
        .get_mut::<ShipShields>(entity)
        .unwrap()
        .0
        .facings
    {
        arc.focus_available = true;
    }
    observe(&mut app);
    assert!(decide(&mut app).iter().any(|command| matches!(
        command.payload,
        Control::SetShieldArcFocus { focused: true }
    )));
}
