use super::*;
use bevy::ecs::message::Messages;

const VICTIM_A: &str = "00000000-0000-4000-8000-000000000001";
const VICTIM_B: &str = "00000000-0000-4000-8000-000000000002";
const SOURCE: &str = "00000000-0000-4000-8000-000000000003";

fn damage(victim: &str, attacker: Option<&str>, weapon: &str, amount: f32) -> BalanceEvent {
    BalanceEvent::DamageApplied {
        attacker: attacker.map(str::to_owned),
        victim: victim.to_owned(),
        victim_kind: VictimKind::Ship,
        weapon: weapon.to_owned(),
        amount,
        shield_absorbed: 1.0,
        hull_damage: amount - 1.0,
        system_hit: None,
    }
}

fn take(app: &mut App) -> Vec<GmActivityFeedPayload> {
    app.world_mut()
        .resource_mut::<Messages<GmActivityFeedChanged>>()
        .drain()
        .map(|event| event.payload)
        .collect()
}

fn app(depth: u32) -> App {
    let mut config = crate::world::config::WorldConfig::default();
    config.global.gm_activity_history_depth = depth;
    let mut app = App::new();
    app.add_plugins(bevy::state::app::StatesPlugin)
        .init_state::<GamePhase>()
        .insert_resource(BrowserGameMaster)
        .insert_resource(crate::sim_tick::SimTick(0))
        .insert_resource(config)
        .add_message::<BalanceEvent>()
        .add_plugins(GmActivityPlugin);
    app
}

#[test]
fn pure_projection_canonically_orders_damage_then_destruction_and_preserves_repeats() {
    let events = vec![
        BalanceEvent::EntityDestroyed {
            victim: VICTIM_A.into(),
            killer: Some(SOURCE.into()),
        },
        damage(VICTIM_B, Some(SOURCE), "z-bank", 8.0),
        damage(VICTIM_A, None, "region", 4.0),
        damage(VICTIM_A, Some(SOURCE), "a-bank", 3.0),
        damage(VICTIM_A, Some(SOURCE), "a-bank", 3.0),
    ];
    let projected = project_balance_events(17, &events, &BTreeMap::new());
    assert_eq!(projected.len(), 5);
    assert_eq!(projected[0].victim.entity_id, VICTIM_A);
    assert!(projected[0].source.is_none());
    assert_eq!(projected[1], projected[2], "exact repeats are not deduped");
    assert_eq!(projected[3].victim.entity_id, VICTIM_B);
    assert_eq!(projected[4].category, GmActivityCategory::Destruction);
    assert!(projected.iter().all(|entry| entry.tick == 17));
}

#[test]
fn pure_projection_uses_float_total_order_for_damage_details() {
    let events = vec![
        damage(VICTIM_A, Some(SOURCE), "bank", f32::INFINITY),
        damage(VICTIM_A, Some(SOURCE), "bank", -0.0),
        damage(VICTIM_A, Some(SOURCE), "bank", 0.0),
        damage(VICTIM_A, Some(SOURCE), "bank", f32::NEG_INFINITY),
    ];
    let projected = project_balance_events(3, &events, &BTreeMap::new());
    let amounts: Vec<f32> = projected
        .iter()
        .map(|entry| entry.damage.as_ref().unwrap().amount)
        .collect();
    assert_eq!(amounts, vec![f32::NEG_INFINITY, -0.0, 0.0, f32::INFINITY]);
}

#[test]
fn pure_projection_ignores_every_unrelated_balance_variant() {
    let events = vec![
        BalanceEvent::WeaponFired {
            shooter: Some(SOURCE.into()),
            weapon: "bank".into(),
            kind: "beam".into(),
        },
        BalanceEvent::RedAlertChanged {
            ship: VICTIM_A.into(),
            on: true,
        },
        BalanceEvent::RepairApplied {
            ship: VICTIM_A.into(),
            hp: 3.0,
        },
    ];
    assert!(project_balance_events(2, &events, &BTreeMap::new()).is_empty());
}

#[test]
fn pure_history_bounds_oldest_first_without_collapsing_repeats() {
    let events = vec![
        damage(VICTIM_A, None, "region", 2.0),
        damage(VICTIM_A, None, "region", 2.0),
        damage(VICTIM_A, None, "region", 2.0),
    ];
    let repeated = project_balance_events(4, &events, &BTreeMap::new());
    let mut history = GmActivityHistory::new(2);
    assert!(history.append(repeated));
    let payload = history.payload();
    assert_eq!(payload.capacity, 2);
    assert_eq!(payload.entries.len(), 2);
    assert_eq!(payload.entries[0], payload.entries[1]);
}

#[test]
fn plugin_uses_current_tick_authored_capacity_none_source_and_no_redundant_publish() {
    let mut app = app(2);
    app.world_mut().resource_mut::<crate::sim_tick::SimTick>().0 = 41;
    app.world_mut()
        .resource_mut::<Messages<BalanceEvent>>()
        .write(damage(VICTIM_A, None, "region", 2.0));
    app.world_mut().run_schedule(FixedLast);
    let first = take(&mut app);
    assert_eq!(first.len(), 1);
    assert_eq!(first[0].capacity, 2);
    assert_eq!(first[0].entries[0].tick, 41);
    assert!(first[0].entries[0].source.is_none());

    app.world_mut().run_schedule(FixedLast);
    assert!(
        take(&mut app).is_empty(),
        "unchanged feed is not republished"
    );

    app.world_mut()
        .resource_mut::<crate::world::config::WorldConfig>()
        .global
        .gm_activity_history_depth = 1;
    app.world_mut().run_schedule(FixedLast);
    let resized = take(&mut app);
    assert_eq!(resized.len(), 1, "a capacity change is republished once");
    assert_eq!(resized[0].capacity, 1);
    assert_eq!(resized[0].entries.len(), 1);
    app.world_mut().run_schedule(FixedLast);
    assert!(take(&mut app).is_empty());
}

#[test]
fn identity_cache_survives_despawn_and_uses_uuid_fallback_for_ordinary_asteroids() {
    let mut app = app(8);
    let ship = app
        .world_mut()
        .spawn((
            EntityUuid(VICTIM_A.into()),
            EntityName("Named victim".into()),
        ))
        .id();
    let rock = app.world_mut().spawn(AsteroidUuid(VICTIM_B.into())).id();
    app.world_mut().run_schedule(FixedUpdate);
    app.world_mut().despawn(ship);
    app.world_mut().despawn(rock);
    app.world_mut()
        .resource_mut::<Messages<BalanceEvent>>()
        .write(damage(VICTIM_A, Some(VICTIM_B), "impact", 2.0));
    app.world_mut()
        .resource_mut::<Messages<BalanceEvent>>()
        .write(BalanceEvent::DamageApplied {
            attacker: None,
            victim: VICTIM_B.into(),
            victim_kind: VictimKind::Asteroid,
            weapon: "collision".into(),
            amount: 1.0,
            shield_absorbed: 0.0,
            hull_damage: 1.0,
            system_hit: None,
        });
    app.world_mut().run_schedule(FixedLast);
    let payload = take(&mut app).pop().unwrap();
    assert_eq!(payload.entries[0].victim.name, "Named victim");
    assert_eq!(payload.entries[0].source.as_ref().unwrap().name, VICTIM_B);
    assert_eq!(payload.entries[1].victim.name, VICTIM_B);
    assert_eq!(
        payload.entries[1].damage.as_ref().unwrap().victim_kind,
        VictimKind::Asteroid,
        "an ordinary asteroid stays an asteroid in the feed projection"
    );
}

#[test]
fn identity_cache_prunes_asteroid_churn_but_keeps_names_used_by_bounded_rows() {
    let mut app = app(1);
    let retained = app
        .world_mut()
        .spawn((
            AsteroidUuid(VICTIM_A.into()),
            EntityName("Retained rock".into()),
        ))
        .id();
    app.world_mut().run_schedule(FixedUpdate);
    app.world_mut().despawn(retained);
    app.world_mut()
        .resource_mut::<Messages<BalanceEvent>>()
        .write(BalanceEvent::DamageApplied {
            attacker: None,
            victim: VICTIM_A.into(),
            victim_kind: VictimKind::Asteroid,
            weapon: "collision".into(),
            amount: 1.0,
            shield_absorbed: 0.0,
            hull_damage: 1.0,
            system_hit: None,
        });
    app.world_mut().run_schedule(FixedLast);
    let first = take(&mut app).pop().unwrap();
    assert_eq!(first.entries[0].victim.name, "Retained rock");
    assert_eq!(app.world().resource::<GmActivityState>().names.len(), 1);

    // Asteroid streaming continually mints fresh UUIDs. Once each unreferenced
    // rock leaves the ECS, its cached fallback must not outlive this bounded
    // collection pass, while the removed rock in history remains readable.
    for index in 0..64 {
        let transient = app
            .world_mut()
            .spawn(AsteroidUuid(format!("streamed-asteroid-{index}")))
            .id();
        app.world_mut().run_schedule(FixedUpdate);
        app.world_mut().despawn(transient);
        app.world_mut().run_schedule(FixedLast);
        assert_eq!(
            app.world().resource::<GmActivityState>().names.len(),
            1,
            "unreferenced streamed asteroid {index} leaked into the directory"
        );
    }
    let state = app.world().resource::<GmActivityState>();
    assert_eq!(
        state.names.get(VICTIM_A).map(String::as_str),
        Some("Retained rock")
    );
    assert_eq!(
        state.history.payload().entries[0].victim.name,
        "Retained rock"
    );

    // Once the ring evicts that row, its removed identity is no longer needed.
    let replacement = app
        .world_mut()
        .spawn((
            AsteroidUuid(VICTIM_B.into()),
            EntityName("Replacement rock".into()),
        ))
        .id();
    app.world_mut().run_schedule(FixedUpdate);
    app.world_mut().despawn(replacement);
    app.world_mut()
        .resource_mut::<Messages<BalanceEvent>>()
        .write(BalanceEvent::EntityDestroyed {
            victim: VICTIM_B.into(),
            killer: None,
        });
    app.world_mut().run_schedule(FixedLast);
    let state = app.world().resource::<GmActivityState>();
    assert_eq!(state.names.len(), 1);
    assert!(!state.names.contains_key(VICTIM_A));
    assert_eq!(
        state.names.get(VICTIM_B).map(String::as_str),
        Some("Replacement rock")
    );
}

#[test]
fn lobby_reset_clears_history_and_retained_names_but_game_over_does_not() {
    let mut app = app(8);
    let entity = app
        .world_mut()
        .spawn((EntityUuid(VICTIM_A.into()), EntityName("Old run".into())))
        .id();
    app.world_mut().run_schedule(FixedUpdate);
    app.world_mut()
        .resource_mut::<Messages<BalanceEvent>>()
        .write(damage(VICTIM_A, None, "region", 2.0));
    app.world_mut().run_schedule(FixedLast);
    assert_eq!(take(&mut app)[0].entries[0].victim.name, "Old run");

    // GameOver has no reset system: the post-run row remains readable.
    app.world_mut().run_schedule(FixedLast);
    assert!(take(&mut app).is_empty());
    assert_eq!(
        app.world()
            .resource::<GmActivityState>()
            .history
            .payload()
            .entries[0]
            .victim
            .name,
        "Old run"
    );

    app.world_mut().despawn(entity);
    app.world_mut().run_system_cached(reset_on_lobby).unwrap();
    app.world_mut().run_schedule(FixedLast);
    assert!(take(&mut app).pop().unwrap().entries.is_empty());

    app.world_mut()
        .resource_mut::<Messages<BalanceEvent>>()
        .write(damage(VICTIM_A, None, "region", 2.0));
    app.world_mut().run_schedule(FixedLast);
    assert_eq!(take(&mut app)[0].entries[0].victim.name, VICTIM_A);
}

#[test]
fn absent_browser_gm_ignores_events() {
    let mut app = app(8);
    app.world_mut().remove_resource::<BrowserGameMaster>();
    app.world_mut()
        .resource_mut::<Messages<BalanceEvent>>()
        .write(damage(VICTIM_A, None, "region", 2.0));
    app.world_mut().run_schedule(FixedLast);
    assert!(take(&mut app).is_empty());
}
