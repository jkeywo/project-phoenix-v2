//! Broadcast delta-cache registry (issue #613).
//!
//! ## Why this module exists
//!
//! Several broadcast producers skip re-sending unchanged state by comparing
//! the current tick's computed value against a "last broadcast" cache
//! (`Resource`). Before this module, each cache was reset ad hoc from two
//! call sites (`reset_broadcast_caches_on_start`, and the #599 quick fix
//! `refresh_caches_on_midgame_reconnect`), and nothing ever pruned per-UUID
//! cache entries for despawned entities — respawning asteroids get a fresh
//! UUID every cycle, so their old health-cache entries lived forever.
//!
//! This module is the single place that knows about all six delta caches and
//! exposes three operations against them:
//!
//! - [`reset_all`] — zero every cache. Used on `OnEnter(GamePhase::InProgress)`
//!   (covers the multi-game restart case where stale cache from a previous
//!   game would otherwise suppress the first tick's updates).
//! - [`resync_for_token`] — push a full-state snapshot to exactly one session
//!   token (used on a mid-game `Welcome`, i.e. a reconnect) **without**
//!   touching the shared caches, so every other client's next tick remains a
//!   normal delta and is not force-resent. This replaces the #599 quick fix,
//!   which reset every cache globally — causing the *next* 10 Hz tick to
//!   broadcast full state to *all* clients whenever *anyone* reconnected.
//! - [`prune`] — remove a set of despawned entity UUIDs from the two
//!   UUID-keyed caches (`LastBroadcastEntityPositions`,
//!   `LastBroadcastEntityHealth`; non-UUID caches are untouched since they
//!   aren't keyed by entity). Hooked into the three despawn/reconciliation
//!   paths: asteroid destruction, asteroid window-eviction, and generic
//!   runtime-entity reconciliation.
//!
//! ## What it owns
//!
//! The five UUID-agnostic / ship-scoped cache resources live in this module:
//! [`LastBroadcastEntityPositions`], [`LastBroadcastEntityHealth`],
//! [`LastBroadcastHull`], [`LastBroadcastShields`],
//! [`LastBroadcastBlackboards`]. They are re-exported from `server_app` (and
//! transitively `crate::simulation`) so existing `ResMut<LastBroadcastX>`
//! system parameters across the codebase are unaffected by the move.
//!
//! The sixth cache, `LastWeaponsUpdate` (`src/console/weapons/server.rs`),
//! stays defined in its natural home next to the weapons producer that reads
//! and writes it every tick — moving the type would ripple through that
//! file's producer closure for no behavioural benefit. This registry's
//! `reset_all` still resets it (via a `ResMut` parameter), so the registry's
//! *interface* still covers all six caches even though one struct definition
//! lives elsewhere, exactly as the module-level docs on
//! `src/ship/system_registry.rs` describe conventions without forcing every
//! consumer through one physical type.
//!
//! ## Session bookkeeping (PRD story 9)
//!
//! See the doc comment on [`crate::lobby::session::SessionManager`] for the
//! decision to *not* prune disconnected/station-less session records: a
//! `Player` entry keyed by token is the only way a later `Identify` for that
//! same token gets matched back to `reconnect()` instead of falling into
//! `register()` as a brand-new player (losing `last_rating` / prior identity
//! continuity). Growth is bounded in practice by the fixed session-count use
//! case (a small fixed roster per ship, not an unbounded public server), so
//! we punt on active pruning here rather than risk breaking reconnect
//! semantics for a bookkeeping optimisation with no observed real cost.

use bevy::prelude::*;
use std::collections::HashMap;

use crate::lobby::Target;
use crate::messages::{ServerMessage, ShieldFacingStatus, SystemBlackboard, SystemId};

// ── Cache resources ─────────────────────────────────────────────────────────

/// Last-broadcast positions for non-asteroid entities (NPCs, stations).
/// Keyed by UUID string; value is (translation, yaw). Used by the
/// sim_state_broadcaster to skip sending position/yaw when unchanged.
#[derive(Resource, Default)]
pub struct LastBroadcastEntityPositions(pub HashMap<String, (bevy::math::Vec3, f32)>);

/// Last-broadcast health (hull_fraction, shield_fraction) for all entities.
/// Keyed by UUID string. Used by the sim_state_broadcaster to skip sending
/// health fields when they haven't changed since the last broadcast, reducing
/// wire payload for stationary / undamaged NPCs.
#[derive(Resource, Default)]
pub struct LastBroadcastEntityHealth(pub HashMap<String, (Option<f32>, Option<f32>)>);

/// Last-broadcast per-system hull state. When the hull changes, a
/// `SystemHullUpdate` event message is emitted and this cache is updated.
#[derive(Resource, Default)]
pub struct LastBroadcastHull(pub Vec<crate::messages::SystemHullStatus>);

/// Last-broadcast shield facings. Used to suppress the per-tick `ShieldStatus`
/// broadcast to all players when nothing has changed.
#[derive(Resource, Default)]
pub struct LastBroadcastShields(pub Vec<ShieldFacingStatus>);

/// Last-broadcast blackboard state per system. The `broadcast_blackboard_updates`
/// system compares `ShipSystemBlackboards` against this and only emits a
/// `BlackboardUpdate` for systems whose blackboard has changed.
#[derive(Resource, Default)]
pub struct LastBroadcastBlackboards(pub HashMap<SystemId, SystemBlackboard>);

// ── Registry interface ──────────────────────────────────────────────────────

/// Zero all six broadcast delta caches.
///
/// Called from `OnEnter(GamePhase::InProgress)` so the first broadcast tick
/// of a (re)started game always sends full state to all players — this also
/// covers the multi-game restart case where a stale cache from a previous
/// game would otherwise suppress initial updates.
pub fn reset_all(
    hull: &mut LastBroadcastHull,
    shields: &mut LastBroadcastShields,
    positions: &mut LastBroadcastEntityPositions,
    health: &mut LastBroadcastEntityHealth,
    weapons: &mut crate::console::weapons::server::LastWeaponsUpdate,
    blackboards: &mut LastBroadcastBlackboards,
) {
    *hull = LastBroadcastHull::default();
    *shields = LastBroadcastShields::default();
    *positions = LastBroadcastEntityPositions::default();
    *health = LastBroadcastEntityHealth::default();
    *weapons = crate::console::weapons::server::LastWeaponsUpdate::default();
    *blackboards = LastBroadcastBlackboards::default();
}

/// Remove a set of despawned entity UUIDs from the UUID-keyed caches.
///
/// Only [`LastBroadcastEntityPositions`] and [`LastBroadcastEntityHealth`]
/// are keyed by entity UUID; the other four caches are ship-scoped or
/// station-scoped (not entity-scoped) and are untouched by prune. Hooked
/// into the despawn/reconciliation paths so respawning entities that reuse
/// UUIDs (they don't — new UUIDs every cycle) or entities that simply vanish
/// (asteroids, runtime-spawned entities) don't leave permanent cache entries
/// behind. This is the fix for unbounded cache growth: a long-running server
/// cycling through thousands of asteroid spawn/despawn events would
/// otherwise accumulate one stale entry per historical UUID forever.
pub fn prune(
    positions: &mut LastBroadcastEntityPositions,
    health: &mut LastBroadcastEntityHealth,
    uuids: &[String],
) {
    for uuid in uuids {
        positions.0.remove(uuid);
        health.0.remove(uuid);
    }
}

/// Push a full-state resync targeted at exactly one session token.
///
/// Used on a mid-game `Welcome` (i.e. a reconnect) so the reconnecting
/// client gets fresh full state without disturbing the shared delta caches
/// — every other client's next broadcast tick remains a normal delta, not a
/// forced full resend. This replaces the #599 quick fix
/// (`refresh_caches_on_midgame_reconnect`), which zeroed the shared caches
/// and thus caused the *next* tick to broadcast full state to *all* clients.
///
/// Constructs `SystemHullUpdate`, `ShieldStatus`, `BlackboardUpdate`, and
/// (when the reconnecting token currently holds the Tactical station)
/// `WeaponsUpdate` directly from live `LocalShip` component state (the same
/// computations the regular cache-diffing producers use) and pushes them
/// into `SimOutbox` targeted at `Target::Token(token)`. Entity
/// positions/health for the wider world are already covered by the
/// `Welcome` message's `GameState.world` snapshot (built from the live
/// `WorldResource`), so this function does not duplicate that here.
///
/// `WeaponsUpdate` is gated on station ownership (unlike the other three
/// messages, which are unconditional) because, unlike hull/shields/
/// blackboards, it is genuinely station-scoped: it's normally only ever sent
/// to whoever holds the ship's weapons station (`Audience::HoldingWeapons` in
/// `weapons_update_broadcaster`), so a reconnecting client who does not hold
/// that station has no use for it and should not receive it here either.
/// The owning station is resolved from the ship config — it's "tactical" on
/// the crewed hulls but "pilot" on the single-station Courier.
/// This deliberately does **not** touch `LastWeaponsUpdate` — same rule as
/// the other three shared caches, so the next periodic broadcaster tick
/// still diffs normally instead of being forced to re-send to everyone.
pub fn resync_for_token(world: &mut World, token: &str) {
    use crate::console::weapons::server::compute_current_weapons_update;
    use crate::entity_spawner::EntitySystemHull;
    use crate::lobby::Sessions;
    use crate::messages::StationId;
    use crate::ship::shields::ShipShields;
    use crate::simulation::{LocalShip, ShipSystemBlackboards, SimOutbox};

    let target = Target::Token(token.to_string());
    let mut messages: Vec<ServerMessage> = Vec::new();

    // ── SystemHullUpdate: current per-system hull for the reconnecting client's ship.
    {
        let mut q = world.query_filtered::<&EntitySystemHull, With<LocalShip>>();
        if let Ok(hull) = q.single(world) {
            let entries: Vec<crate::messages::SystemHullStatus> = hull
                .0
                .iter()
                .map(|(sid, entry)| crate::messages::SystemHullStatus {
                    system_id: sid.clone(),
                    display_name: entry.display_name.clone(),
                    current: entry.current,
                    max_hp: entry.max,
                    tier: hull.0.tier_for(sid),
                    debuff_magnitude: hull.0.debuff_magnitude_for(sid),
                })
                .collect();
            messages.push(ServerMessage::SystemHullUpdate { entries });
        }
    }

    // ── ShieldStatus: current shield facings.
    {
        let mut q = world.query_filtered::<&ShipShields, With<LocalShip>>();
        if let Ok(shields) = q.single(world) {
            let frequency = shields.frequency();
            let facings: Vec<ShieldFacingStatus> = shields
                .0
                .snapshot()
                .into_iter()
                .map(|s| ShieldFacingStatus {
                    label: s.label,
                    hp: s.hp,
                    max_hp: s.max_hp,
                    online: s.online,
                    offline_remaining: s.offline_remaining,
                    is_focused: s.is_focused,
                    center_deg: s.center_deg,
                    width_deg: s.width_deg,
                    arc_id: s.id,
                    priority: s.priority,
                })
                .collect();
            messages.push(ServerMessage::ShieldStatus { facings, frequency });
        }
    }

    // ── BlackboardUpdate: every current blackboard, not just changed ones.
    {
        let mut q = world.query_filtered::<&ShipSystemBlackboards, With<LocalShip>>();
        if let Ok(bb) = q.single(world) {
            let updates: Vec<(SystemId, SystemBlackboard)> =
                bb.0.iter()
                    .map(|(id, bb)| (id.clone(), bb.clone()))
                    .collect();
            if !updates.is_empty() {
                messages.push(ServerMessage::BlackboardUpdate { updates });
            }
        }
    }

    // ── WeaponsUpdate: only meaningful if the reconnecting token currently
    // holds the Tactical station — WeaponsUpdate is normally only ever sent
    // to that holder (Audience::Holding), so anyone else has no use for it.
    // Deliberately does not read/write `LastWeaponsUpdate`.
    {
        // Resolve the weapons owner from the ship config, then check the
        // holder. The StationId is cloned out of the query before touching
        // Sessions so the query borrow on `world` is released first.
        //
        // Falls back to "tactical" when the config can't answer — either no
        // ShipConfigComponent or an empty default one. A real LocalShip always
        // carries a populated config, so this only covers the pre-spawn window;
        // defaulting to the historical station keeps a reconnecting player from
        // silently losing their WeaponsUpdate if that ever changes.
        let weapons_station: StationId = world
            .query_filtered::<&crate::ship_plugin::ShipConfigComponent, With<LocalShip>>()
            .single(world)
            .ok()
            .and_then(|c| c.0.weapons_station())
            .unwrap_or_else(|| StationId(crate::system_registry::TACTICAL_SYSTEM_ID.into()));
        let holds_tactical = world
            .resource::<Sessions>()
            .0
            .holder_for_station(&weapons_station)
            == Some(token);
        if holds_tactical {
            let current = compute_current_weapons_update(world);
            messages.push(ServerMessage::WeaponsUpdate {
                target_uuid: current.target_uuid,
                target_name: current.target_name,
                banks: current.banks,
                tubes: current.tubes,
                torpedo_count: current.torpedo_count,
                phaser_mode: current.phaser_mode,
                blasters: current.blasters,
                phaser_frequency: current.phaser_frequency,
            });
        }
    }

    if !messages.is_empty() {
        let mut outbox = world.resource_mut::<SimOutbox>();
        for msg in messages {
            outbox.0.push((target.clone(), msg));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::simulation::{LocalShip, ShipSystemBlackboards, SimOutbox};

    #[test]
    fn reset_all_zeroes_every_cache() {
        let mut hull = LastBroadcastHull(vec![]);
        hull.0.push(crate::messages::SystemHullStatus {
            system_id: SystemId("helm".into()),
            display_name: "Helm".into(),
            current: 10.0,
            max_hp: 25.0,
            tier: crate::damage::DamageTier::Damaged,
            debuff_magnitude: 0.5,
        });
        let mut shields = LastBroadcastShields(vec![ShieldFacingStatus {
            label: "Fore".into(),
            hp: 50,
            max_hp: 100,
            online: true,
            offline_remaining: 0.0,
            is_focused: false,
            center_deg: 0.0,
            width_deg: 90.0,
            arc_id: "fore".into(),
            priority: 1,
        }]);
        let mut positions = LastBroadcastEntityPositions::default();
        positions
            .0
            .insert("uuid-1".into(), (bevy::math::Vec3::ZERO, 0.0));
        let mut health = LastBroadcastEntityHealth::default();
        health.0.insert("uuid-1".into(), (Some(1.0), Some(1.0)));
        let mut weapons = crate::console::weapons::server::LastWeaponsUpdate {
            target_uuid: Some("uuid-1".into()),
            ..Default::default()
        };
        let mut blackboards = LastBroadcastBlackboards::default();
        blackboards.0.insert(
            SystemId("helm".into()),
            SystemBlackboard::Helm(crate::messages::HelmBlackboard {
                yaw: 1.0,
                forward_speed: 1.0,
                x: 1.0,
                z: 1.0,
                impulse_charge: 1.0,
                boost_battery: 1.0,
                boost_active: true,
                boost_enabled: true,
                radar_range: 0.0,
                lateral_speed: 0.0,
            }),
        );

        reset_all(
            &mut hull,
            &mut shields,
            &mut positions,
            &mut health,
            &mut weapons,
            &mut blackboards,
        );

        assert!(
            hull.0.is_empty(),
            "hull cache must be empty after reset_all"
        );
        assert!(
            shields.0.is_empty(),
            "shields cache must be empty after reset_all"
        );
        assert!(
            positions.0.is_empty(),
            "positions cache must be empty after reset_all"
        );
        assert!(
            health.0.is_empty(),
            "health cache must be empty after reset_all"
        );
        assert_eq!(
            weapons,
            crate::console::weapons::server::LastWeaponsUpdate::default(),
            "weapons cache must be default after reset_all"
        );
        assert!(
            blackboards.0.is_empty(),
            "blackboards cache must be empty after reset_all"
        );
    }

    #[test]
    fn prune_removes_exactly_given_uuids_and_nothing_else() {
        let mut positions = LastBroadcastEntityPositions::default();
        positions
            .0
            .insert("keep-1".into(), (bevy::math::Vec3::ZERO, 0.0));
        positions
            .0
            .insert("gone-1".into(), (bevy::math::Vec3::ZERO, 0.0));
        positions
            .0
            .insert("gone-2".into(), (bevy::math::Vec3::ZERO, 0.0));

        let mut health = LastBroadcastEntityHealth::default();
        health.0.insert("keep-1".into(), (Some(1.0), Some(1.0)));
        health.0.insert("gone-1".into(), (Some(0.5), Some(0.5)));
        health.0.insert("gone-2".into(), (Some(0.2), None));

        prune(
            &mut positions,
            &mut health,
            &["gone-1".to_string(), "gone-2".to_string()],
        );

        assert_eq!(
            positions.0.len(),
            1,
            "expected exactly one surviving position entry"
        );
        assert!(positions.0.contains_key("keep-1"));
        assert!(!positions.0.contains_key("gone-1"));
        assert!(!positions.0.contains_key("gone-2"));

        assert_eq!(
            health.0.len(),
            1,
            "expected exactly one surviving health entry"
        );
        assert!(health.0.contains_key("keep-1"));
        assert!(!health.0.contains_key("gone-1"));
        assert!(!health.0.contains_key("gone-2"));
    }

    #[test]
    fn prune_of_unknown_uuid_is_noop() {
        let mut positions = LastBroadcastEntityPositions::default();
        positions
            .0
            .insert("keep-1".into(), (bevy::math::Vec3::ZERO, 0.0));
        let mut health = LastBroadcastEntityHealth::default();
        health.0.insert("keep-1".into(), (Some(1.0), Some(1.0)));

        prune(&mut positions, &mut health, &["never-existed".to_string()]);

        assert_eq!(positions.0.len(), 1);
        assert_eq!(health.0.len(), 1);
    }

    #[test]
    fn long_session_spawn_despawn_cycle_leaves_cache_bounded() {
        // Simulate many spawn/despawn cycles of asteroids that each get a
        // fresh UUID (as real asteroid respawns do). Without prune, the
        // health/position caches would grow by one entry per historical
        // UUID forever. With prune called on each despawn, the cache size
        // stays bounded by the number of *currently live* entities.
        let mut positions = LastBroadcastEntityPositions::default();
        let mut health = LastBroadcastEntityHealth::default();

        const LIVE_AT_ONCE: usize = 5;
        const CYCLES: usize = 500;

        for cycle in 0..CYCLES {
            let uuid = format!("asteroid-{cycle}");
            positions
                .0
                .insert(uuid.clone(), (bevy::math::Vec3::ZERO, 0.0));
            health.0.insert(uuid.clone(), (Some(1.0), None));

            // Once more than LIVE_AT_ONCE entries have ever been inserted,
            // despawn (and prune) the oldest one so the live set stays
            // bounded, mirroring a ring-buffer window that keeps a fixed
            // number of asteroids active at a time.
            if cycle >= LIVE_AT_ONCE {
                let despawned_uuid = format!("asteroid-{}", cycle - LIVE_AT_ONCE);
                prune(&mut positions, &mut health, &[despawned_uuid]);
            }
        }

        assert_eq!(
            positions.0.len(),
            LIVE_AT_ONCE,
            "positions cache must stay bounded across {CYCLES} spawn/despawn cycles, got {}",
            positions.0.len()
        );
        assert_eq!(
            health.0.len(),
            LIVE_AT_ONCE,
            "health cache must stay bounded across {CYCLES} spawn/despawn cycles, got {}",
            health.0.len()
        );
    }

    // ── resync_for_token ────────────────────────────────────────────────

    fn resync_test_app() -> App {
        let mut app = App::new();
        app.add_plugins(crate::lobby::LobbyPlugin);
        app.init_resource::<crate::simulation::SimOutbox>();
        let ship = app
            .world_mut()
            .spawn((
                crate::simulation::LocalShip,
                crate::simulation::ShipSystemBlackboards::default(),
                crate::ship::shields::ShipShields(
                    crate::weapons::shield::ShieldSystem::default(),
                    0.5,
                ),
                crate::entity_spawner::EntitySystemHull(crate::damage::SystemHull::from_config(&[
                    (SystemId("helm".into()), 25.0),
                ])),
            ))
            .id();
        let _ = ship;
        app
    }

    #[test]
    fn resync_for_token_targets_only_that_token() {
        let mut app = resync_test_app();

        {
            let mut q = app
                .world_mut()
                .query_filtered::<&mut ShipSystemBlackboards, With<LocalShip>>();
            if let Ok(mut bbs) = q.single_mut(app.world_mut()) {
                bbs.0.insert(
                    SystemId("helm".into()),
                    SystemBlackboard::Helm(crate::messages::HelmBlackboard {
                        yaw: 0.0,
                        forward_speed: 0.0,
                        x: 0.0,
                        z: 0.0,
                        impulse_charge: 0.0,
                        boost_battery: 1.0,
                        boost_active: false,
                        boost_enabled: true,
                        radar_range: 0.0,
                        lateral_speed: 0.0,
                    }),
                );
            }
        }

        resync_for_token(app.world_mut(), "reconnector");

        let outbox = app.world().resource::<SimOutbox>();
        assert!(
            !outbox.0.is_empty(),
            "resync_for_token must push at least one message"
        );
        for (target, _msg) in &outbox.0 {
            assert_eq!(
                *target,
                Target::Token("reconnector".to_string()),
                "every resync message must target only the reconnecting token"
            );
        }
        let has_bb_update = outbox
            .0
            .iter()
            .any(|(_, msg)| matches!(msg, ServerMessage::BlackboardUpdate { .. }));
        assert!(
            has_bb_update,
            "resync must include a BlackboardUpdate with full blackboard state"
        );
    }

    #[test]
    fn resync_for_token_does_not_touch_shared_caches() {
        let mut app = resync_test_app();
        app.init_resource::<LastBroadcastBlackboards>();
        app.init_resource::<LastBroadcastHull>();
        app.init_resource::<LastBroadcastShields>();

        // Seed the shared caches as if a prior tick already broadcast state.
        app.world_mut()
            .resource_mut::<LastBroadcastBlackboards>()
            .0
            .insert(
                SystemId("helm".into()),
                SystemBlackboard::Helm(crate::messages::HelmBlackboard {
                    yaw: 5.0,
                    forward_speed: 5.0,
                    x: 5.0,
                    z: 5.0,
                    impulse_charge: 5.0,
                    boost_battery: 5.0,
                    boost_active: true,
                    boost_enabled: true,
                    radar_range: 0.0,
                    lateral_speed: 0.0,
                }),
            );

        resync_for_token(app.world_mut(), "reconnector");

        // The shared cache must be untouched — other clients' next tick
        // still diffs against this pre-existing value, i.e. resync must not
        // reset it (that was the #599 bug this replaces).
        let cache = app.world().resource::<LastBroadcastBlackboards>();
        assert_eq!(
            cache.0.get(&SystemId("helm".into())),
            Some(&SystemBlackboard::Helm(crate::messages::HelmBlackboard {
                yaw: 5.0,
                forward_speed: 5.0,
                x: 5.0,
                z: 5.0,
                impulse_charge: 5.0,
                boost_battery: 5.0,
                boost_active: true,
                boost_enabled: true,
                radar_range: 0.0,
                lateral_speed: 0.0,
            })),
            "resync_for_token must not mutate the shared LastBroadcastBlackboards cache"
        );
    }

    // ── resync_for_token: WeaponsUpdate (issue #613 review fix) ────────────

    /// Build a `resync_test_app` whose ship carries the resources
    /// `compute_current_weapons_update` unconditionally reads (torpedo
    /// system, phaser mode, phaser combat config) and register `token` as
    /// the current holder of the Tactical station, so `resync_for_token`
    /// takes the "reconnecting client holds Tactical" branch.
    fn resync_test_app_with_tactical_holder(token: &str) -> App {
        use crate::console::weapons::server::{
            CurrentPhaserMode, PhaserCombatConfigResource, TorpedoSystemResource, WeaponsTarget,
        };
        use crate::lobby::Sessions;
        use crate::messages::StationId;
        use crate::weapons::torpedo::{TorpedoConfig, TorpedoSystem};

        let mut app = resync_test_app();
        app.insert_resource(CurrentPhaserMode(crate::messages::PhaserMode::Manual));
        app.insert_resource(PhaserCombatConfigResource::default());
        app.world_mut()
            .resource_mut::<Sessions>()
            .0
            .register(token.to_string(), "Reconnecting Player".to_string())
            .unwrap();
        app.world_mut()
            .resource_mut::<Sessions>()
            .0
            .set_station(token, Some(StationId("tactical".into())));

        let mut q = app.world_mut().query_filtered::<Entity, With<LocalShip>>();
        let ship = q.single(app.world()).expect("LocalShip must exist");
        app.world_mut().entity_mut(ship).insert((
            WeaponsTarget(Some("target-uuid".into())),
            TorpedoSystemResource(TorpedoSystem::new(TorpedoConfig::default())),
        ));
        app
    }

    #[test]
    fn resync_for_token_includes_weapons_update_for_tactical_holder() {
        let mut app = resync_test_app_with_tactical_holder("reconnector");

        resync_for_token(app.world_mut(), "reconnector");

        let outbox = app.world().resource::<SimOutbox>();
        let weapons_update = outbox.0.iter().find_map(|(target, msg)| match msg {
            ServerMessage::WeaponsUpdate {
                target_uuid,
                phaser_mode,
                ..
            } => Some((target.clone(), target_uuid.clone(), *phaser_mode)),
            _ => None,
        });
        let (target, target_uuid, phaser_mode) = weapons_update.expect(
            "resync_for_token must include a WeaponsUpdate for a Tactical-holding reconnector",
        );
        assert_eq!(
            target,
            Target::Token("reconnector".to_string()),
            "WeaponsUpdate resync must target only the reconnecting token"
        );
        assert_eq!(
            target_uuid.as_deref(),
            Some("target-uuid"),
            "WeaponsUpdate resync must reflect the ship's current locked target"
        );
        assert_eq!(
            phaser_mode,
            crate::messages::PhaserMode::Manual,
            "WeaponsUpdate resync must reflect the ship's current phaser mode"
        );
    }

    #[test]
    fn resync_for_token_omits_weapons_update_for_non_tactical_holder() {
        // "reconnector" does not hold any station in the plain `resync_test_app`
        // fixture (SessionManager::new() has no registered players at all), so
        // WeaponsUpdate must not appear — this is the "no station -> no weapons
        // resync" branch that keeps a reconnecting non-Tactical player from
        // getting a message meant only for whoever currently holds Tactical.
        let mut app = resync_test_app();

        resync_for_token(app.world_mut(), "reconnector");

        let outbox = app.world().resource::<SimOutbox>();
        let has_weapons_update = outbox
            .0
            .iter()
            .any(|(_, msg)| matches!(msg, ServerMessage::WeaponsUpdate { .. }));
        assert!(
            !has_weapons_update,
            "resync_for_token must not send WeaponsUpdate to a reconnector who does not hold Tactical"
        );
    }

    #[test]
    fn resync_for_token_does_not_touch_last_weapons_update_cache() {
        use crate::console::weapons::server::LastWeaponsUpdate;

        let mut app = resync_test_app_with_tactical_holder("reconnector");
        app.init_resource::<LastWeaponsUpdate>();

        // Seed the shared weapons cache as if a prior tick already broadcast
        // this exact state — if resync_for_token wrote to this cache (it must
        // not), the next periodic broadcaster tick would wrongly treat this
        // reconnect resync as its own "last sent" baseline.
        let seeded = LastWeaponsUpdate {
            target_uuid: Some("stale-target".into()),
            target_name: Some("Stale Target".into()),
            banks: vec![],
            tubes: vec![],
            torpedo_count: 7,
            phaser_mode: crate::messages::PhaserMode::Auto,
            blasters: vec![],
            phaser_frequency: 0.5,
        };
        *app.world_mut().resource_mut::<LastWeaponsUpdate>() = seeded.clone();

        resync_for_token(app.world_mut(), "reconnector");

        let cache = app.world().resource::<LastWeaponsUpdate>();
        assert_eq!(
            *cache, seeded,
            "resync_for_token must not mutate the shared LastWeaponsUpdate cache"
        );
    }
}
