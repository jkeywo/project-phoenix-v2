use bevy::prelude::*;

use crate::core::messages::{
    CoordinationPayload, ModifierSlot, SystemBlackboard, WeaponEmitterArc, WeaponFamily,
    WeaponTargetGeometry, WeaponsBlackboard,
};
use crate::entities::spawner::FactionComponent;
use crate::server_app::AsteroidUuid;
use crate::ship::state::ShipPhysics;
use crate::ship_plugin::{CoordinationEnqueue, ShipSystemControlSources};
use crate::weapons::torpedo::{TorpedoConfig, TorpedoSystem};

/// Delay before NPC tactical AI auto-matches phaser frequency to the locked
/// target's shield frequency (seconds). Defined here as a tuning constant
/// rather than an inline literal (code review finding #679).
const NPC_FREQ_MATCH_DELAY: f32 = 2.0;

// ── Resources ─────────────────────────────────────────────────────────────

/// Rendering config for the phaser beam (colour, max range).
/// Populated from ship entity TOML during world setup; defaults are used if
/// the TOML is absent.
///
/// Derives both `Resource` (existing player-ship singleton path) and
/// `Component` (per-entity path, PR 5 unification).
#[derive(Resource, Component, Clone, Debug)]
pub struct PhaserRenderConfig {
    /// RGBA beam colour in 0.0–1.0.
    pub beam_color: [f32; 4],
    /// Maximum beam range (world units); beam endpoint is clamped to this.
    pub beam_range: f32,
}

impl Default for PhaserRenderConfig {
    fn default() -> Self {
        Self {
            beam_color: crate::weapons::beam_render::DEFAULT_BEAM_COLOR,
            beam_range: 40.0,
        }
    }
}

/// Bevy message fired (with world-space position) when an asteroid is destroyed
/// by phaser fire. The renderer uses this to spawn a ripple VFX at the site.
#[derive(Message, Clone, Debug)]
pub struct AsteroidDestroyedVfx {
    pub x: f32,
    pub z: f32,
}

/// Bevy message fired when a non-asteroid combat target is destroyed.
#[derive(Message, Clone, Copy, Debug)]
pub struct ShipDestroyedVfx {
    pub x: f32,
    pub z: f32,
    pub radius: f32,
}

/// Fallback explosion radius for targets without collider configuration.
pub const DEFAULT_SHIP_EXPLOSION_RADIUS: f32 = 3.0;

// ── Plugin ─────────────────────────────────────────────────────────────────

/// Per-ship frequency match state for NPC auto-match frequency AI.
#[derive(Resource, Default)]
pub struct NpcFrequencyMatchStates(
    pub std::collections::HashMap<Entity, crate::console_ai::FrequencyMatchState>,
);

pub struct WeaponsPlugin;

impl Plugin for WeaponsPlugin {
    fn build(&self, app: &mut App) {
        use crate::command_admission::{ConsumerMatcher, RegisterAdmittedConsumer};
        // Weapons owns its delta cache and ownership-filtered reconnect
        // projection beside the live publisher in `blackboard.rs`. The generic
        // lifecycle runner receives only the registered callbacks.
        super::blackboard::register_weapons_replication_lifecycle(app);
        // The AI host spine's read-only world context (issue #1207): the phaser,
        // blaster, torpedo, doctrine and tactical-selector hosts in this plugin
        // now consume `AiHostEnv`, a bare-`Res` param, so every app that runs
        // them must register the same resources production does. Idempotent and
        // mirrored by `ConsoleAiPlugin` / `ship::test_support`.
        crate::ai::host::register_ai_host_env(app);
        // Admitted-command consumers (issue #833, expanded by #846): every
        // weapons command — fire, load, unload, target selection, phaser
        // control — now travels as a `ControlSystem` envelope through the
        // admission seam. Each phaser bank (`phaser-{bank}`), torpedo tube
        // (`torpedo-tube-{id}`), and blaster bank (`blaster-{bank}`) is a
        // routed admitted consumer. Prefix registration mirrors the canonical
        // forward mappings for every authored bank/tube id and also covers
        // `SetTorpedoVolleyTarget`.
        app.register_admitted_consumer(ConsumerMatcher::exact(
            crate::ship::system_registry::TACTICAL_RADAR_KIND,
            crate::ship::system_registry::TACTICAL_RADAR_SYSTEM_ID,
        ))
        .register_admitted_consumer(ConsumerMatcher::exact(
            crate::ship::system_registry::PHASER_CONTROL_KIND,
            crate::ship::system_registry::PHASER_CONTROL_SYSTEM_ID,
        ))
        .register_admitted_consumer(ConsumerMatcher::prefix(
            crate::ship::system_registry::TORPEDO_TUBE_KIND,
            "torpedo-tube-",
        ))
        .register_admitted_consumer(ConsumerMatcher::prefix(
            crate::ship::system_registry::PHASER_BANK_KIND,
            "phaser-",
        ))
        // Blaster banks converge on the admission seam (issue #781): both a
        // human's `ChargeBlasterStart` and the AI decider's emitted one travel
        // as admitted `blaster-{bank}` commands consumed by `handle_fire_blaster`.
        .register_admitted_consumer(ConsumerMatcher::prefix(
            crate::ship::system_registry::BLASTER_BANK_KIND,
            "blaster-",
        ));
        app.init_resource::<crate::core::messages::InterSystemQueue>();
        // The ONE shared AI decision cadence (issue #889): the three AI
        // deciders registered below (`ai_phaser_auto_fire`,
        // `ai_target_selection`, `tick_blaster_auto_fire`) were ungated, i.e.
        // deciding once per rendered frame.
        crate::ai::cadence::register_ai_cadence(app);
        app.init_resource::<CurrentPhaserMode>()
            .init_resource::<PhaserRenderConfig>()
            .init_resource::<PhaserCombatConfigResource>()
            .init_resource::<NpcFrequencyMatchStates>()
            .init_resource::<BlasterSystemResource>()
            .init_resource::<BeamContext>()
            .init_resource::<TorpedoTargetSnapshot>()
            .insert_resource(TorpedoSystemResource(TorpedoSystem::new(
                TorpedoConfig::default(),
            )))
            .add_message::<AsteroidDestroyedVfx>()
            .add_message::<ShipDestroyedVfx>()
            .add_message::<CoordinationEnqueue>()
            .add_message::<crate::ship_plugin::DeliveredCoordination>()
            .add_observer(on_beam_started)
            .add_observer(on_beam_ended)
            .add_systems(
                FixedUpdate,
                (
                    // `handle_set_target` is the SOLE writer of
                    // `TacticalRadarSelection` (issue #887). Both origins reach it
                    // as an admitted `SetTarget` on `tactical-radar`: a human's
                    // console message via `admit_system_commands` (before `Input`),
                    // and the Tactical AI's own choice via `ai_target_selection`'s
                    // `emit_ai_command` (in `Input`, below). It therefore runs
                    // AFTER the decider — an admitted command emitted in `Input`
                    // has to be consumed later in the SAME tick, because
                    // admission's `clear_before_input` empties the queue at the
                    // top of the next one. This is the ordinary
                    // decide-then-apply shape the other weapons commands already
                    // use (`ai_phaser_auto_fire` → `handle_fire_phaser`), just
                    // with both halves in `Input` so the applied lock is still
                    // pre-physics.
                    //
                    // The two origins can never collide, and that is what makes
                    // one applier safe: `accept_human_input` and `operate_ai` are
                    // mutually exclusive on a single `SystemId`, and both paths now
                    // gate on the SAME id — `tactical-radar`. A Human radar refuses
                    // the AI's emit at admission and skips the decider outright; an
                    // Ai radar refuses the human's message at admission. There is
                    // no ordering in which one clobbers the other, because only one
                    // of them ever produces a command.
                    handle_set_target
                        .in_set(crate::sim_sets::SimSet::Input)
                        .after(ai_target_selection),
                    // Phaser auto-fire DECIDE (issue #846): emits to
                    // AdmittedCommands through the shared AI seam. Stays in
                    // `Input` so it keeps reading pre-physics `Transform`s.
                    // Gated on the ONE shared AI cadence (issue #889): before
                    // it, this ran once per rendered frame.
                    ai_phaser_auto_fire
                        .in_set(crate::sim_sets::SimSet::Input)
                        .run_if(crate::ai::cadence::ai_tick_ready),
                    // Weapons DOCTRINE decide (issue #956): resolves the
                    // ship's authored arc-bearing rank ladder and asks Helm to
                    // turn. Since #956 this is an AI POLICY HOST
                    // (`ai_flag_hosts::WEAPONS_DOCTRINE`), so it takes the same
                    // gate as the three deciders it shares this tuple with —
                    // AGENTS.md rule 7: every policy host decides on the ONE
                    // shared cadence, not once per logical tick. Nothing here
                    // is exempt from that: the
                    // withdrawal and the debounce are both derived from the
                    // same resolved order, so running them between decisions
                    // would only let a family re-qualify against an order
                    // nobody re-resolved. Stays in `Input` so it keeps reading
                    // pre-physics `Transform`s, like the other three.
                    tick_weapons_arc_request
                        .in_set(crate::sim_sets::SimSet::Input)
                        .run_if(crate::ai::cadence::ai_tick_ready),
                    handle_set_phaser_mode.in_set(crate::sim_sets::SimSet::Input),
                    handle_set_phaser_frequency.in_set(crate::sim_sets::SimSet::Input),
                    handle_set_torpedo_volley_target.in_set(crate::sim_sets::SimSet::Input),
                    // Tactical AI target selection (issues #697, #700, #887).
                    //
                    // Stays in `SimSet::Input` — where the pre-split
                    // `operate_tactical_ai` lived — rather than moving to the
                    // `Physics` + `AiTickLabel` set that ConsoleAiPlugin's
                    // decide/integrate pairs use. Since #887 it no longer WRITES
                    // `TacticalRadarSelection`; it emits an admitted `SetTarget` on
                    // `tactical-radar` that `handle_set_target` applies later in the
                    // same set (the `.after` edge above). Keeping both halves in
                    // `Input` is what makes the lock land pre-physics, on the tick
                    // it was decided, exactly as the direct write used to.
                    ai_target_selection
                        .in_set(crate::sim_sets::SimSet::Input)
                        .run_if(crate::ai::cadence::ai_tick_ready),
                    tick_npc_auto_match_frequency.in_set(crate::sim_sets::SimSet::Input),
                    // Applies a Sensors frequency hint a backfilled Tactical
                    // consumed off the channel-3 bus last tick (issue #873).
                    // Ordered AFTER the omniscient auto-match so an advisory
                    // that actually arrived is the value that sticks: the bus
                    // is the modelled information path, the auto-match is the
                    // fallback for a ship with nobody on Sensors at all.
                    apply_tactical_frequency_hint
                        .in_set(crate::sim_sets::SimSet::Input)
                        .after(tick_npc_auto_match_frequency),
                    receive_tactical_coordination
                        .in_set(crate::sim_sets::SimSet::Modifiers)
                        .after(crate::ship_plugin::process_coordination_lag),
                    // Blaster auto-fire DECIDE (issue #781): emits an admitted
                    // `ChargeBlasterStart` through the shared AI seam, converging
                    // with the human path at `handle_fire_blaster` (Physics).
                    // Stays in `Input` so it reads pre-physics `Transform`s.
                    tick_blaster_auto_fire
                        .in_set(crate::sim_sets::SimSet::Input)
                        .run_if(crate::ai::cadence::ai_tick_ready),
                ),
            )
            .add_systems(
                FixedUpdate,
                (
                    // Beam tick split into three phases (issue #723), connected
                    // by the one-tick `BeamContext` resource: prepare writes it,
                    // apply-damage reads/mutates it, tick-lifetimes reads it.
                    // Explicit `.chain()` edges keep the three deterministic
                    // within `SimSet::Damage`. Instance-based `.chain()` rather
                    // than type-set `.after(...)` edges (the
                    // system-type ordering style) because the
                    // weapons test harness registers a second instance of each
                    // phase, which would make a `SystemTypeSet` ordering
                    // ambiguous and panic at schedule build.
                    (
                        tick_beams_prepare,
                        tick_beams_apply_damage,
                        tick_beams_tick_lifetimes,
                    )
                        .chain()
                        .in_set(crate::sim_sets::SimSet::Damage),
                    handle_blaster_hits.in_set(crate::sim_sets::SimSet::Damage),
                    // Weapons fire/load consumers (issue #846): read per-ship
                    // `AdmittedCommands` that the AI deciders (SimSet::Input)
                    // wrote this tick. Running here in Physics means admission's
                    // `clear_before_input` has already run, and the AI deciders
                    // (Input) have already emitted — so commands from either
                    // origin survive the tick.
                    handle_fire_phaser.in_set(crate::sim_sets::SimSet::Physics),
                    handle_fire_torpedo.in_set(crate::sim_sets::SimSet::Physics),
                    handle_load_tube.in_set(crate::sim_sets::SimSet::Physics),
                    handle_unload_tube.in_set(crate::sim_sets::SimSet::Physics),
                    // Torpedo tick split into two phases (issue #724),
                    // connected by the one-tick `TorpedoTargetSnapshot`
                    // resource: the builder writes it, the lifecycle reads
                    // it. Instance-based `.chain()` rather than type-set
                    // `.after(...)` for the same reason as the beam-tick
                    // chain above: the weapons test harness registers a
                    // second instance of each phase.
                    (build_torpedo_target_snapshot, tick_torpedo_lifecycle)
                        .chain()
                        .in_set(crate::sim_sets::SimSet::Physics),
                    // Magazine consumer runs in Physics — reads channel-2 claims
                    // that handle_load_tube emitted this tick (both now in
                    // Physics, issue #846). Ordered after handle_load_tube and
                    // build_torpedo_target_snapshot / tick_torpedo_lifecycle so
                    // its own state mutations are seen.
                    handle_torpedo_magazine_inter_system
                        .in_set(crate::sim_sets::SimSet::Physics)
                        .after(handle_load_tube),
                    // Blaster fire CONSUME (issue #781): reads per-ship
                    // `AdmittedCommands` that both the human (via
                    // `admit_system_commands`) and the AI decider
                    // (`tick_blaster_auto_fire`, Input) wrote this tick, and arms
                    // the volley. Ordered before `tick_blaster_system` so the
                    // armed volley launches the same tick (mirrors the Input →
                    // Physics phaser flow).
                    handle_fire_blaster
                        .in_set(crate::sim_sets::SimSet::Physics)
                        .before(tick_blaster_system),
                    tick_blaster_system.in_set(crate::sim_sets::SimSet::Physics),
                ),
            )
            .add_systems(
                FixedUpdate,
                // One system per blackboard type (issue #725). Each writes
                // disjoint ShipSystemBlackboards keys, so there is no
                // ordering dependency between them — bare tuple, no chain().
                (
                    publish_weapons_core_blackboard,
                    publish_tactical_radar_blackboard,
                    publish_phaser_bank_blackboards,
                    publish_torpedo_tube_blackboards,
                    publish_torpedo_magazine_blackboard,
                )
                    .in_set(crate::sim_sets::SimSet::Publish),
            );
    }
}

// ── Systems ─────────────────────────────────────────────────────────────────

// The weapon-family submodules live beside this adapter under `console::weapons`
// (issue #1186 moved the plugin here). Bring the module names into scope so the
// module-qualified references in this file (`blaster::…`, `torpedo::…`,
// `shared::…`) keep resolving now that the `pub mod` declarations sit in `mod.rs`.
use super::{blaster, shared, torpedo};

// Shared weapons utilities extracted to `shared.rs` (issue #721). Re-exported
// so `use super::*;` in the test module keeps resolving them.
pub(crate) use super::shared::{
    any_tactical_system_operates_ai, live_entity_xz, BeamContext, DirectFireGeometry,
    TorpedoTargetSnapshot,
};

// Blaster systems extracted to `blaster.rs` (issue #726). `BlasterSystemResource`
// stays `pub` here for external consumers (`src/server/pfx.rs`,
// `src/entities/spawner.rs` via the `weapons_plugin` alias); the systems are
// re-exported so the plugin build fn and the test module keep resolving them.
pub(crate) use super::blaster::{
    handle_blaster_hits, handle_fire_blaster, tick_blaster_auto_fire, tick_blaster_system,
};
pub use super::blaster::{seed_blaster_bank_facts, BlasterBankAiPolicies, BlasterSystemResource};

// Beam (phaser) types and systems extracted to `beam.rs` (issue #727). The
// public types stay re-exported here for external consumers; the systems are
// re-exported so the plugin build fn and the test module keep resolving them.
pub(crate) use super::beam::{
    ai_phaser_auto_fire, handle_fire_phaser, handle_set_phaser_frequency, handle_set_phaser_mode,
    handle_set_target, on_beam_ended, on_beam_started, tick_beams_apply_damage, tick_beams_prepare,
    tick_beams_tick_lifetimes,
};
pub use super::beam::{
    seed_phaser_bank_facts, ActiveBeam, BeamEndedEvent, BeamStartedEvent, CurrentPhaserMode,
    LastShipAttacker, PhaserBankAiPolicies, PhaserCombatConfigResource, PhaserCooldown,
    TacticalRadarSelection, TacticalTargetSelector, BEAM_DAMAGE_PER_SEC,
};

// Torpedo systems extracted to `torpedo.rs` (issue #728). `TorpedoSystemResource`
// and `handle_torpedo_magazine_inter_system` stay `pub` here for external
// consumers (`src/server_app.rs` chained re-exports, `src/server/pfx.rs`,
// `src/entities/spawner.rs`, `src/console_ai/server.rs`, and friends); the
// other systems are re-exported so the plugin build fn and the test module
// keep resolving them.
pub(crate) use super::torpedo::{
    build_torpedo_target_snapshot, handle_fire_torpedo, handle_load_tube,
    handle_set_torpedo_volley_target, handle_unload_tube, tick_torpedo_lifecycle,
};
pub use super::torpedo::{
    handle_torpedo_magazine_inter_system, seed_torpedo_conservation_facts,
    seed_torpedo_magazine_facts, seed_torpedo_tube_launch_facts, seed_torpedo_tube_load_facts,
    torpedo_conservation_declared, torpedo_conservation_policy_fires,
    torpedo_magazine_grant_policy_fires, torpedo_tube_launch_policy_fires,
    torpedo_tube_load_policy_fires, TorpedoMagazineAiPolicy, TorpedoSystemResource,
    TorpedoTubeAiPolicies,
};

// Blackboard publish systems, broadcaster, and cache resources extracted to
// `blackboard.rs` (issue #729). `LastWeaponsUpdate`, `compute_current_weapons_update`,
// and `weapons_update_broadcaster` stay `pub` here for external consumers
// (`src/server_app.rs` chained re-exports); the lifecycle callbacks stay owned
// privately beside them in `blackboard.rs`.
// The publish systems are re-exported so the plugin build fn and test module
// keep resolving them.
pub use super::blackboard::{
    compute_current_weapons_update, weapons_update_broadcaster, LastWeaponsUpdate,
};
pub(crate) use super::blackboard::{
    publish_phaser_bank_blackboards, publish_tactical_radar_blackboard,
    publish_torpedo_magazine_blackboard, publish_torpedo_tube_blackboards,
    publish_weapons_core_blackboard,
};

/// Tracks the last arc-bearing request Weapons asked Helm for, so the
/// channel-3 request only re-fires when the *condition* changes — a different
/// weapon family, a different target, or a change to the usable emitter arcs
/// (e.g. a bank going offline / a range modifier shifting) — rather than every
/// tick the same miss persists (issues #677, #767).
#[derive(Component, Default, Clone)]
pub struct WeaponsArcRequestState {
    pub last: Option<(WeaponFamily, String, Vec<WeaponEmitterArc>)>,
}

/// The ship's authored WEAPONS DOCTRINE policy (issue #956): the order in which
/// it presents its weapon families when the target is in range but outside every
/// arc of one.
///
/// Built at spawn from `[weapons_console.ai]` — the fleet's baseline lives in
/// `assets/entities/fragments/ai/fleet_baseline.toml`, so a hull with no
/// preference of its own resolves the AUTHORED baseline rather than an inline
/// Rust order. Strict AI-declaration mode rejects an AI-bearing hull that
/// authors none, so there is no Rust-side stand-in to fall back to.
#[derive(Component, Default, Clone, Debug)]
pub struct WeaponsDoctrineAiPolicy(pub crate::ai::policy::AiPolicy);

/// Seed the per-tick fact snapshot the WEAPONS DOCTRINE resolves its
/// arc-bearing rank channels over (issue #956).
///
/// Deliberately narrow. The two readings here are the ones a "which gun do I
/// turn to present?" decision is actually made on:
///
/// * `target_facing_shields` — HP of the one arc a round from this ship would
///   strike, resolved through the target's own arc router
///   ([`crate::weapons::shield::ShieldSystem::hp_facing_attacker`]), so this snapshot and
///   the tube's own launch guard ask the same question of the same number. It is
///   what makes "lead with the tubes once the screen is down" authorable, and it
///   was previously seeded for torpedo tubes ALONE — a phaser or blaster guard
///   could not see target shield state at all, so no cross-family preference
///   could be expressed.
/// * `red_alert` — this ship's own alert state, the same typed fact every fire
///   guard in the fleet is written against, so a doctrine can present a
///   different family before and after the captain calls stations. Since issue
///   #1041 it is seeded from the ship's whole firing
///   [`WeaponsAlertPosture`], so a hull under a weapons hold stops asking Helm
///   to swing a gun into line as well as holding its fire — one posture, one
///   value, rather than the same fact meaning two things on one ship in one
///   tick.
///
/// Every emitter's arc, range and online state is a HOST reading resolved after
/// the order comes back, never a fact: which families are *capable* is not a
/// decision, and a policy that could contradict the geometry would ask Helm to
/// turn for a gun that is not there.
pub fn seed_weapons_doctrine_facts(
    target_facing_shields: i32,
    posture: WeaponsAlertPosture,
) -> crate::world::flags::AiFacts {
    let mut facts = crate::world::flags::AiFacts::new();
    facts.set(
        crate::entities::config::TARGET_FACING_SHIELDS_FACT,
        target_facing_shields as f64,
    );
    facts.set(
        crate::entities::config::POWER_RED_ALERT_FACT,
        posture.alert_fact_value(),
    );
    facts
}

/// The ship-wide firing posture every weapons host seeds the `red_alert` fact
/// from (issue #1041): its Red Alert, and whether its captain has called a
/// **weapons hold**.
///
/// # Why a posture rather than a second fact
///
/// Issue #1041's acceptance criteria require the hold to compose with the
/// authored fire gate **with no new doctrine vocabulary**. Every armed hull in
/// the fleet writes exactly one gate —
/// `when = "fact(red_alert) >= param(min_alert_to_fire)"` — and differs only in
/// the threshold: an Alliance hull with a captain's console authors `1`, the
/// always-armed Harrow gun line authors `0` (issue #872). A second fact would
/// mean editing every one of those predicates to AND it in, which is precisely
/// the vocabulary change the AC forbids, and a hull whose author forgot would
/// silently ignore the captain's order.
///
/// So the hold is composed into the VALUE of the fact the gate already reads.
/// [`Self::alert_fact_value`] is the whole mechanism:
///
/// | posture | fact | `>= 0` (Harrow) | `>= 1` (Alliance) |
/// |---|---|---|---|
/// | stood down | `0.0` | fires | holds |
/// | red alert | `1.0` | fires | fires |
/// | **weapons hold** | [`WEAPONS_HOLD_ALERT_FACT`] | **holds** | **holds** |
///
/// Read the numbers as a LADDER rather than as a boolean with a sentinel bolted
/// on: `min_alert_to_fire` is a floor on how hot the ship must be before a bank
/// will open up, and a weapons hold is a rung BELOW stood-down — colder than
/// cold, because a stood-down ship is merely not expecting trouble while a held
/// one has been ordered not to shoot. Sitting under every authored floor is what
/// makes the lever work on the always-armed hulls too, which a `0.0` could not
/// do: `0 >= 0` is true, and a Harrow under orders would have kept firing. That
/// is the AI/human symmetry half of the AC — the NPC fire hosts respect the hold
/// identically because they read the same seeded fact through the same authored
/// predicate, not because anything checks who is flying.
///
/// # Why Red Alert's own behaviour cannot have moved
///
/// With the hold released this returns exactly `1.0` / `0.0` — bit for bit the
/// expression each host inlined before this issue. Nothing else in the fire path
/// changed, so a run in which nobody engages a hold folds the same digest it
/// always did; the committed world anchors are the standing proof.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WeaponsAlertPosture {
    /// This ship's own [`crate::ship::state::ShipRedAlert`].
    pub red_alert: bool,
    /// This ship's own [`crate::ship::state::ShipWeaponsHold`].
    pub weapons_hold: bool,
    /// A Command stance directing this weapons Station's alert posture (issue
    /// #1107), overriding the ship's own Red Alert for the fire gate. `None`
    /// means no Command stance is in force → the posture tracks `red_alert`
    /// exactly as before this issue, so an undirected hull stays byte-identical.
    pub stance_high_alert: Option<bool>,
}

/// The `red_alert` fact value seeded at a weapons host while the ship is under a
/// weapons hold — one rung BELOW stood-down, so it fails every `>= threshold`
/// gate a hull can author, the always-armed `0` included.
///
/// Not authored TOML on purpose: it is not a tunable but the bottom of the
/// ladder the authored `min_alert_to_fire` floors sit on, and a designer moving
/// it could only ever break the lever. `authored_ai_pins` pins every shipped
/// bank's threshold at or above zero so this stays true of the shipped fleet.
pub const WEAPONS_HOLD_ALERT_FACT: f64 = -1.0;

impl WeaponsAlertPosture {
    /// Read the posture off one ship's two optional components. Absent reads as
    /// "not at alert" / "not holding" — the same fail-open reading the hosts
    /// used before #1041, so a bare-`App` fixture that spawns neither behaves
    /// exactly as it did.
    pub fn from_components(
        red_alert: Option<&crate::ship::state::ShipRedAlert>,
        weapons_hold: Option<&crate::ship::state::ShipWeaponsHold>,
    ) -> Self {
        Self::from_parts(red_alert, weapons_hold, None)
    }

    /// As [`from_components`](Self::from_components) but with a Command stance
    /// override (issue #1107). `stance_high_alert: None` is exactly
    /// `from_components`, so every pre-#1107 call site keeps its behaviour.
    pub fn from_parts(
        red_alert: Option<&crate::ship::state::ShipRedAlert>,
        weapons_hold: Option<&crate::ship::state::ShipWeaponsHold>,
        stance_high_alert: Option<bool>,
    ) -> Self {
        Self {
            red_alert: red_alert.is_some_and(|r| r.0),
            weapons_hold: weapons_hold.is_some_and(|h| h.0),
            stance_high_alert,
        }
    }

    /// A posture with no hold — the shorthand every test written before #1041
    /// keeps working through, and the one the byte-identical claim rests on.
    pub fn alert(red_alert: bool) -> Self {
        Self {
            red_alert,
            weapons_hold: false,
            stance_high_alert: None,
        }
    }

    /// The value seeded for the `red_alert` fact. See the type docs for the
    /// ladder this implements.
    ///
    /// A weapons hold still wins over everything (the captain's restraint lever
    /// is absolute). Otherwise a Command stance override, when present, decides
    /// the posture in place of the ship's own Red Alert — this is the seam the
    /// migrated Red Alert fire branch travels through (issue #1107). With no
    /// override the reading is the pre-#1107 `red_alert` value, bit for bit.
    pub fn alert_fact_value(self) -> f64 {
        if self.weapons_hold {
            WEAPONS_HOLD_ALERT_FACT
        } else if let Some(high) = self.stance_high_alert {
            if high {
                1.0
            } else {
                0.0
            }
        } else if self.red_alert {
            1.0
        } else {
            0.0
        }
    }
}

/// Resolve the ship's authored weapon-family order for the channel-3 arc-bearing
/// request (issue #956) — the replacement for the Rust `[Phasers, Blasters,
/// Torpedoes]` array.
///
/// Walks the [`crate::entities::config::ARC_BEARING_CHANNELS`] rank ladder in
/// order, resolving each as an ordinary channel, and collects the family each
/// one names. Two properties matter and both are deliberate:
///
/// * **Repeats are dropped.** A doctrine that promotes a family into an earlier
///   rank while leaving the baseline rule on a later one is the natural way to
///   author a conditional preference, and it must not make the same family
///   qualify twice.
/// * **An unauthored (or held) rank is simply absent.** The order shortens; the
///   ship does not fall back to a Rust default for the empty slot. A doctrine
///   that names one family only turns for that one, which is a statement a hull
///   is entitled to make.
///
/// The `flags` chain is the caller's — this helper receives one already built,
/// like every other `*_policy_fires` helper — so authored `flag(...)` /
/// `counter(...)` guards on this host read live world state.
pub(crate) fn resolve_arc_bearing_order(
    policy: &crate::ai::policy::AiPolicy,
    facts: &crate::world::flags::AiFacts,
    flags: &[&crate::world::flags::FlagStore],
) -> Vec<WeaponFamily> {
    use crate::ai::policy::AiPolicyVerb;
    let mut order: Vec<WeaponFamily> = Vec::new();
    for channel in crate::entities::config::ARC_BEARING_CHANNELS {
        let family = match policy.resolve_channel(channel, facts, flags) {
            Some(AiPolicyVerb::BringPhasersToBear) => WeaponFamily::Phasers,
            Some(AiPolicyVerb::BringBlastersToBear) => WeaponFamily::Blasters,
            Some(AiPolicyVerb::BringTorpedoesToBear) => WeaponFamily::Torpedoes,
            // Content validation restricts these channels to the three family
            // verbs, so any other resolution is unreachable through the load
            // path; treated as "this rank says nothing" rather than panicking.
            _ => continue,
        };
        if !order.contains(&family) {
            order.push(family);
        }
    }
    order
}

/// One weapon emitter's inputs to the pure family-arc evaluation (issue #767):
/// its online/usable state, its family arc geometry against the current target,
/// and the arc/range Helm would need to turn toward.
struct EmitterArcInput {
    /// Emitter is not offline (disabled / destroyed).
    online: bool,
    /// Emitter can actually fire a round if it bore (torpedo tube loaded);
    /// always true for phasers/blasters, which have no per-emitter ammo.
    usable: bool,
    facing_deg: f32,
    arc_deg: f32,
    range: f32,
    /// Shared geometry against the locked target for this emitter's arc/range.
    geometry: WeaponTargetGeometry,
}

/// Pure, testable core of the arc-bearing family condition (issues #764, #767).
///
/// A family qualifies for an arc-bearing request ONLY when it is capable
/// (≥1 emitter), at least one usable ONLINE emitter is blocked specifically by
/// `OutOfArc` while in range, and NO emitter is `Ready`. Returns the union of
/// the family's usable ONLINE emitter arcs to carry in the request, or `None`
/// when the family does not qualify — incapable (no emitters), unavailable
/// (all offline), or out of range (no bearing helps). Driven off the shared
/// [`WeaponReadiness`] classification so emitter and Helm agree on the
/// condition; cooldown/loading are deliberately not considered here (a bearing
/// request is about *facing*, matching the pre-#767 phaser behaviour).
fn evaluate_family_arc_request(emitters: &[EmitterArcInput]) -> Option<Vec<WeaponEmitterArc>> {
    if emitters.is_empty() {
        return None; // incapable — nothing to bring to bear
    }
    let mut arcs = Vec::new();
    let mut has_out_of_arc_in_range = false;
    let mut has_ready = false;
    for e in emitters {
        if e.online && e.usable {
            arcs.push(WeaponEmitterArc {
                facing_deg: e.facing_deg,
                arc_deg: e.arc_deg,
                range: e.range,
            });
        }
        // Cooldown/loading are intentionally `false`: arc-bearing is a facing
        // request, not a timing one. `no_ammo` stands in for "unusable" (an
        // empty torpedo tube) so it classifies as `NoAmmo`, never `OutOfArc`.
        let readiness = crate::core::messages::WeaponReadiness::evaluate(
            e.online,
            false,
            false,
            !e.usable,
            Some(e.geometry),
        );
        if readiness.blocking_reason == crate::core::messages::WeaponBlockReason::OutOfArc {
            has_out_of_arc_in_range = true;
        }
        if readiness.ready {
            has_ready = true;
        }
    }
    (has_out_of_arc_in_range && !has_ready).then_some(arcs)
}

/// One emitter's contribution to a ship's direct-fire reach (issue #788).
///
/// Deliberately narrower than [`EmitterArcInput`]: reach is a question about
/// *range*, so no arc geometry and no target are involved. Kept as its own type
/// (rather than reusing the arc-request struct) because building the arc struct
/// requires a live target to compute geometry against, and the reach of a ship's
/// guns exists whether or not it currently has anyone in its sights.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DirectFireEmitter {
    /// Emitter is not offline (disabled / destroyed).
    pub online: bool,
    /// Emitter can actually fire a round. Always true for phasers and blasters,
    /// which carry no per-emitter ammunition.
    pub usable: bool,
    /// Effective reach of this emitter, world units.
    pub range: f32,
}

/// The longest range at which a ship can put **direct fire** on a target
/// (issue #788): the maximum effective range across its usable, ONLINE phaser
/// and blaster emitters.
///
/// Pure and testable; the Bevy caller ([`crate::ai::server`]'s world-snapshot
/// build) supplies the emitter list.
///
/// ## Why torpedoes are excluded
///
/// A torpedo homes. Its reach is a lifespan × speed budget for a round that
/// chases you, so standing off at "torpedo reach + a margin" buys nothing — the
/// round simply flies further. Direct fire is the family whose threat genuinely
/// stops at a radius, which is what makes a *safe ring* a meaningful place to
/// sit. Excluding them is a deliberate modelling choice, not an oversight.
///
/// ## Why `online && usable`
///
/// The same gate [`evaluate_family_arc_request`] applies, for the same reason:
/// a bank that is offline (disabled or shot out) is not a threat, so its range
/// must not inflate the ring an opponent keeps. A ship whose banks are all
/// offline has a reach of `0.0`, which correctly collapses the safe ring to the
/// authored margin alone.
pub fn longest_usable_direct_fire_range(emitters: &[DirectFireEmitter]) -> f32 {
    emitters
        .iter()
        .filter(|e| e.online && e.usable)
        .map(|e| e.range)
        .fold(0.0_f32, f32::max)
        .max(0.0)
}

/// Emit a channel-3 `ArcBearingRequest` coordination message to Helm whenever
/// the selected usable weapon family has the current target in range but
/// outside every one of that family's available arcs (issues #677, #767).
///
/// Generalises the pre-#767 phaser-only request to be weapon-family-aware:
/// each capable family (phasers, blasters, torpedoes) is evaluated via
/// [`evaluate_family_arc_request`]; a single family's request is emitted so
/// exactly one is ever active.
///
/// ## The family order is AUTHORED (issue #956)
///
/// It used to be a Rust array — `[Phasers, Blasters, Torpedoes]` — with a doc
/// comment here calling it "structural, not a gameplay value". That was wrong:
/// which gun a ship manoeuvres to present is a tactical decision, and a fleet
/// that always turns for its beams can never fight a doctrine built on fixed bow
/// tubes. The order now comes from the ship's own
/// [`WeaponsDoctrineAiPolicy`] via [`resolve_arc_bearing_order`], resolved fresh
/// every tick against [`seed_weapons_doctrine_facts`] — so a hull can lead with
/// its tubes while the target's striking arc is down and with its beams
/// otherwise. A hull with no preference of its own resolves the FLEET BASELINE
/// in `fragments/ai/fleet_baseline.toml`, which authors exactly the old order;
/// a hull with no policy at all (a bare test fixture — strict AI-declaration
/// mode rejects a shipped one) asks for nothing, which is the fail-closed
/// direction.
///
/// Exactly one request stays active because the walk stops at the first family
/// that qualifies, as it always did; what changed is who wrote the walk order.
///
/// Iterates every ship (player + NPC), mirroring `tick_sensors_frequency_hint`.
/// Debounced via [`WeaponsArcRequestState`]: re-fires when the family, target,
/// OR the usable arc set changes, not on every tick the same miss persists.
/// The two extra read-only lookups `tick_weapons_arc_request` needs on top of
/// the shared [`DirectFireGeometry`] pair, bundled as one `SystemParam` (issue
/// #1185): the entity-name table (for resolving an authored objective/hail
/// target name) and the locked target's own shields + heading (the one
/// cross-family reading the arc-bearing doctrine is authored against).
///
/// A signature grouping only — both queries keep their exact shapes, so the
/// access set is byte-for-byte unchanged, and the system destructures it back to
/// its `entity_name_q` / `target_shield_q` locals at entry.
#[derive(bevy::ecs::system::SystemParam)]
struct ArcRequestTargetLookups<'w, 's> {
    entity_names: Query<
        'w,
        's,
        (
            &'static crate::entities::spawner::EntityUuid,
            &'static crate::entities::spawner::EntityName,
        ),
    >,
    target_shields: Query<
        'w,
        's,
        (
            &'static crate::entities::spawner::EntityUuid,
            &'static crate::ship::shields::ShipShields,
            Option<&'static ShipPhysics>,
            &'static Transform,
        ),
    >,
}

#[allow(clippy::too_many_arguments)]
fn tick_weapons_arc_request(
    // The read-only AI-host world context — flag chain, sessions, and origin
    // stamps — behind one bare-`Res` system param (issue #1207). A fixture that
    // runs this host must register it (`register_ai_host_env`) or fail loudly at
    // schedule build, so a bare `App` cannot silently diverge from production.
    ai_env: crate::ai::host::AiHostEnv,
    mut ship_q: Query<
        (
            Entity,
            &ShipSystemControlSources,
            &ShipPhysics,
            &crate::ship_plugin::ShipConfigComponent,
            &crate::server_app::ShipSystemBlackboards,
            Option<&PhaserCombatConfigResource>,
            Option<&blaster::BlasterSystemResource>,
            Option<&torpedo::TorpedoSystemResource>,
            Option<&WeaponsDoctrineAiPolicy>,
            Option<&crate::ship::state::ShipRedAlert>,
            // The restraint lever (issue #1041). `Option` for the same reason
            // as the alert beside it: a bare-`App` fixture spawns neither.
            Option<&crate::ship::state::ShipWeaponsHold>,
            &mut WeaponsArcRequestState,
        ),
        With<crate::server_app::Ship>,
    >,
    // The two live-position lookups bundled as one `SystemParam` (issue #1185).
    geometry: DirectFireGeometry,
    // The entity-name table and the locked target's shields/heading, bundled as
    // one `SystemParam` (issue #1185). `target_shields` stays separate from
    // `geometry.entities` because `live_entity_xz` is shared with the other
    // Weapons systems and has no business growing a shield lookup (issue #956).
    target_lookups: ArcRequestTargetLookups,
    mut writer: MessageWriter<CoordinationEnqueue>,
) {
    use crate::entities::config::PhaserCombatConfig;

    // Restore the pre-#1185 locals so the body below is byte-for-byte unchanged.
    let DirectFireGeometry {
        asteroids: asteroid_q,
        entities: entity_q,
    } = geometry;
    let ArcRequestTargetLookups {
        entity_names: entity_name_q,
        target_shields: target_shield_q,
    } = target_lookups;

    for (
        ship_entity,
        control_sources,
        physics,
        ship_config,
        blackboards,
        combat_config_opt,
        blaster_opt,
        torpedo_opt,
        doctrine_opt,
        red_alert_opt,
        weapons_hold_opt,
        mut state,
    ) in ship_q.iter_mut()
    {
        let Some(helm_address) = crate::ship::coordination::address_for_system(
            &ship_config.0,
            &crate::ship::system_registry::helm_steering_system_id(),
        ) else {
            continue;
        };
        // Frozen Combat Lock from this ship's viewscreen (issue #829, spec §3).
        let combat_lock = match blackboards
            .0
            .get(&crate::ship::system_registry::viewscreen_system_id())
        {
            Some(crate::core::messages::SystemBlackboard::Viewscreen(bb)) => bb.combat_lock.clone(),
            _ => None,
        };
        let Some(target_uuid) = combat_lock else {
            state.last = None;
            continue;
        };
        let Some((tx, tz)) = live_entity_xz(&target_uuid, &asteroid_q, &entity_q) else {
            state.last = None;
            continue;
        };

        let is_offline = |sid: Option<crate::core::messages::SystemId>| -> bool {
            match sid {
                Some(id) => control_sources.0.is_offline(&id),
                None => false,
            }
        };
        let geometry = |range: f32, facing_deg: f32, arc_deg: f32| {
            crate::weapons::phaser::target_geometry(
                tx,
                tz,
                physics.x,
                physics.z,
                physics.yaw,
                range,
                facing_deg,
                arc_deg,
            )
        };

        // ── Phaser banks: facing + AI auto-fire arc, per-bank beam range. ────
        // The authored `beam_range`, unscaled (issue #955) — the same number
        // the blaster and torpedo emitters below already use for their own
        // families, and the same one `ai_phaser_auto_fire` fires on. An arc
        // request sized against anything else would ask Helm to turn for a
        // shot the guns cannot take, or decline one they can.
        let phaser_emitters: Vec<EmitterArcInput> = combat_config_opt
            .map(|cfg| {
                cfg.0
                    .banks
                    .iter()
                    .map(|b| {
                        let range = if b.beam_range > 0.0 {
                            b.beam_range
                        } else {
                            PhaserCombatConfig::DEFAULT_PHASER_RANGE
                        };
                        EmitterArcInput {
                            online: !is_offline(
                                crate::ship::system_registry::phaser_bank_system_id(&b.id),
                            ),
                            usable: true,
                            facing_deg: b.facing_deg,
                            arc_deg: b.auto_arc_deg,
                            range,
                            geometry: geometry(range, b.facing_deg, b.auto_arc_deg),
                        }
                    })
                    .collect()
            })
            .unwrap_or_default();

        // ── Blaster banks: facing + fire arc, per-bank projectile range. ─────
        let blaster_emitters: Vec<EmitterArcInput> = blaster_opt
            .map(|res| {
                res.0
                    .iter()
                    .map(|bs| {
                        let c = &bs.config;
                        EmitterArcInput {
                            online: !is_offline(
                                crate::ship::system_registry::blaster_bank_system_id(&c.id),
                            ),
                            usable: true,
                            facing_deg: c.facing_deg,
                            arc_deg: c.fire_arc_deg,
                            range: c.range,
                            geometry: geometry(c.range, c.facing_deg, c.fire_arc_deg),
                        }
                    })
                    .collect()
            })
            .unwrap_or_default();

        // ── Torpedo tubes: facing + fire arc, homing reach (speed × lifespan);
        // only a loaded tube is `usable`. ────────────────────────────────────
        let torpedo_emitters: Vec<EmitterArcInput> = torpedo_opt
            .map(|res| {
                let reach = res.0.config.speed * res.0.config.lifespan;
                res.0
                    .tubes
                    .iter()
                    .map(|t| EmitterArcInput {
                        online: !is_offline(crate::ship::system_registry::torpedo_tube_system_id(
                            &t.id,
                        )),
                        usable: t.loaded_count > 0,
                        facing_deg: t.facing_deg,
                        arc_deg: t.fire_arc_deg,
                        range: reach,
                        geometry: geometry(reach, t.facing_deg, t.fire_arc_deg),
                    })
                    .collect()
            })
            .unwrap_or_default();

        // ── The AUTHORED family order (issue #956) ───────────────────────────
        //
        // The ship's own doctrine decides which gun it turns to present, and it
        // decides it against the target's live striking-arc shields — the one
        // reading that makes "lead with the tubes once the screen is down" a
        // doctrine rather than a fixed ordering. A hull carrying no policy asks
        // for nothing; strict AI-declaration mode makes that unreachable for a
        // shipped hull, so in practice it is the bare-fixture case.
        //
        // The fact is seeded whether or not any guard reads it, exactly as the
        // fire hosts seed `red_alert`: a doctrine that ignores the screen is a
        // doctrine, not a reason to withhold the reading.
        let target_facing_shields: i32 = target_shield_q
            .iter()
            .find(|(u, ..)| u.0 == target_uuid)
            .map(|(_, shields, tphys, _)| {
                shields.0.hp_facing_attacker(
                    physics.x,
                    physics.z,
                    tx,
                    tz,
                    tphys.map(|p| p.yaw).unwrap_or(0.0),
                )
            })
            .unwrap_or(0);
        let doctrine_facts = seed_weapons_doctrine_facts(
            target_facing_shields,
            WeaponsAlertPosture::from_components(red_alert_opt, weapons_hold_opt),
        );
        let flag_chain = ai_env.flag_chain(ship_entity);
        let order = doctrine_opt
            .map(|d| resolve_arc_bearing_order(&d.0, &doctrine_facts, &flag_chain))
            .unwrap_or_default();

        // Emit for the first family in the authored order that qualifies, so
        // exactly one request is ever active.
        let emitters_of = |family: WeaponFamily| -> &Vec<EmitterArcInput> {
            match family {
                WeaponFamily::Phasers => &phaser_emitters,
                WeaponFamily::Blasters => &blaster_emitters,
                WeaponFamily::Torpedoes => &torpedo_emitters,
            }
        };
        let selected = order.iter().find_map(|fam| {
            evaluate_family_arc_request(emitters_of(*fam)).map(|arcs| (*fam, arcs))
        });

        let Some((family, arcs)) = selected else {
            // Issue #932: no family qualifies any more. `state.last` is
            // ALWAYS cleared here (pre-#932 behaviour, unconditionally) so a
            // family that requalifies later — even the same family, same
            // target, same arcs — is a genuinely new occurrence rather than
            // a debounce hit. But whether Helm is actively told to withdraw a
            // STANDING request is narrower than "no family qualifies": most
            // of the ways `selected` goes `None` are already Helm's own job
            // to notice — the family is already satisfied (some emitter is
            // `Ready`) or the target left the range of every carried arc —
            // and `apply_arc_bearing_request`'s geometry clear (`helm_ai.rs`)
            // already handles both against the SNAPSHOT of arcs the request
            // carried. Re-deriving usability from THAT snapshot and firing a
            // withdrawal on every such transition would fight that existing
            // seam instead of extending it — and did, in practice: an early
            // version of this fix withdrew on every transient satisfied/
            // out-of-range tick, not only genuine incapacitation, and it
            // measurably changed shipped combat behaviour (the death-gated
            // wave chain then in `combat_test.toml` stopped linking, which
            // that world's real-run guard — now
            // `combat_test_spawns_its_waves_on_the_clock_in_a_real_run`,
            // renamed with the schedule in #960 — caught).
            //
            // So: withdraw ONLY when the family `state.last` names has
            // become genuinely UNUSABLE — no ONLINE, USABLE emitter left at
            // all (every tube drained, every bank knocked offline) — which
            // is the one condition `apply_arc_bearing_request` cannot notice
            // on its own, because it only ever re-reads the arc snapshot
            // taken when the request was raised, never the family's current
            // usability.
            if let Some((withdrawn_family, ..)) = state.last.take() {
                let still_usable = match withdrawn_family {
                    WeaponFamily::Phasers => phaser_emitters.iter().any(|e| e.online && e.usable),
                    WeaponFamily::Blasters => blaster_emitters.iter().any(|e| e.online && e.usable),
                    WeaponFamily::Torpedoes => {
                        torpedo_emitters.iter().any(|e| e.online && e.usable)
                    }
                };
                if !still_usable {
                    let sender_origin = control_sources.0.source_for(&arc_request_sender_system(
                        withdrawn_family,
                        blaster_opt,
                        torpedo_opt,
                    ));
                    let presentation = crate::core::messages::CoordinationPresentation::titled(
                        "coordination.arc_withdraw.title",
                    )
                    .with_title_param("weapon", coordination_weapon_family_param(withdrawn_family));
                    writer.write(CoordinationEnqueue {
                        source_entity: ship_entity,
                        sender_origin,
                        address: helm_address.clone(),
                        payload: CoordinationPayload::ArcBearingWithdraw {
                            family: withdrawn_family,
                        },
                        presentation,
                        sender_label: crate::ship::coordination::CHATTER_SENDER_WEAPONS.to_string(),
                        sender_system: arc_request_sender_system(
                            withdrawn_family,
                            blaster_opt,
                            torpedo_opt,
                        ),
                    });
                }
            }
            continue;
        };

        // Debounce on family + target + the usable arc set: a change to any of
        // these is a new condition and must re-fire (issue #767 AC4).
        let key = (family, target_uuid.clone(), arcs.clone());
        if state.last.as_ref() == Some(&key) {
            continue;
        }
        state.last = Some(key);

        let label = entity_name_q
            .iter()
            .find_map(|(u, n)| (u.0 == target_uuid).then(|| n.0.clone()))
            .unwrap_or_else(|| target_uuid.clone());

        let sender_origin = control_sources.0.source_for(&arc_request_sender_system(
            family,
            blaster_opt,
            torpedo_opt,
        ));

        let presentation = crate::core::messages::CoordinationPresentation::new(
            "coordination.arc_bearing.title",
            label.clone(),
        )
        .with_title_param("weapon", coordination_weapon_family_param(family));

        writer.write(CoordinationEnqueue {
            source_entity: ship_entity,
            sender_origin,
            address: helm_address,
            payload: CoordinationPayload::ArcBearingRequest {
                uuid: target_uuid,
                label,
                family,
                arcs,
            },
            presentation,
            sender_label: crate::ship::coordination::CHATTER_SENDER_WEAPONS.to_string(),
            sender_system: arc_request_sender_system(family, blaster_opt, torpedo_opt),
        });
    }
}

fn coordination_weapon_family_param(
    family: WeaponFamily,
) -> crate::core::messages::CoordinationParam {
    match family {
        WeaponFamily::Phasers => {
            crate::core::messages::CoordinationParam::text("coordination.weapon_family.phasers")
        }
        WeaponFamily::Blasters => {
            crate::core::messages::CoordinationParam::text("coordination.weapon_family.blasters")
        }
        WeaponFamily::Torpedoes => {
            crate::core::messages::CoordinationParam::text("coordination.weapon_family.torpedoes")
        }
    }
}

/// The representative sender system for control-source resolution when
/// Weapons addresses Helm over channel-3: the emitting family's canonical
/// fine system (a blaster/torpedo-only ship has no phaser-fore), falling back
/// to phaser-fore. Shared by the request and its issue #932 withdrawal so
/// both name the same sender for the same family.
fn arc_request_sender_system(
    family: WeaponFamily,
    blaster_opt: Option<&blaster::BlasterSystemResource>,
    torpedo_opt: Option<&torpedo::TorpedoSystemResource>,
) -> crate::core::messages::SystemId {
    match family {
        WeaponFamily::Phasers => crate::ship::system_registry::phaser_fore_system_id(),
        WeaponFamily::Blasters => blaster_opt
            .and_then(|res| res.0.first())
            .and_then(|bs| crate::ship::system_registry::blaster_bank_system_id(&bs.config.id))
            .unwrap_or_else(crate::ship::system_registry::phaser_fore_system_id),
        WeaponFamily::Torpedoes => torpedo_opt
            .and_then(|res| res.0.tubes.first())
            .and_then(|t| crate::ship::system_registry::torpedo_tube_system_id(&t.id))
            .unwrap_or_else(crate::ship::system_registry::phaser_fore_system_id),
    }
}

// ── Tactical AI ───────────────────────────────────────────────────────────
//
// `ai_target_selection` is the whole of the Tactical AI's targeting path
// (issues #697, #700). It reads the world, the ship's own objective
// blackboard, and its last attacker; it publishes the chosen target to
// `WeaponsBlackboard.locked_target` as observable intent, and applies that
// same choice to the authoritative `TacticalRadarSelection` component (truth) in the
// same system.
//
// It began (#697) as a decide/integrate pair — `ai_target_selection` →
// `operate_tactical_ai` — mirroring the decide/apply shape the other console
// AIs used at the time (e.g. the pre-#826 shields pair). #700 folded the integrator
// back in, because unlike those pairs the two halves could not be separated by
// a sim set: at the time every `WeaponsTarget` reader ran in `SimSet::Input`, so the
// write had to stay in `Input` too, which left the "pair" as two systems in the same
// set held together by an explicit `.before` edge and an `Option<Option<_>>`
// to distinguish "the decider never ran" from "the decider chose nothing".
// (Post-#829 the only `Input` readers of the selection component are its two
// writers — `handle_set_target` and `ai_target_selection`; cross-system consumers
// read the frozen viewscreen `combat_lock` — but the writer/writer `.before` edge
// still keeps a human lock atomic against the AI decider within the tick.)
//
// Folding them back makes read-seed-decide-write atomic with respect to the
// other `Input` writer of `TacticalRadarSelection` (`handle_set_target`), which is what
// the `.before` edge existed to enforce. See `WeaponsPlugin::build`.
//
// This system does not fire weapons. Issue #846 migrated the decide/integrate
// pair off private intent components: `ai_phaser_auto_fire` /
// `ai_torpedo_auto_fire` now emit admitted `ControlSystem` payloads via
// `emit_ai_command`; the weapons handlers consume via `AdmittedCommands`
// in `SimSet::Physics`.

/// Publish `ai_target_selection`'s decision on a ship's blackboards, creating
/// the Weapons entry if the ship has none yet.
///
/// This is observability, not a control channel: nothing reads `locked_target`
/// back to drive behaviour — `ai_target_selection` applies its own decision to
/// `TacticalRadarSelection` directly. The field is what lets a client (or a human
/// watching a backfilled console) see *why* the ship's lock is what it is, and
/// it is what distinguishes AI intent from a human's lock on the wire.
///
/// `publish_weapons_core_blackboard` rebuilds the entry from real ship state later
/// in the same tick, so a bare default entry never escapes to the wire.
fn record_locked_target_decision(
    blackboards: &mut crate::server_app::ShipSystemBlackboards,
    value: Option<String>,
) {
    let entry = blackboards
        .0
        .entry(crate::ship::system_registry::tactical_station_key())
        .or_insert_with(|| SystemBlackboard::Weapons(WeaponsBlackboard::default()));
    if let SystemBlackboard::Weapons(weapons) = entry {
        weapons.locked_target = value;
    }
}

/// Drop any stale Tactical AI intent from a ship the selector is skipping.
///
/// A no-op when the ship has no Weapons blackboard entry, rather than an
/// insert of an empty one: `publish_weapons_core_blackboard` owns creating the entry
/// with real ship state, and a ship the AI does not target for has no intent to
/// report in the first place.
fn clear_locked_target_if_present(blackboards: &mut crate::server_app::ShipSystemBlackboards) {
    if let Some(SystemBlackboard::Weapons(weapons)) = blackboards
        .0
        .get_mut(&crate::ship::system_registry::tactical_station_key())
    {
        weapons.locked_target = None;
    }
}

/// The three read-only scan surfaces `ai_target_selection` acquires targets
/// from, bundled as one `SystemParam` (issue #1185): the asteroid field, the
/// `Without<Asteroid>` entity surface (wide enough to resolve an authored name),
/// and the deliberately narrower tier-4 auto-acquisition surface.
///
/// A signature grouping only — every query keeps its exact shape and filter, so
/// the access set is byte-for-byte unchanged; the system destructures it back to
/// its `asteroid_q` / `other_ships_q` / `hostile_scan_q` locals at entry.
#[derive(bevy::ecs::system::SystemParam)]
struct TargetSelectionScans<'w, 's> {
    asteroids: Query<
        'w,
        's,
        (&'static AsteroidUuid, &'static Transform),
        With<crate::server_app::Asteroid>,
    >,
    other_ships: Query<
        'w,
        's,
        (
            &'static crate::entities::spawner::EntityUuid,
            &'static Transform,
            Option<&'static crate::entities::spawner::EntityName>,
        ),
        Without<crate::server_app::Asteroid>,
    >,
    /// The tier-4 scan surface, deliberately narrower than `other_ships`. That
    /// query is `Without<Asteroid>`, wide enough for resolving an authored name
    /// (a mission may name anything, and a miss is harmless), but as an
    /// *auto-acquisition* surface it would lock any factioned entity that
    /// happens to carry an `EntityUuid` + `Transform`. The filter used to be
    /// `(With<Ship>, Without<StaticPointDefence>)` — until the first factioned
    /// station, `assets/entities/station_axiom.toml` (issue #1011): `spawn_entity`
    /// inserts BOTH `StaticPointDefence` and `Ship` for a static-point-defence
    /// entity, so the station carries the `Ship` marker the old filter asked for
    /// — it was the `Without<StaticPointDefence>` half that excluded it. Dropping
    /// that `Without` is the load-bearing change; the `With<StaticPointDefence>`
    /// arm is a hedge for a future entity that carries `StaticPointDefence`
    /// without `Ship`. It does NOT make an unfactioned `StaticPointDefence`
    /// acquirable: the hostility verdicts require BOTH sides to carry a
    /// `FactionComponent`, so a factionless turret stays invisible to
    /// auto-acquisition — this filter only decides who is IN the scan.
    hostiles: Query<
        'w,
        's,
        (
            &'static crate::entities::spawner::EntityUuid,
            &'static Transform,
            Option<&'static FactionComponent>,
        ),
        Or<(
            With<crate::server_app::Ship>,
            With<crate::entities::spawner::StaticPointDefence>,
        )>,
    >,
}

/// The read-only resource context `ai_target_selection` reads besides the
/// [`crate::ai::host::AiHostEnv`] — the log filter, the reselection-event tick
/// stamp, the session table its `emit_ai_command` consults, and the faction
/// registry the hostility verdicts need — bundled as one `SystemParam` (issue
/// #1185).
///
/// A signature grouping only — every field keeps its type and `Option` fallback
/// (`sim_tick`/`log`/`faction_registry` stay `Option` for the bare-`App`
/// fixtures that never insert them), so the access set is byte-for-byte
/// unchanged; the system destructures it back to its original locals at entry.
#[derive(bevy::ecs::system::SystemParam)]
struct TargetSelectionEnv<'w> {
    log: Option<Res<'w, crate::logging::LogFilterConfig>>,
    sim_tick: Option<Res<'w, crate::sim_tick::SimTick>>,
    sessions: Res<'w, crate::lobby::Sessions>,
    faction_registry: Option<Res<'w, crate::entities::config_cache::FactionRegistryResource>>,
}

/// Tactical AI target prioritisation (issues #697, #700, #703).
///
/// Runs for every ship whose Tactical surface is AI-controlled — player ship
/// and NPC alike, with no `AiHighFidelity` gate.
///
/// Acquisition precedence, highest first:
///
/// 1. The explicit target of the highest-scoring Weapons-relevant `Destroy`
///    objective.
/// 2. The lock the ship already holds, while it is still resolvable and still
///    inside radar range.
/// 3. The ship's `LastShipAttacker` — whoever last hit it with a beam.
/// 4. The nearest hostile (issue #703), but *only* when that top `Destroy`
///    objective is untargeted (`Destroy { target: "" }`), i.e. standing
///    "engage anything hostile" doctrine.
///
/// If no tier yields a candidate the current lock is kept. Any candidate must
/// be inside the damage-scaled tactical radar range (issue #680), and a lock
/// that goes dead or drifts out of range is dropped.
///
/// Tier 4 exists because tiers 1 and 3 both come up empty for shipped content:
/// no asset TOML authors a `directive_target`, and `LastShipAttacker` is only
/// written once a phaser beam connects. Without it an NPC could not fire until
/// the player shot it first.
///
/// "Hostile" and "nearest" are decided by `ai::core::find_nearest_hostile`, fed
/// a `WorldView` built below and the live `FactionRegistry`.
///
/// ## This is the only selector (issue #702)
///
/// There is exactly one place a ship's target is chosen by AI, and this is it.
/// The Helm does not acquire: `ai::core::helm_destroy` reads `TacticalRadarSelection` and
/// closes on whatever it names, ignoring even the `Destroy` directive's own
/// `target` (tier 1 resolves that, here). So "helm and weapons pick the same
/// ship" is not an invariant two paths have to maintain in step — it is
/// structural. There is one decision and one surface.
///
/// That is the whole point of #702, and it is worth stating plainly because the
/// code it replaced was not obviously wrong: the Helm used to run its own
/// four-tier `resolve_destroy_target` with the identical tiers in the identical
/// order, and it still diverged, because each side applied its verdict inside
/// its own separately-authored radar horizon (187.5 helm vs 75 weapons on the
/// alliance hulls). Two selectors kept in step by documentation is the bug.
/// **Do not reintroduce a second one.** If acquisition should change, it changes
/// here, and every consumer follows because every consumer reads `TacticalRadarSelection`.
///
/// ## Why this order, and not a different one
///
/// The tiers no longer exist to mirror anybody, but the order still earns its
/// keep on this side alone.
///
/// Tier 2 — keep the engagement we are already in — is what stops tier 4 from
/// re-scanning every tick and handing the lock to whoever is nearest *right
/// now*. Without it, two converging hostiles flip the lock to the newcomer, and
/// near-equidistant pairs thrash it every tick, retargeting beams and restarting
/// `tick_npc_auto_match_frequency`'s `delay_secs`. Because the Helm pursues this
/// lock, that thrash is also the ship slewing between two bearings.
///
/// Tier 2 sitting *above* `LastShipAttacker` is the deliberate part, and it is a
/// change from #703's first cut. Retaliating instantly reads like the more
/// aggressive choice, but it means any bystander that grazes an engaged ship
/// drags both its guns and its nose off the target it committed to. The ship
/// still shoots back at whoever hit it the moment it is not already engaged (no
/// lock, or its lock died or slipped out of radar range). Stickiness while
/// engaged is the rule; if retaliation should preempt an existing engagement,
/// reorder the tiers here — that is now a one-place change.
///
/// ## Known behaviour worth knowing
///
/// **Unresolvable tier-1 target (intentional).** When tier 1 names a target that
/// cannot be resolved, selection falls through to tiers 2–3 (pre-existing #697
/// behaviour). Note it is 2–3, not 2–4: `top_destroy` is `Some(name)` with
/// `name` non-empty, so `destroy_is_untargeted` is `false` and the
/// nearest-hostile tier is gated off — a `Destroy` naming someone specific never
/// decays into "shoot whoever is closest".
///
/// **The Helm's radar horizon still gates pursuit.** This system locks against
/// `effective_tactical_range` (`[weapons_console.radar] range`), while
/// `helm_ai_world_view` (`ship_plugin.rs`) builds the Helm's view from
/// `[helm_console.radar] range`. These differ on the alliance hulls, so Tactical
/// can lock a ship the Helm cannot see. That no longer splits the decision — the
/// lock is the lock — but `helm_destroy` returns `None` for a target outside its
/// own view, and the Helm falls through to a lower-priority directive rather
/// than flying at a bearing it cannot confirm. It shoots at the locked ship
/// without closing on it, which is a coherent outcome, not a split brain.
///
/// ## The decision travels as an admitted command (issue #887)
///
/// This system does not write `TacticalRadarSelection`. It emits the chosen UUID
/// as a `SetTarget` on `tactical-radar` through `emit_ai_command`, and
/// `handle_set_target` — the ship's ONE applier, human or AI — resolves and
/// applies it later in the same `SimSet::Input`. A cleared selection is emitted
/// as `SetTarget { uuid: "" }`: an unresolvable UUID drops the lock, which is
/// already what the applier does for a human who names something that no longer
/// exists, so no origin-specific "clear" verb is needed.
///
/// The gate is the **radar's own** `operate_ai`, not "any tactical fine system
/// is AI" (`any_tactical_system_operates_ai`, the pre-#887 predicate). That
/// predicate was strictly wider than the one admission applies to the human,
/// which is what let an Ai torpedo tube licence the AI to overwrite a Human
/// radar's lock on a mixed-rating ship — `alliance_cruiser`'s shipped
/// `Simplified` Tactical rating (AI phaser banks, human radar and tubes) is
/// exactly that shape. Gating on `tactical-radar` makes the two origins mutually
/// exclusive (`accept_human_input` and `operate_ai` cannot both hold on one
/// `SystemId`), which is what makes a single applier safe.
///
/// `TacticalRadarSelection` remains the single source of truth every consumer
/// reads, one lock per ship. Every AI weapon host — `ai_phaser_auto_fire`,
/// `tick_blaster_auto_fire`, `ai_torpedo_auto_fire` — aims at the frozen
/// viewscreen `combat_lock` derived from it, so no weapon bank picks a target of
/// its own and an AI ship can no more engage two hostiles at once than a crewed
/// one can (AGENTS.md #6).
fn ai_target_selection(
    // The log filter, reselection-event tick stamp, session table, and faction
    // registry bundled as one `SystemParam` (issue #1185). See
    // [`TargetSelectionEnv`].
    env: TargetSelectionEnv,
    mut ship_query: Query<
        (
            Entity,
            &crate::ship_plugin::ShipConfigComponent,
            &ShipSystemControlSources,
            &LastShipAttacker,
            &ShipPhysics,
            &TacticalRadarSelection,
            &mut crate::core::messages::AdmittedCommands,
            &mut crate::server_app::ShipSystemBlackboards,
            Option<&crate::modifiers::ShipModifiers>,
            Option<&crate::entities::spawner::WeaponsConsoleSection>,
            // Self identity + faction, for the nearest-hostile tier (#703):
            // the UUID excludes self from the scan, the faction decides who
            // counts as hostile. Both `Option` because minimal test spawns
            // omit them; a ship with no faction acquires nothing this way.
            Option<&crate::entities::spawner::EntityUuid>,
            Option<&FactionComponent>,
            // Per-ship data-driven Tactical target selector (issue #777).
            // `Option` so bare-`App` fixtures without an attached component fall
            // back to the canonical default selector built once below.
            Option<&TacticalTargetSelector>,
            // A static point-defence platform (the station) has no `[behaviour]`
            // doctrine, so `top_destroy` is always `None` and the untargeted-
            // Destroy licence below would never open the nearest-hostile source.
            // A turret whose whole purpose is to shoot the nearest hostile its
            // radar can see needs that source without a doctrine, so it licenses
            // the source directly (mirrors the `hostile_scan_q` `Or<>` filter).
            Has<crate::entities::spawner::StaticPointDefence>,
        ),
        With<crate::server_app::Ship>,
    >,
    // The three read-only scan surfaces bundled as one `SystemParam` (issue
    // #1185). See [`TargetSelectionScans`].
    scans: TargetSelectionScans,
    // Kept out of the already-maximal ship tuple: these read-only operation
    // components provide objective-directed executable reach and live coupling
    // identity for the exact ship being selected.
    operate_systems: Query<(
        Option<&crate::tractor::TractorBeam>,
        Option<&crate::console::repair::ExternalRepairDispatch>,
    )>,
    // The read-only AI-host world context — flag chain, sessions, and origin
    // stamps — behind one bare-`Res` system param (issue #1207). A fixture that
    // runs this host must register it (`register_ai_host_env`) or fail loudly at
    // schedule build, so a bare `App` cannot silently diverge from production.
    ai_env: crate::ai::host::AiHostEnv,
) {
    // Restore the pre-#1185 locals so the body below is byte-for-byte unchanged.
    let TargetSelectionEnv {
        log,
        sim_tick,
        sessions,
        faction_registry,
    } = env;
    let TargetSelectionScans {
        asteroids: asteroid_q,
        other_ships: other_ships_q,
        hostiles: hostile_scan_q,
    } = scans;

    let registry_default = crate::ai::faction::FactionRegistry::default();
    let registry: &crate::ai::faction::FactionRegistry = faction_registry
        .as_deref()
        .map(|r| &r.0)
        .unwrap_or(&registry_default);

    // World-space position of a targetable UUID, asteroid or entity. Tactical
    // radar deliberately projects this onto (x, z), while an operation's
    // executable reach uses all three axes like the tractor/repair appliers do.
    let target_xyz = |uuid: &str| -> Option<(f32, f32, f32)> {
        asteroid_q
            .iter()
            .find_map(|(u, t)| {
                (u.0 == uuid).then_some((t.translation.x, t.translation.y, t.translation.z))
            })
            .or_else(|| {
                other_ships_q.iter().find_map(|(u, t, _)| {
                    (u.0 == uuid).then_some((t.translation.x, t.translation.y, t.translation.z))
                })
            })
    };
    let target_xz = |uuid: &str| -> Option<(f32, f32)> { target_xyz(uuid).map(|(x, _, z)| (x, z)) };

    // Resolve a targetable UUID to a display name for readable log lines,
    // falling back to the raw UUID when the entity carries no `EntityName`.
    let name_of = |uuid: &str| -> String {
        other_ships_q
            .iter()
            .find_map(|(u, _, n)| (u.0 == uuid).then(|| n.map(|n| n.0.clone())))
            .flatten()
            .unwrap_or_else(|| uuid.to_string())
    };

    for (
        ship_entity,
        ship_config,
        control_sources,
        last_attacker,
        physics,
        weapons_target,
        mut admitted,
        mut blackboards,
        modifiers,
        weapons_section,
        self_uuid,
        self_faction,
        target_selector,
        is_static_point_defence,
    ) in ship_query.iter_mut()
    {
        // A query of two optional components matches every live entity. Keep a
        // defensive empty fallback for minimal fixtures and despawn boundaries.
        let (tractor, external_repair) = operate_systems.get(ship_entity).unwrap_or((None, None));
        // Only select for ships whose TACTICAL RADAR is AI-controlled (issue
        // #887). The lock belongs to the radar, so the radar's own policy is
        // what licenses an AI selection — the same `SystemId` admission checks
        // for a human's `SetTarget`, and `accept_human_input` / `operate_ai` are
        // mutually exclusive on one id, so exactly one origin can ever hold it.
        //
        // This replaced `any_tactical_system_operates_ai` ("ANY phaser bank,
        // torpedo tube or the magazine is Ai"), which was strictly wider than
        // the human's gate: an Ai torpedo tube alongside a Human radar let the
        // AI overwrite the human's lock. `alliance_cruiser`'s shipped
        // `Simplified` Tactical rating is exactly that ship.
        //
        // A human-operated radar therefore selects nothing here; the operator
        // drives the lock through the same `handle_set_target` applier. Clearing
        // the intent stops a ship that flips from AI to human control leaving a
        // stale selection on its blackboard.
        // Explicit AI-or-idle declaration for the Tactical radar (issue #781,
        // AC6). An authored `selector_idle = true` makes the radar take no AI
        // selection even when a tactical fine system is AI-operated — the
        // explicit opt-out that distinguishes "radar deliberately idle" from
        // "radar ranks its candidates".
        //
        // An ABSENT component is the third case, and since #885b stage 5d it is
        // also a stand-down rather than a synthesised default: no authored
        // `[weapons_console.selector]` means no ranking exists to run, so the
        // radar clears any stale lock and takes no selection.
        let radar_idle = target_selector.is_none_or(|s| s.idle);
        // Control-Source gate through the shared AI host spine (issue #1208): the
        // radar's OWN `operate_ai`. Tactical target selection resolves a
        // data-driven SELECTOR the spine does not model, so only its gate — the
        // one step it shares with the policy hosts — routes here.
        let radar_operates_ai = crate::ai::host::ai_operates(
            &control_sources.0,
            crate::ship::system_registry::tactical_radar_system_id(),
        );
        if radar_idle || !radar_operates_ai {
            clear_locked_target_if_present(&mut blackboards);
            continue;
        }
        let Some(selector_comp) = target_selector else {
            continue;
        };

        // Damage-scaled tactical radar range (issue #680). Scale the base
        // per-ship config range by the shared RadarRange modifier multiplier.
        let radar_range_mult = modifiers
            .map(|m| m.get(&ModifierSlot::RadarRange))
            .unwrap_or(1.0);
        let base_range = weapons_section
            .and_then(|s| s.0.radar.as_ref().map(|r| r.range))
            .unwrap_or(0.0);
        let effective_tactical_range = base_range * radar_range_mult;
        // A non-positive or non-finite range means "unbounded" — the ship
        // declares no radar, so range never culls a candidate.
        let range_bounds_targets =
            effective_tactical_range > 0.0 && effective_tactical_range.is_finite();
        let within_range = |uuid: &str| -> bool {
            match target_xz(uuid) {
                Some((tx, tz)) => {
                    let dx = tx - physics.x;
                    let dz = tz - physics.z;
                    dx * dx + dz * dz <= effective_tactical_range * effective_tactical_range
                }
                None => false,
            }
        };

        // The ship's current lock, cloned out of the `Mut` once so the candidate
        // build below can read it (as the retention candidate) and pass it to
        // the selector as `current`, without holding a borrow across the
        // write-back at the end.
        let current_lock: Option<String> = weapons_target.0.clone();

        // ── Data-driven Tactical target ranking (#777) ──────────────────────
        // The retired four-tier `if/else if` chain (objective ≫ retained ≫
        // last-attacker ≫ nearest-hostile) is now four registered candidate
        // sources fed to the ship's authored `TargetSelector`. The selector
        // unions + dedups them, applies the authored eligibility (independent
        // hostility revalidation, AC3), sums the additive per-source utility
        // (the Sensors-favour bonus is one such term, AC2), and retains the
        // current lock through the authored switch margin (AC5). Tactical stays
        // the SOLE writer of `TacticalRadarSelection`: the selector only RANKS;
        // the host applies the chosen UUID directly below (AC4).
        //
        // The host keeps owning the live, damage-scaled horizon: ordinary
        // candidates are pre-filtered to `within_range` here (AC5), so the
        // selector's own authored horizon is a static outer bound only. The
        // one explicit exception is an objective-operate candidate within its
        // installed system's authored executable reach, resolved below.
        let top_destroy = top_destroy_objective_target(Some(&*blackboards));
        // A Patrol objective that names a visible entity is a defend marker:
        // Backfill still flies its route, but when hostiles arrive it chooses
        // the attacker closest to the protected hull rather than the one
        // closest to itself.  Ordinary patrols with only anchors remain the
        // historical nearest-to-self behaviour.
        let defend_target = top_defend_objective_target(Some(&*blackboards)).and_then(|name| {
            resolve_objective_target_uuid(name, Some(ai_env.content_runtime()), &other_ships_q)
        });
        // An *untargeted* Destroy directive — `Destroy { target: "" }` — is
        // standing "engage any hostile you detect" doctrine (every shipped
        // hostile TOML). It is the only case that licenses the nearest-hostile
        // source: a Destroy naming someone specific must not decay into
        // shoot-whoever-is-closest.
        let destroy_is_untargeted = matches!(top_destroy, Some(""));
        let objective_target: Option<String> = match top_destroy {
            Some("") => None,
            Some(target_name) => resolve_objective_target_uuid(
                target_name,
                Some(ai_env.content_runtime()),
                &other_ships_q,
            ),
            None => None,
        };
        // The operate directive's named target (issue #1162), resolved to a live
        // UUID the same way a Destroy target is. `None` unless an active
        // Tow/Stabilise/Escort/FieldRepair names something, keeping the
        // `source_operate` candidate inert in ordinary combat.
        let operate_order = top_operate_objective_target(Some(&*blackboards));
        let operate_target: Option<String> = operate_order.and_then(|(name, _)| {
            resolve_objective_target_uuid(name, Some(ai_env.content_runtime()), &other_ships_q)
        });
        // An explicit operate objective names work for a concrete installed
        // system. That system's authored executable reach is an authoritative
        // acquisition horizon for this ONE candidate: Tow/Stabilise/Escort use
        // the tractor reach, FieldRepair uses the external dispatch reach.
        // Ordinary combat/scan candidates remain Tactical-radar bounded.
        let operate_range = operate_order.and_then(|(_, system)| match system {
            OperateLockSystem::Tractor => tractor.map(|beam| beam.config.range),
            OperateLockSystem::ExternalRepair => {
                external_repair.map(|dispatch| dispatch.config.range)
            }
        });

        // The advisory Sensors designation (AC2), read from the FROZEN
        // viewscreen `science_target` (#829) — never the channel-3
        // `TargetDesignation` chatter, which is viewscreen-only and unreadable
        // here. Cloned to own it before the mutable blackboard write below.
        let science_target: Option<String> = match blackboards
            .0
            .get(&crate::ship::system_registry::viewscreen_system_id())
        {
            Some(SystemBlackboard::Viewscreen(vbb)) => vbb.science_target.clone(),
            _ => None,
        };

        // Independent hostility verdict (AC3): re-run `faction::is_enemy` over
        // the live registry per candidate UUID — the Sensors pick's hostility is
        // never trusted. Faction is read from the `With<Ship>` scan surface;
        // factionless / non-ship candidates are neutral (never auto-hostile).
        let self_faction_uuid = self_faction.map(|f| f.0);
        let is_hostile = |uuid: &str| -> bool {
            let target_faction = hostile_scan_q.iter().find_map(|(u, _, faction)| {
                (u.0 == uuid).then_some(faction.map(|f| f.0)).flatten()
            });
            crate::ai::faction::is_enemy(self_faction_uuid, target_faction, registry)
        };

        // Build a candidate for `uuid` from `source_fact`, pre-filtered to the
        // live radar horizon and stamped with the independent hostility verdict.
        // `detectable` is implied by passing the host's own range gate.
        let make_candidate =
            |uuid: &str, source_fact: &str| -> Option<crate::ai::selector::SelectorCandidate> {
                let (tx, ty, tz) = target_xyz(uuid)?;
                // Acquisition remains bounded by Tactical radar. Once the
                // tractor is actually holding the active operate target,
                // however, the coupling itself is authoritative contact: its
                // authored offset may place the load outside that radar on the
                // very next tick. Preserve only that exact lock while the exact
                // operate directive remains active; ending either the coupling
                // or directive restores the ordinary range gate immediately.
                let is_operate_target =
                    source_fact == crate::entities::ai_flag_hosts::SOURCE_OPERATE.name();
                let operation_distance =
                    Vec3::new(physics.x, physics.y, physics.z).distance(Vec3::new(tx, ty, tz));
                let is_within_operate_reach = is_operate_target
                    && operate_range.is_some_and(|range| operation_distance <= range);
                let is_coupled_operate_target = is_operate_target
                    && matches!(operate_order, Some((_, OperateLockSystem::Tractor)))
                    && tractor
                        .and_then(|beam| beam.coupled_target.as_deref())
                        .is_some_and(|coupled| coupled == uuid);
                if range_bounds_targets
                    && !within_range(uuid)
                    && !is_within_operate_reach
                    && !is_coupled_operate_target
                {
                    return None;
                }
                use crate::entities::ai_flag_hosts as fid;
                let mut facts = crate::world::flags::AiFacts::new();
                facts.set_fact(fid::DETECTABLE, 1.0);
                facts.set_fact(fid::HOSTILE, if is_hostile(uuid) { 1.0 } else { 0.0 });
                // `source_fact` is a registry constant's `.name()` handed in by
                // the caller (issue #1210) — one of the `SOURCE_*` catalogue ids.
                facts.set(source_fact, 1.0);
                Some(crate::ai::selector::SelectorCandidate {
                    uuid: uuid.to_string(),
                    position: [tx, 0.0, tz],
                    facts,
                })
            };

        // Nearest faction-hostile (issue #703), delegating the faction verdict
        // and distance ordering to `ai::core::find_nearest_hostile` over a
        // `WorldView` built here rather than open-coding "hostile"/"nearest".
        let nearest_hostile =
            |origin: [f32; 3], registry: &crate::ai::faction::FactionRegistry| -> Option<String> {
                let self_faction_uuid = self_faction.map(|f| f.0)?;
                let self_uuid_str = self_uuid.map(|u| u.0.as_str()).unwrap_or("");
                let entities: Vec<crate::ai::AiWorldEntity> = hostile_scan_q
                    .iter()
                    .filter(|(u, _, _)| u.0 != self_uuid_str)
                    .filter_map(|(u, t, faction)| {
                        // Only canonically-UUID'd entities can take part: an
                        // unparseable id would collapse to the nil UUID and let
                        // two entities alias each other in the scan.
                        let parsed = uuid::Uuid::parse_str(&u.0).ok()?;
                        Some(crate::ai::AiWorldEntity {
                            uuid: parsed,
                            position: [t.translation.x, t.translation.y, t.translation.z],
                            faction: faction.map(|f| f.0),
                            ..Default::default()
                        })
                    })
                    .collect();
                let world_view = crate::ai::WorldView {
                    entity_pos: origin,
                    entity_yaw: physics.yaw,
                    entities,
                    self_faction: Some(self_faction_uuid),
                    ..crate::ai::WorldView::default()
                };
                let found = crate::ai::find_nearest_hostile(&world_view, registry)?;
                hostile_scan_q.iter().find_map(|(u, _, _)| {
                    (uuid::Uuid::parse_str(&u.0).ok() == Some(found)).then(|| u.0.clone())
                })
            };

        use crate::ai::selector::SelectorCandidate;
        let mut candidates: Vec<SelectorCandidate> = Vec::new();

        // Source: sensors-designation — the advisory Sensors pick (AC2). Copied
        // only if it survives independent revalidation (AC3): the candidate
        // carries the recomputed `hostile` fact, and the authored eligibility
        // drops a friendly / out-of-range designation.
        if let Some(sci) = science_target.as_deref() {
            if let Some(c) = make_candidate(
                sci,
                crate::entities::ai_flag_hosts::SOURCE_SENSORS_DESIGNATION.name(),
            ) {
                candidates.push(c);
            }
        }
        // Source: objective-destroy — the explicit named Destroy target.
        if defend_target.is_none() {
            if let Some(obj) = objective_target.as_deref() {
                if let Some(c) =
                    make_candidate(obj, crate::entities::ai_flag_hosts::SOURCE_OBJECTIVE.name())
                {
                    candidates.push(c);
                }
            }
        }
        // Source: objective-operate (issue #1162) — the target an active
        // per-verb operate directive named. Injected ONLY here, so the tractor
        // and external-repair dispatch can reach a NON-hostile lock the ship was
        // ordered to work. Directive-gated: `operate_target` is `None` with no
        // such directive, so no candidate carries `source_operate` and the
        // widened eligibility disjunct stays inert — combat auto-lock is
        // unchanged and radar/scan auto-lock stays hostile-gated. The candidate
        // carries its independent `hostile` verdict like any other, but the
        // authored `source_operate` eligibility term admits it regardless.
        if let Some(op) = operate_target.as_deref() {
            if let Some(c) =
                make_candidate(op, crate::entities::ai_flag_hosts::SOURCE_OPERATE.name())
            {
                candidates.push(c);
            }
        }
        // Source: last-attacker — whoever last hit us.
        if let Some(att) = last_attacker.0.as_deref() {
            if let Some(c) = make_candidate(
                att,
                crate::entities::ai_flag_hosts::SOURCE_LAST_ATTACKER.name(),
            ) {
                candidates.push(c);
            }
        }
        // Retention candidate — the ship's own current lock (the old tier-2),
        // surfaced internally (NOT a cross-system source) so the selector can
        // retain it. Combat-appropriateness gates it exactly as before: under
        // untargeted combat doctrine, retain only an opposing ship; otherwise
        // retain the standing lock regardless of faction (a human or
        // objective-driven assault lock on scenery). The eligibility guard
        // admits `source_retained` without a hostility check for that reason.
        if defend_target.is_none() {
            if let Some(cur) = current_lock.as_deref() {
                let combat_appropriate = !destroy_is_untargeted || is_hostile(cur);
                if combat_appropriate {
                    if let Some(c) =
                        make_candidate(cur, crate::entities::ai_flag_hosts::SOURCE_RETAINED.name())
                    {
                        candidates.push(c);
                    }
                }
            }
        }
        // Source: radar-contacts — nearest faction-hostile, licensed by
        // untargeted combat doctrine (see `destroy_is_untargeted`) OR by being a
        // static point-defence platform. A turret has no doctrine to make its
        // Destroy untargeted, but "shoot the nearest hostile my radar sees" is
        // its entire job; every candidate still passes the selector's own
        // hostility + detectability + radar-horizon eligibility, so this only
        // ever locks a hostile actually in range.
        if destroy_is_untargeted || is_static_point_defence || defend_target.is_some() {
            let origin = defend_target
                .as_deref()
                .and_then(|uuid| target_xz(uuid).map(|(x, z)| [x, 0.0, z]))
                .unwrap_or([physics.x, 0.0, physics.z]);
            if let Some(nearest) = nearest_hostile(origin, registry) {
                if let Some(c) = make_candidate(
                    &nearest,
                    crate::entities::ai_flag_hosts::SOURCE_RADAR.name(),
                ) {
                    candidates.push(c);
                }
            }
        }

        // Self context: position (the selector's own outer horizon filter) plus
        // the authored power rating, exposed as `self_fact(power_rating)` (AC2).
        let mut self_facts = crate::world::flags::AiFacts::new();
        if let Some(pr) = selector_comp.power_rating {
            self_facts.set_fact(crate::entities::ai_flag_hosts::POWER_RATING, pr as f64);
        }
        let self_ctx = crate::ai::selector::SelfContext {
            position: [physics.x, 0.0, physics.z],
            facts: self_facts,
        };

        // Rank. Passing the current lock lets the selector apply switch-margin
        // retention (AC5); an invalid current lock fails eligibility / is absent
        // from the candidates and is replaced this same tick (AC5). The
        // scenario flag chain is anchored at the layer that spawned this ship
        // (issue #891 stage 2).
        let flag_chain = ai_env.flag_chain(ship_entity);
        let selected = selector_comp.selector.select(
            &self_ctx,
            &candidates,
            current_lock.as_deref(),
            &flag_chain,
        );

        // Publish the decision as intent (observability), then send it to the
        // applier as an admitted command.
        record_locked_target_decision(&mut blackboards, selected.clone());
        // Compare before emitting: a `SetTarget` per AI tick would re-resolve
        // and re-broadcast an unchanged lock every tick, and fire the
        // component's change detection with it.
        if weapons_target.0 != selected {
            // Target CHANGED — the single most load-bearing balance line, now a
            // STRUCTURED `ai`-category event (issue #1146): the headline `info`
            // edge carries `tick`, `ship`, `prev` and `new` as tracing fields, so
            // a per-ship target-reselection timeline is `grep`-able out of a run's
            // log stream. Entity-scoped so `--log-entity <ship>` narrows it to one
            // hull. The `debug` line reports the data-driven ranking that produced
            // the pick (the "why" behind the reselection).
            let from = weapons_target
                .0
                .as_deref()
                .map(name_of)
                .unwrap_or_else(|| "none".to_string());
            let to = selected
                .as_deref()
                .map(name_of)
                .unwrap_or_else(|| "none".to_string());
            let tick = sim_tick.as_deref().map(|t| t.0).unwrap_or(0);
            let ship = self_uuid
                .map(|u| name_of(&u.0))
                .unwrap_or_else(|| "<unnamed>".to_string());
            crate::pinfo!(
                log,
                crate::logging::LogCat::Ai,
                entity = ship_entity,
                ai_event = "target_reselection",
                tick = tick,
                ship = ship.as_str(),
                prev = from.as_str(),
                new = to.as_str(),
                "target {from} -> {to}"
            );
            crate::pdebug!(
                log,
                crate::logging::LogCat::Ai,
                entity = ship_entity,
                tick = tick,
                ship = ship.as_str(),
                "acquired {to} via data-driven Tactical selector ({} candidates)",
                candidates.len()
            );
            // Emit, don't write (issue #887). The empty string is how the AI
            // drops a lock: `handle_set_target` clears any target it cannot
            // resolve, which is the same thing it does for a human naming an
            // entity that has since died. Admission re-checks `operate_ai` on
            // `tactical-radar`, so this is refused on exactly the ships the gate
            // above already skipped — belt and braces, not a second policy.
            crate::command_admission::ai_emit::emit_ai_command(
                self_uuid,
                crate::ship::system_registry::tactical_radar_system_id(),
                crate::core::messages::SystemControlPayload::SetTarget {
                    uuid: selected.unwrap_or_default(),
                },
                control_sources,
                &sessions,
                Some(ship_config),
                &mut admitted,
            );
        }
    }
}

/// NPC auto-match phaser frequency to locked target's shield frequency.
///
/// Runs in `SimSet::Input`. When the ship's tactical system is AI-operated
/// and a target is locked, waits `delay_secs` then writes the matching
/// frequency to `ShipPhaserFrequency`.
fn tick_npc_auto_match_frequency(
    time: Res<Time>,
    target_shields_q: Query<(
        &crate::entities::spawner::EntityUuid,
        Option<&crate::ship::shields::ShipShields>,
    )>,
    mut ship_q: Query<(
        Entity,
        &ShipSystemControlSources,
        &crate::ship_plugin::ShipConfigComponent,
        &crate::server_app::ShipSystemBlackboards,
        &mut crate::ship::state::ShipPhaserFrequency,
        Has<crate::ai::server::AiHighFidelity>,
    )>,
    mut states: ResMut<NpcFrequencyMatchStates>,
) {
    let dt = time.delta_secs();
    // Gate: this frequency-hint system only runs for high-fidelity NPCs
    // (issue #692 AC — both frequency-hint systems gated on `AiHighFidelity`).
    //
    // `AiHighFidelity` is read via `Has<>` and folded into the in-loop gate
    // below rather than applied as a `With<AiHighFidelity>` query FILTER on
    // purpose: `NpcFrequencyMatchStates` is only cleaned up here, in the gate's
    // cleanup branch (`states.0.remove(&entity)`), which runs only while the
    // entity is still iterated. A `With<>` filter would stop iterating demoted
    // (no-longer-high-fidelity) NPCs entirely, orphaning their HashMap entries
    // forever — a state leak. `Has<>` + in-loop gate keeps the cleanup path
    // alive so demoted ships' state is pruned. Do not "simplify" this into a
    // query filter.
    for (entity, control_sources, ship_config, blackboards, mut phaser_freq, has_high_fidelity) in
        ship_q.iter_mut()
    {
        if !has_high_fidelity || !any_tactical_system_operates_ai(control_sources, &ship_config.0) {
            states.0.remove(&entity);
            continue;
        }

        // Frozen Combat Lock from this ship's viewscreen (issue #829, spec §3).
        let locked_target = match blackboards
            .0
            .get(&crate::ship::system_registry::viewscreen_system_id())
        {
            Some(crate::core::messages::SystemBlackboard::Viewscreen(bb)) => bb.combat_lock.clone(),
            _ => None,
        };
        let target_frequency = locked_target
            .as_ref()
            .and_then(|uuid| {
                target_shields_q
                    .iter()
                    .find(|(u, _)| u.0.as_str() == uuid.as_str())
                    .and_then(|(_, shields)| shields.map(|s| s.frequency()))
            })
            .unwrap_or(0.5);

        let input = crate::console_ai::FrequencyMatchInput {
            locked_target,
            target_frequency,
            dt,
            delay_secs: NPC_FREQ_MATCH_DELAY,
            trigger_active: true,
        };

        let state = states.0.entry(entity).or_default();
        let output = crate::console_ai::tick_auto_match_frequency(state, &input);

        if let crate::console_ai::FrequencyMatchOutput::Match { frequency } = output {
            phaser_freq.0 = frequency;
        }
    }
}

/// Tactical-owned receiver for delayed Coordination deliveries (issue #1254).
///
/// The generic lag router emits only after resolving this Station to an AI
/// consumer. Tactical retains ownership of what a typed `FrequencyHint` does:
/// latch its frequency for the existing next-tick applier. Re-checking the
/// live owning Station and Tactical control state closes the same late-claim
/// window the applier checks below.
pub(crate) fn receive_tactical_coordination(
    mut delivered: MessageReader<crate::ship_plugin::DeliveredCoordination>,
    mut ships: Query<
        (
            &mut crate::ship_plugin::PendingTacticalFrequencyHint,
            &crate::ship_plugin::ShipSystemControlSources,
            &crate::ship_plugin::ShipConfigComponent,
        ),
        With<crate::server_app::Ship>,
    >,
) {
    for message in delivered.read() {
        if message.delivery != crate::ship_plugin::CoordinationDelivery::Ai {
            continue;
        }
        let Ok((mut pending, control_sources, ship_config)) = ships.get_mut(message.source_entity)
        else {
            continue;
        };
        let crate::core::messages::CoordinationAddress::Station(addressed_station) =
            &message.address
        else {
            continue;
        };
        if ship_config.0.weapons_station().as_ref() != Some(addressed_station)
            || !shared::any_tactical_system_operates_ai(control_sources, &ship_config.0)
        {
            continue;
        }
        if let CoordinationPayload::FrequencyHint { frequency } = &message.payload {
            pending.0 = Some(*frequency);
        }
    }
}

/// Apply a channel-3 `FrequencyHint` that a backfilled Tactical has consumed
/// (issue #873).
///
/// This is the "react" half of the Sensors→Tactical advisory. The routing half
/// lives in `process_coordination_lag`; the Tactical-owned receiver above lands
/// a value in [`PendingTacticalFrequencyHint`] only after the full lag and while
/// this ship's Tactical still operates AI. The reaction delay is the bus's,
/// not a second timer here.
///
/// Deliberately blind to the *sender's* origin (AGENTS.md rule 6): the hint is
/// applied whether it came from a human on Sensors or from that ship's own
/// Sensors AI. Nothing here reads `sender_origin` — by the time a value lands in
/// the slot the router has already decided it may be consumed.
///
/// # Why the receiver's control source IS re-checked
///
/// The receiver's write and this application are a tick apart:
/// `receive_tactical_coordination` lands the router's typed delivery later in
/// `Modifiers`, and this runs in the FOLLOWING tick's Input. A human claiming
/// Tactical inside that window used to get their freshly-dialled phaser
/// frequency silently overwritten by an advisory addressed to the AI that was
/// holding the guns a tick ago. So the applier repeats the router's own predicate,
/// [`shared::any_tactical_system_operates_ai`], and **drops** — never applies —
/// a value whose addressee no longer exists. That is not a human/AI branch on
/// behaviour: it is the same admission question the router asked, re-asked
/// because the answer can change between the two.
///
/// `take()` unconditionally, applied conditionally: a hint is consumed exactly
/// once either way, so a dropped value cannot re-assert itself on a later tick.
///
/// # Why everything but the slot itself is `Option`
///
/// The slot is the only component this system requires. If any of the other
/// three were taken non-optionally, a `Ship` missing one would be filtered OUT
/// of the query rather than iterated — so its pending hint would never be
/// drained, and a hint from an arbitrarily old tick would apply the moment the
/// missing component appeared. Every shipped spawn site attaches all four today,
/// so that was latent rather than live; taking them as `Option` and `take()`ing
/// before any of them is needed makes the "consumed exactly once" sentence above
/// an invariant of this system instead of a property of the current spawn sites.
/// A ship that cannot answer the predicate, or has no frequency to move, DROPS
/// the hint — the same disposition as an addressee that stopped being AI.
pub(crate) fn apply_tactical_frequency_hint(
    mut ship_q: Query<
        (
            &mut crate::ship_plugin::PendingTacticalFrequencyHint,
            Option<&mut crate::ship::state::ShipPhaserFrequency>,
            Option<&crate::ship_plugin::ShipSystemControlSources>,
            Option<&crate::ship_plugin::ShipConfigComponent>,
        ),
        With<crate::server_app::Ship>,
    >,
) {
    for (mut pending, phaser_freq, control_sources, ship_config) in ship_q.iter_mut() {
        // Drain FIRST, unconditionally — before any early-out below can skip it.
        let Some(frequency) = pending.0.take() else {
            continue;
        };
        let (Some(mut phaser_freq), Some(control_sources), Some(ship_config)) =
            (phaser_freq, control_sources, ship_config)
        else {
            continue;
        };
        if !shared::any_tactical_system_operates_ai(control_sources, &ship_config.0) {
            continue;
        }
        phaser_freq.0 = frequency;
    }
}

fn top_destroy_objective_target(
    blackboards: Option<&crate::server_app::ShipSystemBlackboards>,
) -> Option<&str> {
    let bb = blackboards?
        .0
        .get(&crate::ship::system_registry::viewscreen_system_id())?;
    let crate::core::messages::SystemBlackboard::Viewscreen(viewscreen) = bb else {
        return None;
    };
    viewscreen.scored_objectives.iter().find_map(|objective| {
        if objective.score <= 0.0
            || !objective
                .relevance
                .contains(&crate::core::messages::SystemAffinity::Weapons)
        {
            return None;
        }
        match &objective.directive {
            crate::core::messages::AiDirective::Destroy { target } => Some(target.as_str()),
            _ => None,
        }
    })
}

/// Return the entity named by the highest-scored Patrol objective, when that
/// objective explicitly presents a protected target.  This deliberately uses
/// the human snapshot target rather than inventing a second directive variant:
/// route-only Patrol objectives have no target and therefore retain their
/// normal target-selection behaviour.
fn top_defend_objective_target(
    blackboards: Option<&crate::server_app::ShipSystemBlackboards>,
) -> Option<&str> {
    let bb = blackboards?
        .0
        .get(&crate::ship::system_registry::viewscreen_system_id())?;
    let crate::core::messages::SystemBlackboard::Viewscreen(viewscreen) = bb else {
        return None;
    };
    viewscreen.scored_objectives.iter().find_map(|objective| {
        (objective.score > 0.0)
            .then_some(&objective.directive)
            .filter(|directive| {
                matches!(directive, crate::core::messages::AiDirective::Patrol { .. })
            })
            .and_then(|_| objective.snapshot.targets.first().map(String::as_str))
    })
}

/// The target named by the top-scored active OPERATE directive that needs the
/// ship's one combat lock (issue #1162): `Tow`, `Stabilise`, `Escort` (the
/// tractor verbs) and `FieldRepair` (external dispatch). This is the
/// `objective-operate` candidate source — the DIRECTIVE-GATED way `ai_target_
/// selection` reaches a NON-hostile lock.
///
/// Deliberately NOT filtered on `SystemAffinity::Weapons`: an operate directive
/// routes to Engineering / Repair (its owning seat), never Weapons, so the
/// affinity filter `top_destroy_objective_target` uses would drop every one. It
/// is instead matched by directive KIND — only the lock-requiring verbs, so a
/// `Transfer` (dock + umbilical, which use the docked partner, not the combat
/// lock) never pulls a lock. Returns `None` when no such directive is active, so
/// the source injects no candidate and the widened eligibility term stays inert.
/// The returned system kind selects the candidate's authored executable reach:
/// tractor for Tow/Stabilise/Escort, external dispatch for FieldRepair.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum OperateLockSystem {
    Tractor,
    ExternalRepair,
}

pub(super) fn top_operate_objective_target(
    blackboards: Option<&crate::server_app::ShipSystemBlackboards>,
) -> Option<(&str, OperateLockSystem)> {
    let bb = blackboards?
        .0
        .get(&crate::ship::system_registry::viewscreen_system_id())?;
    let crate::core::messages::SystemBlackboard::Viewscreen(viewscreen) = bb else {
        return None;
    };
    viewscreen.scored_objectives.iter().find_map(|objective| {
        if objective.score <= 0.0 {
            return None;
        }
        match &objective.directive {
            crate::core::messages::AiDirective::Tow { target }
            | crate::core::messages::AiDirective::Stabilise { target }
            | crate::core::messages::AiDirective::Escort { target } => {
                Some((target.as_str(), OperateLockSystem::Tractor))
            }
            crate::core::messages::AiDirective::FieldRepair { target } => {
                Some((target.as_str(), OperateLockSystem::ExternalRepair))
            }
            _ => None,
        }
    })
}

pub(super) fn resolve_objective_target_uuid(
    target_name: &str,
    runtime: Option<&crate::world::server::WorldContentRuntime>,
    targetable_q: &Query<
        (
            &crate::entities::spawner::EntityUuid,
            &Transform,
            Option<&crate::entities::spawner::EntityName>,
        ),
        Without<crate::server_app::Asteroid>,
    >,
) -> Option<String> {
    runtime
        .and_then(|rt| rt.name_to_uuid.get(target_name).cloned())
        .or_else(|| {
            targetable_q.iter().find_map(|(uuid, _, name)| {
                (uuid.0 == target_name || name.is_some_and(|n| n.0 == target_name))
                    .then(|| uuid.0.clone())
            })
        })
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
// Fixture ids only (issue #907): a test that needs "some distinct id" has no
// run to reproduce. Production identity is minted by `crate::world_id`, and
// clippy.toml bans `Uuid::new_v4` outside scopes like this one.
#[allow(clippy::disallowed_methods)]
#[path = "server_tests.rs"]
mod tests;
