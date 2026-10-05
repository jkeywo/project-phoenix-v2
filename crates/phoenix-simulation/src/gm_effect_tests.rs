use super::*;
use crate::core::messages::SystemId;
use crate::ship::damage::SystemHull;

fn hull(systems: &[(&str, f32)]) -> SystemHull {
    SystemHull::from_config(
        &systems
            .iter()
            .map(|(id, max)| (SystemId((*id).into()), *max))
            .collect::<Vec<_>>(),
    )
}

/// A ship config whose `[[system]]` rows carry the authored Station
/// ownership a narrowed scope resolves against (issue #1311).
///
/// Parsed from TOML rather than built from struct literals so the
/// ownership under test is the SAME field a shipped hull authors, read the
/// same way — a hand-built `SystemInstanceConfig` would let this test pass
/// while `[[system]] station = "..."` meant something else.
fn ship_config(systems: &[(&str, Option<&str>)]) -> crate::ship::config::ShipConfig {
    let mut toml = String::new();
    let mut stations: Vec<&str> = systems.iter().filter_map(|(_, station)| *station).collect();
    stations.sort_unstable();
    stations.dedup();
    for station in stations {
        toml.push_str(&format!(
            r#"
[[station]]
id = "{station}"
name = "station.{station}.display_name"
description = "station.{station}.description"
rank = "Lieutenant"
"#
        ));
    }
    for (system, station) in systems {
        toml.push_str(&format!(
            r#"
[[system]]
id = "{system}"
kind = "{system}"
"#
        ));
        if let Some(station) = station {
            toml.push_str(&format!("station = \"{station}\"\n"));
        }
    }
    toml::from_str(&toml).expect("a well-formed authoring fixture")
}

/// Every answer a scope can give about which Systems it names, in one
/// place — because this is the function BOTH the reducer and the damage
/// phase call, and a disagreement between them is exactly what "Station
/// scope never affects a System outside its authored ownership" forbids.
#[test]
fn a_scope_names_only_the_systems_its_authoring_gives_it() {
    let target = hull(&[
        ("impulse-drive", 60.0),
        ("phaser-bank", 40.0),
        ("core", 30.0),
    ]);
    let config = ship_config(&[
        ("impulse-drive", Some("helm")),
        ("phaser-bank", Some("tactical")),
        // Authored under no Station at all, the courier's ownerless bucket.
        ("core", None),
        // Authored but NOT tracked by the hull — every Alliance radar is
        // one. It must not widen `helm`'s scope by being owned by it.
        ("nav-radar", Some("helm")),
    ]);

    assert_eq!(
        scope_systems(&GmDirectEffectScope::Entity, &target, Some(&config)),
        Ok(None),
        "the whole hull is the absence of a restriction, not a list of everything"
    );
    assert_eq!(
        scope_systems(
            &GmDirectEffectScope::Station(crate::core::messages::StationId("helm".into())),
            &target,
            Some(&config)
        ),
        Ok(Some(vec![SystemId("impulse-drive".into())])),
        "an authored System the hull does not track is not a damageable one"
    );
    assert_eq!(
        scope_systems(
            &GmDirectEffectScope::System(SystemId("phaser-bank".into())),
            &target,
            Some(&config)
        ),
        Ok(Some(vec![SystemId("phaser-bank".into())])),
    );
    assert_eq!(
        scope_systems(
            &GmDirectEffectScope::System(SystemId("core".into())),
            &target,
            Some(&config)
        ),
        Ok(Some(vec![SystemId("core".into())])),
        "an ownerless System is still a legitimate System-scope target"
    );
    assert_eq!(
        scope_systems(
            &GmDirectEffectScope::Station(crate::core::messages::StationId("science".into())),
            &target,
            Some(&config)
        ),
        Err(GmDirectEffectScopeError::UnknownStation),
    );
    assert_eq!(
        scope_systems(
            &GmDirectEffectScope::System(SystemId("nav-radar".into())),
            &target,
            Some(&config)
        ),
        Err(GmDirectEffectScopeError::UnknownSystem),
        "authored but undamageable and absent are the same honest answer"
    );
    assert_eq!(
        scope_systems(
            &GmDirectEffectScope::Station(crate::core::messages::StationId("helm".into())),
            &target,
            None
        ),
        Err(GmDirectEffectScopeError::UnknownStation),
        "a target with no ship config authors no Station, so it has none"
    );
    assert_eq!(
        scope_systems(&GmDirectEffectScope::Entity, &target, None),
        Ok(None),
        "...and the whole hull never needed one"
    );

    // A Station that owns only systems this hull does not track is the
    // SCOPED spelling of an undamageable target, not an unknown Station.
    let radar_only = hull(&[("impulse-drive", 60.0)]);
    assert_eq!(
        scope_systems(
            &GmDirectEffectScope::Station(crate::core::messages::StationId("science".into())),
            &radar_only,
            Some(&ship_config(&[
                ("impulse-drive", Some("helm")),
                ("nav-radar", Some("science")),
            ]))
        ),
        Err(GmDirectEffectScopeError::NotDamageable),
    );
}

/// The clamp is the scope's and the lethality is the hull's, which is the
/// one thing a narrowed effect must not conflate: emptying a Station is
/// not sinking a ship (issue #1311).
#[test]
fn a_scoped_resolution_clamps_to_its_scope_but_kills_only_the_whole_hull() {
    // 25 asked of a Station holding 20, on a hull holding 60.
    let station =
        resolve_direct_effect_within(GmDirectEffectKind::Damage, 25_000, 20.0, 60.0, 60.0);
    assert_eq!(station.applied_milli_hp, 20_000, "clamped by the SCOPE");
    assert_eq!(station.discarded_milli_hp, 5_000);
    assert!(
        !station.destroyed,
        "40 points elsewhere on the hull are still alive"
    );

    // The same press when that Station holds every point the hull has left.
    let last = resolve_direct_effect_within(GmDirectEffectKind::Damage, 25_000, 20.0, 60.0, 20.0);
    assert_eq!(last.applied_milli_hp, 20_000);
    assert!(
        last.destroyed,
        "a scoped hit that empties the last living Systems IS the kill"
    );

    // Healing is never lethal whatever the totals say.
    assert!(
        !resolve_direct_effect_within(GmDirectEffectKind::Heal, 25_000, 0.0, 60.0, 0.0).destroyed
    );

    // The whole-hull path is exactly the scoped one with one total.
    for (requested, current, max) in [
        (25_000u32, 100.0f32, 100.0f32),
        (250_000, 40.0, 100.0),
        (1, 0.0, 100.0),
    ] {
        assert_eq!(
            resolve_direct_effect(GmDirectEffectKind::Damage, requested, current, max),
            resolve_direct_effect_within(
                GmDirectEffectKind::Damage,
                requested,
                current,
                max,
                current
            ),
            "`Entity` scope must have no second spelling"
        );
    }
}

/// A scoped total measures the scope, not the hull it sits in — which is
/// what makes the clamp, the overflow figure and the lethality preview
/// answer the question the operator actually asked.
#[test]
fn scope_totals_measure_the_scope_rather_than_the_hull() {
    let mut target = hull(&[("impulse-drive", 60.0), ("phaser-bank", 40.0)]);
    target.set_hp(&SystemId("impulse-drive".into()), 20.0);
    assert_eq!(scope_totals(&target, None), (60.0, 100.0));
    assert_eq!(
        scope_totals(&target, Some(&[SystemId("impulse-drive".into())])),
        (20.0, 60.0)
    );
    assert_eq!(
        scope_totals(&target, Some(&[SystemId("phaser-bank".into())])),
        (40.0, 40.0)
    );
}

#[test]
fn damage_within_the_hull_is_applied_whole_and_discards_nothing() {
    let result = resolve_direct_effect(GmDirectEffectKind::Damage, 25_000, 100.0, 100.0);
    assert_eq!(
        result,
        GmDirectEffectResult {
            kind: GmDirectEffectKind::Damage,
            applied_milli_hp: 25_000,
            discarded_milli_hp: 0,
            destroyed: false,
        }
    );
}

#[test]
fn damage_beyond_the_remaining_hull_is_lethal_and_reports_the_overflow() {
    let result = resolve_direct_effect(GmDirectEffectKind::Damage, 250_000, 100.0, 100.0);
    assert_eq!(
        result,
        GmDirectEffectResult {
            kind: GmDirectEffectKind::Damage,
            applied_milli_hp: 100_000,
            discarded_milli_hp: 150_000,
            destroyed: true,
        }
    );
}

#[test]
fn damage_exactly_equal_to_the_remaining_hull_is_lethal_with_no_overflow() {
    let result = resolve_direct_effect(GmDirectEffectKind::Damage, 100_000, 100.0, 100.0);
    assert!(result.destroyed);
    assert_eq!(result.discarded_milli_hp, 0);
}

#[test]
fn healing_clamps_at_the_maxima_and_reports_the_discarded_remainder() {
    let mut target = hull(&[("helm", 100.0), ("power", 100.0)]);
    target.set_hp(&SystemId("helm".into()), 40.0);
    let result = resolve_direct_effect(
        GmDirectEffectKind::Heal,
        250_000,
        target.total_current(),
        target.total_max(),
    );
    assert_eq!(
        result,
        GmDirectEffectResult {
            kind: GmDirectEffectKind::Heal,
            applied_milli_hp: 60_000,
            discarded_milli_hp: 190_000,
            destroyed: false,
        }
    );
}

#[test]
fn healing_an_undamaged_hull_applies_nothing_and_discards_everything() {
    let result = resolve_direct_effect(GmDirectEffectKind::Heal, 10_000, 100.0, 100.0);
    assert_eq!(result.applied_milli_hp, 0);
    assert_eq!(result.discarded_milli_hp, 10_000);
    assert!(!result.destroyed);
}

#[test]
fn damaging_an_already_destroyed_hull_applies_nothing_and_is_not_a_second_kill() {
    let result = resolve_direct_effect(GmDirectEffectKind::Damage, 5_000, 0.0, 100.0);
    assert_eq!(result.applied_milli_hp, 0);
    assert_eq!(result.discarded_milli_hp, 5_000);
    assert!(!result.destroyed);
}

/// A hull with no systems at all has no totals to work with; the reducer
/// refuses such a target outright, and the resolution agrees.
#[test]
fn a_hull_with_no_systems_absorbs_nothing_at_all() {
    let empty = SystemHull::default();
    let result = resolve_direct_effect(
        GmDirectEffectKind::Damage,
        1_000,
        empty.total_current(),
        empty.total_max(),
    );
    assert_eq!(result.applied_milli_hp, 0);
    assert_eq!(result.discarded_milli_hp, 1_000);
}

/// A hull holding a fractional remainder still keeps the promise: a result
/// that says `destroyed` resolves an amount that actually empties it, and
/// the damage phase agrees.
#[test]
fn a_lethal_resolution_covers_a_fractional_remainder() {
    let result = resolve_direct_effect(GmDirectEffectKind::Damage, 40_000, 30.0004, 100.0);
    assert!(result.destroyed);
    assert_eq!(result.applied_milli_hp, 30_001);
    assert!(
        milli_to_hp(result.applied_milli_hp) >= 30.0004,
        "the clamp rounds UP so nothing is left behind a lethal promise"
    );

    let mut target = hull(&[("helm", 100.0)]);
    target.set_hp(&SystemId("helm".into()), 30.0004);
    let (_, destroyed) = crate::ship::damage::apply_hull_damage(
        &mut target,
        milli_to_hp(result.applied_milli_hp),
        &mut crate::sim_rng::unseeded_test_rng(),
    );
    assert!(destroyed);
}

/// The projection a reducer walks forward: what one effect leaves for the
/// next grant on the same target in the same canonical drain.
#[test]
fn totals_after_an_effect_are_what_the_next_grant_measures_against() {
    let hit = resolve_direct_effect(GmDirectEffectKind::Damage, 60_000, 100.0, 100.0);
    assert_eq!(totals_after(&hit, 100.0, 100.0), (40.0, 100.0));
    let heal = resolve_direct_effect(GmDirectEffectKind::Heal, 60_000, 40.0, 100.0);
    assert_eq!(totals_after(&heal, 40.0, 100.0), (100.0, 100.0));
}

/// The reducer can run twice before the damage phase does — a paused
/// session is exactly that — so a second press must measure against what
/// the arms already waiting will take.
#[test]
fn already_armed_effects_are_part_of_what_the_next_press_measures_against() {
    let mut pending = PendingGmDirectEffects::default();
    pending.push(PendingGmDirectEffect {
        tick: 1,
        order: GmActionOrder::new(crate::command_admission::HostSlot(1), 1),
        target: "npc-1".into(),
        scope: GmDirectEffectScope::Entity,
        kind: GmDirectEffectKind::Damage,
        amount_milli_hp: 60_000,
    });
    let target = hull(&[("helm", 100.0)]);
    assert_eq!(
        pending.project_totals(
            "npc-1",
            None,
            &target,
            None,
            GmDirectEffectKind::Damage,
            100.0,
            100.0
        ),
        (40.0, 100.0)
    );
    assert_eq!(
        pending.project_totals(
            "npc-1",
            None,
            &target,
            None,
            GmDirectEffectKind::Heal,
            100.0,
            100.0
        ),
        (40.0, 100.0),
        "a whole-hull arm is CONTAINED by a whole-hull press, so it is exact \
             for a heal press too — the kind only suppresses arms wider than \
             the scope being measured"
    );
    assert_eq!(
        pending.project_totals(
            "npc-2",
            None,
            &target,
            None,
            GmDirectEffectKind::Damage,
            100.0,
            100.0
        ),
        (100.0, 100.0),
        "another hull's queue is not this one's"
    );
}

#[test]
fn the_queue_drains_due_effects_in_canonical_order_and_retains_the_future() {
    let effect = |tick: u64, sequence: u64| PendingGmDirectEffect {
        tick,
        order: GmActionOrder::new(crate::command_admission::HostSlot(1), sequence),
        target: format!("uuid-{sequence}"),
        scope: GmDirectEffectScope::Entity,
        kind: GmDirectEffectKind::Damage,
        amount_milli_hp: 1_000,
    };
    let mut pending = PendingGmDirectEffects::default();
    pending.push(effect(4, 3));
    pending.push(effect(2, 2));
    pending.push(effect(9, 1));

    let due = pending.take_due(4);
    assert_eq!(
        due.iter().map(|e| e.order.sequence).collect::<Vec<_>>(),
        vec![2, 3]
    );
    assert_eq!(pending.entries().len(), 1);
    assert_eq!(pending.entries()[0].order.sequence, 1);
}

#[test]
fn milli_hp_round_trips_a_typed_amount() {
    assert_eq!(hp_to_milli(25.0), 25_000);
    assert!((milli_to_hp(25_000) - 25.0).abs() < f32::EPSILON);
    assert_eq!(hp_to_milli(-4.0), 0);
    assert_eq!(hp_to_milli(f32::NAN), 0);
}

// -- The ordinary damage-phase applier -----------------------------------

fn damage_app(tick: u64, seed: u64) -> App {
    let mut app = App::new();
    app.insert_resource(crate::sim_tick::SimTick(tick))
        .insert_sim_rng(crate::sim_rng::SimRng::new(
            seed,
            crate::sim_rng::SeedSource::Cli,
        ))
        .init_resource::<PendingGmDirectEffects>()
        .init_resource::<crate::server_app::GameOverReason>()
        .init_resource::<bevy::ecs::message::Messages<crate::core::balance::BalanceEvent>>()
        .init_resource::<bevy::ecs::message::Messages<crate::ai::server::AiEntityDestroyed>>()
        .add_systems(Update, apply_gm_direct_effects);
    app
}

fn spawn_hull(app: &mut App, uuid: &str, systems: &[(&str, f32)], fleet: bool) -> Entity {
    let hull = crate::entities::spawner::EntitySystemHull(hull(systems));
    let mut entity = app
        .world_mut()
        .spawn((crate::entities::spawner::EntityUuid(uuid.into()), hull));
    if fleet {
        entity.insert(crate::lockstep::FleetSlotOf(
            crate::command_admission::HostSlot(1),
        ));
    }
    entity.id()
}

/// [`spawn_hull`] plus the `ShipConfigComponent` a narrowed scope resolves
/// its Station ownership against (issue #1311).
fn spawn_stationed_hull(
    app: &mut App,
    uuid: &str,
    systems: &[(&str, f32, Option<&str>)],
) -> Entity {
    let entity = spawn_hull(
        app,
        uuid,
        &systems
            .iter()
            .map(|(id, max, _)| (*id, *max))
            .collect::<Vec<_>>(),
        false,
    );
    let config = ship_config(
        &systems
            .iter()
            .map(|(id, _, station)| (*id, *station))
            .collect::<Vec<_>>(),
    );
    app.world_mut()
        .entity_mut(entity)
        .insert(crate::ship::components::ShipConfigComponent(config));
    entity
}

/// Attach the per-arc hull pool a crewed hull's `[[shield_arc]]` blocks
/// give it (issue #514) — `alliance_courier.toml` declares two.
fn attach_arc_hull(app: &mut App, entity: Entity, arcs: &[(&str, f32)]) {
    let arc_hull = crate::ship::damage::ShipArcHull::from_entries(
        arcs.iter()
            .map(|(id, max)| {
                (
                    (*id).to_string(),
                    crate::ship::damage::ArcHullEntry {
                        current: *max,
                        max: *max,
                        tier_config: crate::ship::damage::ConsoleTierConfig::default(),
                    },
                )
            })
            .collect(),
    );
    app.world_mut()
        .entity_mut(entity)
        .insert(crate::entities::spawner::EntityShipArcHull(arc_hull));
}

fn arc_total(app: &App, entity: Entity) -> f32 {
    app.world()
        .entity(entity)
        .get::<crate::entities::spawner::EntityShipArcHull>()
        .expect("a live arc pool")
        .0
        .iter()
        .map(|(_, entry)| entry.current)
        .sum()
}

fn arm(
    app: &mut App,
    sequence: u64,
    tick: u64,
    target: &str,
    kind: GmDirectEffectKind,
    amount_milli_hp: u32,
) {
    arm_scoped(
        app,
        sequence,
        tick,
        target,
        GmDirectEffectScope::Entity,
        kind,
        amount_milli_hp,
    );
}

#[allow(clippy::too_many_arguments)]
fn arm_scoped(
    app: &mut App,
    sequence: u64,
    tick: u64,
    target: &str,
    scope: GmDirectEffectScope,
    kind: GmDirectEffectKind,
    amount_milli_hp: u32,
) {
    app.world_mut()
        .resource_mut::<PendingGmDirectEffects>()
        .push(PendingGmDirectEffect {
            tick,
            order: GmActionOrder::new(crate::command_admission::HostSlot(1), sequence),
            target: target.into(),
            scope,
            kind,
            amount_milli_hp,
        });
}

fn station(id: &str) -> GmDirectEffectScope {
    GmDirectEffectScope::Station(crate::core::messages::StationId(id.into()))
}

fn system(id: &str) -> GmDirectEffectScope {
    GmDirectEffectScope::System(SystemId(id.into()))
}

fn total_current(app: &App, entity: Entity) -> f32 {
    app.world()
        .entity(entity)
        .get::<crate::entities::spawner::EntitySystemHull>()
        .expect("a live hull")
        .0
        .total_current()
}

/// Pre-damage one System so a scoped repair has somewhere to go.
fn set_system_hp(app: &mut App, entity: Entity, id: &str, hp: f32) {
    let mut target = app.world_mut().entity_mut(entity);
    let mut hull = target
        .get_mut::<crate::entities::spawner::EntitySystemHull>()
        .expect("a live hull");
    hull.0.set_hp(&SystemId(id.into()), hp);
}

/// The live HP of one System, which is how a scoped test states "this one
/// moved and that one did not" without inferring it from a total.
fn system_current(app: &App, entity: Entity, id: &str) -> f32 {
    app.world()
        .entity(entity)
        .get::<crate::entities::spawner::EntitySystemHull>()
        .expect("a live hull")
        .0
        .get(&SystemId(id.into()))
        .expect("a tracked system")
        .current
}

fn balance_events(app: &mut App) -> Vec<crate::core::balance::BalanceEvent> {
    let messages = app
        .world_mut()
        .resource_mut::<bevy::ecs::message::Messages<crate::core::balance::BalanceEvent>>();
    messages.iter_current_update_messages().cloned().collect()
}

/// A direct hit lands the resolved amount, bypasses nothing but shields
/// (there are none here to bypass), and reports through the SAME structured
/// balance event a beam hit reports through -- which is what makes the
/// crew-facing damage row appear with no extra plumbing.
#[test]
fn a_direct_hit_lands_the_armed_amount_and_reports_an_ordinary_damage_event() {
    let mut app = damage_app(4, 77);
    let target = spawn_hull(&mut app, "npc-1", &[("helm", 60.0), ("power", 40.0)], false);
    arm(&mut app, 1, 4, "npc-1", GmDirectEffectKind::Damage, 25_000);
    app.update();

    assert!((total_current(&app, target) - 75.0).abs() < 0.01);
    let events = balance_events(&mut app);
    assert!(events.iter().any(|event| matches!(
        event,
        crate::core::balance::BalanceEvent::DamageApplied {
            attacker: None,
            weapon,
            hull_damage,
            shield_absorbed,
            ..
        } if weapon == WEAPON_KIND_GM_DIRECT
            && (*hull_damage - 25.0).abs() < 0.01
            && *shield_absorbed == 0.0
    )));
    assert!(app.world().resource::<PendingGmDirectEffects>().is_empty());
}

/// The acceptance criterion itself, measured at the DAMAGE PHASE rather
/// than at the reducer: a Station-scoped hit larger than everything that
/// Station owns empties exactly those Systems and leaves every sibling
/// untouched — no weighted selection, no float-precision fallback and no
/// spill can reach one (issue #1311).
///
/// The overrun is deliberate. A hit sized to fit would pass even if the
/// allow-list were consulted only for the FIRST draw; only an amount the
/// scope cannot absorb exercises the spill loop, which is the step that
/// would otherwise walk into a sibling.
#[test]
fn a_station_scoped_hit_empties_its_own_systems_and_never_reaches_a_sibling() {
    let mut app = damage_app(4, 77);
    let target = spawn_stationed_hull(
        &mut app,
        "npc-1",
        &[
            ("impulse-drive", 40.0, Some("helm")),
            ("manoeuvre-thrusters", 20.0, Some("helm")),
            ("phaser-bank", 40.0, Some("tactical")),
            ("core", 30.0, None),
        ],
    );
    arm_scoped(
        &mut app,
        1,
        4,
        "npc-1",
        station("helm"),
        GmDirectEffectKind::Damage,
        250_000,
    );
    app.update();

    assert_eq!(system_current(&app, target, "impulse-drive"), 0.0);
    assert_eq!(system_current(&app, target, "manoeuvre-thrusters"), 0.0);
    assert_eq!(
        system_current(&app, target, "phaser-bank"),
        40.0,
        "another Station's System is invisible to a Station-scoped walk"
    );
    assert_eq!(
        system_current(&app, target, "core"),
        30.0,
        "and so is a System no Station owns"
    );
    let events = balance_events(&mut app);
    assert!(
        events.iter().any(|event| matches!(
            event,
            crate::core::balance::BalanceEvent::DamageApplied { weapon, hull_damage, .. }
                if weapon == WEAPON_KIND_GM_DIRECT && (*hull_damage - 60.0).abs() < 0.01
        )),
        "the ordinary structured damage event still reports what LANDED"
    );
    assert!(
        !events.iter().any(|event| matches!(
            event,
            crate::core::balance::BalanceEvent::EntityDestroyed { .. }
        )),
        "70 hull points are still alive, so the entity is not destroyed"
    );
}

/// System scope never spills to a sibling — the same rule with a scope of
/// exactly one, where "spill" would be the whole rest of the hull.
#[test]
fn a_system_scoped_hit_stops_at_its_own_system() {
    let mut app = damage_app(4, 4242);
    let target = spawn_stationed_hull(
        &mut app,
        "npc-1",
        &[
            ("impulse-drive", 40.0, Some("helm")),
            ("manoeuvre-thrusters", 20.0, Some("helm")),
            ("phaser-bank", 40.0, Some("tactical")),
        ],
    );
    arm_scoped(
        &mut app,
        1,
        4,
        "npc-1",
        system("impulse-drive"),
        GmDirectEffectKind::Damage,
        100_000,
    );
    app.update();

    assert_eq!(system_current(&app, target, "impulse-drive"), 0.0);
    assert_eq!(
        system_current(&app, target, "manoeuvre-thrusters"),
        20.0,
        "a sibling on the SAME Station is still outside a System scope"
    );
    assert_eq!(system_current(&app, target, "phaser-bank"), 40.0);
    assert!((total_current(&app, target) - 60.0).abs() < 0.01);
}

/// Healing mirrors scope: a Station repair fills its own Systems and
/// cannot quietly refill another Station's, and it still clamps at each
/// maximum rather than overfilling one to spend the amount.
#[test]
fn a_station_scoped_repair_mirrors_the_scope_and_clamps_at_the_maxima() {
    let mut app = damage_app(4, 99);
    let target = spawn_stationed_hull(
        &mut app,
        "npc-1",
        &[
            ("impulse-drive", 40.0, Some("helm")),
            ("manoeuvre-thrusters", 20.0, Some("helm")),
            ("phaser-bank", 40.0, Some("tactical")),
        ],
    );
    set_system_hp(&mut app, target, "impulse-drive", 0.0);
    set_system_hp(&mut app, target, "manoeuvre-thrusters", 5.0);
    set_system_hp(&mut app, target, "phaser-bank", 1.0);
    arm_scoped(
        &mut app,
        1,
        4,
        "npc-1",
        station("helm"),
        GmDirectEffectKind::Heal,
        250_000,
    );
    app.update();

    assert_eq!(
        system_current(&app, target, "impulse-drive"),
        40.0,
        "a System at zero has the largest headroom and is revived first"
    );
    assert_eq!(system_current(&app, target, "manoeuvre-thrusters"), 20.0);
    assert_eq!(
        system_current(&app, target, "phaser-bank"),
        1.0,
        "another Station's wreck is not repaired by this Station's repair"
    );
}

/// A Station-scoped hit that empties the LAST living Systems still ends the
/// entity: destruction is a fact about the whole hull, which is why the
/// scope narrows the distribution and not the question.
#[test]
fn a_scoped_hit_that_empties_the_last_living_systems_still_destroys_the_entity() {
    let mut app = damage_app(4, 7);
    let target = spawn_stationed_hull(
        &mut app,
        "npc-1",
        &[
            ("impulse-drive", 40.0, Some("helm")),
            ("phaser-bank", 40.0, Some("tactical")),
        ],
    );
    set_system_hp(&mut app, target, "phaser-bank", 0.0);
    arm_scoped(
        &mut app,
        1,
        4,
        "npc-1",
        station("helm"),
        GmDirectEffectKind::Damage,
        40_000,
    );
    app.update();

    assert!(
        balance_events(&mut app).iter().any(|event| matches!(
            event,
            crate::core::balance::BalanceEvent::EntityDestroyed { victim, .. }
                if victim == "npc-1"
        )),
        "the ordinary destruction path still runs for a scoped killing blow"
    );
}

/// An arm whose scope this hull can no longer resolve — its ship config
/// replaced between the apply tick and the damage phase — is dropped
/// exactly as a vanished target is, rather than silently widening back to
/// the whole hull.
#[test]
fn an_arm_whose_scope_no_longer_resolves_is_dropped_rather_than_widened() {
    let mut app = damage_app(4, 11);
    let target = spawn_stationed_hull(
        &mut app,
        "npc-1",
        &[
            ("impulse-drive", 40.0, Some("helm")),
            ("phaser-bank", 40.0, Some("tactical")),
        ],
    );
    arm_scoped(
        &mut app,
        1,
        4,
        "npc-1",
        station("science"),
        GmDirectEffectKind::Damage,
        40_000,
    );
    app.update();

    assert!((total_current(&app, target) - 80.0).abs() < 0.01);
    assert!(app.world().resource::<PendingGmDirectEffects>().is_empty());
}

/// Two GMs pressing overlapping scopes on one boundary. The narrower press
/// measures against what the wider arm will take, bounded at zero, so its
/// durable result can never promise more than the hull will honour — and a
/// press on a DISJOINT scope is unaffected by either.
///
/// Round-1 review (#1311): an arm WIDER than the press only lands an
/// unknown fraction inside it, so it may be projected only in the direction
/// that shrinks the press's headroom. Same kind shrinks it; the OPPOSITE
/// kind would grow it, on points the applier may never deliver, so it is
/// skipped — the two mixed-kind cases at the end of this test.
#[test]
fn an_overlapping_arm_is_what_the_next_scoped_press_measures_against() {
    let target = hull(&[
        ("impulse-drive", 40.0),
        ("manoeuvre-thrusters", 20.0),
        ("phaser-bank", 40.0),
    ]);
    let config = ship_config(&[
        ("impulse-drive", Some("helm")),
        ("manoeuvre-thrusters", Some("helm")),
        ("phaser-bank", Some("tactical")),
    ]);
    let helm = scope_systems(&station("helm"), &target, Some(&config)).unwrap();
    let drive = scope_systems(&system("impulse-drive"), &target, Some(&config)).unwrap();
    let phaser = scope_systems(&system("phaser-bank"), &target, Some(&config)).unwrap();

    let mut pending = PendingGmDirectEffects::default();
    pending.push(PendingGmDirectEffect {
        tick: 1,
        order: GmActionOrder::new(crate::command_admission::HostSlot(1), 1),
        target: "npc-1".into(),
        scope: station("helm"),
        kind: GmDirectEffectKind::Damage,
        amount_milli_hp: 50_000,
    });

    assert_eq!(
        pending.project_totals(
            "npc-1",
            helm.as_deref(),
            &target,
            Some(&config),
            GmDirectEffectKind::Damage,
            60.0,
            60.0
        ),
        (10.0, 60.0),
        "the same scope is exact: 50 of this Station's 60 points are spoken for"
    );
    assert_eq!(
        pending.project_totals(
            "npc-1",
            drive.as_deref(),
            &target,
            Some(&config),
            GmDirectEffectKind::Damage,
            40.0,
            40.0
        ),
        (0.0, 40.0),
        "a scope INSIDE the arm cannot know which of its Systems absorbs it, \
             so it takes the conservative answer and never over-promises"
    );
    assert_eq!(
        pending.project_totals(
            "npc-1",
            phaser.as_deref(),
            &target,
            Some(&config),
            GmDirectEffectKind::Damage,
            40.0,
            40.0
        ),
        (40.0, 40.0),
        "a DISJOINT scope is untouched by the arm — this is the whole point \
             of resolving overlap rather than subtracting every queued effect"
    );
    assert_eq!(
        pending.project_totals(
            "npc-1",
            None,
            &target,
            Some(&config),
            GmDirectEffectKind::Damage,
            100.0,
            100.0
        ),
        (50.0, 100.0),
        "the whole hull overlaps everything, exactly as it did before scopes"
    );

    // A WIDER arm of the OPPOSITE kind (#1311 round-1 review). Only the
    // damage phase decides how much of a whole-hull effect lands inside one
    // Station, so projecting it whole here would move the narrow press's
    // headroom the UNSAFE way — up — and the durable result would promise
    // hull points the world never lands.
    let mut healed = PendingGmDirectEffects::default();
    healed.push(PendingGmDirectEffect {
        tick: 1,
        order: GmActionOrder::new(crate::command_admission::HostSlot(1), 1),
        target: "npc-1".into(),
        scope: GmDirectEffectScope::Entity,
        kind: GmDirectEffectKind::Heal,
        amount_milli_hp: 40_000,
    });
    assert_eq!(
        healed.project_totals(
            "npc-1",
            helm.as_deref(),
            &target,
            Some(&config),
            GmDirectEffectKind::Damage,
            10.0,
            60.0
        ),
        (10.0, 60.0),
        "a whole-hull HEAL arm must not raise a Station DAMAGE press's \
             projected current: the heal may land every one of its 40 points \
             outside this Station, and a press clamped to 50 would report an \
             amount the helm systems cannot absorb"
    );
    assert_eq!(
        healed.project_totals(
            "npc-1",
            helm.as_deref(),
            &target,
            Some(&config),
            GmDirectEffectKind::Heal,
            10.0,
            60.0
        ),
        (50.0, 60.0),
        "the same wider arm against a HEAL press shrinks the headroom, \
             which is the safe direction, so it still counts whole"
    );

    let mut hit = PendingGmDirectEffects::default();
    hit.push(PendingGmDirectEffect {
        tick: 1,
        order: GmActionOrder::new(crate::command_admission::HostSlot(1), 1),
        target: "npc-1".into(),
        scope: GmDirectEffectScope::Entity,
        kind: GmDirectEffectKind::Damage,
        amount_milli_hp: 40_000,
    });
    assert_eq!(
        hit.project_totals(
            "npc-1",
            helm.as_deref(),
            &target,
            Some(&config),
            GmDirectEffectKind::Heal,
            10.0,
            60.0
        ),
        (10.0, 60.0),
        "a whole-hull DAMAGE arm must not grow a Station HEAL press's \
             headroom: the damage may land entirely on another Station, and \
             the healed points would then be discarded by the hull"
    );
    assert_eq!(
        hit.project_totals(
            "npc-1",
            helm.as_deref(),
            &target,
            Some(&config),
            GmDirectEffectKind::Damage,
            10.0,
            60.0
        ),
        (0.0, 60.0),
        "against a DAMAGE press the same arm shrinks the headroom, so it \
             counts whole and bounds at zero"
    );
}

/// The per-arc hull pool (issue #514) follows a GM hit exactly as it
/// follows a beam, a torpedo, a collision and a damage zone.
///
/// `EntityShipArcHull` tracks TOTAL hull damage taken, it is authoritative
/// snapshot state, and `sync_console_damage_tiers` derives the
/// `shield-arc-<id>` offline entries from it — so a GM hit that moved
/// `EntitySystemHull` alone would leave the arc tiers stuck at whatever
/// they read before the hit. Both pools move by the SAME applied amount.
#[test]
fn a_direct_hit_moves_the_per_arc_hull_pool_by_the_same_amount() {
    let mut app = damage_app(4, 4242);
    let target = spawn_hull(
        &mut app,
        "player-1",
        &[("helm", 60.0), ("power", 40.0)],
        true,
    );
    attach_arc_hull(&mut app, target, &[("fore", 50.0), ("aft", 50.0)]);
    arm(
        &mut app,
        1,
        4,
        "player-1",
        GmDirectEffectKind::Damage,
        25_000,
    );
    app.update();

    assert!((total_current(&app, target) - 75.0).abs() < 0.01);
    assert!(
        (arc_total(&app, target) - 75.0).abs() < 0.01,
        "the arc pool absorbs the same 25 points the system hull did"
    );
}

/// Healing does NOT touch the arc pool, and that is deliberate: no repair
/// path the crew can reach restores arc hull, and `ShipArcHull` has no
/// distributed restore at all. GM healing behaves like every other repair
/// rather than inventing a distribution of its own.
#[test]
fn a_direct_heal_leaves_the_per_arc_hull_pool_where_the_damage_left_it() {
    let mut app = damage_app(4, 7);
    let target = spawn_hull(
        &mut app,
        "player-1",
        &[("helm", 60.0), ("power", 40.0)],
        true,
    );
    attach_arc_hull(&mut app, target, &[("fore", 50.0), ("aft", 50.0)]);
    arm(
        &mut app,
        1,
        4,
        "player-1",
        GmDirectEffectKind::Damage,
        25_000,
    );
    app.update();
    arm(&mut app, 2, 4, "player-1", GmDirectEffectKind::Heal, 25_000);
    app.update();

    assert!((total_current(&app, target) - 100.0).abs() < 0.01);
    assert!(
        (arc_total(&app, target) - 75.0).abs() < 0.01,
        "arc hull only ever refills the way the ordinary repair path refills it"
    );
}

/// The crew feel a GM hit on their own hull the way they feel every other
/// hit: `DamageTaken` on the outbox drives the haptic pulse and the
/// forcefield audio spike. Presentation only, never folded — the gate is
/// the ONE `LocalShip` read in this system.
#[test]
fn a_direct_hit_on_the_local_hull_reports_ordinary_crew_hit_feedback() {
    let mut app = damage_app(4, 31);
    app.init_resource::<crate::server_app::SimOutbox>();
    let target = spawn_hull(
        &mut app,
        "player-1",
        &[("helm", 60.0), ("power", 40.0)],
        true,
    );
    app.world_mut()
        .entity_mut(target)
        .insert(crate::server_app::LocalShip);
    arm(
        &mut app,
        1,
        4,
        "player-1",
        GmDirectEffectKind::Damage,
        25_000,
    );
    app.update();

    let outbox = app.world().resource::<crate::server_app::SimOutbox>();
    assert!(
        outbox.iter().any(|(_, message)| matches!(
            message,
            ServerMessage::DamageTaken { hull, shield }
                if (*hull - 25.0).abs() < 0.01 && *shield == 0.0
        )),
        "a direct hit bypasses shields, so the crew feedback is all hull"
    );
}

/// A hull that is not this host's own crew's stays silent on the outbox.
///
/// The GM peer has no `LocalShip` at all, so this is also the shape every
/// GM peer sees for every target: the fold is identical on both, and only
/// the crew host emits the pulse.
#[test]
fn a_direct_hit_on_another_hull_reports_no_crew_hit_feedback() {
    let mut app = damage_app(4, 31);
    app.init_resource::<crate::server_app::SimOutbox>();
    spawn_hull(&mut app, "npc-1", &[("helm", 60.0), ("power", 40.0)], false);
    arm(&mut app, 1, 4, "npc-1", GmDirectEffectKind::Damage, 25_000);
    app.update();

    let outbox = app.world().resource::<crate::server_app::SimOutbox>();
    assert!(
        !outbox
            .iter()
            .any(|(_, message)| matches!(message, ServerMessage::DamageTaken { .. })),
        "another hull's damage is not this crew's hit feedback"
    );
}

/// The distribution is the ordinary weighted one: the same seed and the
/// same canonical order land on the same systems on every peer, while the
/// seed genuinely moves WHICH system absorbs the hit. The TOTAL never
/// moves — that is resolved before the draw, which is why the durable
/// result can promise it.
#[test]
fn the_distribution_is_seeded_but_the_total_is_not() {
    let spread = |seed: u64, sequence: u64| {
        let mut app = damage_app(1, seed);
        let target = spawn_hull(
            &mut app,
            "npc-1",
            &[("helm", 50.0), ("power", 50.0), ("shields", 50.0)],
            false,
        );
        arm(
            &mut app,
            sequence,
            1,
            "npc-1",
            GmDirectEffectKind::Damage,
            30_000,
        );
        app.update();
        app.world()
            .entity(target)
            .get::<crate::entities::spawner::EntitySystemHull>()
            .expect("a live hull")
            .0
            .entries()
            .map(|(_, current, _)| current)
            .collect::<Vec<_>>()
    };
    assert_eq!(spread(1234, 1), spread(1234, 1));

    let shapes: Vec<Vec<f32>> = (0..8u64).map(|seed| spread(seed, 1)).collect();
    for shape in &shapes {
        let total: f32 = shape.iter().sum();
        assert!(
            (total - 120.0).abs() < 0.01,
            "the resolved amount lands whole however it is distributed"
        );
    }
    assert!(
        shapes.iter().any(|shape| shape != &shapes[0]),
        "which system absorbs a GM hit is a function of the run seed"
    );
}

/// The "NPC scalar path" is the ordinary path applied to a hull of one.
///
/// `entities::spawner` turns a legacy scalar `hull_integrity` into a
/// single-entry `SystemHull` at spawn, so there is no second formula to
/// write and no second branch to take: the weighted walk has exactly one
/// candidate, and every hull point lands on it.
#[test]
fn a_single_entry_hull_takes_the_whole_hit_on_its_one_system() {
    let mut app = damage_app(2, 909);
    let target = spawn_hull(&mut app, "npc-1", &[("captain", 40.0)], false);
    arm(&mut app, 1, 2, "npc-1", GmDirectEffectKind::Damage, 15_000);
    app.update();

    let hull = app
        .world()
        .entity(target)
        .get::<crate::entities::spawner::EntitySystemHull>()
        .expect("a live hull");
    assert!((hull.0.current_for(&SystemId("captain".into())).unwrap() - 25.0).abs() < 0.01);
    assert!(!hull.0.is_destroyed());
}

/// Healing is the mirror: it fills, clamps at the maxima, and can bring a
/// destroyed system back -- the one route out of `Destroyed` there is.
#[test]
fn a_direct_heal_refills_the_hull_and_revives_a_destroyed_system() {
    let mut app = damage_app(2, 5);
    let target = spawn_hull(&mut app, "npc-1", &[("helm", 40.0), ("power", 60.0)], false);
    app.world_mut()
        .entity_mut(target)
        .get_mut::<crate::entities::spawner::EntitySystemHull>()
        .expect("a live hull")
        .0
        .set_hp(&SystemId("helm".into()), 0.0);
    arm(&mut app, 1, 2, "npc-1", GmDirectEffectKind::Heal, 40_000);
    app.update();

    let hull = app
        .world()
        .entity(target)
        .get::<crate::entities::spawner::EntitySystemHull>()
        .expect("a live hull");
    assert!((hull.0.total_current() - 100.0).abs() < 0.01);
    assert_eq!(
        hull.0.tier_for(&SystemId("helm".into())),
        crate::ship::damage::DamageTier::Operational
    );
}

/// A non-crewed kill follows the ordinary NPC destruction path: despawn,
/// `AiEntityDestroyed`, `EntityDespawned` and the structured destruction
/// event a balance ledger and the GM feed both already read.
#[test]
fn a_lethal_direct_hit_despawns_an_npc_through_the_ordinary_destruction_path() {
    let mut app = damage_app(3, 11);
    app.init_resource::<crate::server_app::SimOutbox>();
    app.init_resource::<crate::server_app::TrackedEntities>();
    let target = spawn_hull(&mut app, "npc-1", &[("helm", 20.0)], false);
    arm(&mut app, 1, 3, "npc-1", GmDirectEffectKind::Damage, 20_000);
    app.update();

    assert!(app.world().get_entity(target).is_err(), "the NPC despawned");
    let events = balance_events(&mut app);
    assert!(events.iter().any(|event| matches!(
        event,
        crate::core::balance::BalanceEvent::EntityDestroyed { victim, killer: None }
            if victim == "npc-1"
    )));
    assert_eq!(
        app.world_mut()
            .resource_mut::<bevy::ecs::message::Messages<crate::ai::server::AiEntityDestroyed>>()
            .iter_current_update_messages()
            .map(|event| event.entity_uuid.clone())
            .collect::<Vec<_>>(),
        vec!["npc-1".to_string()]
    );
}

/// A crewed hull is NEVER despawned: the run ends and the report reads from
/// the wreck. Keyed on fleet membership, never `LocalShip` -- the GM peer
/// that pressed the button has no local ship at all.
#[test]
fn a_lethal_direct_hit_on_a_fleet_hull_ends_the_run_without_despawning_it() {
    let mut app = damage_app(6, 3);
    let target = spawn_hull(&mut app, "player-1", &[("helm", 30.0)], true);
    arm(
        &mut app,
        1,
        6,
        "player-1",
        GmDirectEffectKind::Damage,
        30_000,
    );
    app.update();

    assert!(app.world().get_entity(target).is_ok(), "the wreck remains");
    let reason = app.world().resource::<crate::server_app::GameOverReason>();
    assert_eq!(reason.0.as_deref(), Some("server.game_over.ship_destroyed"));
    assert_eq!(reason.1, Some(crate::core::balance::Outcome::Defeat));
}

/// An arm whose target left the world between the apply boundary and the
/// damage phase is dropped, not retried and not panicked over.
#[test]
fn an_arm_whose_target_vanished_is_dropped() {
    let mut app = damage_app(8, 2);
    arm(&mut app, 1, 8, "gone", GmDirectEffectKind::Damage, 1_000);
    app.update();
    assert!(app.world().resource::<PendingGmDirectEffects>().is_empty());
}

/// A future boundary is not this tick's business.
#[test]
fn an_effect_armed_for_a_later_tick_is_retained() {
    let mut app = damage_app(1, 2);
    let target = spawn_hull(&mut app, "npc-1", &[("helm", 50.0)], false);
    arm(&mut app, 1, 9, "npc-1", GmDirectEffectKind::Damage, 10_000);
    app.update();
    assert!((total_current(&app, target) - 50.0).abs() < f32::EPSILON);
    assert_eq!(
        app.world()
            .resource::<PendingGmDirectEffects>()
            .entries()
            .len(),
        1
    );
}
