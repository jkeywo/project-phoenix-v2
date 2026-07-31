use crate::simmath;
use bevy::prelude::*;

use super::beam::{
    ActiveBeam, CurrentPhaserMode, PhaserCombatConfigResource, PhaserCooldown,
    TacticalRadarSelection,
};
use super::blaster::BlasterSystemResource;
use super::shared::live_entity_xz;
use super::torpedo::TorpedoSystemResource;
use crate::lobby::WorldResource;
use crate::messages::{
    BlasterBankState, ModifierSlot, PhaserBankClientConfig, PhaserBankState, PhaserMode, RadarBlip,
    RadarRegion, ServerMessage, SystemBlackboard, TorpedoTubeClientConfig, TorpedoTubeState,
    WeaponsBlackboard,
};
use crate::ship_plugin::ShipSystemControlSources;
use crate::ship_state::ShipPhysics;
use crate::simulation::AsteroidUuid;
use crate::torpedo::{TorpedoConfig, TorpedoSystem};

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
    // Combat Lock from the local ship's own frozen viewscreen blackboard, not
    // the live `TacticalRadarSelection` (spec §3). `weapons_update_broadcaster`
    // runs in `SimSet::Broadcast`, i.e. *after* the viewscreen aggregators in
    // `SimSet::PublishAggregate`, so this reads the current tick's lock with no
    // lag; the reconnect-resync caller (`cache_registry::resync_for_token`)
    // reads whatever the last completed tick aggregated.
    let target_uuid: Option<String> = {
        let mut q = world
            .query_filtered::<&crate::server_app::ShipSystemBlackboards, With<crate::server_app::LocalShip>>();
        q.single(world).ok().and_then(viewscreen_combat_lock)
    };
    // The whole per-bank beam map (issue #790), not a single (active?, bank)
    // pair: a hull with overlapping arcs can be burning several banks at once.
    let active_beam: ActiveBeam = {
        let mut q = world.query_filtered::<&ActiveBeam, With<crate::server_app::LocalShip>>();
        q.single(world).ok().cloned().unwrap_or_default()
    };
    let bank_cooldowns: std::collections::HashMap<String, f32> = {
        let mut q = world.query_filtered::<&PhaserCooldown, With<crate::server_app::LocalShip>>();
        q.single(world)
            .ok()
            .map(|cd| cd.per_bank.clone())
            .unwrap_or_default()
    };
    // Per-entity component on `LocalShip` is authoritative (#832): every ship
    // with a `[torpedoes]` block carries its own `TorpedoSystemResource`
    // component; a ship without one legitimately has no tubes. Raw tubes +
    // effective homing range are captured here; the `TorpedoTubeState` list
    // (with the readiness contract) is built below, once `target_live_pos` and
    // the offline probe are resolved.
    let (raw_tubes, torpedo_effective_range): (Vec<crate::torpedo::TorpedoTube>, f32) = {
        let mut q =
            world.query_filtered::<&TorpedoSystemResource, With<crate::server_app::LocalShip>>();
        q.single(world)
            .ok()
            .map(|ts| (ts.0.tubes.clone(), ts.0.config.speed * ts.0.config.lifespan))
            .unwrap_or_default()
    };
    let torpedo_count = {
        // Per-entity component on `LocalShip` is authoritative (#832).
        let mut q =
            world.query_filtered::<&TorpedoSystemResource, With<crate::server_app::LocalShip>>();
        q.single(world)
            .ok()
            .map(|ts| ts.0.torpedoes_remaining)
            .unwrap_or_default()
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
        // Per-entity component on `LocalShip` is authoritative (#832): the player
        // ship unconditionally carries a `PhaserCombatConfigResource` component
        // (config-derived or defaulted) at spawn.
        let mut q = world
            .query_filtered::<&PhaserCombatConfigResource, With<crate::server_app::LocalShip>>();
        q.single(world)
            .ok()
            .map(|cc| cc.0.banks.clone())
            .unwrap_or_default()
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

    // Per-system offline state for the LocalShip, used by the phaser, blaster,
    // and torpedo readiness contracts below. Cloned once as owned data so the
    // probe closure does not hold a borrow on `world` (which the queries below
    // need mutably).
    let local_control: Option<crate::ship::control_source::ControlSourceResolver> = {
        let mut q =
            world.query_filtered::<&ShipSystemControlSources, With<crate::server_app::LocalShip>>();
        q.single(world).ok().map(|cs| cs.0.clone())
    };
    let ship_offline = |sysid: Option<crate::messages::SystemId>| -> bool {
        match sysid {
            None => false,
            Some(id) => local_control
                .as_ref()
                .map(|c| c.is_offline(&id))
                .unwrap_or(false),
        }
    };

    // Blaster bank states, with the shared readiness contract (issue #764).
    // Computed here where ship physics + the frozen combat-lock target position
    // are already resolved, rather than re-querying in the broadcaster.
    let blasters: Vec<BlasterBankState> = {
        let raw: Vec<crate::blaster::BlasterSystem> = {
            let mut q = world
                .query_filtered::<&BlasterSystemResource, With<crate::server_app::LocalShip>>();
            q.single(world)
                .ok()
                .map(|r| r.0.clone())
                .unwrap_or_default()
        };
        raw.iter()
            .map(|b| {
                let is_online =
                    !ship_offline(crate::system_registry::blaster_bank_system_id(&b.config.id));
                b.bank_state(ship_x, ship_z, ship_yaw, target_live_pos, is_online)
            })
            .collect()
    };

    // Torpedo tube states, with the shared readiness contract (issue #764).
    let physics = ShipPhysics {
        x: ship_x,
        z: ship_z,
        yaw: ship_yaw,
        ..Default::default()
    };
    let tubes: Vec<TorpedoTubeState> = raw_tubes
        .iter()
        .map(|t| {
            let remaining = match &t.load_state {
                crate::torpedo::TubeLoadState::Loading { remaining, .. }
                | crate::torpedo::TubeLoadState::Unloading { remaining, .. } => *remaining,
                _ => 0.0,
            };
            let is_online = !ship_offline(crate::system_registry::torpedo_tube_system_id(&t.id));
            let readiness = tube_readiness(
                t,
                torpedo_effective_range,
                physics,
                target_live_pos,
                is_online,
            );
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
                readiness,
                active_barrels: t.active_barrels.clone(),
                pattern_step: t.pattern_step,
                pattern_len: t.pattern_len(),
            }
        })
        .collect();

    let banks: Vec<PhaserBankState> = if banks_config.is_empty() {
        let effective_phaser_range =
            crate::entity_config::PhaserCombatConfig::DEFAULT_PHASER_RANGE * radar_range_mult;
        // Default (no-config) bank is a 180° forward arc, facing 0 — matches
        // `radar::is_fire_ready_with_range`.
        let geometry = target_live_pos.map(|(tx, tz)| {
            crate::weapons::phaser::target_geometry(
                tx,
                tz,
                ship_x,
                ship_z,
                ship_yaw,
                effective_phaser_range,
                0.0,
                180.0,
            )
        });
        let cd = bank_cooldowns.get("").copied().unwrap_or(0.0);
        let on_cooldown = active_beam.is_bank_firing("") || cd > 0.0;
        let is_online = !ship_offline(crate::system_registry::phaser_bank_system_id(""));
        let fire_ready = geometry.map(|g| g.in_range && g.in_arc).unwrap_or(false);
        let readiness = crate::messages::WeaponReadiness::evaluate(
            is_online,
            on_cooldown,
            false,
            false,
            geometry,
        );
        vec![PhaserBankState {
            id: String::new(),
            fire_ready,
            on_cooldown,
            cooldown_remaining: cd,
            readiness,
        }]
    } else {
        banks_config
            .iter()
            .map(|b| {
                let bank_base_range = if b.beam_range > 0.0 {
                    b.beam_range
                } else {
                    crate::entity_config::PhaserCombatConfig::DEFAULT_PHASER_RANGE
                };
                let effective_bank_range = bank_base_range * radar_range_mult;
                let geometry = target_live_pos.map(|(tx, tz)| {
                    crate::weapons::phaser::target_geometry(
                        tx,
                        tz,
                        ship_x,
                        ship_z,
                        ship_yaw,
                        effective_bank_range,
                        b.facing_deg,
                        b.fire_arc_deg,
                    )
                });
                let cd = bank_cooldowns.get(b.id.as_str()).copied().unwrap_or(0.0);
                let on_cooldown = active_beam.is_bank_firing(&b.id) || cd > 0.0;
                let is_online = !ship_offline(crate::system_registry::phaser_bank_system_id(&b.id));
                let fire_ready = geometry.map(|g| g.in_range && g.in_arc).unwrap_or(false);
                let readiness = crate::messages::WeaponReadiness::evaluate(
                    is_online,
                    on_cooldown,
                    false,
                    false,
                    geometry,
                );
                PhaserBankState {
                    id: b.id.clone(),
                    fire_ready,
                    on_cooldown,
                    cooldown_remaining: cd,
                    readiness,
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
        blasters,
        phaser_frequency,
    }
}

pub fn weapons_update_broadcaster() -> crate::core::broadcast::SimBroadcaster {
    crate::core::broadcast::SimBroadcaster::new().register(
        crate::core::broadcast::Audience::HoldingWeapons,
        crate::core::broadcast::Cadence::Hz(10.0),
        |world: &mut World| {
            let current = compute_current_weapons_update(world);

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

/// Read the **Combat Lock** out of a ship's own frozen `ViewscreenBlackboard`.
///
/// The one target surface Weapons — a cross-system consumer of the tactical
/// radar — is allowed to read (spec §3: "Cross-system target reads must not
/// bypass the viewscreen blackboard to reach a radar's live selection
/// synchronously"). Mirrors `blaster::blaster_combat_lock` and the beam /
/// torpedo / helm / shields / sensors readers converted in issue #829.
///
/// Callers in `SimSet::Publish` see the value the viewscreen aggregators wrote
/// in `SimSet::PublishAggregate` **last** tick — the accepted one-tick lag.
fn viewscreen_combat_lock(bbs: &crate::server_app::ShipSystemBlackboards) -> Option<String> {
    match bbs.0.get(&crate::system_registry::viewscreen_system_id()) {
        Some(SystemBlackboard::Viewscreen(bb)) => bb.combat_lock.clone(),
        _ => None,
    }
}

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
    // The whole per-bank beam map (issue #790): each bank's `on_cooldown` asks
    // whether THAT bank is burning, so a single (active?, bank) pair could not
    // describe a ship with two broadsides lit.
    beam: &ActiveBeam,
    radar_range_mult: f32,
    physics: ShipPhysics,
    target_live_pos: Option<(f32, f32)>,
    control_sources: Option<&ShipSystemControlSources>,
) -> Vec<PhaserBankState> {
    // Per-bank offline probe (issue #764). `None` control sources (test paths
    // with no Ship component) treat every bank as online, matching the
    // `is_online` fallback the fine-blackboard publishers already use.
    let bank_online = |bank_id: &str| -> bool {
        match (
            control_sources,
            crate::system_registry::phaser_bank_system_id(bank_id),
        ) {
            (Some(cs), Some(id)) => !cs.0.is_offline(&id),
            _ => true,
        }
    };
    if combat_config.0.banks.is_empty() {
        let effective_range =
            crate::entity_config::PhaserCombatConfig::DEFAULT_PHASER_RANGE * radar_range_mult;
        // Default (no-config) bank is a 180° forward arc, facing 0 — matches
        // `radar::is_fire_ready_with_range`.
        let geometry = target_live_pos.map(|(tx, tz)| {
            crate::weapons::phaser::target_geometry(
                tx,
                tz,
                physics.x,
                physics.z,
                physics.yaw,
                effective_range,
                0.0,
                180.0,
            )
        });
        let cd = cooldown.bank_remaining_secs("");
        let on_cooldown = beam.is_bank_firing("") || cd > 0.0;
        let fire_ready = geometry.map(|g| g.in_range && g.in_arc).unwrap_or(false);
        let readiness = crate::messages::WeaponReadiness::evaluate(
            bank_online(""),
            on_cooldown,
            false,
            false,
            geometry,
        );
        vec![PhaserBankState {
            id: String::new(),
            fire_ready,
            on_cooldown,
            cooldown_remaining: cd,
            readiness,
        }]
    } else {
        combat_config
            .0
            .banks
            .iter()
            .map(|b| {
                let bank_base_range = if b.beam_range > 0.0 {
                    b.beam_range
                } else {
                    crate::entity_config::PhaserCombatConfig::DEFAULT_PHASER_RANGE
                };
                let effective_bank_range = bank_base_range * radar_range_mult;
                let geometry = target_live_pos.map(|(tx, tz)| {
                    crate::weapons::phaser::target_geometry(
                        tx,
                        tz,
                        physics.x,
                        physics.z,
                        physics.yaw,
                        effective_bank_range,
                        b.facing_deg,
                        b.fire_arc_deg,
                    )
                });
                let cd = cooldown.bank_remaining_secs(b.id.as_str());
                let on_cooldown = beam.is_bank_firing(&b.id) || cd > 0.0;
                let fire_ready = geometry.map(|g| g.in_range && g.in_arc).unwrap_or(false);
                let readiness = crate::messages::WeaponReadiness::evaluate(
                    bank_online(&b.id),
                    on_cooldown,
                    false,
                    false,
                    geometry,
                );
                PhaserBankState {
                    id: b.id.clone(),
                    fire_ready,
                    on_cooldown,
                    cooldown_remaining: cd,
                    readiness,
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
fn build_tube_states(
    torpedo_sys: &TorpedoSystemResource,
    physics: ShipPhysics,
    target_live_pos: Option<(f32, f32)>,
    control_sources: Option<&ShipSystemControlSources>,
) -> Vec<TorpedoTubeState> {
    // A homing torpedo's effective reach is its cruise speed × lifespan — the
    // authoritative range the readiness contract reports (issue #764).
    let effective_range = torpedo_sys.0.config.speed * torpedo_sys.0.config.lifespan;
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
            let is_online = match (
                control_sources,
                crate::system_registry::torpedo_tube_system_id(&t.id),
            ) {
                (Some(cs), Some(id)) => !cs.0.is_offline(&id),
                _ => true,
            };
            let readiness = tube_readiness(t, effective_range, physics, target_live_pos, is_online);
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
                readiness,
                active_barrels: t.active_barrels.clone(),
                pattern_step: t.pattern_step,
                pattern_len: t.pattern_len(),
            }
        })
        .collect()
}

/// Shared torpedo-tube readiness projection (issue #764). Reused by the inline
/// broadcaster/resync builder in [`compute_current_weapons_update`] and by
/// [`build_tube_states`] so the tube contract cannot diverge between the two.
///
/// Range/arc use the tube's own `facing_deg`/`fire_arc_deg` (the same values
/// the torpedo fire path checks at launch) against the effective homing range.
/// A tube with a loaded round is gated on target range/arc; an empty tube
/// reports `Loading` while a round is loading and `NoAmmo` otherwise.
fn tube_readiness(
    tube: &crate::torpedo::TorpedoTube,
    effective_range: f32,
    physics: ShipPhysics,
    target_live_pos: Option<(f32, f32)>,
    is_online: bool,
) -> crate::messages::WeaponReadiness {
    let geometry = target_live_pos.map(|(tx, tz)| {
        crate::weapons::phaser::target_geometry(
            tx,
            tz,
            physics.x,
            physics.z,
            physics.yaw,
            effective_range,
            tube.facing_deg,
            tube.fire_arc_deg,
        )
    });
    let loading = tube.loaded_count == 0 && tube.load_state.label() == "loading";
    let no_ammo = tube.loaded_count == 0 && !loading;
    crate::messages::WeaponReadiness::evaluate(is_online, false, loading, no_ammo, geometry)
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
/// `target_uuid` is the ship's **Combat Lock**, read from its own frozen
/// `ViewscreenBlackboard::combat_lock` — never from the live
/// `TacticalRadarSelection` component, which only the tactical radar's own
/// publisher and the selection writers may touch (spec §3).
///
/// Two tiers of field, split by `Has<LocalShip>` in the loop:
///
/// - **Ship state** — `target_uuid`, `locked_target`, `target_name`, `banks`,
///   `tubes`, `torpedo_count`, and `blasters`. All derived from per-entity
///   components (or the ship's own blackboards) that NPCs carry, so they are
///   computed for every ship.
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
pub(crate) fn publish_weapons_core_blackboard(
    mut ship_q: Query<
        (
            Option<&ActiveBeam>,
            Option<&PhaserCooldown>,
            Option<&PhaserCombatConfigResource>,
            Option<&TorpedoSystemResource>,
            Option<&BlasterSystemResource>,
            Option<&ShipPhysics>,
            Option<&crate::modifiers::ShipModifiers>,
            Option<&ShipSystemControlSources>,
            &mut crate::server_app::ShipSystemBlackboards,
            Has<crate::server_app::LocalShip>,
        ),
        With<crate::server_app::Ship>,
    >,
    phaser_mode: Res<CurrentPhaserMode>,
    ship_config: Res<crate::lobby::server::ShipClientConfigResource>,
    entity_name_q: Query<(
        &crate::entity_spawner::EntityUuid,
        &crate::entities::spawner::EntityName,
    )>,
    asteroid_q: Query<(&AsteroidUuid, &Transform), Without<crate::entity_spawner::EntityUuid>>,
    entity_q: Query<(&crate::entity_spawner::EntityUuid, &Transform), Without<AsteroidUuid>>,
) {
    for (
        beam,
        cooldown,
        combat_config,
        torpedo_sys,
        blaster_res,
        ship_physics,
        modifiers,
        control_sources,
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
        // (`Physics`) both clear `TacticalRadarSelection` after a kill, later in the same
        // tick. Carrying the dead value forward would publish `locked_target !=
        // target_uuid` for one tick, contradicting the field's own contract (see
        // `WeaponsBlackboard::locked_target` in `core::messages`) that the two
        // agree once a tick has settled. Selection re-derives from `TacticalRadarSelection`
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

        // The **Combat Lock** from this ship's own frozen viewscreen blackboard
        // (spec §1/§3). Weapons is a cross-system consumer of the tactical
        // radar's selection, so it must not reach the live
        // `TacticalRadarSelection` component synchronously — it reads the
        // aggregated viewscreen fact like every other consumer (#829).
        //
        // This system runs in `SimSet::Publish` and the viewscreen aggregators
        // run in `SimSet::PublishAggregate`, so the value read here is the one
        // written LAST tick: a deliberate one-tick lag at 30Hz, accepted by
        // spec §1. `target_uuid` is display/observability only — nothing
        // downstream steers on it — so the lag costs a single frame of console
        // latency and nothing else.
        let target_uuid = viewscreen_combat_lock(&entity_bbs);
        let radar_range_mult = modifiers.get(&ModifierSlot::RadarRange);

        let target_live_pos: Option<(f32, f32)> = target_uuid
            .as_deref()
            .and_then(|uuid| live_entity_xz(uuid, &asteroid_q, &entity_q));

        // Liveness filter, for the same reason `locked_target` carries one
        // (above): the combat lock read here is last tick's, so a target killed
        // during this tick — or during the tick after the aggregator froze the
        // lock — would otherwise be published for one extra frame, breaking the
        // documented `locked_target == target_uuid` agreement and drawing a
        // reticle on a despawned entity. `target_live_pos` is exactly that
        // liveness probe, so this costs no extra lookup.
        let target_uuid = target_uuid.filter(|_| target_live_pos.is_some());

        let target_name: Option<String> = target_uuid.as_deref().and_then(|uuid| {
            entity_name_q
                .iter()
                .find_map(|(u, n)| (u.0 == uuid).then(|| n.0.clone()))
        });

        let banks: Vec<PhaserBankState> = build_bank_states(
            combat_config,
            cooldown,
            beam,
            radar_range_mult,
            physics,
            target_live_pos,
            control_sources,
        );

        let tubes: Vec<TorpedoTubeState> =
            build_tube_states(torpedo_sys, physics, target_live_pos, control_sources);

        // ── Client render data (LocalShip only) ──────────────────────────────
        // Phaser mode + arc geometry are drawn by the browser Tactical console
        // and are sourced from the two player-only resources. An NPC has no
        // client, so it gets empty vectors and the default phaser mode. Radar
        // blips + region overlays moved to `publish_tactical_radar_blackboard`
        // (issue #829) — they belong to the tactical-radar system now.
        let mut phaser_arcs: Vec<PhaserBankClientConfig> = Vec::new();
        let mut torpedo_arcs: Vec<TorpedoTubeClientConfig> = Vec::new();
        let mut mode = crate::messages::PhaserMode::default();

        if is_local {
            mode = phaser_mode.0;
            phaser_arcs = ship_config.0.phaser_banks.clone();
            torpedo_arcs = ship_config.0.torpedo_tubes.clone();
        }

        // Blaster bank states from this ship's own BlasterSystemResource, with
        // the shared readiness contract (issue #764) derived from this ship's
        // physics + frozen combat-lock target position + per-bank offline state.
        let blasters: Vec<BlasterBankState> = blaster_res
            .map(|r| {
                r.0.iter()
                    .map(|b| {
                        let is_online = control_sources
                            .and_then(|cs| {
                                crate::system_registry::blaster_bank_system_id(&b.config.id)
                                    .map(|id| !cs.0.is_offline(&id))
                            })
                            .unwrap_or(true);
                        b.bank_state(
                            physics.x,
                            physics.z,
                            physics.yaw,
                            target_live_pos,
                            is_online,
                        )
                    })
                    .collect()
            })
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

/// Publish each ship's Tactical Radar blackboard (issue #829). Runs in
/// `SimSet::Publish` alongside the other publishers — writes only the
/// `tactical-radar` key, so no ordering against them.
///
/// The tactical radar owns the **Combat Lock**: `selected_target` mirrors this
/// ship's `TacticalRadarSelection` component (its authoritative selection) for every
/// ship, so the viewscreen aggregator can lift it. `blips`/`regions` are the
/// expensive O(entities) client render data — computed for the `LocalShip`
/// only, exactly as they were in `publish_weapons_core_blackboard` before this
/// system took ownership. Reading the ship's own selection here is not a
/// cross-system read: this system *is* the tactical radar authority (spec §3).
pub(crate) fn publish_tactical_radar_blackboard(
    mut ship_q: Query<
        (
            Option<&TacticalRadarSelection>,
            Option<&ShipPhysics>,
            Option<&crate::modifiers::ShipModifiers>,
            &mut crate::server_app::ShipSystemBlackboards,
            Has<crate::server_app::LocalShip>,
        ),
        With<crate::server_app::Ship>,
    >,
    ship_config: Res<crate::lobby::server::ShipClientConfigResource>,
    world_res: Res<WorldResource>,
    asteroid_q: Query<(&AsteroidUuid, &Transform), Without<crate::entity_spawner::EntityUuid>>,
    entity_q: Query<(&crate::entity_spawner::EntityUuid, &Transform), Without<AsteroidUuid>>,
) {
    for (weapons_target, ship_physics, modifiers, mut entity_bbs, is_local) in ship_q.iter_mut() {
        let physics = ship_physics.copied().unwrap_or_default();
        let radar_range_mult = modifiers
            .map(|m| m.get(&ModifierSlot::RadarRange))
            .unwrap_or(1.0);
        let selected_target = weapons_target.and_then(|wt| wt.0.clone());

        let mut blips: Vec<RadarBlip> = Vec::new();
        let mut regions: Vec<RadarRegion> = Vec::new();

        if is_local {
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

        entity_bbs.0.insert(
            crate::system_registry::tactical_radar_system_id(),
            SystemBlackboard::TacticalRadar(crate::messages::TacticalRadarBlackboard {
                selected_target,
                blips,
                regions,
            }),
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
pub(crate) fn publish_phaser_bank_blackboards(
    mut ship_q: Query<
        (
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
    for (beam, cooldown, combat_config, ship_physics, modifiers, control_sources, mut entity_bbs) in
        ship_q.iter_mut()
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

        // Combat Lock from this ship's frozen viewscreen blackboard, exactly as
        // `publish_weapons_core_blackboard` reads it (spec §3) — never the live
        // `TacticalRadarSelection`. Drives the per-bank in-arc / in-range
        // `fire_ready` display state, which is observability only: the firing
        // paths (`handle_fire_phaser`, `ai_phaser_auto_fire`) do their own
        // in-arc check off the same combat lock and never read these entries.
        // One-tick lag, as above.
        let target_uuid = viewscreen_combat_lock(&entity_bbs);
        let target_live_pos: Option<(f32, f32)> = target_uuid
            .as_deref()
            .and_then(|uuid| live_entity_xz(uuid, &asteroid_q, &entity_q));
        let radar_range_mult = modifiers.get(&ModifierSlot::RadarRange);

        let banks = build_bank_states(
            combat_config,
            cooldown,
            beam,
            radar_range_mult,
            physics,
            target_live_pos,
            control_sources,
        );

        for bank_state in &banks {
            let Some(bank_sysid) = crate::system_registry::phaser_bank_system_id(&bank_state.id)
            else {
                continue;
            };
            let is_online = control_sources
                .map(|cs| !cs.0.is_offline(&bank_sysid))
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
pub(crate) fn publish_torpedo_tube_blackboards(
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

        // This system feeds only the fine `TorpedoTubeBlackboard`, which does
        // not carry the readiness contract, so the tube states' readiness field
        // is unused here — pass no physics/target (default geometry).
        let tubes = build_tube_states(torpedo_sys, ShipPhysics::default(), None, control_sources);
        for tube_state in &tubes {
            let Some(tube_sysid) = crate::system_registry::torpedo_tube_system_id(&tube_state.id)
            else {
                continue;
            };
            let is_online = control_sources
                .map(|cs| !cs.0.is_offline(&tube_sysid))
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
pub(crate) fn publish_torpedo_magazine_blackboard(
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

        let magazine_sysid = crate::system_registry::torpedo_magazine_system_id();
        let magazine_online = control_sources
            .map(|cs| !cs.0.is_offline(&magazine_sysid))
            .unwrap_or(true);
        entity_bbs.0.insert(
            magazine_sysid,
            SystemBlackboard::TorpedoMagazine(crate::messages::TorpedoMagazineBlackboard {
                is_online: magazine_online,
                torpedoes_remaining: torpedo_sys.0.torpedoes_remaining,
                capacity: torpedo_sys.0.config.count,
                // AC5: publish the in-flight count as a public authoritative fact
                // so other policies read it on the next AI tick.
                torpedoes_in_flight: torpedo_sys.0.in_flight.len() as u32,
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
    let cos_y = simmath::cos(ship_yaw);
    let sin_y = simmath::sin(ship_yaw);
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
