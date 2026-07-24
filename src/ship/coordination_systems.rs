use bevy::prelude::*;

use crate::console_bridge::AiChatterEvent;
use crate::damage::DamageTier;
use crate::lobby::{InboundMessage, Sessions};
use crate::messages::{ClientMessage, CoordinationPayload};
use crate::server_app::LocalShip;
use crate::ship::components::{
    CoordinationEnqueue, CoordinationQueue, HelmWaypointClearance, PendingArcBearingRequest,
    RepairHumanAlerted, ShipConfigComponent, ShipSystemControlSources,
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

/// Format a `CoordinationPayload` into a short text string for viewscreen chatter.
fn format_coordination_chatter(payload: &CoordinationPayload) -> String {
    match payload {
        CoordinationPayload::Advisory { message } => message.clone(),
        CoordinationPayload::Alert { title, body } => {
            if body.is_empty() {
                title.clone()
            } else {
                format!("{title}: {body}")
            }
        }
        CoordinationPayload::FrequencyHint { frequency } => {
            format!("Frequency hint: {frequency:.1}")
        }
        CoordinationPayload::ShieldFacingDown {
            label,
            offline_remaining,
        } => {
            format!("{label} offline ({offline_remaining:.0}s)")
        }
        CoordinationPayload::ShieldFacingRestored { label } => {
            format!("{label} restored")
        }
        CoordinationPayload::TargetDesignation { label, .. } => {
            format!("Designating target: {label}")
        }
        CoordinationPayload::ArcBearingRequest { label, family, .. } => {
            // Family-aware (issue #767): name the weapon family that needs the
            // bearing. This is server-side AI-to-AI viewscreen flavour text
            // (there is no host-side string table); the player-facing popup
            // label is built family-aware in coordination-popup.js, following
            // that file's existing inline-English chatter pattern.
            let weapons = match family {
                crate::messages::WeaponFamily::Phasers => "phasers",
                crate::messages::WeaponFamily::Blasters => "blasters",
                crate::messages::WeaponFamily::Torpedoes => "torpedoes",
            };
            format!("Come about, bring {weapons} to bear on {label}")
        }
        CoordinationPayload::PowerBrownout {
            label,
            allocated_level,
            ..
        } => {
            format!("{label} brownout (level {allocated_level})")
        }
        CoordinationPayload::NavigateTo { label, .. } => {
            // The generation is an internal handle, not something a bridge
            // officer would say out loud; the label is the human-facing part.
            format!("Navigation: steer toward {label}")
        }
        CoordinationPayload::RepairRequest {
            station_label,
            tier,
            ..
        } => {
            format!("Repair requested for {station_label} ({tier:?})")
        }
        CoordinationPayload::ThreatBearing {
            bearing_rad, label, ..
        } => {
            let bearing_deg = (bearing_rad.to_degrees() + 360.0) % 360.0;
            format!("Sensors: threat bearing {bearing_deg:.0}° - {label}")
        }
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
            // Coordination targets are console-level station-id keys (issue
            // #801), so a helm-directed message cannot gate on a `helm`
            // system that no longer exists. The Helm console's effective
            // control source derives from its stick axes: AI when both
            // `helm-thrust` and `helm-steering` are AI-operated (the shape
            // Backfill and NPC spawn produce), otherwise the steering axis —
            // the axis helm-directed coordination (arc bearings, navigation
            // clearances) actually drives — is the representative. Every
            // other target resolves through `policy_for` as before; station
            // keys with no registered system (e.g. `"tactical"`) get the
            // default Human policy, unchanged from when the coarse tactical
            // id was undeclared.
            let helm_key = crate::system_registry::helm_station_key();
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
                    // AI→AI: emit viewscreen chatter for the LocalShip only.
                    if is_local {
                        let from_label = if msg.sender_label.is_empty() {
                            "AI".to_string()
                        } else {
                            msg.sender_label.clone()
                        };
                        let to_label = msg.target.0.clone();
                        let text = format_coordination_chatter(&msg.payload);
                        chatter_writer.write(AiChatterEvent {
                            from_label,
                            to_label,
                            text,
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
                        "AI".to_string()
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
}
