use bevy::prelude::*;

use crate::console_bridge::AiChatterEvent;
use crate::core::messages::{ClientMessage, CoordinationPayload};
use crate::lobby::{InboundMessage, Sessions};
use crate::server_app::LocalShip;
use crate::ship::components::{
    ActiveStationRatings, CoordinationEnqueue, CoordinationQueue, HelmWaypointClearance,
    HumanSeekingHosts, PendingArcBearingRequest, PendingTacticalFrequencyHint, RepairHumanAlerted,
    ScenarioDetailFloor, ShipConfigComponent, ShipSystemControlSources, VisitingStationHosts,
};
use crate::ship::control_source::ControlSource;
use crate::ship::coordination;
use crate::ship::coordination::QueuedCoordination;
use crate::ship::damage::DamageTier;
use crate::ship::helm_ai::helm_axes_operate_ai;

pub fn handle_coordination_enqueue(
    mut ship_components: Query<
        (Entity, &ShipConfigComponent, &mut CoordinationQueue),
        With<crate::server_app::Ship>,
    >,
    local_ship_q: Query<Entity, With<LocalShip>>,
    mut events: MessageReader<CoordinationEnqueue>,
    mut inbound: MessageReader<InboundMessage>,
    sessions: Res<Sessions>,
    time: Res<Time>,
) {
    let now = time.elapsed_secs();
    let coord_events: Vec<_> = events.read().cloned().collect();
    let inbound_msgs: Vec<_> = inbound.read().cloned().collect();

    // Route typed CoordinationEnqueue events to their source ship's queue.
    for ev in &coord_events {
        let Ok((_e, ship_config, mut queue)) = ship_components.get_mut(ev.source_entity) else {
            // Source ship despawned or lacks a CoordinationQueue — silently drop.
            continue;
        };
        let lag = ship_config.0.coordination_lag_secs;
        // Channel-3 addresses crew by STATION, not by the fine system that
        // spoke (Task 2). Resolve the emitting system to its owning station's
        // display id here, where the SOURCE ship's config is in scope; an empty
        // `sender_system` opts out and keeps the pre-resolved `sender_label`
        // (the intent-narration path already stamps its own station id).
        let sender_label = if ev.sender_system.0.is_empty() {
            ev.sender_label.clone()
        } else {
            coordination::station_addressee_label(&ship_config.0, &ev.sender_system)
        };
        queue.0.enqueue(QueuedCoordination {
            sender_origin: ev.sender_origin,
            target: ev.target.clone(),
            payload: ev.payload.clone(),
            sender_label,
            due_time: now + lag,
        });
    }

    // Route human `SendCoordination` messages to the LocalShip only.
    // `SendCoordination` is always a ClientMessage from a human, always
    // scoped to that human's own ship.
    let Some(local_entity) = local_ship_q.iter().next() else {
        return;
    };
    let Ok((_e, ship_config, mut queue)) = ship_components.get_mut(local_entity) else {
        return;
    };
    let lag = ship_config.0.coordination_lag_secs;
    for msg in &inbound_msgs {
        let ClientMessage::SendCoordination { target, payload } = &msg.msg else {
            continue;
        };
        let player = match sessions.0.players().iter().find(|p| p.token == msg.token) {
            Some(p) => p,
            None => continue,
        };
        // A Spectator (issue #1105) issues no simulation-influencing traffic.
        // Channel-3 coordination shapes crew/AI behaviour, so it is a
        // simulation command in the AC3 sense — drop a spectator's here rather
        // than enqueue it. (A spectator holds no station, but the explicit role
        // check is the robust gate, mirroring command admission.)
        if player.spectator {
            continue;
        }
        let sender_origin = if player.station.is_none() {
            ControlSource::Ai
        } else {
            ControlSource::Human
        };
        queue.0.enqueue(QueuedCoordination {
            sender_origin,
            target: target.clone(),
            payload: payload.clone(),
            sender_label: player.name.clone(),
            due_time: now + lag,
        });
    }
}

/// Strip exact damage numbers from an outbound `CoordinationPopup` payload that
/// `viewer` is not entitled to (issue #737).
///
/// `CoordinationPayload::RepairRequest` is the only coordination payload that
/// carries a hull number. It is targeted at the `repair` system, which resolves
/// to the Engineering station, so before this gate existed every worsening tier
/// crossing handed Engineering the exact HP deficit of an arbitrary non-Core
/// system with no team dispatched and no travel elapsed — the exact thing the
/// hull projection withholds, arriving through the other door.
///
/// The gate is [`HullVisibility::can_see`], the same predicate the hull rows and
/// the repair blackboard use, resolved through the same
/// `RepairTeams::on_site_systems()` on-site set. Core systems and the viewer's
/// own systems keep exact detail; a non-Core system keeps it only while a team
/// is on site. Otherwise the tier still crosses and the popup still fires — the
/// number is simply absent, which is the "needs attention" signal.
///
/// Withholding is the default: with no visibility (a ship carrying no hull
/// component) the deficit is dropped.
fn coarsen_repair_request(
    payload: &CoordinationPayload,
    vis: Option<&crate::console::repair::visibility::HullVisibility>,
    viewer: Option<&crate::core::messages::StationId>,
) -> CoordinationPayload {
    let CoordinationPayload::RepairRequest {
        system_id,
        station_id,
        station_label,
        tier,
        deficit,
    } = payload
    else {
        return payload.clone();
    };
    let entitled = vis.map(|v| v.can_see(viewer, system_id)).unwrap_or(false);
    CoordinationPayload::RepairRequest {
        system_id: system_id.clone(),
        station_id: station_id.clone(),
        station_label: station_label.clone(),
        tier: *tier,
        deficit: if entitled { *deficit } else { None },
    }
}

/// Build the seat list the pure ship-broadcast router works from (issue #879).
///
/// One entry per authored station, **in authored order** — the deterministic
/// ordering the router's contract depends on; iterating a map here would make
/// the popup sequence depend on hash seeding, which two lockstep peers cannot
/// agree on. Each station's fine systems are resolved through `policy_for`
/// (which honours damage-offline) and reduced by
/// [`coordination::seat_control_source`].
fn ship_seats(
    config: &crate::ship::config::ShipConfig,
    control_sources: &ShipSystemControlSources,
    sessions: &Sessions,
) -> Vec<coordination::ShipSeat> {
    config
        .stations
        .iter()
        .map(|station| {
            let policies: Vec<crate::ship::control_source::ControlTickPolicy> = config
                .systems
                .iter()
                .filter(|s| s.station.as_ref() == Some(&station.id))
                .map(|s| control_sources.0.policy_for(&s.id))
                .collect();
            coordination::ShipSeat {
                station: station.id.clone(),
                control: coordination::seat_control_source(&policies),
                holder: sessions
                    .0
                    .holder_for_station(&station.id)
                    .map(|t| t.to_string()),
            }
        })
        .collect()
}

// ── Human-seeking hosts (issue #984) ──────────────────────────────────────────

/// Resolve the active world's hull-agnostic detail-floor vocabulary to the
/// selected LocalShip's concrete System ids. This is the production writer of
/// [`ScenarioDetailFloor`]; the Station resolver below remains a pure consumer.
/// Re-running is deliberate: a scenario resource or selected hull may change
/// between missions without leaving the prior world's floor latched.
pub fn write_scenario_detail_floor(
    world: Option<Res<crate::world::config::WorldConfig>>,
    mut ships: Query<(&ShipConfigComponent, &mut ScenarioDetailFloor), With<LocalShip>>,
) {
    let selectors = world
        .as_deref()
        .map(|world| world.scenario_detail_floor.as_slice())
        .unwrap_or_default();
    for (config, mut floor) in ships.iter_mut() {
        let resolved = coordination::resolve_scenario_detail_floor(&config.0, selectors);
        if floor.0 != resolved {
            floor.0 = resolved;
        }
    }
}

/// Re-resolve, EVERY tick, which station is hosting each `human_seeking`
/// `[[system]]` on the player's ship, and put that system's `ControlSource`
/// where the answer says it belongs.
///
/// Comms and Navigation "always try to be under human control" (pasm decision
/// `console-complexity-human-seeking-systems`). The decision itself is the pure
/// [`coordination::seek_human_host`] over the pure
/// [`coordination::seeking_seats`]; this adapter only supplies the ship's
/// authored config, its live control sources, its active ratings and the lobby
/// session map, then writes the two things the rest of the host reads:
///
/// * `ControlSource::Human` on a hit, `ControlSource::Ai` on a miss, for the
///   seeking system ALONE. This is not cosmetic:
///   `command_admission::is_command_authorized` tests `accept_human_input`
///   BEFORE station tenure, so a sought human whose comms system was still
///   `Ai` (because its owning station is backfilled) would be refused at the
///   gate with no other symptom. No other system's source is touched — a
///   station's rating is the player's to set, and inflating the systems around
///   the sought one would silently promote a Backfill console.
/// * [`HumanSeekingHosts`], the system→station map
///   `command_admission::station_for_system` consults, so tenure is checked
///   against the seat the seek actually chose.
/// * [`VisitingStationHosts`], including each complete visiting Station's
///   observable effective rating after the live [`ScenarioDetailFloor`] has
///   been composed with its authored visiting baseline.
///
/// **Every tick, idempotently, never on-change.** `apply_rating` fires on lobby
/// events (a claim, a disconnect, a `SetStationRating`) and rewrites every
/// system its station owns — including a sought one. A resolver that only ran
/// on a change of its own inputs would be silently undone by the next rating
/// event. Writing unconditionally but only *dereferencing* when a value
/// actually differs keeps that re-assertion free of spurious change-detection
/// ticks.
///
/// Scoped to the `LocalShip`, which is where `ship_seats` above is already
/// scoped for the same reason: `Sessions` describes the local crew and nothing
/// else, so resolving an NPC hull against it would let a human seated on the
/// player's bridge switch an enemy alliance hull's comms off AI.
///
/// A system may author its own walk (`seek_order`), which this adapter hands
/// straight through to [`coordination::seek_human_host_in`]. It is one more
/// authored vector read in authored order, so nothing below changes: an empty
/// `seek_order` takes the derived owner-first walk, and the shared `seats` list
/// is still built once for the whole hull (the order chooses among the seats,
/// it does not change what a seat is).
///
/// Determinism (issue #984 §2.4): the iteration order is the authored
/// `ShipConfig.systems` and `ShipConfig.stations` vectors — never a map — and
/// the only non-config inputs are `Sessions` and the control sources, neither
/// of which is folded into `state_digest`. A headless or replayed run has an
/// EMPTY `Sessions` (`headless/app.rs`), so no seat ever has a holder, every
/// seek returns `None`, and this system writes the `Ai` that
/// `seed_boot_ratings` already set.
///
/// That argument is only half the story, and the missing half cost two digests.
/// The VALUES this system writes are indeed digest-free, but the SHAPE of the
/// entity it writes them to is not: a `Commands::insert` of
/// [`HumanSeekingHosts`] on the first tick moved the player ship to a fresh
/// archetype mid-run, which shifted the archetype ids every LATER-created
/// archetype received, which swapped the order two NPC hull groups are iterated
/// in by every query that matches both — and `duel` and `rng_coverage` both
/// moved on that alone. (Proven, not inferred: substituting an unrelated
/// zero-sized marker for the real component reproduced both digests exactly.)
/// So the component is NOT inserted here. `LocalShip` `#[require]`s it, so it
/// arrives in the same archetype transition as the marker during the spawn
/// burst, this system takes it as a plain `&mut`, and there is no `Commands`
/// parameter left through which a mid-run move could be reintroduced.
pub fn resolve_human_seeking_hosts(
    mut ships: Query<
        (
            &ShipConfigComponent,
            &mut ShipSystemControlSources,
            Option<&ActiveStationRatings>,
            &ScenarioDetailFloor,
            &mut HumanSeekingHosts,
            &mut VisitingStationHosts,
        ),
        With<LocalShip>,
    >,
    sessions: Res<Sessions>,
) {
    let no_ratings = std::collections::HashMap::new();
    for (ship_config, mut control_sources, ratings, scenario_floor, mut hosts, mut station_hosts) in
        ships.iter_mut()
    {
        let config = &ship_config.0;
        if !config.systems.iter().any(|s| s.human_seeking)
            && !config.stations.iter().any(|station| station.human_seeking)
        {
            continue;
        }
        let seats = coordination::seeking_seats(
            config,
            &control_sources.0,
            ratings.map(|r| &r.0).unwrap_or(&no_ratings),
            |station| sessions.0.holder_for_station(station).map(str::to_string),
        );

        let mut resolved: std::collections::BTreeMap<
            crate::core::messages::SystemId,
            crate::core::messages::StationId,
        > = std::collections::BTreeMap::new();
        let mut decisions: Vec<(crate::core::messages::SystemId, ControlSource)> = Vec::new();
        for system in config.systems.iter().filter(|s| s.human_seeking) {
            match coordination::seek_human_host_in(
                system.station.as_ref(),
                &system.seek_order,
                &seats,
            ) {
                Some(seat) => {
                    resolved.insert(system.id.clone(), seat.station.clone());
                    decisions.push((system.id.clone(), ControlSource::Human));
                }
                None => decisions.push((system.id.clone(), ControlSource::Ai)),
            }
        }

        if decisions
            .iter()
            .any(|(id, source)| control_sources.0.source_for(id) != *source)
        {
            for (id, source) in &decisions {
                control_sources.0.set(id.clone(), *source);
            }
        }
        let mut resolved_stations = Vec::new();
        for station in config
            .stations
            .iter()
            .filter(|station| station.human_seeking)
        {
            let mut assignment = coordination::resolve_visiting_station(
                config,
                station,
                // A candidate direct seat is a valid host for THIS visiting
                // station only when its holder is present (not AFK, issue #1104)
                // AND eligible for it (issue #1103 AC2). An AFK or ineligible
                // holder is skipped and the walk falls through to the next
                // `host_order` entry or AI — the resolver never sees the AFK
                // state's cause, the settings or the reason, only the booleans.
                // Pure per-tick recompute, so an AFK holder is dropped as a host
                // deterministically the moment they step away and re-included the
                // tick after they return (AC3/AC4).
                |candidate| {
                    sessions.0.holder_for_station(candidate).is_some_and(|tok| {
                        !sessions.0.is_afk(tok) && sessions.0.is_eligible(tok, &station.id)
                    })
                },
                &scenario_floor.0,
            );
            if assignment.host.as_ref() == Some(&station.id) {
                assignment.rating = ratings
                    .and_then(|active| active.0.get(&station.id))
                    .cloned()
                    .or_else(|| station.ratings.first().map(|rating| rating.name.clone()))
                    .unwrap_or_default();
            }

            let automated = station
                .ratings
                .iter()
                .find(|rating| rating.name == assignment.rating)
                .map(|rating| &rating.automated_systems);
            for system in config
                .systems
                .iter()
                .filter(|system| system.station.as_ref() == Some(&station.id))
            {
                let source = if assignment.host.is_none()
                    || automated.is_some_and(|ids| ids.contains(&system.id))
                {
                    ControlSource::Ai
                } else {
                    ControlSource::Human
                };
                control_sources.0.set(system.id.clone(), source);
                if let Some(host) = assignment.host.as_ref() {
                    resolved.insert(system.id.clone(), host.clone());
                }
            }
            resolved_stations.push(assignment);
        }
        if hosts.0 != resolved {
            hosts.0 = resolved;
        }
        if station_hosts.0 != resolved_stations {
            station_hosts.0 = resolved_stations;
        }
    }
}

pub fn process_coordination_lag(
    time: Res<Time>,
    mut ship_components: Query<
        (
            &ShipConfigComponent,
            &ShipSystemControlSources,
            &mut CoordinationQueue,
            Option<&mut PendingArcBearingRequest>,
            Option<&mut HelmWaypointClearance>,
            Option<&mut RepairHumanAlerted>,
            Option<&mut crate::console::repair::server::RepairRequestQueue>,
            Option<&mut crate::ship::shields::PendingShieldsThreatBearing>,
            Option<&mut PendingTacticalFrequencyHint>,
            // Read-only, and only for the #737 popup gate below: the same
            // damage store and repair-team state machine the visibility
            // projection reads, so the popup cannot drift from the wire rule.
            Option<&crate::entities::spawner::EntitySystemHull>,
            Option<&crate::console::repair::server::ShipRepairTeams>,
            Has<LocalShip>,
        ),
        With<crate::server_app::Ship>,
    >,
    sessions: Res<Sessions>,
    mut outbox: ResMut<crate::lobby::LobbyOutbox>,
    mut chatter_writer: MessageWriter<AiChatterEvent>,
) {
    let repair_id = crate::ship::system_registry::repair_system_id();
    let shields_id = crate::ship::system_registry::shields_system_id();
    let now = time.elapsed_secs();
    for (
        ship_config,
        control_sources,
        mut queue,
        mut pending_bearing,
        mut waypoint_clearance,
        mut alerted,
        mut repair_queue,
        mut pending_shields_threat,
        mut pending_tactical_hint,
        entity_hull,
        entity_teams,
        is_local,
    ) in ship_components.iter_mut()
    {
        // The #737 damage-visibility projection for this ship. Built once per
        // ship per tick and only used to decide how much detail a
        // `CoordinationPopup` may carry — see `coarsen_repair_request`.
        //
        // Deliberately built by `ship_hull_visibility`, the same constructor the
        // wire projection uses, rather than by calling `HullVisibility
        // ::from_parts` directly: a second on-site resolution path here is free
        // to drift from the one the broadcast enforces, which is the very shape
        // of bug this issue closes. Issue #830: every ship (player + NPC) reads
        // its own per-entity `ShipRepairTeams` — no global-Resource fallback.
        let repair_vis = entity_hull.map(|hull| {
            crate::console::repair::visibility::ship_hull_visibility(
                &hull.0,
                &ship_config.0,
                entity_teams,
            )
        });
        let due = queue.0.due_messages(now);
        for msg in due {
            // ── Ship-wide broadcast (issue #879) ─────────────────────────
            //
            // An intent advisory is not addressed to one console: it goes to
            // every human seat on the SOURCE ship, so a partly-backfilled
            // bridge shares one picture of what the automation is doing. The
            // decision of who receives it is the pure
            // `coordination::broadcast_to_ship`, which is `route_coordination`
            // applied per seat — so a backfilled sender pops up at every human
            // seat, an AI or offline seat gets nothing, and a human sender is
            // suppressed at every seat exactly as human→human always has been.
            // The addressed path below is untouched for every other payload.
            if matches!(msg.payload, CoordinationPayload::IntentAdvisory { .. }) {
                // Popups require a browser-connected console holder; only the
                // LocalShip has one, so NPC bridges narrate to nobody.
                if !is_local {
                    continue;
                }
                let label = if msg.sender_label.is_empty() {
                    coordination::CHATTER_SENDER_AI.to_string()
                } else {
                    msg.sender_label.clone()
                };
                let seats = ship_seats(&ship_config.0, control_sources, &sessions);
                for seat in coordination::broadcast_to_ship(msg.sender_origin, &seats) {
                    let Some(token) = seat.holder.clone() else {
                        continue;
                    };
                    outbox.0.push((
                        crate::lobby::handler::Target::Token(token),
                        crate::core::messages::ServerMessage::CoordinationPopup {
                            target: msg.target.clone(),
                            // The #737 gate, re-applied PER RECIPIENT. An
                            // intent advisory carries no figures of its own,
                            // but a broadcast fans one payload out to seats
                            // with different entitlements, so the coarsening
                            // has to be resolved against each seat rather than
                            // once for the message — otherwise a ship-wide
                            // send would be a way around the boundary the
                            // addressed path enforces.
                            payload: coarsen_repair_request(
                                &msg.payload,
                                repair_vis.as_ref(),
                                Some(&seat.station),
                            ),
                            sender_label: label.clone(),
                        },
                    ));
                }
                continue;
            }

            // Coordination targets are console-level station-id keys (issue
            // #801), so a helm-directed message cannot gate on a `helm`
            // system that no longer exists. The Helm console's effective
            // control source derives from its stick axes: AI when both
            // `helm-thrust` and `helm-steering` are AI-operated (the shape
            // Backfill and NPC spawn produce), otherwise the steering axis —
            // the axis helm-directed coordination (arc bearings, navigation
            // clearances) actually drives — is the representative. Every
            // other target resolves through `policy_for` as before.
            //
            // Issue #873: the Tactical station key gets the same treatment, for
            // the same reason. `SystemId("tactical")` is a *station* key with no
            // registered `[[system]]` behind it (#801 deleted the coarse block),
            // so `policy_for` resolved it to the `Human` default on every hull —
            // which made a BACKFILLED Tactical invisible to the router. A
            // frequency hint or target designation aimed at an AI-run Tactical
            // could only Suppress (human sender) or raise an ownerless broadcast
            // popup (AI sender); it could never Consume, and the AI running the
            // guns never saw it. `any_tactical_system_operates_ai` is the
            // Tactical analogue of `helm_axes_operate_ai` — the same predicate
            // `ai_target_selection` and `tick_npc_auto_match_frequency` already
            // use to decide the guns are on AI — so the bus and the gunnery now
            // agree about who is holding Tactical. When it is false the key
            // falls through to `policy_for` exactly as before, so a human-held
            // Tactical routes unchanged.
            let helm_key = crate::ship::system_registry::helm_station_key();
            let tactical_key = crate::ship::system_registry::tactical_station_key();
            let (target_policy, target_control) = if msg.target == helm_key {
                if helm_axes_operate_ai(control_sources) {
                    (
                        crate::ship::control_source::control_tick_policy(ControlSource::Ai),
                        ControlSource::Ai,
                    )
                } else {
                    let rep = crate::ship::system_registry::helm_steering_system_id();
                    (
                        control_sources.0.policy_for(&rep),
                        control_sources.0.source_for(&rep),
                    )
                }
            } else if msg.target == tactical_key
                && crate::console::weapons::shared::any_tactical_system_operates_ai(
                    control_sources,
                    &ship_config.0,
                )
            {
                (
                    crate::ship::control_source::control_tick_policy(ControlSource::Ai),
                    ControlSource::Ai,
                )
            } else {
                (
                    control_sources.0.policy_for(&msg.target),
                    control_sources.0.source_for(&msg.target),
                )
            };
            let action = if !target_policy.operate_ai && !target_policy.accept_human_input {
                coordination::DeliverAction::Consume
            } else {
                coordination::route_coordination(msg.sender_origin, target_control)
            };

            match action {
                coordination::DeliverAction::Consume => {
                    // RepairRequest for AI repair: push into the priority queue.
                    if target_policy.operate_ai && msg.target == repair_id {
                        if let CoordinationPayload::RepairRequest {
                            station_id,
                            station_label,
                            tier,
                            deficit,
                            ..
                        } = &msg.payload
                        {
                            if let Some(ref mut rq) = repair_queue {
                                rq.push_or_merge(
                                    crate::console::repair::server::RepairQueueEntry {
                                        station_id: station_id.clone(),
                                        station_label: station_label.clone(),
                                        tier: *tier,
                                        // Host-internal path: the enqueue side
                                        // always fills this in. A coarsened
                                        // `None` never reaches the queue, but
                                        // sorting on 0.0 is the safe reading if
                                        // one ever does.
                                        deficit: deficit.unwrap_or(0.0),
                                    },
                                );
                            }
                        }
                    }
                    // AI Helm folds a consumed arc-bearing request into its
                    // steering (issue #677) rather than only chattering about it.
                    if target_policy.operate_ai && msg.target == helm_key {
                        if let CoordinationPayload::ArcBearingRequest { uuid, arcs, .. } =
                            &msg.payload
                        {
                            if let Some(pending) = pending_bearing.as_deref_mut() {
                                // Carry the emitting family's arcs (issue #767)
                                // so `ai_helm_steering` biases toward — and
                                // self-clears against — that family's geometry.
                                pending.target = uuid::Uuid::parse_str(uuid).ok();
                                pending.arcs = arcs.clone();
                            }
                        }
                        // Issue #932: withdraw a standing request whose
                        // emitting family has gone unusable. This is expiry,
                        // not a steering decision — cleared unconditionally,
                        // never gated on `leg_yields_to_arc_requests` (#918),
                        // which only ever gates the steering WRITE a live
                        // request can bias.
                        if let CoordinationPayload::ArcBearingWithdraw { .. } = &msg.payload {
                            if let Some(pending) = pending_bearing.as_deref_mut() {
                                pending.target = None;
                                pending.arcs.clear();
                            }
                        }
                        // Channel-3 Navigation-to-Helm handoff (issues #681,
                        // #702): the order has now served its delivery lag, so
                        // clear the AI Helm to follow this generation of the
                        // ship's `NavigationWaypoint`. No position is copied —
                        // the waypoint is the goal, and `operate_helm` reads it
                        // straight off the ship.
                        if let CoordinationPayload::NavigateTo { generation, .. } = &msg.payload {
                            if let Some(clearance) = waypoint_clearance.as_deref_mut() {
                                clearance.0 = Some(*generation);
                            }
                        }
                    }
                    // AI Shields consumes a Sensors threat bearing to rotate
                    // the closest facing toward the incoming threat (issue #683).
                    if target_policy.operate_ai && msg.target == shields_id {
                        if let CoordinationPayload::ThreatBearing { bearing_rad, .. } = &msg.payload
                        {
                            if let Some(pending) = pending_shields_threat.as_deref_mut() {
                                pending.0 = Some(*bearing_rad);
                            }
                        }
                    }
                    // A backfilled Tactical folds a consumed Sensors frequency
                    // hint into its phaser frequency (issue #873) rather than
                    // dropping it on the floor. `apply_tactical_frequency_hint`
                    // reads this next tick. The sender may be a human on
                    // Sensors or that ship's own Sensors AI — the payload is
                    // identical and nothing here inspects `sender_origin`,
                    // which is what makes a human-crewed Sensors console able
                    // to advise a backfilled Tactical at all.
                    if target_policy.operate_ai && msg.target == tactical_key {
                        if let CoordinationPayload::FrequencyHint { frequency } = &msg.payload {
                            if let Some(pending) = pending_tactical_hint.as_deref_mut() {
                                pending.0 = Some(*frequency);
                            }
                        }
                    }
                    // AI→AI: emit viewscreen chatter for the LocalShip only.
                    // The typed payload crosses to the client, which turns it
                    // into words through the same `gui/coordination-popup.js`
                    // normaliser the phone popup uses (issue #975) — no sentence
                    // is composed here. `from_label` is the sender's owning
                    // STATION id (resolved at enqueue), and `to_label` is the
                    // target's owning STATION id (resolved here) — the station
                    // ALONE, never the station+system pair (Task 3). Both are
                    // `station.*.name` ids the client's `localiseTree` resolves.
                    if is_local && msg.sender_origin == ControlSource::Ai {
                        let from_label = if msg.sender_label.is_empty() {
                            coordination::CHATTER_SENDER_AI.to_string()
                        } else {
                            msg.sender_label.clone()
                        };
                        chatter_writer.write(AiChatterEvent {
                            from_label,
                            to_label: coordination::station_addressee_label(
                                &ship_config.0,
                                &msg.target,
                            ),
                            payload: msg.payload.clone(),
                        });
                    }
                }
                coordination::DeliverAction::Suppress => {}
                coordination::DeliverAction::Popup => {
                    // Popups require a browser-connected console holder.
                    // Only the LocalShip has one — NPCs drain silently.
                    if !is_local {
                        continue;
                    }

                    // Escalation-only filter for repair popups (issue #682):
                    // human repair sees popups only on first-damage and
                    // Disabled/Destroyed tier crossings.
                    if msg.target == repair_id {
                        if let CoordinationPayload::RepairRequest {
                            station_id, tier, ..
                        } = &msg.payload
                        {
                            let already = alerted
                                .as_deref()
                                .and_then(|a| a.0.get(station_id).copied())
                                .unwrap_or(DamageTier::Operational);
                            if *tier < DamageTier::Disabled && already != DamageTier::Operational {
                                continue;
                            }
                            if let Some(a) = alerted.as_deref_mut() {
                                a.0.insert(station_id.clone(), *tier);
                            }
                        }
                    }

                    let label = if msg.sender_label.is_empty() {
                        coordination::CHATTER_SENDER_AI.to_string()
                    } else {
                        msg.sender_label
                    };

                    if let Some(station_id) =
                        coordination::station_for_target(&ship_config.0, &msg.target)
                    {
                        if ship_config.0.station(&station_id).is_some() {
                            let token: Option<String> = sessions
                                .0
                                .holder_for_station(&station_id)
                                .map(|t| t.to_string());

                            if let Some(token) = token {
                                outbox.0.push((
                                    crate::lobby::handler::Target::Token(token),
                                    crate::core::messages::ServerMessage::CoordinationPopup {
                                        target: msg.target.clone(),
                                        payload: coarsen_repair_request(
                                            &msg.payload,
                                            repair_vis.as_ref(),
                                            Some(&station_id),
                                        ),
                                        sender_label: label,
                                    },
                                ));
                            }
                        }
                    } else {
                        // Ownerless target — broadcast. No recipient is
                        // entitled to exact non-Core detail, so coarsen against
                        // "no station".
                        outbox.0.push((
                            crate::lobby::handler::Target::All,
                            crate::core::messages::ServerMessage::CoordinationPopup {
                                target: msg.target.clone(),
                                payload: coarsen_repair_request(
                                    &msg.payload,
                                    repair_vis.as_ref(),
                                    None,
                                ),
                                sender_label: label,
                            },
                        ));
                    }
                }
            }
        }
    }
}

#[cfg(test)]
// Fixture ids only (issue #907): a test that needs "some distinct id" has no
// run to reproduce. Production identity is minted by `crate::world_id`, and
// clippy.toml bans `Uuid::new_v4` outside scopes like this one.
#[allow(clippy::disallowed_methods)]
#[path = "coordination_systems_tests.rs"]
mod tests;
