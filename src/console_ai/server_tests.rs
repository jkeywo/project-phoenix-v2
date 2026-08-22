use super::*;
use crate::ai::server::AiHighFidelity;
use crate::console::weapons::TacticalRadarSelection;
use crate::core::messages::{AdmittedCommands, CoordinationPayload};
use crate::ship::control_source::ControlSource;
use crate::ship::shields::{
    PendingShieldsThreatBearing, ShieldsAiConfigResource, ShieldsDamageHistory,
    ShieldsFocusAiPolicy, ShipShields,
};
use crate::ship_plugin::{CoordinationEnqueue, ShipSystemControlSources};

#[derive(Resource, Default)]
struct CoordBox(Vec<CoordinationEnqueue>);

fn collect_coord(mut reader: MessageReader<CoordinationEnqueue>, mut box_: ResMut<CoordBox>) {
    for m in reader.read() {
        box_.0.push(m.clone());
    }
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
    crate::ai::host::register_ai_host_env(&mut app);
    app.add_plugins(bevy::time::TimePlugin)
        .insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
            std::time::Duration::from_millis(100),
        ))
        .init_resource::<crate::ai::server::WorldSnapshot>()
        .init_resource::<ShieldsAiConfigResource>()
        .init_resource::<CoordBox>()
        .insert_resource(crate::lobby::Sessions(
            crate::lobby::session::SessionManager::new(),
        ))
        .add_message::<CoordinationEnqueue>()
        .add_systems(
            Update,
            (
                ai_shield_focus.before(crate::ship::shields::handle_shields_messages),
                crate::ship::shields::handle_shields_messages,
            ),
        )
        .add_systems(PostUpdate, collect_coord);

    app.world_mut().spawn((
        crate::server_app::Ship,
        ShipShields(crate::weapons::shield::ShieldSystem::new(&config), 0.5),
        ShieldsDamageHistory::default(),
        PendingShieldsThreatBearing::default(),
        ai_shield_control_sources(),
        AdmittedCommands::default(),
        AiHighFidelity,
        default_focus_policy(),
    ));

    app
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

/// Coarse shields system (the decide gate) + every synthesised
/// `shield-arc-<id>` fine system (the admission gate) set to Ai —
/// matching how the entity spawner rosters an NPC's systems (arcs are
/// synthesised into `ShipConfig.systems`, so the all-Ai loop covers
/// them in production).
fn ai_shield_control_sources() -> ShipSystemControlSources {
    let mut control_sources = ShipSystemControlSources::default();
    control_sources.0.set(
        crate::ship::system_registry::shields_system_id(),
        ControlSource::Ai,
    );
    for arc_id in ["fore", "port", "aft", "starboard"] {
        control_sources.0.set(
            crate::ship::system_registry::shield_arc_system_id(arc_id).expect("arc id"),
            ControlSource::Ai,
        );
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
    let b = app
        .world_mut()
        .spawn((
            crate::server_app::Ship,
            ShipShields(crate::weapons::shield::ShieldSystem::new(&config), 0.5),
            ShieldsDamageHistory::default(),
            PendingShieldsThreatBearing::default(),
            ai_shield_control_sources(),
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
    // A second ship carrying the permissive tuning as its own policy param.
    let tuned = app
        .world_mut()
        .spawn((
            crate::server_app::Ship,
            ShipShields(crate::weapons::shield::ShieldSystem::new(&config), 0.5),
            ShieldsDamageHistory::default(),
            PendingShieldsThreatBearing::default(),
            ai_shield_control_sources(),
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

    // Advance ~5s of ManualDuration(100ms) ticks with no further hits. The
    // record at ~t=0.2 ages past the 4s window and prunes; the focus must
    // revert to None once concentration is gone and health is balanced.
    for _ in 0..50 {
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
    app.world_mut()
        .entity_mut(e)
        .get_mut::<ShipSystemControlSources>()
        .unwrap()
        .0
        .set(
            crate::ship::system_registry::shields_system_id(),
            ControlSource::Human,
        );

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
    app.world_mut()
        .entity_mut(e)
        .get_mut::<PendingShieldsThreatBearing>()
        .unwrap()
        .0 = Some(90_f32.to_radians());

    app.update();

    let focused = focused_facing(&app, e);
    assert!(
        focused.is_some(),
        "threat-bearing override must focus a facing via the admitted-command path"
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
        hint.target,
        crate::ship::system_registry::tactical_station_key(),
        "frequency hint should target Tactical"
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

[[system]]
id = "sensors"
kind = "sensors"
station = "sensors"
"#;
    crate::ship::config::ShipConfig::from_toml(toml, &["sensors"]).unwrap()
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

// ── ai_power_allocation (inline stateless policy spine, issue #784) ──────

use crate::entities::config::{
    FineSystemAiConfigToml, FineSystemAiRuleToml, POWER_SET_ALLOCATION_VERB,
};
use crate::ship::power::PowerAiPolicy;

/// Build a `PowerAiPolicy` through the real `to_policy` decode path so the
/// tests exercise the value-carrying `set_power_group_allocation` verb + the
/// `level` payload just as authored TOML would.
fn power_policy(params: &[(&str, f32)], rules: Vec<FineSystemAiRuleToml>) -> PowerAiPolicy {
    let cfg = FineSystemAiConfigToml {
        evaluate_every_ticks: crate::entities::config::default_evaluate_every_ticks(),
        idle: false,
        param: params.iter().map(|(k, v)| (k.to_string(), *v)).collect(),
        rule: rules,
        initial_state: None,
        state: Vec::new(),
        memory: std::collections::HashMap::new(),
    };
    PowerAiPolicy(cfg.to_policy().expect("power policy decodes"))
}

fn alloc_rule(priority: i32, channel: &str, when: &str, level: u8) -> FineSystemAiRuleToml {
    FineSystemAiRuleToml {
        priority,
        channel: channel.to_string(),
        when: when.to_string(),
        verb: POWER_SET_ALLOCATION_VERB.to_string(),
        value: false,
        level,
        response_index: 0,
    }
}

fn default_power_policy() -> PowerAiPolicy {
    PowerAiPolicy(
        crate::entities::authored_ai_pins::shipped_policy_toml("power")
            .to_policy()
            .unwrap(),
    )
}

/// Wires the real production pair: the AI decide system
/// (`ai_power_allocation`, emit) `.before` the single applier
/// (`ship::power::handle_power_messages`, issue #831). Attaches the canonical
/// default `PowerAiPolicy` (baseline: helm←thrust / weapons←red alert with
/// reserve guards) unless the caller overrides it.
fn power_test_app() -> App {
    let mut app = App::new();
    crate::ai::host::register_ai_host_env(&mut app);
    app.insert_resource(Time::<()>::default())
        .init_resource::<crate::ship::power::PowerConfigResource>()
        .insert_resource(crate::lobby::Sessions(
            crate::lobby::session::SessionManager::new(),
        ))
        .add_systems(
            Update,
            (
                ai_power_allocation.before(crate::ship::power::handle_power_messages),
                crate::ship::power::handle_power_messages,
            ),
        );

    let mut control_sources = ShipSystemControlSources::default();
    control_sources.0.set(
        crate::ship::system_registry::power_reactor_system_id(),
        ControlSource::Ai,
    );

    app.world_mut().spawn((
        crate::server_app::Ship,
        control_sources,
        crate::ship::power::ShipPowerSystem(crate::modifiers::power_system::PowerSystem::default()),
        crate::ship::state::ShipRedAlert::default(),
        crate::ship::helm::ThrustInput::default(),
        default_power_policy(),
        crate::core::messages::AdmittedCommands::default(),
        AiHighFidelity,
    ));

    app
}

fn power_ship_entity(app: &mut App) -> Entity {
    app.world_mut()
        .query_filtered::<Entity, With<crate::ship::power::ShipPowerSystem>>()
        .single(app.world())
        .unwrap()
}

fn power_level(app: &App, e: Entity, group: &str) -> u8 {
    app.world()
        .entity(e)
        .get::<crate::ship::power::ShipPowerSystem>()
        .unwrap()
        .0
        .level_for(&crate::core::messages::PowerGroupId(group.into()))
}

fn set_battery(app: &mut App, e: Entity, charge: f32) {
    app.world_mut()
        .entity_mut(e)
        .get_mut::<crate::ship::power::ShipPowerSystem>()
        .unwrap()
        .0
        .battery_charge = charge;
}

fn power_tick_with_dt(app: &mut App, dt_secs: f32) {
    let mut time = app.world_mut().resource_mut::<Time>();
    time.advance_by(std::time::Duration::from_secs_f32(dt_secs));
    app.update();
}

/// Variant of [`power_test_app`] built from a SHIPPED hull file: its own
/// `[power]` reactor (capacity, rates, emergency threshold), its own
/// `[power_groups.*]` seeding, and its own `[power.ai_policy]`, with
/// `ship::power::tick_power_system` chained after the applier so the reactor
/// integrates the battery and exhaustion lock every tick.
///
/// Nothing is hand-written: everything the ladder depends on comes off the
/// file the fleet actually flies.
///
/// The queue is NOT cleared between ticks, which
/// `the_shipped_combat_stations_allocation_settles`
/// documents needing. Anything running more than a handful of ticks wants
/// [`shipped_hull_power_app_clearing`] instead.
fn shipped_hull_power_app(path: &str) -> (App, Entity) {
    shipped_hull_power_app_inner(path, false)
}

/// [`shipped_hull_power_app`] plus the per-tick `AdmittedCommands` clear
/// production's admission seam performs — the same mechanism
/// [`over_budget_power_app`] uses, and for the same two reasons.
///
/// Correctness first: without it `handle_power_messages` replays the WHOLE
/// accumulated queue on every tick, so a thousand-tick probe re-applies
/// every decision the ship has ever taken, a thousand times over. Then
/// observability: with the clear in place `emitted_allocations` reports what
/// THIS arm decided rather than the run's whole history, which is what makes
/// "the AI stopped re-deciding" assertable at all.
fn shipped_hull_power_app_clearing(path: &str) -> (App, Entity) {
    shipped_hull_power_app_inner(path, true)
}

fn shipped_hull_power_app_inner(path: &str, clear_admitted: bool) -> (App, Entity) {
    let config = crate::entities::include_resolve::load_entity_config(path)
        .unwrap_or_else(|e| panic!("{path}: {e}"));
    let reactor = config.power.as_ref().expect("hull authors [power]");
    let power_groups = config
        .ship_config
        .as_ref()
        .map(|s| s.power_groups.clone())
        .unwrap_or_default();
    let power_config =
        crate::ship::power::PowerConfigResource(crate::modifiers::power_system::PowerConfig {
            capacity: reactor.capacity,
            rates: reactor.rates,
            sustainable_total: reactor.sustainable_total,
            max_commanded_total: reactor.max_commanded_total,
            emergency_threshold: reactor.emergency_threshold,
        });
    let seed = crate::ship::power::authored_power_group_seed(&power_groups);
    let policy = PowerAiPolicy(
        reactor
            .ai_policy
            .as_ref()
            .expect("hull authors [power.ai_policy]")
            .to_policy()
            .expect("shipped policy decodes"),
    );

    let mut app = App::new();
    crate::ai::host::register_ai_host_env(&mut app);
    app.insert_resource(Time::<()>::default())
        .init_resource::<crate::ship::power::PowerConfigResource>()
        .insert_resource(crate::lobby::Sessions(
            crate::lobby::session::SessionManager::new(),
        ));
    if clear_admitted {
        app.add_systems(
            Update,
            (
                clear_admitted_each_tick,
                ai_power_allocation,
                crate::ship::power::handle_power_messages,
                crate::ship::power::tick_power_system,
            )
                .chain(),
        );
    } else {
        app.add_systems(
            Update,
            (
                ai_power_allocation,
                crate::ship::power::handle_power_messages,
                crate::ship::power::tick_power_system,
            )
                .chain(),
        );
    }

    let mut control_sources = ShipSystemControlSources::default();
    control_sources.0.set(
        crate::ship::system_registry::power_reactor_system_id(),
        ControlSource::Ai,
    );
    let e = app
        .world_mut()
        .spawn((
            crate::server_app::Ship,
            control_sources,
            crate::ship::power::ShipPowerSystem(
                crate::modifiers::power_system::PowerSystem::from_authored_groups(
                    &power_config.0,
                    &seed,
                ),
            ),
            power_config,
            crate::ship::state::ShipRedAlert(true),
            crate::ship::helm::ThrustInput(0.9),
            policy,
            // The hull's OWN `[[station]]`/`[[system]]` roster. Present so
            // the group `max_level` ceilings are read off the file like
            // production's, and so a human command can be put through
            // `command_admission::validate_and_admit`, which resolves the
            // Power station from exactly this config.
            crate::ship_plugin::ShipConfigComponent(
                config
                    .ship_config
                    .clone()
                    .expect("a shipped hull authors its stations and systems"),
            ),
            AdmittedCommands::default(),
            AiHighFidelity,
        ))
        .id();
    (app, e)
}

#[test]
fn baseline_default_reallocates_toward_weapons_on_red_alert() {
    // Baseline preservation: the synthesised default policy reproduces the
    // retired red-alert→weapons behaviour. Under red alert with a full
    // battery (well above the authored `min_reserve_weapons` shed floor —
    // 25 % since issue #1003) weapons rises to its elevated level 3.
    let mut app = power_test_app();
    let e = power_ship_entity(&mut app);
    app.world_mut()
        .entity_mut(e)
        .get_mut::<crate::ship::state::ShipRedAlert>()
        .unwrap()
        .0 = true;

    power_tick_with_dt(&mut app, 0.1);

    assert_eq!(
        power_level(&app, e, crate::modifiers::power_system::WEAPONS_POWER_GROUP),
        3,
        "sustained red alert must elevate weapons power (default baseline)"
    );
}

#[test]
fn baseline_default_reallocates_toward_helm_on_sustained_thrust() {
    // Baseline preservation: sustained high thrust + healthy battery, AT RED
    // ALERT, raises helm power to its elevated level 3 (reproducing the
    // retired movement→helm behaviour, now absolute + stateless). The
    // red-alert guard was added to fix ships browning out on ordinary
    // (non-combat) transit — `plan_helm_travel` commands near-max thrust for
    // any far-off waypoint, so without the guard this elevation held for the
    // whole cruise, not just combat.
    let mut app = power_test_app();
    let e = power_ship_entity(&mut app);
    app.world_mut()
        .entity_mut(e)
        .get_mut::<crate::ship::helm::ThrustInput>()
        .unwrap()
        .0 = 0.9;
    app.world_mut()
        .entity_mut(e)
        .get_mut::<crate::ship::state::ShipRedAlert>()
        .unwrap()
        .0 = true;

    power_tick_with_dt(&mut app, 0.1);

    assert_eq!(
        power_level(&app, e, crate::modifiers::power_system::HELM_POWER_GROUP),
        3,
        "sustained high thrust at red alert must elevate helm power (default baseline)"
    );
}

#[test]
fn sustained_thrust_without_red_alert_does_not_elevate_helm() {
    // Regression guard for the brownout-outside-combat fix: ordinary cruise
    // thrust (no red alert) must hold helm at the baseline 2, not the
    // combat-burst 3 — the whole point of the `red_alert` guard on the
    // elevate rule.
    let mut app = power_test_app();
    let e = power_ship_entity(&mut app);
    app.world_mut()
        .entity_mut(e)
        .get_mut::<crate::ship::helm::ThrustInput>()
        .unwrap()
        .0 = 0.9;

    power_tick_with_dt(&mut app, 0.1);

    assert_eq!(
        power_level(&app, e, crate::modifiers::power_system::HELM_POWER_GROUP),
        2,
        "sustained thrust away from red alert must NOT elevate helm \
         (that used to brown out ships on ordinary transit)"
    );
}

#[test]
fn reserve_gate_blocks_elevation_below_the_authored_floor() {
    // AC2 + AC5: with the battery drained below the helm's 60% RESTORE
    // floor, the elevate guard cannot fire even under full thrust at red
    // alert, so the baseline fallback holds helm at level 2 — allocation
    // never rises when the battery can't sustain it (no avoidable
    // brownout). Above the restore floor it elevates. 50% is the SEPARATE
    // shed floor the hold rule reads once a channel is already up — this
    // test's helm starts at the default (unelevated) level, so nothing
    // here exercises that boundary; it is pinned separately below.
    let mut app = power_test_app();
    let e = power_ship_entity(&mut app);
    app.world_mut()
        .entity_mut(e)
        .get_mut::<crate::ship::helm::ThrustInput>()
        .unwrap()
        .0 = 0.9;
    app.world_mut()
        .entity_mut(e)
        .get_mut::<crate::ship::state::ShipRedAlert>()
        .unwrap()
        .0 = true;

    // 40% battery is below the 60% helm RESTORE floor → held at baseline 2.
    set_battery(&mut app, e, 40.0);
    power_tick_with_dt(&mut app, 0.1);
    assert_eq!(
        power_level(&app, e, crate::modifiers::power_system::HELM_POWER_GROUP),
        2,
        "below-restore-floor thrust must NOT elevate helm (brownout avoidance)"
    );

    // Recharge above the 60% restore floor → the same thrust now elevates.
    set_battery(&mut app, e, 80.0);
    power_tick_with_dt(&mut app, 0.1);
    assert_eq!(
        power_level(&app, e, crate::modifiers::power_system::HELM_POWER_GROUP),
        3,
        "above-restore-floor thrust must elevate helm"
    );

    // Now pin the boundary the test's NAME actually promises: with helm
    // already elevated (as it is now, from the arm above), a battery under
    // the 50% SHED floor — but nowhere near zero — must give the point
    // back. This is the HOLD rule's `min_reserve_helm` guard, distinct from
    // the ELEVATE rule's `min_restore_helm` exercised above; 45% sits below
    // the shed floor but above both the weapons floors, so only helm moves.
    set_battery(&mut app, e, 45.0);
    power_tick_with_dt(&mut app, 0.1);
    assert_eq!(
        power_level(&app, e, crate::modifiers::power_system::HELM_POWER_GROUP),
        2,
        "below the 50% shed floor an already-elevated helm must shed — the \
         authored floor this test is named for"
    );
}

#[test]
fn reserve_gate_lowers_allocation_when_battery_dips_under_load() {
    // AC5: a group already elevated is brought back down by the lowering
    // baseline rule once the battery falls below the reserve while still
    // under thrust — the per-rule reserve guard is the brownout-avoidance
    // mechanism, with no global emergency exception.
    let mut app = power_test_app();
    let e = power_ship_entity(&mut app);
    app.world_mut()
        .entity_mut(e)
        .get_mut::<crate::ship::helm::ThrustInput>()
        .unwrap()
        .0 = 0.9;
    app.world_mut()
        .entity_mut(e)
        .get_mut::<crate::ship::state::ShipRedAlert>()
        .unwrap()
        .0 = true;

    set_battery(&mut app, e, 80.0);
    power_tick_with_dt(&mut app, 0.1);
    assert_eq!(
        power_level(&app, e, crate::modifiers::power_system::HELM_POWER_GROUP),
        3
    );

    set_battery(&mut app, e, 40.0);
    power_tick_with_dt(&mut app, 0.1);
    assert_eq!(
        power_level(&app, e, crate::modifiers::power_system::HELM_POWER_GROUP),
        2,
        "an elevated group must drop back to baseline once battery dips below reserve"
    );
}

// ── Budget-aware allocation (issue #959) ─────────────────────────────────

/// A four-group reactor (`ops` outside the canonical trio) crewed by
/// `policy`, with the production decide→apply pair and a per-tick
/// `AdmittedCommands` clear so each arm's emits can be counted on their own.
///
/// Commanded at the 8-point cap on arrival — helm 3 / weapons 2 / shields 2
/// / ops 1 — because the interesting failure is what a policy does when
/// there is NOTHING left to spend.
fn over_budget_power_app(policy: PowerAiPolicy) -> (App, Entity) {
    use crate::modifiers::power_system::{
        HELM_POWER_GROUP, SHIELDS_POWER_GROUP, WEAPONS_POWER_GROUP,
    };
    let seed = [
        (
            crate::core::messages::PowerGroupId(HELM_POWER_GROUP.into()),
            3u8,
        ),
        (
            crate::core::messages::PowerGroupId(WEAPONS_POWER_GROUP.into()),
            2,
        ),
        (
            crate::core::messages::PowerGroupId(SHIELDS_POWER_GROUP.into()),
            2,
        ),
        (crate::core::messages::PowerGroupId("ops".into()), 1),
    ];

    let mut app = App::new();
    crate::ai::host::register_ai_host_env(&mut app);
    app.insert_resource(Time::<()>::default())
        .init_resource::<crate::ship::power::PowerConfigResource>()
        .insert_resource(crate::lobby::Sessions(
            crate::lobby::session::SessionManager::new(),
        ))
        .add_systems(
            Update,
            (
                clear_admitted_each_tick,
                ai_power_allocation,
                crate::ship::power::handle_power_messages,
            )
                .chain(),
        );

    let mut control_sources = ShipSystemControlSources::default();
    control_sources.0.set(
        crate::ship::system_registry::power_reactor_system_id(),
        ControlSource::Ai,
    );
    let e = app
        .world_mut()
        .spawn((
            crate::server_app::Ship,
            control_sources,
            crate::ship::power::ShipPowerSystem(
                crate::modifiers::power_system::PowerSystem::from_authored_groups(
                    &crate::modifiers::power_system::PowerConfig::default(),
                    &seed,
                ),
            ),
            crate::ship::state::ShipRedAlert(true),
            crate::ship_plugin::LastHelmInput::default(),
            policy,
            AdmittedCommands::default(),
            AiHighFidelity,
        ))
        .id();
    (app, e)
}

fn commanded(app: &App, e: Entity, group: &str) -> u8 {
    app.world()
        .entity(e)
        .get::<crate::ship::power::ShipPowerSystem>()
        .unwrap()
        .0
        .commanded_level_for(&crate::core::messages::PowerGroupId(group.into()))
}

fn commanded_total(app: &App, e: Entity) -> u8 {
    app.world()
        .entity(e)
        .get::<crate::ship::power::ShipPowerSystem>()
        .unwrap()
        .0
        .commanded_total()
}

/// Every `SetPowerGroupAllocation` this ship emitted on the arm that just
/// ran, in emission order.
fn emitted_allocations(app: &App, e: Entity) -> Vec<(String, u8)> {
    app.world()
        .entity(e)
        .get::<AdmittedCommands>()
        .unwrap()
        .for_target(crate::ship::system_registry::POWER_REACTOR_SYSTEM_ID)
        .filter_map(|c| match &c.payload {
            crate::core::messages::SystemControlPayload::SetPowerGroupAllocation {
                group,
                level,
            } => Some((group.0.clone(), *level)),
            _ => None,
        })
        .collect()
}

/// **The silent cap-refusal-and-reemit loop is gone (issue #959).**
///
/// Three rules ask for level 4 on a reactor that has 8 points and four
/// groups, so the policy is asking for roughly half again what the ship
/// owns. Before this issue each channel was emitted in isolation:
/// `PowerSystem::increase` refused the surplus SILENTLY, the refused groups
/// never reached the level they had been commanded to, and the decider —
/// which only skips an emit when the commanded level already MATCHES —
/// re-issued the identical admitted command on every arm for the rest of
/// the encounter.
///
/// Now the arm plans against the budget: the ship lands on a legal
/// allocation in ONE arm, every emitted command is actually carried out,
/// and the arms that follow emit nothing at all.
#[test]
fn an_over_budget_power_policy_settles_in_one_arm_and_stops_re_emitting() {
    use crate::modifiers::power_system::{
        HELM_POWER_GROUP, SHIELDS_POWER_GROUP, WEAPONS_POWER_GROUP,
    };
    let policy = power_policy(
        &[],
        vec![
            alloc_rule(20, WEAPONS_POWER_GROUP, "true", 4),
            alloc_rule(10, HELM_POWER_GROUP, "true", 4),
            alloc_rule(5, SHIELDS_POWER_GROUP, "true", 4),
        ],
    );
    let (mut app, e) = over_budget_power_app(policy);

    power_tick_with_dt(&mut app, 0.1);

    // Highest authored priority is paid in full; the rest take what is left
    // and land on their minimum, with `ops` (nothing bid for it) reserved.
    assert_eq!(commanded(&app, e, WEAPONS_POWER_GROUP), 4);
    assert_eq!(commanded(&app, e, HELM_POWER_GROUP), 2);
    assert_eq!(commanded(&app, e, SHIELDS_POWER_GROUP), 1);
    assert_eq!(commanded(&app, e, "ops"), 1);

    let total = app
        .world()
        .entity(e)
        .get::<crate::ship::power::ShipPowerSystem>()
        .unwrap()
        .0
        .commanded_total();
    assert_eq!(total, 8, "the budget is spent, and not overspent");

    // The emitted ORDER is load-bearing, not incidental: the applier tests
    // the budget one command at a time, so weapons 2 → 4 is only affordable
    // after helm and shields have given their points back. Emitting the
    // increase first would have it refused — silently — with the ship
    // already at the cap.
    assert_eq!(
        emitted_allocations(&app, e),
        vec![
            (HELM_POWER_GROUP.to_string(), 2),
            (SHIELDS_POWER_GROUP.to_string(), 1),
            (WEAPONS_POWER_GROUP.to_string(), 4),
        ],
        "decreases must be emitted before the increases they pay for"
    );

    // Every command the arm emitted was actually carried out — the silent
    // refusal has nothing left to swallow.
    for (group, level) in emitted_allocations(&app, e) {
        assert_eq!(
            commanded(&app, e, &group),
            level,
            "{group} was commanded to {level} and the reactor refused it"
        );
    }

    // …and the decision has settled: nothing is re-emitted, for ever.
    for arm in 0..6 {
        power_tick_with_dt(&mut app, 0.1);
        assert!(
            emitted_allocations(&app, e).is_empty(),
            "arm {arm} re-emitted after the allocation had settled: {:?}",
            emitted_allocations(&app, e)
        );
        assert_eq!(commanded(&app, e, WEAPONS_POWER_GROUP), 4);
        assert_eq!(commanded(&app, e, HELM_POWER_GROUP), 2);
    }
}

/// **Which group wins a budget collision is the HULL's decision.** The same
/// three over-budget bids with the authored priorities swapped hand the
/// spare points to helm instead of weapons. Both of `plan_allocation`'s
/// ordering keys are authored, so there is no branch to override the
/// authored config with — the Rust seed order breaks only a tie the hull
/// left identical on both.
#[test]
fn the_authored_rule_priority_decides_who_gets_the_last_reactor_point() {
    use crate::modifiers::power_system::{
        HELM_POWER_GROUP, SHIELDS_POWER_GROUP, WEAPONS_POWER_GROUP,
    };
    let policy = power_policy(
        &[],
        vec![
            alloc_rule(5, WEAPONS_POWER_GROUP, "true", 4),
            alloc_rule(20, HELM_POWER_GROUP, "true", 4),
            alloc_rule(10, SHIELDS_POWER_GROUP, "true", 4),
        ],
    );
    let (mut app, e) = over_budget_power_app(policy);
    power_tick_with_dt(&mut app, 0.1);

    assert_eq!(commanded(&app, e, HELM_POWER_GROUP), 4);
    assert_eq!(commanded(&app, e, SHIELDS_POWER_GROUP), 2);
    assert_eq!(commanded(&app, e, WEAPONS_POWER_GROUP), 1);
}

/// The shipped fleet's combat power allocation settles: once the first
/// arm has decided its allocation, a second arm ticked immediately after
/// adds nothing further to the emitted queue.
///
/// This is assertable here without a per-tick `AdmittedCommands` clear:
/// `handle_power_messages` does not drain the queue, so a second arm that
/// decides to emit nothing leaves the emitted list byte-identical.
#[test]
fn the_shipped_combat_stations_allocation_settles() {
    let (mut app, e) = shipped_hull_power_app("assets/entities/alliance_destroyer.toml");
    power_tick_with_dt(&mut app, 0.1);

    let first_arm = emitted_allocations(&app, e);

    // …and it has settled: the second arm adds nothing to the queue.
    power_tick_with_dt(&mut app, 0.1);
    assert_eq!(
        emitted_allocations(&app, e),
        first_arm,
        "the shipped allocation re-emitted after it had settled"
    );
}

// ── The AI shed ladder (issue #1003) ─────────────────────────────────────

/// One hull's authored shed ladder, in battery percentages.
///
/// Each channel has TWO floors since issue #1003 and the gap between them is
/// the whole mechanism, so they travel together rather than as four loose
/// numbers.
#[derive(Debug, Clone, Copy)]
struct ShedLadder {
    /// Below this, helm gives its elevated point back.
    helm_shed: f64,
    /// …and it may not take the point again until the charge is back here.
    helm_restore: f64,
    /// Below this, weapons gives its elevated point back — the rung that
    /// turns the battery around.
    weapons_shed: f64,
    /// …and the charge weapons re-elevates at.
    weapons_restore: f64,
}

/// The hull's own ladder, read off the spawned policy rather than restated,
/// so a retune in TOML retunes every probe below with it (AGENTS.md rule
/// #11).
fn authored_shed_ladder(app: &App, e: Entity) -> ShedLadder {
    use crate::entities::config::{
        POWER_HELM_RESERVE_PARAM, POWER_HELM_RESTORE_PARAM, POWER_WEAPONS_RESERVE_PARAM,
        POWER_WEAPONS_RESTORE_PARAM,
    };
    let policy = app
        .world()
        .entity(e)
        .get::<PowerAiPolicy>()
        .expect("the shipped hull spawned with its authored power policy");
    let param = |name: &str| {
        policy
            .0
            .params
            .get(name)
            .unwrap_or_else(|| panic!("the shipped policy authors `{name}`"))
    };
    ShedLadder {
        helm_shed: param(POWER_HELM_RESERVE_PARAM),
        helm_restore: param(POWER_HELM_RESTORE_PARAM),
        weapons_shed: param(POWER_WEAPONS_RESERVE_PARAM),
        weapons_restore: param(POWER_WEAPONS_RESTORE_PARAM),
    }
}

/// This ship's battery, as a percentage of its authored capacity.
///
/// Deliberately the SAME arithmetic in the same order as
/// `ai_power_allocation`'s `fact(battery_pct)` seeding: the whole quotient
/// is taken in `f32` and only then widened, which is not the same number as
/// widening first when the assertion is a comparison against a floor the
/// charge is sitting exactly on.
fn battery_pct(app: &App, e: Entity) -> f64 {
    let charge = app
        .world()
        .entity(e)
        .get::<crate::ship::power::ShipPowerSystem>()
        .unwrap()
        .0
        .battery_charge;
    let capacity = app
        .world()
        .entity(e)
        .get::<crate::ship::power::PowerConfigResource>()
        .unwrap()
        .0
        .capacity;
    ((charge / capacity) * 100.0) as f64
}

/// Seat a human Power officer on this ship and return their session token.
///
/// Flips the reactor's control source to Human and registers a connected
/// player holding the station THIS hull's own `[[system]]` roster assigns
/// the reactor to — `engineering` on the destroyer, but resolved through
/// `command_admission::station_for_system` rather than named, because that
/// is the resolution the admission gate itself performs and a hull is free
/// to put the reactor anywhere.
fn seat_human_power_officer(app: &mut App, e: Entity) -> String {
    let reactor = crate::ship::system_registry::power_reactor_system_id();
    app.world_mut()
        .entity_mut(e)
        .get_mut::<ShipSystemControlSources>()
        .unwrap()
        .0
        .set(reactor.clone(), ControlSource::Human);

    let station = {
        let config = &app
            .world()
            .entity(e)
            .get::<crate::ship_plugin::ShipConfigComponent>()
            .expect("the shipped hull spawned with its own ShipConfig")
            .0;
        // No human-seeking map: the reactor is not a `human_seeking`
        // system on any hull, so the authored station is the answer.
        crate::command_admission::station_for_system(config, None, &reactor)
            .expect("the hull's reactor belongs to one of its stations")
    };

    let token = "power-officer".to_string();
    let mut sessions = app.world_mut().resource_mut::<crate::lobby::Sessions>();
    sessions
        .0
        .register(token.clone(), "Power Officer".into())
        .expect("a fresh token registers");
    sessions.0.set_station(&token, Some(station));
    token
}

/// Put one human `SetPowerGroupAllocation` through the REAL admission seam
/// and into this ship's queue, returning whether admission accepted it.
///
/// The queue is replaced rather than appended to, standing in for
/// `admit_system_commands`' own clear-and-refill: `handle_power_messages`
/// does not drain what it applies, so the AI's earlier emits would otherwise
/// re-apply a decision the human has taken over from.
fn admit_human_power_command(
    app: &mut App,
    e: Entity,
    token: &str,
    group: &str,
    level: u8,
) -> bool {
    // Cloned out of the world so the admission call can hold `&Sessions`
    // (a resource) and `&mut AdmittedCommands` (a component) at once.
    let sources = app
        .world()
        .entity(e)
        .get::<ShipSystemControlSources>()
        .unwrap()
        .clone();
    let config = app
        .world()
        .entity(e)
        .get::<crate::ship_plugin::ShipConfigComponent>()
        .expect("the shipped hull spawned with its own ShipConfig")
        .0
        .clone();
    let mut admitted = AdmittedCommands::default();
    let ok = crate::command_admission::validate_and_admit(
        token,
        crate::ship::system_registry::power_reactor_system_id(),
        crate::core::messages::SystemControlPayload::SetPowerGroupAllocation {
            group: crate::core::messages::PowerGroupId(group.into()),
            level,
        },
        &sources,
        app.world().resource::<crate::lobby::Sessions>(),
        &config,
        &mut admitted,
    );
    *app.world_mut()
        .entity_mut(e)
        .get_mut::<AdmittedCommands>()
        .unwrap() = admitted;
    ok
}

/// Put the battery at an exact percentage of this hull's own capacity.
fn set_battery_pct(app: &mut App, e: Entity, pct: f64) {
    let capacity = app
        .world()
        .entity(e)
        .get::<crate::ship::power::PowerConfigResource>()
        .unwrap()
        .0
        .capacity;
    set_battery(app, e, (pct as f32 / 100.0) * capacity);
}

/// **The two-step shed ladder, on the hull the fleet flies (issue #1003).**
///
/// The four authored floors are one staircase rather than four unrelated
/// brownout guards, and the emergent ship-wide total is what walks down it.
/// Nothing demotes anything: when a channel's hold AND elevate guards both
/// read false it falls to its own `priority = 0` baseline at level 2, and
/// THAT is the shed. There is no ship-wide "total" rule anywhere in the
/// authored policy, which is exactly why the total is worth asserting.
///
/// This is the FALLING walk, at the shed floors. The rising walk is
/// [`the_shed_ladder_re_elevates_only_once_the_restore_floors_are_back`],
/// and it happens at different, higher charges — which is the whole point of
/// there being two numbers per channel.
///
/// `shields` never bids on any rung, so its authored 2 is reserved rather
/// than cut to buy another group a point — pinned on every rung, because a
/// planner that cut it would land on the same totals for the wrong reason.
#[test]
fn the_shipped_power_policy_sheds_one_group_at_each_authored_floor() {
    use crate::modifiers::power_system::{
        HELM_POWER_GROUP, SHIELDS_POWER_GROUP, WEAPONS_POWER_GROUP,
    };
    let (mut app, e) = shipped_hull_power_app_clearing("assets/entities/alliance_destroyer.toml");
    let ladder = authored_shed_ladder(&app, e);
    assert!(
        ladder.weapons_shed < ladder.helm_shed && ladder.weapons_shed > 0.0,
        "the ladder needs weapons to be the LOWER step and every floor to be a \
         live threshold; read {ladder:?}"
    );
    assert!(
        ladder.weapons_shed < ladder.weapons_restore && ladder.helm_shed < ladder.helm_restore,
        "each channel must restore ABOVE where it sheds, or the shed floor is \
         also the re-elevate floor and the channel flips every tick the charge \
         rests on it; read {ladder:?}"
    );

    // Rung 1 — a full battery at combat stations: both elevations paid.
    power_tick_with_dt(&mut app, 0.1);
    assert_eq!(commanded(&app, e, HELM_POWER_GROUP), 3);
    assert_eq!(commanded(&app, e, WEAPONS_POWER_GROUP), 3);
    assert_eq!(commanded(&app, e, SHIELDS_POWER_GROUP), 2);
    assert_eq!(commanded_total(&app, e), 8, "the combat-stations burst");

    // Rung 2 — between the two shed floors: helm gives its point back,
    // weapons keeps its own. Helm is the cheaper thing to lose; the ship is
    // still shooting at full damage.
    set_battery_pct(&mut app, e, (ladder.helm_shed + ladder.weapons_shed) / 2.0);
    power_tick_with_dt(&mut app, 0.1);
    assert_eq!(
        commanded(&app, e, HELM_POWER_GROUP),
        2,
        "below `min_reserve_helm` the helm hold guard reads false too and the \
         priority-0 baseline takes the channel back — that IS the shed"
    );
    assert_eq!(
        commanded(&app, e, WEAPONS_POWER_GROUP),
        3,
        "the weapons floor has NOT been crossed yet, so this elevation stands"
    );
    assert_eq!(
        commanded(&app, e, SHIELDS_POWER_GROUP),
        2,
        "reserved, not cut"
    );
    assert_eq!(commanded_total(&app, e), 7);

    // Rung 2b — under the weapons RESTORE floor but still over its shed
    // floor. The elevate rule could not fire here; the hold rule can, and
    // does, because weapons is already up. Falling through this band without
    // a change is what a rising ship may NOT do.
    let hold_band = (ladder.weapons_shed + ladder.weapons_restore) / 2.0;
    assert!(hold_band < ladder.weapons_restore && hold_band > ladder.weapons_shed);
    set_battery_pct(&mut app, e, hold_band);
    power_tick_with_dt(&mut app, 0.1);
    assert_eq!(
        commanded(&app, e, WEAPONS_POWER_GROUP),
        3,
        "inside the hysteresis band a channel that is already elevated HOLDS: \
         only `min_reserve_weapons` sheds it, and that is still below"
    );
    assert_eq!(commanded_total(&app, e), 7);

    // Rung 3 — under the lower shed floor: weapons follows, and the ship is
    // at the resting total its own `rates` recharge from. The charging
    // control is the point of the whole ladder: this is the rung that turns
    // the battery around before it can reach the exhaustion lock at 0.
    set_battery_pct(&mut app, e, ladder.weapons_shed / 2.0);
    power_tick_with_dt(&mut app, 0.1);
    assert_eq!(commanded(&app, e, HELM_POWER_GROUP), 2);
    assert_eq!(
        commanded(&app, e, WEAPONS_POWER_GROUP),
        2,
        "below `min_reserve_weapons` the last elevation is shed too"
    );
    assert_eq!(
        commanded(&app, e, SHIELDS_POWER_GROUP),
        2,
        "reserved, not cut"
    );
    assert_eq!(commanded_total(&app, e), 6);

    let (power, cfg) = {
        let ent = app.world().entity(e);
        (
            ent.get::<crate::ship::power::ShipPowerSystem>()
                .unwrap()
                .0
                .clone(),
            ent.get::<crate::ship::power::PowerConfigResource>()
                .unwrap()
                .0
                .clone(),
        )
    };
    assert!(
        power.is_charging(&cfg),
        "the bottom rung must be a rung the reactor RECHARGES from, or the \
         ladder only slows the walk to the exhaustion lock instead of \
         stopping it. This hull's `rates` say {:?} at total {}",
        cfg.rates,
        power.commanded_total()
    );
    assert!(!power.locked(), "nothing on this ladder reaches the lock");
}

/// The ladder is climbed as well as descended — but NOT at the charges it
/// was descended at. Each channel comes back at its `min_restore_*`, ten
/// battery-percent above the floor it shed at, and holds its shed level
/// throughout the band between the two.
///
/// Worth its own test rather than a tail on the one above, for two
/// independent failures. The shed is a guard reading false, so a stuck shed
/// would need nothing more exotic than a fact that stopped being re-seeded —
/// and a ship that gave its combat allocation back for good after one dip is
/// a worse bug than the lock this issue is about. The opposite failure is
/// the one #1003's review found: a channel that re-elevates the moment it is
/// back over the floor it shed at flips every tick, because the lower total
/// recharges past that floor inside a single tick.
#[test]
fn the_shed_ladder_re_elevates_only_once_the_restore_floors_are_back() {
    use crate::modifiers::power_system::{HELM_POWER_GROUP, WEAPONS_POWER_GROUP};
    let (mut app, e) = shipped_hull_power_app_clearing("assets/entities/alliance_destroyer.toml");
    let ladder = authored_shed_ladder(&app, e);

    // Arm-first, as `the_hysteresis_band_holds_steady_...` does: the ship
    // spawns at a full battery, so the first tick actually elevates both
    // channels rather than starting the test already at the resting total.
    power_tick_with_dt(&mut app, 0.1);
    assert_eq!(commanded(&app, e, WEAPONS_POWER_GROUP), 3);
    assert_eq!(commanded(&app, e, HELM_POWER_GROUP), 3);
    assert_eq!(commanded_total(&app, e), 8);

    // Now the real shed: dropping the battery under the floor moves the
    // allocation down from that elevated 8, not from a total that never rose.
    set_battery_pct(&mut app, e, ladder.weapons_shed / 2.0);
    power_tick_with_dt(&mut app, 0.1);
    assert_eq!(commanded_total(&app, e), 6, "shed to the resting total");

    // Back over the weapons SHED floor but not its restore floor. Under the
    // single-threshold ladder this was already an elevation; now it is not.
    set_battery_pct(
        &mut app,
        e,
        (ladder.weapons_shed + ladder.weapons_restore) / 2.0,
    );
    power_tick_with_dt(&mut app, 0.1);
    assert_eq!(
        commanded(&app, e, WEAPONS_POWER_GROUP),
        2,
        "a channel that has SHED may not come back at the floor it shed at — \
         the hold rule needs it to be up already, and the elevate rule reads \
         `min_restore_weapons`, which is still above"
    );
    assert_eq!(commanded_total(&app, e), 6);

    // Over the weapons restore floor: weapons alone comes back.
    set_battery_pct(&mut app, e, ladder.weapons_restore + 1.0);
    power_tick_with_dt(&mut app, 0.1);
    assert_eq!(commanded(&app, e, WEAPONS_POWER_GROUP), 3);
    assert_eq!(commanded(&app, e, HELM_POWER_GROUP), 2);
    assert_eq!(commanded_total(&app, e), 7);

    // The same story one rung up: over helm's shed floor, under its restore
    // floor, helm stays down.
    set_battery_pct(&mut app, e, (ladder.helm_shed + ladder.helm_restore) / 2.0);
    power_tick_with_dt(&mut app, 0.1);
    assert_eq!(
        commanded(&app, e, HELM_POWER_GROUP),
        2,
        "helm's own hysteresis band, and it behaves like weapons'"
    );
    assert_eq!(commanded_total(&app, e), 7);

    // Over both restore floors: the full combat-stations allocation.
    set_battery_pct(&mut app, e, ladder.helm_restore + 5.0);
    power_tick_with_dt(&mut app, 0.1);
    assert_eq!(commanded(&app, e, HELM_POWER_GROUP), 3);
    assert_eq!(commanded(&app, e, WEAPONS_POWER_GROUP), 3);
    assert_eq!(commanded_total(&app, e), 8);
}

/// **Inside the hysteresis band the decision HOLDS — it does not strobe.**
///
/// The regression #1003's review found in the first cut of this issue. With
/// one threshold per channel the shed floor is also the re-elevate floor, so
/// a battery resting on it flips the channel EVERY TICK: shed drops the
/// total, the lower total recharges the battery back over the floor inside
/// one 30 Hz tick (a destroyer moves ±0.095 battery-percent per tick),
/// weapons re-elevates, the higher total drains it back under. That is a
/// `SetPowerGroupAllocation`, a `LogCat::Power` line, and a ×1.25/×1.0 swing
/// on `ModifierSlot::PhaserDamage` at tick rate — invisible to a test that
/// only samples the end state, and very visible to a player.
///
/// The battery is PARKED in the band each tick rather than left to the
/// reactor, because the point is what the policy does at a fixed charge that
/// is neither above the restore floor nor below the shed floor. Both
/// directions of arrival are probed: the band must hold an elevated channel
/// up and a shed channel down, and it is the second that a single-threshold
/// ladder gets wrong.
#[test]
fn the_hysteresis_band_holds_steady_and_stops_the_per_tick_allocation_flip() {
    use crate::modifiers::power_system::WEAPONS_POWER_GROUP;
    let dt = 1.0 / 30.0;
    let (mut app, e) = shipped_hull_power_app_clearing("assets/entities/alliance_destroyer.toml");
    let ladder = authored_shed_ladder(&app, e);
    let band = (ladder.weapons_shed + ladder.weapons_restore) / 2.0;

    // ARRIVING FROM ABOVE. One arm at a full battery puts both elevations
    // on, then the charge is parked in the band.
    power_tick_with_dt(&mut app, dt);
    assert_eq!(commanded(&app, e, WEAPONS_POWER_GROUP), 3);
    for tick in 0..300 {
        set_battery_pct(&mut app, e, band);
        power_tick_with_dt(&mut app, dt);
        assert_eq!(
            commanded(&app, e, WEAPONS_POWER_GROUP),
            3,
            "tick {tick}: an elevated channel HOLDS through the band"
        );
        assert_eq!(commanded_total(&app, e), 7, "tick {tick}");
        // Tick 0 is the helm shed (the band is below `min_reserve_helm`),
        // which is a real decision. Everything after it must be silence.
        if tick > 0 {
            assert!(
                emitted_allocations(&app, e).is_empty(),
                "tick {tick}: the allocation had settled and the arm emitted \
                 {:?} anyway — this is the per-tick flip",
                emitted_allocations(&app, e)
            );
        }
    }

    // ARRIVING FROM BELOW. Shed weapons under its floor, then park the
    // charge back in the band: it must NOT come back until the restore
    // floor, and it must not re-decide while it waits.
    set_battery_pct(&mut app, e, ladder.weapons_shed / 2.0);
    power_tick_with_dt(&mut app, dt);
    assert_eq!(commanded(&app, e, WEAPONS_POWER_GROUP), 2);
    for tick in 0..300 {
        set_battery_pct(&mut app, e, band);
        power_tick_with_dt(&mut app, dt);
        assert_eq!(
            commanded(&app, e, WEAPONS_POWER_GROUP),
            2,
            "tick {tick}: a SHED channel stays down through the band — coming \
             back at `min_reserve_weapons` is exactly the flip"
        );
        assert_eq!(commanded_total(&app, e), 6, "tick {tick}");
        assert!(
            emitted_allocations(&app, e).is_empty(),
            "tick {tick}: nothing to decide, and the arm emitted {:?}",
            emitted_allocations(&app, e)
        );
    }
}

/// **The floors are AI-shed only: a human pushes straight past them.**
///
/// The guards live in the AI's bid stage, upstream of the admitted-command
/// seam. `ship::power::handle_power_messages` — the ONE applier both sides
/// go through (AGENTS.md rule #6) — never reads `battery_pct`, so a Power
/// officer who wants weapons at 3 on a battery under both floors gets
/// weapons at 3, and keeps it.
///
/// That is the intended asymmetry rather than a hole in the symmetry: the
/// seam is symmetric, the JUDGEMENT is not. An AI that never spends its
/// last reserve is a doctrine; a human who does is making a decision the
/// game is supposed to let them make.
///
/// The command goes in through `command_admission::validate_and_admit` with
/// a real session token, not by pushing an `AdmittedCommand` onto the queue
/// by hand. A raw push is the thing this file's own AI emitters are
/// forbidden from doing, and it would prove only that the APPLIER ignores
/// the battery. What the claim needs is that a human command below the
/// floors is ADMITTED and then applied, which is admission plus application
/// — two gates, and only one of them was being tested.
#[test]
fn a_human_power_command_pushes_past_the_ai_shed_floors() {
    use crate::modifiers::power_system::WEAPONS_POWER_GROUP;
    let (mut app, e) = shipped_hull_power_app("assets/entities/alliance_destroyer.toml");
    let ladder = authored_shed_ladder(&app, e);
    let under_both = ladder.weapons_shed / 2.0;

    // Arm first: weapons spawns SEEDED at its authored `default_level`, 2 —
    // the same number the shed baseline falls to — so shedding straight from
    // spawn would prove nothing, the channel was never up to begin with. A
    // full battery clears the restore floor and puts the AI's own ELEVATE
    // rule behind the 3 that follows.
    set_battery_pct(&mut app, e, 100.0);
    power_tick_with_dt(&mut app, 0.1);
    assert_eq!(
        commanded(&app, e, WEAPONS_POWER_GROUP),
        3,
        "weapons must actually elevate on a full battery, or the shed below is \
         not a real transition and the test setup is unproven all over again"
    );

    // The AI, holding the reactor, sheds both elevations at this charge — a
    // REAL shed, down from the 3 just armed above.
    set_battery_pct(&mut app, e, under_both);
    power_tick_with_dt(&mut app, 0.1);
    assert_eq!(
        commanded(&app, e, WEAPONS_POWER_GROUP),
        2,
        "the AI must have shed first, or the human command below proves nothing"
    );

    // A human takes the Power station: the reactor's control source flips to
    // Human and a registered session holds whatever station THIS hull's own
    // config says owns the reactor.
    let token = seat_human_power_officer(&mut app, e);

    // …and commands the very elevation the floor just refused the AI.
    set_battery_pct(&mut app, e, under_both);
    assert!(
        admit_human_power_command(&mut app, e, &token, WEAPONS_POWER_GROUP, 3),
        "the station holder's `SetPowerGroupAllocation` must pass admission \
         on a battery under both AI shed floors — admission consults the \
         control source and the station tenure, never the reserve"
    );
    power_tick_with_dt(&mut app, 0.1);
    assert_eq!(
        commanded(&app, e, WEAPONS_POWER_GROUP),
        3,
        "an admitted human `SetPowerGroupAllocation` must apply on a battery \
         below both AI shed floors — the applier does not read `battery_pct`"
    );
    assert_eq!(commanded_total(&app, e), 7);
    assert!(
        battery_pct(&app, e) < ladder.weapons_shed,
        "the raise must have landed while the battery was still under the \
         floor, or this test drifted into asserting nothing"
    );

    // …and it STICKS. Nothing downstream re-reads the floor and pulls a
    // human's allocation back down.
    {
        let mut ent = app.world_mut().entity_mut(e);
        ent.get_mut::<AdmittedCommands>().unwrap().0.clear();
    }
    set_battery_pct(&mut app, e, under_both);
    power_tick_with_dt(&mut app, 0.1);
    assert_eq!(
        commanded(&app, e, WEAPONS_POWER_GROUP),
        3,
        "the human's allocation was clawed back by a floor that is supposed \
         to be AI-shed only"
    );
}

/// A four-group hull config whose `weapons` group is capped at
/// `max_level = 2`, parsed through the real `ShipConfig` path so
/// `[power_groups.<id>] max_level` is read exactly as an authored hull file
/// supplies it — including the `#[serde(default)]` fallback the other three
/// groups take by omitting the key.
fn weapons_capped_ship_config() -> crate::ship::config::ShipConfig {
    let toml = r#"
[[system]]
id = "power-reactor"
kind = "power_reactor"
ai_only = true

[power_groups.ops]
label = "Operations"
default_level = 1

[power_groups.helm]
label = "Propulsion"
default_level = 3

[power_groups.weapons]
label = "Weapons"
default_level = 2
max_level = 2

[power_groups.shields]
label = "Shields"
default_level = 2
"#;
    crate::ship::config::ShipConfig::from_toml(
        toml,
        &[crate::ship::system_registry::POWER_REACTOR_KIND],
    )
    .unwrap()
}

/// **The hull's own `[power_groups.<id>] max_level` is what caps a grant.**
///
/// The wiring test for `AllocationBid::max_level`. Every other power fixture
/// in this file spawns a ship with no `ShipConfigComponent` at all, so the
/// bid has always taken `ship::config::default_max_power_level`'s 4 through
/// the `unwrap_or_else` fallback — a read that ignored the authored ceiling
/// entirely would have passed the lot of them.
///
/// `weapons` holds the top authored priority and asks for 4 on a hull that
/// caps it at 2, so the two points it may not have fall through to `helm`
/// queued behind it. Read the ceiling wrong and the same 8 points land the
/// exact inverse — weapons 4, helm 2 — which is what makes the authored
/// number observable here rather than merely present.
#[test]
fn the_authored_group_max_level_caps_a_grant_and_the_rest_falls_through() {
    use crate::modifiers::power_system::{
        HELM_POWER_GROUP, SHIELDS_POWER_GROUP, WEAPONS_POWER_GROUP,
    };
    let policy = power_policy(
        &[],
        vec![
            alloc_rule(20, WEAPONS_POWER_GROUP, "true", 4),
            alloc_rule(10, HELM_POWER_GROUP, "true", 4),
            alloc_rule(5, SHIELDS_POWER_GROUP, "true", 4),
        ],
    );
    let (mut app, e) = over_budget_power_app(policy);
    app.world_mut()
        .entity_mut(e)
        .insert(crate::ship_plugin::ShipConfigComponent(
            weapons_capped_ship_config(),
        ));

    power_tick_with_dt(&mut app, 0.1);

    assert_eq!(
        commanded(&app, e, WEAPONS_POWER_GROUP),
        2,
        "the top-priority bid asked for 4 and its own hull caps it at 2"
    );
    assert_eq!(
        commanded(&app, e, HELM_POWER_GROUP),
        4,
        "the points weapons was not allowed to take fall through to the next bid"
    );
    assert_eq!(commanded(&app, e, SHIELDS_POWER_GROUP), 1);
    assert_eq!(commanded(&app, e, "ops"), 1);
    assert_eq!(commanded_total(&app, e), 8);
}

#[test]
fn ai_power_allocation_skips_ships_without_ai_high_fidelity() {
    let mut app = power_test_app();
    let e = power_ship_entity(&mut app);
    app.world_mut().entity_mut(e).remove::<AiHighFidelity>();
    app.world_mut()
        .entity_mut(e)
        .get_mut::<crate::ship::state::ShipRedAlert>()
        .unwrap()
        .0 = true;

    let before = power_level(&app, e, crate::modifiers::power_system::WEAPONS_POWER_GROUP);
    power_tick_with_dt(&mut app, 0.1);
    let after = power_level(&app, e, crate::modifiers::power_system::WEAPONS_POWER_GROUP);
    assert_eq!(
        before, after,
        "ships without AiHighFidelity must not be touched by ai_power_allocation"
    );
}

#[test]
fn human_control_source_holds_and_regains_cleanly() {
    // AC5 human Control Source + lifecycle reset: while a human holds the
    // Power reactor the AI stands down (allocation unchanged). Because the
    // decision is stateless there is nothing to reset — the very next tick
    // after AI control is regained yields a clean decision from the fresh
    // snapshot.
    let mut app = power_test_app();
    let e = power_ship_entity(&mut app);
    app.world_mut()
        .entity_mut(e)
        .get_mut::<crate::ship::state::ShipRedAlert>()
        .unwrap()
        .0 = true;
    app.world_mut()
        .entity_mut(e)
        .get_mut::<ShipSystemControlSources>()
        .unwrap()
        .0
        .set(
            crate::ship::system_registry::power_reactor_system_id(),
            ControlSource::Human,
        );

    let before = power_level(&app, e, crate::modifiers::power_system::WEAPONS_POWER_GROUP);
    power_tick_with_dt(&mut app, 0.1);
    assert_eq!(
        before,
        power_level(&app, e, crate::modifiers::power_system::WEAPONS_POWER_GROUP),
        "human-operated power reactor must not be touched by ai_power_allocation"
    );

    // Hand back to AI: a clean decision this tick, no stale carry-over.
    app.world_mut()
        .entity_mut(e)
        .get_mut::<ShipSystemControlSources>()
        .unwrap()
        .0
        .set(
            crate::ship::system_registry::power_reactor_system_id(),
            ControlSource::Ai,
        );
    power_tick_with_dt(&mut app, 0.1);
    assert_eq!(
        power_level(&app, e, crate::modifiers::power_system::WEAPONS_POWER_GROUP),
        3,
        "regaining AI control yields a clean elevate decision (stateless reset)"
    );
}

#[test]
fn emits_admitted_set_power_group_allocation_and_skips_no_ops() {
    // AC: the decide system emits its reallocation as an admitted
    // `SetPowerGroupAllocation` targeting the reactor, and a saturated no-op
    // (target == current) is NOT re-admitted every tick.
    let mut emit_app = App::new();
    crate::ai::host::register_ai_host_env(&mut emit_app);
    emit_app
        .insert_resource(Time::<()>::default())
        .init_resource::<crate::ship::power::PowerConfigResource>()
        .insert_resource(crate::lobby::Sessions(
            crate::lobby::session::SessionManager::new(),
        ))
        .add_systems(Update, ai_power_allocation);
    let mut cs = ShipSystemControlSources::default();
    cs.0.set(
        crate::ship::system_registry::power_reactor_system_id(),
        ControlSource::Ai,
    );
    let ee = emit_app
        .world_mut()
        .spawn((
            crate::server_app::Ship,
            cs,
            crate::ship::power::ShipPowerSystem(
                crate::modifiers::power_system::PowerSystem::default(),
            ),
            crate::ship::state::ShipRedAlert(true),
            crate::ship_plugin::LastHelmInput::default(),
            default_power_policy(),
            crate::core::messages::AdmittedCommands::default(),
            AiHighFidelity,
        ))
        .id();
    {
        let mut time = emit_app.world_mut().resource_mut::<Time>();
        time.advance_by(std::time::Duration::from_secs_f32(0.1));
    }
    emit_app.update();

    let admitted = emit_app
        .world()
        .entity(ee)
        .get::<crate::core::messages::AdmittedCommands>()
        .unwrap();
    let weapons_alloc = admitted.0.iter().find_map(|c| match &c.payload {
        crate::core::messages::SystemControlPayload::SetPowerGroupAllocation { group, level }
            if c.target == crate::ship::system_registry::power_reactor_system_id()
                && group.0 == crate::modifiers::power_system::WEAPONS_POWER_GROUP =>
        {
            Some(*level)
        }
        _ => None,
    });
    assert_eq!(
        weapons_alloc,
        Some(3),
        "red alert must admit an absolute SetPowerGroupAllocation(3) for weapons"
    );

    // Saturate weapons at the emitted level and clear admissions: a further
    // tick produces the same target and must NOT re-admit a no-op.
    {
        let mut ent = emit_app.world_mut().entity_mut(ee);
        ent.get_mut::<crate::ship::power::ShipPowerSystem>()
            .unwrap()
            .0
            .set_group_allocation(
                &crate::core::messages::PowerGroupId(
                    crate::modifiers::power_system::WEAPONS_POWER_GROUP.into(),
                ),
                3,
            )
            .unwrap();
        ent.get_mut::<crate::core::messages::AdmittedCommands>()
            .unwrap()
            .0
            .clear();
    }
    {
        let mut time = emit_app.world_mut().resource_mut::<Time>();
        time.advance_by(std::time::Duration::from_secs_f32(0.1));
    }
    emit_app.update();
    let admitted = emit_app
        .world()
        .entity(ee)
        .get::<crate::core::messages::AdmittedCommands>()
        .unwrap();
    assert!(
        !admitted.0.iter().any(|c| matches!(
            &c.payload,
            crate::core::messages::SystemControlPayload::SetPowerGroupAllocation { group, .. }
                if group.0 == crate::modifiers::power_system::WEAPONS_POWER_GROUP
        )),
        "a group already at the target level must not re-admit a no-op"
    );
}

#[test]
fn ai_power_reallocation_dual_writes_resource_for_local_ship() {
    // When the AI path reallocates power for the LocalShip, the admitted
    // command flows through `handle_power_messages` — the single applier —
    // which dual-writes the legacy global `ShipPowerSystem` Resource.
    let mut app = power_test_app();
    let e = power_ship_entity(&mut app);
    app.world_mut()
        .entity_mut(e)
        .insert(crate::server_app::LocalShip);
    app.world_mut()
        .insert_resource(crate::ship::power::ShipPowerSystem(
            crate::modifiers::power_system::PowerSystem::default(),
        ));
    app.world_mut()
        .entity_mut(e)
        .get_mut::<crate::ship::state::ShipRedAlert>()
        .unwrap()
        .0 = true;

    power_tick_with_dt(&mut app, 0.1);

    let component_level = power_level(&app, e, crate::modifiers::power_system::WEAPONS_POWER_GROUP);
    let resource_level = app
        .world()
        .resource::<crate::ship::power::ShipPowerSystem>()
        .0
        .level_for(&crate::core::messages::PowerGroupId(
            crate::modifiers::power_system::WEAPONS_POWER_GROUP.into(),
        ));
    assert_eq!(component_level, resource_level);
    assert_eq!(resource_level, 3);
}

#[test]
fn two_ships_with_different_authored_group_layouts_allocate_independently() {
    // AC1 + AC4 + AC6 per-ship isolation: two AI ships carry DIFFERENT
    // authored group layouts. Ship A (helm/weapons/sensors/ops) elevates its
    // `ops` group on thrust; ship B (canonical three) elevates `sensors` on
    // thrust. Under identical thrust each nudges only its own authored group,
    // proving the channels are per-ship data, not a shared catalogue.
    let mut app = App::new();
    crate::ai::host::register_ai_host_env(&mut app);
    app.insert_resource(Time::<()>::default())
        .init_resource::<crate::ship::power::PowerConfigResource>()
        .insert_resource(crate::lobby::Sessions(
            crate::lobby::session::SessionManager::new(),
        ))
        .add_systems(
            Update,
            (
                ai_power_allocation.before(crate::ship::power::handle_power_messages),
                crate::ship::power::handle_power_messages,
            ),
        );

    let spawn = |app: &mut App, groups: &[(&str, u8)], policy: PowerAiPolicy| -> Entity {
        let mut cs = ShipSystemControlSources::default();
        cs.0.set(
            crate::ship::system_registry::power_reactor_system_id(),
            ControlSource::Ai,
        );
        let seed: Vec<(crate::core::messages::PowerGroupId, u8)> = groups
            .iter()
            .map(|(g, l)| (crate::core::messages::PowerGroupId(g.to_string()), *l))
            .collect();
        app.world_mut()
            .spawn((
                crate::server_app::Ship,
                cs,
                crate::ship::power::ShipPowerSystem(
                    crate::modifiers::power_system::PowerSystem::from_authored_groups(
                        &crate::modifiers::power_system::PowerConfig::default(),
                        &seed,
                    ),
                ),
                crate::ship::state::ShipRedAlert::default(),
                crate::ship::helm::ThrustInput(0.9),
                policy,
                crate::core::messages::AdmittedCommands::default(),
                AiHighFidelity,
            ))
            .id()
    };

    let ops_ship = spawn(
        &mut app,
        &[("helm", 2), ("weapons", 2), ("sensors", 1), ("ops", 1)],
        power_policy(
            &[("reserve", 10.0)],
            vec![alloc_rule(
                10,
                "ops",
                "fact(thrust) >= 0.7 and fact(battery_pct) >= param(reserve)",
                3,
            )],
        ),
    );
    let sensors_ship = spawn(
        &mut app,
        &[("helm", 2), ("weapons", 2), ("sensors", 2)],
        power_policy(
            &[("reserve", 10.0)],
            vec![alloc_rule(
                10,
                "sensors",
                "fact(thrust) >= 0.7 and fact(battery_pct) >= param(reserve)",
                4,
            )],
        ),
    );

    power_tick_with_dt(&mut app, 0.1);

    assert_eq!(power_level(&app, ops_ship, "ops"), 3, "ops ship raises ops");
    assert_eq!(
        power_level(&app, ops_ship, "sensors"),
        1,
        "ops ship leaves sensors alone"
    );
    assert_eq!(
        power_level(&app, sensors_ship, "sensors"),
        4,
        "sensors ship raises sensors to its authored level"
    );
    assert_eq!(
        power_level(&app, sensors_ship, "helm"),
        2,
        "sensors ship leaves helm alone"
    );
}

#[test]
fn highest_priority_matching_rule_wins_on_one_group() {
    // AC4 conflicting rules on ONE group: two rules target `weapons`, both
    // firing this tick. The higher-priority rule's absolute level wins.
    let mut app = power_test_app();
    let e = power_ship_entity(&mut app);
    app.world_mut().entity_mut(e).insert(power_policy(
        &[("reserve", 10.0)],
        vec![
            // Low priority: hold weapons at 2.
            alloc_rule(0, "weapons", "true", 2),
            // High priority: on red alert, elevate to 4 — this must win.
            alloc_rule(
                10,
                "weapons",
                "fact(red_alert) > 0 and fact(battery_pct) >= param(reserve)",
                4,
            ),
        ],
    ));
    app.world_mut()
        .entity_mut(e)
        .get_mut::<crate::ship::state::ShipRedAlert>()
        .unwrap()
        .0 = true;

    power_tick_with_dt(&mut app, 0.1);

    assert_eq!(
        power_level(&app, e, crate::modifiers::power_system::WEAPONS_POWER_GROUP),
        4,
        "the highest-priority matching weapons rule wins the channel"
    );
}

#[test]
fn authored_guard_fires_from_seeded_facts() {
    // The #779 empty-facts lesson, applied to power: an authored `fact(...)`
    // guard actually fires because the host SEEDS the fact. Here a guard on
    // the seeded `total_allocation` ship fact elevates shields only once the
    // total crosses the authored threshold.
    let mut app = power_test_app();
    let e = power_ship_entity(&mut app);
    // Default seed totals 2+2+2 = 6. A guard `total_allocation >= 6` fires;
    // `>= 7` would not.
    app.world_mut().entity_mut(e).insert(power_policy(
        &[],
        vec![alloc_rule(10, "shields", "fact(total_allocation) >= 6", 3)],
    ));

    power_tick_with_dt(&mut app, 0.1);
    assert_eq!(
        power_level(&app, e, crate::modifiers::power_system::SHIELDS_POWER_GROUP),
        3,
        "a guard reading the seeded total_allocation fact fires"
    );
}

#[test]
fn scenario_flag_guard_gates_allocation() {
    // AC3: a rule may read read-only SCENARIO flags. Weapons stays at
    // baseline until a world flag is set; once the `WorldContentRuntime`
    // flag chain carries it, the same tick elevates weapons.
    let mut app = App::new();
    crate::ai::host::register_ai_host_env(&mut app);
    app.insert_resource(Time::<()>::default())
        .init_resource::<crate::ship::power::PowerConfigResource>()
        .init_resource::<crate::world::server::WorldContentRuntime>()
        .insert_resource(crate::lobby::Sessions(
            crate::lobby::session::SessionManager::new(),
        ))
        .add_systems(
            Update,
            (
                ai_power_allocation.before(crate::ship::power::handle_power_messages),
                crate::ship::power::handle_power_messages,
            ),
        );
    let mut cs = ShipSystemControlSources::default();
    cs.0.set(
        crate::ship::system_registry::power_reactor_system_id(),
        ControlSource::Ai,
    );
    let e = app
        .world_mut()
        .spawn((
            crate::server_app::Ship,
            cs,
            crate::ship::power::ShipPowerSystem(
                crate::modifiers::power_system::PowerSystem::default(),
            ),
            crate::ship::state::ShipRedAlert::default(),
            crate::ship_plugin::LastHelmInput::default(),
            power_policy(
                &[],
                vec![alloc_rule(10, "weapons", "flag(battle_stations)", 4)],
            ),
            crate::core::messages::AdmittedCommands::default(),
            AiHighFidelity,
        ))
        .id();

    // Flag unset → weapons holds at its seeded baseline 2.
    power_tick_with_dt(&mut app, 0.1);
    assert_eq!(
        power_level(&app, e, crate::modifiers::power_system::WEAPONS_POWER_GROUP),
        2,
        "with the scenario flag unset the guard does not fire"
    );

    // Set the scenario flag → the same guard now fires and elevates weapons.
    app.world_mut()
        .resource_mut::<crate::world::server::WorldContentRuntime>()
        .flags
        .set_flag("battle_stations");
    power_tick_with_dt(&mut app, 0.1);
    assert_eq!(
        power_level(&app, e, crate::modifiers::power_system::WEAPONS_POWER_GROUP),
        4,
        "once the scenario flag is set the guard fires (AC3 read-only flags)"
    );
}

/// Issue #891 stage 2, the LAYERING half: the chain a host passes is
/// anchored at the layer that spawned the ship — not flattened onto the
/// base store. A flag set only in the spawning LAYER's store fires the
/// ship's guard, and a `parent:`-prefixed guard reads the base store from
/// there, exactly as a trigger authored in that layer would.
#[test]
fn scenario_flag_chain_is_anchored_at_the_ships_spawning_layer() {
    let mut app = App::new();
    crate::ai::host::register_ai_host_env(&mut app);
    app.insert_resource(Time::<()>::default())
        .init_resource::<crate::ship::power::PowerConfigResource>()
        .init_resource::<crate::world::server::WorldContentRuntime>()
        .init_resource::<crate::world::server::WorldLayerMap>()
        .insert_resource(crate::lobby::Sessions(
            crate::lobby::session::SessionManager::new(),
        ))
        .add_systems(
            Update,
            (
                ai_power_allocation.before(crate::ship::power::handle_power_messages),
                crate::ship::power::handle_power_messages,
            ),
        );
    let mut cs = ShipSystemControlSources::default();
    cs.0.set(
        crate::ship::system_registry::power_reactor_system_id(),
        ControlSource::Ai,
    );
    let e = app
        .world_mut()
        .spawn((
            crate::server_app::Ship,
            cs,
            crate::ship::power::ShipPowerSystem(
                crate::modifiers::power_system::PowerSystem::default(),
            ),
            crate::ship::state::ShipRedAlert::default(),
            crate::ship_plugin::LastHelmInput::default(),
            power_policy(
                &[],
                vec![
                    alloc_rule(10, "weapons", "flag(layer_flag)", 4),
                    // A DROP, so the ship-wide total cap cannot mask the
                    // read: two simultaneous elevations would fight the
                    // total-allocation clamp.
                    alloc_rule(10, "shields", "flag(parent:base_flag)", 1),
                    // The mirror-image case, driven through the real host
                    // rather than asserted against a hand-built chain
                    // (issue #891 review finding 4): an UNPREFIXED guard
                    // on `base_flag` must NOT fire for this layer-spawned
                    // ship. `resolve_chain` indexes by depth, so an
                    // unprefixed name reads chain[0] — the spawning
                    // layer's own store — and `base_flag` lives only in
                    // the base store two hops further out.
                    alloc_rule(10, "helm", "flag(base_flag)", 3),
                ],
            ),
            crate::core::messages::AdmittedCommands::default(),
            AiHighFidelity,
        ))
        .id();

    // The ship was spawned by a loaded sub-world layer; the layer's OWN
    // store carries `layer_flag`, the BASE store carries `base_flag`.
    // Stamping `EntityOriginLayer` mirrors what the two real spawn sites
    // do (issue #891 review finding 1) — `entity_flag_chain` now reads
    // the origin off this component, not off `spawned_entities`.
    {
        let mut layer = crate::world::server::WorldRuntime::default();
        layer.flags.set_flag("layer_flag");
        layer.spawned_entities.push(e);
        app.world_mut()
            .resource_mut::<crate::world::server::WorldLayerMap>()
            .0
            .insert("assets/worlds/sub.toml".to_string(), layer);
        app.world_mut()
            .resource_mut::<crate::world::server::WorldContentRuntime>()
            .flags
            .set_flag("base_flag");
        app.world_mut()
            .entity_mut(e)
            .insert(crate::world::server::EntityOriginLayer(
                "assets/worlds/sub.toml".to_string(),
            ));
    }

    power_tick_with_dt(&mut app, 0.1);
    assert_eq!(
        power_level(&app, e, crate::modifiers::power_system::WEAPONS_POWER_GROUP),
        4,
        "a flag set in the SPAWNING LAYER's store fires the layer-spawned \
         ship's guard — the chain is anchored at the layer, not the base"
    );
    assert_eq!(
        power_level(&app, e, crate::modifiers::power_system::SHIELDS_POWER_GROUP),
        1,
        "a `parent:`-prefixed guard climbs from the layer to the BASE store \
         — the chain is layered, not flattened"
    );
    assert_eq!(
        power_level(&app, e, crate::modifiers::power_system::HELM_POWER_GROUP),
        2,
        "an UNPREFIXED guard on a layer-spawned ship reads the layer store, \
         not the base store — base_flag never reaches it, so the rule does \
         not fire and helm holds its default level"
    );
}

#[test]
fn idle_policy_holds_every_group() {
    // A ship whose policy is an explicit idle takes no power action.
    let mut app = power_test_app();
    let e = power_ship_entity(&mut app);
    app.world_mut()
        .entity_mut(e)
        .insert(PowerAiPolicy(crate::ai::policy::AiPolicy {
            idle: true,
            ..Default::default()
        }));
    app.world_mut()
        .entity_mut(e)
        .get_mut::<crate::ship::state::ShipRedAlert>()
        .unwrap()
        .0 = true;
    app.world_mut()
        .entity_mut(e)
        .get_mut::<crate::ship::helm::ThrustInput>()
        .unwrap()
        .0 = 0.9;

    power_tick_with_dt(&mut app, 0.1);
    assert_eq!(
        power_level(&app, e, crate::modifiers::power_system::WEAPONS_POWER_GROUP),
        2
    );
    assert_eq!(
        power_level(&app, e, crate::modifiers::power_system::HELM_POWER_GROUP),
        2
    );
}

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
