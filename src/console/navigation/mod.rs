use bevy::prelude::*;

use crate::command_admission::ai_emit::emit_ai_command;
use crate::messages::{
    AdmittedCommands, NavigationBlackboard, SystemBlackboard, SystemControlPayload, SystemId,
    WaypointSnapshot,
};
use crate::ship::system_registry::NAVIGATION_SYSTEM_ID;

pub struct NavigationPlugin;

impl Plugin for NavigationPlugin {
    fn build(&self, app: &mut App) {
        use crate::command_admission::{ConsumerMatcher, RegisterAdmittedConsumer};
        // The shared AI decision cadence (issue #889): `operate_navigation_ai`
        // was one of four hosts #895's FixedUpdate migration left ungated —
        // named deciders (helm axes, shields, power, torpedo, captain,
        // sensors) got `run_if(ai_tick_ready)`/`run_if(ai_snapshot_ready)`, but
        // Navigation, Repair and both Comms hosts were never enumerated and so
        // kept running once per FIXED STEP with no gate at all. `register_ai_cadence`
        // is idempotent — every plugin that adds a gated system calls it, so
        // `NavigationPlugin` used standalone (as several fixtures do) still
        // gets `AiTickReady` inserted rather than panicking on a missing `Res`.
        crate::ai::cadence::register_ai_cadence(app);
        // Admitted-command consumer (issue #833): `handle_navigation_waypoint`
        // reads the `navigation` system's admitted commands.
        app.register_admitted_consumer(ConsumerMatcher::exact(NAVIGATION_SYSTEM_ID));
        // The admitted-waypoint applier moves Input→Physics (issue #830):
        // `operate_navigation_ai` emits its `SetNavigationWaypoint` into
        // `AdmittedCommands` in Physics, and admission clears `AdmittedCommands`
        // once per tick *before* Input, so the applier must run after the AI
        // emit in the same set for a same-tick AI waypoint to land. Human
        // commands admitted before Input survive to Physics unchanged.
        app.add_systems(
            FixedUpdate,
            handle_navigation_waypoint
                .in_set(crate::sim_sets::SimSet::Physics)
                .after(operate_navigation_ai),
        )
        // Refresh anchored waypoints from the parent entity's live
        // Transform every tick, before the broadcaster reads the
        // waypoint into the SimSnapshot. Auto-clear when the parent
        // entity is no longer present.
        .add_systems(
            FixedUpdate,
            refresh_anchored_waypoint.in_set(crate::sim_sets::SimSet::Modifiers),
        )
        .add_systems(
            FixedUpdate,
            operate_navigation_ai
                .in_set(crate::sim_sets::SimSet::Physics)
                .run_if(crate::ai::cadence::ai_tick_ready),
        )
        // The single, origin-agnostic Channel-3 clearance issuer (issue #702
        // follow-up): runs after BOTH waypoint writers — `operate_navigation_ai`
        // (which emits) and `handle_navigation_waypoint` (which applies both the
        // human- and AI-set waypoints, now in Physics, #830) — so a waypoint set
        // this tick gets its clearance this tick, whoever set it (#702 shared-
        // issuer invariant).
        .add_systems(
            FixedUpdate,
            issue_navigate_to_clearance
                .in_set(crate::sim_sets::SimSet::Physics)
                .after(operate_navigation_ai)
                .after(handle_navigation_waypoint),
        )
        .add_systems(
            FixedUpdate,
            publish_navigation_blackboard.in_set(crate::sim_sets::SimSet::Publish),
        );
    }
}

/// Authoritative navigation waypoint state.
///
/// Stores either a free position chosen by tap-to-place, or an entity-anchored
/// waypoint that follows the named entity's transform until the entity
/// despawns.
///
/// Per-entity `Component` on every ship (player + NPC). PR-7 (issue #597)
/// removed the dual `Resource` derive — every ship has its own waypoint.
///
/// # The Helm's goal, not a copy of it (issue #702)
///
/// This is the one navigation goal on the ship. The AI Helm reads it directly
/// rather than keeping a private `AiMemory.nav_goal` copy laundered through a
/// coordination message — one position, no split brain. Humans
/// (`SetNavigationWaypoint`) and `operate_navigation_ai` write it symmetrically.
#[derive(Component, Default, Clone, Debug, PartialEq)]
#[require(NavClearanceIssueState)]
pub struct NavigationWaypoint {
    mode: Option<WaypointMode>,
    generation: u64,
}

/// Bookkeeping for [`issue_navigate_to_clearance`], the single origin-agnostic
/// issuer of the Channel-3 `NavigateTo` clearance.
///
/// Required by [`NavigationWaypoint`], so every ship that can carry a waypoint
/// carries this alongside it — production spawns and minimal test spawns alike.
///
/// Not part of the waypoint itself on purpose: the waypoint is the *goal*
/// (what Navigation wants), this is *delivery state* (whether the Helm has
/// been told about it yet). Mixing them would make setting the same waypoint
/// twice look like a change.
#[derive(Component, Default, Clone, Debug, PartialEq)]
pub struct NavClearanceIssueState {
    /// The waypoint generation the last-enqueued `NavigateTo` carried, so the
    /// issuer sends exactly one order per new waypoint rather than one per
    /// tick — the per-tick re-enqueue the old AI path did would popup-spam a
    /// human Helm and flood the lag queue.
    issued_generation: Option<u64>,
    /// Whether the helm stick axes operated AI last tick. A `false → true`
    /// edge (Backfill after a disconnect, a rating change) re-issues the
    /// clearance for a waypoint whose earlier order was delivered to a human
    /// helm (popup/suppress — no latch), so the AI helm eventually flies the
    /// ship's current waypoint no matter when control flipped.
    helm_axes_were_ai: bool,
}

/// Storage variant of the navigation waypoint.
#[derive(Clone, Debug, PartialEq)]
pub enum WaypointMode {
    /// Tap-to-place: the waypoint is a fixed world position and never moves.
    Free { x: f32, z: f32 },
    /// Anchored to an entity by UUID. `last_x` / `last_z` mirror the entity's
    /// last-known transform; they are refreshed each tick by
    /// [`refresh_anchored_waypoint`]. When the parent entity is no longer
    /// present, the waypoint is auto-cleared.
    Anchored {
        source_uuid: String,
        last_x: f32,
        last_z: f32,
    },
}

/// Whether two modes name the *same* waypoint — the identity the `generation`
/// counts, as distinct from structural equality.
///
/// A `Free` waypoint is its position. An `Anchored` waypoint is its parent
/// entity: `last_x`/`last_z` are a cache of that entity's transform, refreshed
/// every tick by [`refresh_anchored_waypoint`], so comparing them would make a
/// waypoint anchored to a *moving* ship count as brand new on every tick it
/// moved — re-incurring the Channel-3 lag forever and never letting the Helm
/// follow it.
fn same_waypoint(a: &WaypointMode, b: &WaypointMode) -> bool {
    match (a, b) {
        (WaypointMode::Free { x: ax, z: az }, WaypointMode::Free { x: bx, z: bz }) => {
            ax == bx && az == bz
        }
        (
            WaypointMode::Anchored {
                source_uuid: a_uuid,
                ..
            },
            WaypointMode::Anchored {
                source_uuid: b_uuid,
                ..
            },
        ) => a_uuid == b_uuid,
        _ => false,
    }
}

impl NavigationWaypoint {
    /// A waypoint already set to `mode` (generation 1). Test/spawn convenience.
    pub fn new(mode: WaypointMode) -> Self {
        let mut waypoint = Self::default();
        waypoint.set(mode);
        waypoint
    }

    /// The current waypoint, or `None` when none is set.
    pub fn mode(&self) -> Option<&WaypointMode> {
        self.mode.as_ref()
    }

    /// Monotonic id of the *current* waypoint (issue #702).
    ///
    /// Bumped by [`set`](Self::set) and [`clear`](Self::clear) whenever they
    /// actually change which waypoint is named. This is what carries the
    /// Channel-3 Navigation→Helm lag now that `AiMemory.nav_goal` is gone:
    /// `CoordinationPayload::NavigateTo` carries a generation rather than a
    /// position, `process_coordination_lag` latches it into
    /// `HelmWaypointClearance` when the message comes due, and the AI Helm
    /// follows the waypoint only while `clearance == generation`. Every *new*
    /// waypoint therefore re-incurs the lag — which a bare "has been cleared"
    /// bool would not: that would delay only the first.
    ///
    /// A `u64` counter rather than a timestamp because PRD #620 (P2P lockstep)
    /// needs this to be deterministic across peers.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Set the waypoint, bumping [`generation`](Self::generation) if this names
    /// a different waypoint than the current one.
    ///
    /// Re-setting the same waypoint is a no-op: it neither bumps the generation
    /// (which would re-incur the Channel-3 lag on a Helm already following it)
    /// nor marks the component changed. `operate_navigation_ai` re-sets its
    /// waypoint every tick, so this idempotence is load-bearing rather than an
    /// optimisation.
    pub fn set(&mut self, mode: WaypointMode) {
        match &mut self.mode {
            // Same waypoint: refresh the anchor cache in place, no bump.
            Some(current) if same_waypoint(current, &mode) => *current = mode,
            _ => {
                self.mode = Some(mode);
                self.generation = self.generation.wrapping_add(1);
            }
        }
    }

    /// Clear the waypoint, bumping [`generation`](Self::generation) if one was
    /// set. Clearing an already-clear waypoint is a no-op — see [`set`](Self::set).
    pub fn clear(&mut self) {
        if self.mode.is_some() {
            self.mode = None;
            self.generation = self.generation.wrapping_add(1);
        }
    }

    /// Returns the broadcast-shaped snapshot for the current waypoint, or
    /// `None` if no waypoint is set.
    pub fn snapshot(&self) -> Option<WaypointSnapshot> {
        match &self.mode {
            None => None,
            Some(WaypointMode::Free { x, z }) => Some(WaypointSnapshot {
                x: *x,
                z: *z,
                source_uuid: None,
            }),
            Some(WaypointMode::Anchored {
                source_uuid,
                last_x,
                last_z,
            }) => Some(WaypointSnapshot {
                x: *last_x,
                z: *last_z,
                source_uuid: Some(source_uuid.clone()),
            }),
        }
    }
}

fn handle_navigation_waypoint(
    mut ship_query: Query<
        (&AdmittedCommands, &mut NavigationWaypoint),
        With<crate::server_app::Ship>,
    >,
) {
    for (admitted, mut waypoint) in ship_query.iter_mut() {
        for cmd in admitted.for_target(NAVIGATION_SYSTEM_ID) {
            match &cmd.payload {
                SystemControlPayload::SetNavigationWaypoint { x, z, source_uuid }
                    if x.is_finite() && z.is_finite() =>
                {
                    // Only the goal is written here. The Channel-3 `NavigateTo`
                    // clearance for it is issued by the shared, origin-agnostic
                    // `issue_navigate_to_clearance` — the same issuer the AI
                    // write path relies on (AGENTS.md rule 6).
                    waypoint.set(make_waypoint_mode(*x, *z, source_uuid.as_deref()));
                }
                SystemControlPayload::ClearNavigationWaypoint => {
                    // Clearing needs no clearance message: the waypoint has no
                    // snapshot, so the Helm has nothing to follow either way —
                    // the same shape as the AI path's `waypoint.clear()`.
                    waypoint.clear();
                }
                _ => {}
            }
        }
    }
}

/// Issue the Channel-3 `NavigateTo` order clearing the AI Helm to follow the
/// ship's current waypoint generation (issues #702, #804 follow-up).
///
/// This is the ONE place a `NavigateTo` clearance is enqueued. Neither
/// waypoint writer — `operate_navigation_ai` nor the human admitted path in
/// `handle_navigation_waypoint` — sends its own; both just write the shared
/// [`NavigationWaypoint`], and this system observes it. A waypoint therefore
/// reaches the AI Helm through the same message, the same delivery lag, and
/// the same `HelmWaypointClearance` latch regardless of who set it (AGENTS.md
/// rule 6). The message carries the waypoint's `generation`, not its
/// position: the waypoint itself is the goal.
///
/// Issue policy (see [`NavClearanceIssueState`]):
/// - Once per new waypoint generation, whatever the helm's control state —
///   preserving the delivery-time matrix (AI helm consumes and latches after
///   the lag; a human helm gets one popup from an AI sender, or silence from
///   a human sender). Exactly once, never per tick: the queue and a human
///   helm's popups must not be flooded.
/// - Again on the helm axes' Human→AI edge while the current generation is
///   unlatched — the disconnect/Backfill hole: an order delivered to a human
///   helm never latches, so without this re-issue an AI helm taking over
///   would never fly the existing waypoint. `helm_axes_operate_ai` is a
///   target-control-state check, not a sender-identity check (rule 6).
///
/// The delivery lag is never bypassed — every (re-)issue goes through the
/// ship's `CoordinationQueue` with `coordination_lag_secs` like any other
/// Channel-3 traffic.
fn issue_navigate_to_clearance(
    mut ships: Query<
        (
            Entity,
            &NavigationWaypoint,
            &mut NavClearanceIssueState,
            &crate::ship_plugin::ShipSystemControlSources,
            Option<&crate::ship_plugin::HelmWaypointClearance>,
        ),
        With<crate::server_app::Ship>,
    >,
    mut coordination_writer: MessageWriter<crate::ship_plugin::CoordinationEnqueue>,
) {
    for (entity, waypoint, mut state, control_sources, clearance) in ships.iter_mut() {
        let helm_ai = crate::ship_plugin::helm_axes_operate_ai(control_sources);
        let flipped_to_ai = helm_ai && !state.helm_axes_were_ai;
        state.helm_axes_were_ai = helm_ai;

        // No waypoint set: nothing to clear the Helm for. (Clears bump the
        // generation but need no message — the Helm has nothing to follow.)
        let Some(snapshot) = waypoint.snapshot() else {
            continue;
        };
        let generation = waypoint.generation();

        let latched = clearance.map(|c| c.0) == Some(Some(generation));
        if latched {
            // The Helm already holds this clearance; nothing in flight is
            // needed. Keep the marker synced so a later helm flip while
            // latched does not re-issue.
            state.issued_generation = Some(generation);
            continue;
        }

        let new_generation = state.issued_generation != Some(generation);
        if new_generation || flipped_to_ai {
            coordination_writer.write(crate::ship_plugin::CoordinationEnqueue {
                source_entity: entity,
                // The origin is the navigation system's resolved control
                // source — derived from target state like every other
                // post-admission enqueuer, never from the wire path.
                sender_origin: control_sources
                    .0
                    .source_for(&crate::system_registry::navigation_system_id()),
                target: crate::system_registry::helm_station_key(),
                payload: crate::messages::CoordinationPayload::NavigateTo {
                    generation,
                    // Coords for the chatter popup's display only (issue #977);
                    // the Helm latches on `generation` and reads the waypoint
                    // itself, so nothing steers off these.
                    x: snapshot.x,
                    z: snapshot.z,
                },
                sender_label: crate::ship::coordination::CHATTER_SENDER_NAVIGATION.to_string(),
            });
            state.issued_generation = Some(generation);
        }
    }
}

/// Build the appropriate `WaypointMode` from raw coordinates and an optional
/// anchor UUID. An empty UUID string is treated as "no anchor" (free waypoint).
fn make_waypoint_mode(x: f32, z: f32, source_uuid: Option<&str>) -> WaypointMode {
    match source_uuid {
        Some(uuid) if !uuid.is_empty() => WaypointMode::Anchored {
            source_uuid: uuid.to_string(),
            last_x: x,
            last_z: z,
        },
        _ => WaypointMode::Free { x, z },
    }
}

/// Each tick, if any ship's navigation waypoint is anchored to an entity,
/// look up the entity's current `Transform` by `EntityUuid` and refresh
/// the waypoint's stored coordinates. If no entity carries the anchored
/// UUID, auto-clear the waypoint (per the despawn policy). Iterates every
/// ship so both player and NPC waypoints track their anchors.
fn refresh_anchored_waypoint(
    mut waypoint_q: Query<&mut NavigationWaypoint, With<crate::server_app::Ship>>,
    entity_q: Query<(&crate::entity_spawner::EntityUuid, &Transform)>,
) {
    for mut waypoint in waypoint_q.iter_mut() {
        let Some(WaypointMode::Anchored {
            source_uuid,
            last_x,
            last_z,
        }) = waypoint.mode.as_mut()
        else {
            continue;
        };

        let mut found = false;
        for (uuid, transform) in entity_q.iter() {
            if uuid.0 == *source_uuid {
                // Tracking the anchor is not a *new* waypoint: it is the same
                // waypoint, whose parent moved. Mutated in place without
                // bumping the generation, or a waypoint anchored to a moving
                // ship would re-incur the Channel-3 lag every tick and the AI
                // Helm would never follow it (issue #702).
                *last_x = transform.translation.x;
                *last_z = transform.translation.z;
                found = true;
                break;
            }
        }

        if !found {
            // Parent entity has despawned (or never existed). Auto-clear —
            // a real change of waypoint, so this one does bump the generation.
            waypoint.clear();
        }
    }
}

// ── Blackboard publish ────────────────────────────────────────────────────────

/// Per-`Ship` publisher (issue #830). Every ship carries its own
/// `NavigationWaypoint`, so the waypoint field is computed per entity. The chart
/// config (range / shows / selects) is a player-only display surface sourced
/// from the local `ShipClientConfigResource`, so it is gated on `Has<LocalShip>`;
/// NPCs get the default (empty) chart config. Nothing consumes an NPC's nav
/// blackboard, and the wire broadcaster is `LocalShip`-filtered — but the AC
/// asks NPC ships to carry navigation blackboards, so this publishes for every
/// AI-bearing ship (those carrying `ShipSystemBlackboards`).
fn publish_navigation_blackboard(
    ship_config: Res<crate::lobby::server::ShipClientConfigResource>,
    world_config: Option<Res<crate::world::config::WorldConfig>>,
    // The civilian traffic picture (issue #1028). Read-only, so this system
    // stays a pure publisher; the state itself is advanced by
    // `civilian::tick_civilian_traffic` in `SimSet::Input`.
    civilians_q: Query<(
        &crate::entity_spawner::EntityUuid,
        Option<&crate::entity_spawner::EntityName>,
        &crate::civilian::CivilianTraffic,
    )>,
    mut ship_q: Query<
        (
            &NavigationWaypoint,
            bevy::ecs::query::Has<crate::server_app::LocalShip>,
            &mut crate::server_app::ShipSystemBlackboards,
            // Where the human seek landed this system, if anywhere (issue
            // #984). `Option` because only `LocalShip` carries the component,
            // and optional access filters no archetype — the matched set, and
            // so the iteration order, is exactly what it was.
            Option<&crate::ship::components::HumanSeekingHosts>,
        ),
        With<crate::server_app::Ship>,
    >,
) {
    let cfg = &ship_config.0;
    // Built once for the whole publish rather than per ship: it is the same
    // world picture for everyone, and UUID-sorted so archetype order never
    // reaches the wire.
    let civilians = civilian_traffic_rows(&civilians_q, world_config.as_deref());
    let nav_id = SystemId(NAVIGATION_SYSTEM_ID.to_string());
    for (waypoint, is_local, mut bbs, hosts) in ship_q.iter_mut() {
        let navigation_waypoint = waypoint.snapshot();
        let bb = if is_local {
            NavigationBlackboard {
                nav_chart_range: cfg.nav_chart_range,
                nav_chart_shows: cfg.nav_chart_shows.clone(),
                nav_chart_selects: cfg.nav_chart_selects.clone(),
                navigation_waypoint,
                civilians: civilians.clone(),
                host_station: hosts.and_then(|h| h.0.get(&nav_id).cloned()),
            }
        } else {
            NavigationBlackboard {
                navigation_waypoint,
                ..Default::default()
            }
        };
        bbs.0
            .insert(nav_id.clone(), SystemBlackboard::Navigation(bb));
    }
}

/// Project every civilian's live traffic state onto the wire (issue #1028).
///
/// UUID order, for the reason every other authoritative walk sorts: Bevy's
/// archetype iteration order is not part of the simulation's contract, and a
/// traffic list that re-ordered itself between ticks would make the console's
/// rows jump under the operator's finger.
fn civilian_traffic_rows(
    civilians: &Query<(
        &crate::entity_spawner::EntityUuid,
        Option<&crate::entity_spawner::EntityName>,
        &crate::civilian::CivilianTraffic,
    )>,
    world_config: Option<&crate::world::config::WorldConfig>,
) -> Vec<crate::messages::CivilianTrafficSnapshot> {
    use crate::civilian::CivilianOrder;
    let mut rows: Vec<crate::messages::CivilianTrafficSnapshot> = civilians
        .iter()
        .map(|(uuid, name, traffic)| {
            let state = &traffic.0;
            let route = state.route().unwrap_or_default().to_string();
            let legs = world_config
                .and_then(|wc| wc.route(&route))
                .map(|r| r.legs.len() as u32)
                .unwrap_or(0);
            let (order, destination) = match state.order() {
                None => (String::new(), String::new()),
                Some(order) => (
                    order.kind().as_str().to_string(),
                    match order {
                        CivilianOrder::Hold => String::new(),
                        CivilianOrder::Divert { route, anchor } => {
                            route.clone().or_else(|| anchor.clone()).unwrap_or_default()
                        }
                        CivilianOrder::Dock { structure } => structure.clone(),
                    },
                ),
            };
            crate::messages::CivilianTrafficSnapshot {
                uuid: uuid.0.clone(),
                name: name.map(|n| n.0.clone()).unwrap_or_default(),
                route,
                leg: state.leg() as u32,
                legs,
                order,
                order_destination: destination,
                compliance: state.compliance().as_str().to_string(),
                reason: state.reason().unwrap_or_default().to_string(),
            }
        })
        .collect();
    rows.sort_by(|a, b| a.uuid.cmp(&b.uuid));
    rows
}

// ── AI controller ──────────────────────────────────────────────────────────────

/// Per-ship resolved Navigation target selector (issue #778).
///
/// Holds the ship's data-driven [`crate::ai::selector::TargetSelector`], decoded
/// from the authored `[navigation_console.selector]` block, plus the authored
/// ship `power_rating`, which `operate_navigation_ai` exposes to the selector's
/// expressions as `self_fact(power_rating)`. Attached at spawn alongside the
/// Sensors/Tactical selectors.
///
/// Since #885b stage 5d there is no Rust-side synthesised default behind it: a
/// ship without the component ranks nothing and `operate_navigation_ai` skips
/// it. Mirrors [`crate::ship::sensors::SensorsTargetSelector`].
#[derive(Component, Clone, Debug)]
pub struct NavigationTargetSelector {
    /// The resolved ranking policy.
    pub selector: crate::ai::selector::TargetSelector,
    /// Authored ship power rating, seeded from `EntityConfig.power_rating`.
    pub power_rating: Option<f32>,
}

/// Per-entity AI loop for navigation. Loops over ALL ship entities (player and NPC)
/// where the Navigation system is `ControlSource::Ai`.
///
/// Reads the viewscreen blackboard's `scored_objectives`, ranks the positive
/// Helm-relevant objectives and the eligible chart contacts through the
/// REUSABLE [`crate::ai::selector::TargetSelector`] (issue #778), and —
/// decide-and-emit (issue #830) — emits an admitted `SetNavigationWaypoint` /
/// `ClearNavigationWaypoint` through the shared
/// [`crate::command_admission::validate_and_admit`] seam with this ship's own
/// `ai:<uuid>` token, rather than writing `NavigationWaypoint` directly (the §2
/// violation). `handle_navigation_waypoint` applies it later this tick (Physics,
/// `.after(operate_navigation_ai)`). Emission is on-change only (compared
/// against the current `NavigationWaypoint`): the applier's `set` is
/// generation-idempotent, but emit-on-change keeps `AdmittedCommands` clean and
/// mirrors #828. The Channel-3 `NavigateTo` clearance is issued by
/// [`issue_navigate_to_clearance`], the shared origin-agnostic issuer — not here.
///
/// # Selector reuse — a UUID spine driving a Waypoint (issue #778)
///
/// [`crate::ai::selector::TargetSelector::select`] returns one candidate UUID,
/// but Navigation needs a [`WaypointMode`] (a fixed `Free` anchor OR a live
/// `Anchored` entity). So while assembling candidates the host builds a
/// parallel `uuid → WaypointMode` side-table: an entity-anchored candidate (a
/// Destroy objective's target, a chart contact) keys on the real entity UUID
/// and maps to `Anchored`; a fixed-anchor candidate (Reach / Retreat / Patrol →
/// world anchors, which are positions, not entities) synthesises a stable
/// position-derived key and maps to `Free`. The winning UUID is looked back up
/// through the side-table to recover the waypoint variant, so the entity-UUID
/// selector spine is reused unchanged.
pub fn operate_navigation_ai(
    sessions: Res<crate::lobby::Sessions>,
    mut ships: Query<(
        Entity,
        &crate::ship_plugin::ShipSystemControlSources,
        &crate::server_app::ShipSystemBlackboards,
        &NavigationWaypoint,
        &crate::ship_state::ShipPhysics,
        Option<&crate::entity_spawner::EntityUuid>,
        Option<&crate::ai_plugin::ObjectiveCursors>,
        Option<&crate::ship_plugin::ShipConfigComponent>,
        Option<&NavigationTargetSelector>,
        &mut crate::messages::AdmittedCommands,
    )>,
    entities: Query<(
        &crate::entity_spawner::EntityUuid,
        &Transform,
        Option<&crate::entities::spawner::EntityName>,
    )>,
    runtime: Option<Res<crate::world::server::WorldContentRuntime>>,
    // Loaded sub-world layers (issue #891 stage 2): the selector's flag chain
    // is anchored at the layer that spawned each ship.
    layers: Option<Res<crate::world::server::WorldLayerMap>>,
    // The per-ship origin-layer stamp (issue #891 review finding 1): an O(1)
    // read replacing the old `WorldLayerMap` scan inside `entity_flag_chain`.
    origin_q: Query<&crate::world::server::EntityOriginLayer>,
    world_config: Option<Res<crate::world::config::WorldConfig>>,
) {
    let all_entities: Vec<(String, Option<String>, [f32; 3])> = entities
        .iter()
        .map(|(uuid, transform, name)| {
            (
                uuid.0.clone(),
                name.map(|n| n.0.clone()),
                [
                    transform.translation.x,
                    transform.translation.y,
                    transform.translation.z,
                ],
            )
        })
        .collect();

    for (
        ship_entity,
        sources,
        blackboards,
        waypoint,
        ship_physics,
        entity_uuid,
        cursors,
        ship_config,
        target_selector,
        mut admitted,
    ) in ships.iter_mut()
    {
        let policy = sources
            .0
            .policy_for(&crate::system_registry::navigation_system_id());
        if !policy.operate_ai {
            continue;
        }
        // No authored `[navigation_console.selector]` ⇒ no component ⇒ no
        // destination ranking. Since #885b stage 5d there is no synthesised
        // stand-in.
        let Some(selector_comp) = target_selector else {
            continue;
        };

        let scored: Vec<crate::messages::ScoredObjective> = match blackboards
            .0
            .get(&crate::system_registry::viewscreen_system_id())
        {
            Some(crate::messages::SystemBlackboard::Viewscreen(bb)) => bb.scored_objectives.clone(),
            _ => vec![],
        };

        // ── Build candidate sources for the reusable selector (#778) ──────────
        // The selector ranks entity UUIDs; Navigation needs Waypoints, so a
        // parallel `uuid → WaypointMode` side-table records the intended variant
        // for every candidate, and the winning UUID is looked back up through it.
        use crate::ai::selector::{SelectorCandidate, SelfContext};
        let mut candidates: Vec<SelectorCandidate> = Vec::new();
        let mut modes: std::collections::HashMap<String, WaypointMode> =
            std::collections::HashMap::new();

        // Source: navigation-objectives. The host ranks the objective pool by
        // score (the retired "top positive Helm-relevant objective" filter) and
        // resolves the winner's directive to its destination — the sole
        // `reachable` candidate this source contributes. There is deliberately no
        // range filter: Navigation is the ship's whole-system chart, and the
        // Channel-3 hand-off exists to steer a short-ranged Helm toward something
        // it cannot yet see for itself.
        let top = scored
            .iter()
            .filter(|o| {
                o.score > 0.0 && o.relevance.contains(&crate::messages::SystemAffinity::Helm)
            })
            .max_by(|a, b| {
                a.score
                    .partial_cmp(&b.score)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
        if let Some(top_obj) = top {
            match &top_obj.directive {
                // A `Destroy` target may be authored as a UUID *or* an entity
                // name (`combat_test.toml` names "Starbase Alpha"), so resolve
                // both the way every other objective consumer does. The candidate
                // keys on the *resolved* entity UUID and maps to `Anchored`, so
                // the waypoint tracks a moving target by UUID rather than name.
                // Dock (issue #1028) resolves EXACTLY as Destroy does, through
                // the same resolver and onto the same `Anchored` waypoint: both
                // name a live entity to close on, and the only difference is
                // what the hull does once it arrives. Sharing the arm is what
                // makes a civilian's berthing approach the same code path a
                // warship's attack run already uses.
                crate::messages::AiDirective::Destroy { target }
                | crate::messages::AiDirective::Dock { target }
                    if !target.is_empty() =>
                {
                    if let Some((uuid, pos)) =
                        resolve_destroy_target(&all_entities, runtime.as_deref(), target)
                    {
                        modes.insert(
                            uuid.clone(),
                            WaypointMode::Anchored {
                                source_uuid: uuid.clone(),
                                last_x: pos[0],
                                last_z: pos[2],
                            },
                        );
                        candidates.push(nav_objective_candidate(&uuid, pos, top_obj.score));
                    }
                }
                // Reach / Retreat name a fixed world anchor: a `Free` waypoint
                // keyed on a stable position-derived synthetic UUID (anchors are
                // positions, not entities, so there is no real UUID to key on).
                crate::messages::AiDirective::Reach { anchor }
                | crate::messages::AiDirective::Retreat { anchor }
                    if !anchor.is_empty() =>
                {
                    if let Some(pos) = anchor_pos(&world_config, anchor) {
                        let key = free_candidate_key(pos[0], pos[2]);
                        modes.insert(
                            key.clone(),
                            WaypointMode::Free {
                                x: pos[0],
                                z: pos[2],
                            },
                        );
                        candidates.push(nav_objective_candidate(&key, pos, top_obj.score));
                    }
                }
                // Patrol resolves from the objective's *active cursor target*, not
                // `anchors[0]` (issue #702): the cursor is the objective's own
                // record of where it is on its route — the same one `helm_patrol`
                // steers from — so Navigation and Helm agree on the current leg.
                crate::messages::AiDirective::Patrol { anchors, loop_path } => {
                    let index = cursors
                        .and_then(|c| {
                            c.0.iter()
                                .find(|cursor| cursor.objective_id == top_obj.id)
                                .map(|cursor| cursor.index())
                        })
                        .unwrap_or(0);
                    let world_anchors = world_config
                        .as_ref()
                        .map(|wc| wc.anchors.clone())
                        .unwrap_or_default();
                    if let Some(pos) = crate::ai::patrol_cursor::cursor_target(
                        index,
                        anchors,
                        *loop_path,
                        &world_anchors,
                    ) {
                        let key = free_candidate_key(pos[0], pos[2]);
                        modes.insert(
                            key.clone(),
                            WaypointMode::Free {
                                x: pos[0],
                                z: pos[2],
                            },
                        );
                        candidates.push(nav_objective_candidate(&key, pos, top_obj.score));
                    }
                }
                _ => {}
            }
        }

        // Source: chart-contacts. Every live chart entity is surfaced as an
        // entity-anchored candidate. Under the canonical policy they lack the
        // `reachable` marker (default eligibility admits only `reachable`), so
        // they do not independently select — they merge their
        // `source_chart_contact` marker into a coincident objective destination
        // (the selector dedups by UUID, folding facts) and stand ready for an
        // author to weight into eligible destinations.
        for (uuid, _name, pos) in &all_entities {
            modes.entry(uuid.clone()).or_insert(WaypointMode::Anchored {
                source_uuid: uuid.clone(),
                last_x: pos[0],
                last_z: pos[2],
            });
            candidates.push(chart_contact_candidate(uuid, *pos));
        }

        // Self context: position (horizon filter) + authored power rating,
        // exposed to the selector expressions as `self_fact(power_rating)`.
        let mut self_facts = crate::world::flags::AiFacts::new();
        if let Some(pr) = selector_comp.power_rating {
            self_facts.set("power_rating", pr as f64);
        }
        let self_ctx = SelfContext {
            position: [ship_physics.x, 0.0, ship_physics.z],
            facts: self_facts,
        };

        // A stable "current" key derived from the ship's current waypoint so the
        // selector's hysteresis (AC3) can retain it: an `Anchored` waypoint keys
        // on its entity UUID, a `Free` waypoint on its position-derived synthetic
        // UUID — the same keys the candidates above use.
        let current_key = match waypoint.mode() {
            Some(WaypointMode::Anchored { source_uuid, .. }) => Some(source_uuid.clone()),
            Some(WaypointMode::Free { x, z }) => Some(free_candidate_key(*x, *z)),
            None => None,
        };

        // Rank through the reusable selector, then map the winning UUID back to
        // the waypoint variant via the side-table. No eligible winner ⇒ clear.
        // The scenario flag chain is anchored at the layer that spawned this
        // ship (issue #891 stage 2).
        let flag_chain = crate::world::server::entity_flag_chain(
            origin_q.get(ship_entity).ok(),
            runtime.as_deref(),
            layers.as_deref(),
        );
        let desired: Option<WaypointMode> = selector_comp
            .selector
            .select(&self_ctx, &candidates, current_key.as_deref(), &flag_chain)
            .and_then(|uuid| modes.get(&uuid).cloned());

        // ── Emit on change only (issue #828 shape) ───────────────────────────
        // Compare against the current waypoint by identity (`same_waypoint`):
        // an anchored waypoint whose parent has merely moved is the same
        // waypoint, so no re-emit — `refresh_anchored_waypoint` keeps its cache
        // fresh independently.
        let changed = match (waypoint.mode(), &desired) {
            (None, None) => false,
            (Some(_), None) | (None, Some(_)) => true,
            (Some(current), Some(next)) => !same_waypoint(current, next),
        };
        if !changed {
            continue;
        }

        let payload = match &desired {
            Some(WaypointMode::Free { x, z }) => SystemControlPayload::SetNavigationWaypoint {
                x: *x,
                z: *z,
                source_uuid: None,
            },
            Some(WaypointMode::Anchored {
                source_uuid,
                last_x,
                last_z,
            }) => SystemControlPayload::SetNavigationWaypoint {
                x: *last_x,
                z: *last_z,
                source_uuid: Some(source_uuid.clone()),
            },
            None => SystemControlPayload::ClearNavigationWaypoint,
        };

        emit_navigation_ai_command(
            entity_uuid,
            payload,
            sources,
            &sessions,
            ship_config,
            &mut admitted,
        );
    }
}

/// Emit an admitted Navigation AI command targeting the navigation system
/// through the shared [`crate::command_admission::validate_and_admit`] seam,
/// using this ship's own `ai:<uuid>` token (mirrors `emit_sensors_ai_command`).
fn emit_navigation_ai_command(
    entity_uuid: Option<&crate::entity_spawner::EntityUuid>,
    payload: SystemControlPayload,
    sources: &crate::ship_plugin::ShipSystemControlSources,
    sessions: &crate::lobby::Sessions,
    ship_config: Option<&crate::ship_plugin::ShipConfigComponent>,
    admitted: &mut AdmittedCommands,
) -> bool {
    emit_ai_command(
        entity_uuid,
        crate::system_registry::navigation_system_id(),
        payload,
        sources,
        sessions,
        ship_config,
        admitted,
    )
}

/// A stable synthetic candidate UUID for a fixed-anchor (`Free`) destination,
/// derived purely from its planar position (issue #778).
///
/// Anchors are world positions, not entities, so there is no real UUID to key a
/// `Free` candidate on. The key must be reconstructable from the ship's current
/// [`WaypointMode::Free`] for the selector's hysteresis to retain it, so it is
/// derived from the exact `x`/`z` the waypoint stores — the same values the
/// candidate carries — making the build-time and current-time keys bit-identical.
fn free_candidate_key(x: f32, z: f32) -> String {
    format!("anchor:{x}:{z}")
}

/// One `navigation-objectives` candidate: a genuinely reachable destination
/// Navigation has been ordered toward (issue #778). Carries the `reachable`
/// marker the default eligibility keys on, its source marker, and the
/// originating objective's score (authorable via `candidate_fact(objective_score)`).
fn nav_objective_candidate(
    uuid: &str,
    position: [f32; 3],
    objective_score: f32,
) -> crate::ai::selector::SelectorCandidate {
    let mut facts = crate::world::flags::AiFacts::new();
    facts.set("reachable", 1.0);
    facts.set("source_nav_objective", 1.0);
    facts.set("objective_score", objective_score as f64);
    crate::ai::selector::SelectorCandidate {
        uuid: uuid.to_string(),
        position,
        facts,
    }
}

/// One `chart-contacts` candidate: a live entity the Navigation chart shows
/// (issue #778). It carries only `source_chart_contact`, NOT `reachable`, so
/// under the canonical policy it enriches a coincident objective destination
/// rather than independently steering the ship; an author may widen the
/// selector's eligibility to admit it as a destination in its own right.
fn chart_contact_candidate(
    uuid: &str,
    position: [f32; 3],
) -> crate::ai::selector::SelectorCandidate {
    let mut facts = crate::world::flags::AiFacts::new();
    facts.set("source_chart_contact", 1.0);
    crate::ai::selector::SelectorCandidate {
        uuid: uuid.to_string(),
        position,
        facts,
    }
}

/// Resolve a `Destroy` directive's target to `(uuid, position)`.
///
/// `target` may be an entity UUID or an entity name. The world runtime's
/// `name_to_uuid` map is the authoritative name index; the scan over spawned
/// entities covers names that were assigned at spawn time without going through
/// the runtime (and doubles as the UUID path).
fn resolve_destroy_target(
    entities: &[(String, Option<String>, [f32; 3])],
    runtime: Option<&crate::world::server::WorldContentRuntime>,
    target: &str,
) -> Option<(String, [f32; 3])> {
    let mapped = runtime.and_then(|rt| rt.name_to_uuid.get(target));
    entities
        .iter()
        .find(|(uuid, name, _)| {
            Some(uuid) == mapped || uuid == target || name.as_deref().is_some_and(|n| n == target)
        })
        .map(|(uuid, _, pos)| (uuid.clone(), *pos))
}

/// World position of a named anchor, if the world config declares one.
fn anchor_pos(
    world_config: &Option<Res<crate::world::config::WorldConfig>>,
    anchor: &str,
) -> Option<[f32; 3]> {
    world_config
        .as_ref()
        .and_then(|wc| wc.anchors.get(anchor).copied())
}

#[cfg(test)]
// Fixture ids only (issue #907): a test that needs "some distinct id" has no
// run to reproduce. Production identity is minted by `crate::world_id`, and
// clippy.toml bans `Uuid::new_v4` outside scopes like this one.
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;
    use crate::lobby::{InboundMessage, LobbyPlugin, OutboundMessage};
    use crate::messages::{ClientMessage, ServerMessage};
    use crate::server_app::{
        sim_state_broadcaster, LastBroadcastEntityPositions, LastBroadcastHull,
        LastBroadcastShields, ShipImpulse,
    };

    #[derive(Resource, Default)]
    struct Outbox(Vec<OutboundMessage>);

    fn collect(mut reader: MessageReader<OutboundMessage>, mut sink: ResMut<Outbox>) {
        for msg in reader.read() {
            sink.0.push(msg.clone());
        }
    }

    fn test_app() -> App {
        let mut app = App::new();
        app.add_plugins(LobbyPlugin)
            .add_plugins(bevy::time::TimePlugin)
            .add_plugins(crate::server_app::AdmissionPlugin)
            // Chain SimSet phases so handle (Input) → refresh (Modifiers) →
            // broadcast (Broadcast) run in the right order. Without this,
            // adding a second resource-touching system to a different set
            // makes the schedule non-deterministic and breaks the existing
            // broadcast assertions.
            .configure_sets(
                FixedUpdate,
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
            .init_resource::<crate::server_app::LastBroadcastBlackboards>()
            .init_resource::<crate::lobby::server::ShipClientConfigResource>()
            .add_plugins(NavigationPlugin)
            .add_plugins(sim_state_broadcaster())
            .add_plugins(crate::server_app::sim_outbox_broadcaster())
            .init_resource::<crate::simulation::SimOutbox>()
            .add_systems(
                FixedUpdate,
                crate::server_app::broadcast_blackboard_updates
                    .in_set(crate::sim_sets::SimSet::PublishAggregate),
            )
            .init_resource::<Outbox>()
            .init_resource::<LastBroadcastEntityPositions>()
            .init_resource::<crate::simulation::LastBroadcastEntityHealth>()
            .init_resource::<LastBroadcastHull>()
            .init_resource::<LastBroadcastShields>()
            .add_message::<crate::ship_plugin::CoordinationEnqueue>()
            .add_systems(PostUpdate, collect);
        // Spawn the player ship entity so handle_navigation_waypoint can query it.
        app.world_mut().spawn((
            crate::simulation::Ship,
            crate::simulation::LocalShip,
            crate::simulation::ShipSystemBlackboards::default(),
            crate::ship_plugin::ShipConfigComponent::default(),
            crate::ship_plugin::ShipSystemControlSources::default(),
            crate::messages::AdmittedCommands::default(),
            crate::ship_plugin::ActiveStationRatings::default(),
            crate::ship_plugin::CoordinationQueue::default(),
            // PR 7 (issue #597) — NavigationWaypoint is now a per-entity Component.
            NavigationWaypoint::default(),
            ShipImpulse(crate::impulse::ImpulseState::new()),
            crate::modifiers::ShipModifiers::new(),
            crate::ship_state::ShipPhysics::default(),
            // The AUTHORED `[navigation_console.selector]` block every shipped
            // hull carries. Since #885b stage 5d `operate_navigation_ai` has no
            // synthesised fallback — a ship with no selector ranks nothing — so
            // a fixture that wants a waypoint must attach the declaration a real
            // hull writes.
            NavigationTargetSelector {
                selector: crate::entities::authored_ai_pins::shipped_selector_toml("navigation")
                    .to_selector()
                    .expect("the shipped Navigation selector decodes"),
                power_rating: None,
            },
        ));
        // One fixed step per update (issue #895): the plugin's systems run on
        // the logical tick, and each harness tick advances it once.
        crate::ship::test_support::drive_one_fixed_step_per_update(
            &mut app,
            std::time::Duration::from_millis(200),
        );
        app
    }

    /// PR 7 test helper — read the LocalShip's `NavigationWaypoint` component.
    fn get_nav_waypoint(app: &mut App) -> Option<WaypointMode> {
        let mut q = app
            .world_mut()
            .query_filtered::<&NavigationWaypoint, With<crate::server_app::LocalShip>>();
        q.single(app.world()).ok().and_then(|w| w.mode().cloned())
    }

    /// The LocalShip waypoint's current generation (issue #702).
    fn nav_waypoint_generation(app: &mut App) -> u64 {
        let mut q = app
            .world_mut()
            .query_filtered::<&NavigationWaypoint, With<crate::server_app::LocalShip>>();
        q.single(app.world())
            .expect("LocalShip must carry a NavigationWaypoint")
            .generation()
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
        let out = app.world().resource::<Outbox>().0.clone();
        app.world_mut().resource_mut::<Outbox>().0.clear();
        out
    }

    fn start_game_with_navigation(app: &mut App) {
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
            "navigation",
            ClientMessage::Identify {
                token: "navigation".into(),
                name: "Decker".into(),
            },
        );
        tick(app);
        push(
            app,
            "navigation",
            ClientMessage::SelectStation {
                station: "Navigation".into(),
            },
        );
        tick(app);
        push(app, "captain", ClientMessage::SetReady { ready: true });
        push(app, "navigation", ClientMessage::SetReady { ready: true });
        tick(app);
    }

    // Test helper mirroring `latest_navigation_blackboard` below; no test in
    // this module currently asserts on raw SimSnapshot, retained for parity.
    #[allow(dead_code)]
    fn latest_sim_snapshot(out: &[OutboundMessage]) -> Option<crate::messages::SimSnapshot> {
        out.iter().rev().find_map(|m| match &m.msg {
            ServerMessage::SimState { snapshot } => Some(snapshot.clone()),
            _ => None,
        })
    }

    fn latest_navigation_blackboard(
        out: &[OutboundMessage],
    ) -> Option<crate::messages::NavigationBlackboard> {
        out.iter().rev().find_map(|m| match &m.msg {
            ServerMessage::BlackboardUpdate { updates } => {
                updates.iter().find_map(|(_, bb)| match bb {
                    crate::messages::SystemBlackboard::Navigation(nav) => Some(nav.clone()),
                    _ => None,
                })
            }
            _ => None,
        })
    }

    /// **Issue #1028, AC4.** A civilian's lane, leg and compliance reach the
    /// Navigation blackboard, so a console can show who is and is not doing as
    /// asked — and a world with no traffic publishes exactly what it did before.
    #[test]
    fn the_navigation_blackboard_carries_the_civilian_traffic_picture() {
        use crate::civilian::{
            CivilianConfig, CivilianOrder, CivilianState, CivilianTraffic, ComplianceDisposition,
        };

        // Read off the local ship's own blackboard rather than off the wire:
        // `broadcast_blackboard_updates` is diffed, so an unchanged picture is
        // deliberately not re-sent and the control below would have nothing to
        // look at.
        fn local_blackboard(app: &mut App) -> crate::messages::NavigationBlackboard {
            let mut q = app.world_mut().query_filtered::<
                &crate::server_app::ShipSystemBlackboards,
                With<crate::server_app::LocalShip>,
            >();
            let bbs = q
                .iter(app.world())
                .next()
                .expect("the local ship publishes");
            match bbs.0.get(&SystemId(NAVIGATION_SYSTEM_ID.to_string())) {
                Some(crate::messages::SystemBlackboard::Navigation(nav)) => nav.clone(),
                other => panic!("expected a navigation blackboard, got {other:?}"),
            }
        }

        let mut app = test_app();
        start_game_with_navigation(&mut app);
        tick(&mut app);
        assert!(
            local_blackboard(&mut app).civilians.is_empty(),
            "a world with no civilian traffic publishes the payload it always did"
        );

        // One hauler, ordered to dock and already complying.
        let config = CivilianConfig {
            route: Some("depot_run".into()),
            ..CivilianConfig::default()
        };
        let mut state = CivilianState::from_config(&config);
        let disposition = ComplianceDisposition {
            ack_secs: 0,
            decide_secs: 0,
            ..ComplianceDisposition::default()
        };
        state.receive_order(
            CivilianOrder::dock_at("world.entity.skyhook_depot.name"),
            &disposition,
            0,
            60.0,
        );
        state.advance(0, true, &disposition, 60.0);
        state.advance(0, true, &disposition, 60.0);
        app.world_mut().spawn((
            crate::entity_spawner::EntityUuid("civ-1".into()),
            crate::entity_spawner::EntityName("world.entity.hauler_kestrel.name".into()),
            CivilianTraffic(state),
        ));

        let out = tick(&mut app);
        let bb = latest_navigation_blackboard(&out)
            .expect("the changed traffic picture reaches the wire");
        assert_eq!(
            bb.civilians.len(),
            1,
            "the hauler is on the traffic picture"
        );
        let row = &bb.civilians[0];
        assert_eq!(
            row.uuid, "civ-1",
            "the row key is what an order names it by"
        );
        assert_eq!(row.name, "world.entity.hauler_kestrel.name");
        assert_eq!(row.route, "depot_run");
        assert_eq!(row.order, "dock");
        assert_eq!(row.order_destination, "world.entity.skyhook_depot.name");
        assert_eq!(
            row.compliance, "complying",
            "the whole point of the row: whether it is doing as it was asked"
        );
    }

    #[test]
    fn navigation_holder_can_set_and_clear_waypoint() {
        let mut app = test_app();
        start_game_with_navigation(&mut app);

        push(
            &mut app,
            "navigation",
            ClientMessage::ControlSystem {
                target: crate::messages::SystemId("navigation".into()),
                payload: SystemControlPayload::SetNavigationWaypoint {
                    x: 120.0,
                    z: -45.0,
                    source_uuid: None,
                },
            },
        );
        tick(&mut app);
        assert_eq!(
            get_nav_waypoint(&mut app),
            Some(WaypointMode::Free { x: 120.0, z: -45.0 })
        );

        push(
            &mut app,
            "navigation",
            ClientMessage::ControlSystem {
                target: crate::messages::SystemId("navigation".into()),
                payload: SystemControlPayload::ClearNavigationWaypoint,
            },
        );
        tick(&mut app);
        assert!(get_nav_waypoint(&mut app).is_none());
    }

    #[test]
    fn non_navigation_sender_cannot_change_waypoint() {
        let mut app = test_app();
        start_game_with_navigation(&mut app);

        push(
            &mut app,
            "captain",
            ClientMessage::ControlSystem {
                target: crate::messages::SystemId("navigation".into()),
                payload: SystemControlPayload::SetNavigationWaypoint {
                    x: 5.0,
                    z: 6.0,
                    source_uuid: None,
                },
            },
        );
        tick(&mut app);
        assert!(get_nav_waypoint(&mut app).is_none());
    }

    #[test]
    fn invalid_waypoint_coordinates_are_ignored() {
        let mut app = test_app();
        start_game_with_navigation(&mut app);

        push(
            &mut app,
            "navigation",
            ClientMessage::ControlSystem {
                target: crate::messages::SystemId("navigation".into()),
                payload: SystemControlPayload::SetNavigationWaypoint {
                    x: f32::NAN,
                    z: 1.0,
                    source_uuid: None,
                },
            },
        );
        tick(&mut app);
        assert!(get_nav_waypoint(&mut app).is_none());
    }

    #[test]
    fn sim_state_broadcast_includes_and_omits_waypoint() {
        let mut app = test_app();
        start_game_with_navigation(&mut app);

        push(
            &mut app,
            "navigation",
            ClientMessage::ControlSystem {
                target: crate::messages::SystemId("navigation".into()),
                payload: SystemControlPayload::SetNavigationWaypoint {
                    x: 10.0,
                    z: 20.0,
                    source_uuid: None,
                },
            },
        );
        let out = tick(&mut app);
        let bb = latest_navigation_blackboard(&out).expect("expected NavigationBlackboard");
        assert_eq!(
            bb.navigation_waypoint,
            Some(WaypointSnapshot {
                x: 10.0,
                z: 20.0,
                source_uuid: None,
            })
        );

        push(
            &mut app,
            "navigation",
            ClientMessage::ControlSystem {
                target: crate::messages::SystemId("navigation".into()),
                payload: SystemControlPayload::ClearNavigationWaypoint,
            },
        );
        let out = tick(&mut app);
        let bb = latest_navigation_blackboard(&out).expect("expected NavigationBlackboard");
        assert!(bb.navigation_waypoint.is_none());
    }

    #[test]
    fn anchored_waypoint_tracks_moving_entity() {
        let mut app = test_app();
        start_game_with_navigation(&mut app);

        // Spawn an entity carrying EntityUuid + Transform that the waypoint
        // will anchor to.
        let target_uuid = "target-1";
        let target = app
            .world_mut()
            .spawn((
                crate::entity_spawner::EntityUuid(target_uuid.into()),
                Transform::from_xyz(50.0, 0.0, -100.0),
            ))
            .id();

        // Anchor the waypoint to that entity. The seed coords are the
        // entity's current position.
        push(
            &mut app,
            "navigation",
            ClientMessage::ControlSystem {
                target: crate::messages::SystemId("navigation".into()),
                payload: SystemControlPayload::SetNavigationWaypoint {
                    x: 50.0,
                    z: -100.0,
                    source_uuid: Some(target_uuid.into()),
                },
            },
        );
        let out = tick(&mut app);
        let bb = latest_navigation_blackboard(&out).expect("expected NavigationBlackboard");
        assert_eq!(
            bb.navigation_waypoint,
            Some(WaypointSnapshot {
                x: 50.0,
                z: -100.0,
                source_uuid: Some(target_uuid.into()),
            })
        );

        // Move the entity. The next broadcast should reflect the new
        // position with source_uuid preserved.
        app.world_mut()
            .entity_mut(target)
            .insert(Transform::from_xyz(75.0, 0.0, -150.0));
        let out = tick(&mut app);
        let bb = latest_navigation_blackboard(&out).expect("expected NavigationBlackboard");
        assert_eq!(
            bb.navigation_waypoint,
            Some(WaypointSnapshot {
                x: 75.0,
                z: -150.0,
                source_uuid: Some(target_uuid.into()),
            })
        );
    }

    #[test]
    fn anchored_waypoint_auto_clears_when_parent_despawns() {
        let mut app = test_app();
        start_game_with_navigation(&mut app);

        let target_uuid = "target-despawn";
        let target = app
            .world_mut()
            .spawn((
                crate::entity_spawner::EntityUuid(target_uuid.into()),
                Transform::from_xyz(10.0, 0.0, 20.0),
            ))
            .id();

        push(
            &mut app,
            "navigation",
            ClientMessage::ControlSystem {
                target: crate::messages::SystemId("navigation".into()),
                payload: SystemControlPayload::SetNavigationWaypoint {
                    x: 10.0,
                    z: 20.0,
                    source_uuid: Some(target_uuid.into()),
                },
            },
        );
        tick(&mut app);
        assert!(get_nav_waypoint(&mut app).is_some());

        // Despawn the parent entity. The next tick must auto-clear.
        app.world_mut().entity_mut(target).despawn();
        let out = tick(&mut app);
        assert!(get_nav_waypoint(&mut app).is_none());
        let bb = latest_navigation_blackboard(&out).expect("expected NavigationBlackboard");
        assert!(bb.navigation_waypoint.is_none());
    }

    #[test]
    fn empty_source_uuid_is_treated_as_free_waypoint() {
        let mut app = test_app();
        start_game_with_navigation(&mut app);

        push(
            &mut app,
            "navigation",
            ClientMessage::ControlSystem {
                target: crate::messages::SystemId("navigation".into()),
                payload: SystemControlPayload::SetNavigationWaypoint {
                    x: 1.0,
                    z: 2.0,
                    source_uuid: Some(String::new()),
                },
            },
        );
        tick(&mut app);
        assert_eq!(
            get_nav_waypoint(&mut app),
            Some(WaypointMode::Free { x: 1.0, z: 2.0 })
        );
    }

    // ── ControlSystem dispatch tests ─────────────────────────────────────────

    /// Navigation holder sends `ControlSystem` waypoint — accepted.
    #[test]
    fn control_system_navigation_holder_can_set_and_clear_waypoint() {
        let mut app = test_app();
        start_game_with_navigation(&mut app);

        push(
            &mut app,
            "navigation",
            ClientMessage::ControlSystem {
                target: crate::messages::SystemId("navigation".into()),
                payload: SystemControlPayload::SetNavigationWaypoint {
                    x: 200.0,
                    z: -80.0,
                    source_uuid: None,
                },
            },
        );
        tick(&mut app);
        assert_eq!(
            get_nav_waypoint(&mut app),
            Some(WaypointMode::Free { x: 200.0, z: -80.0 })
        );

        push(
            &mut app,
            "navigation",
            ClientMessage::ControlSystem {
                target: crate::messages::SystemId("navigation".into()),
                payload: SystemControlPayload::ClearNavigationWaypoint,
            },
        );
        tick(&mut app);
        assert!(get_nav_waypoint(&mut app).is_none());
    }

    /// Non-navigation sender sends `ControlSystem` waypoint — rejected.
    #[test]
    fn control_system_unauthorized_sender_rejected() {
        let mut app = test_app();
        start_game_with_navigation(&mut app);

        push(
            &mut app,
            "captain",
            ClientMessage::ControlSystem {
                target: crate::messages::SystemId("navigation".into()),
                payload: SystemControlPayload::SetNavigationWaypoint {
                    x: 5.0,
                    z: 6.0,
                    source_uuid: None,
                },
            },
        );
        tick(&mut app);
        assert!(
            get_nav_waypoint(&mut app).is_none(),
            "non-navigation sender should be rejected"
        );
    }

    /// When navigation system is AI-controlled, `ControlSystem` waypoint is rejected.
    #[test]
    fn control_system_rejected_when_ai_controlled() {
        let mut app = test_app();
        start_game_with_navigation(&mut app);

        {
            let mut q = app.world_mut().query_filtered::<&mut crate::ship_plugin::ShipSystemControlSources, With<crate::server_app::LocalShip>>();
            for mut cs in q.iter_mut(app.world_mut()) {
                cs.0.set(
                    crate::ship::system_registry::navigation_system_id(),
                    crate::ship::control_source::ControlSource::Ai,
                );
            }
        }

        push(
            &mut app,
            "navigation",
            ClientMessage::ControlSystem {
                target: crate::messages::SystemId("navigation".into()),
                payload: SystemControlPayload::SetNavigationWaypoint {
                    x: 99.0,
                    z: 99.0,
                    source_uuid: None,
                },
            },
        );
        tick(&mut app);
        assert!(
            get_nav_waypoint(&mut app).is_none(),
            "should reject waypoint when navigation is AI-controlled"
        );
    }

    /// Anchored waypoint set via `ControlSystem` still tracks the entity.
    #[test]
    fn control_system_anchored_waypoint_tracks_entity() {
        let mut app = test_app();
        start_game_with_navigation(&mut app);

        let target_uuid = "anchor-cs-test";
        let target = app
            .world_mut()
            .spawn((
                crate::entity_spawner::EntityUuid(target_uuid.into()),
                Transform::from_xyz(30.0, 0.0, -60.0),
            ))
            .id();

        push(
            &mut app,
            "navigation",
            ClientMessage::ControlSystem {
                target: crate::messages::SystemId("navigation".into()),
                payload: SystemControlPayload::SetNavigationWaypoint {
                    x: 30.0,
                    z: -60.0,
                    source_uuid: Some(target_uuid.into()),
                },
            },
        );
        tick(&mut app);
        assert_eq!(
            get_nav_waypoint(&mut app),
            Some(WaypointMode::Anchored {
                source_uuid: target_uuid.into(),
                last_x: 30.0,
                last_z: -60.0,
            })
        );

        // Move entity — next tick should update last_x/last_z.
        app.world_mut()
            .entity_mut(target)
            .insert(Transform::from_xyz(40.0, 0.0, -70.0));
        tick(&mut app);
        assert_eq!(
            get_nav_waypoint(&mut app),
            Some(WaypointMode::Anchored {
                source_uuid: target_uuid.into(),
                last_x: 40.0,
                last_z: -70.0,
            })
        );
    }

    #[test]
    fn control_system_set_navigation_waypoint_works() {
        let mut app = test_app();
        start_game_with_navigation(&mut app);

        push(
            &mut app,
            "navigation",
            ClientMessage::ControlSystem {
                target: crate::messages::SystemId("navigation".into()),
                payload: SystemControlPayload::SetNavigationWaypoint {
                    x: 15.0,
                    z: 25.0,
                    source_uuid: None,
                },
            },
        );
        tick(&mut app);
        assert_eq!(
            get_nav_waypoint(&mut app),
            Some(WaypointMode::Free { x: 15.0, z: 25.0 })
        );
    }

    #[test]
    fn control_system_clear_navigation_waypoint_works() {
        let mut app = test_app();
        start_game_with_navigation(&mut app);

        push(
            &mut app,
            "navigation",
            ClientMessage::ControlSystem {
                target: crate::messages::SystemId("navigation".into()),
                payload: SystemControlPayload::SetNavigationWaypoint {
                    x: 5.0,
                    z: 5.0,
                    source_uuid: None,
                },
            },
        );
        tick(&mut app);
        push(
            &mut app,
            "navigation",
            ClientMessage::ControlSystem {
                target: crate::messages::SystemId("navigation".into()),
                payload: SystemControlPayload::ClearNavigationWaypoint,
            },
        );
        tick(&mut app);
        assert!(get_nav_waypoint(&mut app).is_none());
    }

    // ── Helpers for operate_navigation_ai integration tests ────────────────

    fn set_navigation_control_source(
        app: &mut App,
        source: crate::ship::control_source::ControlSource,
    ) {
        let mut q = app.world_mut().query_filtered::<&mut crate::ship_plugin::ShipSystemControlSources, With<crate::server_app::LocalShip>>();
        for mut cs in q.iter_mut(app.world_mut()) {
            cs.0.set(crate::system_registry::navigation_system_id(), source);
        }
    }

    fn inject_viewscreen_objective(
        app: &mut App,
        objectives: Vec<crate::messages::ScoredObjective>,
    ) {
        use crate::messages::{SystemBlackboard, ViewscreenBlackboard};
        use crate::server_app::ShipSystemBlackboards;

        let bb = ViewscreenBlackboard {
            scored_objectives: objectives,
            ..Default::default()
        };
        let mut q = app
            .world_mut()
            .query_filtered::<&mut ShipSystemBlackboards, With<crate::server_app::LocalShip>>();
        if let Ok(mut bbs) = q.single_mut(app.world_mut()) {
            bbs.0.insert(
                crate::system_registry::viewscreen_system_id(),
                SystemBlackboard::Viewscreen(bb),
            );
        }
    }

    fn spawn_test_entity(app: &mut App, uuid: &str, x: f32, z: f32) {
        app.world_mut().spawn((
            crate::entity_spawner::EntityUuid(uuid.into()),
            Transform::from_xyz(x, 0.0, z),
        ));
    }

    #[derive(Resource, Default)]
    struct NavCoordCapture(Vec<crate::ship_plugin::CoordinationEnqueue>);

    fn capture_nav_coord(
        mut reader: MessageReader<crate::ship_plugin::CoordinationEnqueue>,
        mut capture: ResMut<NavCoordCapture>,
    ) {
        for ev in reader.read() {
            capture.0.push(ev.clone());
        }
    }

    fn drain_nav_coord(app: &mut App) -> Vec<crate::ship_plugin::CoordinationEnqueue> {
        let msgs = app.world().resource::<NavCoordCapture>().0.clone();
        app.world_mut().resource_mut::<NavCoordCapture>().0.clear();
        msgs
    }

    #[test]
    fn operate_navigation_ai_destroy_sets_anchored_waypoint_and_emits_navigate_to() {
        let mut app = test_app();
        start_game_with_navigation(&mut app);
        set_navigation_control_source(&mut app, crate::ship::control_source::ControlSource::Ai);
        app.init_resource::<NavCoordCapture>()
            .add_systems(PostUpdate, capture_nav_coord);

        // Insert the entity within nav range (default 500).
        spawn_test_entity(&mut app, "target-entity", 400.0, 0.0);

        // Inject Destroy objective with score > 0.
        inject_viewscreen_objective(
            &mut app,
            vec![crate::messages::ScoredObjective {
                id: "destroy-test".into(),
                score: 80.0,
                directive: crate::messages::AiDirective::Destroy {
                    target: "target-entity".into(),
                },
                source: crate::messages::ObjectiveSource::Mission,
                relevance: vec![
                    crate::messages::SystemAffinity::Helm,
                    crate::messages::SystemAffinity::Weapons,
                ],
                snapshot: crate::messages::ObjectiveSnapshot {
                    id: "destroy-test".into(),
                    text: "Destroy target".into(),
                    mandatory: true,
                    status: crate::messages::ObjectiveStatus::Active,
                    targets: vec!["target-entity".into()],
                    source: crate::messages::ObjectiveSource::Mission,
                },
            }],
        );

        tick(&mut app);

        // Check waypoint is set (Anchored).
        let wp = get_nav_waypoint(&mut app);
        assert!(
            matches!(wp, Some(WaypointMode::Anchored { .. })),
            "expected Anchored waypoint, got {:?}",
            wp
        );
        if let Some(WaypointMode::Anchored {
            source_uuid,
            last_x,
            last_z,
        }) = wp
        {
            assert_eq!(source_uuid, "target-entity");
            assert!((last_x - 400.0).abs() < 0.01);
            assert!((last_z - 0.0).abs() < 0.01);
        }

        // Check NavigateTo was emitted.
        let coords = drain_nav_coord(&mut app);
        let nav_to = coords.iter().find(|c| {
            matches!(
                &c.payload,
                crate::messages::CoordinationPayload::NavigateTo { .. }
            )
        });
        assert!(nav_to.is_some(), "expected NavigateTo coordination event");
    }

    #[test]
    fn operate_navigation_ai_reach_sets_free_waypoint_and_emits_navigate_to() {
        let mut app = test_app();
        start_game_with_navigation(&mut app);
        set_navigation_control_source(&mut app, crate::ship::control_source::ControlSource::Ai);
        app.init_resource::<NavCoordCapture>()
            .add_systems(PostUpdate, capture_nav_coord);

        // Insert a WorldConfig with an anchor.
        let mut wc = crate::world::config::WorldConfig::default();
        wc.anchors.insert("base".into(), [300.0, 0.0, -100.0]);
        app.world_mut().insert_resource(wc);

        inject_viewscreen_objective(
            &mut app,
            vec![crate::messages::ScoredObjective {
                id: "reach-test".into(),
                score: 70.0,
                directive: crate::messages::AiDirective::Reach {
                    anchor: "base".into(),
                },
                source: crate::messages::ObjectiveSource::Mission,
                relevance: vec![crate::messages::SystemAffinity::Helm],
                snapshot: crate::messages::ObjectiveSnapshot {
                    id: "reach-test".into(),
                    text: "Reach base".into(),
                    mandatory: true,
                    status: crate::messages::ObjectiveStatus::Active,
                    targets: vec![],
                    source: crate::messages::ObjectiveSource::Mission,
                },
            }],
        );

        tick(&mut app);

        // Check waypoint is Free.
        let wp = get_nav_waypoint(&mut app);
        assert_eq!(
            wp,
            Some(WaypointMode::Free {
                x: 300.0,
                z: -100.0
            })
        );

        // Check NavigateTo was emitted.
        let coords = drain_nav_coord(&mut app);
        let nav_to = coords.iter().find(|c| {
            matches!(
                &c.payload,
                crate::messages::CoordinationPayload::NavigateTo { .. }
            )
        });
        assert!(nav_to.is_some(), "expected NavigateTo coordination event");
        if let Some(crate::messages::CoordinationPayload::NavigateTo { generation, x, z }) =
            nav_to.map(|c| &c.payload)
        {
            // The generation is the navigation contract the Helm latches on; it
            // must be the waypoint's own so the clearance can match.
            assert_eq!(
                *generation,
                nav_waypoint_generation(&mut app),
                "NavigateTo must carry the current waypoint's generation, or the                  Helm's clearance can never match and it will never fly it"
            );
            // `x` / `z` ride for the chatter popup's display only (issue #977) —
            // Rust no longer composes the English "waypoint (x, z)" label; the
            // client's `coordination.navigate.title` template formats them. They
            // are the waypoint's own coordinates.
            assert_eq!((*x, *z), (300.0, -100.0));
        }
    }

    /// The shared issuer sends exactly ONE `NavigateTo` per waypoint
    /// generation while the order stays unlatched — never one per tick. The
    /// old AI path re-enqueued every tick it ran; at a human helm every
    /// delivery is a popup, so a per-tick loop would popup-spam the operator
    /// and flood the coordination queue unboundedly.
    #[test]
    fn navigate_to_clearance_is_issued_once_per_generation_not_per_tick() {
        let mut app = test_app();
        start_game_with_navigation(&mut app);
        set_navigation_control_source(&mut app, crate::ship::control_source::ControlSource::Ai);
        app.init_resource::<NavCoordCapture>()
            .add_systems(PostUpdate, capture_nav_coord);

        // Helm axes stay on their default Human control: the delivered order
        // can never latch, which is exactly the state a per-tick re-issue
        // loop would spam in.
        let mut wc = crate::world::config::WorldConfig::default();
        wc.anchors.insert("base".into(), [300.0, 0.0, -100.0]);
        app.world_mut().insert_resource(wc);
        inject_viewscreen_objective(
            &mut app,
            vec![crate::messages::ScoredObjective {
                id: "reach-test".into(),
                score: 70.0,
                directive: crate::messages::AiDirective::Reach {
                    anchor: "base".into(),
                },
                source: crate::messages::ObjectiveSource::Mission,
                relevance: vec![crate::messages::SystemAffinity::Helm],
                snapshot: crate::messages::ObjectiveSnapshot {
                    id: "reach-test".into(),
                    text: "Reach base".into(),
                    mandatory: true,
                    status: crate::messages::ObjectiveStatus::Active,
                    targets: vec![],
                    source: crate::messages::ObjectiveSource::Mission,
                },
            }],
        );

        // Many ticks: `operate_navigation_ai` re-sets the same waypoint every
        // one of them, and the clearance never latches (human helm).
        let mut navigate_to_count = 0;
        for _ in 0..20 {
            tick(&mut app);
            navigate_to_count += drain_nav_coord(&mut app)
                .iter()
                .filter(|c| {
                    matches!(
                        &c.payload,
                        crate::messages::CoordinationPayload::NavigateTo { .. }
                    )
                })
                .count();
        }
        assert_eq!(
            navigate_to_count, 1,
            "exactly one NavigateTo per waypoint generation — a re-issue loop \
             at a human helm would popup-spam and grow the queue unboundedly"
        );
    }

    /// Same no-spam property on the human write path: one admitted
    /// `SetNavigationWaypoint` while the helm is human-manned produces exactly
    /// one `NavigateTo`, no matter how many ticks pass unlatched.
    #[test]
    fn human_set_waypoint_issues_one_navigate_to_while_helm_stays_human() {
        let mut app = test_app();
        start_game_with_navigation(&mut app);
        app.init_resource::<NavCoordCapture>()
            .add_systems(PostUpdate, capture_nav_coord);

        push(
            &mut app,
            "navigation",
            ClientMessage::ControlSystem {
                target: crate::messages::SystemId("navigation".into()),
                payload: SystemControlPayload::SetNavigationWaypoint {
                    x: 120.0,
                    z: -45.0,
                    source_uuid: None,
                },
            },
        );
        let mut navigate_to_count = 0;
        for _ in 0..20 {
            tick(&mut app);
            navigate_to_count += drain_nav_coord(&mut app)
                .iter()
                .filter(|c| {
                    matches!(
                        &c.payload,
                        crate::messages::CoordinationPayload::NavigateTo { .. }
                    )
                })
                .count();
        }
        assert_eq!(
            navigate_to_count, 1,
            "one admitted waypoint set must issue exactly one NavigateTo"
        );
    }

    /// Rule-6 symmetry: a *human*-set waypoint issues the same Channel-3
    /// `NavigateTo` clearance — carrying the waypoint's generation — that the
    /// AI path issues (mirrors
    /// `operate_navigation_ai_reach_sets_free_waypoint_and_emits_navigate_to`).
    /// Without it, an AI Helm silently never follows a human-set waypoint:
    /// `cleared_nav_waypoint` only releases a generation the clearance has
    /// latched, and only this message ever latches one.
    #[test]
    fn human_set_waypoint_emits_navigate_to_with_current_generation() {
        let mut app = test_app();
        start_game_with_navigation(&mut app);
        app.init_resource::<NavCoordCapture>()
            .add_systems(PostUpdate, capture_nav_coord);

        push(
            &mut app,
            "navigation",
            ClientMessage::ControlSystem {
                target: crate::messages::SystemId("navigation".into()),
                payload: SystemControlPayload::SetNavigationWaypoint {
                    x: 120.0,
                    z: -45.0,
                    source_uuid: None,
                },
            },
        );
        tick(&mut app);

        assert_eq!(
            get_nav_waypoint(&mut app),
            Some(WaypointMode::Free { x: 120.0, z: -45.0 })
        );

        let coords = drain_nav_coord(&mut app);
        let nav_to = coords
            .iter()
            .find(|c| {
                matches!(
                    &c.payload,
                    crate::messages::CoordinationPayload::NavigateTo { .. }
                )
            })
            .expect(
                "a human-set waypoint must enqueue the same NavigateTo clearance the AI path does",
            );
        assert_eq!(nav_to.target, crate::system_registry::helm_station_key());
        assert_eq!(
            nav_to.sender_origin,
            crate::ship::control_source::ControlSource::Human
        );
        let crate::messages::CoordinationPayload::NavigateTo { generation, .. } = &nav_to.payload
        else {
            unreachable!()
        };
        assert_eq!(
            *generation,
            nav_waypoint_generation(&mut app),
            "NavigateTo must carry the current waypoint's generation, or the \
             AI Helm's clearance can never match and it will never fly it"
        );
    }

    #[test]
    fn operate_navigation_ai_patrol_sets_free_waypoint() {
        let mut app = test_app();
        start_game_with_navigation(&mut app);
        set_navigation_control_source(&mut app, crate::ship::control_source::ControlSource::Ai);

        let mut wc = crate::world::config::WorldConfig::default();
        wc.anchors.insert("patrol_pt".into(), [200.0, 0.0, 50.0]);
        app.world_mut().insert_resource(wc);

        inject_viewscreen_objective(
            &mut app,
            vec![crate::messages::ScoredObjective {
                id: "patrol-test".into(),
                score: 60.0,
                directive: crate::messages::AiDirective::Patrol {
                    anchors: vec!["patrol_pt".into()],
                    loop_path: true,
                },
                source: crate::messages::ObjectiveSource::Mission,
                relevance: vec![crate::messages::SystemAffinity::Helm],
                snapshot: crate::messages::ObjectiveSnapshot {
                    id: "patrol-test".into(),
                    text: "Patrol area".into(),
                    mandatory: false,
                    status: crate::messages::ObjectiveStatus::Active,
                    targets: vec![],
                    source: crate::messages::ObjectiveSource::Mission,
                },
            }],
        );

        tick(&mut app);

        let wp = get_nav_waypoint(&mut app);
        assert_eq!(wp, Some(WaypointMode::Free { x: 200.0, z: 50.0 }));
    }

    /// Navigation resolves a Patrol from the objective's **active cursor
    /// target**, not `anchors[0]` (issue #702).
    ///
    /// This system was cursor-blind: it parked the waypoint on the first anchor
    /// of the route and left it there for the whole patrol, so once the ship had
    /// rounded its first waypoint Navigation was still telling the Helm to fly
    /// to a leg it had finished laps ago. The cursor is the objective's own
    /// record of where it is on its route — the same one `helm_patrol` steers
    /// from and `advance_objective_cursors` advances — so reading it is what
    /// makes the two consoles agree.
    ///
    /// The route below needs two distinct anchors to tell the two behaviours
    /// apart: with a one-anchor route (as
    /// `operate_navigation_ai_patrol_sets_free_waypoint` uses) index 0 and the
    /// cursor always agree, and a cursor-blind implementation passes.
    #[test]
    fn operate_navigation_ai_patrol_follows_the_objective_cursor() {
        let mut app = test_app();
        start_game_with_navigation(&mut app);
        set_navigation_control_source(&mut app, crate::ship::control_source::ControlSource::Ai);

        let mut wc = crate::world::config::WorldConfig::default();
        wc.anchors.insert("leg_a".into(), [100.0, 0.0, 0.0]);
        wc.anchors.insert("leg_b".into(), [900.0, 0.0, -400.0]);
        app.world_mut().insert_resource(wc);

        inject_viewscreen_objective(
            &mut app,
            vec![crate::messages::ScoredObjective {
                id: "patrol-test".into(),
                score: 60.0,
                directive: crate::messages::AiDirective::Patrol {
                    anchors: vec!["leg_a".into(), "leg_b".into()],
                    loop_path: true,
                },
                source: crate::messages::ObjectiveSource::Mission,
                relevance: vec![crate::messages::SystemAffinity::Helm],
                snapshot: crate::messages::ObjectiveSnapshot {
                    id: "patrol-test".into(),
                    text: "Patrol area".into(),
                    mandatory: false,
                    status: crate::messages::ObjectiveStatus::Active,
                    targets: vec![],
                    source: crate::messages::ObjectiveSource::Mission,
                },
            }],
        );

        // The ship has already rounded leg_a; its cursor names leg_b.
        let ship = app
            .world_mut()
            .query_filtered::<Entity, With<crate::server_app::LocalShip>>()
            .single(app.world())
            .expect("LocalShip");
        let mut cursor = crate::ai::patrol_cursor::PatrolCursor::new("patrol-test");
        crate::ai::patrol_cursor::advance_cursor(
            &mut cursor,
            &["leg_a".to_string(), "leg_b".to_string()],
            true,
            [100.0, 0.0, 0.0], // sitting on leg_a
            &app.world()
                .resource::<crate::world::config::WorldConfig>()
                .anchors
                .clone(),
            crate::ai::WAYPOINT_ARRIVAL_RADIUS,
        );
        assert_eq!(cursor.index(), 1, "precondition: cursor must name leg_b");
        app.world_mut()
            .entity_mut(ship)
            .insert(crate::ai_plugin::ObjectiveCursors(vec![cursor]));

        tick(&mut app);

        assert_eq!(
            get_nav_waypoint(&mut app),
            Some(WaypointMode::Free {
                x: 900.0,
                z: -400.0
            }),
            "Navigation must place the waypoint on the cursor's current leg (leg_b), \
             not on the route's first anchor (leg_a) — a cursor-blind Navigation \
             keeps ordering the Helm back to a leg it has already flown"
        );
    }

    #[test]
    fn operate_navigation_ai_no_objective_clears_waypoint() {
        let mut app = test_app();
        start_game_with_navigation(&mut app);
        set_navigation_control_source(&mut app, crate::ship::control_source::ControlSource::Ai);

        // First set a waypoint to verify it gets cleared.
        {
            let mut q = app
                .world_mut()
                .query_filtered::<&mut NavigationWaypoint, With<crate::server_app::LocalShip>>();
            if let Ok(mut wp) = q.single_mut(app.world_mut()) {
                wp.set(WaypointMode::Free { x: 500.0, z: 500.0 });
            }
        }
        assert!(
            get_nav_waypoint(&mut app).is_some(),
            "waypoint must be set before clearing test"
        );

        // Inject empty scored_objectives.
        inject_viewscreen_objective(&mut app, vec![]);

        tick(&mut app);

        assert!(
            get_nav_waypoint(&mut app).is_none(),
            "waypoint must be cleared when no objective"
        );
    }

    /// Helper: a single Helm-relevant `Destroy` objective naming `target`.
    fn inject_destroy_objective(app: &mut App, target: &str) {
        inject_viewscreen_objective(
            app,
            vec![crate::messages::ScoredObjective {
                id: "destroy-far".into(),
                score: 80.0,
                directive: crate::messages::AiDirective::Destroy {
                    target: target.into(),
                },
                source: crate::messages::ObjectiveSource::Mission,
                relevance: vec![crate::messages::SystemAffinity::Helm],
                snapshot: crate::messages::ObjectiveSnapshot {
                    id: "destroy-far".into(),
                    text: "Destroy far target".into(),
                    mandatory: true,
                    status: crate::messages::ObjectiveStatus::Active,
                    targets: vec![target.into()],
                    source: crate::messages::ObjectiveSource::Mission,
                },
            }],
        );
    }

    /// Navigation is the ship's chart, not a radar: it plots the whole system.
    /// It used to cull candidates by `nav_chart_range` — a *display* extent read
    /// off the local player's ship config and applied to NPCs — which defeated
    /// the entire point of the Channel-3 handoff, whose job is to steer a
    /// short-ranged Helm toward something it cannot see for itself.
    #[test]
    fn operate_navigation_ai_sets_waypoint_for_distant_target() {
        let mut app = test_app();
        start_game_with_navigation(&mut app);
        set_navigation_control_source(&mut app, crate::ship::control_source::ControlSource::Ai);

        // Far beyond any chart range a hull declares (the largest authored is 800).
        spawn_test_entity(&mut app, "far-entity", 5000.0, 0.0);
        inject_destroy_objective(&mut app, "far-entity");

        tick(&mut app);

        let wp = get_nav_waypoint(&mut app).expect("distant target must still get a waypoint");
        let WaypointMode::Anchored { last_x, last_z, .. } = wp else {
            panic!("a Destroy target anchors to the entity, got {wp:?}");
        };
        assert_eq!(last_x, 5000.0);
        assert_eq!(last_z, 0.0);
    }

    /// `combat_test.toml` authors its assault doctrine as
    /// `directive_target = "Starbase Alpha"` — a name, not a UUID. Matching on
    /// UUID alone left it unresolvable, so Navigation cleared the waypoint and
    /// the raider fell back to patrolling.
    #[test]
    fn operate_navigation_ai_resolves_destroy_target_by_entity_name() {
        let mut app = test_app();
        start_game_with_navigation(&mut app);
        set_navigation_control_source(&mut app, crate::ship::control_source::ControlSource::Ai);

        let uuid = uuid::Uuid::new_v4().to_string();
        app.world_mut().spawn((
            crate::entity_spawner::EntityUuid(uuid.clone()),
            crate::entities::spawner::EntityName("Starbase Alpha".into()),
            Transform::from_xyz(500.0, 0.0, 100.0),
        ));
        inject_destroy_objective(&mut app, "Starbase Alpha");

        tick(&mut app);

        let wp = get_nav_waypoint(&mut app).expect("a name-authored Destroy must resolve");
        let WaypointMode::Anchored {
            source_uuid,
            last_x,
            last_z,
        } = wp
        else {
            panic!("a Destroy target anchors to the entity, got {wp:?}");
        };
        assert_eq!(last_x, 500.0);
        assert_eq!(last_z, 100.0);
        assert_eq!(
            source_uuid, uuid,
            "an Anchored waypoint tracks its parent by UUID, so it must store the \
             resolved UUID rather than the authored name"
        );
    }

    #[test]
    fn operate_navigation_ai_clears_waypoint_for_unknown_destroy_target() {
        let mut app = test_app();
        start_game_with_navigation(&mut app);
        set_navigation_control_source(&mut app, crate::ship::control_source::ControlSource::Ai);

        inject_destroy_objective(&mut app, "no-such-entity");

        tick(&mut app);

        assert!(
            get_nav_waypoint(&mut app).is_none(),
            "a Destroy naming nothing in the world resolves nowhere"
        );
    }

    #[test]
    fn operate_navigation_ai_human_controlled_does_not_set_waypoint() {
        let mut app = test_app();
        start_game_with_navigation(&mut app);
        // Keep Navigation on Human control (default).
        // set_navigation_control_source is NOT called.

        let mut wc = crate::world::config::WorldConfig::default();
        wc.anchors.insert("some_anchor".into(), [100.0, 0.0, 0.0]);
        app.world_mut().insert_resource(wc);

        inject_viewscreen_objective(
            &mut app,
            vec![crate::messages::ScoredObjective {
                id: "reach-human".into(),
                score: 50.0,
                directive: crate::messages::AiDirective::Reach {
                    anchor: "some_anchor".into(),
                },
                source: crate::messages::ObjectiveSource::Mission,
                relevance: vec![crate::messages::SystemAffinity::Helm],
                snapshot: crate::messages::ObjectiveSnapshot {
                    id: "reach-human".into(),
                    text: "Reach".into(),
                    mandatory: false,
                    status: crate::messages::ObjectiveStatus::Active,
                    targets: vec![],
                    source: crate::messages::ObjectiveSource::Mission,
                },
            }],
        );

        tick(&mut app);

        assert!(
            get_nav_waypoint(&mut app).is_none(),
            "human-controlled navigation must not set waypoints"
        );
    }

    /// Verifies operate_navigation_ai runs per-entity for AI-controlled ships (issue #592 AC).
    #[test]
    fn operate_navigation_ai_per_entity_ai_gate() {
        use crate::ship::control_source::{ControlSource, ControlSourceResolver};
        use crate::ship_plugin::ShipSystemControlSources;

        let mut ai_resolver = ControlSourceResolver::new();
        ai_resolver.set(
            crate::system_registry::navigation_system_id(),
            ControlSource::Ai,
        );
        let ai_sources = ShipSystemControlSources(ai_resolver);
        let policy = ai_sources
            .0
            .policy_for(&crate::system_registry::navigation_system_id());
        assert!(
            policy.operate_ai,
            "AI Navigation must gate through operate_ai"
        );

        // Human-controlled navigation must not operate AI.
        let mut human_resolver = ControlSourceResolver::new();
        human_resolver.set(
            crate::system_registry::navigation_system_id(),
            ControlSource::Human,
        );
        let human_sources = ShipSystemControlSources(human_resolver);
        let human_policy = human_sources
            .0
            .policy_for(&crate::system_registry::navigation_system_id());
        assert!(
            !human_policy.operate_ai,
            "Human Navigation must not operate AI"
        );
    }

    // ── #778 selector: replacement, clearing, chart-contact source ─────────

    /// AC6 (replacement): swapping the active objective's destination makes the
    /// selector pick the new one, and the published waypoint is replaced — the
    /// same observable set-then-set path a human console drives.
    #[test]
    fn operate_navigation_ai_replaces_waypoint_when_objective_changes() {
        let mut app = test_app();
        start_game_with_navigation(&mut app);
        set_navigation_control_source(&mut app, crate::ship::control_source::ControlSource::Ai);

        let mut wc = crate::world::config::WorldConfig::default();
        wc.anchors.insert("first".into(), [100.0, 0.0, 0.0]);
        wc.anchors.insert("second".into(), [-250.0, 0.0, 80.0]);
        app.world_mut().insert_resource(wc);

        let reach = |anchor: &str| {
            vec![crate::messages::ScoredObjective {
                id: "reach".into(),
                score: 70.0,
                directive: crate::messages::AiDirective::Reach {
                    anchor: anchor.into(),
                },
                source: crate::messages::ObjectiveSource::Mission,
                relevance: vec![crate::messages::SystemAffinity::Helm],
                snapshot: crate::messages::ObjectiveSnapshot {
                    id: "reach".into(),
                    text: "Reach".into(),
                    mandatory: true,
                    status: crate::messages::ObjectiveStatus::Active,
                    targets: vec![],
                    source: crate::messages::ObjectiveSource::Mission,
                },
            }]
        };

        inject_viewscreen_objective(&mut app, reach("first"));
        tick(&mut app);
        assert_eq!(
            get_nav_waypoint(&mut app),
            Some(WaypointMode::Free { x: 100.0, z: 0.0 })
        );

        inject_viewscreen_objective(&mut app, reach("second"));
        tick(&mut app);
        assert_eq!(
            get_nav_waypoint(&mut app),
            Some(WaypointMode::Free { x: -250.0, z: 80.0 }),
            "the selector must replace the waypoint when the objective destination changes"
        );
    }

    /// AC2 / AC6: the `chart-contacts` source is genuinely wired — an authored
    /// selector that admits chart contacts (the canonical default keys
    /// eligibility on `reachable`, which they do not carry) selects a live
    /// entity as an anchored destination with NO objective present at all,
    /// exercising the "live entity-anchored destination" path through the
    /// reusable selector.
    #[test]
    fn operate_navigation_ai_selects_chart_contact_when_author_widens_eligibility() {
        let mut app = test_app();
        start_game_with_navigation(&mut app);
        set_navigation_control_source(&mut app, crate::ship::control_source::ControlSource::Ai);

        let cfg = crate::entities::config::FineSystemAiSelectorToml {
            param: Default::default(),
            sources: vec!["navigation-objectives".into(), "chart-contacts".into()],
            horizon: 1.0e9,
            switch_margin: 0.0,
            eligibility: "candidate_fact(source_chart_contact) > 0".into(),
            score: vec![crate::entities::config::ScoreTermToml {
                when: "candidate_fact(source_chart_contact) > 0".into(),
                weight: 1.0,
            }],
        };
        let selector = cfg.to_selector().expect("authored selector resolves");
        let ship = app
            .world_mut()
            .query_filtered::<Entity, With<crate::server_app::LocalShip>>()
            .single(app.world())
            .expect("LocalShip");
        app.world_mut()
            .entity_mut(ship)
            .insert(NavigationTargetSelector {
                selector,
                power_rating: None,
            });

        // No objective — only a live chart contact.
        inject_viewscreen_objective(&mut app, vec![]);
        spawn_test_entity(&mut app, "contact-1", 300.0, -120.0);

        tick(&mut app);

        assert_eq!(
            get_nav_waypoint(&mut app),
            Some(WaypointMode::Anchored {
                source_uuid: "contact-1".into(),
                last_x: 300.0,
                last_z: -120.0,
            }),
            "a widened selector must select a chart contact as an anchored destination"
        );
    }

    /// Issue #891 stage 2, per-host both-directions proof for the Navigation
    /// target selector: an authored eligibility gated on a world flag selects
    /// no destination while the flag is clear and anchors the chart contact
    /// once it is set.
    #[test]
    fn operate_navigation_ai_flag_guard_reads_the_world_in_both_directions() {
        let mut app = test_app();
        start_game_with_navigation(&mut app);
        set_navigation_control_source(&mut app, crate::ship::control_source::ControlSource::Ai);
        app.init_resource::<crate::world::server::WorldContentRuntime>();

        let cfg = crate::entities::config::FineSystemAiSelectorToml {
            param: Default::default(),
            sources: vec!["chart-contacts".into()],
            horizon: 1.0e9,
            switch_margin: 0.0,
            eligibility: "candidate_fact(source_chart_contact) > 0 and flag(survey_authorised)"
                .into(),
            score: vec![crate::entities::config::ScoreTermToml {
                when: "candidate_fact(source_chart_contact) > 0".into(),
                weight: 1.0,
            }],
        };
        let selector = cfg.to_selector().expect("flag-gated selector resolves");
        let ship = app
            .world_mut()
            .query_filtered::<Entity, With<crate::server_app::LocalShip>>()
            .single(app.world())
            .expect("LocalShip");
        app.world_mut()
            .entity_mut(ship)
            .insert(NavigationTargetSelector {
                selector,
                power_rating: None,
            });
        inject_viewscreen_objective(&mut app, vec![]);
        spawn_test_entity(&mut app, "contact-891", 300.0, -120.0);

        // Flag CLEAR → nothing is eligible, no waypoint.
        tick(&mut app);
        assert_eq!(
            get_nav_waypoint(&mut app),
            None,
            "with the world flag clear the eligibility must admit no destination"
        );

        // Flag SET → the SAME eligibility anchors the chart contact.
        app.world_mut()
            .resource_mut::<crate::world::server::WorldContentRuntime>()
            .flags
            .set_flag("survey_authorised");
        tick(&mut app);
        assert_eq!(
            get_nav_waypoint(&mut app),
            Some(WaypointMode::Anchored {
                source_uuid: "contact-891".into(),
                last_x: 300.0,
                last_z: -120.0,
            }),
            "with the world flag set the same eligibility must select the contact"
        );
    }

    /// AC6 (lifecycle reset): a chart contact selected as a destination is
    /// auto-cleared when its entity despawns — AI-authored anchored waypoints get
    /// the same despawn-clear semantics as human-authored ones (AC4), because the
    /// host keeps emitting the same admitted `SetNavigationWaypoint` and
    /// `refresh_anchored_waypoint` is origin-blind.
    #[test]
    fn operate_navigation_ai_chart_contact_waypoint_auto_clears_on_despawn() {
        let mut app = test_app();
        start_game_with_navigation(&mut app);
        set_navigation_control_source(&mut app, crate::ship::control_source::ControlSource::Ai);

        let cfg = crate::entities::config::FineSystemAiSelectorToml {
            param: Default::default(),
            sources: vec!["chart-contacts".into()],
            horizon: 1.0e9,
            switch_margin: 0.0,
            eligibility: "candidate_fact(source_chart_contact) > 0".into(),
            score: vec![crate::entities::config::ScoreTermToml {
                when: "candidate_fact(source_chart_contact) > 0".into(),
                weight: 1.0,
            }],
        };
        let selector = cfg.to_selector().expect("authored selector resolves");
        let ship = app
            .world_mut()
            .query_filtered::<Entity, With<crate::server_app::LocalShip>>()
            .single(app.world())
            .expect("LocalShip");
        app.world_mut()
            .entity_mut(ship)
            .insert(NavigationTargetSelector {
                selector,
                power_rating: None,
            });

        inject_viewscreen_objective(&mut app, vec![]);
        let contact = app
            .world_mut()
            .spawn((
                crate::entity_spawner::EntityUuid("contact-despawn".into()),
                Transform::from_xyz(150.0, 0.0, 40.0),
            ))
            .id();

        tick(&mut app);
        assert!(
            matches!(
                get_nav_waypoint(&mut app),
                Some(WaypointMode::Anchored { .. })
            ),
            "chart contact must be selected before the despawn"
        );

        app.world_mut().entity_mut(contact).despawn();
        tick(&mut app);
        assert!(
            get_nav_waypoint(&mut app).is_none(),
            "an AI-anchored waypoint must auto-clear when its entity despawns, like a human one"
        );
    }

    // ── Host teleport-to-waypoint override (issue #770) ────────────────────
    //
    // These exercise the host-only debug override against a real LocalShip
    // entity: the query shape mirrors `drain_teleport_to_waypoint` (which is
    // wasm-gated and so cannot run under native `cargo test`), and the move
    // itself goes through the pure, testable `apply_teleport_to_waypoint`.

    /// The teleport override snaps the LocalShip's authoritative planar position
    /// onto the shared waypoint while leaving its altitude unchanged, and the
    /// existence predicate the disable-gate reads is `Some` while a waypoint is
    /// set.
    #[test]
    fn host_teleport_moves_local_ship_to_waypoint() {
        use crate::server::bridge::apply_teleport_to_waypoint;

        let mut world = World::new();
        let ship = world
            .spawn((
                crate::server_app::LocalShip,
                crate::ship_state::ShipPhysics {
                    x: 0.0,
                    y: 5.0,
                    z: 0.0,
                    ..Default::default()
                },
                NavigationWaypoint::new(WaypointMode::Free { x: 111.0, z: 222.0 }),
            ))
            .id();

        // Existence predicate (AC2) — a waypoint is set.
        assert!(world
            .get::<NavigationWaypoint>(ship)
            .unwrap()
            .mode()
            .is_some());

        // Mirror `drain_teleport_to_waypoint`'s query + apply.
        let mut q = world.query_filtered::<(
            &mut crate::ship_state::ShipPhysics,
            &NavigationWaypoint,
        ), With<crate::server_app::LocalShip>>();
        for (mut physics, waypoint) in q.iter_mut(&mut world) {
            assert!(apply_teleport_to_waypoint(&mut physics, waypoint));
        }

        let physics = world.get::<crate::ship_state::ShipPhysics>(ship).unwrap();
        assert_eq!(physics.x, 111.0);
        assert_eq!(physics.z, 222.0);
        assert_eq!(physics.y, 5.0, "altitude must be preserved");
    }

    /// With no waypoint the existence predicate reads `None` (the panel disables
    /// the control) and the teleport apply is a no-op.
    #[test]
    fn host_teleport_disabled_and_noop_without_waypoint() {
        use crate::server::bridge::apply_teleport_to_waypoint;

        let mut world = World::new();
        let ship = world
            .spawn((
                crate::server_app::LocalShip,
                crate::ship_state::ShipPhysics {
                    x: 3.0,
                    y: 1.0,
                    z: 4.0,
                    ..Default::default()
                },
                NavigationWaypoint::default(),
            ))
            .id();

        // Existence predicate (AC2) — no waypoint, so the control is disabled.
        assert!(world
            .get::<NavigationWaypoint>(ship)
            .unwrap()
            .mode()
            .is_none());

        let mut q = world.query_filtered::<(
            &mut crate::ship_state::ShipPhysics,
            &NavigationWaypoint,
        ), With<crate::server_app::LocalShip>>();
        for (mut physics, waypoint) in q.iter_mut(&mut world) {
            assert!(!apply_teleport_to_waypoint(&mut physics, waypoint));
        }

        let physics = world.get::<crate::ship_state::ShipPhysics>(ship).unwrap();
        assert_eq!((physics.x, physics.y, physics.z), (3.0, 1.0, 4.0));
    }
}
