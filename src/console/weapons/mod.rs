pub mod beam;
pub mod blackboard;
pub mod blaster;
pub mod shared;
pub mod torpedo;

use bevy::prelude::*;

use crate::entity_spawner::FactionComponent;
use crate::messages::{CoordinationPayload, ModifierSlot, SystemBlackboard, WeaponsBlackboard};
use crate::ship_plugin::{CoordinationEnqueue, ShipSystemControlSources};
use crate::ship_state::ShipPhysics;
use crate::simulation::AsteroidUuid;
use crate::torpedo::{TorpedoConfig, TorpedoSystem};

/// Delay before NPC tactical AI auto-matches phaser frequency to the locked
/// target's shield frequency (seconds). Defined here as a tuning constant
/// rather than an inline literal (code review finding #679).
const NPC_FREQ_MATCH_DELAY: f32 = 2.0;

// ── Resources ─────────────────────────────────────────────────────────────

/// A single torpedo-fire command decided by `console_ai::server::
/// ai_torpedo_auto_fire` (issue #694) for `console_ai::server::
/// integrate_torpedo_intents` to apply the same tick. Carries the tube to
/// fire and the locked target UUID `TorpedoSystem::launch` needs.
#[derive(Clone, Debug, PartialEq)]
pub struct TorpedoCmd {
    pub tube_id: crate::torpedo::TorpedoTubeId,
    pub target_uuid: String,
}

/// Per-ship queue of pending torpedo-fire intents. Written by
/// `ai_torpedo_auto_fire`, drained and applied by `integrate_weapons_state`
/// in the same tick.
///
/// Present only while the ship carries `AiHighFidelity` — bundled alongside
/// that marker at every spawn/promote site, mirroring `ShipPowerAiState`'s
/// scoping (see `ai::server::lod_ai_ships` and the `AiHighFidelity` spawn
/// sites in `server_app.rs` / `ship_plugin.rs` / `ai/server.rs`).
#[derive(Component, Default, Clone, Debug)]
pub struct TorpedoIntents(pub Vec<TorpedoCmd>);

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
            beam_color: crate::beam_render::DEFAULT_BEAM_COLOR,
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
        // Admitted-command consumers (issue #833): the tactical-radar selection
        // handler (`tactical-radar`) and the phaser-mode/control handlers
        // (`phaser-control`).
        //
        // NOTE: the weapons FIRE handlers (`handle_fire_phaser` /
        // `handle_fire_torpedo` / `handle_fire_blaster` / `handle_load_tube` /
        // `handle_unload_tube`) are NOT admitted-command consumers — they read
        // `MessageReader<InboundMessage>` for dedicated top-level
        // `ClientMessage` variants that carry no `SystemId` target. That is a
        // separate client→host channel outside admitted routing (a possible
        // future migration, not #833), so `phaser-fore` and `blaster-*` are
        // deliberately not registered here.
        //
        // `torpedo-tube-*` IS registered, and is the exception #833 scoped out.
        // `handle_set_torpedo_volley_target` was migrated to read per-ship
        // `AdmittedCommands` over `With<Ship>` because it had to: the volley
        // order is the only way a tube ever loads, and as an
        // `InboundMessage` + `With<LocalShip>` handler it was unreachable for
        // every NPC — so AI crews never loaded a torpedo and no NPC ever fired
        // one. `ai_torpedo_load` now issues that order through
        // `emit_ai_command`, the same seam and the same `SystemId` a human's
        // `ControlSystem` command travels, which is exactly the symmetry
        // admitted routing exists to express. A prefix matcher covers the
        // per-hull tube ids (`torpedo-tube-fore-port`, …).
        app.register_admitted_consumer(ConsumerMatcher::exact(
            crate::system_registry::TACTICAL_RADAR_SYSTEM_ID,
        ))
        .register_admitted_consumer(ConsumerMatcher::exact(
            crate::system_registry::PHASER_CONTROL_SYSTEM_ID,
        ))
        .register_admitted_consumer(ConsumerMatcher::prefix("torpedo-tube-"));
        app.init_resource::<crate::messages::InterSystemQueue>();
        app.init_resource::<LastWeaponsUpdate>()
            .init_resource::<CurrentPhaserMode>()
            .init_resource::<PhaserRenderConfig>()
            .init_resource::<PhaserCombatConfigResource>()
            .init_resource::<WeaponsUpdateFirstTick>()
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
            .add_observer(on_beam_started)
            .add_observer(on_beam_ended)
            .add_systems(
                Update,
                (
                    // `handle_set_target` is the other `SimSet::Input` writer of
                    // `TacticalRadarSelection`, and is ordered against `ai_target_selection`
                    // below.
                    //
                    // `ai_target_selection` reads `TacticalRadarSelection` (as the seed for
                    // its selection) and writes it back in the same system, so only
                    // two interleavings exist and both keep a human's lock: either
                    // the handler runs first and selection seeds from the fresh
                    // lock, or it runs second and its write lands last.
                    //
                    // The edge is kept anyway, and is worth keeping: it pins the
                    // better of the two, in which admitted human input seeds
                    // selection in the tick it was admitted rather than a tick
                    // later, so a human's fresh lock survives the AI's
                    // read-modify-write. Post-#829 no *other* `Input` system reads
                    // `TacticalRadarSelection` — cross-system consumers read the
                    // frozen viewscreen `combat_lock` — so this edge exists purely
                    // for that human-lock-survives-the-tick atomicity between the
                    // two writers, not to make any same-tick consumer read fresher.
                    // Both gates hold at once on any mixed-rating ship
                    // (`any_bank_accepts_human_input` for a Human phaser bank,
                    // `any_tactical_system_operates_ai` for, say, an Ai torpedo tube
                    // or magazine), so this is an ordinary configuration, not a
                    // contrived one.
                    handle_set_target
                        .in_set(crate::sim_sets::SimSet::Input)
                        .before(ai_target_selection),
                    handle_fire_phaser.in_set(crate::sim_sets::SimSet::Input),
                    // Phaser auto-fire DECIDE half (issue #698). Stays in
                    // `Input`, where the pre-split `tick_phaser_auto_fire` ran,
                    // so it keeps reading pre-`sync_ship_position` `Transform`s
                    // and pre-physics `ShipPhysics` — moving it to `Physics`
                    // alongside `ai_torpedo_auto_fire` would leave it racing
                    // `sync_ship_position` for target positions. Its apply half
                    // is `integrate_weapons_state`, in `Physics` below.
                    ai_phaser_auto_fire.in_set(crate::sim_sets::SimSet::Input),
                    tick_weapons_arc_request.in_set(crate::sim_sets::SimSet::Input),
                    handle_set_phaser_mode.in_set(crate::sim_sets::SimSet::Input),
                    handle_set_phaser_frequency.in_set(crate::sim_sets::SimSet::Input),
                    handle_fire_torpedo.in_set(crate::sim_sets::SimSet::Input),
                    handle_load_tube.in_set(crate::sim_sets::SimSet::Input),
                    handle_unload_tube.in_set(crate::sim_sets::SimSet::Input),
                    handle_set_torpedo_volley_target.in_set(crate::sim_sets::SimSet::Input),
                    // Tactical AI target selection (issues #697, #700).
                    //
                    // Stays in `SimSet::Input` — where the pre-split
                    // `operate_tactical_ai` lived — rather than moving to the
                    // `Physics` + `AiTickLabel` set that ConsoleAiPlugin's
                    // decide/integrate pairs use. It must run in `Input` to stay
                    // ordered against `handle_set_target` (the `.before` edge above
                    // that keeps a human's lock): both are the only writers of
                    // `TacticalRadarSelection`, and that atomicity is the reason for
                    // the shared set. Post-#829 the consumers (`ai_phaser_auto_fire`,
                    // `handle_fire_phaser`, `tick_npc_auto_match_frequency`) no longer
                    // read this component at all — they read the frozen viewscreen
                    // `combat_lock`, which the radar publisher + viewscreen
                    // aggregator derive from this write one tick later — so the set
                    // choice is about writer/writer atomicity, not about feeding
                    // any same-tick reader.
                    ai_target_selection.in_set(crate::sim_sets::SimSet::Input),
                    tick_npc_auto_match_frequency.in_set(crate::sim_sets::SimSet::Input),
                    tick_blaster_auto_fire.in_set(crate::sim_sets::SimSet::Input),
                    handle_fire_blaster.in_set(crate::sim_sets::SimSet::Input),
                ),
            )
            .add_systems(
                Update,
                (
                    // Beam tick split into three phases (issue #723), connected
                    // by the one-tick `BeamContext` resource: prepare writes it,
                    // apply-damage reads/mutates it, tick-lifetimes reads it.
                    // Explicit `.chain()` edges keep the three deterministic
                    // within `SimSet::Damage`. Instance-based `.chain()` rather
                    // than type-set `.after(...)` edges (the
                    // `drain_power_for_active_beam` style below) because the
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
                    // Weapons AI APPLY half (issue #698): drains both
                    // `PhaserIntents` (written in `Input`) and `TorpedoIntents`
                    // (written in `Physics` by `ConsoleAiPlugin`). Registered
                    // here rather than in `ConsoleAiPlugin` because the phaser
                    // half must run for ships that never reach high-fidelity
                    // AI — see `ai_phaser_auto_fire`'s gating docs. The
                    // `.after` is a no-op when `ConsoleAiPlugin` is absent
                    // (test apps), which is exactly what we want: phasers still
                    // integrate, torpedoes have no producer.
                    integrate_weapons_state
                        .in_set(crate::sim_sets::SimSet::Physics)
                        .after(crate::console_ai_plugin::ai_torpedo_auto_fire),
                    // Beam activation moved from `Input` to `Physics` with the
                    // #698 split, so this drain — which used to be guaranteed
                    // to see a beam started one whole phase earlier — needs an
                    // explicit edge to keep draining power on the beam's first
                    // tick rather than one tick late.
                    drain_power_for_active_beam
                        .in_set(crate::sim_sets::SimSet::Physics)
                        .after(integrate_weapons_state),
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
                    // that handle_load_tube emitted in Input this tick, so the
                    // load starts same-tick (issue #512). Ordered after
                    // build_torpedo_target_snapshot / tick_torpedo_lifecycle
                    // so its own state mutations are seen.
                    handle_torpedo_magazine_inter_system.in_set(crate::sim_sets::SimSet::Physics),
                    tick_blaster_system.in_set(crate::sim_sets::SimSet::Physics),
                ),
            )
            .add_systems(
                Update,
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

// Shared weapons utilities extracted to `shared.rs` (issue #721). Re-exported
// so `use super::*;` in the test module keeps resolving them.
pub(crate) use shared::{
    any_tactical_system_operates_ai, live_entity_xz, BeamContext, TorpedoTargetSnapshot,
};

// Blaster systems extracted to `blaster.rs` (issue #726). `BlasterSystemResource`
// stays `pub` here for external consumers (`src/server/pfx.rs`,
// `src/entities/spawner.rs` via the `weapons_plugin` alias); the systems are
// re-exported so the plugin build fn and the test module keep resolving them.
pub use blaster::BlasterSystemResource;
pub(crate) use blaster::{
    handle_blaster_hits, handle_fire_blaster, tick_blaster_auto_fire, tick_blaster_system,
};

// Beam (phaser) types and systems extracted to `beam.rs` (issue #727). The
// types and `drain_power_for_active_beam` stay `pub` here for external
// consumers (`src/server_app.rs` chained re-exports, `src/ship/power.rs`,
// `src/server/pfx.rs`, and friends); the systems are re-exported so the
// plugin build fn and the test module keep resolving them.
pub(crate) use beam::{
    ai_phaser_auto_fire, handle_fire_phaser, handle_set_phaser_frequency, handle_set_phaser_mode,
    handle_set_target, on_beam_ended, on_beam_started, tick_beams_apply_damage, tick_beams_prepare,
    tick_beams_tick_lifetimes,
};
pub use beam::{
    drain_power_for_active_beam, ActiveBeam, BeamEndedEvent, BeamStartedEvent, CurrentPhaserMode,
    LastShipAttacker, PhaserCmd, PhaserCombatConfigResource, PhaserCooldown, PhaserIntents,
    TacticalRadarSelection, BEAM_DAMAGE_PER_SEC, PHASER_BATTERY_DRAIN_PER_SEC,
};

// Torpedo systems extracted to `torpedo.rs` (issue #728). `TorpedoSystemResource`
// and `handle_torpedo_magazine_inter_system` stay `pub` here for external
// consumers (`src/server_app.rs` chained re-exports, `src/server/pfx.rs`,
// `src/entities/spawner.rs`, `src/console_ai/server.rs`, and friends); the
// other systems are re-exported so the plugin build fn and the test module
// keep resolving them.
pub(crate) use torpedo::{
    build_torpedo_target_snapshot, handle_fire_torpedo, handle_load_tube,
    handle_set_torpedo_volley_target, handle_unload_tube, tick_torpedo_lifecycle,
};
pub use torpedo::{handle_torpedo_magazine_inter_system, TorpedoSystemResource};

// Blackboard publish systems, broadcaster, and cache resources extracted to
// `blackboard.rs` (issue #729). `LastWeaponsUpdate`, `compute_current_weapons_update`,
// and `weapons_update_broadcaster` stay `pub` here for external consumers
// (`src/server_app.rs` chained re-exports, `src/core/broadcast/cache_registry.rs`);
// the publish systems and `WeaponsUpdateFirstTick` are re-exported so the plugin
// build fn and the test module keep resolving them.
pub use blackboard::{
    compute_current_weapons_update, weapons_update_broadcaster, LastWeaponsUpdate,
};
pub(crate) use blackboard::{
    publish_phaser_bank_blackboards, publish_tactical_radar_blackboard,
    publish_torpedo_magazine_blackboard, publish_torpedo_tube_blackboards,
    publish_weapons_core_blackboard, WeaponsUpdateFirstTick,
};

/// Adapter: applies the weapons AI's decisions to authoritative weapons state
/// (issue #698).
///
/// Reads [`PhaserIntents`] (written by [`ai_phaser_auto_fire`], `SimSet::Input`)
/// and [`TorpedoIntents`] (written by `console_ai::server::ai_torpedo_auto_fire`,
/// `SimSet::Physics`) and drives the two weapons state machines: `ActiveBeam`
/// activation + `BeamStartedEvent`, and `TorpedoSystem::launch` +
/// `TorpedoLaunched`. It is the single owner of both mutations — no other
/// system drains either buffer.
///
/// This absorbs the former `console_ai::server::integrate_torpedo_intents`
/// wholesale rather than sitting alongside it: two systems both draining
/// `TorpedoIntents` would race, and the AC's "reads both" only means anything
/// if one system owns the drain.
///
/// # Scheduling
/// Runs in `SimSet::Physics`, ordered `.after(ai_torpedo_auto_fire)` — the
/// later of its two producers, and the constraint that forces it out of
/// `Input` where `tick_phaser_auto_fire` used to apply beams. That move is
/// invisible to every `ActiveBeam` reader: `drain_power_for_active_beam` is
/// explicitly ordered `.after` this system (see `WeaponsPlugin::build`), `pfx`
/// runs `.after(SimSet::Physics)` wholesale, and `tick_beams` /
/// `weapons_update_broadcaster` are in `Damage` / `Publish`. The decider
/// deliberately stays in `Input` so it keeps reading pre-physics `Transform`s
/// exactly as the fused system did.
///
/// **Torpedo dual-write.** Mirrors `ship::power::handle_power_messages`'s
/// `Has<LocalShip>` +
/// Resource-sync pattern: when the entity carries its own per-entity
/// `TorpedoSystemResource` Component and is the `LocalShip`, also snapshot the
/// updated Component into the global `TorpedoSystemResource` Resource (legacy
/// Resource path for tests). This matters because a disconnected player's
/// Tactical station can flip to Backfill AI (AGENTS.md rule 5), so the AI path
/// can legitimately be what drives the player's own ship's torpedoes.
///
/// Every component is `Option` on purpose. `ActiveBeam`/`PhaserIntents` are
/// present on every ship but `TorpedoIntents` is `AiHighFidelity`-scoped, and
/// legacy spawns exist without `EntityUuid`/`ShipPhysics`; requiring any of
/// them would silently drop ships the pre-#698 systems served.
pub(crate) fn integrate_weapons_state(
    mut commands: Commands,
    mut ships: Query<
        (
            Entity,
            Option<&crate::entity_spawner::EntityUuid>,
            Option<&ShipPhysics>,
            Option<&mut ActiveBeam>,
            Option<&mut PhaserIntents>,
            Option<&mut TorpedoIntents>,
            Option<&mut TorpedoSystemResource>,
            Has<crate::server_app::LocalShip>,
        ),
        With<crate::server_app::Ship>,
    >,
    mut torpedo_sys_res: ResMut<TorpedoSystemResource>,
    mut outbox: ResMut<crate::simulation::SimOutbox>,
    // `Option<ResMut<Messages<_>>>` so bare-`App` fixtures that never
    // registered the message still pass Bevy's parameter validation.
    mut balance_events: Option<ResMut<bevy::ecs::message::Messages<crate::balance::BalanceEvent>>>,
) {
    for (
        ship_entity,
        ship_uuid,
        physics,
        beam,
        phaser_intents,
        torpedo_intents,
        mut torpedo_sys_comp,
        is_local,
    ) in ships.iter_mut()
    {
        // ── Phasers ──────────────────────────────────────────────────────────
        if let (Some(mut beam), Some(mut intents)) = (beam, phaser_intents) {
            for cmd in intents.0.drain(..) {
                // The decider only proposes a bank while the beam is idle, but
                // `handle_fire_phaser` (SimSet::Input) can start a beam after
                // the decision was taken. Never stomp a live beam.
                if beam.target_uuid.is_some() {
                    continue;
                }
                beam.target_uuid = Some(cmd.target_uuid.clone());
                beam.remaining_secs = cmd.beam_duration_secs;
                beam.damage_accumulator = 0.0;
                beam.bank = Some(cmd.bank.clone());

                commands.trigger(BeamStartedEvent {
                    bank: cmd.bank,
                    target_uuid: cmd.target_uuid,
                    source_entity: ship_entity,
                });
            }
        }

        // ── Torpedoes ────────────────────────────────────────────────────────
        let Some(mut intents) = torpedo_intents else {
            continue;
        };
        if intents.0.is_empty() {
            continue;
        }
        let Some(physics) = physics else {
            continue;
        };
        let source_uuid = ship_uuid.map(|u| u.0.clone());

        {
            // Prefer per-entity component; fall back to global resource for
            // legacy test paths that only set up the Resource.
            let torpedo_sys: &mut crate::torpedo::TorpedoSystem =
                match torpedo_sys_comp.as_deref_mut() {
                    Some(c) => &mut c.0,
                    None => &mut torpedo_sys_res.0,
                };

            for cmd in intents.0.drain(..) {
                let torpedo_uuid = uuid::Uuid::new_v4().to_string();
                let tube_facing_rad = torpedo_sys
                    .tube(cmd.tube_id.as_str())
                    .map(|t| t.facing_deg.to_radians())
                    .unwrap_or(0.0);
                let launch_heading = physics.yaw + tube_facing_rad;
                use crate::torpedo::LaunchResult;
                let result = torpedo_sys.launch(
                    cmd.tube_id.as_str(),
                    torpedo_uuid.clone(),
                    physics.x,
                    physics.z,
                    launch_heading,
                    Some(cmd.target_uuid.clone()),
                    source_uuid.clone(),
                );
                match result {
                    LaunchResult::Launched {
                        uuid: launched_uuid,
                        ..
                    } => {
                        // Balance tracer: the torpedo left the tube. The human
                        // path (`handle_fire_torpedo`) has always written this;
                        // this AI path did not, so every AI-launched torpedo
                        // was invisible in `shots_fired` while its damage still
                        // showed up in `by_weapon`. Now that AI crews actually
                        // load and fire tubes, that gap made the ledger lie.
                        if let Some(ref mut msgs) = balance_events {
                            msgs.write(crate::balance::BalanceEvent::WeaponFired {
                                shooter: source_uuid.clone().filter(|u| !u.is_empty()),
                                weapon: cmd.tube_id.clone(),
                                kind: crate::balance::FIRED_KIND_TORPEDO.to_string(),
                            });
                        }
                        outbox.0.push((
                            crate::lobby::Target::All,
                            crate::messages::ServerMessage::TorpedoLaunched {
                                uuid: launched_uuid,
                                tube: cmd.tube_id,
                                x: physics.x,
                                z: physics.z,
                                heading: launch_heading,
                            },
                        ));
                    }
                    LaunchResult::TubeNotLoaded
                    | LaunchResult::NoTorpedoes
                    | LaunchResult::UnknownTube => {}
                }
            }
        }

        // Dual-write: keep the Resource in sync with the LocalShip's
        // per-entity Component (legacy Resource path for tests).
        if is_local {
            if let Some(c) = torpedo_sys_comp.as_deref() {
                torpedo_sys_res.0 = c.0.clone();
            }
        }
    }
}

/// Tracks the last target for which Weapons asked Helm to bring the phaser
/// arc to bear, so the channel-3 request only fires on a new/changed arc
/// miss rather than every tick (issue #677).
#[derive(Component, Default, Clone)]
pub struct WeaponsArcRequestState {
    pub last_notified_target: Option<String>,
}

/// Emit a channel-3 `ArcBearingRequest` coordination message to Helm whenever
/// the current weapons target is within at least one bank's range but
/// outside every bank's firing arc.
///
/// Iterates every ship (player + NPC), mirroring `tick_sensors_frequency_hint`.
/// Debounced via [`WeaponsArcRequestState`]: only fires when the arc-missed
/// target changes (including transitioning from "no miss" to "miss" or back),
/// not on every tick the same miss persists.
fn tick_weapons_arc_request(
    mut ship_q: Query<
        (
            Entity,
            &ShipSystemControlSources,
            &ShipPhysics,
            &crate::server_app::ShipSystemBlackboards,
            Option<&PhaserCombatConfigResource>,
            Option<&crate::modifiers::ShipModifiers>,
            &mut WeaponsArcRequestState,
        ),
        With<crate::server_app::Ship>,
    >,
    asteroid_q: Query<(&AsteroidUuid, &Transform), Without<crate::entity_spawner::EntityUuid>>,
    entity_q: Query<(&crate::entity_spawner::EntityUuid, &Transform), Without<AsteroidUuid>>,
    entity_name_q: Query<(
        &crate::entities::spawner::EntityUuid,
        &crate::entities::spawner::EntityName,
    )>,
    mut writer: MessageWriter<CoordinationEnqueue>,
) {
    use crate::entity_config::PhaserCombatConfig;

    for (
        ship_entity,
        control_sources,
        physics,
        blackboards,
        combat_config_opt,
        modifiers_opt,
        mut state,
    ) in ship_q.iter_mut()
    {
        // Frozen Combat Lock from this ship's viewscreen (issue #829, spec §3).
        let combat_lock = match blackboards
            .0
            .get(&crate::system_registry::viewscreen_system_id())
        {
            Some(crate::messages::SystemBlackboard::Viewscreen(bb)) => bb.combat_lock.clone(),
            _ => None,
        };
        let Some(target_uuid) = combat_lock else {
            state.last_notified_target = None;
            continue;
        };
        let Some((tx, tz)) = live_entity_xz(&target_uuid, &asteroid_q, &entity_q) else {
            state.last_notified_target = None;
            continue;
        };

        let combat_config_default = PhaserCombatConfigResource::default();
        let combat_config: &PhaserCombatConfigResource =
            combat_config_opt.unwrap_or(&combat_config_default);
        // No banks configured means no meaningful "firing arc" concept —
        // nothing to request Helm to bear on.
        if combat_config.0.banks.is_empty() {
            state.last_notified_target = None;
            continue;
        }
        let modifiers_default = crate::modifiers::ShipModifiers::new();
        let modifiers: &crate::modifiers::ShipModifiers =
            modifiers_opt.unwrap_or(&modifiers_default);

        // A target is a valid arc-request candidate when it's within range of
        // at least one bank but outside every bank's arc — i.e. Weapons could
        // fire if Helm brought the ship around, but can't right now.
        let any_in_range_and_arc = combat_config.0.banks.iter().any(|b| {
            let bank_base_range = if b.beam_range > 0.0 {
                b.beam_range
            } else {
                PhaserCombatConfig::DEFAULT_PHASER_RANGE
            };
            let effective_range = bank_base_range * modifiers.get(&ModifierSlot::RadarRange);
            let range_ok = (tx - physics.x).powi(2) + (tz - physics.z).powi(2)
                <= effective_range * effective_range;
            let (rx, ry) =
                crate::weapons::phaser::ship_local(tx, tz, physics.x, physics.z, physics.yaw);
            let arc_ok = crate::weapons::phaser::in_arc(rx, ry, b.facing_deg, b.auto_arc_deg);
            range_ok && arc_ok
        });
        let any_in_range = combat_config.0.banks.iter().any(|b| {
            let bank_base_range = if b.beam_range > 0.0 {
                b.beam_range
            } else {
                PhaserCombatConfig::DEFAULT_PHASER_RANGE
            };
            let effective_range = bank_base_range * modifiers.get(&ModifierSlot::RadarRange);
            (tx - physics.x).powi(2) + (tz - physics.z).powi(2) <= effective_range * effective_range
        });

        let arc_missed = any_in_range && !any_in_range_and_arc;
        if !arc_missed {
            state.last_notified_target = None;
            continue;
        }

        if state.last_notified_target.as_deref() == Some(target_uuid.as_str()) {
            // Already notified Helm about this exact arc miss; debounced.
            continue;
        }
        state.last_notified_target = Some(target_uuid.clone());

        let label = entity_name_q
            .iter()
            .find_map(|(u, n)| (u.0 == target_uuid).then(|| n.0.clone()))
            .unwrap_or_else(|| target_uuid.clone());

        // No coarse weapons SystemId exists; the fore phaser bank is used as
        // the representative sender for control-source resolution, mirroring
        // `emit_shields_coordination`'s "first arc as representative sender".
        let sender_origin = control_sources
            .0
            .source_for(&crate::system_registry::phaser_fore_system_id());

        writer.write(CoordinationEnqueue {
            source_entity: ship_entity,
            sender_origin,
            target: crate::system_registry::helm_station_key(),
            payload: CoordinationPayload::ArcBearingRequest {
                uuid: target_uuid,
                label,
            },
            sender_label: "Weapons".to_string(),
        });
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
// This system does not fire weapons. Issue #698 split firing itself into a
// decide/integrate pair that *does* straddle sim sets: `ai_phaser_auto_fire` /
// `ai_torpedo_auto_fire` decide and write `PhaserIntents` / `TorpedoIntents`;
// `integrate_weapons_state` is the sole system that mutates `ActiveBeam` and
// `TorpedoSystem` from those intents.

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
        .entry(crate::system_registry::tactical_station_key())
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
        .get_mut(&crate::system_registry::tactical_station_key())
    {
        weapons.locked_target = None;
    }
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
/// The decision is applied to `TacticalRadarSelection` here, in the same system that
/// makes it. `TacticalRadarSelection` is the single source of truth every consumer reads
/// (`handle_fire_phaser`, `ai_phaser_auto_fire`, `ai_torpedo_auto_fire`,
/// `tick_npc_auto_match_frequency`, …), and this is the only path by which the
/// AI reaches it — a human-operated Tactical's lock is never overwritten,
/// because the `operate_ai` gate below skips the ship entirely.
///
/// Seeding the selection from `TacticalRadarSelection` and writing it back within one
/// system is deliberate: it makes the AI's read-modify-write atomic with
/// respect to `handle_set_target`, the only other `SimSet::Input` writer of
/// `TacticalRadarSelection`. `Input` has no intra-set ordering by default, so a
/// separate integrator system could be scheduled between the two and write back
/// a decision made before the human's `SetTarget` landed, silently dropping the
/// human's lock on any mixed-rating ship. Do not re-split this without
/// re-establishing that ordering — see `WeaponsPlugin::build` and
/// `human_set_target_survives_the_tick_on_a_mixed_rating_ship`.
fn ai_target_selection(
    // `Option<Res<_>>`, never a bare `Res` — this system runs in bare-`App`
    // weapons fixtures that never insert `LogFilterConfig` (see the macro docs).
    log: Option<Res<crate::logging::LogFilterConfig>>,
    mut ship_query: Query<
        (
            Entity,
            &crate::ship_plugin::ShipConfigComponent,
            &ShipSystemControlSources,
            &LastShipAttacker,
            &ShipPhysics,
            &mut TacticalRadarSelection,
            &mut crate::server_app::ShipSystemBlackboards,
            Option<&crate::modifiers::ShipModifiers>,
            Option<&crate::entity_spawner::WeaponsConsoleSection>,
            // Self identity + faction, for the nearest-hostile tier (#703):
            // the UUID excludes self from the scan, the faction decides who
            // counts as hostile. Both `Option` because minimal test spawns
            // omit them; a ship with no faction acquires nothing this way.
            Option<&crate::entity_spawner::EntityUuid>,
            Option<&FactionComponent>,
        ),
        With<crate::server_app::Ship>,
    >,
    asteroid_q: Query<(&AsteroidUuid, &Transform), With<crate::simulation::Asteroid>>,
    runtime: Option<Res<crate::world::server::WorldContentRuntime>>,
    // `Option` so test apps without the entity-config cache still run; an
    // absent registry behaves as an empty one, i.e. nobody is hostile.
    faction_registry: Option<Res<crate::entities::config_cache::FactionRegistryResource>>,
    other_ships_q: Query<
        (
            &crate::entity_spawner::EntityUuid,
            &Transform,
            Option<&crate::entities::spawner::EntityName>,
        ),
        Without<crate::simulation::Asteroid>,
    >,
    // The tier-4 scan surface, deliberately narrower than `other_ships_q`.
    // That query is `Without<Asteroid>`, which is wide enough for resolving an
    // authored name (a mission may name anything, and a miss is harmless), but
    // as an *auto-acquisition* surface it would lock any factioned entity that
    // happens to carry an `EntityUuid` + `Transform`. `With<Ship>` is the code
    // spelling of the tactical radar's own `shows: [EntityTag::Ship]`: today
    // every shipped asset declaring a `faction` is a ship, so the two agree by
    // accident — the first factioned station, mine, or probe template is what
    // this filter is here for.
    hostile_scan_q: Query<
        (
            &crate::entity_spawner::EntityUuid,
            &Transform,
            Option<&FactionComponent>,
        ),
        With<crate::server_app::Ship>,
    >,
) {
    let registry_default = crate::faction::FactionRegistry::default();
    let registry: &crate::faction::FactionRegistry = faction_registry
        .as_deref()
        .map(|r| &r.0)
        .unwrap_or(&registry_default);

    // World-space (x, z) of a targetable UUID, asteroid or entity.
    let target_xz = |uuid: &str| -> Option<(f32, f32)> {
        asteroid_q
            .iter()
            .find_map(|(u, t)| (u.0 == uuid).then_some((t.translation.x, t.translation.z)))
            .or_else(|| {
                other_ships_q.iter().find_map(|(u, t, _)| {
                    (u.0 == uuid).then_some((t.translation.x, t.translation.z))
                })
            })
    };

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
        mut weapons_target,
        mut blackboards,
        modifiers,
        weapons_section,
        self_uuid,
        self_faction,
    ) in ship_query.iter_mut()
    {
        // Only select for ships whose Tactical surface is AI-controlled.
        // Post-#512, "tactical is AI-controlled" means "at least one tactical
        // fine system (phaser bank, torpedo tube, or the torpedo magazine) has
        // `operate_ai == true` on its own policy". Ships that declare no
        // tactical fine systems (test / legacy) fall back to the coarse
        // `tactical.operate_ai` policy.
        //
        // The player ship's Tactical fine systems may be human — select nothing
        // in that case; the human operator drives `TacticalRadarSelection` directly via
        // `handle_set_target`. Clearing the intent here stops a ship that flips
        // from AI to human control leaving a stale selection on its blackboard.
        if !any_tactical_system_operates_ai(control_sources, &ship_config.0) {
            clear_locked_target_if_present(&mut blackboards);
            continue;
        }

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

        // Start from the ship's current lock: acquisition below only *replaces*
        // it when a fresh in-range candidate exists, and the staleness guard
        // only clears it — preserving the pre-split semantics exactly.
        //
        // Cloned out of the `Mut` once, so the retention tier below can read it
        // without holding a borrow across the write-back at the end.
        let current_lock: Option<String> = weapons_target.0.clone();
        let mut selected: Option<String> = current_lock.clone();

        // Acquire from Destroy objectives, falling back to the lock we already
        // hold, then to the last attacker, then to the nearest hostile.
        let top_destroy = top_destroy_objective_target(Some(&*blackboards));
        // An *untargeted* Destroy directive — `Destroy { target: "" }` — is
        // standing "engage any hostile you detect" doctrine, which is what
        // every shipped hostile TOML authors (`directive_kind = "Destroy"` with
        // no `directive_target`). It is the only case that licenses the
        // nearest-hostile scan: a Destroy naming someone specific must not
        // wander onto a different ship just because that ship is closer.
        let destroy_is_untargeted = matches!(top_destroy, Some(""));
        let objective_target = match top_destroy {
            Some("") => None,
            Some(target_name) => {
                resolve_objective_target_uuid(target_name, runtime.as_deref(), &other_ships_q)
            }
            None => None,
        };

        // Tier 2: keep the engagement we are already in — "still resolvable,
        // and still inside our own radar horizon". See the ordering rationale on
        // this system's doc comment: without this tier the nearest-hostile scan
        // below re-decides from scratch every tick, and since the helm pursues
        // this lock, the ship slews between bearings as well as retargeting.
        let retained_lock = || -> Option<String> {
            let current = current_lock.clone()?;
            let alive = target_xz(&current).is_some();
            // An untargeted Destroy directive is combat doctrine: retain only
            // an opposing ship. This lets a combat_test attacker drop its
            // factionless Starbase assault lock after `not_attacked` closes
            // that named objective, then acquire the player/attacker.
            let combat_appropriate = if destroy_is_untargeted {
                self_faction.map(|f| f.0).is_some_and(|self_faction_uuid| {
                    hostile_scan_q
                        .iter()
                        .find_map(|(u, _, faction)| {
                            (u.0 == current).then_some(faction.map(|f| f.0)).flatten()
                        })
                        .is_some_and(|target_faction| {
                            crate::faction::is_enemy(
                                Some(self_faction_uuid),
                                Some(target_faction),
                                registry,
                            )
                        })
                })
            } else {
                true
            };
            (alive && combat_appropriate && (!range_bounds_targets || within_range(&current)))
                .then_some(current)
        };

        // Tier 4 (issue #703): standing Destroy doctrine with nobody named,
        // nothing already locked, and nobody having shot us yet. Before this
        // tier an NPC could only acquire a weapons target *after* taking a
        // phaser hit (the only writer of `LastShipAttacker`), so shipped
        // hostiles never opened fire.
        //
        // Delegates the faction verdict and the distance ordering to
        // `ai::core::find_nearest_hostile` over a `WorldView` built here, rather
        // than open-coding "hostile" and "nearest" a second time.
        let nearest_hostile = |registry: &crate::faction::FactionRegistry| -> Option<String> {
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
                entity_pos: [physics.x, 0.0, physics.z],
                entity_yaw: physics.yaw,
                entities,
                self_faction: Some(self_faction_uuid),
                ..crate::ai::WorldView::default()
            };
            let found = crate::ai::find_nearest_hostile(&world_view, registry)?;
            // Map back to the entity's own UUID string rather than
            // re-serialising, so the result is byte-identical to what
            // `target_xz` / `live_entity_xz` look up.
            hostile_scan_q.iter().find_map(|(u, _, _)| {
                (uuid::Uuid::parse_str(&u.0).ok() == Some(found)).then(|| u.0.clone())
            })
        };

        // Which tier produced the acquisition, kept alongside the result for the
        // `debug`-level "why this target" line below. The `if let ... else if`
        // chain preserves the exact short-circuit laziness of the original
        // `.or_else` chain: `retained_lock` and `nearest_hostile` are only
        // evaluated when the higher tiers yield `None`.
        let (acquired, acquired_tier): (Option<String>, &'static str) =
            if let Some(t) = objective_target {
                (Some(t), "objective")
            } else if let Some(t) = retained_lock() {
                (Some(t), "retained")
            } else if let Some(t) = last_attacker.0.clone() {
                (Some(t), "last-attacker")
            } else if let Some(t) = destroy_is_untargeted
                .then(|| nearest_hostile(registry))
                .flatten()
            {
                (Some(t), "nearest-hostile")
            } else {
                (None, "none")
            };

        // The radar gate applies to every tier alike (issue #680): a ship must
        // not lock what its own damage-scaled tactical radar cannot see.
        if let Some(uuid) = acquired {
            if !range_bounds_targets || within_range(&uuid) {
                selected = Some(uuid);
            }
        }

        // Stale-target guard: if the selection points at an entity that no
        // longer exists in the world, drop it. This prevents AI from sitting
        // idle after its last Destroy-objective target is killed — without this
        // guard, ai_phaser_auto_fire and the torpedo path both skip on the
        // dead entity UUID and never acquire a fresh target.
        // Also drops targets beyond radar range (issue #680).
        if let Some(current) = selected.clone() {
            let alive = target_xz(&current).is_some();
            if !alive || (range_bounds_targets && !within_range(&current)) {
                selected = None;
            }
        }

        // Publish the decision as intent (observability), then apply it to the
        // authoritative lock.
        record_locked_target_decision(&mut blackboards, selected.clone());
        // Compare before writing: an unconditional assignment through `Mut`
        // would fire change detection every tick even when the lock is
        // unchanged.
        if weapons_target.0 != selected {
            // Target CHANGED — the single most load-bearing balance line: the
            // headline `info` edge names the from→to, and the `debug` line
            // records which acquisition tier won (why this target). Entity-
            // scoped so `--log-entity <ship>` narrows it to one hull.
            let from = weapons_target
                .0
                .as_deref()
                .map(name_of)
                .unwrap_or_else(|| "none".to_string());
            let to = selected
                .as_deref()
                .map(name_of)
                .unwrap_or_else(|| "none".to_string());
            crate::pinfo!(
                log,
                crate::logging::LogCat::Ai,
                entity = ship_entity,
                "target {from} -> {to}"
            );
            crate::pdebug!(
                log,
                crate::logging::LogCat::Ai,
                entity = ship_entity,
                "acquired {to} via tier {acquired_tier}"
            );
            weapons_target.0 = selected;
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
        &crate::entity_spawner::EntityUuid,
        Option<&crate::ship::shields::ShipShields>,
    )>,
    mut ship_q: Query<(
        Entity,
        &ShipSystemControlSources,
        &crate::ship_plugin::ShipConfigComponent,
        &crate::server_app::ShipSystemBlackboards,
        &mut crate::ship_state::ShipPhaserFrequency,
        Has<crate::ai_plugin::AiHighFidelity>,
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
            .get(&crate::system_registry::viewscreen_system_id())
        {
            Some(crate::messages::SystemBlackboard::Viewscreen(bb)) => bb.combat_lock.clone(),
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

fn top_destroy_objective_target(
    blackboards: Option<&crate::server_app::ShipSystemBlackboards>,
) -> Option<&str> {
    let bb = blackboards?
        .0
        .get(&crate::system_registry::viewscreen_system_id())?;
    let crate::messages::SystemBlackboard::Viewscreen(viewscreen) = bb else {
        return None;
    };
    viewscreen.scored_objectives.iter().find_map(|objective| {
        if objective.score <= 0.0
            || !objective
                .relevance
                .contains(&crate::messages::SystemAffinity::Weapons)
        {
            return None;
        }
        match &objective.directive {
            crate::messages::AiDirective::Destroy { target } => Some(target.as_str()),
            _ => None,
        }
    })
}

fn resolve_objective_target_uuid(
    target_name: &str,
    runtime: Option<&crate::world::server::WorldContentRuntime>,
    targetable_q: &Query<
        (
            &crate::entity_spawner::EntityUuid,
            &Transform,
            Option<&crate::entities::spawner::EntityName>,
        ),
        Without<crate::simulation::Asteroid>,
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
#[path = "server_tests.rs"]
mod tests;
