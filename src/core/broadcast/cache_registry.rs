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
//! This module is the transitional home of the delta caches that have not yet
//! moved to the stable-keyed replication lifecycle registry. It exposes the
//! legacy operations against those caches:
//!
//! - [`reset_unregistered`] — zero the caches still listed here. Used beside
//!   the lifecycle reset runner on `OnEnter(GamePhase::InProgress)`.
//! - [`resync_for_token`] — push a full-state snapshot to exactly one session
//!   token (used on a mid-game `Welcome`, i.e. a reconnect) **without**
//!   touching the shared caches, so every other client's next tick remains a
//!   normal delta and is not force-resent. This replaces the #599 quick fix,
//!   which reset the registry-covered caches globally — causing the *next* 10
//!   Hz tick to
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
//! Two remaining cache resources still live in this module:
//! [`LastBroadcastEntityPositions`] and [`LastBroadcastEntityHealth`]. They are re-exported
//! transitively through `crate::server_app` so existing
//! `ResMut<LastBroadcastX>` system parameters across the codebase are
//! unaffected by the move.
//!
//! [`crate::server_app::LastBroadcastBlackboards`] and
//! [`crate::server_app::LastBroadcastHull`] plus their reset/reconnect
//! projections have moved beside their live publishers. The generic lifecycle
//! runner invokes those owners without this module knowing their caches or
//! message shapes. Issue #1250 removed `LastBroadcastShields` when the live
//! Shields publisher moved to an owner-local broadcaster without a delta cache.
//!
//! `LastWeaponsUpdate` and its reset/reconnect projection now live entirely
//! beside the Weapons publisher. The registered adapter keeps this transitional
//! module from knowing either the cache resource or `WeaponsUpdate` shape.
//!
//! [`crate::console::repair::visibility::LastVisibleRepairBlackboard`] is a
//! separate per-token Repair projection cache owned by the Blackboard
//! lifecycle adapter. It is not part of this transitional registry or its
//! [`resync_for_token`] / [`prune`] operations.
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

use crate::core::messages::{ServerMessage, ShieldFacingStatus};
use crate::lobby::Target;

// ── Cache resources ─────────────────────────────────────────────────────────

/// Last-broadcast positions for non-asteroid entities (NPCs, stations).
/// Keyed by UUID string; value is (translation, yaw). Used by the
/// sim_state_broadcaster to skip sending position/yaw when unchanged.
#[derive(Resource, Default)]
pub struct LastBroadcastEntityPositions(pub HashMap<String, (bevy::math::Vec3, f32)>);

/// Last-broadcast health (hull_fraction, shield_fraction, shield facings,
/// shield frequency) for all entities. Keyed by UUID string. Used by the
/// sim_state_broadcaster to skip sending health/shield fields when they
/// haven't changed since the last broadcast, reducing wire payload for
/// stationary / undamaged NPCs.
///
/// Widened by issue #927 to also delta-compress the per-facing detail and
/// generator frequency `EntityStateSnapshot.shields` /
/// `.shield_freq` carry — until #927 `sim_state_broadcaster` always sent
/// `shields: None` and there was no `shield_freq` field at all, so
/// `target_shields` / `target_shield_freq` were empty on the wire for every
/// Sensors target regardless of which console rendered them.
#[derive(Resource, Default)]
pub struct LastBroadcastEntityHealth(
    pub  HashMap<
        String,
        (
            Option<f32>,
            Option<f32>,
            Option<Vec<ShieldFacingStatus>>,
            Option<f32>,
        ),
    >,
);

// ── Registry interface ──────────────────────────────────────────────────────

/// Zero the delta caches not yet owned by lifecycle adapters.
///
/// Called from `OnEnter(GamePhase::InProgress)` so the first broadcast tick
/// of a (re)started game always sends full state to all players — this also
/// covers the multi-game restart case where a stale cache from a previous
/// game would otherwise suppress initial updates.
pub fn reset_unregistered(world: &mut World) {
    *world.resource_mut::<LastBroadcastEntityPositions>() = LastBroadcastEntityPositions::default();
    *world.resource_mut::<LastBroadcastEntityHealth>() = LastBroadcastEntityHealth::default();
}

/// Remove a set of despawned entity UUIDs from the UUID-keyed caches.
///
/// Only [`LastBroadcastEntityPositions`] and [`LastBroadcastEntityHealth`]
/// are keyed by entity UUID; the other caches are ship-scoped or station-scoped
/// (not entity-scoped) and are untouched by prune. Hooked
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
/// Constructs the still-unregistered `ShieldStatus` directly from live
/// `LocalShip` component state. Registered lifecycle owners contribute their
/// projections in stable key order, without this runner knowing their cache or
/// payload types. The combined
/// batch is pushed into `SimOutbox` targeted at `Target::Token(token)`. Entity
/// positions/health for the wider world are already covered by the
/// `Welcome` message's `GameState.world` snapshot (built from the live
/// `WorldResource`), so this function does not duplicate that here.
/// `ShieldStatus` has no delta cache after issue #1250; reconnect builds that
/// one-shot projection directly from the live `ShipShields` component.
pub fn resync_for_token(world: &mut World, token: &str) {
    use crate::server_app::{LocalShip, SimOutbox};
    use crate::ship::shields::ShipShields;

    let target = Target::Token(token.to_string());
    let mut messages: Vec<ServerMessage> = Vec::new();

    // ── ShieldStatus: current shield facings.
    {
        let mut q = world.query_filtered::<&ShipShields, With<LocalShip>>();
        if let Ok(shields) = q.single(world) {
            let frequency = shields.frequency();
            let facings = crate::ship::shields::shield_facing_statuses(&shields.0.snapshot());
            messages.push(ServerMessage::ShieldStatus { facings, frequency });
        }
    }

    // Registered owner projections are invoked by stable key. This runner
    // deliberately knows none of their cache resources or message variants.
    messages.extend(crate::core::broadcast::reconnect_registered_replication(
        world, token,
    ));

    if !messages.is_empty() {
        let mut outbox = world.resource_mut::<SimOutbox>();
        outbox.extend_snapshot(
            messages
                .into_iter()
                .map(|message| (target.clone(), message)),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::console::repair::visibility::LastBroadcastHull;
    use crate::core::messages::{SystemBlackboard, SystemId};
    use crate::server_app::{
        LastBroadcastBlackboards, LocalShip, ShipSystemBlackboards, SimOutbox,
    };

    #[test]
    fn reset_unregistered_zeroes_each_remaining_cache() {
        let mut positions = LastBroadcastEntityPositions::default();
        positions
            .0
            .insert("uuid-1".into(), (bevy::math::Vec3::ZERO, 0.0));
        let mut health = LastBroadcastEntityHealth::default();
        health
            .0
            .insert("uuid-1".into(), (Some(1.0), Some(1.0), None, None));
        let mut app = App::new();
        app.insert_resource(positions);
        app.insert_resource(health);

        reset_unregistered(app.world_mut());

        assert!(
            app.world()
                .resource::<LastBroadcastEntityPositions>()
                .0
                .is_empty(),
            "positions cache must be empty after reset_unregistered"
        );
        assert!(
            app.world()
                .resource::<LastBroadcastEntityHealth>()
                .0
                .is_empty(),
            "health cache must be empty after reset_unregistered"
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
        health
            .0
            .insert("keep-1".into(), (Some(1.0), Some(1.0), None, None));
        health
            .0
            .insert("gone-1".into(), (Some(0.5), Some(0.5), None, None));
        health
            .0
            .insert("gone-2".into(), (Some(0.2), None, None, None));

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
        health
            .0
            .insert("keep-1".into(), (Some(1.0), Some(1.0), None, None));

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
            health.0.insert(uuid.clone(), (Some(1.0), None, None, None));

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
        app.init_resource::<crate::server_app::SimOutbox>();
        crate::server_app::register_blackboard_replication_lifecycle(&mut app);
        crate::console::repair::visibility::register_hull_replication_lifecycle(&mut app);
        let ship = app
            .world_mut()
            .spawn((
                crate::server_app::LocalShip,
                crate::server_app::ShipSystemBlackboards::default(),
                crate::ship_plugin::ShipConfigComponent::default(),
                crate::ship::shields::ShipShields(
                    crate::weapons::shield::ShieldSystem::default(),
                    0.5,
                ),
                crate::entities::spawner::EntitySystemHull(
                    crate::ship::damage::SystemHull::from_config(&[(
                        SystemId("helm".into()),
                        25.0,
                    )]),
                ),
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
                let value = SystemBlackboard::Helm(crate::core::messages::HelmBlackboard {
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
                    hostile_weapon_arcs: Vec::new(),
                });
                // Deliberately insert in non-key order. The reconnect payload
                // must contain every current entry in stable SystemId order.
                for id in ["zulu", "alpha", "middle"] {
                    bbs.0.insert(SystemId(id.into()), value.clone());
                }
            }
        }

        resync_for_token(app.world_mut(), "reconnector");

        let entries = app.world_mut().resource_mut::<SimOutbox>().drain();
        assert!(
            !entries.is_empty(),
            "resync_for_token must push at least one message"
        );
        for entry in &entries {
            assert_eq!(
                &entry.target,
                &Target::Token("reconnector".to_string()),
                "every resync message must target only the reconnecting token"
            );
            assert_eq!(
                entry.delivery,
                crate::core::messages::DeliveryClass::Snapshot,
                "every reconnect projection must use the Snapshot channel"
            );
        }
        let blackboard_ids = entries.iter().find_map(|entry| match &entry.message {
            ServerMessage::BlackboardUpdate { updates } => Some(
                updates
                    .iter()
                    .map(|(id, _)| id.0.as_str())
                    .collect::<Vec<_>>(),
            ),
            _ => None,
        });
        assert_eq!(
            blackboard_ids,
            Some(vec!["alpha", "middle", "zulu"]),
            "resync must include every current Blackboard in stable SystemId order"
        );
        assert!(
            entries
                .iter()
                .any(|entry| matches!(&entry.message, ServerMessage::SystemHullUpdate { .. })),
            "the registered Hull owner must contribute its reconnect projection"
        );
    }

    #[test]
    fn resync_for_token_does_not_touch_shared_caches() {
        let mut app = resync_test_app();
        app.init_resource::<LastBroadcastBlackboards>();
        // Seed the shared caches as if a prior tick already broadcast state.
        app.world_mut()
            .resource_mut::<LastBroadcastBlackboards>()
            .0
            .insert(
                SystemId("helm".into()),
                SystemBlackboard::Helm(crate::core::messages::HelmBlackboard {
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
                    hostile_weapon_arcs: Vec::new(),
                }),
            );
        {
            let mut hull = app.world_mut().resource_mut::<LastBroadcastHull>();
            hull.0.insert(
                "existing-client".into(),
                crate::console::repair::visibility::HullProjection {
                    entries: vec![crate::core::messages::SystemHullStatus {
                        system_id: SystemId("helm".into()),
                        display_name: "Helm".into(),
                        current: 10.0,
                        max_hp: 25.0,
                        tier: crate::ship::damage::DamageTier::Damaged,
                        debuff_magnitude: 0.5,
                    }],
                    aggregate_fraction: Some(0.4),
                    destroyed_fraction: Some(0.0),
                },
            );
        }
        {
            let mut projected = app
                .world_mut()
                .resource_mut::<crate::console::repair::visibility::LastVisibleRepairBlackboard>(
            );
            projected.projections.insert(
                "existing-client".into(),
                crate::core::messages::RepairBlackboard::default(),
            );
            projected.stations.insert(
                "existing-client".into(),
                Some(crate::core::messages::StationId("engineering".into())),
            );
        }

        resync_for_token(app.world_mut(), "reconnector");

        // The shared cache must be untouched — other clients' next tick
        // still diffs against this pre-existing value, i.e. resync must not
        // reset it (that was the #599 bug this replaces).
        let cache = app.world().resource::<LastBroadcastBlackboards>();
        assert_eq!(
            cache.0.get(&SystemId("helm".into())),
            Some(&SystemBlackboard::Helm(
                crate::core::messages::HelmBlackboard {
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
                    hostile_weapon_arcs: Vec::new(),
                }
            )),
            "resync_for_token must not mutate the shared LastBroadcastBlackboards cache"
        );
        let hull = app.world().resource::<LastBroadcastHull>();
        assert_eq!(
            hull.0
                .get("existing-client")
                .and_then(|projection| projection.entries.first())
                .map(|entry| (entry.system_id.0.as_str(), entry.current)),
            Some(("helm", 10.0)),
            "reconnect must not mutate another client's live Hull projection"
        );
        let projected = app
            .world()
            .resource::<crate::console::repair::visibility::LastVisibleRepairBlackboard>();
        assert!(projected.projections.contains_key("existing-client"));
        assert_eq!(
            projected
                .stations
                .get("existing-client")
                .and_then(Option::as_ref)
                .map(|station| station.0.as_str()),
            Some("engineering"),
            "reconnect must not invalidate another client's live Repair projection"
        );
    }
}
