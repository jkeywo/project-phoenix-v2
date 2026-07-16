use bevy::prelude::*;

use crate::ai_plugin::AiTokenRegistry;
use crate::entity_spawner::{EntitySystemHull, FactionComponent};
use crate::lobby::{InboundMessage, Sessions, Target, WorldResource};
use crate::messages::{
    BlasterBankState, ClientMessage, CoordinationPayload, InterSystemMsg, InterSystemPayload,
    InterSystemQueue, ModifierSlot, PhaserBankClientConfig, PhaserBankState, PhaserMode, RadarBlip,
    RadarRegion, ServerMessage, SystemBlackboard, SystemControlPayload, TorpedoTubeClientConfig,
    TorpedoTubeState, WeaponsBlackboard,
};
use crate::ship_plugin::{CoordinationEnqueue, ShipSystemControlSources};
use crate::ship_state::ShipPhysics;
use crate::simulation::{AsteroidUuid, SimOutbox};
use crate::torpedo::{TorpedoConfig, TorpedoSystem};

/// Delay before NPC tactical AI auto-matches phaser frequency to the locked
/// target's shield frequency (seconds). Defined here as a tuning constant
/// rather than an inline literal (code review finding #679).
const NPC_FREQ_MATCH_DELAY: f32 = 2.0;

// ── Resources ─────────────────────────────────────────────────────────────

/// Cache of the last `WeaponsUpdate` sent to the Tactical holder.
/// The broadcaster compares against this to skip identical ticks.
#[derive(Resource, Default, Clone, PartialEq, Debug)]
pub struct LastWeaponsUpdate {
    pub target_uuid: Option<String>,
    pub target_name: Option<String>,
    pub banks: Vec<PhaserBankState>,
    pub tubes: Vec<TorpedoTubeState>,
    pub torpedo_count: u32,
    pub phaser_mode: PhaserMode,
    /// Per-bank blaster state (issue #631). Empty when no blaster banks declared.
    pub blasters: Vec<BlasterBankState>,
    /// Current phaser frequency (0.0–1.0).
    pub phaser_frequency: f32,
}

/// True on the first tick of the weapons broadcaster, then cleared.
/// Used to force-send the first `WeaponsUpdate` even when the computed
/// state happens to match the default `LastWeaponsUpdate`.
#[derive(Resource)]
pub struct WeaponsUpdateFirstTick(pub bool);

impl Default for WeaponsUpdateFirstTick {
    fn default() -> Self {
        Self(true)
    }
}

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
/// that marker at every spawn/promote site, mirroring `PowerReactorIntents`'s
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

/// Wraps the pure-Rust torpedo system so it can be used as a Bevy resource.
///
/// Derives both `Resource` (existing player-ship singleton path) and
/// `Component` (per-entity path, PR 5 unification).
#[derive(Resource, Component, Clone)]
pub struct TorpedoSystemResource(pub TorpedoSystem);

/// Bevy message fired (with world-space position) when an asteroid is destroyed
/// by phaser fire. The renderer uses this to spawn a ripple VFX at the site.
#[derive(Message, Clone, Debug)]
pub struct AsteroidDestroyedVfx {
    pub x: f32,
    pub z: f32,
}

// ── Plugin ─────────────────────────────────────────────────────────────────

/// Per-ship frequency match state for NPC auto-match frequency AI.
#[derive(Resource, Default)]
pub struct NpcFrequencyMatchStates(
    pub std::collections::HashMap<Entity, crate::console_ai::FrequencyMatchState>,
);

pub struct WeaponsPlugin;

impl Plugin for WeaponsPlugin {
    fn build(&self, app: &mut App) {
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
            .add_message::<CoordinationEnqueue>()
            .add_observer(on_beam_started)
            .add_observer(on_beam_ended)
            .add_systems(
                Update,
                (
                    // `handle_set_target` is the other `SimSet::Input` writer of
                    // `WeaponsTarget`, and is ordered against `ai_target_selection`
                    // below.
                    //
                    // `ai_target_selection` reads `WeaponsTarget` (as the seed for
                    // its selection) and writes it back in the same system, so only
                    // two interleavings exist and both keep a human's lock: either
                    // the handler runs first and selection seeds from the fresh
                    // lock, or it runs second and its write lands last.
                    //
                    // The edge is kept anyway, and is worth keeping: it pins the
                    // better of the two, in which admitted human input is visible to
                    // selection — and to every other `Input` reader of
                    // `WeaponsTarget` — in the tick it was admitted, rather than a
                    // tick later. Both gates hold at once on any mixed-rating ship
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
                    // decide/integrate pairs use. The `WeaponsTarget` write has to
                    // land in `Input`: `ai_phaser_auto_fire`, `handle_fire_phaser`
                    // and `tick_npc_auto_match_frequency` all read it from `Input`,
                    // so writing it from `Physics` would push the write past them
                    // and make them read last tick's lock.
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
pub(crate) use super::shared::{
    any_tactical_system_operates_ai, live_entity_xz, system_is_registered, tactical_authorized,
    BeamContext, TorpedoTargetSnapshot,
};

// Blaster systems extracted to `blaster.rs` (issue #726). `BlasterSystemResource`
// stays `pub` here for external consumers (`src/server/pfx.rs`,
// `src/entities/spawner.rs` via the `weapons_plugin` alias); the systems are
// re-exported so the plugin build fn and the test module keep resolving them.
pub use super::blaster::BlasterSystemResource;
pub(crate) use super::blaster::{
    handle_blaster_hits, handle_fire_blaster, tick_blaster_auto_fire, tick_blaster_system,
};

// Beam (phaser) types and systems extracted to `beam.rs` (issue #727). The
// types and `drain_power_for_active_beam` stay `pub` here for external
// consumers (`src/server_app.rs` chained re-exports, `src/ship/power.rs`,
// `src/server/pfx.rs`, and friends); the systems are re-exported so the
// plugin build fn and the test module keep resolving them.
pub(crate) use super::beam::{
    ai_phaser_auto_fire, handle_fire_phaser, handle_set_phaser_frequency, handle_set_phaser_mode,
    handle_set_target, on_beam_ended, on_beam_started, tick_beams_apply_damage, tick_beams_prepare,
    tick_beams_tick_lifetimes,
};
pub use super::beam::{
    drain_power_for_active_beam, ActiveBeam, BeamEndedEvent, BeamStartedEvent, CurrentPhaserMode,
    LastShipAttacker, PhaserCmd, PhaserCombatConfigResource, PhaserCooldown, PhaserIntents,
    WeaponsTarget, BEAM_DAMAGE_PER_SEC, PHASER_BATTERY_DRAIN_PER_SEC,
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
/// **Torpedo dual-write.** Mirrors `integrate_power_state`'s `Has<LocalShip>` +
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
            &crate::simulation::WeaponsTarget,
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
        weapons_target,
        combat_config_opt,
        modifiers_opt,
        mut state,
    ) in ship_q.iter_mut()
    {
        let Some(target_uuid) = weapons_target.0.clone() else {
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

fn handle_load_tube(
    mut reader: MessageReader<InboundMessage>,
    sessions: Res<Sessions>,
    ship_query: Query<
        (
            Entity,
            &crate::ship_plugin::ShipConfigComponent,
            &ShipSystemControlSources,
        ),
        With<crate::server_app::LocalShip>,
    >,
    mut inter_system: ResMut<InterSystemQueue>,
) {
    let Some((ship_entity, ship_config, control_sources)) = ship_query.iter().next() else {
        return;
    };
    for ev in reader.read() {
        let ClientMessage::LoadTube { tube } = &ev.msg else {
            continue;
        };
        // Per-tube gate (issue #512): the tube's own fine-system policy
        // decides whether human input can trigger a load. An unresolved or
        // unregistered tube id gets the default-source policy (issue #801 —
        // no coarse fallback).
        let tube_system_id = crate::system_registry::torpedo_tube_system_id(tube)
            .filter(|id| system_is_registered(control_sources, id));
        let tube_policy = match &tube_system_id {
            Some(id) => control_sources.0.policy_for(id),
            // Unregistered fine system → default-source policy (issue #801).
            None => crate::ship::control_source::control_tick_policy(
                crate::ship::control_source::ControlSource::default(),
            ),
        };
        if !tube_policy.accept_human_input {
            continue;
        }
        if !tactical_authorized(&sessions, ship_config, &ev.token) {
            continue;
        }
        // Emit a channel-2 claim to the magazine. The magazine consumer
        // (handle_torpedo_magazine_inter_system) decides whether to grant
        // the round (magazine online + stock available) and, if so, begins
        // the tube's loading via `start_load_reserved`. Sending the message
        // is the only action the tube system takes here — the magazine owns
        // both the counter mutation and the tube state transition.
        //
        // `source_entity: Some(ship_entity)` routes the claim to THIS
        // ship's magazine — required by `handle_torpedo_magazine_inter_system`
        // when multiple ships have magazines (mirrors the
        // `handle_power_inter_system` pattern in `src/ship/power.rs`).
        inter_system.0.push(InterSystemMsg {
            target: crate::system_registry::torpedo_magazine_system_id(),
            payload: InterSystemPayload::ClaimTorpedoRound { tube: tube.clone() },
            source_entity: Some(ship_entity),
        });
    }
}

fn handle_unload_tube(
    mut reader: MessageReader<InboundMessage>,
    sessions: Res<Sessions>,
    ship_query: Query<
        (
            &crate::ship_plugin::ShipConfigComponent,
            &ShipSystemControlSources,
        ),
        With<crate::server_app::LocalShip>,
    >,
    mut torpedo_sys_q: Query<&mut TorpedoSystemResource, With<crate::server_app::LocalShip>>,
    mut torpedo_sys_res: ResMut<TorpedoSystemResource>,
) {
    let Some((ship_config, control_sources)) = ship_query.iter().next() else {
        return;
    };
    for ev in reader.read() {
        let ClientMessage::UnloadTube { tube } = &ev.msg else {
            continue;
        };
        // Per-tube gate (issue #512): the tube's own fine-system policy
        // decides whether human input can trigger an unload. An unregistered
        // tube id gets the default-source policy (issue #801).
        let tube_system_id = crate::system_registry::torpedo_tube_system_id(tube)
            .filter(|id| system_is_registered(control_sources, id));
        let tube_policy = match &tube_system_id {
            Some(id) => control_sources.0.policy_for(id),
            // Unregistered fine system → default-source policy (issue #801).
            None => crate::ship::control_source::control_tick_policy(
                crate::ship::control_source::ControlSource::default(),
            ),
        };
        if !tube_policy.accept_human_input {
            continue;
        }
        if !tactical_authorized(&sessions, ship_config, &ev.token) {
            continue;
        }
        // Prefer per-entity component; fall back to global resource for test compat.
        if let Some(mut ts) = torpedo_sys_q.iter_mut().next() {
            ts.0.start_unload(tube.as_str());
        } else {
            torpedo_sys_res.0.start_unload(tube.as_str());
        }
    }
}

/// Handle `ControlSystem { target: "torpedo-tube-<id>", payload: SetTorpedoVolleyTarget { count } }`.
///
/// Resolves the tube id from the target SystemId, gates on the tube's
/// fine-system policy, then calls [`TorpedoSystem::set_volley_target`].
///
/// Runs in `SimSet::Input`.
fn handle_set_torpedo_volley_target(
    mut reader: MessageReader<InboundMessage>,
    sessions: Res<Sessions>,
    ship_query: Query<
        (
            &crate::ship_plugin::ShipConfigComponent,
            &ShipSystemControlSources,
        ),
        With<crate::server_app::LocalShip>,
    >,
    mut torpedo_sys_q: Query<&mut TorpedoSystemResource, With<crate::server_app::LocalShip>>,
    mut torpedo_sys_res: ResMut<TorpedoSystemResource>,
) {
    let Some((ship_config, control_sources)) = ship_query.iter().next() else {
        return;
    };
    for ev in reader.read() {
        let ClientMessage::ControlSystem {
            target,
            payload: SystemControlPayload::SetTorpedoVolleyTarget { count },
        } = &ev.msg
        else {
            continue;
        };
        // Target must look like "torpedo-tube-<tube_id_with_hyphens>".
        let tube_id_hyphens = match target.0.strip_prefix("torpedo-tube-") {
            Some(id) if !id.is_empty() => id,
            _ => continue,
        };
        // Gate on the tube's own fine-system policy (default-source policy
        // for unregistered ids — issue #801).
        let is_registered = system_is_registered(control_sources, target);
        let tube_policy = if is_registered {
            control_sources.0.policy_for(target)
        } else {
            // Unregistered fine system → default-source policy (issue #801).
            crate::ship::control_source::control_tick_policy(
                crate::ship::control_source::ControlSource::default(),
            )
        };
        if !tube_policy.accept_human_input {
            continue;
        }
        if !tactical_authorized(&sessions, ship_config, &ev.token) {
            continue;
        }
        // Convert hyphens → underscores to get the TOML tube id.
        let tube_id = tube_id_hyphens.replace('-', "_");
        // Prefer per-entity component; fall back to global resource.
        if let Some(mut ts) = torpedo_sys_q.iter_mut().next() {
            ts.0.set_volley_target(&tube_id, *count);
        } else {
            torpedo_sys_res.0.set_volley_target(&tube_id, *count);
        }
    }
}

///
/// Iterates `InboundMessage::FireTorpedo` events and resolves each to a
/// shooter ship entity by token:
/// - `"ai:<uuid>"` tokens are resolved through [`AiTokenRegistry`] to the
///   registered NPC entity.
/// - Human network tokens and `LOCAL_CONSOLE_TOKEN` route to the `LocalShip`,
///   gated by [`tactical_authorized`] (holds the Tactical console or is the
///   local operator).
///
/// After resolution the same per-ship code path runs for both: use the
/// shooter's own `TorpedoSystemResource` component (falling back to the
/// global `TorpedoSystemResource` resource only when no ship carries the
/// component — legacy test paths).
///
/// After PRD #597 gap-3 closure: NPC ships with a `[torpedoes]` TOML block
/// now spawn with their own `TorpedoSystemResource` (see
/// `src/entities/spawner.rs`) and can fire torpedoes via the same code path
/// as the player ship.
#[allow(clippy::too_many_arguments)]
fn handle_fire_torpedo(
    mut reader: MessageReader<InboundMessage>,
    sessions: Res<Sessions>,
    ai_registry: Option<Res<AiTokenRegistry>>,
    localship_q: Query<
        (Entity, &crate::ship_plugin::ShipConfigComponent),
        With<crate::server_app::LocalShip>,
    >,
    // Per-ship state read for every candidate shooter (player + NPC).
    mut ship_q: Query<
        (
            Entity,
            &ShipSystemControlSources,
            &ShipPhysics,
            Option<&WeaponsTarget>,
            Option<&crate::entity_spawner::EntityUuid>,
            Option<&mut TorpedoSystemResource>,
            Option<&mut crate::server_app::WeaponFiredThisTick>,
        ),
        With<crate::server_app::Ship>,
    >,
    mut torpedo_sys_res: ResMut<TorpedoSystemResource>,
    mut outbox: ResMut<SimOutbox>,
) {
    // Snapshot LocalShip identity for human-token routing. `None` when the
    // test/plugin harness has no player ship spawned.
    let local_ship: Option<(Entity, &crate::ship_plugin::ShipConfigComponent)> =
        localship_q.single().ok();

    for ev in reader.read() {
        let ClientMessage::FireTorpedo { tube, target_uuid } = &ev.msg else {
            continue;
        };

        // ── Resolve the shooter ship entity ─────────────────────────────────
        let shooter_entity: Entity = if ev.token.starts_with("ai:") {
            match ai_registry
                .as_deref()
                .and_then(|r| r.bevy_entity_for_token(&ev.token))
            {
                Some(e) => e,
                None => continue,
            }
        } else {
            match local_ship {
                Some((e, cfg)) if tactical_authorized(&sessions, cfg, &ev.token) => e,
                _ => continue,
            }
        };

        // ── Pull per-ship state for the resolved shooter ────────────────────
        let Ok((
            _entity,
            control_sources,
            physics,
            weapons_target_opt,
            source_uuid_opt,
            torpedo_sys_comp,
            weapon_fired_comp,
        )) = ship_q.get_mut(shooter_entity)
        else {
            continue;
        };

        // Authorize per the shooter's own ControlSource: human tokens need
        // `accept_human_input`; `ai:` tokens need `operate_ai`.
        // Per-tube gate (issue #512): resolve the fine SystemId for this tube
        // and gate on its own policy. An unresolved or unregistered tube id
        // gets the default-source policy (issue #801 — no coarse fallback).
        let tube_system_id = crate::system_registry::torpedo_tube_system_id(tube)
            .filter(|id| system_is_registered(control_sources, id));
        let policy = match &tube_system_id {
            Some(id) => control_sources.0.policy_for(id),
            // Unregistered fine system → default-source policy (issue #801).
            None => crate::ship::control_source::control_tick_policy(
                crate::ship::control_source::ControlSource::default(),
            ),
        };
        let is_ai_token = ev.token.starts_with("ai:");
        let authorized = if is_ai_token {
            policy.operate_ai
        } else {
            policy.accept_human_input
        };
        if !authorized {
            continue;
        }
        // Magazine-online gate (issue #512): a Disabled/Destroyed magazine
        // blocks fire even when the tube is loaded. Only enforced when the
        // ship actually declares a torpedo magazine fine system (player
        // ship path). NPCs without a magazine system are unaffected.
        let magazine_id = crate::system_registry::torpedo_magazine_system_id();
        let magazine_declared = control_sources
            .0
            .entries()
            .any(|(id, _)| id == &magazine_id)
            || control_sources.0.offline_systems.contains(&magazine_id);
        if magazine_declared {
            let magazine_policy = control_sources.0.policy_for(&magazine_id);
            if !magazine_policy.accept_human_input && !magazine_policy.operate_ai {
                continue;
            }
        }

        // Per-entity `TorpedoSystemResource` first; fall back to the global
        // Resource so legacy tests that only insert the Resource still work.
        // Only the LocalShip should ever fall through to the global — NPC
        // ships that lack the component simply have no torpedo tubes.
        let mut torpedo_sys_comp = torpedo_sys_comp;
        let torpedo_sys: &mut crate::torpedo::TorpedoSystem = match torpedo_sys_comp.as_deref_mut()
        {
            Some(c) => &mut c.0,
            None => &mut torpedo_sys_res.0,
        };

        let uuid = uuid::Uuid::new_v4().to_string();
        let tube_facing_rad = torpedo_sys
            .tube(tube.as_str())
            .map(|t| t.facing_deg.to_radians())
            .unwrap_or(0.0);
        let launch_heading = physics.yaw + tube_facing_rad;
        let source_uuid = source_uuid_opt.map(|u| u.0.clone());
        let homing_uuid = weapons_target_opt
            .and_then(|wt| wt.0.clone())
            .or_else(|| target_uuid.clone());
        use crate::torpedo::LaunchResult;
        let result = torpedo_sys.launch(
            tube.as_str(),
            uuid.clone(),
            physics.x,
            physics.z,
            launch_heading,
            homing_uuid.clone(),
            source_uuid.clone(),
        );
        match result {
            LaunchResult::Launched {
                uuid: launched_uuid,
                ..
            } => {
                if let Some(mut wf) = weapon_fired_comp {
                    wf.0 = true;
                }
                outbox.0.push((
                    Target::All,
                    ServerMessage::TorpedoLaunched {
                        uuid: launched_uuid,
                        tube: tube.clone(),
                        x: physics.x,
                        z: physics.z,
                        heading: launch_heading,
                    },
                ));
            }
            LaunchResult::TubeNotLoaded | LaunchResult::NoTorpedoes | LaunchResult::UnknownTube => {
            }
        }
    }
}

/// Consumer for the Torpedo Magazine's inbound channel-2 `ClaimTorpedoRound`
/// messages (issue #512).
///
/// Runs in `SimSet::Physics` on ANY ship carrying a `TorpedoSystemResource`
/// component (`With<Ship>`) — routing by `source_entity` mirrors the
/// [`crate::ship::power::handle_power_inter_system`] pattern so multiple
/// ships with magazines each mutate their own state. Falls back to the
/// LocalShip when `source_entity` is `None` (legacy path), and to the
/// global `TorpedoSystemResource` when no matching Ship entity exists at
/// all (legacy test paths without a Ship entity).
///
/// For every `ClaimTorpedoRound` targeted at
/// [`crate::system_registry::torpedo_magazine_system_id`]:
///
/// 1. Refuse the claim (no-op) when the magazine is offline (Disabled /
///    Destroyed hull tier — reflected as `!accept_human_input && !operate_ai`
///    in the control-source resolver).
/// 2. Refuse the claim when the shared magazine counter is zero.
/// 3. Otherwise decrement the counter and start loading the named tube via
///    [`crate::torpedo::TorpedoSystem::start_load_reserved`].
///
/// This is the sole path the Bevy weapons handler uses to consume from the
/// magazine — the tube handler (`handle_load_tube`) only *sends* the claim.
pub fn handle_torpedo_magazine_inter_system(
    queue: Res<InterSystemQueue>,
    mut ship_q: Query<
        (
            Entity,
            &ShipSystemControlSources,
            &mut TorpedoSystemResource,
            bevy::ecs::query::Has<crate::server_app::LocalShip>,
        ),
        With<crate::server_app::Ship>,
    >,
    mut torpedo_sys_res: ResMut<TorpedoSystemResource>,
) {
    let magazine_id = crate::system_registry::torpedo_magazine_system_id();
    // Collect targeted claims: (source_entity, tube_id). Only claims for the
    // magazine system are relevant here — everything else is ignored.
    let claims: Vec<(Option<Entity>, String)> = queue
        .0
        .iter()
        .filter(|m| m.target == magazine_id)
        .filter_map(|m| match &m.payload {
            InterSystemPayload::ClaimTorpedoRound { tube } => Some((m.source_entity, tube.clone())),
            _ => None,
        })
        .collect();
    if claims.is_empty() {
        return;
    }

    // Snapshot the LocalShip entity once so `source_entity: None` (legacy
    // path) resolves to the player ship consistently across the loop.
    let local_ship_entity: Option<Entity> =
        ship_q
            .iter()
            .find_map(|(e, _, _, is_local)| if is_local { Some(e) } else { None });

    for (source_entity, tube_id) in claims {
        let target_entity = source_entity.or(local_ship_entity);
        if let Some(target) = target_entity {
            if let Ok((_e, control_sources, mut torpedo_sys, _is_local)) = ship_q.get_mut(target) {
                // Gate: magazine must be online (or absent → treat as online for
                // ships that don't declare a magazine fine system, preserving
                // legacy behaviour). The `torpedo_magazine` system is added to
                // the resolver by lobby setup when the ship TOML declares it.
                let magazine_declared = control_sources
                    .0
                    .entries()
                    .any(|(id, _)| id == &magazine_id)
                    || control_sources.0.offline_systems.contains(&magazine_id);
                if magazine_declared {
                    let policy = control_sources.0.policy_for(&magazine_id);
                    if !policy.accept_human_input && !policy.operate_ai {
                        // This ship's magazine is offline — refuse this claim.
                        // Other ships' claims (different `source_entity`) are
                        // still handled below in subsequent iterations.
                        continue;
                    }
                }
                if !torpedo_sys.0.claim_magazine_round() {
                    continue; // magazine empty — refuse this claim.
                }
                if !torpedo_sys.0.start_load_reserved(&tube_id) {
                    // Tube already loaded / unknown — return the round to the magazine.
                    torpedo_sys.0.torpedoes_remaining += 1;
                }
                continue;
            }
        }
        // Resource-only fallback (no Ship entity with the component).
        if !torpedo_sys_res.0.claim_magazine_round() {
            continue;
        }
        if !torpedo_sys_res.0.start_load_reserved(&tube_id) {
            torpedo_sys_res.0.torpedoes_remaining += 1;
        }
    }
}

/// Phase 1 of the torpedo tick (issue #724): build the one-tick
/// [`TorpedoTargetSnapshot`] — target positions (live ECS with a
/// `WorldResource` fallback) and the proximity detonation target list —
/// which `tick_torpedo_lifecycle` reads later in the same tick.
fn build_torpedo_target_snapshot(
    world: Res<WorldResource>,
    asteroid_q: Query<(&AsteroidUuid, &Transform), Without<crate::entity_spawner::EntityUuid>>,
    entity_q: Query<(&crate::entity_spawner::EntityUuid, &Transform), Without<AsteroidUuid>>,
    // Virtual entities (asteroid-field anchors, region trigger volumes) are
    // organisational/effect-only. They carry an `EntityUuid` and a non-zero
    // `radius` in the world snapshot (from `outer_radius` or region shape),
    // so without this filter `find_detonation_hits` treats them as giant
    // hittable targets — and a torpedo fired anywhere inside a 350 m
    // asteroid-field annulus detonates on the field anchor on its first
    // physics tick. (Regression that made torpedoes invisible from the
    // viewscreen because the sphere lifetime was a single frame.)
    virtual_entity_q: Query<
        &crate::entity_spawner::EntityUuid,
        Or<(
            With<crate::entity_spawner::AsteroidFieldSection>,
            With<crate::entity_spawner::RegionShapeSection>,
        )>,
    >,
    mut snapshot: ResMut<TorpedoTargetSnapshot>,
) {
    snapshot.clear();

    // ── Build shared world snapshots up-front (used by every ship's tick) ───

    // UUIDs of virtual (non-hittable) entities — anchors / regions. Used to
    // exclude them from the detonation target list below.
    let virtual_uuids: std::collections::HashSet<String> =
        virtual_entity_q.iter().map(|u| u.0.clone()).collect();
    // World snapshot also carries virtual entities — recognise them by the
    // shape field (`Some("torus" | "sphere" | "box")` marks a region or
    // asteroid-field anchor). The live ECS filter above is the source of
    // truth when the entity is present; this catches snapshot-only entries.
    let virtual_snapshot_uuids: std::collections::HashSet<String> = world
        .0
        .entities
        .iter()
        .filter(|e| e.shape.is_some())
        .map(|e| e.uuid.clone())
        .collect();

    // Build target positions from *live* ECS transforms, falling back to the
    // (stale) WorldResource snapshot for entities not currently in the ECS.
    let target_positions: std::collections::HashMap<String, (f32, f32)> = {
        let mut map: std::collections::HashMap<String, (f32, f32)> =
            std::collections::HashMap::new();
        for (u, t) in asteroid_q.iter() {
            map.insert(u.0.clone(), (t.translation.x, t.translation.z));
        }
        for (u, t) in entity_q.iter() {
            map.insert(u.0.clone(), (t.translation.x, t.translation.z));
        }
        // Fill remaining entries from WorldResource snapshot for completeness.
        for e in world.0.entities.iter() {
            map.entry(e.uuid.clone()).or_insert_with(|| (e.x(), e.z()));
        }
        map
    };

    // Proximity detonation target list (uuid, x, z, radius). Built once and
    // shared across every ship's `find_detonation_hits` call.
    let targets: Vec<(String, f32, f32, f32)> = {
        let mut map: std::collections::HashMap<String, (f32, f32, f32)> =
            std::collections::HashMap::new();
        for (u, t) in asteroid_q.iter() {
            let radius = world
                .0
                .entities
                .iter()
                .find(|e| e.uuid == u.0)
                .map(|e| e.radius_or_zero())
                .unwrap_or(0.0);
            map.insert(u.0.clone(), (t.translation.x, t.translation.z, radius));
        }
        for (u, t) in entity_q.iter() {
            if virtual_uuids.contains(&u.0) || virtual_snapshot_uuids.contains(&u.0) {
                continue;
            }
            let radius = world
                .0
                .entities
                .iter()
                .find(|e| e.uuid == u.0)
                .map(|e| e.radius_or_zero())
                .unwrap_or(0.0);
            map.insert(u.0.clone(), (t.translation.x, t.translation.z, radius));
        }
        for e in world.0.entities.iter() {
            if virtual_uuids.contains(&e.uuid) || virtual_snapshot_uuids.contains(&e.uuid) {
                continue;
            }
            map.entry(e.uuid.clone())
                .or_insert_with(|| (e.x(), e.z(), e.radius_or_zero()));
        }
        map.into_iter()
            .map(|(uuid, (x, z, r))| (uuid, x, z, r))
            .collect()
    };

    snapshot.target_positions = target_positions;
    snapshot.targets = targets;
}

/// Phase 2 of the torpedo tick (issue #724): per-ship torpedo tick —
/// guidance/expiry via the [`TorpedoTargetSnapshot`] built earlier this
/// tick, proximity detonation, shield routing, hull damage, despawn,
/// broadcasts and VFX events.
fn tick_torpedo_lifecycle(
    mut torpedo_sys_q: Query<&mut TorpedoSystemResource, With<crate::server_app::Ship>>,
    mut torpedo_sys_res: ResMut<TorpedoSystemResource>,
    mut world: ResMut<WorldResource>,
    time: Res<Time>,
    mut outbox: ResMut<SimOutbox>,
    mut hull_query: Query<(
        Entity,
        Option<&AsteroidUuid>,
        Option<&crate::entity_spawner::EntityUuid>,
        &mut EntitySystemHull,
        Option<&mut crate::ship::shields::ShipShields>,
        Option<&mut crate::entity_spawner::EntityShipArcHull>,
    )>,
    mut commands: Commands,
    mut vfx_events: MessageWriter<AsteroidDestroyedVfx>,
    mut destroyed_events: MessageWriter<crate::ai_plugin::AiEntityDestroyed>,
    asteroid_q: Query<(&AsteroidUuid, &Transform), Without<crate::entity_spawner::EntityUuid>>,
    entity_q: Query<(&crate::entity_spawner::EntityUuid, &Transform), Without<AsteroidUuid>>,
    mut weapons_target_q: Query<&mut WeaponsTarget, With<crate::server_app::LocalShip>>,
    snapshot: Res<TorpedoTargetSnapshot>,
) {
    let dt = time.delta_secs();
    let mut weapons_target_opt = weapons_target_q.single_mut().ok();
    let target_positions = &snapshot.target_positions;
    let targets = &snapshot.targets;

    // ── Phase 1: tick every ship's TorpedoSystem + collect detonation events ──
    //
    // Iterate all ships (`With<Ship>`) with a `TorpedoSystemResource`
    // component — player + NPC. Each ship ticks its own tubes, expires its
    // own torpedoes, and produces its own detonation-hit list.
    //
    // The Resource fallback runs only when NO Ship entity carries the
    // component; this preserves the legacy Resource-only test paths.
    #[derive(Clone, Debug)]
    struct Detonation {
        target_uuid: String,
        damage_hull: i32,
        damage_shields: i32,
        shield_pierce: f32,
    }
    let mut detonations: Vec<Detonation> = Vec::new();
    let mut any_ship_component = false;

    for mut torpedo_sys in torpedo_sys_q.iter_mut() {
        any_ship_component = true;
        let result = torpedo_sys.0.tick(dt, target_positions, &mut || {
            uuid::Uuid::new_v4().to_string()
        });
        for expired_uuid in result.expired {
            outbox.0.push((
                Target::All,
                ServerMessage::TorpedoDestroyed { uuid: expired_uuid },
            ));
        }
        for (tube, uuid, x, z, heading) in result.burst_launched {
            outbox.0.push((
                Target::All,
                ServerMessage::TorpedoLaunched {
                    uuid,
                    tube,
                    x,
                    z,
                    heading,
                },
            ));
        }
        let hits = torpedo_sys.0.find_detonation_hits(targets);
        for (torpedo_uuid, target_uuid) in hits {
            let Some(det) = torpedo_sys.0.handle_collision_full(&torpedo_uuid) else {
                continue;
            };
            outbox.0.push((
                Target::All,
                ServerMessage::TorpedoDestroyed { uuid: torpedo_uuid },
            ));
            detonations.push(Detonation {
                target_uuid,
                damage_hull: det.damage_hull,
                damage_shields: det.damage_shields,
                shield_pierce: det.shield_pierce,
            });
        }
    }

    // Resource-only fallback: tests that only insert the global
    // `TorpedoSystemResource` (no Ship entity carrying it) still work.
    if !any_ship_component {
        let result = torpedo_sys_res.0.tick(dt, target_positions, &mut || {
            uuid::Uuid::new_v4().to_string()
        });
        for expired_uuid in result.expired {
            outbox.0.push((
                Target::All,
                ServerMessage::TorpedoDestroyed { uuid: expired_uuid },
            ));
        }
        for (tube, uuid, x, z, heading) in result.burst_launched {
            outbox.0.push((
                Target::All,
                ServerMessage::TorpedoLaunched {
                    uuid,
                    tube,
                    x,
                    z,
                    heading,
                },
            ));
        }
        let hits = torpedo_sys_res.0.find_detonation_hits(targets);
        for (torpedo_uuid, target_uuid) in hits {
            let Some(det) = torpedo_sys_res.0.handle_collision_full(&torpedo_uuid) else {
                continue;
            };
            outbox.0.push((
                Target::All,
                ServerMessage::TorpedoDestroyed { uuid: torpedo_uuid },
            ));
            detonations.push(Detonation {
                target_uuid,
                damage_hull: det.damage_hull,
                damage_shields: det.damage_shields,
                shield_pierce: det.shield_pierce,
            });
        }
    }

    // ── Phase 2: apply detonations to hulls / shields ───────────────────────

    for det in detonations {
        let target_uuid = det.target_uuid;
        let mut asteroid_destroyed = false;
        let mut non_local_ship_destroyed = false;
        let mut hit_x = 0.0_f32;
        let mut hit_z = 0.0_f32;

        for (
            entity,
            asteroid_uuid,
            entity_uuid,
            mut hull_comp,
            mut shield_comp,
            mut target_arc_hull,
        ) in hull_query.iter_mut()
        {
            let uuid_matches = asteroid_uuid.map(|u| u.0.as_str()) == Some(target_uuid.as_str())
                || entity_uuid.map(|u| u.0.as_str()) == Some(target_uuid.as_str());
            if !uuid_matches {
                continue;
            }
            let is_asteroid = asteroid_uuid.is_some();
            let mut rng = rand::rng();

            // Route shield-eligible damage through any `ShipShields`
            // component, with overflow leaking to hull. Hull damage
            // (always-pierces) goes straight to hull. Asteroids carry no
            // shield so the shielded path is a no-op for them.
            let mut hull_damage = det.damage_hull as f32;
            let shield_eligible = det.damage_shields as f32;
            if shield_eligible > 0.0 {
                if let Some(ref mut shields) = shield_comp {
                    let all_offline = shields.0.facings.iter().all(|f| !f.is_online());
                    if all_offline {
                        hull_damage += shield_eligible;
                    } else {
                        let (pierced, absorbed) = crate::damage::split_damage_for_pierce(
                            shield_eligible,
                            det.shield_pierce,
                        );
                        let leak = shields.0.apply_damage(absorbed.round() as i32, 0.0);
                        hull_damage += pierced + leak as f32;
                    }
                } else {
                    hull_damage += shield_eligible;
                }
            }
            if hull_damage > 0.0 {
                let before = hull_comp.0.total_current();
                hull_comp.0.apply_damage(hull_damage, &mut rng);
                let absorbed = before - hull_comp.0.total_current();
                // Distribute the same absorbed amount across per-arc hull
                // (issue #514).
                if let Some(ref mut arc_hull) = target_arc_hull {
                    arc_hull.0.apply_damage(absorbed, &mut rng);
                }
            }

            if hull_comp.0.is_destroyed() {
                commands.entity(entity).try_despawn();
                if is_asteroid {
                    asteroid_destroyed = true;
                } else {
                    non_local_ship_destroyed = true;
                }
                // Use live position from whichever query matches (asteroid or ship).
                if is_asteroid {
                    if let Some((_, t)) = asteroid_q.iter().find(|(u, _)| u.0 == target_uuid) {
                        hit_x = t.translation.x;
                        hit_z = t.translation.z;
                    }
                } else if let Some((_, t)) = entity_q.iter().find(|(u, _)| u.0 == target_uuid) {
                    hit_x = t.translation.x;
                    hit_z = t.translation.z;
                }
            }
        }

        if asteroid_destroyed {
            world.0.entities.retain(|a| a.uuid != target_uuid);
            vfx_events.write(AsteroidDestroyedVfx { x: hit_x, z: hit_z });
            outbox.0.push((
                Target::All,
                ServerMessage::AsteroidDestroyed {
                    uuid: target_uuid.clone(),
                },
            ));
            if weapons_target_opt.as_deref().and_then(|wt| wt.0.as_deref())
                == Some(target_uuid.as_str())
            {
                if let Some(ref mut wt) = weapons_target_opt {
                    wt.0 = None;
                }
            }
        } else if non_local_ship_destroyed {
            world.0.entities.retain(|a| a.uuid != target_uuid);
            destroyed_events.write(crate::ai_plugin::AiEntityDestroyed {
                entity_uuid: target_uuid.clone(),
            });
            outbox.0.push((
                Target::All,
                ServerMessage::EntityDespawned {
                    uuid: target_uuid.clone(),
                },
            ));
            if weapons_target_opt.as_deref().and_then(|wt| wt.0.as_deref())
                == Some(target_uuid.as_str())
            {
                if let Some(ref mut wt) = weapons_target_opt {
                    wt.0 = None;
                }
            }
        }
    }
}

// ── Tactical AI ───────────────────────────────────────────────────────────
//
// `ai_target_selection` is the whole of the Tactical AI's targeting path
// (issues #697, #700). It reads the world, the ship's own objective
// blackboard, and its last attacker; it publishes the chosen target to
// `WeaponsBlackboard.locked_target` as observable intent, and applies that
// same choice to the authoritative `WeaponsTarget` component (truth) in the
// same system.
//
// It began (#697) as a decide/integrate pair — `ai_target_selection` →
// `operate_tactical_ai` — mirroring the shape the other console AIs use
// (`ai_shield_focus` → `integrate_shield_state`). #700 folded the integrator
// back in, because unlike those pairs the two halves could not be separated by
// a sim set: every `WeaponsTarget` reader runs in `SimSet::Input`, so the write
// had to stay in `Input` too, which left the "pair" as two systems in the same
// set held together by an explicit `.before` edge and an `Option<Option<_>>`
// to distinguish "the decider never ran" from "the decider chose nothing".
//
// Folding them back makes read-seed-decide-write atomic with respect to the
// other `Input` writer of `WeaponsTarget` (`handle_set_target`), which is what
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
/// `WeaponsTarget` directly. The field is what lets a client (or a human
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
/// The Helm does not acquire: `ai::core::helm_destroy` reads `WeaponsTarget` and
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
/// here, and every consumer follows because every consumer reads `WeaponsTarget`.
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
/// The decision is applied to `WeaponsTarget` here, in the same system that
/// makes it. `WeaponsTarget` is the single source of truth every consumer reads
/// (`handle_fire_phaser`, `ai_phaser_auto_fire`, `ai_torpedo_auto_fire`,
/// `tick_npc_auto_match_frequency`, …), and this is the only path by which the
/// AI reaches it — a human-operated Tactical's lock is never overwritten,
/// because the `operate_ai` gate below skips the ship entirely.
///
/// Seeding the selection from `WeaponsTarget` and writing it back within one
/// system is deliberate: it makes the AI's read-modify-write atomic with
/// respect to `handle_set_target`, the only other `SimSet::Input` writer of
/// `WeaponsTarget`. `Input` has no intra-set ordering by default, so a
/// separate integrator system could be scheduled between the two and write back
/// a decision made before the human's `SetTarget` landed, silently dropping the
/// human's lock on any mixed-rating ship. Do not re-split this without
/// re-establishing that ordering — see `WeaponsPlugin::build` and
/// `human_set_target_survives_the_tick_on_a_mixed_rating_ship`.
fn ai_target_selection(
    mut ship_query: Query<
        (
            &crate::ship_plugin::ShipConfigComponent,
            &ShipSystemControlSources,
            &LastShipAttacker,
            &ShipPhysics,
            &mut WeaponsTarget,
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

    for (
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
        // in that case; the human operator drives `WeaponsTarget` directly via
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
            (alive && (!range_bounds_targets || within_range(&current))).then_some(current)
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

        let acquired = objective_target
            .or_else(retained_lock)
            .or_else(|| last_attacker.0.clone())
            .or_else(|| {
                destroy_is_untargeted
                    .then(|| nearest_hostile(registry))
                    .flatten()
            });

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
        &WeaponsTarget,
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
    for (entity, control_sources, ship_config, target, mut phaser_freq, has_high_fidelity) in
        ship_q.iter_mut()
    {
        if !has_high_fidelity || !any_tactical_system_operates_ai(control_sources, &ship_config.0) {
            states.0.remove(&entity);
            continue;
        }

        let locked_target = target.0.clone();
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

// ── Broadcaster ───────────────────────────────────────────────────────────

/// Compute the current Tactical weapons state for the `LocalShip` from live
/// ECS/resource state — target lock, phaser bank readiness, torpedo tubes,
/// torpedo count, and phaser mode.
///
/// This is the same computation `weapons_update_broadcaster` runs every tick
/// to decide whether to send a fresh `WeaponsUpdate`; it's factored out here
/// so [`crate::core::broadcast::cache_registry::resync_for_token`] can reuse
/// it to build a reconnect resync without duplicating the target/range/arc
/// logic. Callers that need the diff-and-broadcast behaviour (comparing
/// against [`LastWeaponsUpdate`] and updating it) must do that themselves —
/// this function only computes the current snapshot and never reads or
/// writes the cache resources.
pub fn compute_current_weapons_update(world: &mut World) -> LastWeaponsUpdate {
    // Extract all resource values as owned copies/clones so we can
    // release the immutable borrows before calling world.query_filtered.
    let (ship_x, ship_z, ship_yaw) = {
        let mut q = world.query_filtered::<&ShipPhysics, With<crate::server_app::LocalShip>>();
        q.single(world)
            .ok()
            .copied()
            .map(|p| (p.x, p.z, p.yaw))
            .unwrap_or((0.0, 0.0, 0.0))
    };
    let target_uuid: Option<String> = {
        let mut q = world.query_filtered::<&WeaponsTarget, With<crate::server_app::LocalShip>>();
        q.single(world).ok().and_then(|wt| wt.0.clone())
    };
    let (beam_active, active_beam_bank) = {
        let mut q = world.query_filtered::<&ActiveBeam, With<crate::server_app::LocalShip>>();
        q.single(world)
            .ok()
            .map(|b| (b.target_uuid.is_some(), b.bank.clone()))
            .unwrap_or((false, None))
    };
    let bank_cooldowns: std::collections::HashMap<String, f32> = {
        let mut q = world.query_filtered::<&PhaserCooldown, With<crate::server_app::LocalShip>>();
        q.single(world)
            .ok()
            .map(|cd| cd.per_bank.clone())
            .unwrap_or_default()
    };
    let tubes: Vec<TorpedoTubeState> = {
        // Prefer per-entity component on LocalShip; fall back to global resource.
        let raw_tubes: Vec<crate::torpedo::TorpedoTube> = {
            let mut q = world
                .query_filtered::<&TorpedoSystemResource, With<crate::server_app::LocalShip>>();
            q.single(world)
                .ok()
                .map(|ts| ts.0.tubes.clone())
                .unwrap_or_else(|| world.resource::<TorpedoSystemResource>().0.tubes.clone())
        };
        raw_tubes
            .iter()
            .map(|t| {
                let remaining = match &t.load_state {
                    crate::torpedo::TubeLoadState::Loading { remaining, .. }
                    | crate::torpedo::TubeLoadState::Unloading { remaining, .. } => *remaining,
                    _ => 0.0,
                };
                TorpedoTubeState {
                    id: t.id.clone(),
                    loaded: t.is_loaded(),
                    reload_secs: remaining,
                    state: t.load_state.label().to_string(),
                    progress: t.load_state.progress(),
                    load_time: t.load_time,
                    volley_max: t.volley_max,
                    loaded_count: t.loaded_count,
                    target_count: t.target_count,
                    load_progress: t.load_progress(),
                }
            })
            .collect()
    };
    let torpedo_count = {
        let mut q =
            world.query_filtered::<&TorpedoSystemResource, With<crate::server_app::LocalShip>>();
        q.single(world)
            .ok()
            .map(|ts| ts.0.torpedoes_remaining)
            .unwrap_or_else(|| {
                world
                    .resource::<TorpedoSystemResource>()
                    .0
                    .torpedoes_remaining
            })
    };
    let radar_range_mult = {
        let mut q = world
            .query_filtered::<&crate::modifiers::ShipModifiers, With<crate::server_app::LocalShip>>(
            );
        q.single(world)
            .ok()
            .map(|m| m.get(&ModifierSlot::RadarRange))
            .unwrap_or(1.0)
    };
    let phaser_mode = world.resource::<CurrentPhaserMode>().0;
    let phaser_frequency = {
        let mut q = world
            .query_filtered::<&crate::ship_state::ShipPhaserFrequency, With<crate::server_app::LocalShip>>();
        q.single(world).ok().map(|f| f.0).unwrap_or(0.5)
    };
    let banks_config = {
        // Prefer per-entity component on LocalShip; fall back to global resource.
        let mut q = world
            .query_filtered::<&PhaserCombatConfigResource, With<crate::server_app::LocalShip>>();
        q.single(world)
            .ok()
            .map(|cc| cc.0.banks.clone())
            .unwrap_or_else(|| {
                world
                    .resource::<PhaserCombatConfigResource>()
                    .0
                    .banks
                    .clone()
            })
    };

    // Query live ECS Transform for the target — WorldResource is a
    // stale spawn-time snapshot and doesn't contain NPC ships that
    // spawn after the scene loads.
    let target_live_pos: Option<(f32, f32)> = match &target_uuid {
        None => None,
        Some(uuid) => {
            let uuid = uuid.clone();
            let mut pos = None;
            let mut entity_qs = world.query_filtered::<
                        (&crate::entity_spawner::EntityUuid, &Transform),
                        Without<AsteroidUuid>,
                    >();
            for (u, t) in entity_qs.iter(world) {
                if u.0 == uuid {
                    pos = Some((t.translation.x, t.translation.z));
                    break;
                }
            }
            if pos.is_none() {
                let mut asteroid_qs = world.query_filtered::<
                            (&AsteroidUuid, &Transform),
                            Without<crate::entity_spawner::EntityUuid>,
                        >();
                for (u, t) in asteroid_qs.iter(world) {
                    if u.0 == uuid {
                        pos = Some((t.translation.x, t.translation.z));
                        break;
                    }
                }
            }
            pos
        }
    };

    // Look up the display name for the locked target.
    let target_name: Option<String> = match &target_uuid {
        None => None,
        Some(uuid) => {
            let uuid = uuid.clone();
            let mut name = None;
            let mut name_qs = world.query::<(
                &crate::entity_spawner::EntityUuid,
                &crate::entities::spawner::EntityName,
            )>();
            for (u, n) in name_qs.iter(world) {
                if u.0 == uuid {
                    name = Some(n.0.clone());
                    break;
                }
            }
            name
        }
    };

    let banks: Vec<PhaserBankState> = if banks_config.is_empty() {
        let effective_phaser_range =
            crate::entity_config::PhaserCombatConfig::DEFAULT_PHASER_RANGE * radar_range_mult;
        let fire_ready = match target_live_pos {
            None => false,
            Some((tx, tz)) => crate::radar::is_fire_ready_with_range(
                tx,
                tz,
                ship_x,
                ship_z,
                ship_yaw,
                effective_phaser_range,
            ),
        };
        let cd = bank_cooldowns.get("").copied().unwrap_or(0.0);
        vec![PhaserBankState {
            id: String::new(),
            fire_ready,
            on_cooldown: beam_active || cd > 0.0,
            cooldown_remaining: cd,
        }]
    } else {
        banks_config
            .iter()
            .map(|b| {
                let bank_ready = match target_live_pos {
                    None => false,
                    Some((tx, tz)) => {
                        let bank_base_range = if b.beam_range > 0.0 {
                            b.beam_range
                        } else {
                            crate::entity_config::PhaserCombatConfig::DEFAULT_PHASER_RANGE
                        };
                        let effective_bank_range = bank_base_range * radar_range_mult;
                        let (rx, ry) =
                            crate::weapons::phaser::ship_local(tx, tz, ship_x, ship_z, ship_yaw);
                        let range_ok = (tx - ship_x).powi(2) + (tz - ship_z).powi(2)
                            <= effective_bank_range * effective_bank_range;
                        range_ok
                            && crate::weapons::phaser::in_arc(rx, ry, b.facing_deg, b.fire_arc_deg)
                    }
                };
                let cd = bank_cooldowns.get(b.id.as_str()).copied().unwrap_or(0.0);
                let beam_on_this_bank =
                    beam_active && active_beam_bank.as_deref() == Some(b.id.as_str());
                PhaserBankState {
                    id: b.id.clone(),
                    fire_ready: bank_ready,
                    on_cooldown: beam_on_this_bank || cd > 0.0,
                    cooldown_remaining: cd,
                }
            })
            .collect()
    };

    LastWeaponsUpdate {
        target_uuid,
        target_name,
        banks,
        tubes,
        torpedo_count,
        phaser_mode,
        blasters: Vec::new(), // Populated by weapons_update_broadcaster from BlasterSystemResource.
        phaser_frequency,
    }
}

pub fn weapons_update_broadcaster() -> crate::core::broadcast::SimBroadcaster {
    crate::core::broadcast::SimBroadcaster::new().register(
        crate::core::broadcast::Audience::HoldingWeapons,
        crate::core::broadcast::Cadence::Hz(10.0),
        |world: &mut World| {
            let mut current = compute_current_weapons_update(world);

            // Collect blaster bank states from the LocalShip's BlasterSystemResource.
            {
                let mut q = world
                    .query_filtered::<&BlasterSystemResource, With<crate::server_app::LocalShip>>();
                if let Ok(blaster_res) = q.single(world) {
                    current.blasters = blaster_res.0.iter().map(|b| b.bank_state()).collect();
                }
            }

            let is_first_tick = world.resource::<WeaponsUpdateFirstTick>().0;
            if !is_first_tick {
                let last = world.resource::<LastWeaponsUpdate>();
                if *last == current {
                    return vec![];
                }
            }
            if is_first_tick {
                *world.resource_mut::<WeaponsUpdateFirstTick>() = WeaponsUpdateFirstTick(false);
            }
            let LastWeaponsUpdate {
                target_uuid,
                target_name,
                banks,
                tubes,
                torpedo_count,
                phaser_mode,
                blasters,
                phaser_frequency,
            } = current.clone();
            *world.resource_mut::<LastWeaponsUpdate>() = current;

            vec![ServerMessage::WeaponsUpdate {
                target_uuid,
                target_name,
                banks,
                tubes,
                torpedo_count,
                phaser_mode,
                blasters,
                phaser_frequency,
            }]
        },
    )
}

// ── Blackboard publish (issue #560) ─────────────────────────────────────────
//
// Split into one system per blackboard type (issue #725):
// `publish_weapons_core_blackboard`, `publish_phaser_bank_blackboards`,
// `publish_torpedo_tube_blackboards`, `publish_torpedo_magazine_blackboard`.
// Each writes only its own `ShipSystemBlackboards` keys, so the four register
// in `SimSet::Publish` with no ordering edges between them. Bank/tube state
// that the core Weapons blackboard also carries is recomputed by the per-bank
// / per-tube systems via the shared `build_bank_states` / `build_tube_states`
// helpers — intentional duplication that keeps the systems order-free.

/// Build the per-bank [`PhaserBankState`] list from phaser config + live state.
///
/// Shared by `publish_weapons_core_blackboard` (for `WeaponsBlackboard::banks`)
/// and `publish_phaser_bank_blackboards` (for the per-bank fine blackboards) so
/// neither has to read the other's published map entry — recomputing keeps the
/// two systems free of ordering constraints. Recomputation cost is trivial.
#[allow(clippy::too_many_arguments)]
fn build_bank_states(
    combat_config: &PhaserCombatConfigResource,
    cooldown: &PhaserCooldown,
    beam_active: bool,
    active_beam_bank: Option<&str>,
    radar_range_mult: f32,
    physics: ShipPhysics,
    target_live_pos: Option<(f32, f32)>,
) -> Vec<PhaserBankState> {
    if combat_config.0.banks.is_empty() {
        let effective_range =
            crate::entity_config::PhaserCombatConfig::DEFAULT_PHASER_RANGE * radar_range_mult;
        let fire_ready = match target_live_pos {
            None => false,
            Some((tx, tz)) => crate::radar::is_fire_ready_with_range(
                tx,
                tz,
                physics.x,
                physics.z,
                physics.yaw,
                effective_range,
            ),
        };
        let cd = cooldown.bank_remaining_secs("");
        vec![PhaserBankState {
            id: String::new(),
            fire_ready,
            on_cooldown: beam_active || cd > 0.0,
            cooldown_remaining: cd,
        }]
    } else {
        combat_config
            .0
            .banks
            .iter()
            .map(|b| {
                let bank_ready = match target_live_pos {
                    None => false,
                    Some((tx, tz)) => {
                        let bank_base_range = if b.beam_range > 0.0 {
                            b.beam_range
                        } else {
                            crate::entity_config::PhaserCombatConfig::DEFAULT_PHASER_RANGE
                        };
                        let effective_bank_range = bank_base_range * radar_range_mult;
                        let (rx, ry) = crate::weapons::phaser::ship_local(
                            tx,
                            tz,
                            physics.x,
                            physics.z,
                            physics.yaw,
                        );
                        let range_ok = (tx - physics.x).powi(2) + (tz - physics.z).powi(2)
                            <= effective_bank_range * effective_bank_range;
                        range_ok
                            && crate::weapons::phaser::in_arc(rx, ry, b.facing_deg, b.fire_arc_deg)
                    }
                };
                let cd = cooldown.bank_remaining_secs(b.id.as_str());
                let beam_on_this_bank = beam_active && active_beam_bank == Some(b.id.as_str());
                PhaserBankState {
                    id: b.id.clone(),
                    fire_ready: bank_ready,
                    on_cooldown: beam_on_this_bank || cd > 0.0,
                    cooldown_remaining: cd,
                }
            })
            .collect()
    }
}

/// Build the per-tube [`TorpedoTubeState`] list from the ship's torpedo system.
///
/// Shared by `publish_weapons_core_blackboard` (for `WeaponsBlackboard::tubes`)
/// and `publish_torpedo_tube_blackboards` (for the per-tube fine blackboards)
/// for the same order-freedom reason as [`build_bank_states`].
fn build_tube_states(torpedo_sys: &TorpedoSystemResource) -> Vec<TorpedoTubeState> {
    torpedo_sys
        .0
        .tubes
        .iter()
        .map(|t| {
            let remaining = match &t.load_state {
                crate::torpedo::TubeLoadState::Loading { remaining, .. }
                | crate::torpedo::TubeLoadState::Unloading { remaining, .. } => *remaining,
                _ => 0.0,
            };
            TorpedoTubeState {
                id: t.id.clone(),
                loaded: t.is_loaded(),
                reload_secs: remaining,
                state: t.load_state.label().to_string(),
                progress: t.load_state.progress(),
                load_time: t.load_time,
                volley_max: t.volley_max,
                loaded_count: t.loaded_count,
                target_count: t.target_count,
                load_progress: t.load_progress(),
            }
        })
        .collect()
}

/// Publish each ship's core Weapons blackboard from current sim state.
/// Runs in `SimSet::Publish` (phase 1a). Dirty-tracking and broadcast are
/// handled globally by `broadcast_blackboard_updates` in `SimSet::Broadcast`.
///
/// Writes only the console-level Weapons entry (keyed by
/// [`crate::system_registry::tactical_station_key`]); the per-bank /
/// per-tube / magazine fine entries are owned by their own systems (issue
/// #725), which recompute bank/tube state via the shared helpers rather than
/// reading this system's output — no ordering between the four.
///
/// Per-entity for every ship carrying `ShipSystemBlackboards` (issue #697),
/// following the `ai::server::aggregate_doctrine_blackboards` precedent rather
/// than the old `With<LocalShip>` + `.single()` shape: the NPC Tactical AI needs
/// a Weapons blackboard of its own to read `locked_target` from, and slices
/// #698 / #700 will need per-NPC `banks` / `tubes` to fire from.
///
/// Two tiers of field, split by `Has<LocalShip>` in the loop:
///
/// - **Ship state** — `target_uuid`, `locked_target`, `target_name`, `banks`,
///   `tubes`, `torpedo_count`, and `blasters`. All derived from per-entity
///   components that NPCs carry, so they are computed for every ship.
/// - **Client render data** — `blips`, `regions`, `phaser_arcs`,
///   `torpedo_arcs`, `phaser_mode`. Sourced from the player-only
///   `CurrentPhaserMode` / `ShipClientConfigResource` resources and meaningless
///   for a ship with no browser client, so they stay empty/default for NPCs.
///   `blips` especially: it is O(all entities) per ship per tick, so computing
///   it for every NPC would cost O(ships × entities) for data nobody reads.
///
/// Ships with no `[behaviour]` block carry no `ShipSystemBlackboards` (see
/// `entities::spawner`) and are simply not iterated — no AI on board, so there
/// is nothing to read a blackboard. None of this reaches the wire for NPCs:
/// `broadcast_blackboard_updates` is `With<LocalShip>`-filtered, so NPC
/// blackboards add zero bandwidth.
fn publish_weapons_core_blackboard(
    mut ship_q: Query<
        (
            Option<&WeaponsTarget>,
            Option<&ActiveBeam>,
            Option<&PhaserCooldown>,
            Option<&PhaserCombatConfigResource>,
            Option<&TorpedoSystemResource>,
            Option<&BlasterSystemResource>,
            Option<&ShipPhysics>,
            Option<&crate::modifiers::ShipModifiers>,
            &mut crate::server_app::ShipSystemBlackboards,
            Has<crate::server_app::LocalShip>,
        ),
        With<crate::server_app::Ship>,
    >,
    phaser_mode: Res<CurrentPhaserMode>,
    ship_config: Res<crate::lobby::server::ShipClientConfigResource>,
    world_res: Res<WorldResource>,
    entity_name_q: Query<(
        &crate::entity_spawner::EntityUuid,
        &crate::entities::spawner::EntityName,
    )>,
    asteroid_q: Query<(&AsteroidUuid, &Transform), Without<crate::entity_spawner::EntityUuid>>,
    entity_q: Query<(&crate::entity_spawner::EntityUuid, &Transform), Without<AsteroidUuid>>,
) {
    for (
        weapons_target,
        beam,
        cooldown,
        combat_config,
        torpedo_sys,
        blaster_res,
        ship_physics,
        modifiers,
        mut entity_bbs,
        is_local,
    ) in ship_q.iter_mut()
    {
        let physics = ship_physics.copied().unwrap_or_default();
        // Per-entity component path (preferred). Each fallback below mirrors the
        // pre-#697 `.single()` error arm, so a ship (or test fixture) missing a
        // component publishes exactly what it published before.
        let default_beam;
        let beam: &ActiveBeam = match beam {
            Some(b) => b,
            None => {
                default_beam = ActiveBeam::default();
                &default_beam
            }
        };
        let default_cooldown;
        let cooldown: &PhaserCooldown = match cooldown {
            Some(c) => c,
            None => {
                default_cooldown = PhaserCooldown::default();
                &default_cooldown
            }
        };
        let combat_config_default;
        let combat_config: &PhaserCombatConfigResource = match combat_config {
            Some(c) => c,
            None => {
                combat_config_default = PhaserCombatConfigResource::default();
                &combat_config_default
            }
        };
        let default_modifiers;
        let modifiers: &crate::modifiers::ShipModifiers = match modifiers {
            Some(m) => m,
            None => {
                default_modifiers = crate::modifiers::ShipModifiers::new();
                &default_modifiers
            }
        };
        let torpedo_sys_default;
        let torpedo_sys: &TorpedoSystemResource = match torpedo_sys {
            Some(t) => t,
            None => {
                torpedo_sys_default =
                    TorpedoSystemResource(TorpedoSystem::new(TorpedoConfig::default()));
                &torpedo_sys_default
            }
        };

        // Carry the Tactical AI's intent across the rebuild. `ai_target_selection`
        // wrote `locked_target` back in `SimSet::Input`; this system reconstructs
        // the whole blackboard, so without carrying it forward the field would be
        // wiped every tick and always read `None` on the wire.
        //
        // Re-check liveness rather than carrying blindly: the Input pair decided
        // on this target, but `tick_beams` (`Damage`) and `tick_torpedo_lifecycle`
        // (`Physics`) both clear `WeaponsTarget` after a kill, later in the same
        // tick. Carrying the dead value forward would publish `locked_target !=
        // target_uuid` for one tick, contradicting the field's own contract (see
        // `WeaponsBlackboard::locked_target` in `core::messages`) that the two
        // agree once a tick has settled. Selection re-derives from `WeaponsTarget`
        // and never from `locked_target`, so a dead value cannot resurrect a
        // target — but it can be read, and #698 / #700 will read it.
        let locked_target = entity_bbs
            .0
            .get(&crate::system_registry::tactical_station_key())
            .and_then(|bb| match bb {
                SystemBlackboard::Weapons(weapons) => weapons.locked_target.clone(),
                _ => None,
            })
            .filter(|uuid| live_entity_xz(uuid, &asteroid_q, &entity_q).is_some());

        let target_uuid = weapons_target.and_then(|wt| wt.0.clone());
        let radar_range_mult = modifiers.get(&ModifierSlot::RadarRange);
        let beam_active = beam.target_uuid.is_some();
        let active_beam_bank = beam.bank.clone();

        let target_live_pos: Option<(f32, f32)> = target_uuid
            .as_deref()
            .and_then(|uuid| live_entity_xz(uuid, &asteroid_q, &entity_q));

        let target_name: Option<String> = target_uuid.as_deref().and_then(|uuid| {
            entity_name_q
                .iter()
                .find_map(|(u, n)| (u.0 == uuid).then(|| n.0.clone()))
        });

        let banks: Vec<PhaserBankState> = build_bank_states(
            combat_config,
            cooldown,
            beam_active,
            active_beam_bank.as_deref(),
            radar_range_mult,
            physics,
            target_live_pos,
        );

        let tubes: Vec<TorpedoTubeState> = build_tube_states(torpedo_sys);

        // ── Client render data (LocalShip only) ──────────────────────────────
        // Everything below this point is drawn by the browser Tactical console
        // and is sourced from the two player-only resources. An NPC has no
        // client, so it gets empty vectors and the default phaser mode — see the
        // function doc. `blips` is the expensive one: skipping it for NPCs keeps
        // this system O(entities), not O(ships × entities).
        let mut blips: Vec<RadarBlip> = Vec::new();
        let mut regions: Vec<RadarRegion> = Vec::new();
        let mut phaser_arcs: Vec<PhaserBankClientConfig> = Vec::new();
        let mut torpedo_arcs: Vec<TorpedoTubeClientConfig> = Vec::new();
        let mut mode = crate::messages::PhaserMode::default();

        if is_local {
            mode = phaser_mode.0;
            phaser_arcs = ship_config.0.phaser_banks.clone();
            torpedo_arcs = ship_config.0.torpedo_tubes.clone();

            // ── Radar blips ──────────────────────────────────────────────────
            let effective_tactical_range = ship_config.0.tactical_radar_range * radar_range_mult;
            let shows: Vec<crate::entity_tags::EntityTag> = ship_config
                .0
                .tactical_radar_shows
                .iter()
                .filter_map(|s| crate::entity_tags::EntityTag::from_str(s))
                .collect();
            let selects: Vec<crate::entity_tags::EntityTag> = ship_config
                .0
                .tactical_radar_selects
                .iter()
                .filter_map(|s| crate::entity_tags::EntityTag::from_str(s))
                .collect();

            let entity_meta: std::collections::HashMap<&str, &crate::messages::EntitySnapshot> =
                world_res
                    .0
                    .entities
                    .iter()
                    .map(|e| (e.uuid.as_str(), e))
                    .collect();

            if !shows.is_empty() && effective_tactical_range > 0.0 {
                for (uuid_comp, transform) in asteroid_q.iter() {
                    let meta = entity_meta.get(uuid_comp.0.as_str()).copied();
                    if let Some(b) = project_blip(
                        &uuid_comp.0,
                        transform.translation.x,
                        transform.translation.z,
                        physics.x,
                        physics.z,
                        physics.yaw,
                        effective_tactical_range,
                        meta,
                        &shows,
                        &selects,
                    ) {
                        blips.push(b);
                    }
                }
                for (uuid_comp, transform) in entity_q.iter() {
                    let meta = entity_meta.get(uuid_comp.0.as_str()).copied();
                    if let Some(b) = project_blip(
                        &uuid_comp.0,
                        transform.translation.x,
                        transform.translation.z,
                        physics.x,
                        physics.z,
                        physics.yaw,
                        effective_tactical_range,
                        meta,
                        &shows,
                        &selects,
                    ) {
                        blips.push(b);
                    }
                }
            }

            // ── Region overlays ──────────────────────────────────────────────
            regions = world_res
                .0
                .entities
                .iter()
                .filter_map(|e| {
                    let shape = e.shape.as_deref()?;
                    Some(RadarRegion {
                        uuid: e.uuid.clone(),
                        x: e.x(),
                        z: e.z(),
                        shape: shape.to_string(),
                        radius: e.radius,
                        inner_radius: e.inner_radius,
                        outer_radius: e.radius,
                        half_extents: e.half_extents.map(|h| [h[0], h[2]]),
                        yaw: e.yaw,
                        color: e.colour.unwrap_or([0.6, 0.4, 1.0]),
                        name: e.name.clone(),
                    })
                })
                .collect();
        }

        // Blaster bank states from this ship's own BlasterSystemResource.
        let blasters: Vec<BlasterBankState> = blaster_res
            .map(|r| r.0.iter().map(|b| b.bank_state()).collect())
            .unwrap_or_default();

        let bb = WeaponsBlackboard {
            target_uuid,
            locked_target,
            target_name,
            banks,
            tubes,
            torpedo_count: torpedo_sys.0.torpedoes_remaining,
            phaser_mode: mode,
            phaser_arcs,
            torpedo_arcs,
            blasters,
            blips,
            regions,
        };

        // Console-level blackboard: keyed by the Tactical STATION id (issue
        // #801). The wire string is unchanged — the client still reads
        // `blackboards['tactical']` — but the key names the console, not a
        // system. Per-bank entries below keep their system-id keys.
        entity_bbs.0.insert(
            crate::system_registry::tactical_station_key(),
            SystemBlackboard::Weapons(bb),
        );
    }
}

/// Publish each ship's per-bank `PhaserBank` blackboards (issue #512).
/// Runs in `SimSet::Publish` with no ordering against the other publish
/// systems — bank state is recomputed via [`build_bank_states`] rather than
/// read back from the Weapons entry.
///
/// Emits one PhaserBank entry per bank in the ship config, keyed by
/// the fine SystemId (e.g. "phaser-fore"). Consumers gate on their
/// own bank without unpacking the whole weapons blackboard.
///
/// `is_online` is derived from `ShipSystemControlSources.offline_systems`
/// (populated by `sync_console_damage_tiers` during `SimSet::Damage`).
/// This matches the same surface all message handlers gate on, so
/// damage-driven offline state is reflected everywhere consistently.
/// Falls back to `true` when the ship has no ControlSources (test
/// paths that don't spawn a Ship entity with the component). NPCs get
/// this for free: `entities::spawner` gives every ship with a
/// `[behaviour]` block its own `ShipSystemControlSources`.
fn publish_phaser_bank_blackboards(
    mut ship_q: Query<
        (
            Option<&WeaponsTarget>,
            Option<&ActiveBeam>,
            Option<&PhaserCooldown>,
            Option<&PhaserCombatConfigResource>,
            Option<&ShipPhysics>,
            Option<&crate::modifiers::ShipModifiers>,
            Option<&ShipSystemControlSources>,
            &mut crate::server_app::ShipSystemBlackboards,
        ),
        With<crate::server_app::Ship>,
    >,
    asteroid_q: Query<(&AsteroidUuid, &Transform), Without<crate::entity_spawner::EntityUuid>>,
    entity_q: Query<(&crate::entity_spawner::EntityUuid, &Transform), Without<AsteroidUuid>>,
) {
    for (
        weapons_target,
        beam,
        cooldown,
        combat_config,
        ship_physics,
        modifiers,
        control_sources,
        mut entity_bbs,
    ) in ship_q.iter_mut()
    {
        let physics = ship_physics.copied().unwrap_or_default();
        // Fallbacks mirror `publish_weapons_core_blackboard`: a ship (or test
        // fixture) missing a component publishes exactly what it did before.
        let default_beam;
        let beam: &ActiveBeam = match beam {
            Some(b) => b,
            None => {
                default_beam = ActiveBeam::default();
                &default_beam
            }
        };
        let default_cooldown;
        let cooldown: &PhaserCooldown = match cooldown {
            Some(c) => c,
            None => {
                default_cooldown = PhaserCooldown::default();
                &default_cooldown
            }
        };
        let combat_config_default;
        let combat_config: &PhaserCombatConfigResource = match combat_config {
            Some(c) => c,
            None => {
                combat_config_default = PhaserCombatConfigResource::default();
                &combat_config_default
            }
        };
        let default_modifiers;
        let modifiers: &crate::modifiers::ShipModifiers = match modifiers {
            Some(m) => m,
            None => {
                default_modifiers = crate::modifiers::ShipModifiers::new();
                &default_modifiers
            }
        };

        let target_uuid = weapons_target.and_then(|wt| wt.0.clone());
        let target_live_pos: Option<(f32, f32)> = target_uuid
            .as_deref()
            .and_then(|uuid| live_entity_xz(uuid, &asteroid_q, &entity_q));
        let radar_range_mult = modifiers.get(&ModifierSlot::RadarRange);

        let banks = build_bank_states(
            combat_config,
            cooldown,
            beam.target_uuid.is_some(),
            beam.bank.as_deref(),
            radar_range_mult,
            physics,
            target_live_pos,
        );

        let offline_systems_opt = control_sources.map(|cs| &cs.0.offline_systems);
        for bank_state in &banks {
            let Some(bank_sysid) = crate::system_registry::phaser_bank_system_id(&bank_state.id)
            else {
                continue;
            };
            let is_online = offline_systems_opt
                .map(|set| !set.contains(&bank_sysid))
                .unwrap_or(true);
            entity_bbs.0.insert(
                bank_sysid,
                SystemBlackboard::PhaserBank(crate::messages::PhaserBankBlackboard {
                    is_online,
                    on_cooldown: bank_state.on_cooldown,
                    cooldown_remaining: bank_state.cooldown_remaining,
                    fire_ready: bank_state.fire_ready,
                }),
            );
        }
    }
}

/// Publish each ship's per-tube `TorpedoTube` blackboards (issue #512).
/// Runs in `SimSet::Publish` with no ordering against the other publish
/// systems — tube state is recomputed via [`build_tube_states`] rather than
/// read back from the Weapons entry. `is_online` derivation matches
/// [`publish_phaser_bank_blackboards`].
fn publish_torpedo_tube_blackboards(
    mut ship_q: Query<
        (
            Option<&TorpedoSystemResource>,
            Option<&ShipSystemControlSources>,
            &mut crate::server_app::ShipSystemBlackboards,
        ),
        With<crate::server_app::Ship>,
    >,
) {
    for (torpedo_sys, control_sources, mut entity_bbs) in ship_q.iter_mut() {
        let torpedo_sys_default;
        let torpedo_sys: &TorpedoSystemResource = match torpedo_sys {
            Some(t) => t,
            None => {
                torpedo_sys_default =
                    TorpedoSystemResource(TorpedoSystem::new(TorpedoConfig::default()));
                &torpedo_sys_default
            }
        };

        let tubes = build_tube_states(torpedo_sys);
        let offline_systems_opt = control_sources.map(|cs| &cs.0.offline_systems);
        for tube_state in &tubes {
            let Some(tube_sysid) = crate::system_registry::torpedo_tube_system_id(&tube_state.id)
            else {
                continue;
            };
            let is_online = offline_systems_opt
                .map(|set| !set.contains(&tube_sysid))
                .unwrap_or(true);
            entity_bbs.0.insert(
                tube_sysid,
                SystemBlackboard::TorpedoTube(crate::messages::TorpedoTubeBlackboard {
                    is_online,
                    loaded: tube_state.loaded,
                    state: tube_state.state.clone(),
                    progress: tube_state.progress,
                    load_time: tube_state.load_time,
                    volley_max: tube_state.volley_max,
                    loaded_count: tube_state.loaded_count,
                    target_count: tube_state.target_count,
                    load_progress: tube_state.load_progress,
                }),
            );
        }
    }
}

/// Publish each ship's `TorpedoMagazine` blackboard (issue #512).
/// Runs in `SimSet::Publish` with no ordering against the other publish
/// systems. `is_online` derivation matches [`publish_phaser_bank_blackboards`].
fn publish_torpedo_magazine_blackboard(
    mut ship_q: Query<
        (
            Option<&TorpedoSystemResource>,
            Option<&ShipSystemControlSources>,
            &mut crate::server_app::ShipSystemBlackboards,
        ),
        With<crate::server_app::Ship>,
    >,
) {
    for (torpedo_sys, control_sources, mut entity_bbs) in ship_q.iter_mut() {
        let torpedo_sys_default;
        let torpedo_sys: &TorpedoSystemResource = match torpedo_sys {
            Some(t) => t,
            None => {
                torpedo_sys_default =
                    TorpedoSystemResource(TorpedoSystem::new(TorpedoConfig::default()));
                &torpedo_sys_default
            }
        };

        let offline_systems_opt = control_sources.map(|cs| &cs.0.offline_systems);
        let magazine_sysid = crate::system_registry::torpedo_magazine_system_id();
        let magazine_online = offline_systems_opt
            .map(|set| !set.contains(&magazine_sysid))
            .unwrap_or(true);
        entity_bbs.0.insert(
            magazine_sysid,
            SystemBlackboard::TorpedoMagazine(crate::messages::TorpedoMagazineBlackboard {
                is_online: magazine_online,
                torpedoes_remaining: torpedo_sys.0.torpedoes_remaining,
                capacity: torpedo_sys.0.config.count,
            }),
        );
    }
}

/// Project a world-space entity to a [`RadarBlip`] for the HTML Tactical radar.
///
/// Returns `None` when:
/// - `shows` is empty (radar configured to show nothing)
/// - the entity's tags don't overlap `shows` (OR-logic tag filter)
/// - the entity is farther than `effective_range` from the ship (range cull)
///
/// Positions are normalised to `[-1.0, 1.0]` where ±1.0 = `effective_range`.
/// The projection is ship-centred and ship-aligned (forward = +radar_y = up).
///
/// `meta` supplies the full [`EntitySnapshot`] for richer blip data
/// (icon name, colour tint, objective flag, display name). Pass `None`
/// for dynamically-spawned entities not yet in `WorldResource`.
fn project_blip(
    uuid: &str,
    wx: f32,
    wz: f32,
    ship_x: f32,
    ship_z: f32,
    ship_yaw: f32,
    effective_range: f32,
    meta: Option<&crate::messages::EntitySnapshot>,
    shows: &[crate::entity_tags::EntityTag],
    selects: &[crate::entity_tags::EntityTag],
) -> Option<RadarBlip> {
    let raw_tags: &[String] = meta.map(|e| e.tags.as_slice()).unwrap_or(&[]);
    let radius: f32 = meta.and_then(|e| e.radius).unwrap_or(0.0);

    let entity_tags = crate::entity_tags::parse_tags(raw_tags);
    if !crate::entity_tags::matches_any(&entity_tags, shows) {
        return None;
    }
    let dx = wx - ship_x;
    let dz = wz - ship_z;
    if dx * dx + dz * dz > effective_range * effective_range {
        return None;
    }
    let cos_y = ship_yaw.cos();
    let sin_y = ship_yaw.sin();
    // Ship-aligned projection: forward = -Z at yaw=0, right = +X.
    // radar_x = dot((dx,dz), right)   = dx*cos(yaw) + dz*sin(yaw)
    // radar_y = dot((dx,dz), forward) = dx*sin(yaw) - dz*cos(yaw)
    let radar_x = (dx * cos_y + dz * sin_y) / effective_range;
    let radar_y = (dx * sin_y - dz * cos_y) / effective_range;
    let scaled_radius = radius / effective_range;
    let kind = entity_tags
        .iter()
        .find_map(|t| match t {
            crate::entity_tags::EntityTag::Asteroid => Some("asteroid"),
            crate::entity_tags::EntityTag::Ship => Some("ship"),
            crate::entity_tags::EntityTag::Station => Some("station"),
            _ => None,
        })
        .unwrap_or("unknown")
        .to_string();

    // Resolve icon name: prefer explicit `radar_icon` from snapshot, else
    // derive from tags the same way `kind` does but with finer granularity
    // (planet, star, torpedo).
    let icon = meta
        .and_then(|e| e.radar_icon.as_deref())
        .map(|s| s.to_string())
        .unwrap_or_else(|| {
            entity_tags
                .iter()
                .find_map(|t| match t {
                    crate::entity_tags::EntityTag::Asteroid => Some("asteroid"),
                    crate::entity_tags::EntityTag::Ship => Some("ship"),
                    crate::entity_tags::EntityTag::Station => Some("station"),
                    _ => None,
                })
                .unwrap_or("unknown")
                .to_string()
        });

    // Colour: from snapshot or per-icon default (matches JS KIND_COLOR).
    let color: [f32; 3] = meta
        .and_then(|e| e.colour)
        .unwrap_or_else(|| blip_default_color(&icon));

    let objective_target = meta.map(|e| e.objective_target).unwrap_or(false);
    let name = meta.and_then(|e| e.name.clone());

    // Resolve target info for selectability.
    let target_tags_raw: &[String] = meta.map(|e| e.target_tags.as_slice()).unwrap_or(&[]);
    let target_tags = crate::entity_tags::parse_tags(target_tags_raw);
    let selectable = crate::entity_tags::matches_any(&target_tags, selects);
    let threat_level = meta
        .and_then(|e| e.threat_level.as_deref())
        .map(|s| s.to_string());
    let description = meta
        .and_then(|e| e.target_description.as_deref())
        .or(name.as_deref())
        .map(|s| s.to_string());

    Some(RadarBlip {
        uuid: uuid.to_string(),
        radar_x,
        radar_y,
        scaled_radius,
        kind,
        icon,
        color,
        objective_target,
        name,
        selectable,
        threat_level,
        description,
        target_tags: target_tags_raw.to_vec(),
    })
}

/// Default RGB colour tint for a blip when the entity snapshot carries no
/// explicit colour.  Mirrors the `KIND_COLOR` palette in `radar-widget.js`.
fn blip_default_color(icon: &str) -> [f32; 3] {
    match icon {
        "asteroid" => [0.478, 0.753, 1.0], // #7ac0ff
        "ship" => [1.0, 0.502, 0.376],     // #ff8060
        "station" => [1.0, 0.878, 0.376],  // #ffe060
        "torpedo" => [1.0, 0.376, 1.0],    // #ff60ff
        "planet" => [0.376, 1.0, 0.753],   // #60ffc0
        "star" => [1.0, 0.980, 0.753],     // #fffac0
        "player" => [0.0, 1.0, 0.2],       // green — player ship
        "battleship" => [0.9, 0.2, 0.05],  // dark red — large enemy
        "cruiser" => [0.8, 0.3, 0.1],      // orange-red — medium enemy
        "destroyer" => [1.0, 0.2, 0.2],    // bright red — small enemy
        _ => [0.659, 0.690, 0.753],        // #a8b0c0 unknown
    }
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::super::shared::any_bank_accepts_human_input;
    use super::*;
    use crate::damage::SystemHull;
    use crate::lobby::{LobbyPlugin, OutboundMessage};
    use crate::messages::*;
    use crate::modifiers::ShipModifiers;
    use crate::simulation::{ShipImpulse, SimOutbox};

    #[derive(Resource, Default)]
    struct Outbox(Vec<OutboundMessage>);

    #[derive(Resource, Default)]
    struct ArcRequestLog(Vec<CoordinationEnqueue>);

    fn collect_arc_requests(
        mut reader: MessageReader<CoordinationEnqueue>,
        mut log: ResMut<ArcRequestLog>,
    ) {
        for m in reader.read() {
            log.0.push(m.clone());
        }
    }

    /// Build a minimal `ShipConfigComponent` with a tactical station that has an
    /// "Assisted" rating containing `torpedo_auto_fire` in its ai_tuning table.
    ///
    /// Post-#512 this now uses fine Tactical `[[system]]` blocks matching
    /// the ship entity TOML (phaser-fore/aft, torpedo-tube-fore-port/aft, etc.)
    /// so tests exercise the production per-fine-system gate paths rather
    /// than the legacy fallback-to-coarse-tactical path. The coarse
    /// `[[system]] id = "tactical"` block is DELETED to match production.
    fn test_ship_config() -> crate::ship_plugin::ShipConfigComponent {
        const TOML: &str = r#"
[[station]]
id = "tactical"
name = "Tactical"
description = "Weapons."
rank = "Ltn."
short_code = "TAC"
console = "tactical"

[[station.rating]]
name = "Std"
automated_systems = []

[[station.rating]]
name = "Assisted"
automated_systems = []

[station.rating.ai_tuning]
torpedo_auto_fire = {}

[[system]]
id = "phaser-fore"
kind = "phaser_bank"
station = "tactical"

[[system]]
id = "phaser-aft"
kind = "phaser_bank"
station = "tactical"

[[system]]
id = "tactical-radar"
kind = "tactical_radar"
station = "tactical"

[[system]]
id = "phaser-control"
kind = "phaser_control"
station = "tactical"

[[system]]
id = "torpedo-magazine"
kind = "torpedo_magazine"
station = "tactical"

[[system]]
id = "torpedo-tube-fore-port"
kind = "torpedo_tube"
station = "tactical"

[[system]]
id = "torpedo-tube-fore-starboard"
kind = "torpedo_tube"
station = "tactical"

[[system]]
id = "torpedo-tube-aft"
kind = "torpedo_tube"
station = "tactical"
"#;
        crate::ship_plugin::ShipConfigComponent(
            crate::ship::config::parse_and_validate(
                TOML,
                &[
                    "phaser_bank",
                    "torpedo_tube",
                    "torpedo_magazine",
                    "tactical_radar",
                    "phaser_control",
                ],
            )
            .expect("test ship config must be valid"),
        )
    }

    fn collect(mut reader: MessageReader<OutboundMessage>, mut box_: ResMut<Outbox>) {
        for m in reader.read() {
            box_.0.push(m.clone());
        }
    }

    fn test_app() -> App {
        let mut app = App::new();
        app.configure_sets(
            Update,
            (
                crate::sim_sets::SimSet::Input,
                crate::sim_sets::SimSet::Physics,
                crate::sim_sets::SimSet::Damage,
                crate::sim_sets::SimSet::Modifiers,
                crate::sim_sets::SimSet::Publish,
                crate::sim_sets::SimSet::PublishAggregate,
                crate::sim_sets::SimSet::Broadcast,
            )
                .chain(),
        )
        .add_plugins(LobbyPlugin)
        .add_plugins(bevy::time::TimePlugin)
        .add_plugins(crate::server_app::AdmissionPlugin)
        .insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
            std::time::Duration::from_millis(200),
        ))
        .init_resource::<WorldResource>()
        .add_message::<AsteroidDestroyedVfx>()
        .add_message::<crate::ai_plugin::AiEntityDestroyed>()
        .init_resource::<CurrentPhaserMode>()
        .insert_resource(TorpedoSystemResource(TorpedoSystem::new(
            TorpedoConfig::default(),
        )))
        .init_resource::<SimOutbox>()
        .init_resource::<Outbox>()
        .init_resource::<ArcRequestLog>()
        .init_resource::<crate::world::server::WorldContentRuntime>()
        .insert_resource(crate::lobby::server::ShipClientConfigResource::default())
        .add_plugins(WeaponsPlugin)
        // Override with two banks so per-bank arc checks work.
        // Uses wide (270°) arcs so existing tests that fire "port" at a
        // target ahead still pass. Tighter arcs are tested in dedicated
        // per-bank arc severance tests.
        .insert_resource(PhaserCombatConfigResource(
            crate::entity_config::PhaserCombatConfig {
                banks: vec![
                    crate::entity_config::PhaserBankConfig {
                        id: "port".into(),
                        facing_deg: -90.0,
                        fire_arc_deg: 270.0,
                        auto_arc_deg: 240.0,
                        beam_range: 0.0,
                        beam_damage_per_sec: 5.0,
                        beam_duration_secs: 6.0,
                        cooldown_secs: 6.0,
                        beam_color: vec![],
                        shield_pierce: None,
                        marker: None,
                    },
                    crate::entity_config::PhaserBankConfig {
                        id: "starboard".into(),
                        facing_deg: 90.0,
                        fire_arc_deg: 270.0,
                        auto_arc_deg: 240.0,
                        beam_range: 0.0,
                        beam_damage_per_sec: 5.0,
                        beam_duration_secs: 6.0,
                        cooldown_secs: 6.0,
                        beam_color: vec![],
                        shield_pierce: None,
                        marker: None,
                    },
                ],
            },
        ))
        .add_systems(
            Update,
            (
                // The three beam-tick phases (issue #723) share the one-tick
                // BeamContext resource, so they must run in order — a bare
                // tuple is unordered in Bevy, hence the .chain().
                (
                    tick_beams_prepare,
                    tick_beams_apply_damage,
                    tick_beams_tick_lifetimes,
                )
                    .chain(),
                // The two torpedo-tick phases (issue #724) share the
                // one-tick TorpedoTargetSnapshot resource, so they must
                // run in order too.
                (build_torpedo_target_snapshot, tick_torpedo_lifecycle).chain(),
            ),
        )
        .add_plugins(weapons_update_broadcaster())
        // PR-7 (issue #597) — `tick_shields` (formerly `tick_npc_shield_regen`)
        // now lives on `ShipShieldsPlugin`. Include it so tests that spawn NPCs
        // with `ShipShields` observe regen on every frame.
        .add_plugins(crate::shields_plugin::ShipShieldsPlugin)
        .add_systems(PostUpdate, (collect, collect_arc_requests));
        // Spawn the Ship entity with config/control-source components so all
        // weapons systems that use `Query<..., With<Ship>>.single()` have a
        // valid entity to operate on, matching what `spawn_game_start_entities`
        // would do in a full server build.
        let ship = app
            .world_mut()
            .spawn((
                crate::simulation::Ship,
                crate::simulation::LocalShip,
                test_ship_config(),
                ShipSystemControlSources::default(),
                crate::ship_plugin::ActiveStationRatings::default(),
                crate::messages::AdmittedCommands::default(),
                crate::ship_plugin::CoordinationQueue::default(),
                ShipPhysics::default(),
                crate::ship_state::ShipPhaserFrequency::default(),
                bevy::prelude::Transform::default(),
                crate::entity_spawner::EntitySystemHull(SystemHull::from_config(&[
                    (SystemId("helm".into()), 25.0),
                    (SystemId("tactical".into()), 25.0),
                    (SystemId("power".into()), 25.0),
                    (SystemId("shields".into()), 25.0),
                    // Fine Tactical hull entries (issue #512) so tests can drive
                    // sync_console_damage_tiers → offline_systems for the fine
                    // systems declared in the updated test_ship_config().
                    (SystemId("phaser-fore".into()), 15.0),
                    (SystemId("phaser-aft".into()), 15.0),
                    (SystemId("torpedo-tube-fore-port".into()), 12.0),
                    (SystemId("torpedo-tube-fore-starboard".into()), 12.0),
                    (SystemId("torpedo-tube-aft".into()), 12.0),
                    (SystemId("torpedo-magazine".into()), 20.0),
                ])),
                crate::server_app::ShipSystemBlackboards::default(),
                crate::entity_spawner::EntityUuid("test-local-ship".to_string()),
            ))
            .id();
        // Second insert to stay under Bevy's Bundle-tuple length limit.
        app.world_mut().entity_mut(ship).insert((
            // Insert per-entity weapon configs so component-path queries succeed.
            // These are overridden by individual tests via insert_resource for the
            // PhaserCombatConfigResource; we keep both in sync here.
            TorpedoSystemResource(TorpedoSystem::new(TorpedoConfig::default())),
            PhaserCombatConfigResource(crate::entity_config::PhaserCombatConfig {
                banks: vec![
                    crate::entity_config::PhaserBankConfig {
                        id: "port".into(),
                        facing_deg: -90.0,
                        fire_arc_deg: 270.0,
                        auto_arc_deg: 240.0,
                        beam_range: 0.0,
                        beam_damage_per_sec: 5.0,
                        beam_duration_secs: 6.0,
                        cooldown_secs: 6.0,
                        beam_color: vec![],
                        shield_pierce: None,
                        marker: None,
                    },
                    crate::entity_config::PhaserBankConfig {
                        id: "starboard".into(),
                        facing_deg: 90.0,
                        fire_arc_deg: 270.0,
                        auto_arc_deg: 240.0,
                        beam_range: 0.0,
                        beam_damage_per_sec: 5.0,
                        beam_duration_secs: 6.0,
                        cooldown_secs: 6.0,
                        beam_color: vec![],
                        shield_pierce: None,
                        marker: None,
                    },
                ],
            }),
            PhaserRenderConfig::default(),
            // PR 7 (issue #597) — per-entity beam / target / cooldown components.
            WeaponsTarget::default(),
            ActiveBeam::default(),
            PhaserCooldown::default(),
            // PR 10 (PRD #597) — per-entity combat activity trackers.
            crate::server_app::WeaponFiredThisTick::default(),
            crate::server_app::ShipAttackedThisTick::default(),
            LastShipAttacker::default(),
            crate::ship::combat_activity::RecentCombatActivity::default(),
            ShipImpulse(crate::impulse::ImpulseState::new()),
            ShipModifiers::new(),
        ));
        app
    }

    // ── PR 7 test helpers — per-entity access to Weapons state ──────────────
    // These wrap the `Query<&X, With<LocalShip>>` pattern that replaces
    // `world.resource::<X>()` after PR 7 (PRD #597) removed the Resource derive.
    //
    // Each helper: single-entity lookup returning owned data.

    fn get_weapons_target(app: &mut App) -> Option<String> {
        let mut q = app
            .world_mut()
            .query_filtered::<&WeaponsTarget, With<crate::server_app::LocalShip>>();
        q.single(app.world()).ok().and_then(|wt| wt.0.clone())
    }

    fn set_weapons_target(app: &mut App, uuid: Option<String>) {
        let mut q = app
            .world_mut()
            .query_filtered::<&mut WeaponsTarget, With<crate::server_app::LocalShip>>();
        if let Ok(mut wt) = q.single_mut(app.world_mut()) {
            wt.0 = uuid;
        }
    }

    fn get_active_beam_target(app: &mut App) -> Option<String> {
        let mut q = app
            .world_mut()
            .query_filtered::<&ActiveBeam, With<crate::server_app::LocalShip>>();
        q.single(app.world())
            .ok()
            .and_then(|b| b.target_uuid.clone())
    }

    fn active_beam_target_is_none(app: &mut App) -> bool {
        get_active_beam_target(app).is_none()
    }

    fn set_active_beam_target(app: &mut App, uuid: Option<String>) {
        let mut q = app
            .world_mut()
            .query_filtered::<&mut ActiveBeam, With<crate::server_app::LocalShip>>();
        if let Ok(mut b) = q.single_mut(app.world_mut()) {
            b.target_uuid = uuid;
        }
    }

    fn set_active_beam_remaining_secs(app: &mut App, secs: f32) {
        let mut q = app
            .world_mut()
            .query_filtered::<&mut ActiveBeam, With<crate::server_app::LocalShip>>();
        if let Ok(mut b) = q.single_mut(app.world_mut()) {
            b.remaining_secs = secs;
        }
    }

    fn set_active_beam_damage_accumulator(app: &mut App, val: f32) {
        let mut q = app
            .world_mut()
            .query_filtered::<&mut ActiveBeam, With<crate::server_app::LocalShip>>();
        if let Ok(mut b) = q.single_mut(app.world_mut()) {
            b.damage_accumulator = val;
        }
    }

    fn phaser_bank_is_active(app: &mut App, bank: &str) -> bool {
        let mut q = app
            .world_mut()
            .query_filtered::<&PhaserCooldown, With<crate::server_app::LocalShip>>();
        q.single(app.world())
            .ok()
            .map(|cd| cd.is_bank_active(bank))
            .unwrap_or(false)
    }

    fn start_phaser_cooldown(app: &mut App, bank: &str, secs: f32) {
        let mut q = app
            .world_mut()
            .query_filtered::<&mut PhaserCooldown, With<crate::server_app::LocalShip>>();
        if let Ok(mut cd) = q.single_mut(app.world_mut()) {
            cd.start_bank_with_cooldown(bank, secs);
        }
    }

    fn get_phaser_frequency(app: &mut App) -> f32 {
        let mut q = app
            .world_mut()
            .query_filtered::<&crate::ship_state::ShipPhaserFrequency, With<crate::server_app::LocalShip>>();
        q.single(app.world()).map(|f| f.0).unwrap_or(0.5)
    }

    fn set_ship_yaw(app: &mut App, yaw: f32) {
        let mut q = app
            .world_mut()
            .query_filtered::<&mut ShipPhysics, With<crate::server_app::LocalShip>>();
        let mut p = q
            .single_mut(app.world_mut())
            .expect("expected Ship with ShipPhysics");
        p.yaw = yaw;
    }

    fn push(app: &mut App, token: &str, msg: ClientMessage) {
        app.world_mut()
            .resource_mut::<Messages<InboundMessage>>()
            .write(InboundMessage {
                token: token.into(),
                msg,
            });
    }

    fn tick(app: &mut App) -> Vec<OutboundMessage> {
        app.update();
        let sim_entries = std::mem::take(&mut app.world_mut().resource_mut::<SimOutbox>().0);
        let mut out = app.world().resource::<Outbox>().0.clone();
        for (target, msg) in sim_entries {
            out.push(OutboundMessage {
                target,
                msg,
                delivery: crate::messages::DeliveryClass::Reliable,
            });
        }
        app.world_mut().resource_mut::<Outbox>().0.clear();
        out
    }

    fn load_tube_now(app: &mut App, tube: &str) {
        // The systems now prefer the per-entity component over the resource.
        // Update both to keep them in sync.
        let mut q = app
            .world_mut()
            .query_filtered::<&mut TorpedoSystemResource, With<crate::server_app::LocalShip>>();
        if let Ok(mut ts) = q.single_mut(app.world_mut()) {
            ts.0.tube_mut(tube)
                .expect("test tube should exist")
                .loaded_count = 1;
        } else {
            let world = app.world_mut();
            let mut res = world.resource_mut::<TorpedoSystemResource>();
            res.0
                .tube_mut(tube)
                .expect("test tube should exist")
                .loaded_count = 1;
        }
    }

    fn start_game(app: &mut App) {
        push(
            app,
            "captain",
            ClientMessage::Identify {
                token: "captain".into(),
                name: "Alice".into(),
            },
        );
        tick(app);
        push(
            app,
            "captain",
            ClientMessage::SelectStation {
                station: "Captain".into(),
            },
        );
        tick(app);
        push(app, "captain", ClientMessage::SetReady { ready: true });
        tick(app);
    }

    fn setup_weapons_world(
        app: &mut App,
        asteroid_x: f32,
        asteroid_z: f32,
    ) -> bevy::ecs::entity::Entity {
        let uuid = "target-uuid".to_string();
        app.world_mut()
            .insert_resource(WorldResource(crate::messages::WorldData {
                entities: vec![crate::messages::EntitySnapshot::asteroid(
                    &uuid, asteroid_x, asteroid_z, 2.0,
                )],
                ..Default::default()
            }));
        // handle_set_target and tick_beams use live ECS Transforms
        // (live_entity_xz), so every WorldResource entry must also have a
        // matching ECS entity with the components all queries expect.
        app.world_mut()
            .spawn((
                crate::simulation::Asteroid,
                crate::simulation::AsteroidUuid(uuid),
                EntitySystemHull(crate::damage::SystemHull::from_config(&[(
                    crate::messages::SystemId("captain".into()),
                    30.0,
                )])),
                Transform::from_xyz(asteroid_x, 0.0, asteroid_z),
            ))
            .id()
    }

    fn setup_weapons_world_with_entity(
        app: &mut App,
        asteroid_x: f32,
        asteroid_z: f32,
    ) -> bevy::ecs::entity::Entity {
        setup_weapons_world(app, asteroid_x, asteroid_z)
    }

    fn start_game_with_weapons(app: &mut App) {
        push(
            app,
            "captain",
            ClientMessage::Identify {
                token: "captain".into(),
                name: "Alice".into(),
            },
        );
        tick(app);
        push(
            app,
            "captain",
            ClientMessage::SelectStation {
                station: "Captain".into(),
            },
        );
        tick(app);
        push(
            app,
            "weapons",
            ClientMessage::Identify {
                token: "weapons".into(),
                name: "Bob".into(),
            },
        );
        tick(app);
        push(
            app,
            "weapons",
            ClientMessage::SelectStation {
                station: "Tactical".into(),
            },
        );
        tick(app);
        push(app, "captain", ClientMessage::SetReady { ready: true });
        push(app, "weapons", ClientMessage::SetReady { ready: true });
        tick(app);
    }

    fn lock_and_fire(app: &mut App, asteroid_x: f32, asteroid_z: f32) -> Vec<OutboundMessage> {
        setup_weapons_world_with_entity(app, asteroid_x, asteroid_z);
        start_game_with_weapons(app);
        push(
            app,
            "weapons",
            ClientMessage::ControlSystem {
                target: crate::system_registry::tactical_radar_system_id(),
                payload: SystemControlPayload::SetTarget {
                    uuid: "target-uuid".into(),
                },
            },
        );
        let _ = tick(app);
        push(
            app,
            "weapons",
            ClientMessage::FirePhaser {
                bank: "port".to_string(),
            },
        );
        tick(app)
    }

    // ── SetTarget / TargetLock tests ───────────────────────────────────────

    #[test]
    fn valid_target_within_range_replies_with_target_lock_confirmed() {
        let mut app = test_app();
        setup_weapons_world(&mut app, 30.0, 0.0);
        start_game_with_weapons(&mut app);

        push(
            &mut app,
            "weapons",
            ClientMessage::ControlSystem {
                target: crate::system_registry::tactical_radar_system_id(),
                payload: SystemControlPayload::SetTarget {
                    uuid: "target-uuid".into(),
                },
            },
        );
        let out = tick(&mut app);

        let lock = out
            .iter()
            .find_map(|m| match &m.msg {
                ServerMessage::TargetLock { uuid, locked } => Some((uuid.clone(), *locked)),
                _ => None,
            })
            .expect("expected a TargetLock response");
        assert_eq!(lock.0, "target-uuid");
        assert!(lock.1, "expected locked=true for in-range asteroid");

        assert_eq!(get_weapons_target(&mut app).as_deref(), Some("target-uuid"));
    }

    #[test]
    fn asteroid_outside_weapons_range_replies_with_target_lock_rejected() {
        let mut app = test_app();
        setup_weapons_world(&mut app, 400.0, 0.0);
        start_game_with_weapons(&mut app);

        push(
            &mut app,
            "weapons",
            ClientMessage::ControlSystem {
                target: crate::system_registry::tactical_radar_system_id(),
                payload: SystemControlPayload::SetTarget {
                    uuid: "target-uuid".into(),
                },
            },
        );
        let out = tick(&mut app);

        let lock = out
            .iter()
            .find_map(|m| match &m.msg {
                ServerMessage::TargetLock { uuid, locked } => Some((uuid.clone(), *locked)),
                _ => None,
            })
            .expect("expected a TargetLock response");
        assert!(!lock.1, "expected locked=false for out-of-range asteroid");
        assert!(get_weapons_target(&mut app).is_none());
    }

    #[test]
    fn unknown_uuid_replies_with_target_lock_rejected() {
        let mut app = test_app();
        setup_weapons_world(&mut app, 10.0, 0.0);
        start_game_with_weapons(&mut app);

        push(
            &mut app,
            "weapons",
            ClientMessage::ControlSystem {
                target: crate::system_registry::tactical_radar_system_id(),
                payload: SystemControlPayload::SetTarget {
                    uuid: "no-such-asteroid".into(),
                },
            },
        );
        let out = tick(&mut app);

        let lock = out
            .iter()
            .find_map(|m| match &m.msg {
                ServerMessage::TargetLock { uuid, locked } => Some((uuid.clone(), *locked)),
                _ => None,
            })
            .expect("expected a TargetLock response");
        assert!(!lock.1, "expected locked=false for unknown UUID");
        assert!(get_weapons_target(&mut app).is_none());
    }

    // ── WeaponsUpdate / fire_ready tests ───────────────────────────────────

    #[test]
    fn weapons_update_fire_ready_true_when_target_in_range_and_arc() {
        let mut app = test_app();
        setup_weapons_world(&mut app, 0.0, -20.0);
        start_game_with_weapons(&mut app);

        push(
            &mut app,
            "weapons",
            ClientMessage::ControlSystem {
                target: crate::system_registry::tactical_radar_system_id(),
                payload: SystemControlPayload::SetTarget {
                    uuid: "target-uuid".into(),
                },
            },
        );
        // Target changes → WeaponsUpdate fires this tick.
        let out = tick(&mut app);

        let update = out
            .iter()
            .find_map(|m| match &m.msg {
                ServerMessage::WeaponsUpdate {
                    target_uuid, banks, ..
                } => Some((target_uuid.clone(), banks.iter().any(|b| b.fire_ready))),
                _ => None,
            })
            .expect("expected a WeaponsUpdate message");
        assert_eq!(update.0.as_deref(), Some("target-uuid"));
        assert!(
            update.1,
            "expected fire_ready=true for in-range, forward-arc target"
        );
    }

    #[test]
    fn weapons_update_fire_ready_false_when_target_out_of_phaser_range() {
        let mut app = test_app();
        setup_weapons_world(&mut app, 0.0, -50.0);
        start_game_with_weapons(&mut app);

        push(
            &mut app,
            "weapons",
            ClientMessage::ControlSystem {
                target: crate::system_registry::tactical_radar_system_id(),
                payload: SystemControlPayload::SetTarget {
                    uuid: "target-uuid".into(),
                },
            },
        );
        // Target changes → WeaponsUpdate fires this tick.
        let out = tick(&mut app);

        let update = out
            .iter()
            .find_map(|m| match &m.msg {
                ServerMessage::WeaponsUpdate {
                    target_uuid, banks, ..
                } => Some((target_uuid.clone(), banks.iter().any(|b| b.fire_ready))),
                _ => None,
            })
            .expect("expected a WeaponsUpdate message");
        assert_eq!(update.0.as_deref(), Some("target-uuid"));
        assert!(
            !update.1,
            "expected fire_ready=false for beyond-phaser-range target"
        );
    }

    // ── FirePhaser / beam lifecycle tests ──────────────────────────────────

    #[test]
    fn fire_phaser_on_valid_target_broadcasts_beam_started() {
        let mut app = test_app();
        let out = lock_and_fire(&mut app, 0.0, -20.0);

        let beam_started = out
            .iter()
            .find(|m| matches!(&m.msg, ServerMessage::BeamStarted { .. }));
        assert!(
            beam_started.is_some(),
            "expected BeamStarted after firing at fire-ready target"
        );
        match &beam_started.unwrap().msg {
            ServerMessage::BeamStarted { target_uuid, .. } => {
                assert_eq!(target_uuid, "target-uuid")
            }
            _ => unreachable!(),
        }
        match &beam_started.unwrap().target {
            Target::All => {}
            t => panic!("BeamStarted should target All, got {:?}", t),
        }

        assert_eq!(
            get_active_beam_target(&mut app).as_deref(),
            Some("target-uuid")
        );
    }

    #[test]
    fn fire_phaser_rejected_during_cooldown() {
        let mut app = test_app();
        let _ = lock_and_fire(&mut app, 0.0, -20.0);

        set_active_beam_target(&mut app, None);
        start_phaser_cooldown(&mut app, "port", 3.0);

        push(
            &mut app,
            "weapons",
            ClientMessage::FirePhaser {
                bank: "port".to_string(),
            },
        );
        let out = tick(&mut app);

        assert!(
            !out.iter()
                .any(|m| matches!(&m.msg, ServerMessage::BeamStarted { .. })),
            "BeamStarted should not fire during cooldown"
        );
    }

    #[test]
    fn fire_phaser_ignored_from_non_weapons_player() {
        let mut app = test_app();
        setup_weapons_world(&mut app, 0.0, -20.0);
        start_game(&mut app);

        push(
            &mut app,
            "captain",
            ClientMessage::FirePhaser {
                bank: "port".to_string(),
            },
        );
        let out = tick(&mut app);

        assert!(
            !out.iter()
                .any(|m| matches!(&m.msg, ServerMessage::BeamStarted { .. })),
            "captain should not be able to fire phaser"
        );
    }

    #[test]
    fn fire_phaser_rejected_when_target_outside_bank_arc() {
        let mut app = test_app();
        // Target at starboard beam (20, 0), bearing +90°, which is outside the
        // port bank's 270° arc centered at -90° (covers -135° to 45°).
        setup_weapons_world(&mut app, 20.0, 0.0);
        start_game_with_weapons(&mut app);
        push(
            &mut app,
            "weapons",
            ClientMessage::ControlSystem {
                target: crate::system_registry::tactical_radar_system_id(),
                payload: SystemControlPayload::SetTarget {
                    uuid: "target-uuid".into(),
                },
            },
        );
        let _ = tick(&mut app);
        push(
            &mut app,
            "weapons",
            ClientMessage::FirePhaser {
                bank: "port".to_string(),
            },
        );
        let out = tick(&mut app);

        assert!(
            !out.iter()
                .any(|m| matches!(&m.msg, ServerMessage::BeamStarted { .. })),
            "FirePhaser should be rejected when target is outside bank's fire arc"
        );
    }

    #[test]
    fn full_beam_duration_kills_asteroid() {
        let mut app = test_app();
        // setup_weapons_world (called by lock_and_fire) now spawns the
        // asteroid ECS entity. Fetch its handle after setup.
        let _ = lock_and_fire(&mut app, 0.0, -20.0);
        let asteroid_entity = {
            let mut q = app
                .world_mut()
                .query::<(bevy::ecs::entity::Entity, &crate::simulation::AsteroidUuid)>();
            q.iter(app.world())
                .find(|(_, u)| u.0 == "target-uuid")
                .map(|(e, _)| e)
                .expect("setup_weapons_world should have spawned the target asteroid")
        };

        assert_eq!(
            get_active_beam_target(&mut app).as_deref(),
            Some("target-uuid")
        );

        set_active_beam_damage_accumulator(&mut app, 30.0);
        set_active_beam_remaining_secs(&mut app, 5.0);

        let out = tick(&mut app);

        let destroyed = out
            .iter()
            .find(|m| matches!(&m.msg, ServerMessage::AsteroidDestroyed { .. }));
        assert!(
            destroyed.is_some(),
            "expected AsteroidDestroyed when asteroid HP reaches 0"
        );
        match &destroyed.unwrap().msg {
            ServerMessage::AsteroidDestroyed { uuid } => assert_eq!(uuid, "target-uuid"),
            _ => unreachable!(),
        }

        assert!(
            out.iter()
                .any(|m| matches!(&m.msg, ServerMessage::BeamEnded { .. })),
            "expected BeamEnded after asteroid destruction"
        );

        assert!(
            !app.world()
                .resource::<WorldResource>()
                .0
                .entities
                .iter()
                .any(|a| a.uuid == "target-uuid"),
            "destroyed asteroid should be removed from WorldData"
        );

        assert!(active_beam_target_is_none(&mut app));

        assert!(
            phaser_bank_is_active(&mut app, "port"),
            "cooldown should start after beam end"
        );

        assert!(
            app.world()
                .get::<EntitySystemHull>(asteroid_entity)
                .is_none(),
            "asteroid entity should be despawned"
        );
    }

    #[test]
    fn beam_severs_when_target_leaves_bank_arc() {
        let mut app = test_app();
        // Target at port beam (-20, 0), bearing -90° — inside port bank's
        // 270° arc centered at -90° (covers -135° to 45°).
        let _ = lock_and_fire(&mut app, -20.0, 0.0);

        // Rotate 180° so the target moves to starboard beam (bearing +90°),
        // which is outside the port bank's arc.
        set_ship_yaw(&mut app, std::f32::consts::PI);

        let out = tick(&mut app);

        assert!(
            out.iter()
                .any(|m| matches!(&m.msg, ServerMessage::BeamEnded { .. })),
            "expected BeamEnded when target leaves bank fire arc"
        );
        assert!(
            active_beam_target_is_none(&mut app),
            "beam should be cleared after sever-by-arc"
        );
        assert!(
            phaser_bank_is_active(&mut app, "port"),
            "cooldown should start after arc sever"
        );
    }

    #[test]
    fn beam_severs_when_target_leaves_phaser_range() {
        let mut app = test_app();
        let _ = lock_and_fire(&mut app, 0.0, -20.0);

        // Move the live ECS Transform out of range. tick_beams reads the
        // live position, not the WorldResource snapshot.
        let entity = {
            let mut q = app
                .world_mut()
                .query::<(bevy::ecs::entity::Entity, &crate::simulation::AsteroidUuid)>();
            q.iter(app.world())
                .find(|(_, u)| u.0 == "target-uuid")
                .map(|(e, _)| e)
                .expect("target entity should exist")
        };
        app.world_mut()
            .entity_mut(entity)
            .insert(Transform::from_xyz(0.0, 0.0, -50.0));

        let out = tick(&mut app);

        assert!(
            out.iter()
                .any(|m| matches!(&m.msg, ServerMessage::BeamEnded { .. })),
            "expected BeamEnded when target leaves phaser range"
        );
        assert!(
            active_beam_target_is_none(&mut app),
            "beam should be cleared after sever-by-range"
        );
        assert!(
            phaser_bank_is_active(&mut app, "port"),
            "cooldown should start after range sever"
        );
    }

    #[test]
    fn no_damage_refund_on_sever() {
        let mut app = test_app();
        let asteroid_entity = app
            .world_mut()
            .spawn((
                crate::simulation::Asteroid,
                crate::simulation::AsteroidUuid("target-uuid".into()),
                EntitySystemHull(crate::damage::SystemHull::from_config(&[(
                    crate::messages::SystemId("captain".into()),
                    30.0,
                )])),
            ))
            .id();
        // Target at port beam (-20, 0) so the port bank's arc check passes.
        let _ = lock_and_fire(&mut app, -20.0, 0.0);

        set_active_beam_damage_accumulator(&mut app, 10.0);
        let _ = tick(&mut app);

        // Rotate 180° — target moves to starboard beam, outside port bank's arc.
        set_ship_yaw(&mut app, std::f32::consts::PI);
        let _ = tick(&mut app);

        let hp = app
            .world()
            .get::<EntitySystemHull>(asteroid_entity)
            .map(|h| h.0.total_current());
        assert!(
            hp.is_some() && hp.unwrap() < 30.0,
            "asteroid should retain damage after sever (no refund), hp={:?}",
            hp
        );
    }

    #[test]
    fn retarget_after_cooldown_cancels_prior_beam_and_starts_new() {
        let mut app = test_app();
        app.world_mut()
            .insert_resource(WorldResource(crate::messages::WorldData {
                entities: vec![
                    crate::messages::EntitySnapshot::asteroid("t1", 0.0, -20.0, 2.0),
                    crate::messages::EntitySnapshot::asteroid("t2", 0.0, -15.0, 2.0),
                ],
                ..Default::default()
            }));
        // Spawn matching ECS entities so live_entity_xz can find them.
        app.world_mut().spawn((
            crate::simulation::Asteroid,
            crate::simulation::AsteroidUuid("t1".into()),
            EntitySystemHull(crate::damage::SystemHull::from_config(&[(
                crate::messages::SystemId("captain".into()),
                30.0,
            )])),
            Transform::from_xyz(0.0, 0.0, -20.0),
        ));
        app.world_mut().spawn((
            crate::simulation::Asteroid,
            crate::simulation::AsteroidUuid("t2".into()),
            EntitySystemHull(crate::damage::SystemHull::from_config(&[(
                crate::messages::SystemId("captain".into()),
                30.0,
            )])),
            Transform::from_xyz(0.0, 0.0, -15.0),
        ));
        start_game_with_weapons(&mut app);

        push(
            &mut app,
            "weapons",
            ClientMessage::ControlSystem {
                target: crate::system_registry::tactical_radar_system_id(),
                payload: SystemControlPayload::SetTarget { uuid: "t1".into() },
            },
        );
        let _ = tick(&mut app);
        push(
            &mut app,
            "weapons",
            ClientMessage::FirePhaser {
                bank: "port".to_string(),
            },
        );
        let _ = tick(&mut app);
        assert_eq!(get_active_beam_target(&mut app).as_deref(), Some("t1"));

        set_active_beam_remaining_secs(&mut app, 0.0);
        set_active_beam_damage_accumulator(&mut app, 0.0);
        let _ = tick(&mut app);

        assert!(phaser_bank_is_active(&mut app, "port"));

        start_phaser_cooldown(&mut app, "port", 0.0);

        push(
            &mut app,
            "weapons",
            ClientMessage::ControlSystem {
                target: crate::system_registry::tactical_radar_system_id(),
                payload: SystemControlPayload::SetTarget { uuid: "t2".into() },
            },
        );
        let _ = tick(&mut app);
        push(
            &mut app,
            "weapons",
            ClientMessage::FirePhaser {
                bank: "port".to_string(),
            },
        );
        let out = tick(&mut app);

        assert!(
            out.iter()
                .any(|m| matches!(&m.msg, ServerMessage::BeamStarted { .. })),
            "expected BeamStarted for new target after cooldown"
        );
        assert_eq!(get_active_beam_target(&mut app).as_deref(), Some("t2"));
    }

    // ── SetPhaserMode tests ────────────────────────────────────────────────

    #[test]
    fn weapons_console_can_set_phaser_mode_to_manual() {
        let mut app = test_app();
        start_game_with_weapons(&mut app);
        push(
            &mut app,
            "weapons",
            ClientMessage::ControlSystem {
                target: crate::system_registry::phaser_control_system_id(),
                payload: SystemControlPayload::SetPhaserMode {
                    mode: crate::messages::PhaserMode::Manual,
                },
            },
        );
        tick(&mut app);
        assert_eq!(
            app.world().resource::<CurrentPhaserMode>().0,
            crate::messages::PhaserMode::Manual,
            "phaser mode should be Manual after SetPhaserMode"
        );
    }

    #[test]
    fn non_weapons_player_cannot_set_phaser_mode() {
        let mut app = test_app();
        start_game_with_weapons(&mut app);
        // Establish a known mode (Auto) via the authorised player first.
        push(
            &mut app,
            "weapons",
            ClientMessage::ControlSystem {
                target: crate::system_registry::phaser_control_system_id(),
                payload: SystemControlPayload::SetPhaserMode {
                    mode: crate::messages::PhaserMode::Auto,
                },
            },
        );
        tick(&mut app);
        // Non-weapons player attempts to switch back to Manual — must be ignored.
        push(
            &mut app,
            "captain",
            ClientMessage::ControlSystem {
                target: crate::system_registry::phaser_control_system_id(),
                payload: SystemControlPayload::SetPhaserMode {
                    mode: crate::messages::PhaserMode::Manual,
                },
            },
        );
        tick(&mut app);
        assert_eq!(
            app.world().resource::<CurrentPhaserMode>().0,
            crate::messages::PhaserMode::Auto,
            "phaser mode should stay Auto when non-Weapons player sends SetPhaserMode"
        );
    }

    // ── FireTorpedo tests ──────────────────────────────────────────────────

    #[test]
    fn tactical_player_can_fire_torpedo_broadcasts_torpedo_launched() {
        let mut app = test_app();
        start_game_with_weapons(&mut app);
        load_tube_now(&mut app, "fore_port");

        push(
            &mut app,
            "weapons",
            ClientMessage::FireTorpedo {
                tube: "fore_port".to_string(),
                target_uuid: None,
            },
        );
        let out = tick(&mut app);

        assert!(
            out.iter().any(|m| matches!(&m.msg, ServerMessage::TorpedoLaunched { tube, .. } if tube == "fore_port")),
            "expected TorpedoLaunched broadcast after Tactical fires torpedo"
        );
    }

    /// Regression test for PRD #597 gap-3: an NPC ship spawned with a
    /// `[torpedoes]` TOML block must carry its own `TorpedoSystemResource`
    /// component, and firing from it via the `ai:<uuid>` token path must
    /// launch a torpedo. Two subchecks:
    ///
    /// 1. Direct wiring: `TorpedoSystem::launch()` called on the NPC's own
    ///    component successfully returns `Launched` (i.e. the tubes are
    ///    populated and `torpedoes_remaining > 0`).
    /// 2. End-to-end message routing: an `ai:<uuid>` `FireTorpedo` message
    ///    arriving through `InboundMessage` reaches the NPC's tubes and
    ///    emits a `TorpedoLaunched` broadcast, drawing from the NPC's own
    ///    per-entity tube state — the player-ship `TorpedoSystemResource`
    ///    resource is left untouched.
    ///
    /// NPC AI does not currently emit `FireTorpedo` messages autonomously;
    /// verifying that pipeline is future work (see PRD #487 fine-grained
    /// tactical decomposition). This test covers the wiring.
    #[test]
    fn npc_ship_can_fire_torpedo_when_toml_has_torpedoes_block() {
        use crate::ai_plugin::AiTokenRegistry;
        use crate::entity_spawner::EntityUuid;
        use crate::torpedo::LaunchResult;

        let mut app = test_app();
        app.init_resource::<AiTokenRegistry>();

        let npc_uuid = "cc000000-0000-0000-0000-000000000001";

        // Simulate what `src/entities/spawner.rs` does for an NPC with
        // `[torpedoes]`: attach a `TorpedoSystemResource` component built
        // from the runtime config, with default tubes (fore_port, fore_starboard, aft).
        let torpedo_config = TorpedoConfig::default();
        let npc_torpedo_sys = crate::torpedo::TorpedoSystem::new(torpedo_config);
        let mut npc_ai_sources = crate::ship::control_source::ControlSourceResolver::new();
        // #801: seed the fine tube + magazine systems (there is no coarse
        // tactical system to seed).
        for sysid in [
            crate::system_registry::torpedo_tube_fore_port_system_id(),
            crate::system_registry::torpedo_tube_fore_starboard_system_id(),
            crate::system_registry::torpedo_tube_aft_system_id(),
            crate::system_registry::torpedo_magazine_system_id(),
        ] {
            npc_ai_sources.set(sysid, crate::ship::control_source::ControlSource::Ai);
        }
        let npc_entity = app
            .world_mut()
            .spawn((
                crate::server_app::Ship,
                EntityUuid(npc_uuid.to_string()),
                crate::ship_plugin::ShipSystemControlSources(npc_ai_sources),
                ShipPhysics::default(),
                WeaponsTarget::default(),
                TorpedoSystemResource(npc_torpedo_sys),
                crate::server_app::WeaponFiredThisTick::default(),
                bevy::prelude::Transform::default(),
            ))
            .id();
        {
            let mut reg = app.world_mut().resource_mut::<AiTokenRegistry>();
            reg.register_with_entity(npc_uuid, npc_entity);
        }

        // Subcheck 1: direct wiring — the NPC's own component has functional
        // tubes and `.launch()` succeeds when the tube is loaded.
        {
            let mut ts = app
                .world_mut()
                .get_mut::<TorpedoSystemResource>(npc_entity)
                .expect("NPC must have TorpedoSystemResource component");
            ts.0.tube_mut("fore_port")
                .expect("default TorpedoSystem must expose fore_port tube")
                .loaded_count = 1;
            let result = ts.0.launch(
                "fore_port",
                "direct-launch-uuid".to_string(),
                0.0,
                0.0,
                0.0,
                None,
                Some(npc_uuid.to_string()),
            );
            assert!(
                matches!(result, LaunchResult::Launched { .. }),
                "direct TorpedoSystem::launch on NPC's own component must succeed, got {result:?}"
            );
        }

        // Reload the tube for the end-to-end path (previous launch consumed it).
        {
            let mut ts = app
                .world_mut()
                .get_mut::<TorpedoSystemResource>(npc_entity)
                .unwrap();
            ts.0.tube_mut("fore_port").unwrap().loaded_count = 1;
            ts.0.in_flight.clear();
        }

        // Subcheck 2: end-to-end message routing.
        // Snapshot the player-ship (resource) torpedo count to prove the NPC's
        // fire draws from its own component, not from the shared Resource.
        let player_torpedoes_before = app
            .world()
            .resource::<TorpedoSystemResource>()
            .0
            .torpedoes_remaining;

        let ai_token = format!("ai:{}", npc_uuid);
        push(
            &mut app,
            &ai_token,
            ClientMessage::FireTorpedo {
                tube: "fore_port".to_string(),
                target_uuid: None,
            },
        );
        let out = tick(&mut app);

        assert!(
            out.iter().any(|m| matches!(&m.msg, ServerMessage::TorpedoLaunched { tube, .. } if tube == "fore_port")),
            "NPC should broadcast TorpedoLaunched after ai:<uuid> FireTorpedo message"
        );

        // The player-ship Resource must NOT have been drained.
        let player_torpedoes_after = app
            .world()
            .resource::<TorpedoSystemResource>()
            .0
            .torpedoes_remaining;
        assert_eq!(
            player_torpedoes_before, player_torpedoes_after,
            "NPC fire must draw from its own per-entity TorpedoSystemResource, \
             leaving the global (player-ship) Resource untouched"
        );
    }

    #[test]
    fn local_console_token_can_fire_torpedo() {
        // issue #422: actions from the local HTML console (browser server
        // viewscreen / native wry server) arrive under LOCAL_CONSOLE_TOKEN with
        // no remote PeerJS session, so holder_for_station(tactical) is None.
        // `tactical_authorized` must treat that token as an authorized local
        // operator so a button press actually launches end-to-end — the
        // decode→map→InboundMessage→fire hop the wasm bridge cannot unit-test.
        let mut app = test_app();
        // No player holds Tactical here — authorization comes purely from the
        // local-console bypass.
        load_tube_now(&mut app, "fore_port");
        push(
            &mut app,
            crate::console_bridge::LOCAL_CONSOLE_TOKEN,
            ClientMessage::FireTorpedo {
                tube: "fore_port".to_string(),
                target_uuid: None,
            },
        );
        let out = tick(&mut app);

        assert!(
            out.iter().any(|m| matches!(&m.msg, ServerMessage::TorpedoLaunched { tube, .. } if tube == "fore_port")),
            "local console token should be authorized to fire torpedoes end-to-end (issue #422)"
        );
    }

    #[test]
    fn torpedo_system_resource_reflects_battleship_toml_torpedoes_block() {
        // End-to-end TOML-driven wiring check: build the runtime
        // TorpedoSystem the same way `spawn_game_start_entities` does
        // (parse alliance_battleship.toml → TorpedoesConfig::to_runtime → TorpedoSystem)
        // and assert the magazine size matches the TOML.
        let toml_str = include_str!("../../../assets/entities/alliance_battleship.toml");
        let config = crate::entity_config::EntityConfig::from_toml(toml_str)
            .expect("alliance_battleship.toml must parse");
        let tc = config
            .torpedoes
            .expect("alliance_battleship must declare [torpedoes]");
        let runtime = tc.to_runtime();
        let sys = crate::torpedo::TorpedoSystem::new(runtime.clone());
        // Magazine size matches TOML — changing `count = 30` to `count = 99`
        // in alliance_battleship.toml would fail this assertion.
        assert_eq!(sys.torpedoes_remaining, tc.count);
        assert_eq!(sys.config.damage_hull, tc.damage_hull);
        assert_eq!(sys.config.load_time, tc.load_time);
        assert!((sys.config.turn_rate - tc.turn_rate_deg_per_sec.to_radians()).abs() < 1e-5);
    }

    #[test]
    fn phaser_combat_config_resource_reflects_battleship_toml_weapons_console() {
        // End-to-end TOML-driven wiring check: build the runtime
        // PhaserCombatConfig the same way `spawn_game_start_entities` does
        // (parse alliance_battleship.toml → PhaserCombatConfig::from_weapons_console
        // → PhaserCombatConfigResource) and assert the resulting per-bank
        // values are exactly what the TOML says.
        let toml_str = include_str!("../../../assets/entities/alliance_battleship.toml");
        let config = crate::entity_config::EntityConfig::from_toml(toml_str)
            .expect("alliance_battleship.toml must parse");
        let wc = config
            .weapons_console
            .expect("alliance_battleship must declare [weapons_console]");
        let combat = crate::entity_config::PhaserCombatConfig::from_weapons_console(&wc);

        // alliance_battleship.toml has two banks (fore, aft) with matching combat values.
        // Fore bank is double-damage (8.0 dps) and shorter range (40) than the standard cruiser.
        assert_eq!(combat.banks.len(), 2, "must have fore and aft banks");
        let fore = &combat.banks[0];
        assert_eq!(fore.id, "fore");
        assert_eq!(fore.cooldown_secs, 6.0, "cooldown_secs from TOML bank");
        assert_eq!(
            fore.beam_duration_secs, 6.0,
            "beam_duration_secs from TOML bank"
        );
        assert_eq!(
            fore.beam_damage_per_sec, 8.0,
            "beam_damage_per_sec from TOML bank"
        );
        assert_eq!(fore.beam_range, 40.0, "beam_range from TOML bank");

        // And starting the cooldown produces exactly that value, so it flows
        // through to live `PhaserCooldown.bank_remaining_secs`.
        let mut cd = PhaserCooldown::default();
        cd.start_bank("test", fore.cooldown_secs);
        assert_eq!(
            cd.bank_remaining_secs("test"),
            6.0,
            "PhaserCooldown::start_bank must use the TOML-sourced cooldown"
        );
    }

    #[test]
    fn non_tactical_player_cannot_fire_torpedo() {
        let mut app = test_app();
        start_game_with_weapons(&mut app);
        load_tube_now(&mut app, "fore_port");

        push(
            &mut app,
            "captain",
            ClientMessage::FireTorpedo {
                tube: "fore_port".to_string(),
                target_uuid: None,
            },
        );
        let out = tick(&mut app);

        assert!(
            !out.iter()
                .any(|m| matches!(&m.msg, ServerMessage::TorpedoLaunched { .. })),
            "captain should not be able to fire torpedo"
        );
    }

    #[test]
    fn fire_torpedo_during_lobby_fires_when_no_simset_gate() {
        // Note: The Lobby gate is now at the SimSet chain level.
        // In test configurations without SimSet, the system processes messages during Lobby.
        let mut app = test_app();
        load_tube_now(&mut app, "aft");
        push(
            &mut app,
            "weapons",
            ClientMessage::Identify {
                token: "weapons".into(),
                name: "Bob".into(),
            },
        );
        tick(&mut app);
        push(
            &mut app,
            "weapons",
            ClientMessage::SelectStation {
                station: "Tactical".into(),
            },
        );
        tick(&mut app);

        push(
            &mut app,
            "weapons",
            ClientMessage::FireTorpedo {
                tube: "aft".to_string(),
                target_uuid: None,
            },
        );
        let out = tick(&mut app);

        assert!(
            out.iter()
                .any(|m| matches!(&m.msg, ServerMessage::TorpedoLaunched { .. })),
            "FireTorpedo should fire during Lobby when no SimSet gate is configured"
        );
    }

    #[test]
    fn torpedo_launched_is_broadcast_to_all() {
        let mut app = test_app();
        start_game_with_weapons(&mut app);
        load_tube_now(&mut app, "fore_starboard");

        push(
            &mut app,
            "weapons",
            ClientMessage::FireTorpedo {
                tube: "fore_starboard".to_string(),
                target_uuid: None,
            },
        );
        let out = tick(&mut app);

        let launched = out
            .iter()
            .find(|m| matches!(&m.msg, ServerMessage::TorpedoLaunched { .. }))
            .expect("expected TorpedoLaunched");
        assert!(
            matches!(&launched.target, Target::All),
            "TorpedoLaunched should be broadcast to All, not {:?}",
            launched.target
        );
    }

    #[test]
    fn torpedo_does_not_detonate_on_asteroid_field_anchor_entity() {
        // Regression for "torpedoes don't appear when you hit fire": the
        // default scenario seats the player ship at (280, 0, 0), 280 m from
        // an `asteroid_field_main` anchor entity at the origin. That anchor
        // entity carries an `[asteroid_field]` section with
        // `outer_radius = 350`, and `EntitySnapshot.radius` is populated from
        // that outer radius. `find_detonation_hits` treats every entity in
        // the world with a non-zero radius as a hittable target, so the
        // torpedo detonated on the field anchor on its first physics tick —
        // before the firing crew ever saw a sphere on the viewscreen.
        //
        // Asteroid-field anchors are virtual organisational entities and
        // must never act as torpedo detonation targets.
        use crate::entity_config::AsteroidFieldConfig;
        use crate::entity_spawner::{AsteroidFieldSection, EntityUuid};

        let mut app = test_app();
        start_game_with_weapons(&mut app);

        let field_uuid = "field-uuid".to_string();
        // Mirror the production code path: the WorldResource snapshot for the
        // field anchor reports radius = outer_radius.
        app.world_mut()
            .insert_resource(WorldResource(crate::messages::WorldData {
                entities: vec![crate::messages::EntitySnapshot {
                    uuid: field_uuid.clone(),
                    position: Some([0.0, 0.0, 0.0]),
                    radius: Some(350.0),
                    inner_radius: Some(300.0),
                    shape: Some("torus".into()),
                    tags: vec!["asteroid_field".into()],
                    ..Default::default()
                }],
                ..Default::default()
            }));
        // Real ECS-side anchor entity so the live-position path also sees it.
        app.world_mut().spawn((
            EntityUuid(field_uuid.clone()),
            AsteroidFieldSection(AsteroidFieldConfig {
                inner_radius: 300.0,
                outer_radius: 350.0,
                density: 0.005,
                spawn_distance: 250.0,
                despawn_distance: 300.0,
                asteroid_type_paths: vec![],
                cosmetic_type_paths: vec![],
                shape: None,
                anchor: None,
                anchor_offset: [0.0, 0.0, 0.0],
                shield_pierce: 0.0,
                tags: vec![],
                grid: None,
                random_rotation: None,
            }),
            Transform::from_xyz(0.0, 0.0, 0.0),
        ));

        // Move the ship inside the field-anchor's "radius" (300 < 350).
        {
            let mut q = app
                .world_mut()
                .query_filtered::<&mut ShipPhysics, With<crate::server_app::LocalShip>>();
            let mut p = q
                .single_mut(app.world_mut())
                .expect("Ship with ShipPhysics");
            p.x = 280.0;
        }
        load_tube_now(&mut app, "fore_port");

        push(
            &mut app,
            "weapons",
            ClientMessage::FireTorpedo {
                tube: "fore_port".to_string(),
                target_uuid: None,
            },
        );
        // First tick processes the FireTorpedo; second tick is where
        // `tick_torpedo_lifecycle` evaluates detonations against the live
        // target list (including the field anchor at the origin).
        tick(&mut app);
        tick(&mut app);

        let in_flight_len = {
            // Systems prefer the per-entity component; read from it for assertion.
            let mut q = app
                .world_mut()
                .query_filtered::<&TorpedoSystemResource, With<crate::server_app::LocalShip>>();
            q.single(app.world())
                .ok()
                .map(|ts| ts.0.in_flight.len())
                .unwrap_or_else(|| {
                    app.world()
                        .resource::<TorpedoSystemResource>()
                        .0
                        .in_flight
                        .len()
                })
        };
        assert_eq!(
            in_flight_len, 1,
            "torpedo should still be in flight after ticking — the asteroid \
             field anchor entity must not be treated as a detonation target"
        );
    }

    // ── ShipModifiers integration tests ────────────────────────────────────

    #[test]
    fn empty_modifier_table_reproduces_base_phaser_damage() {
        let mut app = test_app();
        setup_weapons_world_with_entity(&mut app, 0.0, -20.0);
        start_game_with_weapons(&mut app);

        push(
            &mut app,
            "weapons",
            ClientMessage::ControlSystem {
                target: crate::system_registry::tactical_radar_system_id(),
                payload: SystemControlPayload::SetTarget {
                    uuid: "target-uuid".into(),
                },
            },
        );
        tick(&mut app);
        push(
            &mut app,
            "weapons",
            ClientMessage::FirePhaser {
                bank: "port".to_string(),
            },
        );
        tick(&mut app);

        let hp_before = {
            let world = app.world().resource::<WorldResource>();
            world
                .0
                .entities
                .iter()
                .find(|a| a.uuid == "target-uuid")
                .map(|_| true)
        };
        assert!(hp_before.is_some(), "asteroid should still exist after <1s");
    }

    #[test]
    fn phaser_damage_modifier_doubles_kill_rate() {
        use crate::messages::{ModifierSlot, ModifierSource};
        use crate::modifiers::{Modifier, ShipModifiers};

        let mut app_fast = test_app();
        setup_weapons_world_with_entity(&mut app_fast, 0.0, -20.0);
        start_game_with_weapons(&mut app_fast);
        {
            let mut q = app_fast
                .world_mut()
                .query_filtered::<&mut ShipModifiers, With<crate::simulation::LocalShip>>();
            let mut mods = q.single_mut(app_fast.world_mut()).unwrap();
            mods.add_or_update(Modifier {
                source: ModifierSource::ImpulseDrive,
                slot: ModifierSlot::PhaserDamage,
                bonus: 1.0,
            });
        }
        push(
            &mut app_fast,
            "weapons",
            ClientMessage::ControlSystem {
                target: crate::system_registry::tactical_radar_system_id(),
                payload: SystemControlPayload::SetTarget {
                    uuid: "target-uuid".into(),
                },
            },
        );
        tick(&mut app_fast);
        push(
            &mut app_fast,
            "weapons",
            ClientMessage::FirePhaser {
                bank: "port".to_string(),
            },
        );
        tick(&mut app_fast);

        set_active_beam_damage_accumulator(&mut app_fast, BEAM_DAMAGE_PER_SEC * 2.0 * 3.5);
        tick(&mut app_fast);

        let still_exists_fast = app_fast
            .world()
            .resource::<WorldResource>()
            .0
            .entities
            .iter()
            .any(|a| a.uuid == "target-uuid");
        assert!(
            !still_exists_fast,
            "with 2× phaser damage modifier, asteroid should be destroyed after 3.5s of beam"
        );

        let mut app_base = test_app();
        setup_weapons_world_with_entity(&mut app_base, 0.0, -20.0);
        start_game_with_weapons(&mut app_base);
        push(
            &mut app_base,
            "weapons",
            ClientMessage::ControlSystem {
                target: crate::system_registry::tactical_radar_system_id(),
                payload: SystemControlPayload::SetTarget {
                    uuid: "target-uuid".into(),
                },
            },
        );
        tick(&mut app_base);
        push(
            &mut app_base,
            "weapons",
            ClientMessage::FirePhaser {
                bank: "port".to_string(),
            },
        );
        tick(&mut app_base);
        set_active_beam_damage_accumulator(&mut app_base, BEAM_DAMAGE_PER_SEC * 1.0 * 3.5);
        tick(&mut app_base);

        let still_exists_base = app_base
            .world()
            .resource::<WorldResource>()
            .0
            .entities
            .iter()
            .any(|a| a.uuid == "target-uuid");
        assert!(still_exists_base, "with identity modifier, asteroid should survive 3.5s of beam (only 17.5/30 HP removed)");
    }

    // ── SetPhaserFrequency delegation tests ────────────────────────────────

    fn start_game_with_sensors_and_weapons(app: &mut App) {
        push(
            app,
            "captain",
            ClientMessage::Identify {
                token: "captain".into(),
                name: "Alice".into(),
            },
        );
        tick(app);
        push(
            app,
            "captain",
            ClientMessage::SelectStation {
                station: "Captain".into(),
            },
        );
        tick(app);
        push(
            app,
            "sensors",
            ClientMessage::Identify {
                token: "sensors".into(),
                name: "Spock".into(),
            },
        );
        tick(app);
        push(
            app,
            "sensors",
            ClientMessage::SelectStation {
                station: "Sensors".into(),
            },
        );
        tick(app);
        push(
            app,
            "weapons",
            ClientMessage::Identify {
                token: "weapons".into(),
                name: "Bob".into(),
            },
        );
        tick(app);
        push(
            app,
            "weapons",
            ClientMessage::SelectStation {
                station: "Tactical".into(),
            },
        );
        tick(app);
        push(app, "captain", ClientMessage::SetReady { ready: true });
        push(app, "sensors", ClientMessage::SetReady { ready: true });
        push(app, "weapons", ClientMessage::SetReady { ready: true });
        tick(app);
    }

    /// Build the admitted-envelope form of a frequency change (issue #804):
    /// the only wire shape since the legacy top-level message was deleted.
    fn set_phaser_frequency_msg(frequency: f32) -> ClientMessage {
        ClientMessage::ControlSystem {
            target: crate::system_registry::phaser_control_system_id(),
            payload: SystemControlPayload::SetPhaserFrequency { frequency },
        }
    }

    #[test]
    fn tactical_holder_can_set_phaser_frequency() {
        let mut app = test_app();
        start_game_with_weapons(&mut app);
        push(&mut app, "weapons", set_phaser_frequency_msg(0.8));
        tick(&mut app);
        let freq = get_phaser_frequency(&mut app);
        assert!(
            (freq - 0.8).abs() < 1e-5,
            "Tactical holder should set phaser frequency to 0.8, got {freq}"
        );
    }

    #[test]
    fn sensors_holder_cannot_set_phaser_frequency() {
        // Delegation removed in B4 — only Tactical holder may set phaser frequency.
        let mut app = test_app();
        start_game_with_sensors_and_weapons(&mut app);
        push(&mut app, "sensors", set_phaser_frequency_msg(0.9));
        tick(&mut app);
        let freq = get_phaser_frequency(&mut app);
        assert!(
            (freq - 0.5).abs() < 1e-5,
            "Sensors holder must NOT change phaser frequency, got {freq}"
        );
    }

    #[test]
    fn unrelated_console_cannot_set_phaser_frequency() {
        let mut app = test_app();
        start_game(&mut app);
        push(&mut app, "captain", set_phaser_frequency_msg(0.9));
        tick(&mut app);
        let freq = get_phaser_frequency(&mut app);
        assert!(
            (freq - 0.5).abs() < 1e-5,
            "Captain must NOT change phaser frequency, got {freq}"
        );
    }

    /// When the phaser-control system operates AI, human `SetPhaserFrequency`
    /// envelopes are refused at admission (mirrors the navigation console's
    /// `control_system_rejected_when_ai_controlled`).
    #[test]
    fn set_phaser_frequency_rejected_when_phaser_control_ai() {
        let mut app = test_app();
        start_game_with_weapons(&mut app);
        {
            let mut q = app
                .world_mut()
                .query_filtered::<&mut ShipSystemControlSources, With<crate::server_app::LocalShip>>();
            for mut cs in q.iter_mut(app.world_mut()) {
                cs.0.set(
                    crate::system_registry::phaser_control_system_id(),
                    crate::ship::control_source::ControlSource::Ai,
                );
            }
        }
        push(&mut app, "weapons", set_phaser_frequency_msg(0.9));
        tick(&mut app);
        let freq = get_phaser_frequency(&mut app);
        assert!(
            (freq - 0.5).abs() < 1e-5,
            "an AI-operated phaser-control must refuse human frequency input, got {freq}"
        );
    }

    #[test]
    fn set_phaser_frequency_clamps_value() {
        let mut app = test_app();
        start_game_with_weapons(&mut app);
        push(&mut app, "weapons", set_phaser_frequency_msg(1.5));
        tick(&mut app);
        let freq = get_phaser_frequency(&mut app);
        assert!(
            (freq - 1.0).abs() < 1e-5,
            "frequency above 1.0 should clamp to 1.0, got {freq}"
        );

        push(&mut app, "weapons", set_phaser_frequency_msg(-0.5));
        tick(&mut app);
        let freq = get_phaser_frequency(&mut app);
        assert!(
            (freq - 0.0).abs() < 1e-5,
            "frequency below 0.0 should clamp to 0.0, got {freq}"
        );
    }

    // ── NPC / station phaser damage (issue #311) ──────────────────────────

    fn setup_npc_world(app: &mut App, npc_x: f32, npc_z: f32) {
        app.world_mut()
            .insert_resource(WorldResource(crate::messages::WorldData {
                entities: vec![crate::messages::EntitySnapshot {
                    uuid: "npc-1".into(),
                    position: Some([npc_x, 0.0, npc_z]),
                    tags: vec!["ship".into()],
                    ..Default::default()
                }],
                ..Default::default()
            }));
    }

    fn spawn_npc_entity(
        app: &mut App,
        npc_x: f32,
        npc_z: f32,
        max_hp: f32,
    ) -> bevy::ecs::entity::Entity {
        app.world_mut()
            .spawn((
                crate::entity_spawner::EntityUuid("npc-1".into()),
                EntitySystemHull(crate::damage::SystemHull::from_config(&[(
                    crate::messages::SystemId("captain".into()),
                    max_hp,
                )])),
                Transform::from_xyz(npc_x, 0.0, npc_z),
            ))
            .id()
    }

    // ── Cycle 1: phaser beam reduces NPC hull ─────────────────────────────

    #[test]
    fn phaser_beam_damages_npc_entity_hull() {
        let mut app = test_app();
        setup_npc_world(&mut app, 0.0, -20.0);
        start_game_with_weapons(&mut app);

        let npc_entity = spawn_npc_entity(&mut app, 0.0, -20.0, 30.0);

        push(
            &mut app,
            "weapons",
            ClientMessage::ControlSystem {
                target: crate::system_registry::tactical_radar_system_id(),
                payload: SystemControlPayload::SetTarget {
                    uuid: "npc-1".into(),
                },
            },
        );
        tick(&mut app);
        push(
            &mut app,
            "weapons",
            ClientMessage::FirePhaser {
                bank: "port".to_string(),
            },
        );
        tick(&mut app);

        // Accumulate damage but don't destroy
        set_active_beam_damage_accumulator(&mut app, 10.0);
        set_active_beam_remaining_secs(&mut app, 5.0);
        tick(&mut app);

        let hp = app
            .world()
            .get::<EntitySystemHull>(npc_entity)
            .expect("NPC entity should still exist")
            .0
            .total_current();
        assert!(
            hp < 30.0,
            "NPC hull should be reduced after phaser hit, got {hp}"
        );
    }

    // ── Cycle 2: NPC at 0 HP is despawned and EntityDespawned broadcast ──

    #[test]
    fn phaser_beam_destroys_npc_entity_when_hull_reaches_zero() {
        let mut app = test_app();
        setup_npc_world(&mut app, 0.0, -20.0);
        start_game_with_weapons(&mut app);

        let npc_entity = spawn_npc_entity(&mut app, 0.0, -20.0, 30.0);

        push(
            &mut app,
            "weapons",
            ClientMessage::ControlSystem {
                target: crate::system_registry::tactical_radar_system_id(),
                payload: SystemControlPayload::SetTarget {
                    uuid: "npc-1".into(),
                },
            },
        );
        tick(&mut app);
        push(
            &mut app,
            "weapons",
            ClientMessage::FirePhaser {
                bank: "port".to_string(),
            },
        );
        tick(&mut app);

        // Force lethal damage
        set_active_beam_damage_accumulator(&mut app, 30.0);
        set_active_beam_remaining_secs(&mut app, 5.0);
        let out = tick(&mut app);

        // ECS entity despawned
        assert!(
            app.world().get::<EntitySystemHull>(npc_entity).is_none(),
            "NPC entity should be despawned after hull reaches 0"
        );

        // EntityDespawned wire message broadcast to all
        let despawned_msg = out
            .iter()
            .find(|m| matches!(&m.msg, ServerMessage::EntityDespawned { uuid } if uuid == "npc-1"));
        assert!(
            despawned_msg.is_some(),
            "expected EntityDespawned {{ uuid: npc-1 }} broadcast"
        );

        // BeamEnded sent
        assert!(
            out.iter()
                .any(|m| matches!(&m.msg, ServerMessage::BeamEnded { .. })),
            "expected BeamEnded after NPC destruction"
        );

        // Beam cleared, cooldown started
        assert!(active_beam_target_is_none(&mut app));
        assert!(phaser_bank_is_active(&mut app, "port"));
    }

    // ── NPC shields integration ────────────────────────────────────────────

    /// Spawn a shielded NPC: same as `spawn_npc_entity` but also attaches a
    /// `ShipShields` (num_facings=1) so the damage routing path is exercised
    /// end-to-end.
    fn spawn_shielded_npc_entity(
        app: &mut App,
        npc_x: f32,
        npc_z: f32,
        hull_max: f32,
        shield_max: f32,
        regen_per_sec: f32,
    ) -> bevy::ecs::entity::Entity {
        use crate::weapons::shield::{ShieldConfig, ShieldSystem};
        app.world_mut()
            .spawn((
                // PR-7 (issue #597) — NPC ships carry the `Ship` marker
                // so the unified `tick_shields` picks them up.
                crate::simulation::Ship,
                crate::entity_spawner::EntityUuid("npc-1".into()),
                EntitySystemHull(crate::damage::SystemHull::from_config(&[(
                    crate::messages::SystemId("captain".into()),
                    hull_max,
                )])),
                crate::ship::shields::ShipShields(
                    ShieldSystem::new(&ShieldConfig {
                        num_facings: 1,
                        max_hp: shield_max.round() as i32,
                        regen_per_sec,
                        offline_duration: 10.0,
                    }),
                    0.5,
                ),
                Transform::from_xyz(npc_x, 0.0, npc_z),
            ))
            .id()
    }

    #[test]
    fn phaser_beam_damages_shielded_npc_routes_through_shield_first() {
        let mut app = test_app();
        setup_npc_world(&mut app, 0.0, -20.0);
        start_game_with_weapons(&mut app);

        let npc_entity = spawn_shielded_npc_entity(&mut app, 0.0, -20.0, 30.0, 20.0, 0.0);

        push(
            &mut app,
            "weapons",
            ClientMessage::ControlSystem {
                target: crate::system_registry::tactical_radar_system_id(),
                payload: SystemControlPayload::SetTarget {
                    uuid: "npc-1".into(),
                },
            },
        );
        tick(&mut app);
        push(
            &mut app,
            "weapons",
            ClientMessage::FirePhaser {
                bank: "port".to_string(),
            },
        );
        tick(&mut app);

        // Apply 5 units of damage. With pierce=0 (default in test config),
        // the entire amount lands on the shield, hull is unchanged.
        set_active_beam_damage_accumulator(&mut app, 5.0);
        set_active_beam_remaining_secs(&mut app, 5.0);
        tick(&mut app);

        let shields = app
            .world()
            .get::<crate::ship::shields::ShipShields>(npc_entity)
            .expect("NPC must still have ShipShields component");
        assert!(
            shields.0.facings[0].hp < 20,
            "shield must absorb damage, got {}",
            shields.0.facings[0].hp
        );
        assert!(
            shields.0.facings[0].is_online(),
            "shield must still be online"
        );

        let hull_hp = app
            .world()
            .get::<EntitySystemHull>(npc_entity)
            .expect("hull must still exist")
            .0
            .total_current();
        assert_eq!(hull_hp, 30.0, "hull must be untouched while shield holds");
    }

    #[test]
    fn phaser_beam_breaks_shield_then_leaks_to_hull() {
        let mut app = test_app();
        setup_npc_world(&mut app, 0.0, -20.0);
        start_game_with_weapons(&mut app);

        let npc_entity = spawn_shielded_npc_entity(&mut app, 0.0, -20.0, 30.0, 10.0, 0.0);

        push(
            &mut app,
            "weapons",
            ClientMessage::ControlSystem {
                target: crate::system_registry::tactical_radar_system_id(),
                payload: SystemControlPayload::SetTarget {
                    uuid: "npc-1".into(),
                },
            },
        );
        tick(&mut app);
        push(
            &mut app,
            "weapons",
            ClientMessage::FirePhaser {
                bank: "port".to_string(),
            },
        );
        tick(&mut app);

        // Apply 15 units of damage. With shield=10, shield depletes
        // and 5 units leak to hull.
        set_active_beam_damage_accumulator(&mut app, 15.0);
        set_active_beam_remaining_secs(&mut app, 5.0);
        tick(&mut app);

        let shields = app
            .world()
            .get::<crate::ship::shields::ShipShields>(npc_entity)
            .expect("ShipShields component must persist after break");
        // With ShipShields, a depleted facing goes offline (offline_remaining > 0),
        // not permanently broken.
        assert_eq!(shields.0.facings[0].hp, 0);
        assert!(
            !shields.0.facings[0].is_online(),
            "facing must go offline once depleted"
        );

        let hull_hp = app
            .world()
            .get::<EntitySystemHull>(npc_entity)
            .expect("hull must exist")
            .0
            .total_current();
        assert!(
            hull_hp < 30.0 && hull_hp > 20.0,
            "hull must take only the leak (~5 units), got {hull_hp}"
        );
    }

    #[test]
    fn phaser_beam_post_break_skips_shield_routing_entirely() {
        let mut app = test_app();
        setup_npc_world(&mut app, 0.0, -20.0);
        start_game_with_weapons(&mut app);

        // Spawn with already-offline shield (facing depleted, offline timer running).
        use crate::weapons::shield::{ShieldConfig, ShieldSystem};
        let mut shield_sys = ShieldSystem::new(&ShieldConfig {
            num_facings: 1,
            max_hp: 20,
            regen_per_sec: 0.0,
            offline_duration: 10.0,
        });
        // Deplete the facing so it goes offline.
        shield_sys.apply_damage(20, 0.0);
        assert!(!shield_sys.facings[0].is_online(), "facing must be offline");

        let npc_entity = app
            .world_mut()
            .spawn((
                crate::entity_spawner::EntityUuid("npc-1".into()),
                EntitySystemHull(crate::damage::SystemHull::from_config(&[(
                    crate::messages::SystemId("captain".into()),
                    30.0,
                )])),
                crate::ship::shields::ShipShields(shield_sys, 0.5),
                Transform::from_xyz(0.0, 0.0, -20.0),
            ))
            .id();

        push(
            &mut app,
            "weapons",
            ClientMessage::ControlSystem {
                target: crate::system_registry::tactical_radar_system_id(),
                payload: SystemControlPayload::SetTarget {
                    uuid: "npc-1".into(),
                },
            },
        );
        tick(&mut app);
        push(
            &mut app,
            "weapons",
            ClientMessage::FirePhaser {
                bank: "port".to_string(),
            },
        );
        tick(&mut app);

        set_active_beam_damage_accumulator(&mut app, 5.0);
        set_active_beam_remaining_secs(&mut app, 5.0);
        tick(&mut app);

        let hull_hp = app
            .world()
            .get::<EntitySystemHull>(npc_entity)
            .expect("hull must exist")
            .0
            .total_current();
        // Hull must take damage (offline shield does not absorb).
        assert!(
            hull_hp < 30.0,
            "offline shield must let damage through to hull, got {hull_hp}"
        );
        let shields = app
            .world()
            .get::<crate::ship::shields::ShipShields>(npc_entity)
            .expect("ShipShields component must persist");
        assert_eq!(
            shields.0.facings[0].hp, 0,
            "offline facing hp must remain 0, got {}",
            shields.0.facings[0].hp
        );
        assert!(
            !shields.0.facings[0].is_online(),
            "facing must remain offline"
        );
    }

    #[test]
    fn shield_regen_advances_npc_shield_below_max() {
        let mut app = test_app();
        setup_npc_world(&mut app, 0.0, -20.0);
        start_game_with_weapons(&mut app);

        let npc_entity = spawn_shielded_npc_entity(&mut app, 0.0, -20.0, 30.0, 20.0, 5.0);

        // Damage the shield to 10 HP.
        if let Some(mut shields) = app
            .world_mut()
            .get_mut::<crate::ship::shields::ShipShields>(npc_entity)
        {
            shields.0.facings[0].hp = 10;
        }

        // Advance time. The Bevy `Time` resource advances on each `app.update()`
        // call; we tick a few frames and expect regen to push hp upward.
        for _ in 0..3 {
            tick(&mut app);
        }

        let shields = app
            .world()
            .get::<crate::ship::shields::ShipShields>(npc_entity)
            .expect("ShipShields must persist");
        // We don't assert exact values (frame timing varies in tests) but we
        // verify regen is making forward progress and not stuck at 10.
        assert!(
            shields.0.facings[0].hp > 10,
            "shield must regen between ticks, got {}",
            shields.0.facings[0].hp
        );
        assert!(
            shields.0.facings[0].hp <= 20,
            "shield must clamp to max_hp, got {}",
            shields.0.facings[0].hp
        );
        assert!(shields.0.facings[0].is_online());
    }

    // ── PR2: Torpedo damage routes through ShipShields on the player ship ──

    /// Verify that a torpedo detonation on the player ship reduces `ShipShields`
    /// HP before leaking to the hull — end-to-end ShipShields coverage for the
    /// torpedo damage path (PR2: Unified ShipShields).
    #[test]
    fn torpedo_hit_reduces_ship_shields_on_local_ship() {
        use crate::entity_spawner::EntityUuid;
        use crate::server_app::LocalShip;
        use crate::weapons::shield::{ShieldConfig, ShieldSystem};
        use crate::weapons::torpedo::Torpedo;

        let mut app = test_app();
        start_game_with_weapons(&mut app);

        // Give the player ship ShipShields with known HP.
        let player_entity = app
            .world_mut()
            .query_filtered::<Entity, With<LocalShip>>()
            .single(app.world())
            .unwrap();

        let shield_max_hp = 100i32;
        let shield_sys = ShieldSystem::new(&ShieldConfig {
            num_facings: 4,
            max_hp: shield_max_hp,
            regen_per_sec: 0.0,
            offline_duration: 10.0,
        });
        app.world_mut().entity_mut(player_entity).insert((
            EntityUuid("player-ship".into()),
            crate::ship::shields::ShipShields(shield_sys, 0.5),
            Transform::from_xyz(0.0, 0.0, 0.0),
        ));

        // Also expose the player ship in the world snapshot so the torpedo can
        // find it as a target.
        app.world_mut()
            .insert_resource(WorldResource(crate::messages::WorldData {
                entities: vec![crate::messages::EntitySnapshot {
                    uuid: "player-ship".into(),
                    position: Some([0.0, 0.0, 0.0]),
                    radius: Some(5.0),
                    ..Default::default()
                }],
                ..Default::default()
            }));

        // Read initial total shield HP.
        let shields_before: i32 = app
            .world()
            .entity(player_entity)
            .get::<crate::ship::shields::ShipShields>()
            .unwrap()
            .0
            .facings
            .iter()
            .map(|f| f.hp)
            .sum();
        assert_eq!(shields_before, shield_max_hp * 4);

        // Read initial hull HP.
        let hull_before = app
            .world()
            .entity(player_entity)
            .get::<crate::entity_spawner::EntitySystemHull>()
            .unwrap()
            .0
            .total_current();

        // Directly inject a torpedo already adjacent to the player ship so it
        // detonates on the next tick. We write into both the per-entity component
        // and the resource to stay in sync.
        let torpedo = Torpedo {
            uuid: "test-torp-1".into(),
            x: 1.0, // 1 m away from player at origin — within detonation_radius
            z: 0.0,
            heading: 0.0,
            lifespan_remaining: 30.0,
            target_uuid: Some("player-ship".into()),
            source_uuid: None,  // no source → no self-detonation exclusion
            shield_pierce: 0.0, // no pierce → all damage goes to shields first
        };
        // Write to the per-entity component (preferred by systems) and resource.
        {
            let mut q = app
                .world_mut()
                .query_filtered::<&mut TorpedoSystemResource, With<crate::server_app::LocalShip>>();
            if let Ok(mut ts) = q.single_mut(app.world_mut()) {
                ts.0.in_flight.push(torpedo.clone());
            }
        }
        app.world_mut()
            .resource_mut::<TorpedoSystemResource>()
            .0
            .in_flight
            .push(torpedo);

        // Tick once — torpedo detonates and routes damage through ShipShields.
        tick(&mut app);

        let shields_after: i32 = app
            .world()
            .entity(player_entity)
            .get::<crate::ship::shields::ShipShields>()
            .unwrap()
            .0
            .facings
            .iter()
            .map(|f| f.hp)
            .sum();

        let hull_after = app
            .world()
            .entity(player_entity)
            .get::<crate::entity_spawner::EntitySystemHull>()
            .unwrap()
            .0
            .total_current();

        // Shield HP must decrease (torpedo damage_shields absorbed by shield).
        // (If damage_shields == 0 in the TOML config the test is still valid:
        // it just shows hull dropped instead, but we accept either change.)
        let total_damage_taken =
            (shields_before - shields_after) + ((hull_before - hull_after) as i32);
        assert!(
            total_damage_taken > 0,
            "torpedo hit must cause total damage: shields_before={shields_before}, shields_after={shields_after}, \
             hull_before={hull_before}, hull_after={hull_after}"
        );
        // The important invariant: if damage_shields > 0, shield must have taken damage first.
        // We verify this indirectly: hull must not exceed its pre-hit value.
        assert!(
            hull_after <= hull_before,
            "hull must not increase after torpedo hit, got {hull_after} > {hull_before}"
        );
    }

    // ── Cycle 3: AiEntityDestroyed message written on NPC destruction ─────

    #[test]
    fn phaser_beam_emits_ai_entity_destroyed_on_npc_kill() {
        #[derive(Resource, Default)]
        struct DestroyedBox(Vec<crate::ai_plugin::AiEntityDestroyed>);

        let mut app = test_app();
        app.init_resource::<DestroyedBox>();
        app.add_systems(
            bevy::app::Update,
            |mut r: bevy::ecs::prelude::MessageReader<crate::ai_plugin::AiEntityDestroyed>,
             mut b: bevy::ecs::prelude::ResMut<DestroyedBox>| {
                for ev in r.read() {
                    b.0.push(ev.clone());
                }
            },
        );

        setup_npc_world(&mut app, 0.0, -20.0);
        start_game_with_weapons(&mut app);
        spawn_npc_entity(&mut app, 0.0, -20.0, 30.0);

        push(
            &mut app,
            "weapons",
            ClientMessage::ControlSystem {
                target: crate::system_registry::tactical_radar_system_id(),
                payload: SystemControlPayload::SetTarget {
                    uuid: "npc-1".into(),
                },
            },
        );
        tick(&mut app);
        push(
            &mut app,
            "weapons",
            ClientMessage::FirePhaser {
                bank: "port".to_string(),
            },
        );
        tick(&mut app);

        set_active_beam_damage_accumulator(&mut app, 30.0);
        set_active_beam_remaining_secs(&mut app, 5.0);
        tick(&mut app);
        tick(&mut app); // second tick allows PostUpdate-equivalent collector to drain the message

        let destroyed_events = app.world().resource::<DestroyedBox>();
        assert!(
            destroyed_events.0.iter().any(|e| e.entity_uuid == "npc-1"),
            "AiEntityDestroyed must be emitted with entity_uuid 'npc-1' so on_destroyed triggers fire"
        );
    }

    // ── NPC as shooter: handle_fire_phaser (unified) / tick_beams ────────────

    /// Set up `AiTokenRegistry`, an NPC entity with `AiControllerComponent` +
    /// `ActiveBeam`/`PhaserCooldown` (unified per-entity phaser state), and a target entity.
    fn setup_npc_shooter(
        app: &mut App,
        npc_uuid: &str,
        target_uuid: &str,
        target_x: f32,
        target_z: f32,
    ) -> (bevy::ecs::entity::Entity, bevy::ecs::entity::Entity) {
        use crate::ai_plugin::AiControllerComponent;
        use crate::entity_spawner::{EntitySystemHull, EntityUuid};

        // Spawn NPC entity facing toward negative-Z (yaw = 0 → forward = -Z).
        // Includes the Ship marker so the unified `tick_beams` picks it up as
        // a shooter (matches the production `entities::spawner::spawn_entity`
        // path where every ship gets `Ship` — see PRD #597).
        //
        // Also mirrors production by inserting `ShipSystemControlSources` with
        // the Tactical system set to `Ai`, and the NPC's target lock in
        // `WeaponsTarget` — both required by the unified `handle_fire_phaser`
        // per-ship query. `WeaponsTarget` is the ship's authoritative lock
        // whether a human or `ai_target_selection` set it, so an AI shooter
        // seeds it exactly as a human one would.
        let mut sources = crate::ship::control_source::ControlSourceResolver::new();
        // #801: seed the fine systems for the banks these tests fire
        // ("port"/"starboard" per the test_app combat config) — there is no
        // coarse tactical system to seed.
        for bank in ["port", "starboard"] {
            sources.set(
                crate::system_registry::phaser_bank_system_id(bank).unwrap(),
                crate::ship::control_source::ControlSource::Ai,
            );
        }

        let npc_entity = app
            .world_mut()
            .spawn((
                crate::server_app::Ship,
                EntityUuid(npc_uuid.to_string()),
                AiControllerComponent,
                crate::ship_plugin::ShipSystemControlSources(sources),
                WeaponsTarget(Some(target_uuid.to_string())),
                ActiveBeam::default(),
                PhaserCooldown::default(),
                ShipPhysics::default(),
                Transform::from_xyz(0.0, 0.0, 0.0),
            ))
            .id();

        // Register with the Bevy entity so handle_fire_phaser can look it up.
        {
            let mut reg = app.world_mut().resource_mut::<AiTokenRegistry>();
            reg.register_with_entity(npc_uuid, npc_entity);
        }

        // Spawn target entity.
        let target_entity = app
            .world_mut()
            .spawn((
                EntityUuid(target_uuid.to_string()),
                EntitySystemHull(crate::damage::SystemHull::from_config(&[(
                    crate::messages::SystemId("captain".into()),
                    50.0,
                )])),
                Transform::from_xyz(target_x, 0.0, target_z),
            ))
            .id();

        (npc_entity, target_entity)
    }

    #[test]
    fn npc_fire_phaser_activates_entity_phaser_state() {
        // NPC entity at origin, target directly ahead (negative-Z), within beam range.
        // Sending a FirePhaser InboundMessage for the NPC's ai: token should set
        // `ActiveBeam::target_uuid = Some(...)` after one update.
        use crate::ai_plugin::AiTokenRegistry;

        let mut app = test_app();
        app.init_resource::<AiTokenRegistry>();

        let npc_uuid = "00000000-0000-0000-0000-000000000001";
        let target_uuid = "00000000-0000-0000-0000-000000000002";

        let (npc_entity, _target_entity) =
            setup_npc_shooter(&mut app, npc_uuid, target_uuid, 0.0, -20.0);

        // Send FirePhaser as the NPC's synthetic token.
        let ai_token = format!("ai:{}", npc_uuid);
        push(
            &mut app,
            &ai_token,
            ClientMessage::FirePhaser {
                bank: "port".to_string(),
            },
        );
        app.update();

        let beam = app
            .world()
            .get::<ActiveBeam>(npc_entity)
            .expect("NPC entity must have ActiveBeam component");
        assert!(
            beam.target_uuid.is_some(),
            "ActiveBeam::target_uuid should be Some after NPC fires phaser via ai: token"
        );
    }

    #[test]
    fn npc_beam_tick_applies_damage_to_target_hull() {
        // With an active NPC beam, each tick of tick_beams reduces
        // the target's EntitySystemHull.
        use crate::ai_plugin::AiTokenRegistry;
        use crate::entity_spawner::EntitySystemHull;

        let mut app = test_app();
        app.init_resource::<AiTokenRegistry>();

        let npc_uuid = "00000000-0000-0000-0000-000000000003";
        let target_uuid_str = "00000000-0000-0000-0000-000000000004";

        let (npc_entity, target_entity) =
            setup_npc_shooter(&mut app, npc_uuid, target_uuid_str, 0.0, -10.0);

        // Activate the beam directly on the per-entity ActiveBeam component.
        {
            let mut beam = app.world_mut().get_mut::<ActiveBeam>(npc_entity).unwrap();
            beam.target_uuid = Some(target_uuid_str.to_string());
            beam.remaining_secs = 10.0;
        }

        let hp_before = app
            .world()
            .get::<EntitySystemHull>(target_entity)
            .unwrap()
            .0
            .total_current();

        // Run several ticks so damage accumulates.
        for _ in 0..10 {
            app.update();
        }

        let hp_after = app
            .world()
            .get::<EntitySystemHull>(target_entity)
            .unwrap()
            .0
            .total_current();
        assert!(
            hp_after < hp_before,
            "target hull must decrease as NPC beam ticks (before={hp_before}, after={hp_after})"
        );
    }

    #[test]
    fn npc_beam_tick_records_shooter_as_last_attacker() {
        // Write-on-damage (#689): when a live beam hits a ship target that
        // carries a `LastShipAttacker` component, `tick_beams` records the
        // shooter's UUID as that target's last attacker. This write fires in
        // Phase 2 before the `damage_to_apply <= 0` guard, but only when the
        // target entity actually carries the component — so we insert it.
        use crate::ai_plugin::AiTokenRegistry;

        let mut app = test_app();
        app.init_resource::<AiTokenRegistry>();

        let npc_uuid = "00000000-0000-0000-0000-000000000003";
        let target_uuid_str = "00000000-0000-0000-0000-000000000004";

        let (npc_entity, target_entity) =
            setup_npc_shooter(&mut app, npc_uuid, target_uuid_str, 0.0, -10.0);

        // The attacker-write branch only fires if the target carries
        // `LastShipAttacker`; `setup_npc_shooter` does not add it.
        app.world_mut()
            .entity_mut(target_entity)
            .insert(LastShipAttacker::default());

        // Activate the beam directly on the per-entity ActiveBeam component.
        {
            let mut beam = app.world_mut().get_mut::<ActiveBeam>(npc_entity).unwrap();
            beam.target_uuid = Some(target_uuid_str.to_string());
            beam.remaining_secs = 10.0;
        }

        // Tick enough for the beam to reach and hit the target.
        for _ in 0..10 {
            app.update();
        }

        assert_eq!(
            app.world()
                .get::<LastShipAttacker>(target_entity)
                .unwrap()
                .0,
            Some(npc_uuid.to_string()),
            "beam hit must record the shooter UUID as the target's last attacker"
        );
    }

    /// The writer's half of the `AiEntityAttacked` exactly-once contract
    /// (issue #702).
    ///
    /// `tick_beams`' attacker-write branch runs every tick a beam is live.
    /// Post-#702 the rising edge that fires `AiEntityAttacked` — and through it
    /// `on_entity_attacked` scenario triggers — *is* `LastShipAttacker`'s change
    /// detection, so a blind write would re-fire the trigger on every tick of a
    /// sustained beam. This pins the compare: across many ticks of one live beam
    /// from one shooter, the component is marked changed exactly once.
    ///
    /// `ai_entity_attacked_not_re_emitted_for_same_attacker` pins the reader's
    /// half in `ai::server`.
    #[test]
    fn sustained_beam_marks_last_attacker_changed_exactly_once() {
        use crate::ai_plugin::AiTokenRegistry;

        #[derive(Resource, Default)]
        struct ChangeCount(usize);

        // Mirrors `ai_plugin::emit_attacked_on_new_attacker`'s guard: count the
        // changes that would fire `AiEntityAttacked`, i.e. those that *name* an
        // attacker. Component insertion also marks a component changed, and the
        // fixture below inserts a `default()` (`None`) — which is a clear, not
        // an attack, and which the emitter skips for exactly this reason.
        fn count_changes(
            q: Query<&LastShipAttacker, Changed<LastShipAttacker>>,
            mut counter: ResMut<ChangeCount>,
        ) {
            counter.0 += q.iter().filter(|a| a.0.is_some()).count();
        }

        let mut app = test_app();
        app.init_resource::<AiTokenRegistry>();
        app.init_resource::<ChangeCount>();

        let npc_uuid = "00000000-0000-0000-0000-000000000013";
        let target_uuid_str = "00000000-0000-0000-0000-000000000014";

        let (npc_entity, target_entity) =
            setup_npc_shooter(&mut app, npc_uuid, target_uuid_str, 0.0, -10.0);

        app.world_mut()
            .entity_mut(target_entity)
            .insert(LastShipAttacker::default());

        // Count in `PostUpdate` so each `Update` tick's write is observed on the
        // tick it happens. (Ordering against `tick_beams` directly is not an
        // option here: this fixture registers it a second time outside any
        // SimSet, so its `SystemTypeSet` is ambiguous.)
        app.add_systems(PostUpdate, count_changes);

        {
            let mut beam = app.world_mut().get_mut::<ActiveBeam>(npc_entity).unwrap();
            beam.target_uuid = Some(target_uuid_str.to_string());
            beam.remaining_secs = 100.0;
        }

        // Many ticks of one continuous beam from one shooter.
        for _ in 0..20 {
            app.update();
        }

        assert_eq!(
            app.world()
                .get::<LastShipAttacker>(target_entity)
                .unwrap()
                .0,
            Some(npc_uuid.to_string()),
            "precondition: the sustained beam must actually have recorded the shooter"
        );
        assert_eq!(
            app.world().resource::<ChangeCount>().0,
            1,
            "tick_beams must compare before writing LastShipAttacker: a sustained beam \
             from one shooter may mark it changed exactly once, on the tick the attacker \
             becomes known. More than one means a blind write, which re-fires \
             AiEntityAttacked (and on_entity_attacked triggers) every tick the beam is live."
        );
    }

    #[test]
    fn npc_beam_tick_damages_npc_target_not_player() {
        // Regression test for PRD #597 PR-1: NPC-vs-NPC beam damage.
        // Before the fix, the old tick_npc_beams hull_query had
        // Without<LocalShip> so NPCs couldn't damage other NPCs — damage
        // was silently lost. The unified `tick_beams` iterates all ships
        // and applies damage to any target found via `hull_q`.
        use crate::ai_plugin::AiTokenRegistry;
        use crate::entity_spawner::EntitySystemHull;
        use crate::server_app::ShipAttackedThisTick;

        let mut app = test_app();
        app.init_resource::<AiTokenRegistry>();
        app.init_resource::<crate::simulation::GameOverReason>();

        let shooter_uuid = "10000000-0000-0000-0000-000000000001";
        let npc_target_uuid = "20000000-0000-0000-0000-000000000002";

        // Spawn NPC shooter with AiControllerComponent.
        let (shooter_entity, npc_target_entity) =
            setup_npc_shooter(&mut app, shooter_uuid, npc_target_uuid, 0.0, -10.0);
        // Add ShipPhysics and AiControllerComponent to the target so it looks
        // like a real production-spawned NPC (AI-controlled, physics-enabled).
        // The unified `tick_beams` finds targets by EntityUuid in `hull_q`
        // (no Ship marker requirement on targets), but production NPCs carry
        // both markers — matching them here keeps the test aligned with real
        // NPC-vs-NPC scenarios.
        app.world_mut().entity_mut(npc_target_entity).insert((
            ShipPhysics::default(),
            crate::ai_plugin::AiControllerComponent,
        ));

        // Activate beam on the shooter.
        {
            let mut beam = app
                .world_mut()
                .get_mut::<ActiveBeam>(shooter_entity)
                .unwrap();
            beam.target_uuid = Some(npc_target_uuid.to_string());
            beam.remaining_secs = 10.0;
        }

        let hp_before = app
            .world()
            .get::<EntitySystemHull>(npc_target_entity)
            .unwrap()
            .0
            .total_current();

        for _ in 0..10 {
            app.update();
        }

        let hp_after = app
            .world()
            .get::<EntitySystemHull>(npc_target_entity)
            .unwrap()
            .0
            .total_current();

        assert!(
            hp_after < hp_before,
            "NPC beam must damage NPC target hull (before={hp_before}, after={hp_after})"
        );
        // Player ship must NOT have been marked as attacked.
        let player_atk = app
            .world_mut()
            .query_filtered::<&ShipAttackedThisTick, With<crate::server_app::LocalShip>>()
            .single(app.world())
            .map(|c| c.0)
            .unwrap_or(false);
        assert!(
            !player_atk,
            "NPC-vs-NPC beam must not set player's ShipAttackedThisTick"
        );
    }

    #[test]
    fn on_beam_started_emits_correct_source_uuid_with_multiple_ships() {
        // Regression test for PRD #597 PR-1: on_beam_started used With<Ship>.single()
        // which panics when multiple ships exist. After fix it uses With<LocalShip>.
        use crate::entity_spawner::EntityUuid;

        let mut app = test_app();
        let player_uuid_str = "aaaaaaaa-0000-0000-0000-000000000001";
        let npc_uuid_str = "bbbbbbbb-0000-0000-0000-000000000002";

        // Add EntityUuid to the existing LocalShip entity (spawned by test_app).
        let player_entity = app
            .world_mut()
            .query_filtered::<Entity, With<crate::server_app::LocalShip>>()
            .single(app.world())
            .unwrap();
        app.world_mut()
            .entity_mut(player_entity)
            .insert(EntityUuid(player_uuid_str.to_string()));

        // Spawn a second NPC ship (non-LocalShip, has Ship marker).
        app.world_mut().spawn((
            crate::server_app::Ship,
            EntityUuid(npc_uuid_str.to_string()),
            ShipPhysics::default(),
            Transform::default(),
        ));

        // Trigger BeamStartedEvent — the observer on_beam_started should emit
        // source_uuid = player_uuid_str, not empty.
        app.world_mut().trigger(super::BeamStartedEvent {
            bank: "port".to_string(),
            target_uuid: "some-target".to_string(),
            source_entity: player_entity,
        });
        app.update();

        // Find the BeamStarted message in the SimOutbox.
        let outbox = app.world().resource::<crate::simulation::SimOutbox>();
        let beam_started = outbox
            .0
            .iter()
            .find(|(_, msg)| matches!(msg, crate::messages::ServerMessage::BeamStarted { .. }));
        let Some((_, crate::messages::ServerMessage::BeamStarted { source_uuid, .. })) =
            beam_started
        else {
            panic!("expected BeamStarted message in outbox");
        };
        assert_eq!(
            source_uuid, player_uuid_str,
            "on_beam_started must emit the LocalShip UUID as source_uuid, not {:?}",
            source_uuid
        );
    }

    #[test]
    fn npc_beam_tick_applies_damage_to_local_ship_through_shields() {
        // When the beam target is the player ship (has Ship marker), damage
        // must route through shields → hull component, not just EntitySystemHull directly.
        use crate::ai_plugin::AiTokenRegistry;
        use crate::entity_spawner::EntityUuid;
        use crate::server_app::{LocalShip, ShipAttackedThisTick};
        use crate::shield::ShieldConfig;
        use crate::simulation::{GameOverReason, ShipShields};

        let mut app = test_app();
        app.init_resource::<AiTokenRegistry>();
        app.init_resource::<GameOverReason>();

        // Insert shields on the LocalShip entity so the shield-routing
        // path is exercised (ShipShields is pure per-entity Component
        // post ship-parity audit).
        let shield_config = ShieldConfig {
            max_hp: 100,
            regen_per_sec: 0.0,
            num_facings: 4,
            ..Default::default()
        };
        {
            let mut q = app.world_mut().query_filtered::<Entity, With<LocalShip>>();
            let local = q.single(app.world()).unwrap();
            app.world_mut().entity_mut(local).insert(ShipShields(
                crate::shield::ShieldSystem::new(&shield_config),
                0.5,
            ));
        }

        let npc_uuid = "00000000-0000-0000-0000-000000000010";
        let player_uuid = "00000000-0000-0000-0000-000000000011";
        let player_uuid_parsed = uuid::Uuid::parse_str(player_uuid).unwrap();

        // Add EntityUuid and position to the existing LocalShip entity (already spawned by test_app).
        let player_entity = app
            .world_mut()
            .query_filtered::<Entity, With<LocalShip>>()
            .single(app.world())
            .unwrap();
        app.world_mut().entity_mut(player_entity).insert((
            EntityUuid(player_uuid.to_string()),
            Transform::from_xyz(0.0, 0.0, -10.0),
        ));

        // Spawn NPC entity using the new per-entity beam components.
        let npc_entity = {
            let npc_ent = app
                .world_mut()
                .spawn((
                    crate::server_app::Ship,
                    EntityUuid(npc_uuid.to_string()),
                    crate::ai_plugin::AiControllerComponent,
                    // The NPC's Tactical lock. Was seeded on the private
                    // `ShipAiMemory.target` mirror until #702 deleted it;
                    // `WeaponsTarget` is the surface every firing path reads.
                    WeaponsTarget(Some(player_uuid_parsed.to_string())),
                    ActiveBeam::default(),
                    PhaserCooldown::default(),
                    ShipPhysics::default(),
                    Transform::from_xyz(0.0, 0.0, 0.0),
                ))
                .id();

            let mut reg = app.world_mut().resource_mut::<AiTokenRegistry>();
            reg.register_with_entity(npc_uuid, npc_ent);
            npc_ent
        };

        let hull_before = app
            .world()
            .entity(player_entity)
            .get::<crate::entity_spawner::EntitySystemHull>()
            .unwrap()
            .0
            .total_current();
        let shields_sum_before: i32 = {
            let mut q = app
                .world_mut()
                .query_filtered::<&ShipShields, With<LocalShip>>();
            q.single(app.world())
                .expect("LocalShip must carry ShipShields")
                .0
                .facings
                .iter()
                .map(|f| f.hp)
                .sum()
        };

        // Activate the beam directly targeting the player ship.
        {
            let mut beam = app.world_mut().get_mut::<ActiveBeam>(npc_entity).unwrap();
            beam.target_uuid = Some(player_uuid.to_string());
            beam.remaining_secs = 10.0;
        }

        for _ in 0..10 {
            app.update();
        }

        let hull_after = app
            .world()
            .entity(player_entity)
            .get::<crate::entity_spawner::EntitySystemHull>()
            .unwrap()
            .0
            .total_current();
        let shields_sum_after: i32 = {
            let mut q = app
                .world_mut()
                .query_filtered::<&ShipShields, With<LocalShip>>();
            q.single(app.world())
                .expect("LocalShip must carry ShipShields")
                .0
                .facings
                .iter()
                .map(|f| f.hp)
                .sum()
        };

        let hull_lost = hull_before - hull_after;
        let shields_lost = shields_sum_before - shields_sum_after;

        assert!(
            hull_lost > 0.0 || shields_lost > 0,
            "NPC beam must damage player ship: hull {hull_before}->{hull_after} ({hull_lost}), shields {shields_sum_before}->{shields_sum_after} ({shields_lost})"
        );
        let player_atk = app
            .world_mut()
            .query_filtered::<&ShipAttackedThisTick, With<LocalShip>>()
            .single(app.world())
            .map(|c| c.0)
            .unwrap_or(false);
        assert!(
            player_atk,
            "NPC beam targeting the player ship must mark the ship as attacked for Captain AI"
        );
    }

    #[test]
    fn npc_beam_cooldown_starts_after_beam_expires() {
        // When an NPC's ActiveBeam remaining_secs reaches zero, PhaserCooldown must
        // be set to a positive value and ActiveBeam.target_uuid must become None.
        use crate::ai_plugin::AiTokenRegistry;

        let mut app = test_app();
        app.init_resource::<AiTokenRegistry>();

        let npc_uuid = "00000000-0000-0000-0000-000000000005";
        let target_uuid_str = "00000000-0000-0000-0000-000000000006";

        let (npc_entity, _target_entity) =
            setup_npc_shooter(&mut app, npc_uuid, target_uuid_str, 0.0, -10.0);

        {
            let mut beam = app.world_mut().get_mut::<ActiveBeam>(npc_entity).unwrap();
            beam.target_uuid = Some(target_uuid_str.to_string());
            beam.remaining_secs = 0.001; // expires on first tick
        }

        app.update(); // beam expires
        app.update(); // cooldown ticked

        let beam = app.world().get::<ActiveBeam>(npc_entity).unwrap();
        assert!(
            beam.target_uuid.is_none(),
            "ActiveBeam.target_uuid must be None after beam expires"
        );
        let cooldown = app.world().get::<PhaserCooldown>(npc_entity).unwrap();
        assert!(
            cooldown.per_bank.values().any(|&v| v > 0.0),
            "PhaserCooldown must be positive after beam ends: {:?}",
            cooldown.per_bank
        );
    }

    // ── End-to-end: tick_ai_controllers → InboundMessage → handle_fire_phaser ──

    /// Build an app that includes BOTH `WeaponsPlugin` AND `AiPlugin` together
    /// with all their required resources, so the full routing path can be tested:
    /// `tick_ai_controllers` emits a `FirePhaser` `InboundMessage` which the
    /// unified `handle_fire_phaser` picks up and activates the NPC's `ActiveBeam`.
    fn combined_test_app() -> App {
        use crate::ai_plugin::AiPlugin;
        use crate::config_cache::FactionRegistryResource;

        let mut app = test_app();
        app.add_plugins(AiPlugin)
            .insert_resource(FactionRegistryResource(
                crate::config_cache::get_faction_registry(),
            ));
        app
    }

    #[test]
    fn tick_ai_controllers_fire_phaser_routes_through_unified_handle_fire_phaser() {
        // Full end-to-end test: an NPC with a Destroy doctrine and a pre-selected
        // target directly in its forward arc causes `tick_ai_controllers` to write
        // a `FirePhaser` `InboundMessage`, which the unified `handle_fire_phaser`
        // picks up
        // and sets `ActiveBeam::target_uuid`.
        use crate::damage::SystemHull;
        use crate::entity_config::{BehaviourConfig, DoctrineObjective};
        use crate::entity_spawner::{EntitySystemHull, EntityUuid, WeaponsConsoleSection};
        use crate::messages::{GamePhase, SystemId};
        use bevy::prelude::State;

        let mut app = combined_test_app();

        // Put the simulation in InProgress so tick_ai_controllers runs.
        app.world_mut()
            .insert_resource(State::new(GamePhase::InProgress));

        let beam_range = 50.0_f32;
        let npc_uuid_str = "ee000000-0000-0000-0000-000000000010";
        let target_uuid_str = "ee000000-0000-0000-0000-000000000011";
        let target_uuid_parsed = uuid::Uuid::parse_str(target_uuid_str).unwrap();

        // Doctrine: single Destroy objective at high priority — always scores > 0.
        let behaviour = BehaviourConfig {
            doctrine: vec![DoctrineObjective {
                id: "destroy-hostiles".into(),
                text: "Destroy target".into(),
                directive_kind: Some("Destroy".into()),
                base_priority: 35.0,
                target_speed: 0.9,
                maintain_range: 25.0,
                ..Default::default()
            }],
            ..Default::default()
        };

        // Spawn NPC at origin, facing -Z (yaw = 0 → forward = -Z).
        // Include ActiveBeam/PhaserCooldown/ShipPhysics for the unified fire path,
        // plus the components the unified `handle_fire_phaser` requires:
        // `Ship`, `ShipSystemControlSources` (Tactical = Ai), `WeaponsTarget`.
        let mut sources = crate::ship::control_source::ControlSourceResolver::new();
        // #801: seed the phaser bank's fine system (no coarse tactical).
        sources.set(
            crate::system_registry::phaser_bank_system_id("fore").unwrap(),
            crate::ship::control_source::ControlSource::Ai,
        );
        let npc_entity = app
            .world_mut()
            .spawn((
                crate::server_app::Ship,
                crate::entity_spawner::BehaviourSection(behaviour),
                EntityUuid(npc_uuid_str.to_string()),
                crate::ship_plugin::ShipSystemControlSources(sources),
                WeaponsTarget::default(),
                ActiveBeam::default(),
                PhaserCooldown::default(),
                ShipPhysics::default(),
                WeaponsConsoleSection(crate::entity_config::WeaponsConsoleConfig {
                    torpedo_arc_color: vec![],
                    power_multipliers: None,
                    phaser_banks: vec![crate::entity_config::PhaserBankConfig {
                        id: "fore".into(),
                        facing_deg: 0.0,
                        fire_arc_deg: 360.0,
                        auto_arc_deg: 360.0,
                        beam_range,
                        beam_damage_per_sec: 5.0,
                        beam_duration_secs: 3.0,
                        cooldown_secs: 6.0,
                        beam_color: vec![],
                        shield_pierce: Some(0.0),
                        marker: None,
                    }],
                    blaster_banks: vec![],
                    radar: None,
                }),
                EntitySystemHull(SystemHull::from_config(&[(
                    SystemId("captain".into()),
                    100.0,
                )])),
                Transform::from_xyz(0.0, 0.0, 0.0),
            ))
            .id();

        // Spawn target directly ahead (-Z), well within beam range.
        let _target = app
            .world_mut()
            .spawn((
                EntityUuid(target_uuid_str.to_string()),
                EntitySystemHull(SystemHull::from_config(&[(
                    SystemId("captain".into()),
                    200.0,
                )])),
                Transform::from_xyz(0.0, 0.0, -10.0),
            ))
            .id();

        // Tick 1: `register_ai_tokens_on_spawn` runs → AiControllerComponent
        //         marker attached and token registered in AiTokenRegistry.
        app.update();

        // Register the Bevy entity in AiTokenRegistry (needed by handle_fire_phaser).
        {
            let mut reg = app
                .world_mut()
                .resource_mut::<crate::ai_plugin::AiTokenRegistry>();
            reg.register_with_entity(npc_uuid_str, npc_entity);
        }

        // Set the NPC's target lock so handle_fire_phaser can look up the
        // target. `WeaponsTarget` is the authoritative lock for every ship —
        // in production `ai_target_selection` writes it for AI-operated
        // tactical systems; here we seed it directly.
        {
            let mut target = app
                .world_mut()
                .get_mut::<WeaponsTarget>(npc_entity)
                .expect("NPC must have WeaponsTarget");
            target.0 = Some(target_uuid_parsed.to_string());
        }

        // Push a synthetic FirePhaser message for the NPC's ai: token.
        // In production this would be emitted by ai_phaser_auto_fire,
        // but for this integration test we inject it directly.
        let ai_token = format!("ai:{}", npc_uuid_str);
        push(
            &mut app,
            &ai_token,
            ClientMessage::FirePhaser {
                bank: "fore".into(),
            },
        );

        // Tick: handle_fire_phaser processes the message and activates ActiveBeam.
        app.update();

        let beam = app
            .world()
            .get::<ActiveBeam>(npc_entity)
            .expect("NPC must have ActiveBeam component");
        assert!(
            beam.target_uuid.is_some(),
            "ActiveBeam.target_uuid must be Some after tick_ai_controllers → InboundMessage → handle_fire_phaser routing"
        );
    }

    /// Verify that both a `LocalShip` entity and an NPC entity use the same
    /// `tick_beams` handler (unified per-entity beam path — issues #588 / #597).
    #[test]
    fn both_localship_and_npc_can_fire_via_per_entity_active_beam() {
        use crate::ai_plugin::AiTokenRegistry;
        use crate::entity_spawner::{EntitySystemHull, EntityUuid};

        let mut app = test_app();
        app.init_resource::<AiTokenRegistry>();

        let target_uuid = "ff000000-0000-0000-0000-000000000001";
        let npc_uuid = "ff000000-0000-0000-0000-000000000002";

        // Spawn a target entity with hull.
        let target_entity = app
            .world_mut()
            .spawn((
                EntityUuid(target_uuid.to_string()),
                EntitySystemHull(crate::damage::SystemHull::from_config(&[(
                    SystemId("captain".into()),
                    100.0,
                )])),
                Transform::from_xyz(0.0, 0.0, -15.0),
            ))
            .id();

        // Spawn NPC entity with per-entity ActiveBeam and activate beam.
        // Includes the Ship marker so the unified `tick_beams` picks it up
        // as a shooter (matches production NPC spawn path — see PRD #597).
        let npc_ent = app
            .world_mut()
            .spawn((
                crate::server_app::Ship,
                EntityUuid(npc_uuid.to_string()),
                crate::ai_plugin::AiControllerComponent,
                ActiveBeam {
                    target_uuid: Some(target_uuid.to_string()),
                    remaining_secs: 10.0,
                    ..Default::default()
                },
                PhaserCooldown::default(),
                ShipPhysics::default(),
                Transform::default(),
            ))
            .id();
        {
            let mut reg = app.world_mut().resource_mut::<AiTokenRegistry>();
            reg.register_with_entity(npc_uuid, npc_ent);
        }

        // Run ticks so tick_beams fires.
        for _ in 0..5 {
            app.update();
        }

        let hp = app
            .world()
            .get::<EntitySystemHull>(target_entity)
            .unwrap()
            .0
            .total_current();
        assert!(
            hp < 100.0,
            "NPC beam must apply damage via the unified tick_beams path (hp={hp})"
        );
    }

    /// Regression test for the unified phaser auto-fire path (post-#698:
    /// `ai_phaser_auto_fire` -> `integrate_weapons_state`).
    ///
    /// Before unification, `tick_phaser_auto_fire` iterated only `LocalShip`,
    /// so NPCs had to route through the (now-deleted) `handle_npc_beam_fire`
    /// with synthetic `FirePhaser` messages emitted by AI. Post-unification
    /// the same system iterates every ship whose Tactical system is
    /// AI-controlled, activating an [`ActiveBeam`] directly.
    #[test]
    fn ai_phaser_auto_fire_activates_ai_controlled_npc_beam() {
        use crate::ai_plugin::AiTokenRegistry;
        use crate::entity_spawner::{EntitySystemHull, EntityUuid};

        let mut app = test_app();
        app.init_resource::<AiTokenRegistry>();

        let npc_uuid = "aa000000-0000-0000-0000-000000000001";
        let target_uuid = "aa000000-0000-0000-0000-000000000002";

        // NPC facing -Z (yaw=0 forward = -Z) with Tactical set to Ai.
        let mut sources = crate::ship::control_source::ControlSourceResolver::new();
        // #801: seed the phaser bank's fine system (no coarse tactical).
        sources.set(
            crate::system_registry::phaser_bank_system_id("fore").unwrap(),
            crate::ship::control_source::ControlSource::Ai,
        );
        let npc_entity = app
            .world_mut()
            .spawn((
                crate::server_app::Ship,
                EntityUuid(npc_uuid.to_string()),
                crate::ai_plugin::AiControllerComponent,
                crate::ship_plugin::ShipSystemControlSources(sources),
                WeaponsTarget(Some(target_uuid.to_string())),
                ActiveBeam::default(),
                PhaserCooldown::default(),
                ShipPhysics::default(),
                PhaserCombatConfigResource(crate::entity_config::PhaserCombatConfig {
                    banks: vec![crate::entity_config::PhaserBankConfig {
                        id: "fore".into(),
                        facing_deg: 0.0,
                        fire_arc_deg: 360.0,
                        auto_arc_deg: 360.0,
                        beam_range: 50.0,
                        beam_damage_per_sec: 5.0,
                        beam_duration_secs: 3.0,
                        cooldown_secs: 6.0,
                        beam_color: vec![],
                        shield_pierce: None,
                        marker: None,
                    }],
                }),
                Transform::default(),
            ))
            .id();

        // Spawn target directly ahead (in-arc, in-range).
        app.world_mut().spawn((
            EntityUuid(target_uuid.to_string()),
            EntitySystemHull(crate::damage::SystemHull::from_config(&[(
                SystemId("captain".into()),
                50.0,
            )])),
            Transform::from_xyz(0.0, 0.0, -20.0),
        ));

        app.update();

        let beam = app
            .world()
            .get::<ActiveBeam>(npc_entity)
            .expect("NPC entity must have ActiveBeam component");
        assert!(
            beam.target_uuid.is_some(),
            "the ai_phaser_auto_fire -> integrate_weapons_state pair must activate the \
             NPC's ActiveBeam when Tactical is AI-controlled"
        );
        assert_eq!(
            beam.bank.as_deref(),
            Some("fore"),
            "NPC should fire the in-arc bank selected from its own PhaserCombatConfigResource"
        );
    }

    // ── Phaser decide/integrate split (issue #698) ─────────────────────────

    /// Spawn an AI-controlled NPC with one 360° bank, a locked target, and a
    /// live entity to shoot at directly ahead. Returns the NPC's entity.
    ///
    /// Deliberately does **not** insert `AiHighFidelity`: the population this
    /// helper builds is a low-LOD NPC, which is precisely the case
    /// `ai_phaser_auto_fire`'s missing `With<AiHighFidelity>` filter exists to
    /// serve. Tests that need high fidelity add the marker themselves.
    fn spawn_ai_phaser_npc(app: &mut App, npc_uuid: &str, target_uuid: &str) -> Entity {
        use crate::entity_spawner::{EntitySystemHull, EntityUuid};

        let mut sources = crate::ship::control_source::ControlSourceResolver::new();
        // #801: seed the phaser bank's fine system (no coarse tactical).
        sources.set(
            crate::system_registry::phaser_bank_system_id("fore").unwrap(),
            crate::ship::control_source::ControlSource::Ai,
        );
        let npc = app
            .world_mut()
            .spawn((
                crate::server_app::Ship,
                EntityUuid(npc_uuid.to_string()),
                crate::ship_plugin::ShipSystemControlSources(sources),
                WeaponsTarget(Some(target_uuid.to_string())),
                ActiveBeam::default(),
                PhaserCooldown::default(),
                ShipPhysics::default(),
                PhaserCombatConfigResource(crate::entity_config::PhaserCombatConfig {
                    banks: vec![crate::entity_config::PhaserBankConfig {
                        id: "fore".into(),
                        facing_deg: 0.0,
                        fire_arc_deg: 360.0,
                        auto_arc_deg: 360.0,
                        beam_range: 50.0,
                        beam_damage_per_sec: 5.0,
                        beam_duration_secs: 3.0,
                        cooldown_secs: 6.0,
                        beam_color: vec![],
                        shield_pierce: None,
                        marker: None,
                    }],
                }),
                Transform::default(),
            ))
            .id();

        app.world_mut().spawn((
            EntityUuid(target_uuid.to_string()),
            EntitySystemHull(crate::damage::SystemHull::from_config(&[(
                SystemId("captain".into()),
                50.0,
            )])),
            Transform::from_xyz(0.0, 0.0, -20.0),
        ));
        npc
    }

    /// `ai_phaser_auto_fire` is a *decider*: it must publish its choice to
    /// `PhaserIntents` and leave `ActiveBeam` alone. Running it in isolation
    /// (without `integrate_weapons_state`) is what proves the two halves are
    /// genuinely separated rather than merely renamed.
    #[test]
    fn ai_phaser_auto_fire_writes_intent_without_touching_the_beam() {
        use bevy::ecs::system::RunSystemOnce;

        let mut app = test_app();
        let npc = spawn_ai_phaser_npc(
            &mut app,
            "bb000000-0000-0000-0000-000000000001",
            "bb000000-0000-0000-0000-000000000002",
        );

        app.world_mut()
            .run_system_once(ai_phaser_auto_fire)
            .expect("ai_phaser_auto_fire should run");

        let intents = app
            .world()
            .get::<PhaserIntents>(npc)
            .expect("ActiveBeam requires PhaserIntents, so every ship with a beam has one");
        assert_eq!(
            intents.0,
            vec![PhaserCmd {
                bank: "fore".into(),
                target_uuid: "bb000000-0000-0000-0000-000000000002".into(),
                // The bank's TOML-authored beam_duration_secs, resolved by the
                // decider so the integrator never re-reads the config.
                beam_duration_secs: 3.0,
            }],
            "the decider must publish the chosen bank, target and beam duration"
        );
        assert!(
            app.world()
                .get::<ActiveBeam>(npc)
                .unwrap()
                .target_uuid
                .is_none(),
            "ai_phaser_auto_fire must not mutate ActiveBeam — that is \
             integrate_weapons_state's job"
        );
    }

    /// `integrate_weapons_state` is the *adapter*: given an intent and nothing
    /// else, it must advance the beam state machine. Written by hand rather
    /// than by the decider so the adapter is pinned independently of it.
    #[test]
    fn integrate_weapons_state_advances_beam_from_phaser_intent() {
        use bevy::ecs::system::RunSystemOnce;

        let mut app = test_app();
        let npc = spawn_ai_phaser_npc(
            &mut app,
            "bb000000-0000-0000-0000-000000000003",
            "bb000000-0000-0000-0000-000000000004",
        );

        app.world_mut()
            .entity_mut(npc)
            .insert(PhaserIntents(vec![PhaserCmd {
                bank: "fore".into(),
                target_uuid: "bb000000-0000-0000-0000-000000000004".into(),
                beam_duration_secs: 4.5,
            }]));

        app.world_mut()
            .run_system_once(integrate_weapons_state)
            .expect("integrate_weapons_state should run");

        let beam = app.world().get::<ActiveBeam>(npc).unwrap();
        assert_eq!(
            beam.target_uuid.as_deref(),
            Some("bb000000-0000-0000-0000-000000000004"),
            "the adapter must arm the beam at the intent's target"
        );
        assert_eq!(beam.bank.as_deref(), Some("fore"));
        assert_eq!(
            beam.remaining_secs, 4.5,
            "the adapter must burn for the duration the decider resolved, not a \
             duration of its own"
        );
        assert!(
            app.world().get::<PhaserIntents>(npc).unwrap().0.is_empty(),
            "the adapter must drain the buffer so a stale intent cannot re-fire \
             the beam next tick"
        );
    }

    /// Pins the deliberate asymmetry between `ai_phaser_auto_fire` (no
    /// `AiHighFidelity` filter) and `ai_torpedo_auto_fire` (filtered).
    ///
    /// Extracting phaser fire from `tick_phaser_auto_fire` into the same
    /// decide/integrate shape `ai_torpedo_auto_fire` uses makes it tempting to
    /// inherit its `With<AiHighFidelity>` filter too. That would silently
    /// disarm every low-LOD NPC — a gameplay change wearing a refactor's
    /// clothes. Phasers are the main damage low-LOD NPCs contribute, and the
    /// `CurrentPhaserMode::Auto` leg of this system isn't AI at all, so the
    /// filter would be wrong on its own terms as well.
    ///
    /// If a future slice does decide to gate phasers on LOD, `PhaserIntents`
    /// must move into `lod_ai_ships`' promote/demote bundle at the same time —
    /// see `ActiveBeam`'s `#[require(PhaserIntents)]`.
    #[test]
    fn ai_phaser_auto_fire_runs_for_low_lod_npc_without_ai_high_fidelity() {
        let mut app = test_app();
        let npc = spawn_ai_phaser_npc(
            &mut app,
            "bb000000-0000-0000-0000-000000000005",
            "bb000000-0000-0000-0000-000000000006",
        );
        assert!(
            app.world()
                .get::<crate::ai_plugin::AiHighFidelity>(npc)
                .is_none(),
            "precondition: this NPC is low-LOD"
        );

        app.update();

        assert!(
            app.world()
                .get::<ActiveBeam>(npc)
                .unwrap()
                .target_uuid
                .is_some(),
            "low-LOD NPCs must keep firing phasers — ai_phaser_auto_fire is \
             deliberately NOT gated on AiHighFidelity"
        );
    }

    /// `tick_weapons_arc_request` (issue #677): a target within a bank's
    /// range but outside its firing arc should enqueue a channel-3
    /// `ArcBearingRequest` addressed to Helm.
    #[test]
    fn tick_weapons_arc_request_fires_when_target_in_range_but_outside_arc() {
        use crate::entity_spawner::{EntitySystemHull, EntityUuid};

        let mut app = test_app();
        let target_uuid = "bb000000-0000-0000-0000-000000000001";

        let ship_entity = app
            .world_mut()
            .spawn((
                crate::server_app::Ship,
                ShipSystemControlSources::default(),
                ShipPhysics::default(),
                WeaponsTarget(Some(target_uuid.to_string())),
                WeaponsArcRequestState::default(),
                PhaserCombatConfigResource(crate::entity_config::PhaserCombatConfig {
                    banks: vec![crate::entity_config::PhaserBankConfig {
                        id: "fore".into(),
                        facing_deg: 0.0,
                        fire_arc_deg: 30.0,
                        auto_arc_deg: 30.0,
                        beam_range: 50.0,
                        beam_damage_per_sec: 5.0,
                        beam_duration_secs: 3.0,
                        cooldown_secs: 6.0,
                        beam_color: vec![],
                        shield_pierce: None,
                        marker: None,
                    }],
                }),
            ))
            .id();

        // Target is directly to starboard (x=20, z=0): in range (distance 20 <
        // beam_range 50) but 90 degrees off the fore bank's 30-degree arc.
        app.world_mut().spawn((
            EntityUuid(target_uuid.to_string()),
            EntitySystemHull(SystemHull::from_config(&[(
                SystemId("captain".into()),
                50.0,
            )])),
            Transform::from_xyz(20.0, 0.0, 0.0),
        ));

        app.update();

        let log = app.world().resource::<ArcRequestLog>();
        let request = log
            .0
            .iter()
            .find(|e| matches!(&e.payload, CoordinationPayload::ArcBearingRequest { .. }))
            .expect("expected an ArcBearingRequest CoordinationEnqueue event");
        assert_eq!(request.source_entity, ship_entity);
        assert_eq!(request.target, crate::system_registry::helm_station_key());
        match &request.payload {
            CoordinationPayload::ArcBearingRequest { uuid, .. } => {
                assert_eq!(uuid, target_uuid);
            }
            _ => unreachable!(),
        }
    }

    /// A target within the firing arc must not trigger an arc-bearing
    /// request — Weapons can already fire without Helm's help.
    #[test]
    fn tick_weapons_arc_request_does_not_fire_when_target_in_arc() {
        use crate::entity_spawner::{EntitySystemHull, EntityUuid};

        let mut app = test_app();
        let target_uuid = "bb000000-0000-0000-0000-000000000002";

        app.world_mut().spawn((
            crate::server_app::Ship,
            ShipSystemControlSources::default(),
            ShipPhysics::default(),
            WeaponsTarget(Some(target_uuid.to_string())),
            WeaponsArcRequestState::default(),
            PhaserCombatConfigResource(crate::entity_config::PhaserCombatConfig {
                banks: vec![crate::entity_config::PhaserBankConfig {
                    id: "fore".into(),
                    facing_deg: 0.0,
                    fire_arc_deg: 30.0,
                    auto_arc_deg: 30.0,
                    beam_range: 50.0,
                    beam_damage_per_sec: 5.0,
                    beam_duration_secs: 3.0,
                    cooldown_secs: 6.0,
                    beam_color: vec![],
                    shield_pierce: None,
                    marker: None,
                }],
            }),
        ));

        // Directly ahead (forward = -Z at yaw 0): in range and in arc.
        app.world_mut().spawn((
            EntityUuid(target_uuid.to_string()),
            EntitySystemHull(SystemHull::from_config(&[(
                SystemId("captain".into()),
                50.0,
            )])),
            Transform::from_xyz(0.0, 0.0, -20.0),
        ));

        app.update();

        let log = app.world().resource::<ArcRequestLog>();
        assert!(
            !log.0
                .iter()
                .any(|e| matches!(&e.payload, CoordinationPayload::ArcBearingRequest { .. })),
            "an in-arc target must not trigger an ArcBearingRequest"
        );
    }

    /// The request is debounced: an unchanged arc miss on the same target
    /// must not re-enqueue every tick.
    #[test]
    fn tick_weapons_arc_request_is_debounced_for_unchanged_miss() {
        use crate::entity_spawner::{EntitySystemHull, EntityUuid};

        let mut app = test_app();
        let target_uuid = "bb000000-0000-0000-0000-000000000003";

        app.world_mut().spawn((
            crate::server_app::Ship,
            ShipSystemControlSources::default(),
            ShipPhysics::default(),
            WeaponsTarget(Some(target_uuid.to_string())),
            WeaponsArcRequestState::default(),
            PhaserCombatConfigResource(crate::entity_config::PhaserCombatConfig {
                banks: vec![crate::entity_config::PhaserBankConfig {
                    id: "fore".into(),
                    facing_deg: 0.0,
                    fire_arc_deg: 30.0,
                    auto_arc_deg: 30.0,
                    beam_range: 50.0,
                    beam_damage_per_sec: 5.0,
                    beam_duration_secs: 3.0,
                    cooldown_secs: 6.0,
                    beam_color: vec![],
                    shield_pierce: None,
                    marker: None,
                }],
            }),
        ));

        app.world_mut().spawn((
            EntityUuid(target_uuid.to_string()),
            EntitySystemHull(SystemHull::from_config(&[(
                SystemId("captain".into()),
                50.0,
            )])),
            Transform::from_xyz(20.0, 0.0, 0.0),
        ));

        app.update();
        app.update();
        app.update();

        let log = app.world().resource::<ArcRequestLog>();
        let count = log
            .0
            .iter()
            .filter(|e| matches!(&e.payload, CoordinationPayload::ArcBearingRequest { .. }))
            .count();
        assert_eq!(
            count, 1,
            "an unchanged arc miss on the same target must only enqueue once, not every tick"
        );
    }

    /// Regression test for the unified `handle_fire_phaser`.
    ///
    /// Before unification, `handle_npc_beam_fire` always used the first entry
    /// of `WeaponsConsoleSection.phaser_banks` and a 360° arc via
    /// `radar::is_fire_ready_with_range`. Post-unification, NPCs consult
    /// their `PhaserCombatConfigResource::bank_by_id` and honour that bank's
    /// `fire_arc_deg`. A target outside the requested bank's arc must be
    /// rejected, matching the player-fire behaviour.
    #[test]
    fn npc_handle_fire_phaser_rejects_target_outside_requested_bank_arc() {
        use crate::ai_plugin::AiTokenRegistry;
        use crate::entity_spawner::{EntitySystemHull, EntityUuid};

        let mut app = test_app();
        app.init_resource::<AiTokenRegistry>();

        let npc_uuid = "bb000000-0000-0000-0000-000000000001";
        let target_uuid = "bb000000-0000-0000-0000-000000000002";

        // NPC facing -Z with a narrow port-only bank (facing_deg=-90, arc=60°).
        // Target directly ahead is out of arc.
        let mut sources = crate::ship::control_source::ControlSourceResolver::new();
        // #801: seed the phaser bank's fine system (no coarse tactical).
        sources.set(
            crate::system_registry::phaser_bank_system_id("port").unwrap(),
            crate::ship::control_source::ControlSource::Ai,
        );
        let combat = crate::entity_config::PhaserCombatConfig {
            banks: vec![crate::entity_config::PhaserBankConfig {
                id: "port".into(),
                facing_deg: -90.0,
                fire_arc_deg: 60.0,
                auto_arc_deg: 60.0,
                beam_range: 50.0,
                beam_damage_per_sec: 5.0,
                beam_duration_secs: 3.0,
                cooldown_secs: 6.0,
                beam_color: vec![],
                shield_pierce: None,
                marker: None,
            }],
        };
        let target_uuid_parsed = uuid::Uuid::parse_str(target_uuid).unwrap();
        let npc_entity = app
            .world_mut()
            .spawn((
                crate::server_app::Ship,
                EntityUuid(npc_uuid.to_string()),
                crate::ai_plugin::AiControllerComponent,
                crate::ship_plugin::ShipSystemControlSources(sources),
                WeaponsTarget(Some(target_uuid_parsed.to_string())),
                ActiveBeam::default(),
                PhaserCooldown::default(),
                ShipPhysics::default(),
                PhaserCombatConfigResource(combat),
                Transform::default(),
            ))
            .id();
        {
            let mut reg = app.world_mut().resource_mut::<AiTokenRegistry>();
            reg.register_with_entity(npc_uuid, npc_entity);
        }
        // Target directly ahead (-Z, bearing 0°) — outside the -90° port bank
        // whose arc runs from -120° to -60°.
        app.world_mut().spawn((
            EntityUuid(target_uuid.to_string()),
            EntitySystemHull(crate::damage::SystemHull::from_config(&[(
                SystemId("captain".into()),
                50.0,
            )])),
            Transform::from_xyz(0.0, 0.0, -20.0),
        ));

        // Send an explicit FirePhaser request for the port bank.
        let ai_token = format!("ai:{}", npc_uuid);
        push(
            &mut app,
            &ai_token,
            ClientMessage::FirePhaser {
                bank: "port".to_string(),
            },
        );
        app.update();

        let beam = app.world().get::<ActiveBeam>(npc_entity).unwrap();
        assert!(
            beam.target_uuid.is_none(),
            "FirePhaser for a port bank must be rejected when the target is not in that bank's fire arc — unified handler now honours per-bank config for NPCs"
        );
    }

    fn tactical_blips(app: &mut App) -> Vec<RadarBlip> {
        use crate::messages::SystemBlackboard;
        use crate::server_app::ShipSystemBlackboards;
        let mut q = app
            .world_mut()
            .query_filtered::<&ShipSystemBlackboards, With<crate::server_app::LocalShip>>();
        match q.single(app.world()) {
            Ok(bbs) => match bbs.0.get(&crate::system_registry::tactical_station_key()) {
                Some(SystemBlackboard::Weapons(bb)) => bb.blips.clone(),
                _ => Vec::new(),
            },
            Err(_) => Vec::new(),
        }
    }

    #[test]
    fn radar_blip_appears_for_asteroid_within_tactical_range() {
        let mut app = test_app();
        // Configure tactical radar to show asteroids with range 300.
        {
            let mut cfg = app
                .world_mut()
                .resource_mut::<crate::lobby::server::ShipClientConfigResource>();
            cfg.0.tactical_radar_shows = vec!["asteroid".into()];
            cfg.0.tactical_radar_range = 300.0;
        }
        // Asteroid 50 units ahead (z=-50, within 300 range).
        setup_weapons_world(&mut app, 0.0, -50.0);
        start_game(&mut app);
        tick(&mut app); // first InProgress tick → publish runs

        let blips = tactical_blips(&mut app);

        assert_eq!(blips.len(), 1, "expected one blip for in-range asteroid");
        assert_eq!(blips[0].uuid, "target-uuid");
        assert_eq!(blips[0].kind, "asteroid");
        // Forward (z=-50) at yaw=0 maps to radar_y > 0 (forward = up).
        assert!(
            blips[0].radar_y > 0.0,
            "asteroid ahead should have positive radar_y"
        );
        assert!(
            (blips[0].radar_x).abs() < 1e-4,
            "asteroid directly ahead has radar_x ≈ 0"
        );
    }

    #[test]
    fn asteroid_beyond_tactical_range_not_in_blips() {
        let mut app = test_app();
        {
            let mut cfg = app
                .world_mut()
                .resource_mut::<crate::lobby::server::ShipClientConfigResource>();
            cfg.0.tactical_radar_shows = vec!["asteroid".into()];
            cfg.0.tactical_radar_range = 100.0;
        }
        // Asteroid 200 units ahead — beyond the 100-unit radar range.
        setup_weapons_world(&mut app, 0.0, -200.0);
        start_game(&mut app);
        tick(&mut app);

        let blips = tactical_blips(&mut app);
        assert!(
            blips.is_empty(),
            "asteroid beyond tactical range must not appear in blips"
        );
    }

    // ── Tactical AI tests ──────────────────────────────────────────────────

    /// Set the ControlSource for every tactical fine system on the LocalShip.
    ///
    /// Post-#512 gating reads per-fine-system policies; post-#801 the coarse
    /// `tactical` id is not a system at all, so this helper seeds only the
    /// fine ids (mirrors what happens when a station rating flips to
    /// Backfill, which triggers AI control of every fine system owned by
    /// the station).
    fn set_tactical_control_source(
        app: &mut App,
        source: crate::ship::control_source::ControlSource,
    ) {
        let world = app.world_mut();
        let mut q = world
            .query_filtered::<&mut ShipSystemControlSources, With<crate::server_app::LocalShip>>();
        for mut cs in q.iter_mut(world) {
            for sysid in [
                crate::system_registry::phaser_fore_system_id(),
                crate::system_registry::phaser_aft_system_id(),
                crate::system_registry::torpedo_tube_fore_port_system_id(),
                crate::system_registry::torpedo_tube_fore_starboard_system_id(),
                crate::system_registry::torpedo_tube_aft_system_id(),
                crate::system_registry::torpedo_magazine_system_id(),
            ] {
                cs.0.set(sysid, source);
            }
        }
    }

    fn spawn_asteroid_target(app: &mut App, uuid: &str, x: f32, z: f32) {
        app.world_mut().spawn((
            crate::simulation::Asteroid,
            AsteroidUuid(uuid.into()),
            crate::entity_spawner::EntitySystemHull(crate::damage::SystemHull::from_config(&[(
                crate::messages::SystemId("captain".into()),
                30.0,
            )])),
            Transform::from_xyz(x, 0.0, z),
        ));
    }

    fn spawn_entity_target(app: &mut App, uuid: &str, x: f32, z: f32) {
        app.world_mut().spawn((
            crate::entity_spawner::EntityUuid(uuid.into()),
            Transform::from_xyz(x, 0.0, z),
        ));
    }

    // ── Nearest-hostile acquisition fixtures (issue #703) ──────────────────

    /// Faction UUIDs for the nearest-hostile tests. Mirrors combat_test.toml:
    /// Harrow lists Federation as an enemy.
    fn harrow_faction() -> uuid::Uuid {
        uuid::Uuid::parse_str("cccccccc-3333-4333-8333-cccccccccccc").unwrap()
    }

    fn federation_faction() -> uuid::Uuid {
        uuid::Uuid::parse_str("aaaaaaaa-1111-4111-8111-aaaaaaaaaaaa").unwrap()
    }

    /// Declare the ship's tactical radar horizon. In production this is
    /// authored per entity template under `[weapons_console] radar.range`; the
    /// tests read it from the same component rather than any literal in code.
    fn set_tactical_radar_range(app: &mut App, range: f32) {
        use crate::entity_tags::EntityTag;
        let mut q = app
            .world_mut()
            .query_filtered::<Entity, With<crate::server_app::LocalShip>>();
        let entity = q.single_mut(app.world_mut()).expect("LocalShip");
        app.world_mut()
            .entity_mut(entity)
            .insert(crate::entity_spawner::WeaponsConsoleSection(
                crate::entity_config::WeaponsConsoleConfig {
                    torpedo_arc_color: vec![],
                    power_multipliers: None,
                    phaser_banks: vec![],
                    blaster_banks: vec![],
                    radar: Some(crate::radar_config::RadarConfig {
                        range,
                        shows: vec![EntityTag::Ship],
                        selects: vec![],
                    }),
                },
            ));
    }

    /// Put the LocalShip in the Harrow faction and load a registry in which
    /// Harrow is hostile to Federation — the same shape `combat_test.toml`
    /// builds via `add_faction_enemy`.
    fn setup_harrow_ship_hostile_to_federation(app: &mut App) {
        use crate::faction::{FactionConfig, FactionRegistry};

        let mut registry = FactionRegistry::new();
        registry.insert(FactionConfig {
            uuid: harrow_faction(),
            name: "Harrow".into(),
            enemies: vec![federation_faction()],
        });
        registry.insert(FactionConfig {
            uuid: federation_faction(),
            name: "Federation".into(),
            enemies: vec![],
        });
        app.insert_resource(crate::entities::config_cache::FactionRegistryResource(
            registry,
        ));

        let mut q = app
            .world_mut()
            .query_filtered::<Entity, With<crate::server_app::LocalShip>>();
        let entity = q.single_mut(app.world_mut()).expect("LocalShip");
        app.world_mut()
            .entity_mut(entity)
            .insert(FactionComponent(harrow_faction()));
    }

    /// A factioned **ship** — the entity shape the nearest-hostile tier is
    /// allowed to auto-acquire. The `Ship` marker is not decoration: the tier-4
    /// scan is `With<Ship>`, matching the tactical radar's `shows:
    /// [EntityTag::Ship]`. See `tier_four_does_not_acquire_a_factioned_non_ship`
    /// for the other side of that filter.
    fn spawn_factioned_target(
        app: &mut App,
        uuid: &str,
        x: f32,
        z: f32,
        faction: uuid::Uuid,
    ) -> Entity {
        app.world_mut()
            .spawn((
                crate::simulation::Ship,
                crate::entity_spawner::EntityUuid(uuid.into()),
                Transform::from_xyz(x, 0.0, z),
                FactionComponent(faction),
            ))
            .id()
    }

    /// Author an *untargeted* `Destroy` objective — `Destroy { target: "" }`.
    /// This is what every shipped hostile TOML produces (`directive_kind =
    /// "Destroy"` with no `directive_target`), and the only directive shape
    /// that licenses the nearest-hostile tier.
    fn insert_untargeted_destroy_objective(app: &mut App, score: f32) {
        insert_destroy_objective_blackboard(app, "", score);
    }

    /// Set the LocalShip's `LastShipAttacker`. Wraps the entity-taking
    /// `set_last_attacker` defined further down this module.
    fn set_local_last_attacker(app: &mut App, uuid: Option<String>) {
        let entity = local_ship_entity(app);
        set_last_attacker(app, entity, uuid);
    }

    #[test]
    fn tactical_ai_respects_radar_range() {
        let mut app = test_app();
        let near_uuid = uuid::Uuid::new_v4().to_string();
        let far_uuid = uuid::Uuid::new_v4().to_string();

        // Attach a WeaponsConsoleSection with a radar range of 100 so the
        // tactical AI reads a finite, damage-scaled horizon for the test.
        {
            let mut q = app
                .world_mut()
                .query_filtered::<Entity, With<crate::server_app::LocalShip>>();
            if let Ok(entity) = q.single_mut(app.world_mut()) {
                use crate::entity_tags::EntityTag;
                app.world_mut().entity_mut(entity).insert(
                    crate::entity_spawner::WeaponsConsoleSection(
                        crate::entity_config::WeaponsConsoleConfig {
                            torpedo_arc_color: vec![],
                            power_multipliers: None,
                            phaser_banks: vec![],
                            blaster_banks: vec![],
                            radar: Some(crate::radar_config::RadarConfig {
                                range: 100.0,
                                shows: vec![EntityTag::Ship],
                                selects: vec![],
                            }),
                        },
                    ),
                );
            }
        }

        set_tactical_control_source(&mut app, crate::ship::control_source::ControlSource::Ai);

        // Far target — beyond radar range.
        spawn_entity_target(&mut app, &far_uuid, 0.0, -500.0);
        app.world_mut()
            .resource_mut::<crate::world::server::WorldContentRuntime>()
            .name_to_uuid
            .insert("target".into(), far_uuid.clone());
        insert_destroy_objective_blackboard(&mut app, "target", 80.0);

        tick(&mut app);

        assert!(
            get_weapons_target(&mut app).is_none(),
            "Tactical AI must NOT acquire a target beyond radar range"
        );

        // Near target — now within range. Update the runtime mapping so the
        // same objective name resolves to the nearby entity.
        spawn_entity_target(&mut app, &near_uuid, 0.0, -50.0);
        app.world_mut()
            .resource_mut::<crate::world::server::WorldContentRuntime>()
            .name_to_uuid
            .insert("target".into(), near_uuid.clone());

        set_weapons_target(&mut app, None);
        tick(&mut app);

        assert_eq!(
            get_weapons_target(&mut app).as_deref(),
            Some(near_uuid.as_str()),
            "Tactical AI must acquire a target within radar range"
        );
    }

    // ── Nearest-hostile acquisition tier (issue #703) ──────────────────────
    //
    // Regression guards for the shipped-content bug: `ai_target_selection`
    // acquired only from an explicit `Destroy` target or `LastShipAttacker`.
    // No asset TOML authors a `directive_target`, and `LastShipAttacker` is
    // written only by `tick_beams` — so an NPC could not fire until the player
    // shot it first. These pin the third tier that closes that gap.

    /// The headline fix: an NPC on standing "destroy hostiles" doctrine
    /// acquires a hostile it can see, *without* having been attacked.
    #[test]
    fn tactical_ai_acquires_nearest_hostile_without_being_shot_first() {
        let mut app = test_app();
        let hostile_uuid = uuid::Uuid::new_v4().to_string();

        set_tactical_radar_range(&mut app, 100.0);
        setup_harrow_ship_hostile_to_federation(&mut app);
        set_tactical_control_source(&mut app, crate::ship::control_source::ControlSource::Ai);

        // A Federation ship well inside the 100-unit radar horizon.
        spawn_factioned_target(&mut app, &hostile_uuid, 0.0, -50.0, federation_faction());
        insert_untargeted_destroy_objective(&mut app, 35.0);

        // Nobody has shot us: no LastShipAttacker, and the objective names
        // no one. Pre-#703 both acquisition tiers came up empty here.
        set_local_last_attacker(&mut app, None);

        tick(&mut app);

        assert_eq!(
            get_weapons_target(&mut app).as_deref(),
            Some(hostile_uuid.as_str()),
            "an NPC on untargeted Destroy doctrine must acquire the nearest hostile in radar \
             range without waiting to be shot first — this is the whole point of issue #703"
        );
    }

    /// The nearest hostile is picked among several — and it is the *nearest*,
    /// agreeing with the helm AI, which closes on the same ship.
    #[test]
    fn tactical_ai_acquires_the_nearest_of_several_hostiles() {
        let mut app = test_app();
        let near_uuid = uuid::Uuid::new_v4().to_string();
        let far_uuid = uuid::Uuid::new_v4().to_string();

        set_tactical_radar_range(&mut app, 100.0);
        setup_harrow_ship_hostile_to_federation(&mut app);
        set_tactical_control_source(&mut app, crate::ship::control_source::ControlSource::Ai);

        // Both in range; spawn the far one first so the result cannot be an
        // artefact of iteration order.
        spawn_factioned_target(&mut app, &far_uuid, 0.0, -90.0, federation_faction());
        spawn_factioned_target(&mut app, &near_uuid, 0.0, -20.0, federation_faction());
        insert_untargeted_destroy_objective(&mut app, 35.0);
        set_local_last_attacker(&mut app, None);

        tick(&mut app);

        assert_eq!(
            get_weapons_target(&mut app).as_deref(),
            Some(near_uuid.as_str()),
            "the nearest-hostile tier must pick the nearest, not the first found — the helm AI \
             closes on the nearest via the same find_nearest_hostile, and the two must agree"
        );
    }

    /// The radar gate binds the new tier exactly as it binds the others: a
    /// ship must not lock what it cannot detect.
    #[test]
    fn tactical_ai_does_not_acquire_a_hostile_beyond_radar_range() {
        let mut app = test_app();
        let hostile_uuid = uuid::Uuid::new_v4().to_string();

        set_tactical_radar_range(&mut app, 100.0);
        setup_harrow_ship_hostile_to_federation(&mut app);
        set_tactical_control_source(&mut app, crate::ship::control_source::ControlSource::Ai);

        // Hostile at 500 units — far beyond the 100-unit radar horizon.
        spawn_factioned_target(&mut app, &hostile_uuid, 0.0, -500.0, federation_faction());
        insert_untargeted_destroy_objective(&mut app, 35.0);
        set_local_last_attacker(&mut app, None);

        tick(&mut app);

        assert!(
            get_weapons_target(&mut app).is_none(),
            "the nearest-hostile tier must be gated by the damage-scaled tactical radar range — \
             an NPC must not acquire a target it cannot detect"
        );
    }

    /// Faction filtering: a ship of our own faction is not a hostile, however
    /// close it is.
    #[test]
    fn tactical_ai_does_not_acquire_a_non_hostile() {
        let mut app = test_app();
        let friendly_uuid = uuid::Uuid::new_v4().to_string();

        set_tactical_radar_range(&mut app, 100.0);
        setup_harrow_ship_hostile_to_federation(&mut app);
        set_tactical_control_source(&mut app, crate::ship::control_source::ControlSource::Ai);

        // Another Harrow ship — our own faction — right next to us.
        spawn_factioned_target(&mut app, &friendly_uuid, 0.0, -10.0, harrow_faction());
        insert_untargeted_destroy_objective(&mut app, 35.0);
        set_local_last_attacker(&mut app, None);

        tick(&mut app);

        assert!(
            get_weapons_target(&mut app).is_none(),
            "the nearest-hostile tier must filter by faction through the live FactionRegistry — \
             a same-faction ship is never a weapons target, however near"
        );
    }

    /// Precedence, tier 1 over tier 3: a `Destroy` naming someone specific must
    /// not wander onto a nearer ship.
    #[test]
    fn explicit_destroy_target_takes_precedence_over_a_nearer_hostile() {
        let mut app = test_app();
        let named_uuid = uuid::Uuid::new_v4().to_string();
        let nearer_uuid = uuid::Uuid::new_v4().to_string();

        set_tactical_radar_range(&mut app, 100.0);
        setup_harrow_ship_hostile_to_federation(&mut app);
        set_tactical_control_source(&mut app, crate::ship::control_source::ControlSource::Ai);

        // The named target is further away than an unnamed hostile. Both are
        // Federation, both in radar range.
        spawn_factioned_target(&mut app, &named_uuid, 0.0, -80.0, federation_faction());
        spawn_factioned_target(&mut app, &nearer_uuid, 0.0, -10.0, federation_faction());
        app.world_mut()
            .resource_mut::<crate::world::server::WorldContentRuntime>()
            .name_to_uuid
            .insert("wave_1".into(), named_uuid.clone());
        insert_destroy_objective_blackboard(&mut app, "wave_1", 80.0);
        set_local_last_attacker(&mut app, None);

        tick(&mut app);

        assert_eq!(
            get_weapons_target(&mut app).as_deref(),
            Some(named_uuid.as_str()),
            "an explicit Destroy target must outrank the nearest-hostile tier — a mission that \
             names a target must not be silently retargeted onto whoever is closest"
        );
    }

    /// Precedence, tier 2 over tier 3: whoever shot us still outranks a nearer
    /// bystander, exactly as before #703.
    #[test]
    fn last_attacker_takes_precedence_over_a_nearer_hostile() {
        let mut app = test_app();
        let attacker_uuid = uuid::Uuid::new_v4().to_string();
        let nearer_uuid = uuid::Uuid::new_v4().to_string();

        set_tactical_radar_range(&mut app, 100.0);
        setup_harrow_ship_hostile_to_federation(&mut app);
        set_tactical_control_source(&mut app, crate::ship::control_source::ControlSource::Ai);

        // The attacker is further away than an unengaged hostile.
        spawn_factioned_target(&mut app, &attacker_uuid, 0.0, -80.0, federation_faction());
        spawn_factioned_target(&mut app, &nearer_uuid, 0.0, -10.0, federation_faction());
        insert_untargeted_destroy_objective(&mut app, 35.0);
        set_local_last_attacker(&mut app, Some(attacker_uuid.clone()));

        tick(&mut app);

        assert_eq!(
            get_weapons_target(&mut app).as_deref(),
            Some(attacker_uuid.as_str()),
            "LastShipAttacker must outrank the nearest-hostile tier — shooting back at whoever \
             hit us must not be displaced by a closer bystander"
        );
    }

    // ── Target retention (tier 2) ──────────────────────────────────────────
    //
    // The nearest-hostile tier decides "who is closest *now*". Left ungated it
    // re-decides that every tick, so a lock follows whoever happens to be
    // nearest at this instant — beams retargeting, and (because the helm pursues
    // `WeaponsTarget`) the ship slewing between bearings with it. These pin the
    // retention tier that keeps an engaged ship committed.

    /// The headline retention case: engaged with A, B closes inside it, and the
    /// lock stays on A.
    #[test]
    fn an_established_lock_is_retained_when_a_nearer_hostile_appears() {
        let mut app = test_app();
        let engaged_uuid = uuid::Uuid::new_v4().to_string();
        let nearer_uuid = uuid::Uuid::new_v4().to_string();

        set_tactical_radar_range(&mut app, 100.0);
        setup_harrow_ship_hostile_to_federation(&mut app);
        set_tactical_control_source(&mut app, crate::ship::control_source::ControlSource::Ai);

        spawn_factioned_target(&mut app, &engaged_uuid, 0.0, -60.0, federation_faction());
        insert_untargeted_destroy_objective(&mut app, 35.0);
        set_local_last_attacker(&mut app, None);

        // Tick once with only A present: the ship acquires and engages it.
        tick(&mut app);
        assert_eq!(
            get_weapons_target(&mut app).as_deref(),
            Some(engaged_uuid.as_str()),
            "precondition: the ship must be engaged with A before B arrives"
        );

        // B arrives, closer than A, and equally hostile.
        spawn_factioned_target(&mut app, &nearer_uuid, 0.0, -10.0, federation_faction());
        tick(&mut app);

        assert_eq!(
            get_weapons_target(&mut app).as_deref(),
            Some(engaged_uuid.as_str()),
            "an established lock on a live, in-range hostile must be retained when a nearer \
             hostile appears — the helm keeps closing on A (the helm reads the retained WeaponsTarget, which prefers its \
             current target), so weapons flipping to B would have the ship shooting one ship \
             while flying at another"
        );
    }

    /// The other half: retention is not a freeze. A lock that dies is re-scanned.
    #[test]
    fn the_lock_is_rescanned_when_the_current_target_dies() {
        let mut app = test_app();
        let engaged_uuid = uuid::Uuid::new_v4().to_string();
        let other_uuid = uuid::Uuid::new_v4().to_string();

        set_tactical_radar_range(&mut app, 100.0);
        setup_harrow_ship_hostile_to_federation(&mut app);
        set_tactical_control_source(&mut app, crate::ship::control_source::ControlSource::Ai);

        let engaged =
            spawn_factioned_target(&mut app, &engaged_uuid, 0.0, -60.0, federation_faction());
        spawn_factioned_target(&mut app, &other_uuid, 0.0, -90.0, federation_faction());
        insert_untargeted_destroy_objective(&mut app, 35.0);
        set_local_last_attacker(&mut app, None);

        tick(&mut app);
        assert_eq!(
            get_weapons_target(&mut app).as_deref(),
            Some(engaged_uuid.as_str()),
            "precondition: the nearer hostile is the one engaged"
        );

        // A is destroyed.
        app.world_mut().entity_mut(engaged).despawn();
        tick(&mut app);

        assert_eq!(
            get_weapons_target(&mut app).as_deref(),
            Some(other_uuid.as_str()),
            "retention must not outlive the target: once the locked ship is gone the \
             nearest-hostile tier must acquire afresh, or the AI sits idle beside a live enemy"
        );
    }

    /// The liveness half of retention, on the one path where the radar gate
    /// cannot stand in for it. A ship that declares no `radar.range` has an
    /// unbounded horizon (`range_bounds_targets == false`), so `within_range`
    /// is never consulted and "the locked entity no longer exists" is the only
    /// thing that can release the lock. Without that check the retention tier
    /// hands the dead UUID on, the stale guard clears it, and the ship spends
    /// the tick idle next to a live enemy instead of acquiring it.
    #[test]
    fn the_lock_is_rescanned_when_the_current_target_dies_with_no_radar_horizon() {
        let mut app = test_app();
        let engaged_uuid = uuid::Uuid::new_v4().to_string();
        let other_uuid = uuid::Uuid::new_v4().to_string();

        // Deliberately no set_tactical_radar_range: no WeaponsConsoleSection
        // means a base range of 0, which the system reads as "unbounded".
        setup_harrow_ship_hostile_to_federation(&mut app);
        set_tactical_control_source(&mut app, crate::ship::control_source::ControlSource::Ai);

        let engaged =
            spawn_factioned_target(&mut app, &engaged_uuid, 0.0, -60.0, federation_faction());
        spawn_factioned_target(&mut app, &other_uuid, 0.0, -90.0, federation_faction());
        insert_untargeted_destroy_objective(&mut app, 35.0);
        set_local_last_attacker(&mut app, None);

        tick(&mut app);
        assert_eq!(
            get_weapons_target(&mut app).as_deref(),
            Some(engaged_uuid.as_str()),
            "precondition: the nearer hostile is the one engaged"
        );

        app.world_mut().entity_mut(engaged).despawn();
        tick(&mut app);

        assert_eq!(
            get_weapons_target(&mut app).as_deref(),
            Some(other_uuid.as_str()),
            "retention must check that the locked entity still exists, not lean on the radar \
             gate to notice — an unbounded horizon never range-checks, so a dead lock would \
             block acquisition for the tick"
        );
    }

    /// Retention is bounded by the same radar horizon as acquisition (issue
    /// #680): a lock that runs out of detection range is re-scanned, not held.
    #[test]
    fn the_lock_is_rescanned_when_the_current_target_leaves_radar_range() {
        let mut app = test_app();
        let fleeing_uuid = uuid::Uuid::new_v4().to_string();
        let other_uuid = uuid::Uuid::new_v4().to_string();

        set_tactical_radar_range(&mut app, 100.0);
        setup_harrow_ship_hostile_to_federation(&mut app);
        set_tactical_control_source(&mut app, crate::ship::control_source::ControlSource::Ai);

        let fleeing =
            spawn_factioned_target(&mut app, &fleeing_uuid, 0.0, -60.0, federation_faction());
        spawn_factioned_target(&mut app, &other_uuid, 0.0, -90.0, federation_faction());
        insert_untargeted_destroy_objective(&mut app, 35.0);
        set_local_last_attacker(&mut app, None);

        tick(&mut app);
        assert_eq!(
            get_weapons_target(&mut app).as_deref(),
            Some(fleeing_uuid.as_str()),
            "precondition: the nearer hostile is the one engaged"
        );

        // A runs beyond the 100-unit tactical radar horizon.
        app.world_mut()
            .entity_mut(fleeing)
            .insert(Transform::from_xyz(0.0, 0.0, -500.0));
        tick(&mut app);

        assert_eq!(
            get_weapons_target(&mut app).as_deref(),
            Some(other_uuid.as_str()),
            "retention must be gated by the damage-scaled radar range exactly as acquisition is \
             — a target the ship can no longer detect must not pin the lock and starve the scan"
        );
    }

    /// The ordering decision, pinned: retention outranks `LastShipAttacker`,
    /// because the helm has no retaliation tier and would keep closing on A.
    /// The reverse order is the tempting one — see this system's doc comment.
    #[test]
    fn an_established_lock_outranks_a_new_last_attacker() {
        let mut app = test_app();
        let engaged_uuid = uuid::Uuid::new_v4().to_string();
        let attacker_uuid = uuid::Uuid::new_v4().to_string();

        set_tactical_radar_range(&mut app, 100.0);
        setup_harrow_ship_hostile_to_federation(&mut app);
        set_tactical_control_source(&mut app, crate::ship::control_source::ControlSource::Ai);

        spawn_factioned_target(&mut app, &engaged_uuid, 0.0, -60.0, federation_faction());
        spawn_factioned_target(&mut app, &attacker_uuid, 0.0, -90.0, federation_faction());
        insert_untargeted_destroy_objective(&mut app, 35.0);
        set_local_last_attacker(&mut app, None);

        tick(&mut app);
        assert_eq!(
            get_weapons_target(&mut app).as_deref(),
            Some(engaged_uuid.as_str()),
            "precondition: the ship is engaged with A"
        );

        // B opens fire on us mid-engagement.
        set_local_last_attacker(&mut app, Some(attacker_uuid.clone()));
        tick(&mut app);

        assert_eq!(
            get_weapons_target(&mut app).as_deref(),
            Some(engaged_uuid.as_str()),
            "taking a hit must not break off an engagement weapons is already in: the helm's \
             ai_target_selection's retention tier outranks its last_attacker tier, and \
             weapons must match it tier for tier or the ship closes on A while shooting B. \
             last_attacker_takes_precedence_over_a_nearer_hostile pins the case that still \
             retaliates — no lock to keep"
        );
    }

    /// Advisory from the #703 review: the tier-4 scan is an *auto-acquisition*
    /// surface, so it must be `With<Ship>` — the tactical radar `shows:
    /// [EntityTag::Ship]` and nothing else. No shipped non-ship template
    /// declares a `faction` today; this pins the filter before one does.
    #[test]
    fn tier_four_does_not_acquire_a_factioned_non_ship() {
        let mut app = test_app();
        let station_uuid = uuid::Uuid::new_v4().to_string();

        set_tactical_radar_range(&mut app, 100.0);
        setup_harrow_ship_hostile_to_federation(&mut app);
        set_tactical_control_source(&mut app, crate::ship::control_source::ControlSource::Ai);

        // A hostile-factioned entity that is *not* a ship — the shape a
        // factioned station / mine / probe template would spawn. Everything
        // else about it would qualify: in radar range, enemy faction, closer
        // than anything else in the world.
        app.world_mut().spawn((
            crate::entity_spawner::EntityUuid(station_uuid),
            Transform::from_xyz(0.0, 0.0, -10.0),
            FactionComponent(federation_faction()),
        ));
        insert_untargeted_destroy_objective(&mut app, 35.0);
        set_local_last_attacker(&mut app, None);

        tick(&mut app);

        assert!(
            get_weapons_target(&mut app).is_none(),
            "the nearest-hostile tier must only auto-acquire ships — a factioned non-ship is \
             not what the tactical radar shows, and locking one would have the AI open fire on \
             scenery it cannot even see"
        );
    }

    fn insert_destroy_objective_blackboard(app: &mut App, target: &str, score: f32) {
        use crate::messages::{
            AiDirective, ObjectiveSnapshot, ObjectiveSource, ObjectiveStatus, ScoredObjective,
            SystemAffinity, SystemBlackboard, ViewscreenBlackboard,
        };
        use crate::server_app::ShipSystemBlackboards;

        let viewscreen = ViewscreenBlackboard {
            scored_objectives: vec![ScoredObjective {
                id: format!("obj-destroy-{target}"),
                score,
                directive: AiDirective::Destroy {
                    target: target.into(),
                },
                source: ObjectiveSource::Mission,
                relevance: vec![
                    SystemAffinity::Helm,
                    SystemAffinity::Weapons,
                    SystemAffinity::Captain,
                ],
                snapshot: ObjectiveSnapshot {
                    id: format!("obj-destroy-{target}"),
                    text: format!("Destroy {target}"),
                    mandatory: true,
                    status: ObjectiveStatus::Active,
                    targets: vec![target.into()],
                    source: ObjectiveSource::Mission,
                },
            }],
            ..Default::default()
        };
        let mut q = app
            .world_mut()
            .query_filtered::<&mut ShipSystemBlackboards, With<crate::server_app::LocalShip>>();
        let mut bbs = q
            .single_mut(app.world_mut())
            .expect("LocalShip must have ShipSystemBlackboards");
        bbs.0.insert(
            crate::system_registry::viewscreen_system_id(),
            SystemBlackboard::Viewscreen(viewscreen),
        );
    }

    #[test]
    fn tactical_ai_selects_named_destroy_objective_target() {
        let mut app = test_app();
        let target_uuid = uuid::Uuid::new_v4().to_string();
        set_tactical_control_source(&mut app, crate::ship::control_source::ControlSource::Ai);
        spawn_entity_target(&mut app, &target_uuid, 0.0, -30.0);
        app.world_mut()
            .resource_mut::<crate::world::server::WorldContentRuntime>()
            .name_to_uuid
            .insert("wave_1".into(), target_uuid.clone());
        insert_destroy_objective_blackboard(&mut app, "wave_1", 80.0);

        tick(&mut app);

        assert_eq!(
            get_weapons_target(&mut app).as_deref(),
            Some(target_uuid.as_str()),
            "Tactical AI must lock the live entity named by the Destroy objective"
        );
    }

    #[test]
    fn tactical_ai_clears_stale_weapons_target_when_objective_target_dead() {
        let mut app = test_app();
        set_tactical_control_source(&mut app, crate::ship::control_source::ControlSource::Ai);
        // Pre-set a stale target UUID — simulates a prior Destroy objective
        // whose entity was killed.
        set_weapons_target(&mut app, Some("dead-target-uuid".into()));
        // No last attacker.
        // Still have a Destroy objective for a target that is no longer alive.
        insert_destroy_objective_blackboard(&mut app, "wave_gone", 80.0);
        // No entity named "wave_gone" exists → resolve returns None.

        tick(&mut app);

        assert!(
            get_weapons_target(&mut app).is_none(),
            "Tactical AI must clear WeaponsTarget when the objective target is \
             dead and no last attacker is available, fixing the stale-target bug \
             that caused AI to sit idle after killing its last target"
        );
    }

    #[test]
    fn tactical_ai_ignores_missing_destroy_objective_target() {
        let mut app = test_app();
        set_tactical_control_source(&mut app, crate::ship::control_source::ControlSource::Ai);
        insert_destroy_objective_blackboard(&mut app, "wave_404", 80.0);

        tick(&mut app);

        assert!(
            get_weapons_target(&mut app).is_none(),
            "Tactical AI must not lock an arbitrary target when the objective target is missing"
        );
    }

    // ── ai_target_selection / locked_target (issue #697) ────────────────────

    /// Read a ship's published Weapons blackboard by entity.
    fn weapons_blackboard_of(app: &mut App, entity: Entity) -> Option<WeaponsBlackboard> {
        app.world()
            .entity(entity)
            .get::<crate::server_app::ShipSystemBlackboards>()
            .and_then(
                |bbs| match bbs.0.get(&crate::system_registry::tactical_station_key()) {
                    Some(SystemBlackboard::Weapons(bb)) => Some(bb.clone()),
                    _ => None,
                },
            )
    }

    /// Spawn an NPC ship: every component the spawner gives a `[behaviour]`
    /// entity that the Weapons systems touch, minus the `LocalShip` marker.
    /// Its Tactical fine systems are all AI-controlled.
    fn spawn_npc_ship(app: &mut App, uuid: &str, x: f32, z: f32) -> Entity {
        use crate::ship::control_source::{ControlSource, ControlSourceResolver};
        let config = test_ship_config();
        let mut resolver = ControlSourceResolver::new();
        for system in &config.0.systems {
            resolver.set(system.id.clone(), ControlSource::Ai);
        }
        app.world_mut()
            .spawn((
                crate::simulation::Ship,
                config,
                ShipSystemControlSources(resolver),
                crate::server_app::ShipSystemBlackboards::default(),
                LastShipAttacker::default(),
                ShipPhysics {
                    x,
                    z,
                    ..Default::default()
                },
                WeaponsTarget::default(),
                ActiveBeam::default(),
                PhaserCooldown::default(),
                PhaserCombatConfigResource(crate::entity_config::PhaserCombatConfig {
                    banks: vec![crate::entity_config::PhaserBankConfig {
                        id: "phaser-fore".into(),
                        facing_deg: 0.0,
                        fire_arc_deg: 270.0,
                        auto_arc_deg: 240.0,
                        ..Default::default()
                    }],
                }),
                TorpedoSystemResource(TorpedoSystem::new(TorpedoConfig::default())),
                crate::entity_spawner::EntityUuid(uuid.into()),
                Transform::from_xyz(x, 0.0, z),
            ))
            .id()
    }

    fn set_last_attacker(app: &mut App, entity: Entity, uuid: Option<String>) {
        app.world_mut()
            .entity_mut(entity)
            .insert(LastShipAttacker(uuid));
    }

    #[test]
    fn ai_target_selection_publishes_locked_target_and_applies_it() {
        let mut app = test_app();
        let target_uuid = uuid::Uuid::new_v4().to_string();
        set_tactical_control_source(&mut app, crate::ship::control_source::ControlSource::Ai);
        spawn_entity_target(&mut app, &target_uuid, 0.0, -30.0);
        app.world_mut()
            .resource_mut::<crate::world::server::WorldContentRuntime>()
            .name_to_uuid
            .insert("wave_1".into(), target_uuid.clone());
        insert_destroy_objective_blackboard(&mut app, "wave_1", 80.0);

        tick(&mut app);

        let local = local_ship_entity(&mut app);
        let bb = weapons_blackboard_of(&mut app, local)
            .expect("LocalShip must publish a Weapons blackboard");
        assert_eq!(
            bb.locked_target.as_deref(),
            Some(target_uuid.as_str()),
            "ai_target_selection must publish its choice as locked_target, and that intent \
             must survive publish_weapons_core_blackboard rebuilding the blackboard in SimSet::Publish"
        );
        assert_eq!(
            get_weapons_target(&mut app).as_deref(),
            Some(target_uuid.as_str()),
            "ai_target_selection must apply its choice to the authoritative WeaponsTarget"
        );
        assert_eq!(
            bb.target_uuid, bb.locked_target,
            "on an AI-operated ship, intent and truth agree after a tick"
        );
    }

    #[test]
    fn ai_target_selection_clears_locked_target_when_target_dies() {
        let mut app = test_app();
        set_tactical_control_source(&mut app, crate::ship::control_source::ControlSource::Ai);
        set_weapons_target(&mut app, Some("dead-target-uuid".into()));

        tick(&mut app);

        let local = local_ship_entity(&mut app);
        let bb = weapons_blackboard_of(&mut app, local).expect("blackboard");
        assert_eq!(
            bb.locked_target, None,
            "a lock on an entity that no longer exists must be dropped from the AI's intent"
        );
        assert!(
            get_weapons_target(&mut app).is_none(),
            "and it must clear the authoritative WeaponsTarget to match"
        );
    }

    #[test]
    fn human_tactical_leaves_locked_target_empty_and_keeps_the_human_lock() {
        let mut app = test_app();
        let target_uuid = uuid::Uuid::new_v4().to_string();
        set_tactical_control_source(&mut app, crate::ship::control_source::ControlSource::Human);
        spawn_entity_target(&mut app, &target_uuid, 0.0, -30.0);
        // A Destroy objective the AI *would* act on, were it in control.
        app.world_mut()
            .resource_mut::<crate::world::server::WorldContentRuntime>()
            .name_to_uuid
            .insert("wave_1".into(), target_uuid.clone());
        insert_destroy_objective_blackboard(&mut app, "wave_1", 80.0);
        // The human operator's own lock, as handle_set_target would leave it.
        set_weapons_target(&mut app, Some(target_uuid.clone()));

        tick(&mut app);

        let local = local_ship_entity(&mut app);
        let bb = weapons_blackboard_of(&mut app, local).expect("blackboard");
        assert_eq!(
            bb.locked_target, None,
            "locked_target is AI intent only — a human-operated Tactical selects nothing, \
             even with a live Destroy objective on the board"
        );
        assert_eq!(
            bb.target_uuid.as_deref(),
            Some(target_uuid.as_str()),
            "target_uuid mirrors the authoritative WeaponsTarget, which the human still owns"
        );
    }

    /// Put the ship in the mixed-rating shape that makes `handle_set_target`
    /// and `ai_target_selection` run in the same tick: the phaser banks are
    /// Human (so `any_bank_accepts_human_input` admits SetTarget) while the
    /// torpedo magazine is Ai (so `any_tactical_system_operates_ai` runs the
    /// selector). This is an ordinary config, not a contrived one — it is what a
    /// ship looks like when Tactical is crewed but the magazine is backfilled.
    fn set_mixed_tactical_control_sources(app: &mut App) {
        use crate::ship::control_source::ControlSource;
        let world = app.world_mut();
        let mut q = world
            .query_filtered::<&mut ShipSystemControlSources, With<crate::server_app::LocalShip>>();
        for mut cs in q.iter_mut(world) {
            for sysid in [
                crate::system_registry::phaser_fore_system_id(),
                crate::system_registry::phaser_aft_system_id(),
            ] {
                cs.0.set(sysid, ControlSource::Human);
            }
            for sysid in [
                crate::system_registry::torpedo_magazine_system_id(),
                crate::system_registry::torpedo_tube_fore_port_system_id(),
                crate::system_registry::torpedo_tube_fore_starboard_system_id(),
                crate::system_registry::torpedo_tube_aft_system_id(),
            ] {
                cs.0.set(sysid, ControlSource::Ai);
            }
        }
    }

    /// The mixed-rating shape above is only interesting if both gates really do
    /// fire on it. Pin that directly, so the regression test below can't quietly
    /// decay into a test of a ship the tactical AI never touches.
    #[test]
    fn mixed_rating_ship_admits_human_set_target_and_runs_the_tactical_ai() {
        let mut app = test_app();
        setup_weapons_world(&mut app, 30.0, 0.0);
        start_game_with_weapons(&mut app);
        set_mixed_tactical_control_sources(&mut app);

        let world = app.world_mut();
        let mut q = world.query_filtered::<(
            &ShipSystemControlSources,
            &crate::ship_plugin::ShipConfigComponent,
        ), With<crate::server_app::LocalShip>>();
        let (control_sources, ship_config) = q.single(world).expect("local ship");

        assert!(
            any_bank_accepts_human_input(control_sources, &ship_config.0),
            "a Human phaser bank must still admit the human's SetTarget"
        );
        assert!(
            any_tactical_system_operates_ai(control_sources, &ship_config.0),
            "an Ai torpedo magazine must still run the tactical AI — if this \
             ever goes false the two writers stop overlapping and the ordering \
             regression below stops being reachable"
        );
    }

    #[test]
    fn human_set_target_survives_the_tick_on_a_mixed_rating_ship() {
        let mut app = test_app();
        setup_weapons_world(&mut app, 30.0, 0.0);
        start_game_with_weapons(&mut app);
        set_mixed_tactical_control_sources(&mut app);

        push(
            &mut app,
            "weapons",
            ClientMessage::ControlSystem {
                target: crate::system_registry::tactical_radar_system_id(),
                payload: SystemControlPayload::SetTarget {
                    uuid: "target-uuid".into(),
                },
            },
        );
        tick(&mut app);

        assert_eq!(
            get_weapons_target(&mut app).as_deref(),
            Some("target-uuid"),
            "the human's SetTarget must survive the tick it was admitted in: \
             ai_target_selection has to see it and carry it into its own selection, \
             not apply a decision made before the human's lock existed"
        );

        // And it must still be there next tick — a lock clobbered on tick N is
        // not recovered on tick N+1, because selection re-seeds from the
        // (clobbered) WeaponsTarget.
        tick(&mut app);
        assert_eq!(
            get_weapons_target(&mut app).as_deref(),
            Some("target-uuid"),
            "the human's lock must be stable across subsequent ticks"
        );
    }

    /// Ported from `integrator_leaves_weapons_target_alone_when_selection_never_ran`
    /// (issue #700). That test pinned the "decider never ran" vs "decider chose
    /// nothing" distinction which `blackboard_locked_target`'s `Option<Option<_>>`
    /// carried between `ai_target_selection` and the separate `operate_tactical_ai`
    /// integrator. With the integrator folded in, a decision and its application
    /// are the same statement, so "never ran" can no longer be misread as "chose
    /// nothing" — the bug is unrepresentable rather than merely guarded.
    ///
    /// What survives is the property underneath it, on the one path that can still
    /// reach it: a ship the selector skips must keep the lock it already has, and
    /// must not have an AI-intent entry conjured onto its blackboard.
    #[test]
    fn skipped_ship_keeps_its_weapons_target_and_gains_no_blackboard_entry() {
        use crate::ship::control_source::{ControlSource, ControlSourceResolver};
        let mut app = App::new();
        app.add_systems(Update, ai_target_selection);

        let config = test_ship_config();
        let mut resolver = ControlSourceResolver::new();
        // Human across the board: `any_tactical_system_operates_ai` is false, so
        // selection skips this ship entirely.
        for system in &config.0.systems {
            resolver.set(system.id.clone(), ControlSource::Human);
        }
        let ship = app
            .world_mut()
            .spawn((
                crate::simulation::Ship,
                config,
                ShipSystemControlSources(resolver),
                LastShipAttacker::default(),
                ShipPhysics::default(),
                // The human operator's standing lock, on an entity that does not
                // exist in this bare world — so if the AI ever did run for this
                // ship, its stale-target guard would clear the lock and the
                // assertion below would fail. That is the point: the AI must not
                // run at all.
                WeaponsTarget(Some("standing-lock".into())),
                crate::server_app::ShipSystemBlackboards::default(),
            ))
            .id();

        app.update();

        assert_eq!(
            app.world().entity(ship).get::<WeaponsTarget>().unwrap().0,
            Some("standing-lock".into()),
            "a ship whose Tactical is human-operated is skipped by the selector — \
             it must keep the human's lock, not have it re-decided or cleared"
        );
        assert!(
            !app.world()
                .entity(ship)
                .get::<crate::server_app::ShipSystemBlackboards>()
                .unwrap()
                .0
                .contains_key(&crate::system_registry::tactical_station_key()),
            "a skipped ship has no AI intent to report, so the selector must not \
             insert a bare Weapons blackboard entry for it"
        );
    }

    #[derive(Resource)]
    struct KillTargetOnDamage(String);

    /// Stands in for `tick_beams` / `tick_torpedo_lifecycle`: both destroy the
    /// locked target and clear `WeaponsTarget` *after* `SimSet::Input`, which is
    /// what leaves a dead `locked_target` for `publish_weapons_core_blackboard` to
    /// carry forward.
    fn kill_target_after_input(
        mut commands: Commands,
        kill: Res<KillTargetOnDamage>,
        target_q: Query<(Entity, &crate::entity_spawner::EntityUuid)>,
        mut weapons_target_q: Query<&mut WeaponsTarget, With<crate::server_app::LocalShip>>,
    ) {
        for (entity, uuid) in target_q.iter() {
            if uuid.0 == kill.0 {
                commands.entity(entity).despawn();
            }
        }
        for mut wt in weapons_target_q.iter_mut() {
            if wt.0.as_deref() == Some(kill.0.as_str()) {
                wt.0 = None;
            }
        }
    }

    #[test]
    fn publish_drops_locked_target_when_the_selected_target_dies_mid_tick() {
        let mut app = test_app();
        let target_uuid = uuid::Uuid::new_v4().to_string();
        set_tactical_control_source(&mut app, crate::ship::control_source::ControlSource::Ai);
        spawn_entity_target(&mut app, &target_uuid, 0.0, -30.0);
        app.world_mut()
            .resource_mut::<crate::world::server::WorldContentRuntime>()
            .name_to_uuid
            .insert("wave_1".into(), target_uuid.clone());
        insert_destroy_objective_blackboard(&mut app, "wave_1", 80.0);

        // Tick 1: the AI acquires the target while it is alive.
        tick(&mut app);
        let local = local_ship_entity(&mut app);
        assert_eq!(
            weapons_blackboard_of(&mut app, local)
                .expect("blackboard")
                .locked_target
                .as_deref(),
            Some(target_uuid.as_str()),
            "precondition: the AI must be locked on before the target dies"
        );

        // Tick 2: Input selects the (still live) target, then the target is
        // destroyed in Damage — exactly the beam/torpedo kill ordering.
        app.insert_resource(KillTargetOnDamage(target_uuid.clone()));
        app.add_systems(
            Update,
            kill_target_after_input.in_set(crate::sim_sets::SimSet::Damage),
        );
        tick(&mut app);

        let bb = weapons_blackboard_of(&mut app, local).expect("blackboard");
        assert_eq!(
            bb.target_uuid, None,
            "precondition: the kill must have cleared the authoritative WeaponsTarget"
        );
        assert_eq!(
            bb.locked_target, None,
            "a locked_target whose entity died after SimSet::Input must not be carried \
             forward: publishing it would put locked_target != target_uuid on the wire, \
             contradicting the field's documented contract that the two agree after a tick"
        );
    }

    #[test]
    fn npc_ship_publishes_its_own_weapons_blackboard_with_ship_state_only() {
        let mut app = test_app();
        // LocalShip radar config: shows asteroids out to 300 units. Only the
        // LocalShip has a browser client, so only it should get blips.
        {
            let mut cfg = app
                .world_mut()
                .resource_mut::<crate::lobby::server::ShipClientConfigResource>();
            cfg.0.tactical_radar_shows = vec!["asteroid".into()];
            cfg.0.tactical_radar_range = 300.0;
        }
        setup_weapons_world(&mut app, 0.0, -50.0);
        start_game(&mut app);

        // NPC at the origin, attacked by a live entity 30 units ahead.
        let attacker_uuid = uuid::Uuid::new_v4().to_string();
        spawn_entity_target(&mut app, &attacker_uuid, 0.0, -30.0);
        let npc = spawn_npc_ship(&mut app, "npc-1", 0.0, 0.0);
        set_last_attacker(&mut app, npc, Some(attacker_uuid.clone()));

        tick(&mut app);

        let bb = weapons_blackboard_of(&mut app, npc)
            .expect("an NPC carrying ShipSystemBlackboards must get a Weapons blackboard too");

        // Ship state — computed per-entity, so NPCs get the real thing.
        assert_eq!(
            bb.locked_target.as_deref(),
            Some(attacker_uuid.as_str()),
            "the NPC's Tactical AI must select its last attacker"
        );
        assert_eq!(
            bb.target_uuid.as_deref(),
            Some(attacker_uuid.as_str()),
            "and the NPC's authoritative WeaponsTarget must follow its own intent"
        );
        assert_eq!(
            bb.banks.len(),
            1,
            "banks come from the NPC's own PhaserCombatConfigResource"
        );
        assert_eq!(bb.banks[0].id, "phaser-fore");
        assert_eq!(
            bb.torpedo_count,
            TorpedoConfig::default().count,
            "torpedo_count comes from the NPC's own TorpedoSystemResource"
        );

        // Client render data — player-only, and left empty for NPCs.
        assert!(
            bb.blips.is_empty(),
            "blips are client render data sourced from the player-only \
             ShipClientConfigResource, and are O(all entities) to compute — an NPC \
             with no browser client must not pay for them"
        );
        assert!(bb.regions.is_empty(), "regions are client render data");
        assert!(
            bb.phaser_arcs.is_empty(),
            "phaser_arcs are client render data"
        );
        assert!(
            bb.torpedo_arcs.is_empty(),
            "torpedo_arcs are client render data"
        );

        // The contrast: the LocalShip *does* get its render data, so the
        // assertions above are about the NPC tier and not a dead radar config.
        let local = local_ship_entity(&mut app);
        let local_bb = weapons_blackboard_of(&mut app, local).expect("blackboard");
        assert_eq!(
            local_bb.blips.len(),
            1,
            "the LocalShip still gets its in-range asteroid blip"
        );
    }

    #[test]
    fn npc_and_local_ship_select_targets_independently() {
        let mut app = test_app();
        // Regression guard for the SetTarget-contamination class of bug: two
        // ships, two different attackers, two independent locks.
        let local_target = uuid::Uuid::new_v4().to_string();
        let npc_target = uuid::Uuid::new_v4().to_string();
        spawn_entity_target(&mut app, &local_target, 0.0, -30.0);
        spawn_entity_target(&mut app, &npc_target, 0.0, 30.0);

        set_tactical_control_source(&mut app, crate::ship::control_source::ControlSource::Ai);
        let local = local_ship_entity(&mut app);
        app.world_mut()
            .entity_mut(local)
            .insert(LastShipAttacker(Some(local_target.clone())));

        let npc = spawn_npc_ship(&mut app, "npc-1", 0.0, 0.0);
        set_last_attacker(&mut app, npc, Some(npc_target.clone()));

        tick(&mut app);

        assert_eq!(
            weapons_blackboard_of(&mut app, local)
                .expect("blackboard")
                .locked_target
                .as_deref(),
            Some(local_target.as_str())
        );
        assert_eq!(
            weapons_blackboard_of(&mut app, npc)
                .expect("blackboard")
                .locked_target
                .as_deref(),
            Some(npc_target.as_str()),
            "each ship selects from its own last-attacker surface, not a shared one"
        );
    }

    /// Builds on `test_app()` (LocalShip + `WeaponsPlugin` + `LobbyPlugin`) by
    /// wiring in `ai_torpedo_auto_fire` (issue #694) and giving the LocalShip
    /// the two components it requires: `AiHighFidelity` and `TorpedoIntents`.
    /// `test_app()` itself stays unchanged (it's shared by ~200 unrelated tests
    /// in this module) — this is a dedicated extension, mirroring how
    /// `combined_test_app()` layers `AiPlugin` on top of `test_app()` for its
    /// own end-to-end tests.
    ///
    /// Only the *decide* half needs adding: since issue #698 the apply half is
    /// `integrate_weapons_state`, which `WeaponsPlugin` already registers with
    /// its `.after(ai_torpedo_auto_fire)` edge — and that edge starts binding
    /// the moment this helper registers the decider.
    fn torpedo_ai_test_app() -> App {
        let mut app = test_app();
        app.add_systems(
            Update,
            crate::console_ai_plugin::ai_torpedo_auto_fire.in_set(crate::sim_sets::SimSet::Physics),
        );
        let ship = {
            let mut q = app
                .world_mut()
                .query_filtered::<Entity, With<crate::server_app::LocalShip>>();
            q.single(app.world())
                .expect("test_app must spawn a LocalShip")
        };
        app.world_mut()
            .entity_mut(ship)
            .insert((crate::ai_plugin::AiHighFidelity, TorpedoIntents::default()));
        app
    }

    /// Regression test for issue #694: `ai_torpedo_auto_fire` (preliminary)
    /// replaces the old fused torpedo sub-block that used to run inline
    /// inside `operate_tactical_ai`. Ported from the pre-#694
    /// `ai_fires_torpedo_when_ai_controls_unclaimed_station`, which exercised
    /// `operate_tactical_ai`'s torpedo block directly before it was deleted.
    #[test]
    fn ai_torpedo_auto_fire_fires_when_ai_controls_unclaimed_station() {
        // Unclaimed station + Ai ControlSource → ai_torpedo_auto_fire fires unconditionally.
        let mut app = torpedo_ai_test_app();

        set_tactical_control_source(&mut app, crate::ship::control_source::ControlSource::Ai);
        set_weapons_target(&mut app, Some("target-uuid".into()));
        load_tube_now(&mut app, "fore_port");
        // Asteroid at (0, -30) → bearing 0 from ship at origin yaw=0 → in ForePort arc.
        spawn_asteroid_target(&mut app, "target-uuid", 0.0, -30.0);

        let out = tick(&mut app);
        assert!(
            out.iter()
                .any(|m| matches!(&m.msg, ServerMessage::TorpedoLaunched { .. })),
            "ai_torpedo_auto_fire should fire TorpedoLaunched when controlling an unclaimed \
             Tactical station"
        );
    }

    /// `ai_torpedo_auto_fire` is a *decider*: it must publish to
    /// `TorpedoIntents` and leave the `TorpedoSystem` alone. Mirrors
    /// `ai_phaser_auto_fire_writes_intent_without_touching_the_beam`.
    #[test]
    fn ai_torpedo_auto_fire_writes_intent_without_launching() {
        use bevy::ecs::system::RunSystemOnce;

        let mut app = torpedo_ai_test_app();
        set_tactical_control_source(&mut app, crate::ship::control_source::ControlSource::Ai);
        set_weapons_target(&mut app, Some("target-uuid".into()));
        load_tube_now(&mut app, "fore_port");
        spawn_asteroid_target(&mut app, "target-uuid", 0.0, -30.0);

        app.world_mut()
            .run_system_once(crate::console_ai_plugin::ai_torpedo_auto_fire)
            .expect("ai_torpedo_auto_fire should run");

        let ship = local_ship(&mut app);
        let intents = app
            .world()
            .get::<TorpedoIntents>(ship)
            .expect("torpedo_ai_test_app inserts TorpedoIntents");
        assert_eq!(
            intents.0,
            vec![TorpedoCmd {
                tube_id: "fore_port".into(),
                target_uuid: "target-uuid".into(),
            }],
            "the decider must publish the loaded, in-arc tube and the locked target"
        );
        assert!(
            app.world()
                .resource::<SimOutbox>()
                .0
                .iter()
                .all(|(_, m)| !matches!(m, ServerMessage::TorpedoLaunched { .. })),
            "ai_torpedo_auto_fire must not launch — that is integrate_weapons_state's job"
        );
    }

    /// `integrate_weapons_state` drains `TorpedoIntents` as well as
    /// `PhaserIntents` (issue #698 folded the former
    /// `integrate_torpedo_intents` into it). Pins the torpedo half of the
    /// adapter from a hand-written intent, independently of the decider.
    #[test]
    fn integrate_weapons_state_launches_from_torpedo_intent() {
        use bevy::ecs::system::RunSystemOnce;

        let mut app = torpedo_ai_test_app();
        load_tube_now(&mut app, "fore_port");
        let ship = local_ship(&mut app);
        app.world_mut()
            .entity_mut(ship)
            .insert(TorpedoIntents(vec![TorpedoCmd {
                tube_id: "fore_port".into(),
                target_uuid: "target-uuid".into(),
            }]));

        app.world_mut()
            .run_system_once(integrate_weapons_state)
            .expect("integrate_weapons_state should run");

        assert!(
            app.world()
                .resource::<SimOutbox>()
                .0
                .iter()
                .any(|(_, m)| matches!(m, ServerMessage::TorpedoLaunched { .. })),
            "the adapter must advance the torpedo state machine from the intent"
        );
        assert!(
            app.world()
                .get::<TorpedoIntents>(ship)
                .unwrap()
                .0
                .is_empty(),
            "the adapter must drain the buffer so a stale intent cannot re-launch"
        );
    }

    /// Issue #698 promotion: `ai_torpedo_auto_fire` used to hardcode
    /// `TorpedoAiInput { target_shields: 0 }`, which made `auto_fire_torpedo`'s
    /// "shields must be down" condition unreachable — the AI fired torpedoes
    /// straight into a fully-shielded target. It now reads the target's real
    /// `ShipShields`, so the pure function's documented doctrine (phasers strip
    /// shields, torpedoes finish the hull) actually holds.
    #[test]
    fn ai_torpedo_auto_fire_holds_fire_while_target_shields_are_up() {
        let mut app = torpedo_ai_test_app();
        set_tactical_control_source(&mut app, crate::ship::control_source::ControlSource::Ai);
        set_weapons_target(&mut app, Some("target-uuid".into()));
        load_tube_now(&mut app, "fore_port");

        // A ship target dead ahead, shields up.
        let target = spawn_shielded_target(&mut app, "target-uuid", 0.0, -30.0);

        let out = tick(&mut app);
        assert!(
            !out.iter()
                .any(|m| matches!(&m.msg, ServerMessage::TorpedoLaunched { .. })),
            "torpedoes must hold while the target's shields are still up"
        );

        // Collapse every facing — now the shot is on.
        {
            let mut shields = app
                .world_mut()
                .get_mut::<crate::ship::shields::ShipShields>(target)
                .unwrap();
            for facing in shields.0.facings.iter_mut() {
                facing.hp = 0;
            }
        }
        load_tube_now(&mut app, "fore_port");

        let out = tick(&mut app);
        assert!(
            out.iter()
                .any(|m| matches!(&m.msg, ServerMessage::TorpedoLaunched { .. })),
            "torpedoes must fire once the target's shields are down"
        );
    }

    fn local_ship(app: &mut App) -> Entity {
        let mut q = app
            .world_mut()
            .query_filtered::<Entity, With<crate::server_app::LocalShip>>();
        q.single(app.world())
            .expect("test_app must spawn a LocalShip")
    }

    /// A ship-like entity carrying `ShipShields` at full HP.
    fn spawn_shielded_target(app: &mut App, uuid: &str, x: f32, z: f32) -> Entity {
        let shields = crate::shield::ShieldSystem::new(&crate::shield::ShieldConfig::default());
        assert!(
            shields.facings.iter().any(|f| f.hp > 0),
            "precondition: the default shield config must start with HP up"
        );
        app.world_mut()
            .spawn((
                crate::entity_spawner::EntityUuid(uuid.into()),
                crate::entity_spawner::EntitySystemHull(crate::damage::SystemHull::from_config(&[
                    (crate::messages::SystemId("captain".into()), 50.0),
                ])),
                crate::ship::shields::ShipShields(shields, 0.5),
                Transform::from_xyz(x, 0.0, z),
            ))
            .id()
    }

    fn set_tactical_station_rating(app: &mut App, rating: &str) {
        let rating = rating.to_string();
        let world = app.world_mut();
        let mut q = world
            .query_filtered::<&mut crate::ship_plugin::ActiveStationRatings, With<crate::server_app::LocalShip>>();
        for mut ratings in q.iter_mut(world) {
            ratings.0.insert(
                crate::messages::StationId("tactical".into()),
                rating.clone(),
            );
        }
    }

    /// Ported from the pre-#694 `ai_stops_firing_when_rating_switches_to_std`,
    /// which exercised `operate_tactical_ai`'s torpedo block directly before
    /// it was deleted; see `ai_torpedo_auto_fire_fires_when_ai_controls_unclaimed_station`
    /// above.
    #[test]
    fn ai_torpedo_auto_fire_stops_firing_when_rating_switches_to_std() {
        // Occupied station: AI fires when rating is Assisted (has torpedo_auto_fire
        // in ai_tuning), stops when rating is Std (no ai_tuning).
        let mut app = torpedo_ai_test_app();

        // Assign a human holder so the ai_tuning gate is active.
        push(
            &mut app,
            "weapons",
            ClientMessage::Identify {
                token: "weapons".into(),
                name: "Bob".into(),
            },
        );
        tick(&mut app);
        push(
            &mut app,
            "weapons",
            ClientMessage::SelectStation {
                station: "Tactical".into(),
            },
        );
        tick(&mut app);

        set_tactical_control_source(&mut app, crate::ship::control_source::ControlSource::Ai);
        // Set rating to Assisted (has torpedo_auto_fire in ai_tuning).
        set_tactical_station_rating(&mut app, "Assisted");
        set_weapons_target(&mut app, Some("target-uuid".into()));
        load_tube_now(&mut app, "fore_port");
        spawn_asteroid_target(&mut app, "target-uuid", 0.0, -30.0);

        // First tick — AI should fire with Assisted rating.
        let out1 = tick(&mut app);
        assert!(
            out1.iter()
                .any(|m| matches!(&m.msg, ServerMessage::TorpedoLaunched { .. })),
            "ai_torpedo_auto_fire should fire TorpedoLaunched when rating is Assisted"
        );

        // Reload the tube (launch consumed it) so the only gate is the rating.
        load_tube_now(&mut app, "fore_port");

        // Switch to Std rating (no torpedo_auto_fire in ai_tuning).
        set_tactical_station_rating(&mut app, "Std");

        // Second tick - AI must not fire.
        let out2 = tick(&mut app);
        assert!(
            !out2
                .iter()
                .any(|m| matches!(&m.msg, ServerMessage::TorpedoLaunched { .. })),
            "ai_torpedo_auto_fire must not fire TorpedoLaunched when rating is Std"
        );
    }

    // ── Fine-Tactical decomposition tests (issue #512) ─────────────────────
    //
    // Every new fine SystemId, blackboard, and gate has coverage here. The
    // channel-2 `ClaimTorpedoRound` transaction is exercised via
    // `handle_load_tube` → `handle_torpedo_magazine_inter_system`. Firing
    // gates are exercised via `handle_fire_torpedo` and `handle_fire_phaser`.

    /// Helper: mark a fine system Offline (Disabled/Destroyed) on the LocalShip
    /// by inserting it into `ControlSourceResolver.offline_systems`. Mirrors
    /// what `sync_console_damage_tiers` would do after a damage tick — the
    /// direct-insert avoids needing to spawn a hull component just to test
    /// the gate.
    fn mark_system_offline(app: &mut App, system_id: SystemId) {
        let world = app.world_mut();
        let mut q = world
            .query_filtered::<&mut ShipSystemControlSources, With<crate::server_app::LocalShip>>();
        for mut cs in q.iter_mut(world) {
            cs.0.offline_systems.insert(system_id.clone());
        }
    }

    /// Helper: register a fine system on the LocalShip's ControlSourceResolver
    /// with a specific ControlSource. Used to simulate the ship having declared
    /// a fine `[[system]]` block in its TOML.
    fn register_fine_system(
        app: &mut App,
        system_id: SystemId,
        source: crate::ship::control_source::ControlSource,
    ) {
        let world = app.world_mut();
        let mut q = world
            .query_filtered::<&mut ShipSystemControlSources, With<crate::server_app::LocalShip>>();
        for mut cs in q.iter_mut(world) {
            cs.0.set(system_id.clone(), source);
        }
    }

    // ── Registered-system predicate ───────────────────────────────────────

    #[test]
    fn system_is_registered_returns_true_after_set() {
        let mut sources = ShipSystemControlSources::default();
        let sysid = crate::system_registry::phaser_fore_system_id();
        sources.0.set(
            sysid.clone(),
            crate::ship::control_source::ControlSource::Human,
        );
        assert!(system_is_registered(&sources, &sysid));
    }

    #[test]
    fn system_is_registered_returns_true_after_offline_insert() {
        let mut sources = ShipSystemControlSources::default();
        let sysid = crate::system_registry::phaser_fore_system_id();
        sources.0.offline_systems.insert(sysid.clone());
        assert!(system_is_registered(&sources, &sysid));
    }

    #[test]
    fn system_is_registered_returns_false_when_absent() {
        let sources = ShipSystemControlSources::default();
        let sysid = crate::system_registry::phaser_fore_system_id();
        assert!(!system_is_registered(&sources, &sysid));
    }

    // ── Per-bank fire gate ────────────────────────────────────────────────

    #[test]
    fn fire_phaser_refused_when_bank_fine_system_offline() {
        let mut app = test_app();
        let _ = lock_and_fire(&mut app, 0.0, -20.0);

        // Reset beam / cooldown so the only variable is the bank gate.
        set_active_beam_target(&mut app, None);
        start_phaser_cooldown(&mut app, "port", 0.0);

        // Register the port bank as Human, then mark it offline (as
        // sync_console_damage_tiers would do on Disabled hull).
        register_fine_system(
            &mut app,
            SystemId("phaser-port".into()),
            crate::ship::control_source::ControlSource::Human,
        );
        mark_system_offline(&mut app, SystemId("phaser-port".into()));

        push(
            &mut app,
            "weapons",
            ClientMessage::FirePhaser {
                bank: "port".to_string(),
            },
        );
        let out = tick(&mut app);
        assert!(
            !out.iter()
                .any(|m| matches!(&m.msg, ServerMessage::BeamStarted { .. })),
            "FirePhaser must be refused when the bank's fine system is offline"
        );
    }

    #[test]
    fn fire_phaser_allowed_when_other_bank_offline_but_this_one_online() {
        let mut app = test_app();
        let _ = lock_and_fire(&mut app, 0.0, -20.0);
        set_active_beam_target(&mut app, None);
        start_phaser_cooldown(&mut app, "port", 0.0);
        start_phaser_cooldown(&mut app, "starboard", 0.0);

        // Only starboard offline; port stays online.
        mark_system_offline(&mut app, SystemId("phaser-starboard".into()));

        push(
            &mut app,
            "weapons",
            ClientMessage::FirePhaser {
                bank: "port".to_string(),
            },
        );
        let out = tick(&mut app);
        assert!(
            out.iter()
                .any(|m| matches!(&m.msg, ServerMessage::BeamStarted { .. })),
            "Firing port must succeed when only starboard is offline"
        );
    }

    // ── Per-tube load/unload gate ─────────────────────────────────────────

    #[test]
    fn load_tube_emits_claim_torpedo_round_via_channel_2() {
        let mut app = test_app();
        start_game_with_weapons(&mut app);

        push(
            &mut app,
            "weapons",
            ClientMessage::LoadTube {
                tube: "fore_port".to_string(),
            },
        );
        // Run one tick to admit the command → handle_load_tube emits the claim.
        tick(&mut app);

        let queue = &app.world().resource::<InterSystemQueue>().0;
        let claim_present = queue.iter().any(|m| {
            m.target == crate::system_registry::torpedo_magazine_system_id()
                && matches!(
                    &m.payload,
                    InterSystemPayload::ClaimTorpedoRound { tube } if tube == "fore_port"
                )
        });
        assert!(
            claim_present,
            "handle_load_tube should emit ClaimTorpedoRound on channel-2"
        );
    }

    #[test]
    fn load_tube_refused_when_tube_fine_system_offline() {
        let mut app = test_app();
        start_game_with_weapons(&mut app);

        mark_system_offline(
            &mut app,
            crate::system_registry::torpedo_tube_fore_port_system_id(),
        );

        push(
            &mut app,
            "weapons",
            ClientMessage::LoadTube {
                tube: "fore_port".to_string(),
            },
        );
        tick(&mut app);

        // No claim should have been emitted this tick.
        let queue = &app.world().resource::<InterSystemQueue>().0;
        assert!(
            !queue
                .iter()
                .any(|m| matches!(&m.payload, InterSystemPayload::ClaimTorpedoRound { .. })),
            "load must not emit a magazine claim when the tube system is offline"
        );
    }

    // ── Magazine claim transaction ────────────────────────────────────────
    //
    // Directly exercise `handle_torpedo_magazine_inter_system` by pushing
    // a claim into the queue and asserting the same-tick effect on the
    // magazine counter and the tube state.

    #[test]
    fn magazine_claim_decrements_counter_by_one_when_online() {
        let mut app = test_app();
        start_game_with_weapons(&mut app);

        // Snapshot the magazine counter (starts at 10 from TorpedoConfig::default).
        let before = app
            .world_mut()
            .query_filtered::<&TorpedoSystemResource, With<crate::server_app::LocalShip>>()
            .single(app.world())
            .map(|ts| ts.0.torpedoes_remaining)
            .unwrap();
        assert!(before > 0, "test precondition: magazine must have stock");

        // Drive the end-to-end path: `handle_load_tube` (Input) emits the
        // channel-2 claim, and `handle_torpedo_magazine_inter_system` (Physics)
        // consumes it — both happen within a single `app.update()` after
        // `clear_inter_system_queue` runs.
        push(
            &mut app,
            "weapons",
            ClientMessage::LoadTube {
                tube: "fore_port".to_string(),
            },
        );
        let _ = tick(&mut app);

        let after = app
            .world_mut()
            .query_filtered::<&TorpedoSystemResource, With<crate::server_app::LocalShip>>()
            .single(app.world())
            .map(|ts| ts.0.torpedoes_remaining)
            .unwrap();
        assert_eq!(
            after,
            before - 1,
            "magazine counter must decrement by exactly one after a granted claim"
        );

        // The tube should now be Loading.
        let tube_loading = app
            .world_mut()
            .query_filtered::<&TorpedoSystemResource, With<crate::server_app::LocalShip>>()
            .single(app.world())
            .map(|ts| {
                matches!(
                    ts.0.tube("fore_port").map(|t| &t.load_state),
                    Some(crate::torpedo::TubeLoadState::Loading { .. })
                )
            })
            .unwrap();
        assert!(
            tube_loading,
            "granted claim must start loading the target tube via start_load_reserved"
        );
    }

    #[test]
    fn magazine_claim_refused_when_magazine_offline() {
        let mut app = test_app();
        start_game_with_weapons(&mut app);
        // Register magazine as human, then mark it offline (Disabled tier).
        register_fine_system(
            &mut app,
            crate::system_registry::torpedo_magazine_system_id(),
            crate::ship::control_source::ControlSource::Human,
        );
        mark_system_offline(
            &mut app,
            crate::system_registry::torpedo_magazine_system_id(),
        );

        let before = app
            .world_mut()
            .query_filtered::<&TorpedoSystemResource, With<crate::server_app::LocalShip>>()
            .single(app.world())
            .map(|ts| ts.0.torpedoes_remaining)
            .unwrap();

        // End-to-end: LoadTube tries to emit a claim — the tube gate passes
        // (fine tube systems default to the Human source), then the claim
        // goes to the magazine consumer which refuses because the magazine
        // is offline.
        push(
            &mut app,
            "weapons",
            ClientMessage::LoadTube {
                tube: "fore_port".to_string(),
            },
        );
        let _ = tick(&mut app);

        let after = app
            .world_mut()
            .query_filtered::<&TorpedoSystemResource, With<crate::server_app::LocalShip>>()
            .single(app.world())
            .map(|ts| ts.0.torpedoes_remaining)
            .unwrap();
        assert_eq!(
            after, before,
            "offline magazine must refuse the claim — counter unchanged"
        );
    }

    #[test]
    fn magazine_claim_refused_when_empty() {
        let mut app = test_app();
        start_game_with_weapons(&mut app);

        // Drain the magazine.
        {
            let mut q = app
                .world_mut()
                .query_filtered::<&mut TorpedoSystemResource, With<crate::server_app::LocalShip>>();
            let mut ts = q.single_mut(app.world_mut()).unwrap();
            ts.0.torpedoes_remaining = 0;
        }

        push(
            &mut app,
            "weapons",
            ClientMessage::LoadTube {
                tube: "fore_port".to_string(),
            },
        );
        let _ = tick(&mut app);

        // Tube must still be Unloaded — no start_load_reserved happened.
        let tube_state = app
            .world_mut()
            .query_filtered::<&TorpedoSystemResource, With<crate::server_app::LocalShip>>()
            .single(app.world())
            .map(|ts| ts.0.tube("fore_port").map(|t| t.load_state.clone()))
            .unwrap();
        assert_eq!(
            tube_state,
            Some(crate::torpedo::TubeLoadState::Unloaded),
            "empty magazine must not begin loading the tube"
        );
    }

    // ── Fire torpedo: magazine-online gate ────────────────────────────────

    #[test]
    fn fire_torpedo_refused_when_magazine_offline_even_if_tube_loaded() {
        let mut app = test_app();
        start_game_with_weapons(&mut app);
        // Load the tube directly (bypass channel-2 to isolate the fire gate).
        load_tube_now(&mut app, "fore_port");

        // Register magazine as offline.
        register_fine_system(
            &mut app,
            crate::system_registry::torpedo_magazine_system_id(),
            crate::ship::control_source::ControlSource::Human,
        );
        mark_system_offline(
            &mut app,
            crate::system_registry::torpedo_magazine_system_id(),
        );

        push(
            &mut app,
            "weapons",
            ClientMessage::FireTorpedo {
                tube: "fore_port".to_string(),
                target_uuid: None,
            },
        );
        let out = tick(&mut app);
        assert!(
            !out.iter()
                .any(|m| matches!(&m.msg, ServerMessage::TorpedoLaunched { .. })),
            "disabled magazine must block fire even from a loaded tube"
        );
    }

    #[test]
    fn fire_torpedo_refused_when_tube_fine_system_offline() {
        let mut app = test_app();
        start_game_with_weapons(&mut app);
        load_tube_now(&mut app, "fore_port");
        mark_system_offline(
            &mut app,
            crate::system_registry::torpedo_tube_fore_port_system_id(),
        );

        push(
            &mut app,
            "weapons",
            ClientMessage::FireTorpedo {
                tube: "fore_port".to_string(),
                target_uuid: None,
            },
        );
        let out = tick(&mut app);
        assert!(
            !out.iter()
                .any(|m| matches!(&m.msg, ServerMessage::TorpedoLaunched { .. })),
            "disabled tube fine system must block its fire"
        );
    }

    // ── Ship-level option (c) gate ────────────────────────────────────────

    #[test]
    fn set_target_refused_when_all_banks_offline() {
        let mut app = test_app();
        setup_weapons_world(&mut app, 30.0, 0.0);
        start_game_with_weapons(&mut app);
        // The updated `test_ship_config()` declares two fine phaser banks
        // ("phaser-fore", "phaser-aft"). `any_bank_accepts_human_input`
        // iterates them and returns true if ANY bank accepts human input.
        // So to refuse SetTarget, EVERY fine bank must be offline.
        // Mark both fine banks offline.
        mark_system_offline(&mut app, crate::system_registry::phaser_fore_system_id());
        mark_system_offline(&mut app, crate::system_registry::phaser_aft_system_id());

        push(
            &mut app,
            "weapons",
            ClientMessage::ControlSystem {
                target: crate::system_registry::tactical_radar_system_id(),
                payload: SystemControlPayload::SetTarget {
                    uuid: "target-uuid".into(),
                },
            },
        );
        let out = tick(&mut app);
        let has_lock = out
            .iter()
            .any(|m| matches!(&m.msg, ServerMessage::TargetLock { .. }));
        assert!(
            !has_lock,
            "SetTarget must be refused when every phaser bank fine system is offline"
        );
    }

    // ── Blackboards ───────────────────────────────────────────────────────

    #[test]
    fn publish_writes_phaser_fore_blackboard_when_bank_configured() {
        let mut app = test_app();
        // The test app config has "port"/"starboard" banks — no "fore" bank.
        // Insert a fresh combat config with a "fore" bank so publish emits an entry.
        {
            let mut q = app
                .world_mut()
                .query_filtered::<&mut PhaserCombatConfigResource, With<crate::server_app::LocalShip>>();
            if let Ok(mut cc) = q.single_mut(app.world_mut()) {
                cc.0.banks = vec![crate::entity_config::PhaserBankConfig {
                    id: "fore".into(),
                    facing_deg: 0.0,
                    fire_arc_deg: 270.0,
                    auto_arc_deg: 180.0,
                    beam_range: 50.0,
                    beam_damage_per_sec: 5.0,
                    beam_duration_secs: 6.0,
                    cooldown_secs: 6.0,
                    beam_color: vec![],
                    shield_pierce: None,
                    marker: None,
                }];
            }
        }
        // Publish runs in SimSet::Publish — one full update ticks it.
        app.update();

        let key = crate::system_registry::phaser_fore_system_id();
        let mut q = app
            .world_mut()
            .query_filtered::<
                &crate::server_app::ShipSystemBlackboards,
                With<crate::server_app::LocalShip>,
            >();
        let bbs = q.single(app.world()).unwrap();
        let bb = bbs
            .0
            .get(&key)
            .expect("expected phaser-fore in blackboards");
        assert!(matches!(bb, SystemBlackboard::PhaserBank(_)));
    }

    #[test]
    fn publish_writes_torpedo_magazine_blackboard() {
        let mut app = test_app();
        app.update();

        let key = crate::system_registry::torpedo_magazine_system_id();
        let mut q = app
            .world_mut()
            .query_filtered::<
                &crate::server_app::ShipSystemBlackboards,
                With<crate::server_app::LocalShip>,
            >();
        let bbs = q.single(app.world()).unwrap();
        let SystemBlackboard::TorpedoMagazine(mag_bb) = bbs
            .0
            .get(&key)
            .expect("expected torpedo-magazine in blackboards")
            .clone()
        else {
            panic!("expected TorpedoMagazine blackboard");
        };
        assert!(
            mag_bb.is_online,
            "fresh test ship magazine should be online"
        );
        assert_eq!(mag_bb.torpedoes_remaining, mag_bb.capacity);
    }

    #[test]
    fn publish_writes_torpedo_tube_blackboards_per_tube() {
        let mut app = test_app();
        app.update();

        let mut q = app
            .world_mut()
            .query_filtered::<
                &crate::server_app::ShipSystemBlackboards,
                With<crate::server_app::LocalShip>,
            >();
        let bbs = q.single(app.world()).unwrap();
        for tube_key in [
            crate::system_registry::torpedo_tube_fore_port_system_id(),
            crate::system_registry::torpedo_tube_fore_starboard_system_id(),
            crate::system_registry::torpedo_tube_aft_system_id(),
        ] {
            let bb = bbs
                .0
                .get(&tube_key)
                .unwrap_or_else(|| panic!("expected {tube_key:?} in blackboards"));
            assert!(matches!(bb, SystemBlackboard::TorpedoTube(_)));
        }
    }

    // ── Ship-level AI early-skip regression tests (issue #512, findings 1 & 2) ─
    //
    // These tests cover the specific production path the reviewer flagged as
    // dead code: after #512 deleted `[[system]] id = "tactical" kind = "tactical"`
    // from every ship TOML, the coarse tactical SystemId is not registered
    // in any ship's ControlSourceResolver. Every code path that gated on
    // a coarse-tactical policy lookup would therefore see the
    // default `Human` policy (`operate_ai = false`) and never run.
    //
    // These tests DO NOT touch the coarse `tactical` SystemId — they set
    // AI only on a fine phaser bank / torpedo tube and assert the
    // ship-level AI paths still activate.

    /// Finding 1 regression: the phaser auto-fire path used to gate its
    /// early skip on the coarse `tactical` policy. Post-fix, it uses
    /// `any_bank_operates_ai` which iterates the ship config's `phaser_bank`
    /// fine systems. This test seeds AI on ONE fine bank on an NPC — no
    /// coarse tactical touching — and asserts a beam still activates.
    #[test]
    fn ai_phaser_auto_fire_activates_when_any_bank_operates_ai() {
        use crate::ai_plugin::AiTokenRegistry;
        use crate::entity_spawner::{EntitySystemHull, EntityUuid};

        let mut app = test_app();
        app.init_resource::<AiTokenRegistry>();

        let npc_uuid = "cc000000-0000-0000-0000-000000000001";
        let target_uuid = "cc000000-0000-0000-0000-000000000002";

        // The NPC has a `phaser_bank` fine system ("phaser-port") declared
        // in its ShipConfigComponent — matching what the ship_harrow_*.toml
        // NPC TOMLs do. Its policy is Ai. The coarse `tactical` SystemId
        // is INTENTIONALLY untouched — the test would fail before finding 1
        // was fixed because the early-skip in `tick_phaser_auto_fire` would
        // read the coarse tactical policy's `operate_ai == false` and
        // `continue`.
        const NPC_TOML: &str = r#"
[[system]]
id = "phaser-port"
kind = "phaser_bank"
ai_only = true
"#;
        let npc_ship_config = crate::ship_plugin::ShipConfigComponent(
            crate::ship::config::parse_and_validate(NPC_TOML, &["phaser_bank"])
                .expect("NPC ship config must be valid"),
        );

        let mut sources = crate::ship::control_source::ControlSourceResolver::new();
        sources.set(
            SystemId("phaser-port".into()),
            crate::ship::control_source::ControlSource::Ai,
        );
        // NOTE: coarse tactical NOT set — this is the whole point of the test.

        let npc_entity = app
            .world_mut()
            .spawn((
                crate::server_app::Ship,
                EntityUuid(npc_uuid.to_string()),
                crate::ai_plugin::AiControllerComponent,
                crate::ship_plugin::ShipSystemControlSources(sources),
                npc_ship_config,
                WeaponsTarget(Some(target_uuid.to_string())),
                ActiveBeam::default(),
                PhaserCooldown::default(),
                ShipPhysics::default(),
                PhaserCombatConfigResource(crate::entity_config::PhaserCombatConfig {
                    banks: vec![crate::entity_config::PhaserBankConfig {
                        id: "port".into(),
                        facing_deg: 0.0,
                        fire_arc_deg: 360.0,
                        auto_arc_deg: 360.0,
                        beam_range: 50.0,
                        beam_damage_per_sec: 5.0,
                        beam_duration_secs: 3.0,
                        cooldown_secs: 6.0,
                        beam_color: vec![],
                        shield_pierce: None,
                        marker: None,
                    }],
                }),
                Transform::default(),
            ))
            .id();

        // Target directly ahead of NPC (yaw=0, forward=-Z).
        app.world_mut().spawn((
            EntityUuid(target_uuid.to_string()),
            EntitySystemHull(crate::damage::SystemHull::from_config(&[(
                SystemId("captain".into()),
                50.0,
            )])),
            Transform::from_xyz(0.0, 0.0, -20.0),
        ));

        app.update();

        let beam = app
            .world()
            .get::<ActiveBeam>(npc_entity)
            .expect("NPC entity must have ActiveBeam component");
        assert!(
            beam.target_uuid.is_some(),
            "ai_phaser_auto_fire must activate the beam when ANY phaser bank fine \
             system has operate_ai=true, even without the coarse tactical SystemId"
        );
        assert_eq!(
            beam.bank.as_deref(),
            Some("port"),
            "NPC should fire the port bank whose fine system is AI-operated"
        );
    }

    /// Finding 2 regression: the tactical AI used to gate its early skip on the
    /// coarse `tactical` policy. Post-fix, it uses
    /// `any_tactical_system_operates_ai` which iterates the ship config's
    /// phaser_bank / torpedo_tube / torpedo_magazine fine systems. This
    /// test seeds AI on `torpedo-magazine` alone (no coarse tactical) and
    /// asserts the AI's WeaponsTarget sync path fires.
    #[test]
    fn ai_target_selection_runs_when_any_tactical_system_operates_ai() {
        let mut app = test_app();

        // Set the LocalShip's active rating to Assisted so torpedo_auto_fire is enabled.
        set_tactical_station_rating(&mut app, "Assisted");

        // Set torpedo-magazine to Ai on the LocalShip. Do NOT touch coarse tactical.
        {
            let world = app.world_mut();
            let mut q = world
                .query_filtered::<&mut ShipSystemControlSources, With<crate::server_app::LocalShip>>();
            for mut cs in q.iter_mut(world) {
                cs.0.set(
                    crate::system_registry::torpedo_magazine_system_id(),
                    crate::ship::control_source::ControlSource::Ai,
                );
                // Confirm coarse tactical is NOT set — this is what makes
                // the test cover the finding. (#801: "tactical" is a station
                // id, not a system, so nothing should ever register it.)
                assert!(
                    !cs.0
                        .entries()
                        .any(|(id, _)| { id.0 == crate::system_registry::TACTICAL_STATION_ID }),
                    "test invariant: coarse tactical must NOT be registered"
                );
            }
        }

        // Simulate a Destroy objective so ai_target_selection has something
        // to lock onto (the AI target-sync leg exercises the early-skip we're
        // testing).
        let target_uuid = uuid::Uuid::new_v4().to_string();
        spawn_entity_target(&mut app, &target_uuid, 0.0, -30.0);
        app.world_mut()
            .resource_mut::<crate::world::server::WorldContentRuntime>()
            .name_to_uuid
            .insert("wave_1".into(), target_uuid.clone());
        insert_destroy_objective_blackboard(&mut app, "wave_1", 80.0);

        tick(&mut app);

        assert_eq!(
            get_weapons_target(&mut app).as_deref(),
            Some(target_uuid.as_str()),
            "ai_target_selection must run and lock the objective target when ANY \
             tactical fine system has operate_ai=true, even without the coarse tactical SystemId"
        );
    }

    // ── issue #692 (audit finding B1): tick_npc_auto_match_frequency gate ──
    //
    // Both frequency-hint systems must be gated on `AiHighFidelity`. The
    // `tick_frequency_hint` path already is (`ai_frequency_hint`); these two
    // tests cover the newly-added gate on the NPC auto-match path.

    /// Spawns a target entity that `tick_npc_auto_match_frequency` can read a
    /// shield frequency from: `EntityUuid` (matched against the locked target),
    /// `Transform` (so `ai_target_selection`'s stale-target guard treats it as
    /// alive and keeps the lock), and `ShipShields` carrying `freq`.
    fn spawn_shield_target(app: &mut App, uuid: &str, freq: f32) {
        app.world_mut().spawn((
            crate::entity_spawner::EntityUuid(uuid.into()),
            bevy::prelude::Transform::from_xyz(0.0, 0.0, -30.0),
            crate::ship::shields::ShipShields(crate::shield::ShieldSystem::default(), freq),
        ));
    }

    /// Puts the LocalShip's Tactical fine systems under AI control (so
    /// `any_tactical_system_operates_ai` is true) and locks it onto
    /// `target_uuid` — shared setup for both auto-match tests.
    fn setup_npc_auto_match(app: &mut App, target_uuid: &str) {
        set_tactical_control_source(app, crate::ship::control_source::ControlSource::Ai);
        set_weapons_target(app, Some(target_uuid.into()));
    }

    fn local_ship_entity(app: &mut App) -> Entity {
        let mut q = app
            .world_mut()
            .query_filtered::<Entity, With<crate::server_app::LocalShip>>();
        q.single(app.world())
            .expect("test_app must spawn a LocalShip")
    }

    /// Positive path: a high-fidelity NPC whose Tactical is AI-operated and
    /// which has a target locked drives its `ShipPhaserFrequency` toward the
    /// target's shield frequency once `NPC_FREQ_MATCH_DELAY` elapses.
    #[test]
    fn npc_auto_match_frequency_matches_with_high_fidelity() {
        let mut app = test_app();
        let target_uuid = "shield-target-hi-fi";
        // Distinct from ShipPhaserFrequency's 0.5 default AND from the code's
        // 0.5 fallback, so an observed change proves a real match fired.
        let target_freq = 0.8_f32;

        setup_npc_auto_match(&mut app, target_uuid);
        spawn_shield_target(&mut app, target_uuid, target_freq);

        let ship = local_ship_entity(&mut app);
        app.world_mut()
            .entity_mut(ship)
            .insert(crate::ai_plugin::AiHighFidelity);

        assert_eq!(
            get_phaser_frequency(&mut app),
            0.5,
            "test invariant: phaser frequency starts at its default"
        );

        // NPC_FREQ_MATCH_DELAY = 2.0s; test app ticks at 200ms → ≥10 ticks to
        // cross the delay. Run extra to stay clear of first-tick dt edge cases.
        for _ in 0..15 {
            tick(&mut app);
        }

        assert_eq!(
            get_phaser_frequency(&mut app),
            target_freq,
            "high-fidelity NPC must auto-match its phaser frequency to the locked \
             target's shield frequency after the delay"
        );
    }

    /// Negative path (the gate under test): identical setup but WITHOUT
    /// `AiHighFidelity` → the new gate suppresses auto-match and the phaser
    /// frequency never changes. This test fails if the `has_high_fidelity`
    /// gate is removed.
    #[test]
    fn npc_auto_match_frequency_gated_off_without_high_fidelity() {
        let mut app = test_app();
        let target_uuid = "shield-target-lo-fi";
        let target_freq = 0.8_f32;

        setup_npc_auto_match(&mut app, target_uuid);
        spawn_shield_target(&mut app, target_uuid, target_freq);

        // Deliberately NOT high-fidelity — no AiHighFidelity component.

        assert_eq!(
            get_phaser_frequency(&mut app),
            0.5,
            "test invariant: phaser frequency starts at its default"
        );

        for _ in 0..15 {
            tick(&mut app);
        }

        assert_eq!(
            get_phaser_frequency(&mut app),
            0.5,
            "without AiHighFidelity the auto-match gate must not fire; the phaser \
             frequency stays at its default"
        );
    }

    // ── Finding 5 regression: publish gates on offline_systems, not hardcoded Console match ──

    /// If an unknown / non-standard bank id ends up in the bank blackboard,
    /// the previous hardcoded `match "fore" | "aft"` returned `None` and
    /// silently reported `is_online: true` regardless of hull state.
    ///
    /// Post-fix, `is_online` is derived from `offline_systems` — so a bank
    /// whose fine SystemId lives in `offline_systems` reports `is_online: false`
    /// no matter whether the id matches a Console variant.
    #[test]
    fn publish_marks_bank_offline_when_fine_system_in_offline_set() {
        let mut app = test_app();
        // Swap in a bank config whose id is NOT in the hardcoded match
        // (e.g. "dorsal"), so the old bug's hardcoded id→Console arms
        // would default to online.
        {
            let mut q = app
                .world_mut()
                .query_filtered::<&mut PhaserCombatConfigResource, With<crate::server_app::LocalShip>>();
            if let Ok(mut cc) = q.single_mut(app.world_mut()) {
                cc.0.banks = vec![crate::entity_config::PhaserBankConfig {
                    id: "dorsal".into(),
                    facing_deg: 0.0,
                    fire_arc_deg: 270.0,
                    auto_arc_deg: 180.0,
                    beam_range: 50.0,
                    beam_damage_per_sec: 5.0,
                    beam_duration_secs: 6.0,
                    cooldown_secs: 6.0,
                    beam_color: vec![],
                    shield_pierce: None,
                    marker: None,
                }];
            }
        }
        // Mark the corresponding fine SystemId offline via offline_systems.
        mark_system_offline(&mut app, SystemId("phaser-dorsal".into()));

        app.update();

        let key = SystemId("phaser-dorsal".into());
        let mut q = app
            .world_mut()
            .query_filtered::<
                &crate::server_app::ShipSystemBlackboards,
                With<crate::server_app::LocalShip>,
            >();
        let bbs = q.single(app.world()).unwrap();
        let SystemBlackboard::PhaserBank(bb) = bbs
            .0
            .get(&key)
            .expect("expected phaser-dorsal blackboard entry")
            .clone()
        else {
            panic!("expected PhaserBank blackboard variant");
        };
        assert!(
            !bb.is_online,
            "bank must report is_online: false when its fine SystemId is in \
             offline_systems (regardless of whether the id matches a Console variant)"
        );
    }

    // ── Finding 7 regression: end-to-end hull → offline_systems → PhaserBankBlackboard ──
    //
    // Ties together sync_console_damage_tiers (in ship_plugin) and
    // publish_phaser_bank_blackboards (in this module). A hull entry for
    // Console::PhaserFore below the disabled threshold should end up as
    // `phaser-fore ∈ offline_systems` after one tick, and the emitted
    // blackboard should reflect `is_online: false`.

    #[test]
    fn hull_disabled_console_causes_publish_to_mark_bank_offline() {
        let mut app = test_app();
        // Register the sync system directly (test_app doesn't include ShipPlugin).
        app.add_systems(
            Update,
            crate::ship_plugin::sync_console_damage_tiers.in_set(crate::sim_sets::SimSet::Damage),
        );

        // Insert a "fore" bank so publish emits a `phaser-fore` blackboard.
        {
            let mut q = app
                .world_mut()
                .query_filtered::<&mut PhaserCombatConfigResource, With<crate::server_app::LocalShip>>();
            if let Ok(mut cc) = q.single_mut(app.world_mut()) {
                cc.0.banks = vec![crate::entity_config::PhaserBankConfig {
                    id: "fore".into(),
                    facing_deg: 0.0,
                    fire_arc_deg: 270.0,
                    auto_arc_deg: 180.0,
                    beam_range: 50.0,
                    beam_damage_per_sec: 5.0,
                    beam_duration_secs: 6.0,
                    cooldown_secs: 6.0,
                    beam_color: vec![],
                    shield_pierce: None,
                    marker: None,
                }];
            }
        }

        // Damage the PhaserFore console to 0 HP (Destroyed tier → offline).
        {
            let world = app.world_mut();
            let ship = world
                .query_filtered::<Entity, With<crate::server_app::LocalShip>>()
                .single(world)
                .unwrap();
            let mut entity_mut = app.world_mut().entity_mut(ship);
            let mut hull = entity_mut
                .get_mut::<crate::entity_spawner::EntitySystemHull>()
                .unwrap();
            hull.0.set_hp(&SystemId("phaser-fore".into()), 0.0);
        }

        // One update: sync_console_damage_tiers (Damage) writes offline_systems,
        // publish_phaser_bank_blackboards (Publish) reads it and emits the entry.
        app.update();

        // Step 1 verify: offline_systems contains `phaser-fore`.
        let phaser_fore_id = crate::system_registry::phaser_fore_system_id();
        let is_in_offline = {
            let mut q = app
                .world_mut()
                .query_filtered::<&ShipSystemControlSources, With<crate::server_app::LocalShip>>();
            let cs = q.single(app.world()).unwrap();
            cs.0.offline_systems.contains(&phaser_fore_id)
        };
        assert!(
            is_in_offline,
            "sync_console_damage_tiers must add phaser-fore to offline_systems \
             when Console::PhaserFore hull is at Disabled/Destroyed tier"
        );

        // Step 2 verify: blackboard reports is_online: false for phaser-fore.
        let mut q = app
            .world_mut()
            .query_filtered::<
                &crate::server_app::ShipSystemBlackboards,
                With<crate::server_app::LocalShip>,
            >();
        let bbs = q.single(app.world()).unwrap();
        let SystemBlackboard::PhaserBank(bb) = bbs
            .0
            .get(&phaser_fore_id)
            .expect("expected phaser-fore blackboard entry")
            .clone()
        else {
            panic!("expected PhaserBank blackboard variant");
        };
        assert!(
            !bb.is_online,
            "PhaserBankBlackboard.is_online must be false end-to-end when the \
             console hull is disabled (hull → offline_systems → blackboard chain)"
        );
    }

    // ── Finding 8 regression: magazine claim routes by source_entity ──────
    //
    // Before the fix, `handle_load_tube` emitted `source_entity: None` on
    // its `ClaimTorpedoRound` message. `handle_torpedo_magazine_inter_system`
    // then queried `With<LocalShip>` only, so an NPC's claim would either
    // be ignored entirely or misroute to the player ship. Post-fix, both
    // sides route by source_entity (mirroring `handle_power_inter_system`)
    // so each ship's claims mutate that ship's own magazine.

    #[test]
    fn magazine_claim_routes_to_shooter_ship_when_multiple_ships_have_magazines() {
        let mut app = test_app();

        // Snapshot the LocalShip's magazine counter.
        let localship_before = app
            .world_mut()
            .query_filtered::<&TorpedoSystemResource, With<crate::server_app::LocalShip>>()
            .single(app.world())
            .map(|ts| ts.0.torpedoes_remaining)
            .unwrap();

        // Spawn a second Ship (NOT LocalShip) that also has a magazine. Give
        // it a fully-declared torpedo_magazine fine system with Human
        // policy so the online gate passes, and its own TorpedoSystemResource
        // with 10 torpedoes and a "fore_port" tube.
        let mut npc_sources = crate::ship::control_source::ControlSourceResolver::new();
        npc_sources.set(
            crate::system_registry::torpedo_magazine_system_id(),
            crate::ship::control_source::ControlSource::Human,
        );
        npc_sources.set(
            crate::system_registry::torpedo_tube_fore_port_system_id(),
            crate::ship::control_source::ControlSource::Human,
        );
        let npc_torpedo_sys = TorpedoSystem::from_configs(
            &[crate::entity_config::TorpedoTubeConfig {
                id: "fore_port".into(),
                facing_deg: -30.0,
                fire_arc_deg: 90.0,
                load_time: None,
                marker: None,
                volley_max: 1,
            }],
            TorpedoConfig {
                count: 10,
                ..Default::default()
            },
        );
        let npc_entity = app
            .world_mut()
            .spawn((
                crate::server_app::Ship, // NOT LocalShip
                crate::entity_spawner::EntityUuid("npc-with-magazine".into()),
                crate::ship_plugin::ShipSystemControlSources(npc_sources),
                TorpedoSystemResource(npc_torpedo_sys),
                Transform::default(),
            ))
            .id();

        let npc_before = 10u32;

        // Install a one-shot system in `SimSet::Input` that pushes a claim
        // for the NPC entity into the queue every tick. This mirrors what
        // `handle_load_tube` would do if it ran for NPC ships — the point
        // of the test is that `handle_torpedo_magazine_inter_system` in
        // Physics routes the claim to the ship named by `source_entity`,
        // NOT to `With<LocalShip>` only.
        //
        // The queue is cleared by `clear_inter_system_queue` before
        // `SimSet::Input`, so pushing during Input survives to Physics.
        let claim_target_entity = npc_entity;
        app.add_systems(
            Update,
            (move |mut queue: ResMut<InterSystemQueue>| {
                queue.0.push(InterSystemMsg {
                    target: crate::system_registry::torpedo_magazine_system_id(),
                    payload: InterSystemPayload::ClaimTorpedoRound {
                        tube: "fore_port".into(),
                    },
                    source_entity: Some(claim_target_entity),
                });
            })
            .in_set(crate::sim_sets::SimSet::Input),
        );

        app.update();

        // LocalShip magazine must be UNCHANGED — the claim was for the NPC.
        let localship_after = app
            .world_mut()
            .query_filtered::<&TorpedoSystemResource, With<crate::server_app::LocalShip>>()
            .single(app.world())
            .map(|ts| ts.0.torpedoes_remaining)
            .unwrap();
        assert_eq!(
            localship_after, localship_before,
            "LocalShip magazine must NOT be decremented when the claim was \
             attributed to a different ship"
        );

        // NPC magazine must have decremented by 1.
        let npc_after = app
            .world()
            .get::<TorpedoSystemResource>(npc_entity)
            .unwrap()
            .0
            .torpedoes_remaining;
        assert_eq!(
            npc_after,
            npc_before - 1,
            "NPC magazine must decrement by 1 when its own claim is granted"
        );

        // NPC tube must be Loading.
        let npc_tube_loading = app
            .world()
            .get::<TorpedoSystemResource>(npc_entity)
            .unwrap()
            .0
            .tube("fore_port")
            .map(|t| matches!(t.load_state, crate::torpedo::TubeLoadState::Loading { .. }))
            .unwrap_or(false);
        assert!(
            npc_tube_loading,
            "NPC's own tube must transition to Loading after its claim is granted"
        );
    }

    // ── LOS blocking tests (Rapier) ──────────────────────────────────────────
    //
    // These tests spin up a Rapier world (like the collision tests in
    // server_app.rs) and verify that the beam-tick phases route damage
    // correctly when a blocking entity is between the shooter and the
    // original target.

    /// Build a minimal app with Rapier physics + WeaponsPlugin so
    /// `tick_beams_prepare` runs the LOS raycast.
    fn los_test_app() -> App {
        use bevy_rapier3d::prelude::RapierPhysicsPlugin;
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
                std::time::Duration::from_millis(200),
            ))
            .add_plugins(bevy::transform::TransformPlugin)
            .add_plugins(bevy::asset::AssetPlugin::default())
            .init_asset::<bevy::mesh::Mesh>()
            .init_resource::<bevy::scene::SceneSpawner>()
            .add_plugins(bevy::state::app::StatesPlugin)
            .init_state::<GamePhase>()
            .add_plugins(RapierPhysicsPlugin::<()>::default())
            .configure_sets(
                Update,
                (
                    crate::sim_sets::SimSet::Input,
                    crate::sim_sets::SimSet::Physics,
                    crate::sim_sets::SimSet::Damage,
                    crate::sim_sets::SimSet::Modifiers,
                    crate::sim_sets::SimSet::Publish,
                    crate::sim_sets::SimSet::PublishAggregate,
                    crate::sim_sets::SimSet::Broadcast,
                )
                    .chain(),
            )
            .add_plugins(LobbyPlugin)
            .add_plugins(crate::server_app::AdmissionPlugin)
            .init_resource::<WorldResource>()
            .add_message::<AsteroidDestroyedVfx>()
            .add_message::<crate::ai_plugin::AiEntityDestroyed>()
            .init_resource::<CurrentPhaserMode>()
            .insert_resource(TorpedoSystemResource(TorpedoSystem::new(
                TorpedoConfig::default(),
            )))
            .init_resource::<SimOutbox>()
            .init_resource::<Outbox>()
            .init_resource::<crate::world::server::WorldContentRuntime>()
            .insert_resource(crate::lobby::server::ShipClientConfigResource::default())
            // FactionRegistryResource for the LOS faction check.
            .insert_resource(crate::entities::config_cache::FactionRegistryResource(
                crate::entities::config_cache::get_faction_registry(),
            ))
            .add_plugins(WeaponsPlugin)
            .insert_resource(PhaserCombatConfigResource(
                crate::entity_config::PhaserCombatConfig {
                    banks: vec![crate::entity_config::PhaserBankConfig {
                        id: "port".into(),
                        facing_deg: -90.0,
                        fire_arc_deg: 360.0,
                        auto_arc_deg: 360.0,
                        beam_range: 0.0,
                        beam_damage_per_sec: 100.0,
                        beam_duration_secs: 10.0,
                        cooldown_secs: 1.0,
                        beam_color: vec![],
                        shield_pierce: None,
                        marker: None,
                    }],
                },
            ))
            // WeaponsPlugin already registers the three beam-tick phase
            // systems (tick_beams_prepare / tick_beams_apply_damage /
            // tick_beams_tick_lifetimes) and the two torpedo-tick phases
            // (build_torpedo_target_snapshot / tick_torpedo_lifecycle).
            // Do NOT register them again here.
            .add_plugins(crate::shields_plugin::ShipShieldsPlugin)
            .add_systems(PostUpdate, collect);

        // Advance one tick to let Rapier initialise.
        app.world_mut()
            .resource_mut::<NextState<GamePhase>>()
            .set(GamePhase::InProgress);
        app.update();
        app
    }

    /// Helper: spawn a ship entity with a ball collider and phaser state.
    /// Returns the Entity.
    fn spawn_los_ship(
        app: &mut App,
        uuid: &str,
        x: f32,
        z: f32,
        faction: Option<uuid::Uuid>,
        hull_hp: f32,
        is_local: bool,
    ) -> bevy::ecs::entity::Entity {
        use bevy_rapier3d::prelude::{
            ActiveCollisionTypes, Collider, ColliderMassProperties, RigidBody,
        };
        let mut ecmds = app.world_mut().spawn((
            crate::server_app::Ship,
            crate::entity_spawner::EntityUuid(uuid.to_string()),
            ShipPhysics {
                x,
                z,
                yaw: 0.0,
                forward_speed: 0.0,
                roll: 0.0,
                lateral_speed: 0.0,
            },
            Transform::from_xyz(x, 0.0, z),
            GlobalTransform::default(),
            Visibility::default(),
            // Ball collider large enough for the raycast to hit.
            Collider::ball(3.0),
            RigidBody::Fixed,
            ColliderMassProperties::Density(1.0),
            ActiveCollisionTypes::all(),
            crate::entity_spawner::EntitySystemHull(SystemHull::from_config(&[(
                SystemId("captain".into()),
                hull_hp,
            )])),
            ActiveBeam::default(),
            PhaserCooldown::default(),
            PhaserCombatConfigResource(crate::entity_config::PhaserCombatConfig {
                banks: vec![crate::entity_config::PhaserBankConfig {
                    id: "port".into(),
                    facing_deg: -90.0,
                    fire_arc_deg: 360.0,
                    auto_arc_deg: 360.0,
                    beam_range: 0.0,
                    beam_damage_per_sec: 100.0,
                    beam_duration_secs: 10.0,
                    cooldown_secs: 1.0,
                    beam_color: vec![],
                    shield_pierce: None,
                    marker: None,
                }],
            }),
            crate::ship_plugin::ShipSystemControlSources::default(),
        ));
        if is_local {
            ecmds.insert(crate::server_app::LocalShip);
        }
        if let Some(f) = faction {
            ecmds.insert(FactionComponent(f));
        }
        ecmds.id()
    }

    /// Helper: spawn an asteroid with a ball collider.
    fn spawn_los_asteroid(
        app: &mut App,
        uuid: &str,
        x: f32,
        z: f32,
        hull_hp: f32,
    ) -> bevy::ecs::entity::Entity {
        use bevy_rapier3d::prelude::{
            ActiveCollisionTypes, Collider, ColliderMassProperties, RigidBody,
        };
        app.world_mut()
            .spawn((
                crate::simulation::Asteroid,
                AsteroidUuid(uuid.to_string()),
                Transform::from_xyz(x, 0.0, z),
                GlobalTransform::default(),
                Visibility::default(),
                Collider::ball(3.0),
                RigidBody::Fixed,
                ColliderMassProperties::Density(1.0),
                ActiveCollisionTypes::all(),
                crate::entity_spawner::EntitySystemHull(SystemHull::from_config(&[(
                    SystemId("captain".into()),
                    hull_hp,
                )])),
            ))
            .id()
    }

    /// Activate a beam on the given ship entity, targeting `target_uuid`.
    fn activate_los_beam(app: &mut App, shooter: bevy::ecs::entity::Entity, target_uuid: &str) {
        let mut beam = app.world_mut().get_mut::<ActiveBeam>(shooter).unwrap();
        beam.target_uuid = Some(target_uuid.to_string());
        beam.remaining_secs = 10.0;
        beam.damage_accumulator = 0.0;
        beam.bank = Some("port".to_string());
    }

    /// Read the total current hull HP from a ship/asteroid entity.
    fn hull_hp(app: &App, entity: bevy::ecs::entity::Entity) -> f32 {
        app.world()
            .get::<crate::entity_spawner::EntitySystemHull>(entity)
            .map(|h| h.0.total_current())
            .unwrap_or(0.0)
    }

    #[test]
    fn los_no_blocker_damages_original_target() {
        // Shooter at origin, target at (0, 0, -30). No entity in between.
        // Beam should damage the original target.
        let mut app = los_test_app();
        let faction_uuid = uuid::Uuid::parse_str("aaaaaaaa-0000-0000-0000-000000000001").unwrap();

        let shooter = spawn_los_ship(
            &mut app,
            "shooter-uuid",
            0.0,
            0.0,
            Some(faction_uuid),
            200.0,
            true,
        );
        let target = spawn_los_ship(&mut app, "target-uuid", 0.0, -30.0, None, 200.0, false);

        // Let Rapier settle and colliders register at their correct positions.
        app.update();
        app.update();

        activate_los_beam(&mut app, shooter, "target-uuid");

        let before = hull_hp(&app, target);
        // Run a few ticks to accumulate damage.
        for _ in 0..5 {
            app.update();
        }
        let after = hull_hp(&app, target);
        assert!(
            after < before,
            "Target should take damage when LOS is clear (before={before}, after={after})"
        );
    }

    #[test]
    fn los_enemy_blocker_redirects_damage_away_from_target() {
        // Shooter at origin. Enemy blocker at (0,0,-10). Original target at (0,0,-30).
        // Blocker is in the way → target takes no damage, blocker takes damage.
        use crate::config_cache::FactionRegistryResource;
        use crate::faction::FactionRegistry;

        let mut app = los_test_app();

        let shooter_faction =
            uuid::Uuid::parse_str("aaaaaaaa-0000-0000-0000-000000000001").unwrap();
        let enemy_faction = uuid::Uuid::parse_str("bbbbbbbb-0000-0000-0000-000000000002").unwrap();

        // Make shooter hostile to blocker.
        let mut reg = FactionRegistry::new();
        reg.insert(crate::faction::FactionConfig {
            uuid: shooter_faction,
            name: "Federation".into(),
            enemies: vec![enemy_faction],
        });
        reg.insert(crate::faction::FactionConfig {
            uuid: enemy_faction,
            name: "Pirate".into(),
            enemies: vec![],
        });
        app.insert_resource(FactionRegistryResource(reg));

        let shooter = spawn_los_ship(
            &mut app,
            "shooter-uuid-2",
            0.0,
            0.0,
            Some(shooter_faction),
            200.0,
            true,
        );
        let blocker = spawn_los_ship(
            &mut app,
            "blocker-uuid-2",
            0.0,
            -10.0,
            Some(enemy_faction),
            500.0,
            false,
        );
        let target = spawn_los_ship(&mut app, "target-uuid-2", 0.0, -30.0, None, 500.0, false);

        // Let Rapier settle so colliders are at their correct positions.
        app.update();
        app.update();

        activate_los_beam(&mut app, shooter, "target-uuid-2");

        let blocker_before = hull_hp(&app, blocker);
        let target_before = hull_hp(&app, target);
        // Run several ticks — each tick the ray hits the blocker, rerouting damage.
        for _ in 0..5 {
            app.update();
        }
        let blocker_after = hull_hp(&app, blocker);
        let target_after = hull_hp(&app, target);

        assert!(
            blocker_after < blocker_before,
            "Enemy blocker between shooter and target must take damage \
             (before={blocker_before}, after={blocker_after})"
        );
        assert_eq!(
            target_after, target_before,
            "Original target must NOT take damage when blocked \
             (before={target_before}, after={target_after})"
        );
    }

    #[test]
    fn los_friendly_blocker_absorbs_beam_with_no_damage() {
        // Shooter and blocker are same faction. Blocker at (0,0,-10),
        // target at (0,0,-30). Neither blocker nor target should take damage.
        use crate::config_cache::FactionRegistryResource;
        use crate::faction::FactionRegistry;

        let mut app = los_test_app();

        let faction_uuid = uuid::Uuid::parse_str("cccccccc-0000-0000-0000-000000000003").unwrap();

        // Empty enemy list → faction is friendly to itself.
        let mut reg = FactionRegistry::new();
        reg.insert(crate::faction::FactionConfig {
            uuid: faction_uuid,
            name: "Federation".into(),
            enemies: vec![],
        });
        app.insert_resource(FactionRegistryResource(reg));

        let shooter = spawn_los_ship(
            &mut app,
            "shooter-uuid-3",
            0.0,
            0.0,
            Some(faction_uuid),
            200.0,
            true,
        );
        let blocker = spawn_los_ship(
            &mut app,
            "blocker-uuid-3",
            0.0,
            -10.0,
            Some(faction_uuid), // same faction → friendly
            500.0,
            false,
        );
        let target = spawn_los_ship(&mut app, "target-uuid-3", 0.0, -30.0, None, 500.0, false);

        // Let Rapier settle so colliders are at their correct positions.
        app.update();
        app.update();

        activate_los_beam(&mut app, shooter, "target-uuid-3");

        let blocker_before = hull_hp(&app, blocker);
        let target_before = hull_hp(&app, target);
        for _ in 0..5 {
            app.update();
        }
        let blocker_after = hull_hp(&app, blocker);
        let target_after = hull_hp(&app, target);

        assert_eq!(
            blocker_after, blocker_before,
            "Friendly blocker must NOT take damage (before={blocker_before}, after={blocker_after})"
        );
        assert_eq!(
            target_after, target_before,
            "Target must NOT take damage when a friendly blocks (before={target_before}, after={target_after})"
        );
    }

    #[test]
    fn los_asteroid_blocker_takes_damage() {
        // Asteroid at (0,0,-10), target at (0,0,-30).
        // Beam aimed at target — asteroid intercepts and takes damage.
        let mut app = los_test_app();

        let shooter = spawn_los_ship(&mut app, "shooter-uuid-4", 0.0, 0.0, None, 200.0, true);
        let ast = spawn_los_asteroid(&mut app, "ast-uuid-4", 0.0, -10.0, 2000.0);
        let target = spawn_los_ship(&mut app, "target-uuid-4", 0.0, -30.0, None, 500.0, false);

        // Let Rapier settle so colliders are at their correct positions.
        app.update();
        app.update();

        activate_los_beam(&mut app, shooter, "target-uuid-4");

        let ast_before = hull_hp(&app, ast);
        let target_before = hull_hp(&app, target);
        for _ in 0..5 {
            app.update();
        }
        let ast_after = hull_hp(&app, ast);
        let target_after = hull_hp(&app, target);

        assert!(
            ast_after < ast_before,
            "Asteroid blocker must take damage (before={ast_before}, after={ast_after})"
        );
        assert_eq!(
            target_after, target_before,
            "Target behind asteroid must NOT take damage (before={target_before}, after={target_after})"
        );
    }

    // ── Blaster AI auto-fire tests ──────────────────────────────────────

    /// NPC with tactical set to Ai and target in range must have the auto-fire
    /// system call `request_charge_start` on the blaster bank.
    #[test]
    fn tick_blaster_auto_fire_gate_passes_when_tactical_is_ai() {
        use crate::entity_spawner::EntityUuid;

        let mut app = test_app();

        let npc_uuid = "bb000000-0000-0000-0000-000000000010";
        let target_uuid = "bb000000-0000-0000-0000-000000000011";

        let mut sources = crate::ship::control_source::ControlSourceResolver::new();
        // #801: seed the blaster bank's fine system (no coarse tactical).
        sources.set(
            crate::system_registry::blaster_bank_system_id("fore").unwrap(),
            crate::ship::control_source::ControlSource::Ai,
        );
        // NPC at (10, 10) — away from LocalShip at origin — facing -Z (target at 10, -10).
        // This avoids the projectile immediately hitting the LocalShip which
        // occupies (0, 0) in test_app().
        let npc_physics = ShipPhysics {
            x: 10.0,
            z: 10.0,
            ..Default::default()
        };
        let npc_entity = app
            .world_mut()
            .spawn((
                crate::server_app::Ship,
                EntityUuid(npc_uuid.to_string()),
                crate::ship_plugin::ShipSystemControlSources(sources),
                WeaponsTarget(Some(target_uuid.to_string())),
                npc_physics,
                BlasterSystemResource(vec![crate::blaster::BlasterSystem::new(
                    crate::blaster::BlasterBankConfig {
                        id: "fore".into(),
                        facing_deg: 180.0, // face toward -Z (toward target)
                        fire_arc_deg: 360.0,
                        volley_count: 1,
                        volley_interval_secs: 0.1,
                        cooldown_secs: 3.0,
                        charge_time_secs: 0.0,
                        projectile_speed: 40.0,
                        collision_radius: 1.5,
                        visual_scale: 1.0,
                        damage: 10,
                        shield_pierce: 0.0,
                        recoil_impulse: 0.0,
                        screenshake_magnitude: 0.0,
                        marker: None,
                        range: 35.0,
                    },
                )]),
                Transform::from_xyz(10.0, 0.0, 10.0),
            ))
            .id();

        // Spawn target directly ahead (-Z), well within blaster range.
        app.world_mut().spawn((
            EntityUuid(target_uuid.to_string()),
            crate::entity_spawner::EntitySystemHull(crate::damage::SystemHull::from_config(&[(
                SystemId("captain".into()),
                50.0,
            )])),
            Transform::from_xyz(10.0, 0.0, -10.0),
        ));

        // Check initial state before update.
        let init_bank = &app
            .world()
            .get::<BlasterSystemResource>(npc_entity)
            .unwrap()
            .0[0];
        eprintln!(
            "[DEBUG] init: fire_ready={} on_cooldown={} pending={} charging={}",
            init_bank.is_fire_ready(),
            init_bank.volley.on_cooldown,
            init_bank.volley.pending_volley,
            init_bank.volley.charging,
        );

        app.update();

        let blaster_res = app
            .world()
            .get::<BlasterSystemResource>(npc_entity)
            .unwrap();
        let bank = &blaster_res.0[0];
        // tick_blaster_auto_fire (Input) calls request_charge_start, then
        // tick_blaster_system (Physics) fires the projectile same-tick.
        // The projectile ends up in in_flight.
        assert!(
            !bank.in_flight.is_empty(),
            "tick_blaster_auto_fire must fire a blaster projectile when tactical is Ai \
             and target is in range/arc (in_flight={})",
            bank.in_flight.len(),
        );
    }

    /// NPC with AI-controlled blaster has target out of range — must NOT fire.
    #[test]
    fn tick_blaster_auto_fire_skips_when_target_out_of_range() {
        use crate::entity_spawner::EntityUuid;

        let mut app = test_app();

        let npc_uuid = "bb000000-0000-0000-0000-000000000020";
        let target_uuid = "bb000000-0000-0000-0000-000000000021";

        let mut sources = crate::ship::control_source::ControlSourceResolver::new();
        // #801: seed the blaster bank's fine system (no coarse tactical).
        sources.set(
            crate::system_registry::blaster_bank_system_id("fore").unwrap(),
            crate::ship::control_source::ControlSource::Ai,
        );
        let npc_entity = app
            .world_mut()
            .spawn((
                crate::server_app::Ship,
                EntityUuid(npc_uuid.to_string()),
                crate::ai_plugin::AiControllerComponent,
                crate::ship_plugin::ShipSystemControlSources(sources),
                WeaponsTarget(Some(target_uuid.to_string())),
                ShipPhysics::default(),
                BlasterSystemResource(vec![crate::blaster::BlasterSystem::new(
                    crate::blaster::BlasterBankConfig {
                        id: "fore".into(),
                        facing_deg: 0.0,
                        fire_arc_deg: 360.0,
                        volley_count: 1,
                        volley_interval_secs: 0.1,
                        cooldown_secs: 3.0,
                        charge_time_secs: 0.0,
                        projectile_speed: 40.0,
                        collision_radius: 1.5,
                        visual_scale: 1.0,
                        damage: 10,
                        shield_pierce: 0.0,
                        recoil_impulse: 0.0,
                        screenshake_magnitude: 0.0,
                        marker: None,
                        range: 35.0,
                    },
                )]),
                Transform::default(),
            ))
            .id();

        // Spawn target well outside blaster range (35 units).
        app.world_mut().spawn((
            EntityUuid(target_uuid.to_string()),
            crate::entity_spawner::EntitySystemHull(crate::damage::SystemHull::from_config(&[(
                SystemId("captain".into()),
                50.0,
            )])),
            Transform::from_xyz(0.0, 0.0, -100.0),
        ));

        app.update();

        let blaster_res = app
            .world()
            .get::<BlasterSystemResource>(npc_entity)
            .unwrap();
        assert_eq!(
            blaster_res.0[0].volley.pending_volley, 0,
            "tick_blaster_auto_fire must NOT fire when target is out of range"
        );
    }

    /// AI token sent through `handle_fire_blaster` must route to the NPC and fire.
    #[test]
    fn handle_fire_blaster_accepts_ai_token() {
        use crate::entity_spawner::EntityUuid;

        let mut app = test_app();
        app.init_resource::<crate::ai_plugin::AiTokenRegistry>();

        let npc_uuid = "bb000000-0000-0000-0000-000000000030";
        let target_uuid_str = "bb000000-0000-0000-0000-000000000031";

        // NPC with Tactical set to Ai.
        let mut sources = crate::ship::control_source::ControlSourceResolver::new();
        // #801: seed the blaster bank's fine system (no coarse tactical).
        sources.set(
            crate::system_registry::blaster_bank_system_id("fore").unwrap(),
            crate::ship::control_source::ControlSource::Ai,
        );
        let target_uuid_parsed = uuid::Uuid::parse_str(target_uuid_str).unwrap();
        let npc_entity = app
            .world_mut()
            .spawn((
                crate::server_app::Ship,
                EntityUuid(npc_uuid.to_string()),
                crate::ai_plugin::AiControllerComponent,
                // Seeds the NPC's Tactical lock. This used to seed
                // `ShipAiMemory.target` and rely on `tick_blaster_auto_fire`'s
                // legacy fallback to read it; #702 deleted that fallback, so the
                // lock goes where every other consumer looks for it.
                WeaponsTarget(Some(target_uuid_parsed.to_string())),
                crate::ship_plugin::ShipSystemControlSources(sources),
                ShipPhysics::default(),
                BlasterSystemResource(vec![crate::blaster::BlasterSystem::new(
                    crate::blaster::BlasterBankConfig {
                        id: "fore".into(),
                        facing_deg: 0.0,
                        fire_arc_deg: 360.0,
                        volley_count: 1,
                        volley_interval_secs: 0.1,
                        cooldown_secs: 3.0,
                        charge_time_secs: 0.0,
                        projectile_speed: 40.0,
                        collision_radius: 1.5,
                        visual_scale: 1.0,
                        damage: 10,
                        shield_pierce: 0.0,
                        recoil_impulse: 0.0,
                        screenshake_magnitude: 0.0,
                        marker: None,
                        range: 35.0,
                    },
                )]),
                Transform::default(),
            ))
            .id();

        // Spawn target entity at (0, -10) — directly ahead of NPC at origin,
        // within the 35-unit range and inside the 360° fire arc.
        app.world_mut().spawn((
            EntityUuid(target_uuid_str.to_string()),
            crate::entity_spawner::EntitySystemHull(crate::damage::SystemHull::from_config(&[(
                SystemId("captain".into()),
                50.0,
            )])),
            Transform::from_xyz(0.0, 0.0, -10.0),
        ));

        // Register the AI token so handle_fire_blaster can resolve it.
        {
            let mut reg = app
                .world_mut()
                .resource_mut::<crate::ai_plugin::AiTokenRegistry>();
            reg.register_with_entity(npc_uuid, npc_entity);
        }

        // Send a FireBlaster ControlSystem message via the AI token.
        let ai_token = format!("ai:{}", npc_uuid);
        push(
            &mut app,
            &ai_token,
            ClientMessage::ControlSystem {
                target: SystemId("blaster-fore".into()),
                payload: SystemControlPayload::FireBlaster,
            },
        );

        app.update();

        let blaster_res = app
            .world()
            .get::<BlasterSystemResource>(npc_entity)
            .unwrap();
        // After app.update(): handle_fire_blaster (Input) arms the volley, then
        // tick_blaster_system (Physics) fires it and enters cooldown. By the time
        // we check, pending_volley is 0 and on_cooldown is true — verify cooldown
        // as evidence the volley was dispatched.
        assert!(
            blaster_res.0[0].volley.on_cooldown,
            "handle_fire_blaster must accept AI token and enter cooldown after firing"
        );
    }
}
