//! Broadcast/publish/HUD systems (issue #1199; split out of `broadcast.rs`
//! itself — issue #1241 — which kept only the sim-state snapshot builders
//! once this half grew large enough on its own to justify a sibling module).
//!
//! Public surface: the `SimBroadcaster` factories [`modifier_events_broadcaster`]
//! and [`sim_outbox_broadcaster`]; and the publish systems
//! (`publish_viewscreen_blackboard`, `broadcast_blackboard_updates`,
//! the game-over / world-setup / cache-reset /
//! reconnect broadcasts, and `reconcile_runtime_entities`). All re-exported
//! through `crate::server_app`.
//!
//! Role: reacts to authoritative tick state — draining event queues, folding
//! deltas against the caches, publishing blackboards and HUD messages — as
//! opposed to [`super::broadcast`], which builds the `SimState` snapshot data
//! this module's systems and caches are downstream of. Neither module calls
//! into the other's functions.
//!
//! Load-bearing invariant: anything collected from a `HashMap` before it
//! reaches the wire is sorted (blackboard updates, spawned ids, boost pairs)
//! so two seeded runs emit byte-identical broadcasts; the delta caches are
//! digest exclusions, so the suppression logic here never feeds the digest.

use super::*;

/// Stable lifecycle key for the wire projection of per-System blackboards.
pub(crate) const BLACKBOARD_REPLICATION_KEY: &str = "blackboards";

/// Last live-broadcast blackboard state per System.
///
/// The live publisher compares `ShipSystemBlackboards` against this cache and
/// emits only entries that changed. Its run reset and cache-independent
/// reconnect projection are registered beside the publisher below.
#[derive(Resource, Default)]
pub struct LastBroadcastBlackboards(
    pub  std::collections::HashMap<
        crate::core::messages::SystemId,
        crate::core::messages::SystemBlackboard,
    >,
);

/// Register Blackboard-owned replication lifecycle behavior.
///
/// This is called once by the simulation composition root beside
/// [`broadcast_blackboard_updates`]. The generic lifecycle runner sees only
/// `BLACKBOARD_REPLICATION_KEY` and function pointers; cache types, visibility,
/// and payload construction stay here with the live producer.
pub(crate) fn register_blackboard_replication_lifecycle(app: &mut App) {
    use crate::authoritative::{DeclareState, StateClass};
    use crate::core::broadcast::{RegisterReplicationLifecycle, ReplicationLifecycleAdapter};

    app.init_resource::<LastBroadcastBlackboards>()
        .init_resource::<crate::console::repair::visibility::LastVisibleRepairBlackboard>()
        .declare_state::<LastBroadcastBlackboards>(StateClass::Cache, "digest-exclusion-classes")
        .declare_state::<crate::console::repair::visibility::LastVisibleRepairBlackboard>(
            StateClass::Cache,
            "digest-exclusion-classes",
        )
        .register_replication_lifecycle(
            ReplicationLifecycleAdapter::new(BLACKBOARD_REPLICATION_KEY)
                .with_reset(reset_blackboard_replication)
                .with_reconnect(reconnect_blackboard_projection),
        );
}

fn reset_blackboard_replication(world: &mut World) {
    *world.resource_mut::<LastBroadcastBlackboards>() = LastBroadcastBlackboards::default();
    world
        .resource_mut::<crate::console::repair::visibility::LastVisibleRepairBlackboard>()
        .clear();
}

/// Build every current Blackboard the reconnecting session is permitted to
/// receive. This reads no delta cache and writes no projection cache.
fn reconnect_blackboard_projection(world: &mut World, token: &str) -> Vec<ServerMessage> {
    use crate::console::repair::visibility;
    use crate::lobby::Sessions;

    let hull_visibility = visibility::hull_visibility(world);
    let station = world
        .get_resource::<Sessions>()
        .and_then(|sessions| sessions.0.station_for_token(token).cloned());
    let mut query = world.query_filtered::<&ShipSystemBlackboards, With<LocalShip>>();
    let Ok(blackboards) = query.single(world) else {
        return Vec::new();
    };

    let mut updates: Vec<_> = blackboards
        .0
        .iter()
        .map(|(system_id, blackboard)| {
            (
                system_id.clone(),
                visibility::project_blackboard_for_token(
                    hull_visibility.as_ref(),
                    station.as_ref(),
                    blackboard,
                ),
            )
        })
        .collect();
    updates.sort_by(|a, b| a.0.cmp(&b.0));

    if updates.is_empty() {
        Vec::new()
    } else {
        vec![ServerMessage::BlackboardUpdate { updates }]
    }
}

/// Returns a [`SimBroadcaster`] pre-configured with the `ModifierAdded` and
/// `ModifierRemoved` producers.
///
/// Drains pending modifier events from [`ShipModifiers`] once per fixed tick and
/// broadcasts each as a separate `ServerMessage` to all players (`Audience::All`).
/// Uses `Cadence::OnEvent` so the producer is called every fixed tick regardless of
/// any Hz timer; an empty drain produces no outbound messages.
/// Registered by [`add_simulation_plugins`] and the test harness in `test_app()`.
///
/// After PR 6 (PRD #597): prefers the per-entity `ShipModifiers` component on
/// the LocalShip entity, falling back to the global Resource for tests that
/// only insert the Resource form.
pub fn modifier_events_broadcaster() -> SimBroadcaster {
    SimBroadcaster::new().register(Audience::All, Cadence::OnEvent, |world: &mut World| {
        use crate::modifiers::ModifierEvent;
        let events: Vec<ModifierEvent> = {
            let mut q =
                world.query_filtered::<&mut crate::modifiers::ShipModifiers, With<LocalShip>>();
            if let Some(mut mods_comp) = q.iter_mut(world).next() {
                std::mem::take(&mut mods_comp.pending_events)
            } else {
                Vec::new()
            }
        };
        events
            .into_iter()
            .map(|event| match event {
                ModifierEvent::Added {
                    source,
                    slot,
                    bonus,
                } => ServerMessage::ModifierAdded {
                    source,
                    slot,
                    bonus,
                },
                ModifierEvent::Removed { source, slot } => {
                    ServerMessage::ModifierRemoved { source, slot }
                }
            })
            .collect()
    })
}

/// Returns a [`SimBroadcaster`] that drains [`SimOutbox`] each fixed tick and writes
/// each entry as an `OutboundMessage` with per-message target routing.
///
/// Uses `Cadence::OnEvent` so the producer fires every fixed tick. When the outbox
/// is empty the producer returns an empty `Vec` and no messages are emitted.
/// When populated (by any simulation system) the queued entries are flushed
/// directly to `OutboundMessage` with their original `Target` routing.
pub fn sim_outbox_broadcaster() -> SimBroadcaster {
    SimBroadcaster::new().register(Audience::All, Cadence::OnEvent, |world: &mut World| {
        let entries = world.resource_mut::<SimOutbox>().drain();
        for entry in entries {
            world.write_message(OutboundMessage {
                target: entry.target,
                msg: entry.message,
                delivery: entry.delivery,
            });
        }
        vec![]
    })
}

// -- Systems -------------------------------------------------------------------

/// When the entity identified by `LastShipAttacker` no longer exists in the
/// world, clear the attacker record so stale references are not published.
pub(crate) fn clear_last_attacker_on_death(
    mut attacker_q: Query<&mut LastShipAttacker>,
    entity_uuids: Query<&EntityUuid>,
) {
    for mut attacker in attacker_q.iter_mut() {
        let uuid = match &attacker.0 {
            Some(u) => u.clone(),
            None => continue,
        };
        let still_alive = entity_uuids.iter().any(|eu| eu.0.as_str() == uuid.as_str());
        if !still_alive {
            attacker.0 = None;
        }
    }
}

/// When a ship's red alert transitions from on to off, clear the attacker
/// record — the threat has passed and the old attacker is no longer relevant.
///
/// Covers every ship (player + NPC), not just `LocalShip`: NPC captain-AI can
/// set its own `ShipRedAlert` (`handle_set_red_alert` in
/// `console::captain::server` dispatches `SetRedAlert` per-ship), and an
/// NPC that stands down should stop retaliating just like the player does.
///
/// `ShipRedAlert` only changes via an explicit assignment (never a
/// same-value rewrite in production), so `Changed<ShipRedAlert>` combined
/// with a boolean component reduces to exactly the on→off edge: the only way
/// a two-valued component both changes and reads `false` is if it was `true`
/// the instant before. This also sidesteps needing a per-entity "previous
/// value" store — a single shared `Local<bool>` (the pre-#685-followup
/// version) does not work once more than one ship is in the query, since it
/// would still only remember one entity's last state.
pub(crate) fn clear_last_attacker_on_red_alert_off(
    mut attacker_q: Query<
        (&mut LastShipAttacker, &crate::ship::state::ShipRedAlert),
        Changed<crate::ship::state::ShipRedAlert>,
    >,
) {
    for (mut attacker, ra) in &mut attacker_q {
        if !ra.0 {
            attacker.0 = None;
        }
    }
}

/// Publish the `LocalShip` viewscreen blackboard: hull/alert status plus the
/// scored objective pool the player ship's per-system AI (weapons, helm,
/// navigation) reads to pick a directive to serve.
///
/// # Why this MERGES rather than clobbers (issue #842)
///
/// After #842 the game-start player hull carries a default `[behaviour]`
/// doctrine, so the player ship holds BOTH `LocalShip` and `BehaviourSection`.
/// `aggregate_doctrine_blackboards` (`With<BehaviourSection>`) also writes the
/// same `VIEWSCREEN_SYSTEM_ID` entry, from the *template* doctrine. If this
/// system simply overwrote that entry — or vice versa — one objective pool
/// would silently erase the other: the doctrine writer clobbering here dropped
/// the player's scenario objectives entirely, so a shipped defence scenario
/// (`combat_test`) stopped developing combat and violated AC3 (scenario
/// objectives must outrank template doctrine).
///
/// Instead this system, pinned to run `.after(aggregate_doctrine_blackboards)`,
/// combines both sources into one scored pool: the global `ObjectiveManager`
/// scenario objectives (e.g. targeted `Destroy wave_N` @80) UNIONED with the
/// hull's template doctrine (untargeted `Destroy` @45 + `Hold` @20), re-sorted
/// descending by score. Scenario objectives coexist with and outrank the
/// standing default, so the player pursues the mission (restoring `combat_test`)
/// while the untargeted @45 remains a fallback that licenses proactive
/// engagement whenever no scenario objective is in play (the probe worlds).
///
/// The doctrine pool is scored fresh from the `BehaviourSection` component here
/// — NOT read back out of the blackboard entry the doctrine writer left. Those
/// two writers run at different cadences (the doctrine writer is gated to the
/// 10 Hz AI snapshot; this one runs every tick), so reading the published entry
/// and re-merging would re-consume this system's own prior output on the ticks
/// the doctrine writer skipped and duplicate the pool without bound. Rescoring
/// from the component is the one source that stays correct every tick.
///
/// A `LocalShip` with no `BehaviourSection` (pre-#842 shape) merges an empty
/// doctrine pool — i.e. behaves exactly as before.
pub(crate) fn publish_viewscreen_blackboard(
    time: Res<Time>,
    world_config: Option<Res<crate::world::config::WorldConfig>>,
    hull_q: Query<&crate::entities::spawner::EntitySystemHull, With<LocalShip>>,
    local_uuid_q: Query<&crate::entities::spawner::EntityUuid, With<LocalShip>>,
    objectives: Option<Res<ObjectiveManagerRes>>,
    boost: Option<Res<CaptainPriorityBoost>>,
    mut ship_blackboards_q: Query<
        (
            &mut ShipSystemBlackboards,
            Option<&crate::ship::state::ShipRedAlert>,
            Option<&crate::ship::combat_activity::RecentCombatActivity>,
            Option<&crate::console::weapons::LastShipAttacker>,
            Option<&crate::entities::spawner::BehaviourSection>,
        ),
        With<LocalShip>,
    >,
) {
    use crate::core::messages::{SystemBlackboard, SystemId, ViewscreenBlackboard};
    use crate::objectives::WorldConditions;
    use crate::ship::system_registry::VIEWSCREEN_SYSTEM_ID;

    let entity_state = ship_blackboards_q.single().ok();
    // Lift Combat Lock + Science Target from the local ship's own radar
    // blackboards (issue #829), published this tick in `SimSet::Publish`.
    let combat_lock = entity_state.as_ref().and_then(|(bbs, _, _, _, _)| {
        match bbs
            .0
            .get(&crate::ship::system_registry::tactical_radar_system_id())
        {
            Some(SystemBlackboard::TacticalRadar(bb)) => bb.selected_target.clone(),
            _ => None,
        }
    });
    let science_target = entity_state.as_ref().and_then(|(bbs, _, _, _, _)| {
        match bbs
            .0
            .get(&crate::ship::system_registry::sensor_radar_system_id())
        {
            Some(SystemBlackboard::SensorRadar(bb)) => bb.selected_target.clone(),
            _ => None,
        }
    });
    let red_alert = entity_state
        .as_ref()
        .and_then(|(_, ra, _, _, _)| ra.map(|r| r.0))
        .unwrap_or(false);
    let last_damage_taken_secs = entity_state
        .as_ref()
        .and_then(|(_, _, act, _, _)| act.and_then(|a| a.last_damage_taken));
    let last_hostile_fire_taken_secs = entity_state
        .as_ref()
        .and_then(|(_, _, act, _, _)| act.and_then(|a| a.last_hostile_fire_taken));
    let last_weapon_fired_secs = entity_state
        .as_ref()
        .and_then(|(_, _, act, _, _)| act.and_then(|a| a.last_weapon_fired));
    let last_attacker_uuid = entity_state
        .as_ref()
        .and_then(|(_, _, _, la, _)| la.and_then(|l| l.0.clone()));

    let hull_integrity_pct = hull_q
        .single()
        .map(|h| {
            let max = h.0.total_max();
            let cur = h.0.total_current();
            if max > 0.0 {
                (cur / max * 100.0).clamp(0.0, 100.0)
            } else {
                100.0
            }
        })
        .unwrap_or(100.0);

    let conditions = WorldConditions {
        red_alert,
        hull_fraction: hull_integrity_pct / 100.0,
        attacked: false,
    };
    // Scope the captain boost to this (local) ship, so a boost only ever
    // reorders this ship's own objective consumers (issue #752).
    let local_uuid = local_uuid_q.single().ok().map(|u| u.0.clone());
    let scope = CaptainPriorityBoost::scope_key(local_uuid.as_deref());
    let captain_boost = boost.as_ref().and_then(|b| b.boost_arg(scope));
    let mut scored_objectives = objectives
        .as_ref()
        .map(|o| o.0.scored_pool_with_boost(&conditions, captain_boost))
        .unwrap_or_default();

    // Merge the hull's standing template doctrine into the scenario pool (see
    // the "why this MERGES" note above). Score the doctrine with the same
    // `attacked` signal the NPC path (`aggregate_doctrine_blackboards`) uses, so
    // a backfilled player and a world-spawned copy of the same hull evaluate
    // their identical doctrine identically (#842 AC4 symmetry). Both sites run
    // the one `objectives::last_landed_hit_secs` fold into the one
    // `objectives::attacked_recently` predicate (issue #1010) — a decaying
    // recency window over the last hit that CONNECTED, shields or hull, not the
    // `LastShipAttacker` latch — so the symmetry holds by construction rather
    // than by two copies of the rule staying in step. The scenario pool keeps
    // its own conditions (unchanged), so existing player-objective scoring is
    // untouched.
    if let Some((_, _, _, _, Some(behaviour))) = entity_state.as_ref() {
        // Sim seconds off the fixed clock (`Res<Time>` is `Time<Fixed>` inside
        // `FixedUpdate`), never a wall clock — AGENTS.md #7.
        let attacked_memory_secs = world_config
            .as_deref()
            .map(|wc| wc.global.attacked_memory_secs)
            .unwrap_or_else(|| {
                crate::entities::config::GlobalConfig::default().attacked_memory_secs
            });
        let doctrine_conditions = WorldConditions {
            red_alert,
            hull_fraction: hull_integrity_pct / 100.0,
            attacked: crate::objectives::attacked_recently(
                crate::objectives::last_landed_hit_secs(
                    last_damage_taken_secs,
                    last_hostile_fire_taken_secs,
                ),
                time.elapsed_secs(),
                attacked_memory_secs,
            ),
        };
        let doctrine_pool =
            crate::ai::score_doctrine_pool(&behaviour.0.doctrine, &doctrine_conditions);
        scored_objectives.extend(doctrine_pool);
    }

    // Re-sort the unioned pool descending by score. `sort_by` is stable, so
    // ties keep concatenation order (scenario objectives before doctrine ones —
    // a deterministic tiebreak the `top_destroy_objective_target` / helm
    // consumers rely on to read the highest-scored directive first). `total_cmp`
    // gives a total, deterministic order the rng-determinism guard depends on.
    scored_objectives.sort_by(|a, b| b.score.total_cmp(&a.score));

    let bb = ViewscreenBlackboard {
        red_alert,
        hull_integrity_pct,
        last_damage_taken_secs,
        last_weapon_fired_secs,
        last_attacker_uuid,
        scored_objectives,
        combat_lock,
        science_target,
    };

    // Write directly to the per-entity component.
    if let Some((mut entity_bbs, _, _, _, _)) = ship_blackboards_q.iter_mut().next() {
        entity_bbs.0.insert(
            SystemId(VIEWSCREEN_SYSTEM_ID.to_string()),
            SystemBlackboard::Viewscreen(bb),
        );
    }
}

/// Collision response for ships in contact: applies hull damage, brings the
/// ship to a hard stop, and de-overlaps it from the collider it hit.
///
/// # Sanctioned out-of-band `ShipPhysics` writer (issue #699)
///
/// `integrate_ship_physics` is the sole *helm-path* writer of
/// `ShipPhysics.x/z/yaw/forward_speed/lateral_speed/roll`. This system (with
/// `separate_ship_from_collision`) writes `forward_speed`/`x`/`z` directly and
/// is an intentional exception: collision response is a correction layered on
/// top of the helm integration, not a competing integrator. Routing it through
/// helm intent would let the ship integrate *into* geometry for a frame before
/// responding. It deliberately does not opt into the debug
/// `HelmPhysicsWriteGuard`. See the writer-policy table on `ShipPhysics`
/// (`src/ship/state.rs`).
/// Balance tracer: emit a [`BalanceEvent::PhaseChanged`] for every game-phase
/// transition. Reads the global `StateTransitionEvent<GamePhase>` stream, so it
/// fires exactly once per real transition without tapping each `next_state.set`
/// call site. Same-state "transitions" are skipped. `Option<ResMut<Messages>>`
/// so bare-`App` fixtures without the message registered still validate.
pub(crate) fn emit_phase_change_balance_events(
    mut reader: MessageReader<bevy::state::state::StateTransitionEvent<GamePhase>>,
    mut balance_events: Option<
        ResMut<bevy::ecs::message::Messages<crate::core::balance::BalanceEvent>>,
    >,
) {
    let Some(msgs) = balance_events.as_mut() else {
        return;
    };
    for ev in reader.read() {
        if ev.exited == ev.entered {
            continue;
        }
        let fmt = |s: &Option<GamePhase>| match s {
            Some(p) => format!("{p:?}"),
            None => "None".to_string(),
        };
        msgs.write(crate::core::balance::BalanceEvent::PhaseChanged {
            from: fmt(&ev.exited),
            to: fmt(&ev.entered),
        });
    }
}

/// Tracks whether the initial WorldSetup broadcast has fired, so it only
/// goes out once per game.
#[derive(Resource, Default)]
pub(crate) struct WorldSetupBroadcast {
    sent: bool,
}

/// Broadcast `GameOver { reason, outcome }` to all players when the game enters
/// the GameOver phase. Reads both halves of the `GameOverReason` resource and
/// resets the REASON to `None` after broadcast.
///
/// Only the reason is taken. `.1` is read and left in place, for two separate
/// reasons that happen to agree: the headless exit report reads it after the
/// run to classify victory vs defeat (`src/bin/phoenix_headless.rs`), and
/// `state_digest` folds BOTH halves — clearing the outcome here would move
/// every digest of a run that reaches `GameOver` inside its window, for no
/// gain. `Outcome` is `Copy`, so reading it needs no mutation at all.
pub(crate) fn on_game_over_enter(
    mut game_over_reason: ResMut<GameOverReason>,
    mut outbox: ResMut<SimOutbox>,
) {
    let outcome = game_over_reason.1.map(|o| o.as_str().to_string());
    let reason = game_over_reason.0.take().unwrap_or_default();
    outbox.push_reliable((Target::All, ServerMessage::GameOver { reason, outcome }));
}

/// Reset all change-detection caches when entering InProgress so the first
/// broadcast tick always sends a full state to all players. Also covers the
/// multi-game restart case where stale cache from a previous game would
/// otherwise suppress initial updates.
///
/// The stable-keyed lifecycle runner invokes every owner without knowing its
/// resource types. Owners with no delta cache (such as Shields) simply do not
/// register a reset hook.
pub(crate) fn reset_broadcast_caches_on_start(world: &mut World) {
    crate::core::broadcast::reset_registered_replication(world);
}

/// Emit `BlackboardUpdate` for any system whose blackboard has changed since
/// the last broadcast. Reads from the `LocalShip` entity's per-entity component
/// (populated by `dual_publish_blackboards`). Runs in `SimSet::PublishAggregate`
/// (before `SimSet::Broadcast` so `broadcast::dispatch::<Sim>` sees the outbox entries).
///
/// Since issue #737 the *repair* blackboard is fanned out per session token
/// rather than broadcast to all: it carries exact per-system hull detail, and
/// who may see which system is a host-side decision. Every other blackboard
/// still goes out unprojected at `Target::All`.
/// The `Local` caches the `QueryState` across ticks. `World::query_filtered`
/// builds a *fresh* one on every call, and constructing it walks every archetype
/// in the world to work out which ones match — per tick, for a query that
/// resolves to a single `LocalShip` entity. Bevy asserts the state's world id on
/// use, so a cached state cannot silently be applied to the wrong world.
pub fn broadcast_blackboard_updates(
    world: &mut World,
    mut bb_query: Local<Option<QueryState<&'static ShipSystemBlackboards, With<LocalShip>>>>,
) {
    use crate::console::repair::visibility;

    world.init_resource::<visibility::LastVisibleRepairBlackboard>();

    let mut updates: Vec<(
        crate::core::messages::SystemId,
        crate::core::messages::SystemBlackboard,
    )> = {
        let q = bb_query.get_or_insert_with(|| {
            world.query_filtered::<&ShipSystemBlackboards, With<LocalShip>>()
        });
        let Some(bb) = q.iter(world).next() else {
            return;
        };
        let last = world.resource::<LastBroadcastBlackboards>();
        let mut changed: Vec<(
            crate::core::messages::SystemId,
            crate::core::messages::SystemBlackboard,
        )> =
            bb.0.iter()
                .filter(|(id, bb)| last.0.get(*id) != Some(*bb))
                .map(|(id, bb)| (id.clone(), bb.clone()))
                .collect();
        // Sorted because this vec becomes the `BlackboardUpdate` payload, and
        // it was collected from a `HashMap` whose order follows `RandomState`'s
        // per-process seed — two `--seed` runs emitted the same updates in a
        // different order every time. Sorting the changed set (a handful of
        // entries) rather than ordering the map itself keeps the per-tick
        // publish writes on cheap hash lookups.
        changed.sort_by(|a, b| a.0.cmp(&b.0));
        changed
    };

    let viewers = visibility::connected_viewers(world);

    // A token's station is an input to its repair-blackboard projection, so a
    // player changing station mid-game invalidates it even though nothing the
    // `LastBroadcastBlackboards` diff can see has changed. Without this, an
    // idle undamaged ship would leave the previous station's detail on that
    // phone until the internal blackboard next changed — possibly never.
    let stations_changed = {
        let cache = world.resource::<visibility::LastVisibleRepairBlackboard>();
        cache.stations_changed(&viewers)
    };

    // Prune first so a disconnected token cannot keep suppressing a resend if
    // it reconnects into the same station later in the same game.
    {
        let mut cache = world.resource_mut::<visibility::LastVisibleRepairBlackboard>();
        visibility::prune_repair_blackboard_cache(&mut cache, &viewers);
        cache.record_stations(&viewers);
    }

    if updates.is_empty() && !stations_changed {
        return;
    }

    // Station change with an otherwise-unchanged blackboard: re-feed the
    // current repair blackboard so the per-token projection is recomputed. The
    // per-token cache inside `project_repair_blackboards` still suppresses the
    // send for every token whose *view* did not actually change.
    if stations_changed
        && !updates
            .iter()
            .any(|(_, bb)| matches!(bb, crate::core::messages::SystemBlackboard::Repair(_)))
    {
        let mut q = world.query_filtered::<&ShipSystemBlackboards, With<LocalShip>>();
        if let Some(bb) = q.iter(world).next() {
            if let Some((id, repair)) = bb
                .0
                .iter()
                .find(|(_, bb)| matches!(bb, crate::core::messages::SystemBlackboard::Repair(_)))
            {
                updates.push((id.clone(), repair.clone()));
            }
        }
    }

    // One `resource_mut` for the whole batch, not one per entry: each call is a
    // resource lookup plus a change-tick bump, and this loop runs every tick
    // that anything changed.
    {
        let mut last = world.resource_mut::<LastBroadcastBlackboards>();
        for (id, bb) in &updates {
            last.0.insert(id.clone(), bb.clone());
        }
    }

    let vis = visibility::hull_visibility(world);
    let mut cache = world
        .remove_resource::<visibility::LastVisibleRepairBlackboard>()
        .unwrap_or_default();
    let pending =
        visibility::project_repair_blackboards(updates, vis.as_ref(), &viewers, &mut cache);
    world.insert_resource(cache);
    world.resource_mut::<SimOutbox>().extend_snapshot(pending);
}

/// When a player reconnects mid-game (Identify during InProgress),
/// `handle_identify_system` (in `LobbySystemSet`) queues a `Welcome { .. }` into
/// `LobbyOutbox` targeted at that player's
/// token. Detect this and push a full-state resync to *just that token* via
/// [`crate::core::broadcast::resync_registered_replication_for_token`]
/// (issue #613).
///
/// This replaces the #599 quick fix, which reset every then-registered shared
/// broadcast delta cache — correct for the reconnecting player, but it also
/// invalidated the global delta state. On the *next* 10 Hz tick, shared
/// `Audience::All` paths resent full state to *every other* connected client.
/// The targeted resync leaves the shared caches untouched, so
/// every other client's next tick remains a normal delta.
pub(crate) fn refresh_caches_on_midgame_reconnect(world: &mut World) {
    let state = world.resource::<State<GamePhase>>();
    if *state.get() != GamePhase::InProgress {
        return;
    }
    let reconnecting_tokens: Vec<String> = {
        let lobby_outbox = world.resource::<LobbyOutbox>();
        lobby_outbox
            .0
            .iter()
            .filter_map(|(target, msg)| match (target, msg) {
                (Target::Token(token), ServerMessage::Welcome { .. }) => Some(token.clone()),
                _ => None,
            })
            .collect()
    };
    for token in reconnecting_tokens {
        crate::core::broadcast::resync_registered_replication_for_token(world, &token);
    }
}

/// Emit a single `WorldSetup` broadcast when the game enters `InProgress`.
/// Uses `State<GamePhase>` + sentry to fire exactly once.
pub(crate) fn broadcast_world_setup_on_start(
    state: Res<State<GamePhase>>,
    world: Res<WorldResource>,
    mut sent: ResMut<WorldSetupBroadcast>,
    mut outbox: ResMut<SimOutbox>,
) {
    if sent.sent || state.get() != &GamePhase::InProgress {
        return;
    }
    outbox.push_reliable((
        Target::All,
        ServerMessage::WorldSetup {
            world: world.0.clone(),
        },
    ));
    sent.sent = true;
}

/// For non-asteroid entities carrying `EntityUuid`:
/// - New entities (present in ECS, absent from `reported`) emit `EntitySpawned`
///   and are added to `WorldResource.entities` so they appear on reconnect `Welcome`.
/// - Missing entities (absent from ECS, present in `reported`) emit
///   `EntityDespawned` and are removed from `WorldResource.entities`.
///
/// Asteroids are excluded (they use `AsteroidSpawned` / `AsteroidDestroyed`).
///
/// On the very first `InProgress` tick, seeds `reported` from the initial
/// `WorldResource` entities so those are not re-broadcast.
pub(crate) fn reconcile_runtime_entities(
    mut registry: ResMut<TrackedEntities>,
    mut world: ResMut<WorldResource>,
    query: Query<
        (
            Entity,
            &EntityUuid,
            Option<&EntityId>,
            Option<&EntityName>,
            &Transform,
            Option<&RegionShapeSection>,
            Option<&EntityTagsSection>,
            Option<&RadarAppearanceSection>,
            Option<&AsteroidFieldSection>,
            Option<&crate::entities::spawner::EntitySystemHull>,
            Option<&crate::entities::spawner::EntityTarget>,
            Option<&crate::ship::shields::ShipShields>,
            Option<&crate::infrastructure::InfrastructureCondition>,
        ),
        Without<Asteroid>,
    >,
    mut outbox: ResMut<SimOutbox>,
    objectives: Option<Res<ObjectiveManagerRes>>,
    mut positions_cache: ResMut<LastBroadcastEntityPositions>,
    mut health_cache: ResMut<LastBroadcastEntityHealth>,
) {
    // Build set of entity names referenced by active mission objectives.
    let active_objective_names: std::collections::HashSet<String> = objectives
        .as_ref()
        .map(|obj| {
            obj.0
                .sorted_snapshots()
                .into_iter()
                .filter(|s| s.status == crate::core::messages::ObjectiveStatus::Active)
                .flat_map(|s| s.targets)
                .collect()
        })
        .unwrap_or_default();
    // Build the current set of ECS entity UUIDs.
    let current: HashMap<String, Entity> = query
        .iter()
        .map(|(e, u, _, _, _, _, _, _, _, _, _, _, _)| (u.0.clone(), e))
        .collect();

    /// Serialise a `RegionShape` to the wire string (snake_case variant name).
    fn shape_to_wire(shape: &RegionShapeSection) -> String {
        use crate::regions::shape::RegionShape;
        match &shape.0 {
            RegionShape::Sphere { .. } => "sphere",
            RegionShape::Box { .. } => "box",
            RegionShape::Torus { .. } => "torus",
        }
        .to_string()
    }

    // Seed reported set from ECS on first in-progress frame so that initial
    // world entities (stars, planets, ships, fields) are not re-reported.
    // Also populate WorldData.entities so the reconnect Welcome includes them.
    if !registry.seeded {
        for (uuid, entity) in &current {
            registry.reported.insert(uuid.clone());
            if let Ok((
                _,
                _,
                id,
                name,
                transform,
                region_shape,
                entity_tags,
                radar_appearance,
                asteroid_field,
                hull_comp,
                entity_target,
                shield_comp,
                infrastructure,
            )) = query.get(*entity)
            {
                let hull_fraction = hull_comp.map(|h| {
                    let max = h.0.total_max();
                    if max > 0.0 {
                        h.0.total_current() / max
                    } else {
                        1.0
                    }
                });
                let shield_fraction = shield_comp.map(|s| {
                    let total_hp: i32 = s.0.facings.iter().map(|f| f.hp).sum();
                    let total_max: i32 = s.0.facings.iter().map(|f| f.max_hp).sum();
                    if total_max > 0 {
                        total_hp as f32 / total_max as f32
                    } else {
                        0.0
                    }
                });
                let mut snapshot = EntitySnapshot {
                    uuid: uuid.clone(),
                    id: id.as_ref().map(|i| i.0.clone()),
                    name: name.as_ref().map(|n| n.0.clone()),
                    hull_fraction,
                    shield_fraction,
                    // Issue #1025: minted from the LIVE track, so a structure
                    // reported after it degraded is reported as it now is.
                    infrastructure: infrastructure.and_then(|i| {
                        crate::core::messages::InfrastructureSnapshot::from_state(&i.0)
                    }),
                    position: Some([
                        transform.translation.x,
                        transform.translation.y,
                        transform.translation.z,
                    ]),
                    tags: entity_tags.map(|t| t.0.clone()).unwrap_or_default(),
                    ..EntitySnapshot::default()
                };
                if let Some(shape) = region_shape {
                    snapshot.shape = Some(shape_to_wire(shape));
                    if snapshot.radius.is_none() {
                        match &shape.0 {
                            crate::regions::shape::RegionShape::Sphere { radius } => {
                                snapshot.radius = Some(*radius);
                            }
                            crate::regions::shape::RegionShape::Box { half_extents, .. } => {
                                let max_he = half_extents[0].max(half_extents[2]);
                                snapshot.radius = Some(max_he);
                                snapshot.half_extents = Some(*half_extents);
                            }
                            crate::regions::shape::RegionShape::Torus {
                                inner_radius,
                                outer_radius,
                            } => {
                                snapshot.radius = Some(*outer_radius);
                                snapshot.inner_radius = Some(*inner_radius);
                            }
                        }
                    }
                }
                if snapshot.shape.is_none() {
                    if let Some(field) = asteroid_field {
                        snapshot.shape = Some("torus".to_string());
                        snapshot.radius = Some(field.0.outer_radius);
                        snapshot.inner_radius = Some(field.0.inner_radius);
                    }
                }
                if let Some(ra) = radar_appearance {
                    if let Some(colour) = &ra.0.colour {
                        if colour.len() >= 3 {
                            snapshot.colour = Some([colour[0], colour[1], colour[2]]);
                        }
                    }
                    if let Some(region_colour) = &ra.0.region_colour {
                        if region_colour.len() >= 3 {
                            snapshot.region_colour =
                                Some([region_colour[0], region_colour[1], region_colour[2]]);
                        }
                    }
                    snapshot.radar_size = ra.0.size;
                    snapshot.radar_icon = ra.0.icon.clone();
                }
                if let Some(ref id) = snapshot.id {
                    snapshot.objective_target = active_objective_names.contains(id);
                }
                // Target info
                if let Some(t) = entity_target {
                    snapshot.target_tags = t.0.tags.clone();
                    snapshot.threat_level = Some(t.0.threat_level.as_str().to_string());
                    snapshot.target_description = t.0.description.clone();
                }
                upsert_world_entity(&mut world, snapshot);
            }
        }
        registry.seeded = true;
        return;
    }

    // Emit EntitySpawned for new entities, in UUID order.
    //
    // Iterating `current` directly announced the same ships in a different
    // order on every run, because `HashMap` order follows `RandomState`'s
    // per-process seed. Only the *newly seen* ids are sorted — almost always
    // none, and a handful on the tick a wave spawns — so this stays off the
    // per-tick cost of walking every entity.
    let mut newly_seen: Vec<(&String, &Entity)> = current
        .iter()
        .filter(|(uuid, _)| !registry.reported.contains(*uuid))
        .collect();
    newly_seen.sort_by(|a, b| a.0.cmp(b.0));
    for (uuid, entity) in newly_seen {
        if registry.reported.insert(uuid.clone()) {
            if let Ok((
                _,
                _,
                id,
                name,
                transform,
                region_shape,
                entity_tags,
                radar_appearance,
                asteroid_field,
                hull_comp,
                entity_target,
                shield_comp,
                infrastructure,
            )) = query.get(*entity)
            {
                let hull_fraction = hull_comp.map(|h| {
                    let max = h.0.total_max();
                    if max > 0.0 {
                        h.0.total_current() / max
                    } else {
                        1.0
                    }
                });
                let shield_fraction = shield_comp.map(|s| {
                    let total_hp: i32 = s.0.facings.iter().map(|f| f.hp).sum();
                    let total_max: i32 = s.0.facings.iter().map(|f| f.max_hp).sum();
                    if total_max > 0 {
                        total_hp as f32 / total_max as f32
                    } else {
                        0.0
                    }
                });
                let mut snapshot = EntitySnapshot {
                    uuid: uuid.clone(),
                    id: id.as_ref().map(|i| i.0.clone()),
                    name: name.as_ref().map(|n| n.0.clone()),
                    hull_fraction,
                    shield_fraction,
                    // Issue #1025: minted from the LIVE track, so a structure
                    // reported after it degraded is reported as it now is.
                    infrastructure: infrastructure.and_then(|i| {
                        crate::core::messages::InfrastructureSnapshot::from_state(&i.0)
                    }),
                    position: Some([
                        transform.translation.x,
                        transform.translation.y,
                        transform.translation.z,
                    ]),
                    tags: entity_tags.map(|t| t.0.clone()).unwrap_or_default(),
                    ..EntitySnapshot::default()
                };
                if let Some(shape) = region_shape {
                    snapshot.shape = Some(shape_to_wire(shape));
                    if snapshot.radius.is_none() {
                        match &shape.0 {
                            crate::regions::shape::RegionShape::Sphere { radius } => {
                                snapshot.radius = Some(*radius);
                            }
                            crate::regions::shape::RegionShape::Box { half_extents, .. } => {
                                let max_he = half_extents[0].max(half_extents[2]);
                                snapshot.radius = Some(max_he);
                                snapshot.half_extents = Some(*half_extents);
                            }
                            crate::regions::shape::RegionShape::Torus {
                                inner_radius,
                                outer_radius,
                            } => {
                                snapshot.radius = Some(*outer_radius);
                                snapshot.inner_radius = Some(*inner_radius);
                            }
                        }
                    }
                }
                if snapshot.shape.is_none() {
                    if let Some(field) = asteroid_field {
                        snapshot.shape = Some("torus".to_string());
                        snapshot.radius = Some(field.0.outer_radius);
                        snapshot.inner_radius = Some(field.0.inner_radius);
                    }
                }
                if let Some(ra) = radar_appearance {
                    if let Some(colour) = &ra.0.colour {
                        if colour.len() >= 3 {
                            snapshot.colour = Some([colour[0], colour[1], colour[2]]);
                        }
                    }
                    if let Some(region_colour) = &ra.0.region_colour {
                        if region_colour.len() >= 3 {
                            snapshot.region_colour =
                                Some([region_colour[0], region_colour[1], region_colour[2]]);
                        }
                    }
                    snapshot.radar_size = ra.0.size;
                    snapshot.radar_icon = ra.0.icon.clone();
                }
                if let Some(ref id) = snapshot.id {
                    snapshot.objective_target = active_objective_names.contains(id);
                }
                // Target info
                if let Some(t) = entity_target {
                    snapshot.target_tags = t.0.tags.clone();
                    snapshot.threat_level = Some(t.0.threat_level.as_str().to_string());
                    snapshot.target_description = t.0.description.clone();
                }
                upsert_world_entity(&mut world, snapshot.clone());
                outbox.push_reliable((Target::All, ServerMessage::EntitySpawned { snapshot }));
            }
        }
    }

    // Emit EntityDespawned for entities no longer in the ECS.
    let reported_snapshot: Vec<String> = registry.reported.iter().cloned().collect();
    for uuid in &reported_snapshot {
        if !current.contains_key(uuid) {
            registry.reported.remove(uuid);
            world.0.entities.retain(|e| e.uuid != *uuid);
            // Prune the despawned UUID from the delta caches (issue #613) —
            // runtime-spawned entities (e.g. scenario-triggered NPCs) can
            // despawn and respawn with fresh UUIDs just like asteroids.
            prune_entity_replication_caches(
                &mut positions_cache,
                &mut health_cache,
                std::slice::from_ref(uuid),
            );
            outbox.push_reliable((
                Target::All,
                ServerMessage::EntityDespawned { uuid: uuid.clone() },
            ));
        }
    }
}

#[cfg(test)]
mod lifecycle_tests {
    use super::*;
    use crate::console::repair::visibility::{self, LastVisibleRepairBlackboard};
    use crate::core::broadcast::{
        reconnect_registered_replication, reset_registered_replication,
        ReplicationLifecycleRegistry,
    };
    use crate::core::messages::{HelmBlackboard, RepairBlackboard, SystemBlackboard, SystemId};
    use crate::entities::spawner::EntitySystemHull;
    use crate::lobby::{Sessions, Target};
    use crate::modifiers::repair_teams::RepairTeams;
    use crate::ship::config::ShipConfig;
    use crate::ship::damage::SystemHull;
    use crate::ship_plugin::ShipConfigComponent;

    fn helm_blackboard() -> SystemBlackboard {
        SystemBlackboard::Helm(HelmBlackboard {
            boost_battery: 1.0,
            boost_enabled: true,
            ..Default::default()
        })
    }

    fn blackboard_lifecycle_app() -> App {
        let config = ShipConfig::from_toml(
            r#"
[[station]]
id = "helm"
name = "Helm"
description = "Flying."
rank = "Ltn."

[[station]]
id = "engineering"
name = "Engineering"
description = "Fixing."
rank = "Ltn."

[[system]]
id = "helm-radar"
kind = "helm_radar"
station = "helm"

[[system]]
id = "repair"
kind = "repair"
station = "engineering"
"#,
            &["helm_radar", "repair"],
        )
        .expect("Blackboard lifecycle fixture must parse");

        let mut sessions = crate::lobby::session::SessionManager::new();
        sessions.register("eng".into(), "Engineer".into()).unwrap();
        sessions.register("pilot".into(), "Pilot".into()).unwrap();
        sessions.set_station(
            "eng",
            Some(crate::core::messages::StationId("engineering".into())),
        );
        sessions.set_station(
            "pilot",
            Some(crate::core::messages::StationId("helm".into())),
        );

        let mut blackboards = ShipSystemBlackboards::default();
        // The raw Repair payload deliberately contains detail. The projection
        // must keep only the detail each recipient can see.
        blackboards.0.insert(
            SystemId("repair".into()),
            SystemBlackboard::Repair(RepairBlackboard {
                aggregate_hull_fraction: Some(0.5),
                destroyed_hull_fraction: Some(0.25),
                damageable_systems: vec![
                    SystemId("core".into()),
                    SystemId("helm-radar".into()),
                    SystemId("repair".into()),
                ],
                ..Default::default()
            }),
        );
        blackboards
            .0
            .insert(SystemId("z-shared".into()), helm_blackboard());

        let mut app = App::new();
        app.insert_resource(Sessions(sessions));
        register_blackboard_replication_lifecycle(&mut app);
        app.world_mut().spawn((
            LocalShip,
            ShipConfigComponent(config),
            EntitySystemHull(SystemHull::from_config(&[
                (SystemId("core".into()), 100.0),
                (SystemId("helm-radar".into()), 100.0),
                (SystemId("repair".into()), 100.0),
            ])),
            crate::console::repair::server::ShipRepairTeams(RepairTeams::new(1)),
            blackboards,
        ));
        app
    }

    fn live_updates_for_token(app: &mut App, token: &str) -> Vec<(SystemId, SystemBlackboard)> {
        let mut query = app
            .world_mut()
            .query_filtered::<&ShipSystemBlackboards, With<LocalShip>>();
        let mut current: Vec<_> = query
            .single(app.world())
            .expect("LocalShip Blackboards")
            .0
            .iter()
            .map(|(id, blackboard)| (id.clone(), blackboard.clone()))
            .collect();
        current.sort_by(|a, b| a.0.cmp(&b.0));

        let hull_visibility = visibility::hull_visibility(app.world_mut());
        let viewers = visibility::connected_viewers(app.world());
        let mut last = LastVisibleRepairBlackboard::default();
        let pending = visibility::project_repair_blackboards(
            current,
            hull_visibility.as_ref(),
            &viewers,
            &mut last,
        );

        let mut visible = Vec::new();
        for (target, message) in pending {
            let for_recipient = matches!(target, Target::All)
                || matches!(&target, Target::Token(target_token) if target_token == token);
            if !for_recipient {
                continue;
            }
            if let ServerMessage::BlackboardUpdate { updates } = message {
                visible.extend(updates);
            }
        }
        visible.sort_by(|a, b| a.0.cmp(&b.0));
        visible
    }

    fn reconnect_updates_for_token(
        app: &mut App,
        token: &str,
    ) -> Vec<(SystemId, SystemBlackboard)> {
        reconnect_registered_replication(app.world_mut(), token)
            .into_iter()
            .flat_map(|message| match message {
                ServerMessage::BlackboardUpdate { updates } => updates,
                _ => Vec::new(),
            })
            .collect()
    }

    #[test]
    fn blackboard_owner_registers_reset_for_both_live_projection_caches() {
        let mut app = blackboard_lifecycle_app();
        app.world_mut()
            .resource_mut::<LastBroadcastBlackboards>()
            .0
            .insert(SystemId("cached".into()), helm_blackboard());
        {
            let mut projected = app
                .world_mut()
                .resource_mut::<LastVisibleRepairBlackboard>();
            projected
                .projections
                .insert("eng".into(), RepairBlackboard::default());
            projected.stations.insert(
                "eng".into(),
                Some(crate::core::messages::StationId("engineering".into())),
            );
        }

        assert_eq!(
            app.world()
                .resource::<ReplicationLifecycleRegistry>()
                .keys()
                .collect::<Vec<_>>(),
            vec![BLACKBOARD_REPLICATION_KEY]
        );
        reset_registered_replication(app.world_mut());

        assert!(app
            .world()
            .resource::<LastBroadcastBlackboards>()
            .0
            .is_empty());
        let projected = app.world().resource::<LastVisibleRepairBlackboard>();
        assert!(projected.projections.is_empty());
        assert!(projected.stations.is_empty());
    }

    #[test]
    fn reconnect_projection_matches_live_visibility_for_each_recipient() {
        let mut app = blackboard_lifecycle_app();

        for token in ["eng", "pilot"] {
            let live = live_updates_for_token(&mut app, token);
            let reconnect = reconnect_updates_for_token(&mut app, token);
            assert_eq!(
                reconnect, live,
                "reconnect must reproduce the live Blackboard visibility for {token}"
            );
            assert_eq!(
                reconnect
                    .iter()
                    .map(|(id, _)| id.0.as_str())
                    .collect::<Vec<_>>(),
                vec!["repair", "z-shared"],
                "every current permitted Blackboard must be present in stable order"
            );
        }

        let pilot = reconnect_updates_for_token(&mut app, "pilot");
        let repair = pilot
            .iter()
            .find_map(|(_, blackboard)| match blackboard {
                SystemBlackboard::Repair(repair) => Some(repair),
                _ => None,
            })
            .expect("pilot receives the withholding Repair overwrite");
        assert!(repair.system_hull.is_empty());
        assert!(repair.damageable_systems.is_empty());
        assert!(repair.aggregate_hull_fraction.is_none());
        assert!(repair.destroyed_hull_fraction.is_none());
    }
}
