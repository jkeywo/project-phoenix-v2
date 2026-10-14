use super::*;
use crate::ai::server::AiHighFidelity;
use crate::console::weapons::TacticalRadarSelection;
use crate::core::messages::{AdmittedCommands, CoordinationPayload};
use crate::ship::control_source::ControlSource;
use crate::ship::shields::{
    PendingShieldsThreatBearing, ShieldsAiConfigResource, ShieldsDamageHistory,
    ShieldsFocusAiPolicy, ShipShields,
};
use crate::ship_plugin::{
    CoordinationEnqueue, DeliveredCoordination, ShipConfigComponent, ShipSystemControlSources,
};

#[derive(Resource, Default)]
struct CoordBox(Vec<CoordinationEnqueue>);

fn collect_coord(mut reader: MessageReader<CoordinationEnqueue>, mut box_: ResMut<CoordBox>) {
    for m in reader.read() {
        box_.0.push(m.clone());
    }
}

/// `ai_shield_focus` normally reads the 0-based tick being executed in
/// `FixedUpdate`. These focused fixtures deliberately host the production
/// system in `Update`, so advance the explicit authoritative counter only after
/// every Update-based decision has observed it.
fn advance_shield_fixture_tick(mut sim_tick: ResMut<crate::sim_tick::SimTick>) {
    sim_tick.0 += 1;
}

/// Registers `ai_shield_focus` (decide + admitted emit) chained before
/// the shields module's `handle_shields_messages` (the single applier for
/// human and AI commands, issue #826) — the production pipeline minus
/// `AdmissionPlugin`'s per-tick clear, which these single-shot scenarios
/// don't need.
fn shield_test_app() -> App {
    let config = crate::weapons::shield::ShieldConfig {
        num_facings: 4,
        max_hp: 100,
        regen_per_sec: 0.0,
        offline_duration: 10.0,
    };
    let mut app = App::new();
    let mut ship_config = crate::ship::components::load_ship_config_from_disk();
    crate::ship::test_support::add_default_shield_arc_systems(&mut ship_config.0);
    let control_sources = ai_shield_control_sources(&ship_config.0);
    let mut world_config = crate::world::config::WorldConfig::default();
    world_config.global.sim_tick_hz = 60.0;
    crate::ai::host::register_ai_host_env(&mut app);
    app.add_plugins(bevy::time::TimePlugin)
        .insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
            std::time::Duration::from_millis(100),
        ))
        .insert_resource(crate::sim_tick::SimTick::default())
        .insert_resource(world_config)
        .init_resource::<crate::ai::server::WorldSnapshot>()
        .init_resource::<ShieldsAiConfigResource>()
        .init_resource::<CoordBox>()
        .insert_resource(crate::lobby::Sessions(
            crate::lobby::session::SessionManager::new(),
        ))
        .add_message::<CoordinationEnqueue>()
        .add_message::<DeliveredCoordination>()
        .add_systems(
            Update,
            (
                ai_shield_focus.before(crate::ship::shields::handle_shields_messages),
                crate::ship::shields::handle_shields_messages,
                crate::ship::shields::receive_shields_coordination
                    .after(crate::ship::shields::handle_shields_messages),
            ),
        )
        .add_systems(PostUpdate, collect_coord)
        .add_systems(Last, advance_shield_fixture_tick);

    app.world_mut().spawn((
        crate::server_app::Ship,
        ShipShields(crate::weapons::shield::ShieldSystem::new(&config), 0.5),
        ShieldsDamageHistory::default(),
        PendingShieldsThreatBearing::default(),
        control_sources,
        ship_config,
        AdmittedCommands::default(),
        AiHighFidelity,
        default_focus_policy(),
    ));

    app
}

#[test]
fn authored_shield_windows_convert_to_expected_whole_ticks() {
    assert_eq!(shield_window_ticks(0.0, 60.0), 0);
    assert_eq!(
        shield_window_ticks(0.001, 60.0),
        1,
        "a positive sub-tick window includes the current tick"
    );
    assert_eq!(
        shield_window_ticks(0.6, 60.0),
        36,
        "f32-authored 0.6 seconds must not widen into a spurious 37th tick"
    );
    assert_eq!(
        shield_window_ticks(0.61, 60.0),
        37,
        "a fractional final tick rounds up instead of shortening the window"
    );
}

/// Mimics `AdmissionPlugin`'s per-tick clear of every ship's
/// `AdmittedCommands` for multi-tick shield scenarios. Production clears
/// admitted commands each tick in Input before the AI (Physics) refills
/// them; without it, focus/clear commands would pile up across ticks and a
/// stale `focused: true` could out-vote a later `focused: false`.
/// Scheduled `.before(ai_shield_focus)` so the AI still refills same-tick.
fn clear_admitted_each_tick(mut q: Query<&mut AdmittedCommands>) {
    for mut a in q.iter_mut() {
        a.0.clear();
    }
}

/// The authored Shields capability (the focus-decision gate) + every
/// synthesised `shield_arc` fine system (the admission gates) set to AI,
/// matching the entity spawner's Backfill roster without inventing system ids.
fn ai_shield_control_sources(
    ship_config: &crate::ship::config::ShipConfig,
) -> ShipSystemControlSources {
    let mut control_sources = ShipSystemControlSources::default();
    for system in ship_config.systems.iter().filter(|system| {
        matches!(
            system.kind.as_str(),
            crate::ship::system_registry::SHIELDS_KIND
                | crate::ship::system_registry::SHIELD_ARC_KIND
        )
    }) {
        control_sources.0.set(system.id.clone(), ControlSource::Ai);
    }
    control_sources
}

/// The canonical default Shields focus policy (issue #783) — reproduces
/// today's decisions, kernel and all. Bare-`App` fixtures may omit the
/// component (the host falls back to this same policy), but attaching it
/// explicitly documents the wiring and lets a test swap in an authored one.
fn default_focus_policy() -> ShieldsFocusAiPolicy {
    ShieldsFocusAiPolicy(
        crate::entities::authored_ai_pins::shipped_policy_toml("shields_focus")
            .to_policy()
            .unwrap(),
    )
}

/// A Shields focus policy whose health-imbalance fallback threshold is `pct`
/// (0–100), every other authored number left at the default. Proves
/// per-entity policy `param`s drive the decision (issue #783).
fn focus_policy_with_health_ratio(pct: f32) -> ShieldsFocusAiPolicy {
    let mut cfg = crate::entities::authored_ai_pins::shipped_policy_toml("shields_focus");
    cfg.param.insert(
        crate::entities::config::SHIELD_FOCUS_HEALTH_RATIO_PARAM.to_string(),
        pct,
    );
    ShieldsFocusAiPolicy(cfg.to_policy().unwrap())
}

/// A Shields focus policy that declares an explicit idle — the host must
/// take no AI focus action regardless of damage (issue #783 gate).
fn idle_focus_policy() -> ShieldsFocusAiPolicy {
    ShieldsFocusAiPolicy(crate::ai::policy::AiPolicy {
        idle: true,
        ..Default::default()
    })
}

/// A Shields focus policy with a SINGLE rule guarded on the seeded
/// `recent_damage_total` fact and NO unconditional fallback — so the retained
/// kernel runs only when the bounded recent-damage fact clears the gate.
/// Proves the `fact(...)` guard actually fires (facts are seeded, closing the
/// #779 empty-facts sharp edge).
fn damage_only_focus_policy() -> ShieldsFocusAiPolicy {
    let mut cfg = crate::entities::authored_ai_pins::shipped_policy_toml("shields_focus");
    cfg.param.insert("min_recent_damage".to_string(), 0.0);
    cfg.rule = vec![crate::entities::config::FineSystemAiRuleToml {
        priority: 10,
        channel: crate::entities::config::SHIELD_FOCUS_CHANNEL.to_string(),
        when: "fact(recent_damage_total) > param(min_recent_damage)".to_string(),
        verb: crate::entities::config::SHIELD_FOCUS_VERB.to_string(),
        value: false,
        level: 0,
        response_index: 0,
    }];
    ShieldsFocusAiPolicy(cfg.to_policy().unwrap())
}

/// A Shields focus policy whose ONLY rule is guarded on a world flag — the
/// #891 stage 2 read surface — with no unconditional fallback.
fn flag_only_focus_policy() -> ShieldsFocusAiPolicy {
    let mut cfg = crate::entities::authored_ai_pins::shipped_policy_toml("shields_focus");
    cfg.rule = vec![crate::entities::config::FineSystemAiRuleToml {
        priority: 10,
        channel: crate::entities::config::SHIELD_FOCUS_CHANNEL.to_string(),
        when: "flag(brace_for_impact)".to_string(),
        verb: crate::entities::config::SHIELD_FOCUS_VERB.to_string(),
        value: false,
        level: 0,
        response_index: 0,
    }];
    ShieldsFocusAiPolicy(cfg.to_policy().unwrap())
}

/// Issue #891 stage 2, per-host both-directions proof for the Shields
/// focus host: with heavy damage on facing 0 (the kernel's pick), a
/// `flag()`-gated policy holds while the scenario flag is clear and
/// focuses once it is set.
#[test]
fn ai_shield_focus_flag_guard_reads_the_world_in_both_directions() {
    let mut app = shield_test_app();
    app.init_resource::<crate::world::server::WorldContentRuntime>();
    let e = ship_entity(&mut app);
    app.world_mut()
        .entity_mut(e)
        .insert(flag_only_focus_policy());
    {
        let mut entity_mut = app.world_mut().entity_mut(e);
        let mut shields = entity_mut.get_mut::<ShipShields>().unwrap();
        shields.0.facings[0].hp = 20; // heavy damage to facing 0 only
    }

    // Flag CLEAR -> the gate reads false, the kernel never runs, no focus.
    app.update();
    assert_eq!(
        focused_facing(&app, e),
        None,
        "with the world flag clear the focus gate must read false and hold"
    );

    // Flag SET -> the SAME gate fires and the kernel focuses the weak facing.
    app.world_mut()
        .resource_mut::<crate::world::server::WorldContentRuntime>()
        .flags
        .set_flag("brace_for_impact");
    app.update();
    assert_eq!(
        focused_facing(&app, e),
        Some(0),
        "with the world flag set the same gate must fire and focus facing 0"
    );
}

fn focused_facing(app: &App, e: Entity) -> Option<usize> {
    app.world()
        .entity(e)
        .get::<ShipShields>()
        .unwrap()
        .0
        .focused_facing
}

fn ship_entity(app: &mut App) -> Entity {
    app.world_mut()
        .query_filtered::<Entity, With<ShipShields>>()
        .single(app.world())
        .unwrap()
}

#[test]
fn ai_shield_focus_emits_admitted_focus_toward_damaged_facing() {
    // Simulates an attacker's hit landing on facing 0 (a real attack
    // always lands on one specific facing — "toward the attacker" from
    // the acceptance criteria). `tick_shield_focus_ai`'s health-imbalance
    // branch focuses the critically-weak facing whenever no arc's damage
    // history clears the damage-concentration threshold, which is what
    // fires here since facing 0 (20/100 HP) is far below the others
    // (100/100 HP). This exercises the full ai_shield_focus ->
    // validate_and_admit -> handle_shields_messages pipeline end to end
    // (issue #826).
    let mut app = shield_test_app();
    let e = ship_entity(&mut app);

    {
        let mut entity_mut = app.world_mut().entity_mut(e);
        let mut shields = entity_mut.get_mut::<ShipShields>().unwrap();
        shields.0.facings[0].hp = 20; // heavy damage to facing 0 only
    }
    app.update();

    assert_eq!(
        focused_facing(&app, e),
        Some(0),
        "shield focus should follow the facing that took the attacker's damage \
         (ai_shield_focus decided, handle_shields_messages applied the admitted command)"
    );
}

#[test]
fn ai_emitted_focus_applies_to_npc_own_ship_shields_only() {
    // Two NPC ships, both AI-operated. Only ship A takes damage; the
    // admitted `SetShieldArcFocus` lands in A's own `AdmittedCommands`,
    // so only A's `ShipShields` gains a focus — B is untouched (the
    // per-entity admission routing from issue #824, applied to shields
    // by #826).
    let mut app = shield_test_app();
    let a = ship_entity(&mut app);

    let config = crate::weapons::shield::ShieldConfig {
        num_facings: 4,
        max_hp: 100,
        regen_per_sec: 0.0,
        offline_duration: 10.0,
    };
    let ship_config = app
        .world()
        .entity(a)
        .get::<ShipConfigComponent>()
        .expect("base shield fixture carries authored topology")
        .clone();
    let control_sources = ai_shield_control_sources(&ship_config.0);
    let b = app
        .world_mut()
        .spawn((
            crate::server_app::Ship,
            ShipShields(crate::weapons::shield::ShieldSystem::new(&config), 0.5),
            ShieldsDamageHistory::default(),
            PendingShieldsThreatBearing::default(),
            control_sources,
            ship_config,
            AdmittedCommands::default(),
            AiHighFidelity,
            default_focus_policy(),
        ))
        .id();

    {
        let mut entity_mut = app.world_mut().entity_mut(a);
        let mut shields = entity_mut.get_mut::<ShipShields>().unwrap();
        shields.0.facings[0].hp = 20;
    }
    app.update();

    assert_eq!(
        focused_facing(&app, a),
        Some(0),
        "the damaged NPC's own shields must gain the AI focus"
    );
    assert_eq!(
        focused_facing(&app, b),
        None,
        "the undamaged NPC must not be contaminated by another ship's AI command"
    );
}

#[test]
fn shield_ai_reads_its_own_policy_params_per_entity() {
    // Issue #783 isolation: the authored windows/thresholds now live in each
    // ship's own `ShieldsFocusAiPolicy` `param` map, read per-entity in the
    // host. A ship carrying a permissive 90% health-ratio param must focus a
    // 60/100 arc (0.6 < 0.9 · 1.0), while a ship on the default 50% policy
    // must not (0.6 < 0.5 · 1.0 is false) — proving one ship's authored
    // tuning never bleeds onto another (the #738 isolation guarantee, now
    // carried by the per-entity policy rather than a global Resource).
    let mut app = shield_test_app();
    // The base fixture ship carries the DEFAULT policy (50%).
    let defaulted = ship_entity(&mut app);

    let config = crate::weapons::shield::ShieldConfig {
        num_facings: 4,
        max_hp: 100,
        regen_per_sec: 0.0,
        offline_duration: 10.0,
    };
    let ship_config = app
        .world()
        .entity(defaulted)
        .get::<ShipConfigComponent>()
        .expect("base shield fixture carries authored topology")
        .clone();
    let control_sources = ai_shield_control_sources(&ship_config.0);
    // A second ship carrying the permissive tuning as its own policy param.
    let tuned = app
        .world_mut()
        .spawn((
            crate::server_app::Ship,
            ShipShields(crate::weapons::shield::ShieldSystem::new(&config), 0.5),
            ShieldsDamageHistory::default(),
            PendingShieldsThreatBearing::default(),
            control_sources,
            ship_config,
            AdmittedCommands::default(),
            AiHighFidelity,
            focus_policy_with_health_ratio(90.0),
        ))
        .id();

    for e in [defaulted, tuned] {
        let mut entity_mut = app.world_mut().entity_mut(e);
        let mut shields = entity_mut.get_mut::<ShipShields>().unwrap();
        shields.0.facings[0].hp = 60;
    }
    app.update();

    assert_eq!(
        focused_facing(&app, tuned),
        Some(0),
        "a ship carrying the permissive health-ratio param must focus the weak arc"
    );
    assert_eq!(
        focused_facing(&app, defaulted),
        None,
        "a ship on the default policy must not focus a 60/100 arc — one ship's \
         authored params must never bleed onto another"
    );
}

#[test]
fn human_held_shield_arc_rejects_ai_emission() {
    // The decide gate (coarse shields system) still says AI, but the
    // targeted arc's control source is Human — `validate_and_admit`
    // refuses the `ai:` token (`operate_ai` does not hold on the arc), so
    // no admitted command exists and the focus never flips. This is the
    // admission-refusal path the retired `integrate_shield_state` adapter
    // could not express (it applied intents unconditionally).
    let mut app = shield_test_app();
    let e = ship_entity(&mut app);
    {
        let mut entity_mut = app.world_mut().entity_mut(e);
        let mut cs = entity_mut.get_mut::<ShipSystemControlSources>().unwrap();
        for arc_id in ["fore", "port", "aft", "starboard"] {
            cs.0.set(
                crate::ship::system_registry::shield_arc_system_id(arc_id).expect("arc id"),
                ControlSource::Human,
            );
        }
    }

    {
        let mut entity_mut = app.world_mut().entity_mut(e);
        let mut shields = entity_mut.get_mut::<ShipShields>().unwrap();
        shields.0.facings[0].hp = 20;
    }
    app.update();

    assert_eq!(
        focused_facing(&app, e),
        None,
        "an ai: emission targeting a human-held shield arc must be refused at admission"
    );
    assert!(
        app.world()
            .entity(e)
            .get::<AdmittedCommands>()
            .unwrap()
            .0
            .is_empty(),
        "the refused command must never reach AdmittedCommands"
    );
}

#[test]
fn ai_shield_focus_detects_damage_concentration_without_health_imbalance() {
    // Regression test for a bug where damage-concentration detection was
    // dead code: `prev_hp` was derived from the last DamageRecord's
    // `amount` (a delta, not an HP value) instead of a real per-arc HP
    // baseline, so `facing.hp < prev_hp` could never be true on an arc's
    // first-ever hit — and since a record could therefore never be
    // created, it could never be true on any later hit either.
    //
    // This scenario deliberately keeps health imbalance below its
    // trigger threshold (facing 1 ends at 60/100 — normalized 0.6, not
    // below 0.5 * 1.0) so ONLY the damage-concentration branch can
    // produce a Focus decision. With the bug, this test fails (no
    // focus); with the fix, one recorded hit is enough.
    let mut app = shield_test_app();
    let e = ship_entity(&mut app);

    // Tick 1: establish the damage-history baseline (first observation
    // of this HP value never counts as damage).
    {
        let mut entity_mut = app.world_mut().entity_mut(e);
        let mut shields = entity_mut.get_mut::<ShipShields>().unwrap();
        shields.0.facings[1].hp = 90;
    }
    app.update();
    assert_eq!(
        focused_facing(&app, e),
        None,
        "the baseline-establishing tick must not itself register damage"
    );

    // Tick 2: a real hit lands on facing 1, dropping it further while
    // every other facing stays untouched — 100% of window damage on one
    // arc, but not enough absolute HP loss to trip health imbalance.
    {
        let mut entity_mut = app.world_mut().entity_mut(e);
        let mut shields = entity_mut.get_mut::<ShipShields>().unwrap();
        shields.0.facings[1].hp = 60;
    }
    app.update();

    assert_eq!(
        focused_facing(&app, e),
        Some(1),
        "damage-concentration detection must focus the arc that just took \
         a real hit, even when health imbalance alone would not trigger"
    );
}

#[test]
fn ai_shield_focus_accumulates_repeated_hits_on_one_arc_across_ticks() {
    // Issue #747: repeated hits on the same arc over separate ticks must
    // accumulate in that arc's damage history (not overwrite), so a stream
    // of small hits sums to a concentrated signal over the authored window.
    let mut app = shield_test_app();
    app.add_systems(Update, clear_admitted_each_tick.before(ai_shield_focus));
    let e = ship_entity(&mut app);

    // Tick 1: baseline observation (never counts as damage).
    app.update();
    assert_eq!(focused_facing(&app, e), None);

    // Tick 2: first small hit on facing 1 (100 -> 97). Health stays
    // balanced (0.97), so only concentration can drive a focus.
    {
        let mut em = app.world_mut().entity_mut(e);
        em.get_mut::<ShipShields>().unwrap().0.facings[1].hp = 97;
    }
    app.update();
    assert_eq!(
        focused_facing(&app, e),
        Some(1),
        "the first recorded hit should focus arc 1 by concentration"
    );

    // Tick 3: a second small hit on the same arc (97 -> 94). Both hits
    // must be retained in arc 1's window and keep the arc focused.
    {
        let mut em = app.world_mut().entity_mut(e);
        em.get_mut::<ShipShields>().unwrap().0.facings[1].hp = 94;
    }
    app.update();

    assert_eq!(
        focused_facing(&app, e),
        Some(1),
        "repeated hits on arc 1 must keep it focused"
    );
    let history = app.world().entity(e).get::<ShieldsDamageHistory>().unwrap();
    assert_eq!(
        history.arcs[1].len(),
        2,
        "both hits on arc 1 must accumulate as separate records in the window"
    );
    assert_eq!(
        history.arcs[1]
            .iter()
            .map(|record| record.recorded_tick)
            .collect::<Vec<_>>(),
        vec![1, 2],
        "the two records must carry the authoritative logical ticks they landed on"
    );
    let arc1_total: i32 = history.arcs[1].iter().map(|r| r.amount).sum();
    assert_eq!(
        arc1_total, 6,
        "accumulated window damage on arc 1 must be 3 + 3"
    );
}

#[test]
fn ai_shield_focus_reverts_when_concentrated_damage_expires() {
    // Issue #747: once the concentrated hit ages out of the authored
    // damage window (4s), the concentration signal disappears and, with
    // health balanced, the AI must clear the focus it took. `tick_shields`
    // is scheduled so non-focused arcs settle to their reduced cap (the
    // production steady state) rather than sitting above it forever.
    let mut app = shield_test_app();
    app.add_systems(Update, clear_admitted_each_tick.before(ai_shield_focus));
    app.add_systems(
        Update,
        crate::ship::shields::tick_shields.after(crate::ship::shields::handle_shields_messages),
    );
    let e = ship_entity(&mut app);

    // Tick 1: baseline.
    app.update();
    // Tick 2: one hit on facing 1 (100 -> 90) focuses it by concentration.
    {
        let mut em = app.world_mut().entity_mut(e);
        em.get_mut::<ShipShields>().unwrap().0.facings[1].hp = 90;
    }
    app.update();
    assert_eq!(
        focused_facing(&app, e),
        Some(1),
        "the concentrated hit should focus arc 1"
    );

    // Advance exactly the authored four-second window at the authoritative
    // 60 Hz tick rate. The hit was recorded at tick 1; on the last pass below
    // current_tick is 241, age is exactly 240, and strict `age < window`
    // expires it. The focus must then revert with balanced health.
    let window_ticks = shield_window_ticks(
        ShieldsAiConfigResource::default().damage_window_secs,
        app.world()
            .resource::<crate::world::config::WorldConfig>()
            .global
            .sim_tick_hz,
    );
    for _ in 0..window_ticks {
        app.update();
    }

    assert_eq!(
        focused_facing(&app, e),
        None,
        "focus must clear once the concentrated damage expires from the window"
    );
}

#[test]
fn ai_shield_focus_ignores_focus_decay_as_incoming_damage() {
    // Issue #747: focusing one arc reduces the others' effective max_hp, so
    // `tick_shields` bleeds those non-focused arcs down toward the reduced
    // cap. That HP drop is a focus side effect, not incoming fire — the
    // damage detector must NOT record it, or a decaying arc would steal the
    // focus. Here only arc 1 is ever hit; the decaying arcs must stay
    // record-free and never take the focus.
    let mut app = shield_test_app();
    app.add_systems(Update, clear_admitted_each_tick.before(ai_shield_focus));
    app.add_systems(
        Update,
        crate::ship::shields::tick_shields.after(crate::ship::shields::handle_shields_messages),
    );
    let e = ship_entity(&mut app);

    app.update(); // baseline
    {
        let mut em = app.world_mut().entity_mut(e);
        em.get_mut::<ShipShields>().unwrap().0.facings[1].hp = 90;
    }
    app.update(); // focus arc 1
    assert_eq!(focused_facing(&app, e), Some(1));

    // Let the non-focused arcs decay from 100 toward their reduced cap.
    for _ in 0..20 {
        app.update();
    }

    assert_eq!(
        focused_facing(&app, e),
        Some(1),
        "decay on non-focused arcs must not steal the focus from the hit arc"
    );
    let history = app.world().entity(e).get::<ShieldsDamageHistory>().unwrap();
    for idx in [0usize, 2, 3] {
        assert!(
            history.arcs[idx].is_empty(),
            "non-focused arc {idx} decaying toward its cap must record no incoming damage"
        );
    }
}

#[test]
fn ai_shield_focus_skips_ships_where_shields_are_not_ai_operated() {
    let mut app = shield_test_app();
    let e = ship_entity(&mut app);
    let shields_system_id = app
        .world()
        .entity(e)
        .get::<ShipConfigComponent>()
        .expect("fixture carries authored topology")
        .0
        .systems
        .iter()
        .find(|system| system.kind == crate::ship::system_registry::SHIELDS_KIND)
        .expect("fixture carries the authored Shields capability")
        .id
        .clone();
    app.world_mut()
        .entity_mut(e)
        .get_mut::<ShipSystemControlSources>()
        .unwrap()
        .0
        .set(shields_system_id, ControlSource::Human);

    app.update();
    {
        let mut entity_mut = app.world_mut().entity_mut(e);
        let mut shields = entity_mut.get_mut::<ShipShields>().unwrap();
        shields.0.facings[0].hp = 20;
    }
    app.update();

    assert_eq!(
        focused_facing(&app, e),
        None,
        "human-operated shields must not be focused by the AI decision system"
    );
}

#[test]
fn ai_shield_focus_threat_bearing_override_focuses_closest_facing_via_admission() {
    let mut app = shield_test_app();
    let e = ship_entity(&mut app);
    let address = crate::ship::coordination::address_for_system_kind(
        &app.world()
            .entity(e)
            .get::<ShipConfigComponent>()
            .expect("fixture carries the authored ship topology")
            .0,
        crate::ship::system_registry::SHIELD_ARC_KIND,
    )
    .expect("default player hull has a Station that owns its shield arcs");
    app.world_mut()
        .resource_mut::<Messages<DeliveredCoordination>>()
        .write(DeliveredCoordination {
            source_entity: e,
            address,
            payload: CoordinationPayload::ThreatBearing {
                bearing_rad: 90_f32.to_radians(),
                label: "test.threat.starboard".into(),
            },
            presentation: crate::core::messages::CoordinationPresentation::new(
                "test.coordination.title",
                "test.coordination.body",
            ),
            delivery: crate::ship::components::CoordinationDelivery::Ai,
        });

    // Production delivers in Modifiers, after the Shields AI's Physics pass.
    // This fixture preserves that order explicitly: update one lands the typed
    // handoff in the Shields-owned slot; update two consumes it through the
    // admitted-command path.
    app.update();
    assert_eq!(
        app.world()
            .entity(e)
            .get::<PendingShieldsThreatBearing>()
            .unwrap()
            .0,
        Some(90_f32.to_radians()),
        "Shields' receiver must preserve the typed bearing for the next decision"
    );
    app.update();

    assert_eq!(
        focused_facing(&app, e),
        Some(3),
        "a 90-degree bearing must focus the authored Starboard facing via the admitted-command path"
    );

    // The override takes priority over damage analysis and must consume
    // the pending bearing.
    assert_eq!(
        app.world()
            .entity(e)
            .get::<PendingShieldsThreatBearing>()
            .unwrap()
            .0,
        None,
        "pending threat bearing must be consumed (taken) once applied"
    );
}

#[test]
fn authored_focus_policy_drives_focus_via_its_params() {
    // An authored (non-default) policy carrying a permissive 90% health-ratio
    // param focuses a 60/100 arc the default 50% policy would leave alone —
    // observable proof the authored windows/thresholds route through the
    // policy `param` map into the retained kernel (issue #783 AC2/AC4).
    let mut app = shield_test_app();
    let e = ship_entity(&mut app);
    app.world_mut()
        .entity_mut(e)
        .insert(focus_policy_with_health_ratio(90.0));

    {
        let mut entity_mut = app.world_mut().entity_mut(e);
        let mut shields = entity_mut.get_mut::<ShipShields>().unwrap();
        shields.0.facings[0].hp = 60; // 0.6 < 0.9·1.0 → focus under 90%, not 50%
    }
    app.update();

    assert_eq!(
        focused_facing(&app, e),
        Some(0),
        "an authored permissive policy must focus the weak arc its params allow"
    );
}

#[test]
fn idle_focus_policy_takes_no_ai_focus_even_under_damage() {
    // The gate: an idle policy resolves the `shield_focus` channel to None,
    // so the host emits nothing even when an arc is heavily damaged and the
    // kernel would otherwise focus it (issue #783 AC4 idle opt-out).
    let mut app = shield_test_app();
    let e = ship_entity(&mut app);
    app.world_mut().entity_mut(e).insert(idle_focus_policy());

    {
        let mut entity_mut = app.world_mut().entity_mut(e);
        let mut shields = entity_mut.get_mut::<ShipShields>().unwrap();
        shields.0.facings[0].hp = 20; // heavy damage the default would focus
    }
    app.update();

    assert_eq!(
        focused_facing(&app, e),
        None,
        "an idle Shields focus policy must suppress all AI focus changes"
    );
}

#[test]
fn fact_guarded_focus_rule_fires_only_when_recent_damage_is_seeded() {
    // #779 empty-facts guard: a policy whose ONLY rule is guarded on the
    // seeded `recent_damage_total` fact (no unconditional fallback) must NOT
    // act on a quiet ship, but MUST act once a real hit seeds the fact —
    // proving `seed_shields_focus_facts` populates the window so a `fact(...)`
    // guard can fire at all.
    let mut app = shield_test_app();
    app.add_systems(Update, clear_admitted_each_tick.before(ai_shield_focus));
    let e = ship_entity(&mut app);
    app.world_mut()
        .entity_mut(e)
        .insert(damage_only_focus_policy());

    // Tick 1: baseline observation, no damage recorded yet — the fact-guarded
    // rule finds `recent_damage_total = 0` and does not fire.
    app.update();
    assert_eq!(
        focused_facing(&app, e),
        None,
        "with no recent damage the fact-guarded rule must not fire"
    );

    // Tick 2: a real hit on facing 1 seeds `recent_damage_total > 0`; the
    // guard fires, the kernel runs, and the hit arc is focused.
    {
        let mut entity_mut = app.world_mut().entity_mut(e);
        let mut shields = entity_mut.get_mut::<ShipShields>().unwrap();
        shields.0.facings[1].hp = 60;
    }
    app.update();
    assert_eq!(
        focused_facing(&app, e),
        Some(1),
        "a seeded recent-damage fact must let the guarded rule fire and focus the hit arc"
    );
}

// ── tick_frequency_hint_high_fidelity ─────────────────────────────────

/// Test-only glue (issue #829): seed each ship's viewscreen combat_lock from
/// its `TacticalRadarSelection` before the hint emitter reads the frozen
/// fact — standing in for the radar publisher + viewscreen aggregator the
/// full app runs, exactly like the other frequency/firing test harnesses.
fn seed_viewscreen_from_selection(
    mut q: Query<
        (
            Option<&crate::console::weapons::TacticalRadarSelection>,
            &mut crate::server_app::ShipSystemBlackboards,
        ),
        With<crate::server_app::Ship>,
    >,
) {
    for (tac, mut bbs) in q.iter_mut() {
        let combat_lock = tac.and_then(|t| t.0.clone());
        let mut vbb = match bbs
            .0
            .get(&crate::ship::system_registry::viewscreen_system_id())
        {
            Some(crate::core::messages::SystemBlackboard::Viewscreen(v)) => v.clone(),
            _ => crate::core::messages::ViewscreenBlackboard::default(),
        };
        vbb.combat_lock = combat_lock;
        bbs.0.insert(
            crate::ship::system_registry::viewscreen_system_id(),
            crate::core::messages::SystemBlackboard::Viewscreen(vbb),
        );
    }
}

fn freq_hint_test_app() -> App {
    let mut app = App::new();
    crate::ai::host::register_ai_host_env(&mut app);
    // Manual `Time::advance_by` (mirroring `ai::server`'s LOD tests)
    // rather than `TimePlugin` + `TimeUpdateStrategy`: the latter reports
    // a zero delta on the frame it's added, which would otherwise force
    // every test here to burn an extra warm-up `app.update()`.
    app.insert_resource(Time::<()>::default())
        .init_resource::<crate::ship::sensors::SensorsAiConfigResource>()
        .init_resource::<CoordBox>()
        .add_message::<CoordinationEnqueue>()
        .add_systems(
            Update,
            (
                seed_viewscreen_from_selection,
                tick_frequency_hint_high_fidelity,
            )
                .chain(),
        )
        .add_systems(PostUpdate, collect_coord);

    let mut control_sources = ShipSystemControlSources::default();
    control_sources.0.set(
        crate::ship::system_registry::sensors_system_id(),
        ControlSource::Ai,
    );

    let target = app
        .world_mut()
        .spawn((
            crate::entities::spawner::EntityUuid("target-1".into()),
            ShipShields(crate::weapons::shield::ShieldSystem::default(), 0.75),
        ))
        .id();

    let source = app
        .world_mut()
        .spawn((
            crate::server_app::Ship,
            control_sources,
            crate::ship_plugin::ShipConfigComponent(sensors_ship_config()),
            crate::server_app::ShipSystemBlackboards::default(),
            TacticalRadarSelection(Some("target-1".into())),
            ShipFrequencyHintState::default(),
            AiHighFidelity,
        ))
        .id();

    let _ = target;
    app.insert_resource(SourceShip(source));
    app
}

#[derive(Resource)]
struct SourceShip(Entity);

/// Advance the hint by `secs` of AI-TICK time.
///
/// Issue #889: the hint emitter runs under `run_if(ai_tick_ready)` and
/// advances its delay by one authored tick period per run, not by
/// `Time::delta` — otherwise the gate would stretch the authored delay by
/// the frame-rate-to-tick-rate ratio. So the fixture now drives AI TICKS
/// rather than one oversized wall-clock jump: `secs` of hint time is
/// `secs * ai_tick_hz` updates. Wall-clock is advanced alongside purely so
/// any other `Time` reader in the harness sees a consistent world.
fn tick_with_dt(app: &mut App, secs: f32) {
    let hz = crate::entities::config::GlobalConfig::default().ai_tick_hz;
    let period = 1.0 / hz;
    let ticks = (secs * hz).ceil().max(1.0) as usize;
    for _ in 0..ticks {
        let mut time = app.world_mut().resource_mut::<Time>();
        time.advance_by(std::time::Duration::from_secs_f32(period));
        app.update();
    }
}

#[test]
fn frequency_hint_propagates_after_the_authored_reaction_delay() {
    let mut app = freq_hint_test_app();
    // 4s exceeds the 3s default delay in a single tick.
    tick_with_dt(&mut app, 4.0);

    let coord = &app.world().resource::<CoordBox>().0;
    let hint = coord
        .iter()
        .find(|m| matches!(&m.payload, CoordinationPayload::FrequencyHint { .. }))
        .expect("expected a FrequencyHint CoordinationEnqueue after the delay elapses");

    match &hint.payload {
        CoordinationPayload::FrequencyHint { frequency } => {
            assert!(
                (*frequency - 0.75).abs() < f32::EPSILON,
                "hint should carry the locked target's shield frequency"
            );
        }
        other => panic!("expected FrequencyHint, got {other:?}"),
    }
    assert_eq!(
        hint.address,
        crate::core::messages::CoordinationAddress::Station(crate::core::messages::StationId(
            crate::ship::system_registry::TACTICAL_STATION_ID.into(),
        ),),
        "frequency hint should address Tactical"
    );
}

#[test]
fn npc_frequency_hint_reads_its_own_tuning_not_the_global_resource() {
    // Issue #738 isolation, mirroring the shields case: the hint emitter
    // used to resolve its delay as
    // `per_entity_component.unwrap_or(&*global_resource)` while iterating
    // every ship, so any write to that Resource would have applied
    // fleet-wide. Nothing writes it today (unlike the shields Resource,
    // which `server_app` dual-writes from the player ship) — this test
    // seeds it by hand so the leak stays closed if anything ever does.
    //
    // The global Resource here carries an eager 0.5s delay; one tick of
    // 1.0s therefore fires under the global tuning but not under the
    // parse-time default (3.0s).
    let mut app = freq_hint_test_app();
    let npc = app.world().resource::<SourceShip>().0;
    app.insert_resource(crate::ship::sensors::SensorsAiConfigResource {
        frequency_hint_delay_secs: 0.5,
    });

    let mut tuned_sources = ShipSystemControlSources::default();
    tuned_sources.0.set(
        crate::ship::system_registry::sensors_system_id(),
        ControlSource::Ai,
    );
    let tuned = app
        .world_mut()
        .spawn((
            crate::server_app::Ship,
            tuned_sources,
            crate::ship_plugin::ShipConfigComponent(sensors_ship_config()),
            crate::server_app::ShipSystemBlackboards::default(),
            TacticalRadarSelection(Some("target-1".into())),
            ShipFrequencyHintState::default(),
            AiHighFidelity,
            crate::ship::sensors::SensorsAiConfigResource {
                frequency_hint_delay_secs: 0.5,
            },
        ))
        .id();

    tick_with_dt(&mut app, 1.0);

    let coord = &app.world().resource::<CoordBox>().0;
    let hinting_ships: Vec<Entity> = coord
        .iter()
        .filter(|m| matches!(&m.payload, CoordinationPayload::FrequencyHint { .. }))
        .map(|m| m.source_entity)
        .collect();
    assert!(
        hinting_ships.contains(&tuned),
        "a ship carrying the eager 0.5s delay on its own entity must hint after 1.0s"
    );
    assert!(
        !hinting_ships.contains(&npc),
        "an NPC without its own sensors-AI tuning must fall back to the parse-time \
         3.0s default, never to the global Resource holding the player ship's tuning"
    );
}

/// Issue #873, replacing `ai_frequency_hint_skips_ships_where_sensors_are
/// _not_ai_operated`.
///
/// That test pinned the branch this issue exists to delete: the emitter
/// stood down whenever a human held Sensors, so the ship's frequency
/// advisory came from the AI operator path rather than from authoritative
/// state. Its premise is gone, so it is re-pointed at the rule that
/// replaced it — the same fact, from the same state, for a human-held
/// console — with a strictly stronger assertion: not merely that something
/// is emitted, but that it carries the human origin as a routing TAG.
///
/// `sender_origin == Human` is the whole point. It proves the emitter read
/// the control source (so the tag is live, not a hardcoded `Ai` the way
/// `tick_power_brownout_advisory` used to stamp one) while proving the
/// value did not gate the emission.
#[test]
fn frequency_hint_fires_from_a_human_held_sensors_station_and_tags_it_human() {
    let mut app = freq_hint_test_app();
    let source = app.world().resource::<SourceShip>().0;
    app.world_mut()
        .entity_mut(source)
        .get_mut::<ShipSystemControlSources>()
        .unwrap()
        .0
        .set(
            crate::ship::system_registry::sensors_system_id(),
            ControlSource::Human,
        );

    tick_with_dt(&mut app, 4.0);

    let coord = &app.world().resource::<CoordBox>().0;
    let hint = coord
        .iter()
        .find(|m| matches!(&m.payload, CoordinationPayload::FrequencyHint { .. }))
        .expect(
            "a human-held Sensors console must still feed the ship's coordination bus \
             — the fact comes from authoritative state, not from who is sitting there",
        );
    assert_eq!(
        hint.sender_origin,
        ControlSource::Human,
        "sender_origin must report the live control source, and be used only as a \
         delivery-routing tag"
    );
    match &hint.payload {
        CoordinationPayload::FrequencyHint { frequency } => assert!(
            (*frequency - 0.75).abs() < f32::EPSILON,
            "the human-sent hint must carry the same authoritative shield frequency \
             the AI-sent one does"
        ),
        other => panic!("expected FrequencyHint, got {other:?}"),
    }
}

// ── The retired `auto_hint` rating gate (issue #873) ────────────────────
//
// These two tests used to pin a claimed/unclaimed split copied from
// `ai_torpedo_auto_fire`: once a human session held Sensors, the hint
// additionally required that holder's active rating to declare `auto_hint`
// in its `ai_tuning` table, and stayed silent otherwise.
//
// That is a coordination fact whose emission turned on the presence of a
// human, which AGENTS.md rule 6 forbids and issue #873 removes. Both are
// kept — the fixture is exactly the interesting one — and re-pointed at the
// surviving rule: the rating table is now irrelevant to emission in BOTH
// directions, which takes two tests to state and could not be stated by
// deleting either.

fn sensors_ship_config() -> crate::ship::config::ShipConfig {
    let toml = r#"
[[station]]
id = "sensors"
name = "Sensors"
description = "Long-range sensors."
rank = "Ens."

[[station.rating]]
name = "Assisted"
automated_systems = []
[station.rating.ai_tuning]
auto_hint = {}

[[station.rating]]
name = "Std"
automated_systems = []

[[station]]
id = "tactical"
name = "Tactical"
description = "Weapons control."
rank = "Ltn."

[[console]]
id = "science-console"
home_station = "sensors"
screen = "gui/battleship/sensors.html"
[[console.feed]]
source = "viewscreen"
kind = "Viewscreen"

[[console]]
id = "weapons-console"
home_station = "tactical"
screen = "gui/battleship/tactical.html"

[[system]]
id = "sensors"
kind = "sensors"
console = "science-console"

[[system]]
id = "tactical-radar"
kind = "tactical_radar"
console = "weapons-console"
"#;
    crate::ship::config::ShipConfig::from_toml(toml, &["sensors", "tactical_radar"]).unwrap()
}

#[test]
fn console_combat_lock_requires_the_reading_systems_exact_feed() {
    let mut config = sensors_ship_config();
    let sensors = crate::ship::system_registry::sensors_system_id();
    let tactical = crate::ship::system_registry::tactical_radar_system_id();
    let mut boards = crate::server_app::ShipSystemBlackboards::default();
    boards.0.insert(
        crate::ship::system_registry::viewscreen_system_id(),
        crate::core::messages::SystemBlackboard::Viewscreen(
            crate::core::messages::ViewscreenBlackboard {
                combat_lock: Some("target-1".into()),
                ..Default::default()
            },
        ),
    );
    assert_eq!(
        console_combat_lock(&config, &sensors, &boards).as_deref(),
        Some("target-1")
    );
    // Co-hosting is not a feed, even when both Console home seats match.
    config.consoles[1].home_station = config.consoles[0].home_station.clone();
    assert_eq!(console_combat_lock(&config, &tactical, &boards), None);
    config.consoles[0].feeds[0].kind = phoenix_model::console::ConsoleObservationKind::Sensors;
    assert_eq!(console_combat_lock(&config, &sensors, &boards), None);
}

#[test]
fn frequency_hint_cannot_read_an_unbound_combat_lock() {
    let mut app = freq_hint_test_app();
    let source = app.world().resource::<SourceShip>().0;
    app.world_mut()
        .entity_mut(source)
        .get_mut::<ShipConfigComponent>()
        .unwrap()
        .0
        .consoles[0]
        .feeds
        .clear();
    tick_with_dt(&mut app, 10.0);
    assert!(
        app.world().resource::<CoordBox>().0.is_empty(),
        "a live target elsewhere on the ship does not grant Sensors its lock"
    );
}

/// Adds `ShipConfigComponent` + `ActiveStationRatings` to the ship spawned
/// by `freq_hint_test_app`, and a `Sessions` resource with the Sensors
/// station claimed by `holder_token`. Returns the source ship entity.
fn claim_sensors_station(app: &mut App, holder_token: &str, rating: &str) -> Entity {
    let source = app.world().resource::<SourceShip>().0;
    let sensors_station = crate::core::messages::StationId("sensors".into());

    let mut sm = crate::lobby::session::SessionManager::new();
    sm.register(holder_token.into(), "Operator".into()).unwrap();
    sm.set_station(holder_token, Some(sensors_station.clone()));
    app.insert_resource(crate::lobby::Sessions(sm));

    let mut active_ratings = crate::ship_plugin::ActiveStationRatings::default();
    active_ratings.0.insert(sensors_station, rating.into());

    app.world_mut()
        .entity_mut(source)
        .insert(crate::ship_plugin::ShipConfigComponent(
            sensors_ship_config(),
        ))
        .insert(active_ratings);

    source
}

#[test]
fn frequency_hint_fires_when_a_claimed_station_rating_declares_auto_hint() {
    let mut app = freq_hint_test_app();
    claim_sensors_station(&mut app, "op1", "Assisted");

    tick_with_dt(&mut app, 4.0);

    let coord = &app.world().resource::<CoordBox>().0;
    assert!(
        coord
            .iter()
            .any(|m| matches!(&m.payload, CoordinationPayload::FrequencyHint { .. })),
        "a claimed Sensors station whose active rating declares auto_hint \
         must be hinted"
    );
}

/// The half that changed. `"Std"` is a rating with no `ai_tuning` table at
/// all, held by a live session — the configuration that used to silence the
/// ship's frequency advisory completely.
#[test]
fn frequency_hint_fires_when_a_claimed_station_rating_lacks_auto_hint() {
    let mut app = freq_hint_test_app();
    claim_sensors_station(&mut app, "op1", "Std");

    tick_with_dt(&mut app, 4.0);

    let coord = &app.world().resource::<CoordBox>().0;
    assert!(
        coord
            .iter()
            .any(|m| matches!(&m.payload, CoordinationPayload::FrequencyHint { .. })),
        "a station rating's ai_tuning table must not decide whether a coordination \
         fact derived from authoritative state is emitted at all (issue #873): a human \
         on a rating without auto_hint still feeds the ship's backfilled Tactical"
    );
}

#[test]
fn frequency_hint_fires_unconditionally_when_sensors_station_is_unclaimed() {
    let mut app = freq_hint_test_app();
    let source = app.world().resource::<SourceShip>().0;

    // Ship config + ratings present, but no session holds the station —
    // e.g. an NPC, or the player ship before anyone takes Sensors.
    app.insert_resource(crate::lobby::Sessions(
        crate::lobby::session::SessionManager::new(),
    ));
    app.world_mut()
        .entity_mut(source)
        .insert(crate::ship_plugin::ShipConfigComponent(
            sensors_ship_config(),
        ))
        .insert(crate::ship_plugin::ActiveStationRatings::default());

    tick_with_dt(&mut app, 4.0);

    let coord = &app.world().resource::<CoordBox>().0;
    assert!(
        coord
            .iter()
            .any(|m| matches!(&m.payload, CoordinationPayload::FrequencyHint { .. })),
        "an unclaimed Sensors station must be hinted unconditionally, \
         regardless of any rating's ai_tuning table"
    );
}

// Power tree hosts have independent finite-hardware/knowledge fixtures.
#[path = "power_ai_tests.rs"]
mod power_ai_tests;

// ── AI torpedo loading (admitted-command path) ──────────────────

/// One tube, `volley_max = 2`, no per-tube AI override — so its resolved
/// `ai_target_count` is `volley_max`.
fn torpedo_load_app(tube_source: ControlSource) -> (App, Entity) {
    let mut app = App::new();
    crate::ai::host::register_ai_host_env(&mut app);
    // `Sessions` because the emit goes through the admission seam
    // (`emit_ai_command`), which asks it about station tenure.
    app.insert_resource(crate::lobby::Sessions(
        crate::lobby::session::SessionManager::new(),
    ))
    .add_systems(
        Update,
        (
            ai_torpedo_load,
            crate::console::weapons::handle_set_torpedo_volley_target,
        )
            .chain(),
    );

    let mut control_sources = ShipSystemControlSources::default();
    control_sources.0.set(
        crate::ship::system_registry::torpedo_magazine_system_id(),
        ControlSource::Ai,
    );
    control_sources.0.set(
        crate::ship::system_registry::torpedo_tube_system_id("fore_port").unwrap(),
        tube_source,
    );

    let torpedoes = crate::weapons::torpedo::TorpedoSystem::from_configs(
        &[crate::entities::config::TorpedoTubeConfig {
            id: "fore_port".into(),
            facing_deg: 0.0,
            fire_arc_deg: 90.0,
            load_time: None,
            marker: None,
            barrels: Vec::new(),
            pattern: Vec::new(),
            volley_max: 2,
            ai_target_count: None,
            ai: None,
        }],
        crate::weapons::torpedo::TorpedoConfig::default(),
    );

    let e = app
        .world_mut()
        .spawn((
            crate::server_app::Ship,
            control_sources,
            crate::console::weapons::TorpedoSystemResource(torpedoes),
            AdmittedCommands::default(),
            // The SHIPPED authored per-tube policy. Since #885b stage 5d a
            // tube with no entry in `TorpedoTubeAiPolicies` is never ordered
            // to load — there is no synthesised stand-in.
            crate::console::weapons::TorpedoTubeAiPolicies(
                [(
                    "fore_port".to_string(),
                    crate::entities::authored_ai_pins::shipped_policy_toml("torpedo_tube")
                        .to_policy()
                        .expect("the shipped torpedo-tube policy decodes"),
                )]
                .into_iter()
                .collect(),
            ),
        ))
        .id();
    (app, e)
}

fn tube_target_count(app: &App, e: Entity) -> u32 {
    app.world()
        .entity(e)
        .get::<crate::console::weapons::TorpedoSystemResource>()
        .unwrap()
        .0
        .tube("fore_port")
        .unwrap()
        .target_count
}

/// The gap this system closes: an AI-crewed ship now asks for its tubes to
/// be loaded, and it does so through the same `SetTorpedoVolleyTarget`
/// command a human console sends — so the order lands on an NPC's own
/// torpedo system, which the LocalShip-only handler could never do.
#[test]
fn ai_torpedo_load_sets_volley_target_through_admitted_commands() {
    let (mut app, e) = torpedo_load_app(ControlSource::Ai);
    app.update();

    assert_eq!(
        tube_target_count(&app, e),
        2,
        "an AI-operated tube should be ordered to its configured \
         ai_target_count (volley_max = 2 here)"
    );
    let admitted = app.world().entity(e).get::<AdmittedCommands>().unwrap();
    assert_eq!(
        admitted.0.len(),
        1,
        "exactly one SetTorpedoVolleyTarget should have been issued"
    );
    assert!(
        matches!(
            admitted.0[0].payload,
            crate::core::messages::SystemControlPayload::SetTorpedoVolleyTarget { count: 2 }
        ),
        "the AI must issue the ordinary console command, not poke state"
    );
}

/// The tube is already where the AI wants it, so no second order goes out.
#[test]
fn ai_torpedo_load_does_not_reissue_an_identical_order() {
    let (mut app, e) = torpedo_load_app(ControlSource::Ai);
    app.update();
    app.update();

    let admitted = app.world().entity(e).get::<AdmittedCommands>().unwrap();
    assert_eq!(
        admitted.0.len(),
        1,
        "the AI must not re-issue a volley order the tube already satisfies"
    );
}

/// A human-crewed tube is the operator's to load. The AI must not touch it
/// — this is the behaviour a non-zero `target_count` default in
/// `TorpedoSystem::from_configs` would have broken.
#[test]
fn ai_torpedo_load_leaves_human_controlled_tubes_alone() {
    let (mut app, e) = torpedo_load_app(ControlSource::Human);
    app.update();

    assert_eq!(
        tube_target_count(&app, e),
        0,
        "a Human-controlled tube must stay exactly as its operator left it"
    );
    assert!(app
        .world()
        .entity(e)
        .get::<AdmittedCommands>()
        .unwrap()
        .0
        .is_empty());
}

/// Offline (rating- or damage-driven) means nobody loads it, AI included.
#[test]
fn ai_torpedo_load_skips_offline_tubes() {
    let (mut app, e) = torpedo_load_app(ControlSource::Offline);
    app.update();

    assert_eq!(tube_target_count(&app, e), 0);
}

// ── Per-tube LOAD policy gate (issue #782) ──────────────────────────────

/// Attach a single-tube `TorpedoTubeAiPolicies` map to `e` for the
/// `fore_port` tube, built from an authored `when` guard on the
/// `torpedo_load` channel.
fn attach_load_policy(app: &mut App, e: Entity, when: &str) {
    let ai = crate::entities::config::FineSystemAiConfigToml {
        evaluate_every_ticks: crate::entities::config::default_evaluate_every_ticks(),
        idle: false,
        param: Default::default(),
        rule: vec![crate::entities::config::FineSystemAiRuleToml {
            priority: 0,
            channel: crate::entities::config::TORPEDO_LOAD_CHANNEL.into(),
            when: when.into(),
            verb: crate::entities::config::TORPEDO_LOAD_VERB.into(),
            value: false,
            level: 0,
            response_index: 0,
        }],
        initial_state: None,
        state: Vec::new(),
        memory: std::collections::HashMap::new(),
    };
    let mut map = std::collections::HashMap::new();
    map.insert("fore_port".to_string(), ai.to_policy().unwrap());
    app.world_mut()
        .entity_mut(e)
        .insert(crate::console::weapons::TorpedoTubeAiPolicies(map));
}

/// An idle tube policy holds the load: no `SetTorpedoVolleyTarget` is issued
/// even though the tube is AI-operated and off its configured volley target.
#[test]
fn ai_torpedo_load_idle_tube_policy_holds() {
    let (mut app, e) = torpedo_load_app(ControlSource::Ai);
    let mut map = std::collections::HashMap::new();
    map.insert(
        "fore_port".to_string(),
        crate::ai::policy::AiPolicy {
            idle: true,
            ..Default::default()
        },
    );
    app.world_mut()
        .entity_mut(e)
        .insert(crate::console::weapons::TorpedoTubeAiPolicies(map));
    app.update();

    assert_eq!(
        tube_target_count(&app, e),
        0,
        "an idle tube policy must hold the AI load order"
    );
    assert!(
        app.world()
            .entity(e)
            .get::<AdmittedCommands>()
            .unwrap()
            .0
            .is_empty(),
        "no volley command should be admitted when the tube policy is idle"
    );
}

/// The #779 empty-facts lesson: the host seeds real per-tube facts, so a
/// `fact(...)` guard actually evaluates. A guard that can never hold over the
/// live magazine count (`fact(magazine) > 100`, magazine is 10) holds the
/// load; the complementary guard (`fact(magazine) > 0`) fires it — proving the
/// facts are seeded, not empty.
#[test]
fn ai_torpedo_load_fact_guard_fires_over_seeded_facts() {
    // Unsatisfiable guard → hold.
    let (mut app, e) = torpedo_load_app(ControlSource::Ai);
    attach_load_policy(&mut app, e, "fact(magazine) > 100");
    app.update();
    assert_eq!(
        tube_target_count(&app, e),
        0,
        "a load guard that never holds over the seeded magazine fact must hold"
    );

    // Satisfiable guard → fire. If facts were empty (#779), `fact(magazine)`
    // would read 0 and this guard would also hold — so a fire here proves the
    // magazine fact was seeded.
    let (mut app, e) = torpedo_load_app(ControlSource::Ai);
    attach_load_policy(&mut app, e, "fact(magazine) > 0");
    app.update();
    assert_eq!(
        tube_target_count(&app, e),
        2,
        "a load guard satisfied by the seeded magazine fact must fire the order"
    );
}

/// Issue #891 stage 2, per-host both-directions proof for the Torpedo tube
/// LOAD host: a `flag()` guard fires when the scenario sets the flag and
/// reads false when it does not — through the full decide → admit → apply
/// pipeline, not the policy evaluator alone.
#[test]
fn ai_torpedo_load_flag_guard_reads_the_world_in_both_directions() {
    // Flag CLEAR → the guard reads false and the load holds.
    let (mut app, e) = torpedo_load_app(ControlSource::Ai);
    app.init_resource::<crate::world::server::WorldContentRuntime>();
    attach_load_policy(&mut app, e, "flag(resupply_authorised)");
    app.update();
    assert_eq!(
        tube_target_count(&app, e),
        0,
        "with the world flag clear the load guard must read false and hold"
    );

    // Flag SET → the SAME guard fires and the volley order lands.
    app.world_mut()
        .resource_mut::<crate::world::server::WorldContentRuntime>()
        .flags
        .set_flag("resupply_authorised");
    app.update();
    assert_eq!(
        tube_target_count(&app, e),
        2,
        "with the world flag set the same guard must fire the load order"
    );
}

#[test]
fn concealed_contact_stops_high_fidelity_frequency_hints_until_normal() {
    let mut app = freq_hint_test_app();
    let source = app.world().resource::<SourceShip>().0;
    app.world_mut()
        .entity_mut(source)
        .insert(crate::entities::spawner::EntityUuid("observer".into()));
    crate::gm::operations::contact::set(
        &mut app
            .world_mut()
            .resource_mut::<crate::world::server::WorldContentRuntime>()
            .contact_overrides,
        "observer",
        "target-1",
        crate::gm::operations::contact::ContactMode::Conceal,
    );
    tick_with_dt(&mut app, 4.0);
    assert!(!app
        .world()
        .resource::<CoordBox>()
        .0
        .iter()
        .any(|row| matches!(row.payload, CoordinationPayload::FrequencyHint { .. })));
    crate::gm::operations::contact::set(
        &mut app
            .world_mut()
            .resource_mut::<crate::world::server::WorldContentRuntime>()
            .contact_overrides,
        "observer",
        "target-1",
        crate::gm::operations::contact::ContactMode::Normal,
    );
    tick_with_dt(&mut app, 4.0);
    assert!(app
        .world()
        .resource::<CoordBox>()
        .0
        .iter()
        .any(|row| matches!(row.payload, CoordinationPayload::FrequencyHint { .. })));
}
