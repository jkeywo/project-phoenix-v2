use bevy::prelude::*;

use crate::console_bridge::AiChatterEvent;
use crate::damage::DamageTier;
use crate::lobby::{InboundMessage, Sessions};
use crate::messages::{ClientMessage, CoordinationPayload};
use crate::server_app::LocalShip;
use crate::ship::components::{
    CoordinationEnqueue, CoordinationQueue, HelmWaypointClearance, PendingArcBearingRequest,
    PendingTacticalFrequencyHint, RepairHumanAlerted, ShipConfigComponent,
    ShipSystemControlSources,
};
use crate::ship::control_source::ControlSource;
use crate::ship::coordination;
use crate::ship::coordination::QueuedCoordination;
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
        queue.0.enqueue(QueuedCoordination {
            sender_origin: ev.sender_origin,
            target: ev.target.clone(),
            payload: ev.payload.clone(),
            sender_label: ev.sender_label.clone(),
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
    viewer: Option<&crate::messages::StationId>,
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
            Option<&crate::entity_spawner::EntitySystemHull>,
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
    let shields_id = crate::system_registry::shields_system_id();
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
                        crate::lobby_handler::Target::Token(token),
                        crate::messages::ServerMessage::CoordinationPopup {
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
            let helm_key = crate::system_registry::helm_station_key();
            let tactical_key = crate::system_registry::tactical_station_key();
            let (target_policy, target_control) = if msg.target == helm_key {
                if helm_axes_operate_ai(control_sources) {
                    (
                        crate::ship::control_source::control_tick_policy(ControlSource::Ai),
                        ControlSource::Ai,
                    )
                } else {
                    let rep = crate::system_registry::helm_steering_system_id();
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
                    // is composed here. `from_label` is a `chatter.sender.*`
                    // string id (or a player name) resolved on the client;
                    // `to_label` is the raw target key, shown as-is so the
                    // viewscreen and the popup agree.
                    if is_local {
                        let from_label = if msg.sender_label.is_empty() {
                            coordination::CHATTER_SENDER_AI.to_string()
                        } else {
                            msg.sender_label.clone()
                        };
                        chatter_writer.write(AiChatterEvent {
                            from_label,
                            to_label: msg.target.0.clone(),
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

                    let system = ship_config.0.system(&msg.target);
                    let station_opt = system.and_then(|s| s.station.as_ref());

                    if let Some(station_id) = station_opt {
                        if ship_config.0.station(station_id).is_some() {
                            let token: Option<String> = sessions
                                .0
                                .holder_for_station(station_id)
                                .map(|t| t.to_string());

                            if let Some(token) = token {
                                outbox.0.push((
                                    crate::lobby_handler::Target::Token(token),
                                    crate::messages::ServerMessage::CoordinationPopup {
                                        target: msg.target.clone(),
                                        payload: coarsen_repair_request(
                                            &msg.payload,
                                            repair_vis.as_ref(),
                                            Some(station_id),
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
                            crate::lobby_handler::Target::All,
                            crate::messages::ServerMessage::CoordinationPopup {
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

pub fn handle_coordination_messages(mut reader: MessageReader<InboundMessage>) {
    for msg in reader.read() {
        let ClientMessage::SendCoordination { .. } = &msg.msg else {
            continue;
        };
    }
}

#[cfg(test)]
// Fixture ids only (issue #907): a test that needs "some distinct id" has no
// run to reproduce. Production identity is minted by `crate::world_id`, and
// clippy.toml bans `Uuid::new_v4` outside scopes like this one.
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;
    use crate::control_source::ControlSource;
    use crate::messages::SystemId;
    use crate::ship::components::LastSystemTiers;
    use crate::ship::test_support::*;
    use crate::simulation::Ship;

    // ── Issue #684: Destroyed-tier alerts to Captain ─────────────────────────

    #[derive(Resource, Default)]
    struct CoordEnqueueBox(Vec<CoordinationEnqueue>);

    fn collect_coord(
        mut reader: MessageReader<CoordinationEnqueue>,
        mut box_: ResMut<CoordEnqueueBox>,
    ) {
        for m in reader.read() {
            box_.0.push(m.clone());
        }
    }

    fn drain_coord(app: &mut App) -> Vec<CoordinationEnqueue> {
        let msgs = app.world().resource::<CoordEnqueueBox>().0.clone();
        app.world_mut().resource_mut::<CoordEnqueueBox>().0.clear();
        msgs
    }

    fn coord_test_app() -> App {
        let mut app = test_app();
        app.init_resource::<CoordEnqueueBox>()
            .add_systems(PostUpdate, collect_coord);
        let ship = find_ship_entity(&mut app);
        app.world_mut()
            .entity_mut(ship)
            .insert(LastSystemTiers::default());
        let mut q = app
            .world_mut()
            .query_filtered::<&mut ShipConfigComponent, With<Ship>>();
        for mut cfg in q.iter_mut(app.world_mut()) {
            cfg.0.coordination_lag_secs = 0.0;
        }
        app
    }

    fn set_captain_control_source(app: &mut App, source: ControlSource) {
        let mut q = app
            .world_mut()
            .query_filtered::<&mut ShipSystemControlSources, With<Ship>>();
        for mut cs in q.iter_mut(app.world_mut()) {
            cs.0.set(crate::system_registry::captain_system_id(), source);
        }
    }

    #[test]
    fn destroyed_crossing_emits_alert_to_captain() {
        let mut app = coord_test_app();
        let tact_sid = SystemId("tactical".into());
        set_console_hp_direct(&mut app, tact_sid, 0.0);
        tick(&mut app);
        let emitted = drain_coord(&mut app);
        let alerts: Vec<_> = emitted
            .iter()
            .filter(|e| matches!(&e.payload, CoordinationPayload::Alert { .. }))
            .collect();
        assert_eq!(alerts.len(), 1, "expected exactly one Alert");
        assert_eq!(
            alerts[0].target,
            crate::ship::system_registry::captain_system_id(),
            "Alert must target Captain system"
        );
        assert_eq!(alerts[0].sender_label, "tactical");
        assert!(
            matches!(&alerts[0].payload, CoordinationPayload::Alert { .. }),
            "payload must be Alert"
        );
    }

    #[test]
    fn non_destroyed_crossing_does_not_emit_alert() {
        let mut app = coord_test_app();
        let tact_sid = SystemId("tactical".into());
        set_console_hp_direct(&mut app, tact_sid, 5.0);
        tick(&mut app);
        let emitted = drain_coord(&mut app);
        let alerts: Vec<_> = emitted
            .iter()
            .filter(|e| matches!(&e.payload, CoordinationPayload::Alert { .. }))
            .collect();
        assert_eq!(alerts.len(), 0, "no Alert for non-Destroyed crossing");
        assert!(
            emitted
                .iter()
                .any(|e| matches!(&e.payload, CoordinationPayload::RepairRequest { .. })),
            "expected a RepairRequest for Disabled crossing"
        );
    }

    #[test]
    fn destroyed_alert_fires_once() {
        let mut app = coord_test_app();
        let tact_sid = SystemId("tactical".into());
        set_console_hp_direct(&mut app, tact_sid, 0.0);
        tick(&mut app);
        let emitted_t1 = drain_coord(&mut app);
        assert_eq!(
            emitted_t1
                .iter()
                .filter(|e| matches!(&e.payload, CoordinationPayload::Alert { .. }))
                .count(),
            1,
            "first tick must emit Alert"
        );
        tick(&mut app);
        let emitted_t2 = drain_coord(&mut app);
        assert_eq!(
            emitted_t2
                .iter()
                .filter(|e| matches!(&e.payload, CoordinationPayload::Alert { .. }))
                .count(),
            0,
            "second tick must not re-emit Alert (fire-once)"
        );
    }

    #[test]
    fn destroyed_alert_refires_after_restore_and_re_destroy() {
        let mut app = coord_test_app();
        let tact_sid = SystemId("tactical".into());
        set_console_hp_direct(&mut app, tact_sid.clone(), 0.0);
        tick(&mut app);
        assert_eq!(
            drain_coord(&mut app)
                .iter()
                .filter(|e| matches!(&e.payload, CoordinationPayload::Alert { .. }))
                .count(),
            1,
            "first destroy must emit Alert"
        );
        set_console_hp_direct(&mut app, tact_sid.clone(), 25.0);
        tick(&mut app);
        drain_coord(&mut app);
        set_console_hp_direct(&mut app, tact_sid, 0.0);
        tick(&mut app);
        assert_eq!(
            drain_coord(&mut app)
                .iter()
                .filter(|e| matches!(&e.payload, CoordinationPayload::Alert { .. }))
                .count(),
            1,
            "re-destroy after restore must emit Alert again"
        );
    }

    /// Routing test helper: creates a test app without `collect_coord` (to avoid
    /// interfering with the coordination event readers) and sets lag to 0.
    fn routing_test_app() -> App {
        let mut app = test_app();
        let ship = find_ship_entity(&mut app);
        app.world_mut()
            .entity_mut(ship)
            .insert(LastSystemTiers::default());
        let mut q = app
            .world_mut()
            .query_filtered::<&mut ShipConfigComponent, With<Ship>>();
        for mut cfg in q.iter_mut(app.world_mut()) {
            cfg.0.coordination_lag_secs = 0.0;
        }
        app
    }

    #[test]
    fn destroyed_alert_consumed_by_ai_captain() {
        let mut app = routing_test_app();
        start_game_with_helm_and_science(&mut app);
        set_captain_control_source(&mut app, ControlSource::Ai);
        let tact_sid = SystemId("tactical".into());
        set_console_hp_direct(&mut app, tact_sid, 0.0);
        tick(&mut app);
        tick(&mut app);
        let outbox = app.world().resource::<crate::lobby::LobbyOutbox>();
        let popups: Vec<_> = outbox
            .0
            .iter()
            .filter(|(_, msg)| {
                matches!(
                    msg,
                    crate::messages::ServerMessage::CoordinationPopup { .. }
                )
            })
            .collect();
        assert!(
            popups.is_empty(),
            "AI Captain must not produce CoordinationPopup; got {} popup(s)",
            popups.len()
        );
    }

    #[test]
    fn destroyed_alert_shows_popup_for_human_captain() {
        let mut app = routing_test_app();
        start_game_with_helm_and_science(&mut app);
        {
            let mut q = app
                .world_mut()
                .query_filtered::<&mut ShipSystemControlSources, With<Ship>>();
            for mut cs in q.iter_mut(app.world_mut()) {
                cs.0.set(SystemId("tactical".into()), ControlSource::Ai);
            }
        }
        let tact_sid = SystemId("tactical".into());
        set_console_hp_direct(&mut app, tact_sid, 0.0);
        // Tick 1: detect_damage_tier_crossings writes CoordinationEnqueue
        //         into the message send buffer.
        // Tick 2: buffer-swap → handle_coordination_enqueue reads and enqueues
        //         to CoordinationQueue with due_time = now + 0.
        //         process_coordination_lag reads due messages and dispatches
        //         a CoordinationPopup to the LobbyOutbox.
        // Tick 3: consumes the popup and/or allows the broadcast to flush.
        tick(&mut app);
        tick(&mut app);
        tick(&mut app);
        let outbox = app.world().resource::<crate::lobby::LobbyOutbox>();
        let has_popup = outbox.0.iter().any(|(_, msg)| {
            matches!(
                msg,
                crate::messages::ServerMessage::CoordinationPopup { .. }
            )
        });
        assert!(
            has_popup,
            "Human Captain must receive a CoordinationPopup for destroyed system"
        );
    }

    // ── Issue #737: the repair-request popup is subject to the same boundary ──
    //
    // `CoordinationPayload::RepairRequest` targets the `repair` system, which
    // resolves to the Engineering holder. Before the gate, every worsening tier
    // crossing handed Engineering the exact HP deficit of an arbitrary non-Core
    // system with no team dispatched and no travel elapsed — the projection's
    // boundary, walked around through the coordination bus.

    /// Start a game with a human on the Repair station — the station that owns
    /// the `repair` system on the battleship, i.e. Engineering in the role
    /// sense, and therefore the recipient of every `RepairRequest` popup.
    fn start_game_with_engineer(app: &mut App) {
        for (token, name, station) in [
            ("captain", "Alice", "Captain"),
            ("helm", "Hikaru", "Helm"),
            ("engineer", "Scotty", "Repair"),
        ] {
            push(
                app,
                token,
                ClientMessage::Identify {
                    token: token.into(),
                    name: name.into(),
                },
            );
            tick(app);
            push(
                app,
                token,
                ClientMessage::SelectStation {
                    station: station.into(),
                },
            );
            tick(app);
        }
        for token in ["captain", "helm", "engineer"] {
            push(app, token, ClientMessage::SetReady { ready: true });
        }
        tick(app);
        assert_eq!(
            app.world()
                .resource::<Sessions>()
                .0
                .holder_for_station(&crate::messages::StationId("repair".into())),
            Some("engineer"),
            "test setup must seat a human on the station that owns `repair`"
        );
    }

    /// Give the ship a hull whose entries are *declared* systems, so a tier
    /// crossing resolves to a real owning station. `test_app`'s default hull
    /// holds the retired coarse ids, which resolve to no `[[system]]` and would
    /// therefore land in the ownerless Core bucket — the one case #737 lets
    /// through.
    fn give_ship_hull(app: &mut App, entries: &[(&str, f32)]) {
        let hull = crate::damage::SystemHull::from_config(
            &entries
                .iter()
                .map(|(id, hp)| (SystemId((*id).into()), *hp))
                .collect::<Vec<_>>(),
        );
        let ship = find_ship_entity(app);
        app.world_mut()
            .entity_mut(ship)
            .insert(crate::entity_spawner::EntitySystemHull(hull));
    }

    /// Put `system_id` under AI control. `route_coordination` only raises a
    /// popup for an AI sender talking to a human target, which is the shape the
    /// leak had: an AI-run station reporting damage to a human Engineering.
    fn set_ai(app: &mut App, system_id: &SystemId) {
        let mut q = app
            .world_mut()
            .query_filtered::<&mut ShipSystemControlSources, With<Ship>>();
        for mut cs in q.iter_mut(app.world_mut()) {
            cs.0.set(system_id.clone(), ControlSource::Ai);
        }
    }

    /// The deficit carried by the delivered `RepairRequest` popup, if any.
    fn repair_popup_deficits(app: &App) -> Vec<Option<f32>> {
        app.world()
            .resource::<crate::lobby::LobbyOutbox>()
            .0
            .iter()
            .filter_map(|(_, msg)| match msg {
                crate::messages::ServerMessage::CoordinationPopup {
                    payload: CoordinationPayload::RepairRequest { deficit, .. },
                    ..
                } => Some(*deficit),
                _ => None,
            })
            .collect()
    }

    /// Put a repair team physically on site at `system_id` before the crossing.
    fn place_team_on_site(app: &mut App, system_id: &SystemId) {
        use crate::modifiers::repair_teams::RepairTeams;
        let mut teams = RepairTeams::new(1);
        let mut scratch = crate::damage::SystemHull::from_config(&[(system_id.clone(), 100.0)]);
        scratch.set_hp(system_id, 10.0);
        teams.dispatch(0, system_id.clone(), system_id.0.clone());
        // Travel completes → `Repairing`, which is what `on_site_systems()`
        // counts. Same state machine the wire projection reads.
        teams.tick(60.0, &mut scratch);
        assert!(
            teams.on_site_systems().any(|s| s == system_id),
            "test setup must actually put the team on site"
        );
        let ship = find_ship_entity(app);
        app.world_mut()
            .entity_mut(ship)
            .insert(crate::console::repair::server::ShipRepairTeams(teams));
    }

    // ── Issue #873: a human-operated station feeds the backfilled AI ─────────
    //
    // Rule 6 on the coordination bus. A coordination fact is derived from
    // authoritative system state and emitted regardless of who holds the
    // sending console; `sender_origin` is stamped afterwards and used for one
    // thing only — picking Consume / Popup / Suppress at delivery time. The
    // emit-side halves of this live in `ship::sensors` and `console_ai::server`;
    // what these tests cover is the delivery side, including the backfilled
    // Tactical that the router could not see at all before this issue.

    /// Seat a human on Sensors and on Tactical-adjacent nothing else, so the
    /// remaining stations backfill to AI. Modelled on `start_game_with_engineer`.
    fn start_game_with_sensors_officer(app: &mut App) {
        for (token, name, station) in [
            ("captain", "Alice", "Captain"),
            ("sensors", "Spock", "Sensors"),
        ] {
            push(
                app,
                token,
                ClientMessage::Identify {
                    token: token.into(),
                    name: name.into(),
                },
            );
            tick(app);
            push(
                app,
                token,
                ClientMessage::SelectStation {
                    station: station.into(),
                },
            );
            tick(app);
        }
        for token in ["captain", "sensors"] {
            push(app, token, ClientMessage::SetReady { ready: true });
        }
        tick(app);
        assert_eq!(
            app.world()
                .resource::<Sessions>()
                .0
                .holder_for_station(&crate::messages::StationId("sensors".into())),
            Some("sensors"),
            "test setup must seat a human on Sensors"
        );
    }

    /// Put every tactical FINE system (phaser banks, torpedo tubes, the
    /// magazine) on `source` — the set `any_tactical_system_operates_ai`
    /// inspects, moved as one, which is what claiming or vacating the Tactical
    /// station does.
    fn set_tactical_fine_systems(app: &mut App, source: ControlSource) {
        let ids: Vec<SystemId> = {
            let mut q = app
                .world_mut()
                .query_filtered::<&ShipConfigComponent, With<Ship>>();
            let cfg = q.single(app.world()).expect("ship config").clone();
            cfg.0
                .systems
                .iter()
                .filter(|s| {
                    matches!(
                        s.kind.as_str(),
                        crate::system_registry::PHASER_BANK_KIND
                            | crate::system_registry::TORPEDO_TUBE_KIND
                            | crate::system_registry::TORPEDO_MAGAZINE_KIND
                    )
                })
                .map(|s| s.id.clone())
                .collect()
        };
        assert!(
            !ids.is_empty(),
            "the shipped hull must declare tactical fine systems for this fixture to mean anything"
        );
        for id in ids {
            set_fine_control_source(app, id, source);
        }
    }

    /// The shape an unmanned Tactical station backfills to.
    fn backfill_tactical_to_ai(app: &mut App) {
        set_tactical_fine_systems(app, ControlSource::Ai);
    }

    /// Give the ship the two components the Tactical hint path needs, which
    /// `test_app` does not spawn: the landing slot and the thing that moves.
    fn give_ship_tactical_frequency_surface(app: &mut App) {
        let ship = find_ship_entity(app);
        app.world_mut().entity_mut(ship).insert((
            crate::ship_plugin::PendingTacticalFrequencyHint::default(),
            crate::ship_state::ShipPhaserFrequency(0.1),
        ));
    }

    /// Register the react half in the set production puts it in
    /// (`SimSet::Input`), so the tick boundary these tests reason about is the
    /// real one: `process_coordination_lag` lands a value in `Modifiers`, and
    /// the applier reads it in the FOLLOWING tick's `Input`.
    fn add_tactical_hint_applier(app: &mut App) {
        // `FixedUpdate` (issue #895): the applier joins the SimSet chain in
        // the schedule the chain lives in, preserving the one-tick handover
        // window between the router (Modifiers) and next tick's Input.
        app.add_systems(
            FixedUpdate,
            crate::console::weapons::apply_tactical_frequency_hint
                .in_set(crate::sim_sets::SimSet::Input),
        );
    }

    /// Arm the REAL Sensors emitter (`tick_sensors_frequency_hint`) on this
    /// ship, rather than hand-writing the `CoordinationEnqueue` it would have
    /// produced.
    ///
    /// Three pieces of authoritative state, and nothing about who is sitting
    /// where: the hull is made low-fidelity (that emitter serves ships without
    /// `AiHighFidelity`; the high-fidelity twin
    /// `tick_frequency_hint_high_fidelity` reads the same facts through the
    /// operator reaction-delay model), the viewscreen's frozen Combat Lock names
    /// a target, and that target carries a shield frequency to read.
    ///
    /// Ordered `.before(handle_coordination_enqueue)` so the emit lands on the
    /// bus the same tick it is written rather than at the mercy of intra-set
    /// ordering.
    fn arm_real_sensors_frequency_emitter(app: &mut App, target_uuid: &str, frequency: f32) {
        let ship = find_ship_entity(app);
        app.world_mut()
            .entity_mut(ship)
            .remove::<crate::ai_plugin::AiHighFidelity>()
            .insert(crate::ship::sensors::SensorsFrequencyState::default());
        {
            let mut blackboards = app
                .world_mut()
                .get_mut::<crate::server_app::ShipSystemBlackboards>(ship)
                .expect("ship carries system blackboards");
            blackboards.0.insert(
                crate::system_registry::viewscreen_system_id(),
                crate::messages::SystemBlackboard::Viewscreen(
                    crate::messages::ViewscreenBlackboard {
                        combat_lock: Some(target_uuid.to_string()),
                        ..Default::default()
                    },
                ),
            );
        }
        app.world_mut().spawn((
            crate::entity_spawner::EntityUuid(target_uuid.to_string()),
            crate::ship::shields::ShipShields(crate::shield::ShieldSystem::default(), frequency),
        ));
        // `FixedUpdate` (issue #895): the emitter joins the SimSet chain in
        // the schedule the chain lives in, so its `.before` edge stays real.
        app.add_systems(
            FixedUpdate,
            crate::ship::sensors::tick_sensors_frequency_hint
                .in_set(crate::sim_sets::SimSet::Input)
                .before(handle_coordination_enqueue),
        );
    }

    /// Arm the REAL high-fidelity Sensors emitter — the one the PLAYER SHIP
    /// runs, and therefore the one the issue is actually about.
    ///
    /// `arm_real_sensors_frequency_emitter` above removes `AiHighFidelity` to
    /// reach `tick_sensors_frequency_hint`. That is a real path, but it is the
    /// path a DEMOTED NPC hull takes: `server_app::spawn_game_start_entities`
    /// gives `LocalShip` `ai_high_fidelity_components()` at spawn and
    /// `ai::server::lod_ai_ships` never evaluates `LocalShip`, so the player
    /// hull is permanently high-fidelity. A human on the player ship's Sensors
    /// is served by `tick_frequency_hint_high_fidelity` — which until this
    /// fixture existed was only ever exercised in a bare-`App` fixture in
    /// `console_ai::server` that stops at a collector box and never touches the
    /// bus, the router, or the applier.
    ///
    /// So: the marker STAYS, and the emitter is registered under the same
    /// `ai_tick_ready` cadence production gates it with, so what gets pinned end
    /// to end is the chain the player ship really takes.
    fn arm_real_high_fidelity_sensors_emitter(app: &mut App, target_uuid: &str, frequency: f32) {
        let ship = find_ship_entity(app);
        assert!(
            app.world()
                .get::<crate::ai_plugin::AiHighFidelity>(ship)
                .is_some(),
            "this fixture is the PLAYER-ship chain: the hull must keep AiHighFidelity, \
             otherwise it is silently testing the demoted-NPC emitter instead"
        );
        {
            let mut blackboards = app
                .world_mut()
                .get_mut::<crate::server_app::ShipSystemBlackboards>(ship)
                .expect("ship carries system blackboards");
            blackboards.0.insert(
                crate::system_registry::viewscreen_system_id(),
                crate::messages::SystemBlackboard::Viewscreen(
                    crate::messages::ViewscreenBlackboard {
                        combat_lock: Some(target_uuid.to_string()),
                        ..Default::default()
                    },
                ),
            );
        }
        app.world_mut().spawn((
            crate::entity_spawner::EntityUuid(target_uuid.to_string()),
            crate::ship::shields::ShipShields(crate::shield::ShieldSystem::default(), frequency),
        ));
        crate::ai::cadence::register_ai_cadence(app);
        app.add_systems(
            Update,
            crate::console_ai_plugin::tick_frequency_hint_high_fidelity
                .in_set(crate::sim_sets::SimSet::Input)
                .before(handle_coordination_enqueue)
                .run_if(crate::ai::cadence::ai_tick_ready),
        );
    }

    /// How many emitter runs the AUTHORED reaction delay takes, read from the
    /// same two authored defaults the emitter itself reads (rule 11: no literal
    /// tick count here — an authored change must move the fixture with it).
    fn authored_reaction_delay_runs() -> usize {
        let delay =
            crate::ship::sensors::SensorsAiConfigResource::default().frequency_hint_delay_secs;
        let hz = crate::entity_config::GlobalConfig::default().ai_tick_hz;
        (delay * hz).ceil() as usize
    }

    fn ship_phaser_frequency(app: &mut App) -> f32 {
        let ship = find_ship_entity(app);
        app.world()
            .get::<crate::ship_state::ShipPhaserFrequency>(ship)
            .expect("ship carries a phaser frequency")
            .0
    }

    fn coordination_popups(app: &App) -> usize {
        app.world()
            .resource::<crate::lobby::LobbyOutbox>()
            .0
            .iter()
            .filter(|(_, msg)| {
                matches!(
                    msg,
                    crate::messages::ServerMessage::CoordinationPopup { .. }
                )
            })
            .count()
    }

    fn enqueue_coordination(
        app: &mut App,
        sender_origin: ControlSource,
        target: SystemId,
        payload: CoordinationPayload,
    ) {
        let ship = find_ship_entity(app);
        app.world_mut()
            .resource_mut::<Messages<CoordinationEnqueue>>()
            .write(CoordinationEnqueue {
                source_entity: ship,
                sender_origin,
                target,
                payload,
                sender_label: "Sensors".into(),
            });
    }

    /// AC5, end to end in ONE app. A human sits at Sensors; Tactical is unmanned
    /// and backfilled to AI. The ship's own Sensors emitter derives a frequency
    /// advisory from authoritative state, the bus routes it, and the AI running
    /// the guns acts on it.
    ///
    /// Nothing is hand-written here: the chain starts at the REAL emitter
    /// `ship::sensors::tick_sensors_frequency_hint`, armed only with ship state
    /// (a viewscreen Combat Lock on a target whose shields carry 0.83), and ends
    /// at the phaser frequency. A stub `CoordinationEnqueue` would have proved
    /// the routing and reaction halves while leaving the half this issue
    /// actually changed — whether a human-held Sensors console emits at all —
    /// asserted only in a different app, in a different module.
    ///
    /// Before #873 this produced nothing at all, for either of two reasons:
    /// the emitter stood down on a human-held console, and `SystemId("tactical")`
    /// is a station key with no registered `[[system]]`, so it resolved to the
    /// default `Human` policy no matter who was really running the guns — and a
    /// human-origin message to a "human" target is `Suppress`. The AI Tactical
    /// could not be told anything by the human sitting three feet away from it.
    ///
    /// The origin is read back off the ship's LIVE control sources rather than
    /// written as a literal, so the test would stop being about a human the
    /// moment seating one stopped making Sensors human-held.
    #[test]
    fn human_sensors_advisory_reaches_and_moves_a_backfilled_tactical() {
        let mut app = routing_test_app();
        add_tactical_hint_applier(&mut app);
        start_game_with_sensors_officer(&mut app);
        backfill_tactical_to_ai(&mut app);
        give_ship_tactical_frequency_surface(&mut app);
        arm_real_sensors_frequency_emitter(&mut app, "harrow-raider-1", 0.83);

        assert_eq!(
            get_ship_control_sources(&mut app)
                .0
                .source_for(&crate::system_registry::sensors_system_id()),
            ControlSource::Human,
            "the fixture must actually leave Sensors in human hands"
        );

        // Tick 1: tick_sensors_frequency_hint (Input) reads the Combat Lock and
        //         the target's shield frequency and writes CoordinationEnqueue →
        //         handle_coordination_enqueue queues it (lag 0) →
        //         process_coordination_lag (Modifiers) consumes it into the
        //         pending slot, because Tactical operates AI.
        // Tick 2: apply_tactical_frequency_hint (Input) folds it into the guns.
        tick(&mut app);
        tick(&mut app);

        let frequency = ship_phaser_frequency(&mut app);
        assert!(
            (frequency - 0.83).abs() < f32::EPSILON,
            "a backfilled Tactical must act on the human Sensors officer's advisory, and \
             the advisory must come from the ship's own emitter reading the locked target's \
             shields; phaser frequency is {frequency}, expected 0.83"
        );
        assert_eq!(
            coordination_popups(&app),
            0,
            "an advisory consumed by an AI station must not also raise a popup"
        );
    }

    /// AC5 on the chain the PLAYER SHIP actually takes, end to end in ONE app.
    ///
    /// The test above proves the low-fidelity (demoted-NPC) emitter. This one
    /// proves the high-fidelity emitter, and it is the one the issue is about:
    /// the player hull is permanently high-fidelity, so a human sitting at the
    /// player ship's Sensors is served by `tick_frequency_hint_high_fidelity`,
    /// never by `tick_sensors_frequency_hint`. Both are kept — a ship can be on
    /// either side of the LOD split and both must feed a backfilled Tactical.
    ///
    /// It also pins the consequence that reading the spec would otherwise get
    /// backwards: on the player ship a HUMAN Sensors officer's advisory carries
    /// the authored `frequency_hint_delay_secs` reaction delay, exactly as the
    /// AI's does. That is deliberate. An advisory that is instant for a human
    /// sender and delayed for an AI one is a human-vs-AI branch on a
    /// coordination fact (AGENTS.md rule 6); and the "instant" path it replaced
    /// delivered nothing at all, because it addressed the Tactical station key,
    /// which resolves to the `Human` policy default, making a human-origin hint
    /// route Human→Human = Suppress.
    ///
    /// So the delay is asserted in both directions: silent well inside it,
    /// delivered past it.
    #[test]
    fn human_sensors_advisory_reaches_a_backfilled_tactical_on_the_player_ships_high_fidelity_chain(
    ) {
        let mut app = routing_test_app();
        add_tactical_hint_applier(&mut app);
        start_game_with_sensors_officer(&mut app);
        backfill_tactical_to_ai(&mut app);
        give_ship_tactical_frequency_surface(&mut app);
        arm_real_high_fidelity_sensors_emitter(&mut app, "harrow-raider-1", 0.83);

        assert_eq!(
            get_ship_control_sources(&mut app)
                .0
                .source_for(&crate::system_registry::sensors_system_id()),
            ControlSource::Human,
            "the fixture must actually leave Sensors in human hands"
        );
        let before = ship_phaser_frequency(&mut app);
        assert!(
            (before - 0.83).abs() > f32::EPSILON,
            "the fixture must start away from the advised frequency"
        );

        let runs = authored_reaction_delay_runs();
        // Well inside the authored reaction delay.
        for _ in 0..runs / 2 {
            tick(&mut app);
        }
        assert!(
            (ship_phaser_frequency(&mut app) - before).abs() < f32::EPSILON,
            "the authored Sensors reaction delay applies to a human sender too — half \
             of it must not be enough; phaser frequency already moved to {}",
            ship_phaser_frequency(&mut app)
        );

        // Past it, plus the router tick and the applier tick.
        for _ in 0..(runs - runs / 2 + 3) {
            tick(&mut app);
        }
        let frequency = ship_phaser_frequency(&mut app);
        assert!(
            (frequency - 0.83).abs() < f32::EPSILON,
            "on the permanently-high-fidelity PLAYER hull, a human Sensors officer's \
             advisory must still reach and move the backfilled Tactical; phaser \
             frequency is {frequency}, expected 0.83"
        );
        assert_eq!(
            coordination_popups(&app),
            0,
            "an advisory consumed by an AI station must not also raise a popup"
        );
    }

    /// The same delivery, from the ship's own Sensors AI. Both origins must
    /// reach the backfilled Tactical identically — that symmetry is the point,
    /// and asserting only the human half would let an origin branch survive on
    /// the other side.
    #[test]
    fn ai_sensors_advisory_reaches_a_backfilled_tactical_the_same_way() {
        let mut app = routing_test_app();
        add_tactical_hint_applier(&mut app);
        start_game_with_sensors_officer(&mut app);
        backfill_tactical_to_ai(&mut app);
        give_ship_tactical_frequency_surface(&mut app);

        enqueue_coordination(
            &mut app,
            ControlSource::Ai,
            crate::system_registry::tactical_station_key(),
            CoordinationPayload::FrequencyHint { frequency: 0.83 },
        );
        tick(&mut app);
        tick(&mut app);

        let ship = find_ship_entity(&mut app);
        assert!(
            (app.world()
                .get::<crate::ship_state::ShipPhaserFrequency>(ship)
                .unwrap()
                .0
                - 0.83)
                .abs()
                < f32::EPSILON,
            "an AI-origin advisory must reach a backfilled Tactical too"
        );
        assert_eq!(
            coordination_popups(&app),
            0,
            "before #873 this AI→backfilled-Tactical hint fell through to the ownerless \
             branch and BROADCAST a popup to every connected client, because the tactical \
             station key resolves to no [[system]] and therefore no station holder"
        );
    }

    /// AC3. A human-held Tactical must route exactly as it did before: the new
    /// branch is only taken when `any_tactical_system_operates_ai` holds, so a
    /// manned Tactical still falls through to `policy_for` and an AI-origin
    /// advisory still surfaces to the human.
    #[test]
    fn a_human_held_tactical_still_routes_an_ai_advisory_to_a_popup() {
        let mut app = routing_test_app();
        start_game_with_sensors_officer(&mut app);
        give_ship_tactical_frequency_surface(&mut app);
        // No `backfill_tactical_to_ai` — every tactical fine system stays on
        // the default Human source.

        enqueue_coordination(
            &mut app,
            ControlSource::Ai,
            crate::system_registry::tactical_station_key(),
            CoordinationPayload::FrequencyHint { frequency: 0.83 },
        );
        tick(&mut app);
        tick(&mut app);

        assert!(
            coordination_popups(&app) > 0,
            "a human Tactical must still be shown an AI-origin advisory"
        );
        let ship = find_ship_entity(&mut app);
        assert_eq!(
            app.world()
                .get::<crate::ship_plugin::PendingTacticalFrequencyHint>(ship)
                .unwrap()
                .0,
            None,
            "nothing may be consumed on the AI's behalf while a human holds Tactical"
        );
    }

    /// The handover window. `process_coordination_lag` lands a value in
    /// `Modifiers` and `apply_tactical_frequency_hint` reads it in the NEXT
    /// tick's `Input` — so there is exactly one tick in which a human can claim
    /// Tactical after the router decided the addressee was an AI.
    ///
    /// The applier must therefore re-ask the router's own question,
    /// `any_tactical_system_operates_ai`, and DROP the value when the answer has
    /// changed. Applying it would overwrite the frequency the human just dialled
    /// with an advisory addressed to nobody, and the human would have no idea
    /// why their guns detuned.
    ///
    /// Dropped, not deferred: the slot is emptied either way, so the stale hint
    /// cannot re-assert itself the moment the AI takes Tactical back.
    #[test]
    fn a_hint_pending_when_a_human_claims_tactical_is_dropped_not_applied() {
        let mut app = routing_test_app();
        add_tactical_hint_applier(&mut app);
        start_game_with_sensors_officer(&mut app);
        backfill_tactical_to_ai(&mut app);
        give_ship_tactical_frequency_surface(&mut app);

        enqueue_coordination(
            &mut app,
            ControlSource::Human,
            crate::system_registry::tactical_station_key(),
            CoordinationPayload::FrequencyHint { frequency: 0.83 },
        );
        // Tick 1 only: the router has consumed the hint into the pending slot,
        // and the applier has not yet run on it.
        tick(&mut app);
        let ship = find_ship_entity(&mut app);
        assert_eq!(
            app.world()
                .get::<crate::ship_plugin::PendingTacticalFrequencyHint>(ship)
                .unwrap()
                .0,
            Some(0.83),
            "precondition: the router must have landed the hint, so what follows is \
             about the applier and not about the hint never arriving"
        );

        // A human takes Tactical inside the window.
        set_tactical_fine_systems(&mut app, ControlSource::Human);
        let dialled_by_the_human = ship_phaser_frequency(&mut app);
        tick(&mut app);

        assert!(
            (ship_phaser_frequency(&mut app) - dialled_by_the_human).abs() < f32::EPSILON,
            "a hint addressed to the AI that held Tactical a tick ago must not overwrite \
             the frequency its human successor is holding; phaser frequency moved to {}",
            ship_phaser_frequency(&mut app)
        );
        assert_eq!(
            app.world()
                .get::<crate::ship_plugin::PendingTacticalFrequencyHint>(ship)
                .unwrap()
                .0,
            None,
            "the stale hint must be DROPPED, not left pending — otherwise it lands the \
             moment Tactical goes back to AI, long after the fact that produced it"
        );
    }

    /// The applier drains the slot even for a ship it cannot act on.
    ///
    /// `apply_tactical_frequency_hint` takes everything but the slot itself as
    /// `Option`. If `ShipPhaserFrequency` or `ShipConfigComponent` were required,
    /// a `Ship` missing one would be filtered OUT of the query rather than
    /// iterated — its pending hint would never be drained, and would then apply
    /// the moment the missing component appeared, carrying a frequency from an
    /// arbitrarily old tick. Every shipped spawn site attaches all of them today,
    /// so this is a latent hole rather than a live bug; the point of pinning it
    /// is that the doc-comment's "consumed exactly once" is then an invariant of
    /// the system rather than a property of the current spawn sites.
    #[test]
    fn a_pending_hint_is_drained_even_on_a_ship_that_cannot_apply_it() {
        let mut app = routing_test_app();
        add_tactical_hint_applier(&mut app);
        start_game_with_sensors_officer(&mut app);
        backfill_tactical_to_ai(&mut app);
        give_ship_tactical_frequency_surface(&mut app);

        // A ship with the slot but nothing to move.
        let ship = find_ship_entity(&mut app);
        app.world_mut()
            .entity_mut(ship)
            .remove::<crate::ship_state::ShipPhaserFrequency>();
        app.world_mut()
            .entity_mut(ship)
            .insert(crate::ship_plugin::PendingTacticalFrequencyHint(Some(0.83)));

        tick(&mut app);
        assert_eq!(
            app.world()
                .get::<crate::ship_plugin::PendingTacticalFrequencyHint>(ship)
                .unwrap()
                .0,
            None,
            "the slot must be drained even when the hint cannot be applied — leaving it \
             set makes the value land later, out of time with the fact that produced it"
        );

        // The frequency surface appears afterwards; the stale hint must be gone.
        app.world_mut()
            .entity_mut(ship)
            .insert(crate::ship_state::ShipPhaserFrequency(0.1));
        tick(&mut app);
        assert!(
            (ship_phaser_frequency(&mut app) - 0.1).abs() < f32::EPSILON,
            "a hint dropped on a previous tick must not re-assert itself once the \
             missing component appears; phaser frequency moved to {}",
            ship_phaser_frequency(&mut app)
        );
    }

    /// AC2, Helm half. A human on Tactical asks the backfilled Helm to come
    /// about; the AI Helm must receive the request rather than have it
    /// suppressed as "two humans who can just talk to each other".
    #[test]
    fn human_sender_advisory_is_consumed_by_a_backfilled_helm() {
        let mut app = routing_test_app();
        start_game_with_sensors_officer(&mut app);
        set_helm_control_source(&mut app, ControlSource::Ai);
        let ship = find_ship_entity(&mut app);
        app.world_mut()
            .entity_mut(ship)
            .insert(PendingArcBearingRequest::default());

        let target_uuid = uuid::Uuid::new_v4();
        enqueue_coordination(
            &mut app,
            ControlSource::Human,
            crate::system_registry::helm_station_key(),
            CoordinationPayload::ArcBearingRequest {
                uuid: target_uuid.to_string(),
                label: "Harrow Raider".into(),
                family: crate::messages::WeaponFamily::Phasers,
                arcs: Vec::new(),
            },
        );
        tick(&mut app);
        tick(&mut app);

        assert_eq!(
            app.world()
                .get::<PendingArcBearingRequest>(ship)
                .unwrap()
                .target,
            Some(target_uuid),
            "a backfilled Helm must act on a human-origin arc-bearing request"
        );
        assert_eq!(
            coordination_popups(&app),
            0,
            "an AI Helm consumes silently; there is no console holder to pop up at"
        );
    }

    // ── Issue #873: the power brownout advisory, at DELIVERY ────────────────
    //
    // `tick_power_brownout_advisory` used to stamp `sender_origin:
    // ControlSource::Ai` as a literal. It now reads the ship's live
    // `power-reactor` control source. `ship::power`'s own test asserts the tag
    // at the point of emission, which is not enough to call the consequence
    // deliberate: the tag is the ONLY input `route_coordination` has, so
    // changing it changes who is shown the advisory. AC3 says existing
    // consume/popup/suppress behaviour is unchanged, and for this advisory it is
    // NOT — so the change is pinned here, on the delivery side, in both
    // directions.
    //
    // The behaviour that changed: with a human at Power and a human at Helm, a
    // brownout used to pop up at the Helm because it claimed AI origin. It now
    // routes Human→Human = Suppress. That is the correct reading of the rule the
    // router already implements — two humans on the same bridge talk to each
    // other, the bus does not interrupt them — and a hardcoded origin is exactly
    // the bug class this issue removes. It is a deliberate behaviour change, not
    // an oversight, which is why it has a test of its own.

    /// Arm the REAL brownout emitter on this ship: the two components its query
    /// takes, the reactor on `power_source`, and the system itself in the set
    /// `ShipPowerPlugin` registers it in (`SimSet::Modifiers`, ordered before
    /// `process_coordination_lag` so the write and the routing of it are the
    /// production order). `ShipPlugin` — the only plugin `test_app` installs —
    /// does not carry `ShipPowerPlugin`, so without this the emitter would never
    /// run and BOTH tests below would pass for the wrong reason.
    ///
    /// A `CoordEnqueueBox` collector comes with it for exactly that reason: each
    /// test asserts the advisory was really emitted before asserting what
    /// delivery did with it.
    fn arm_brownout_advisory(app: &mut App, power_source: ControlSource) {
        let ship = find_ship_entity(app);
        app.world_mut().entity_mut(ship).insert((
            crate::ship::power::ShipPowerSystem(
                crate::modifiers::power_system::PowerSystem::default(),
            ),
            crate::ship::power::PowerBrownoutState::default(),
        ));
        {
            let mut q = app
                .world_mut()
                .query_filtered::<&mut ShipSystemControlSources, With<Ship>>();
            for mut cs in q.iter_mut(app.world_mut()) {
                cs.0.set(
                    crate::system_registry::power_reactor_system_id(),
                    power_source,
                );
            }
        }
        app.init_resource::<CoordEnqueueBox>()
            .add_systems(PostUpdate, collect_coord)
            .add_systems(
                Update,
                crate::ship::power::tick_power_brownout_advisory
                    .in_set(crate::sim_sets::SimSet::Modifiers)
                    .before(process_coordination_lag),
            );
    }

    fn brownout_advisories(app: &App) -> usize {
        app.world()
            .resource::<CoordEnqueueBox>()
            .0
            .iter()
            .filter(|e| matches!(&e.payload, CoordinationPayload::PowerBrownout { .. }))
            .count()
    }

    /// Push total allocation past the supply ceiling so the battery drains and
    /// the rising edge fires. Same lever `ship::power`'s own brownout tests use.
    fn drive_ship_into_brownout(app: &mut App) {
        let mut q = app
            .world_mut()
            .query_filtered::<&mut crate::ship::power::ShipPowerSystem, With<Ship>>();
        for mut ps in q.iter_mut(app.world_mut()) {
            let _ = ps.0.set_group_allocation(
                &crate::messages::PowerGroupId(
                    crate::modifiers::power_system::HELM_POWER_GROUP.into(),
                ),
                3,
            );
        }
    }

    /// The changed case. Human at Power, human at Helm → Suppress.
    #[test]
    fn a_human_power_officers_brownout_is_suppressed_at_a_human_helm() {
        let mut app = routing_test_app();
        // Seats a human on Helm (and Captain, and Repair).
        start_game_with_engineer(&mut app);
        arm_brownout_advisory(&mut app, ControlSource::Human);
        assert!(
            !helm_axes_operate_ai(&get_ship_control_sources(&mut app)),
            "fixture precondition: Helm must be in human hands, or the Suppress \
             branch is not the one under test"
        );

        drive_ship_into_brownout(&mut app);
        for _ in 0..4 {
            tick(&mut app);
        }

        assert!(
            brownout_advisories(&app) > 0,
            "precondition: the fixture must actually EMIT a brownout advisory, or the \
             popup assertion below passes for the wrong reason"
        );
        assert_eq!(
            coordination_popups(&app),
            0,
            "a human Power officer's brownout must not interrupt a human Helm — before \
             issue #873 the advisory stamped a literal ControlSource::Ai and took the \
             AI→human popup branch no matter who was at Power. This behaviour CHANGED, \
             deliberately: the tag now reports the live reactor control source, so \
             Human→Human routes to Suppress like every other same-origin advisory"
        );
    }

    /// The unchanged case, asserted alongside it so the fix cannot be mistaken
    /// for "brownouts stopped being delivered".
    #[test]
    fn an_ai_power_brownout_still_pops_up_at_a_human_helm() {
        let mut app = routing_test_app();
        start_game_with_engineer(&mut app);
        arm_brownout_advisory(&mut app, ControlSource::Ai);

        drive_ship_into_brownout(&mut app);
        for _ in 0..4 {
            tick(&mut app);
        }

        assert!(
            brownout_advisories(&app) > 0,
            "precondition: the fixture must actually EMIT a brownout advisory"
        );
        assert!(
            coordination_popups(&app) > 0,
            "an AI-run reactor's brownout must still reach the human Helm exactly as it \
             did before issue #873 — only the human-at-Power case changed"
        );
    }

    #[test]
    fn repair_popup_withholds_exact_non_core_deficit_before_a_team_arrives() {
        let mut app = routing_test_app();
        start_game_with_engineer(&mut app);

        // helm-radar is owned by Helm — non-Core, and no team dispatched.
        let radar = SystemId("helm-radar".into());
        give_ship_hull(&mut app, &[("helm-radar", 100.0), ("repair", 100.0)]);
        set_ai(&mut app, &radar);
        set_console_hp_direct(&mut app, radar.clone(), 1.0);
        tick(&mut app);
        tick(&mut app);
        tick(&mut app);

        let deficits = repair_popup_deficits(&app);
        assert!(
            !deficits.is_empty(),
            "Engineering must still be told the system needs attention"
        );
        assert!(
            deficits.iter().all(|d| d.is_none()),
            "the exact HP deficit of a non-Core system must not reach Engineering \
             before a team is on site; got {deficits:?}"
        );
    }

    #[test]
    fn repair_popup_carries_the_exact_deficit_once_a_team_is_on_site() {
        let mut app = routing_test_app();
        start_game_with_engineer(&mut app);

        let radar = SystemId("helm-radar".into());
        give_ship_hull(&mut app, &[("helm-radar", 100.0), ("repair", 100.0)]);
        set_ai(&mut app, &radar);
        place_team_on_site(&mut app, &radar);
        set_console_hp_direct(&mut app, radar.clone(), 1.0);
        tick(&mut app);
        tick(&mut app);
        tick(&mut app);

        let deficits = repair_popup_deficits(&app);
        assert!(
            deficits.iter().any(|d| d.is_some()),
            "a team on site is the information gate opening; got {deficits:?}"
        );
    }

    // ── Issue #879: the ship-wide intent-advisory broadcast ──────────────────
    //
    // Every payload above is addressed to ONE console. An intent advisory is
    // addressed to the ship: it goes to every human seat, so a crew that has
    // lost seats to Backfill still shares one picture of what the automation is
    // doing. These cover the delivery half; the routing rule itself is the pure
    // `coordination::broadcast_to_ship`, and the "when does anything get
    // emitted at all" half is `ship::intent_narration*`.

    /// Put every system the named station owns on `source` — what claiming or
    /// vacating that seat does to the ship's control sources.
    fn set_station_systems_source(app: &mut App, station: &str, source: ControlSource) {
        let ids: Vec<SystemId> = {
            let mut q = app
                .world_mut()
                .query_filtered::<&ShipConfigComponent, With<Ship>>();
            let cfg = q.single(app.world()).expect("ship config").0.clone();
            cfg.systems
                .iter()
                .filter(|s| s.station.as_ref().map(|st| st.0.as_str()) == Some(station))
                .map(|s| s.id.clone())
                .collect()
        };
        assert!(
            !ids.is_empty(),
            "the shipped hull must give `{station}` systems for this fixture to mean anything"
        );
        for id in ids {
            set_fine_control_source(app, id, source);
        }
    }

    /// The session tokens that received an intent-advisory popup, in order.
    fn intent_popup_tokens(app: &App) -> Vec<String> {
        app.world()
            .resource::<crate::lobby::LobbyOutbox>()
            .0
            .iter()
            .filter_map(|(target, msg)| match (target, msg) {
                (
                    crate::lobby_handler::Target::Token(token),
                    crate::messages::ServerMessage::CoordinationPopup {
                        payload: CoordinationPayload::IntentAdvisory { .. },
                        ..
                    },
                ) => Some(token.clone()),
                _ => None,
            })
            .collect()
    }

    fn enqueue_intent_advisory(app: &mut App, sender_origin: ControlSource) {
        enqueue_coordination(
            app,
            sender_origin,
            crate::system_registry::tactical_station_key(),
            CoordinationPayload::IntentAdvisory {
                kind: crate::messages::IntentKind::TargetSwitched,
                subject: Some("Harrow Raider".into()),
                generation: 7,
            },
        );
    }

    /// AC: a backfilled seat's advisory reaches every human seat on the source
    /// ship, and no AI seat — even one that still carries a session token.
    ///
    /// Three humans are seated (Captain, Helm, Repair) and the Repair station's
    /// systems are then put on AI, which is the shape a seat has while it is
    /// backfilled. The advisory must reach the two human seats and stop at the
    /// AI one; the addressed path could not express this at all, because it
    /// delivers to exactly one console.
    #[test]
    fn an_intent_advisory_reaches_every_human_seat_and_no_ai_seat() {
        let mut app = routing_test_app();
        start_game_with_engineer(&mut app);
        set_station_systems_source(&mut app, "repair", ControlSource::Ai);
        backfill_tactical_to_ai(&mut app);

        enqueue_intent_advisory(&mut app, ControlSource::Ai);
        tick(&mut app);
        tick(&mut app);

        assert_eq!(
            intent_popup_tokens(&app),
            vec!["captain".to_string(), "helm".to_string()],
            "every HUMAN seat on the ship, in authored station order — and not \
             the backfilled Repair seat, whose holder token is still there"
        );
    }

    /// The broadcast inherits the delivery matrix rather than replacing it: a
    /// human-held seat's advisory is suppressed at every human seat, exactly as
    /// human→human channel-3 traffic always has been.
    #[test]
    fn a_human_seats_intent_advisory_is_suppressed_across_the_ship() {
        let mut app = routing_test_app();
        start_game_with_engineer(&mut app);

        enqueue_intent_advisory(&mut app, ControlSource::Human);
        tick(&mut app);
        tick(&mut app);

        assert!(
            intent_popup_tokens(&app).is_empty(),
            "two officers on the same bridge talk to each other; the matrix has \
             said so since #494 and the broadcast does not get to disagree"
        );
    }

    /// Delivery is the existing transient popup surface and nothing else — no
    /// durable log is written anywhere on the way.
    #[test]
    fn an_intent_advisory_is_delivered_verbatim_through_the_popup_surface() {
        let mut app = routing_test_app();
        start_game_with_engineer(&mut app);
        backfill_tactical_to_ai(&mut app);

        enqueue_intent_advisory(&mut app, ControlSource::Ai);
        tick(&mut app);
        tick(&mut app);

        let payloads: Vec<CoordinationPayload> = app
            .world()
            .resource::<crate::lobby::LobbyOutbox>()
            .0
            .iter()
            .filter_map(|(_, msg)| match msg {
                crate::messages::ServerMessage::CoordinationPopup { payload, .. } => {
                    Some(payload.clone())
                }
                _ => None,
            })
            .collect();
        assert!(
            payloads.iter().all(|p| *p
                == CoordinationPayload::IntentAdvisory {
                    kind: crate::messages::IntentKind::TargetSwitched,
                    subject: Some("Harrow Raider".into()),
                    generation: 7,
                }),
            "the advisory rides the CoordinationPopup surface unchanged; got {payloads:?}"
        );
        assert!(
            !payloads.is_empty(),
            "precondition: something was delivered"
        );
    }
}
