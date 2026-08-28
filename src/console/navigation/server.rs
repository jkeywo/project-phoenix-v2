use bevy::prelude::*;

use crate::command_admission::ai_emit::emit_ai_command;
use crate::core::messages::{
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
        app.register_admitted_consumer(ConsumerMatcher::exact(
            crate::ship::system_registry::NAVIGATION_KIND,
            NAVIGATION_SYSTEM_ID,
        ));
        // The admitted-waypoint applier moves Input→Physics (issue #830):
        // `operate_navigation_ai` emits its `SetNavigationWaypoint` into
        // `AdmittedCommands` in Physics, and admission clears `AdmittedCommands`
        // once per tick *before* Input, so the applier must run after the AI
        // emit in the same set for a same-tick AI waypoint to land. Human
        // commands admitted before Input survive to Physics unchanged.
        app.add_systems(
            FixedUpdate,
            // Issue #1141: the Backfill Navigation traffic-order host emits in
            // Input, before the existing origin-blind civilian consumer, so its
            // `OrderCivilian` lands on exactly the same tick as a console press.
            operate_civilian_order_ai
                .in_set(crate::sim_sets::SimSet::Input)
                .run_if(crate::ai::cadence::ai_tick_ready)
                .before(crate::civilian::tick_civilian_traffic),
        )
        .add_systems(
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

/// Backfill Navigation civilian-order host (issue #1141).
///
/// Reads positive `Order { target, route }` objectives in the same deterministic
/// scored order every other objective host uses. The first target not already
/// carrying that authoritative order is sent an `OrderCivilian` payload through
/// [`emit_ai_command`]. `tick_civilian_traffic`, ordered immediately after this
/// host in `SimSet::Input`, consumes it through the same path as a human console
/// press; nothing downstream can tell which actor supplied it.
///
/// Observing `CivilianState::order()` makes emission idempotent. Once one craft
/// has received its order the next AI cadence chooses the next objective, so a
/// scenario can publish one payload-bearing objective per endangered craft
/// without resetting a civilian's acknowledgement clock every tick.
#[allow(clippy::type_complexity)]
pub fn operate_civilian_order_ai(
    sessions: Res<crate::lobby::Sessions>,
    runtime: Option<Res<crate::world::server::WorldContentRuntime>>,
    world_config: Option<Res<crate::world::config::WorldConfig>>,
    civilians: Query<(
        &crate::entities::spawner::EntityUuid,
        &crate::civilian::CivilianTraffic,
    )>,
    mut ships: Query<
        (
            Option<&crate::entities::spawner::EntityUuid>,
            &crate::ship_plugin::ShipSystemControlSources,
            &crate::server_app::ShipSystemBlackboards,
            Option<&crate::ship_plugin::ShipConfigComponent>,
            &mut crate::core::messages::AdmittedCommands,
        ),
        With<crate::server_app::LocalShip>,
    >,
) {
    for (entity_uuid, sources, blackboards, ship_config, mut admitted) in ships.iter_mut() {
        if !crate::ai::host::ai_operates(
            &sources.0,
            crate::ship::system_registry::navigation_system_id(),
        ) {
            continue;
        }
        let Some(crate::core::messages::SystemBlackboard::Viewscreen(view)) = blackboards
            .0
            .get(&crate::ship::system_registry::viewscreen_system_id())
        else {
            continue;
        };

        let next = view.scored_objectives.iter().find_map(|objective| {
            if objective.score <= 0.0
                || !objective
                    .relevance
                    .contains(&crate::core::messages::SystemAffinity::Navigation)
            {
                return None;
            }
            let (target, route) = crate::objectives::order_directive(&objective.directive)?;
            if target.is_empty()
                || route.is_empty()
                || world_config
                    .as_deref()
                    .is_none_or(|world| world.route(route).is_none())
            {
                return None;
            }
            let resolved = runtime
                .as_deref()
                .and_then(|world| world.name_to_uuid.get(target))
                .map(String::as_str)
                .unwrap_or(target);
            let traffic = civilians
                .iter()
                .find(|(uuid, _)| uuid.0 == resolved)
                .map(|(_, traffic)| traffic)?;
            let order = crate::civilian::CivilianOrder::divert_to_route(route);
            (traffic.0.order() != Some(&order)).then(|| (target.to_string(), order))
        });

        if let Some((target, order)) = next {
            emit_ai_command(
                entity_uuid,
                crate::ship::system_registry::navigation_system_id(),
                crate::core::messages::SystemControlPayload::OrderCivilian { target, order },
                sources,
                &sessions,
                ship_config,
                &mut admitted,
            );
        }
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
    /// position, the generic lag router delivers it when due, and Helm's
    /// `receive_helm_coordination` latches it into `HelmWaypointClearance`.
    /// The AI Helm follows the waypoint only while `clearance == generation`.
    /// Every *new*
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

    /// Replace the exact continuation carried by a world snapshot.
    ///
    /// This deliberately bypasses [`set`](Self::set): replaying the public
    /// mutation path would bump `generation` and turn a restore into a new
    /// waypoint command rather than reinstating the captured command frontier.
    pub(crate) fn replace_continuation(
        &mut self,
        mode: Option<WaypointMode>,
        generation: u64,
    ) {
        self.mode = mode;
        self.generation = generation;
    }
}

impl NavClearanceIssueState {
    /// Scalar continuation read by the snapshot boundary.
    pub(crate) fn continuation(&self) -> (Option<u64>, bool) {
        (self.issued_generation, self.helm_axes_were_ai)
    }

    /// Replace the issuer's exact debounce/edge frontier on restore.
    pub(crate) fn replace_continuation(
        &mut self,
        issued_generation: Option<u64>,
        helm_axes_were_ai: bool,
    ) {
        self.issued_generation = issued_generation;
        self.helm_axes_were_ai = helm_axes_were_ai;
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
            &crate::ship_plugin::ShipConfigComponent,
            Option<&crate::ship_plugin::HelmWaypointClearance>,
        ),
        With<crate::server_app::Ship>,
    >,
    mut coordination_writer: MessageWriter<crate::ship_plugin::CoordinationEnqueue>,
) {
    for (entity, waypoint, mut state, control_sources, ship_config, clearance) in ships.iter_mut() {
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
            let Some(address) = crate::ship::coordination::address_for_system(
                &ship_config.0,
                &crate::ship::system_registry::helm_steering_system_id(),
            ) else {
                continue;
            };
            let presentation = crate::core::messages::CoordinationPresentation::titled(
                "coordination.navigate.title",
            )
            .with_title_param("x", coordination_display_integer(snapshot.x))
            .with_title_param("z", coordination_display_integer(snapshot.z));
            coordination_writer.write(crate::ship_plugin::CoordinationEnqueue {
                source_entity: entity,
                // The origin is the navigation system's resolved control
                // source — derived from target state like every other
                // post-admission enqueuer, never from the wire path.
                sender_origin: control_sources
                    .0
                    .source_for(&crate::ship::system_registry::navigation_system_id()),
                address,
                payload: crate::core::messages::CoordinationPayload::NavigateTo {
                    generation,
                    // Coords for the chatter popup's display only (issue #977);
                    // the Helm latches on `generation` and reads the waypoint
                    // itself, so nothing steers off these.
                    x: snapshot.x,
                    z: snapshot.z,
                },
                presentation,
                sender_label: crate::ship::coordination::CHATTER_SENDER_NAVIGATION.to_string(),
                sender_system: crate::ship::system_registry::navigation_system_id(),
            });
            state.issued_generation = Some(generation);
        }
    }
}

/// Match JavaScript `Math.round`, which owned waypoint display rounding before
/// Coordination presentation moved to the producer. In particular, negative
/// half values round toward positive infinity rather than away from zero.
fn coordination_display_integer(value: f32) -> i64 {
    (f64::from(value) + 0.5).floor() as i64
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
    entity_q: Query<(&crate::entities::spawner::EntityUuid, &Transform)>,
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
        &crate::entities::spawner::EntityUuid,
        Option<&crate::entities::spawner::EntityName>,
        &crate::civilian::CivilianSection,
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
        &crate::entities::spawner::EntityUuid,
        Option<&crate::entities::spawner::EntityName>,
        &crate::civilian::CivilianSection,
        &crate::civilian::CivilianTraffic,
    )>,
    world_config: Option<&crate::world::config::WorldConfig>,
) -> Vec<crate::core::messages::CivilianTrafficSnapshot> {
    use crate::civilian::CivilianOrder;
    let mut rows: Vec<crate::core::messages::CivilianTrafficSnapshot> = civilians
        .iter()
        .map(|(uuid, name, section, traffic)| {
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
            crate::core::messages::CivilianTrafficSnapshot {
                uuid: uuid.0.clone(),
                name: name.map(|n| n.0.clone()).unwrap_or_default(),
                route,
                leg: state.leg() as u32,
                legs,
                order,
                order_destination: destination,
                compliance: state.compliance().as_str().to_string(),
                reason: state.reason().unwrap_or_default().to_string(),
                order_options: section
                    .0
                    .order_options
                    .iter()
                    .map(
                        |option| crate::core::messages::CivilianOrderOptionSnapshot {
                            id: option.id.clone(),
                            label: option.label.clone(),
                            order: option.order.clone(),
                        },
                    )
                    .collect(),
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
        &crate::ship::state::ShipPhysics,
        Option<&crate::entities::spawner::EntityUuid>,
        Option<&crate::ai::server::ObjectiveCursors>,
        Option<&crate::ship_plugin::ShipConfigComponent>,
        Option<&NavigationTargetSelector>,
        &mut crate::core::messages::AdmittedCommands,
    )>,
    entities: Query<(
        &crate::entities::spawner::EntityUuid,
        &Transform,
        Option<&crate::entities::spawner::EntityName>,
    )>,
    // The read-only AI-host world context — flag chain, sessions, and origin
    // stamps — behind one bare-`Res` system param (issue #1207). A fixture that
    // runs this host must register it (`register_ai_host_env`) or fail loudly at
    // schedule build, so a bare `App` cannot silently diverge from production.
    ai_env: crate::ai::host::AiHostEnv,
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
        // Control-Source gate through the shared AI host spine (issue #1208): a
        // human holder (or an offline system) stands the selector down.
        // Navigation resolves a data-driven SELECTOR the spine does not model, so
        // only its gate — the one step it shares with the policy hosts — routes
        // here.
        if !crate::ai::host::ai_operates(
            &sources.0,
            crate::ship::system_registry::navigation_system_id(),
        ) {
            continue;
        }
        // No authored `[navigation_console.selector]` ⇒ no component ⇒ no
        // destination ranking. Since #885b stage 5d there is no synthesised
        // stand-in.
        let Some(selector_comp) = target_selector else {
            continue;
        };

        let scored: Vec<crate::core::messages::ScoredObjective> = match blackboards
            .0
            .get(&crate::ship::system_registry::viewscreen_system_id())
        {
            Some(crate::core::messages::SystemBlackboard::Viewscreen(bb)) => {
                bb.scored_objectives.clone()
            }
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
                o.score > 0.0
                    && o.relevance
                        .contains(&crate::core::messages::SystemAffinity::Helm)
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
                crate::core::messages::AiDirective::Destroy { target }
                | crate::core::messages::AiDirective::Dock { target }
                    if !target.is_empty() =>
                {
                    if let Some((uuid, pos)) = resolve_destroy_target(
                        &all_entities,
                        Some(ai_env.content_runtime()),
                        target,
                    ) {
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
                crate::core::messages::AiDirective::Reach { anchor }
                | crate::core::messages::AiDirective::Retreat { anchor }
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
                crate::core::messages::AiDirective::Patrol { anchors, loop_path } => {
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
            self_facts.set_fact(crate::entities::ai_flag_hosts::POWER_RATING, pr as f64);
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
        let flag_chain = ai_env.flag_chain(ship_entity);
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

        emit_ai_command(
            entity_uuid,
            crate::ship::system_registry::navigation_system_id(),
            payload,
            sources,
            &sessions,
            ship_config,
            &mut admitted,
        );
    }
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
    use crate::entities::ai_flag_hosts as fid;
    let mut facts = crate::world::flags::AiFacts::new();
    facts.set_fact(fid::REACHABLE, 1.0);
    facts.set_fact(fid::SOURCE_NAV_OBJECTIVE, 1.0);
    facts.set_fact(fid::OBJECTIVE_SCORE, objective_score as f64);
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
    facts.set_fact(crate::entities::ai_flag_hosts::SOURCE_CHART_CONTACT, 1.0);
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

/// Apply a pending host teleport-to-waypoint to one ship's physics (issue #770;
/// relocated here from `crate::server::bridge` in issue #1194).
///
/// A deliberate host-only simulation override: reads the ship's
/// [`NavigationWaypoint`] snapshot (which resolves BOTH Free and Anchored modes
/// to a live x/z) and, when a waypoint exists, snaps the ship's planar position
/// to it — a discontinuous jump, unlike the helm's velocity integration, and
/// deliberately NOT routed through command admission. Returns `true` when a
/// teleport happened, `false` when there was no waypoint (a no-op).
///
/// `physics.y` (altitude) is left UNCHANGED on purpose: `WaypointMode` is
/// X/Z-only and carries no altitude, so keeping the ship's current height is the
/// least-surprising behaviour — the ship slides across to the waypoint without
/// changing altitude (issue #768 allows ships to sit at nonzero Y).
///
/// Pure and Bevy-`World`-free, so it stays unit-testable under plain `cargo
/// test`; the wasm edge (`drain_teleport_to_waypoint` in `crate::server::bridge`)
/// calls into it.
pub fn apply_teleport_to_waypoint(
    physics: &mut crate::ship::state::ShipPhysics,
    waypoint: &NavigationWaypoint,
) -> bool {
    match waypoint.snapshot() {
        Some(snapshot) => {
            physics.x = snapshot.x;
            physics.z = snapshot.z;
            // physics.y deliberately unchanged — see the doc comment.
            true
        }
        None => false,
    }
}

#[cfg(test)]
// Fixture ids only (issue #907): a test that needs "some distinct id" has no
// run to reproduce. Production identity is minted by `crate::world_id`, and
// clippy.toml bans `Uuid::new_v4` outside scopes like this one.
#[allow(clippy::disallowed_methods)]
#[path = "server_tests.rs"]
mod tests;
