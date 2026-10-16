use bevy::prelude::*;

use crate::console_bridge::AiChatterEvent;
use crate::core::messages::{ClientMessage, CoordinationAddress, CoordinationPayload, StationId};
use crate::lobby::{InboundMessage, Sessions};
use crate::server_app::LocalShip;
use crate::ship::components::{
    ActiveStationRatings, CoordinationDelivery, CoordinationEnqueue, CoordinationEnqueueCursor,
    CoordinationQueue, DeliveredCoordination, HumanSeekingHosts, OrderedCoordinationPopup,
    ScenarioDetailFloor, ShipConfigComponent, ShipSystemControlSources,
};
use crate::ship::control_source::ControlSource;
use crate::ship::coordination;
use crate::ship::coordination::QueuedCoordination;
use crate::ship::helm_ai::helm_axes_operate_ai;
use crate::sim_tick::SimTick;

pub fn handle_coordination_enqueue(
    mut ship_components: Query<
        (Entity, &ShipConfigComponent, &mut CoordinationQueue),
        With<crate::server_app::Ship>,
    >,
    local_ship_q: Query<Entity, With<LocalShip>>,
    events: Res<Messages<CoordinationEnqueue>>,
    mut event_cursor: ResMut<CoordinationEnqueueCursor>,
    mut inbound: MessageReader<InboundMessage>,
    sessions: Res<Sessions>,
    tick: Res<SimTick>,
    world_config: Option<Res<crate::world::config::WorldConfig>>,
    fixed_time: Res<Time<Fixed>>,
) {
    let sim_tick_hz = world_config.as_ref().map_or_else(
        || 1.0 / fixed_time.timestep().as_secs_f32(),
        |config| config.global.sim_tick_hz,
    );
    let coord_events: Vec<_> = event_cursor.0.read(&events).cloned().collect();
    let inbound_msgs: Vec<_> = inbound.read().cloned().collect();

    // Route typed CoordinationEnqueue events to their source ship's queue.
    for ev in &coord_events {
        let Ok((_e, ship_config, mut queue)) = ship_components.get_mut(ev.source_entity) else {
            // Source ship despawned or lacks a CoordinationQueue — silently drop.
            continue;
        };
        let lag = ship_config.0.coordination_lag_secs;
        let lag_ticks = coordination::coordination_lag_ticks(lag, sim_tick_hz);
        // Channel-3 addresses crew by STATION, not by the fine system that
        // spoke (Task 2). Resolve the emitting system to its owning station's
        // display id here, where the SOURCE ship's config is in scope; an empty
        // `sender_system` opts out and keeps the pre-resolved `sender_label`
        // (the intent-narration path already stamps its own station id).
        let sender_label = if ev.sender_system.0.is_empty() {
            ev.sender_label.clone()
        } else {
            coordination::system_addressee_label(&ship_config.0, &ev.sender_system)
        };
        queue.0.enqueue(QueuedCoordination {
            sender_origin: ev.sender_origin,
            address: ev.address.clone(),
            payload: ev.payload.clone(),
            presentation: ev.presentation.clone(),
            sender_label,
            due_tick: tick.0.saturating_add(lag_ticks),
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
    let lag_ticks = coordination::coordination_lag_ticks(lag, sim_tick_hz);
    for msg in &inbound_msgs {
        let ClientMessage::SendCoordination {
            address,
            payload,
            presentation,
        } = &msg.msg
        else {
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
        // Client-supplied evidence cannot impersonate equipment or assert a future sample.
        // Legacy human requests retain unknown sample time and attribution.
        let mut payload = payload.clone();
        if let CoordinationPayload::RepairRequest { observation, .. } = &mut payload {
            *observation = None;
        }
        queue.0.enqueue(QueuedCoordination {
            sender_origin,
            address: address.clone(),
            payload,
            presentation: presentation.clone(),
            sender_label: player.name.clone(),
            due_tick: tick.0.saturating_add(lag_ticks),
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
    let mut result = payload.clone();
    if let CoordinationPayload::RepairRequest {
        system_id, deficit, ..
    } = &mut result
    {
        if !vis.is_some_and(|v| v.can_see(viewer, system_id)) {
            *deficit = None;
        }
    }
    result
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
                .filter(|s| config.system_home_station(s) == Some(&station.id))
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

/// Resolve the active world's hull-agnostic detail-floor vocabulary to each
/// fleet ship's concrete System ids. This is the production writer of
/// [`ScenarioDetailFloor`]; the Station resolver below remains a pure consumer.
/// Re-running is deliberate: a scenario resource or selected hull may change
/// between missions without leaving the prior world's floor latched.
///
/// Scoped to `With<FleetSlotOf>` — every ship a host in the fleet flies, not
/// just this host's own (issue #1116). A scenario floor that applied only to
/// the locally-crewed hull would give two hosts different effective Station
/// Ratings on the same ship, and therefore different systems under AI, from the
/// first tick of the mission. NPCs are still excluded: they carry no slot, and
/// a mission's crew-complexity floor has nothing to say about them.
pub fn write_scenario_detail_floor(
    world: Option<Res<crate::world::config::WorldConfig>>,
    mut ships: Query<
        (&ShipConfigComponent, &mut ScenarioDetailFloor),
        With<crate::lockstep::FleetSlotOf>,
    >,
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

/// Resolve Console placement in authored order every tick, then project its
/// controller to each owned System. Sessions supply local crew availability;
/// lockstep ships use admitted ratings, seeded from the shared frozen roster.
/// Console placement is derived and never resets System intent or AI memory.
pub fn resolve_human_seeking_hosts(
    mut ships: Query<(
        &ShipConfigComponent,
        &mut ShipSystemControlSources,
        Option<&ActiveStationRatings>,
        &ScenarioDetailFloor,
        &mut HumanSeekingHosts,
        Option<&mut crate::ship_plugin::ConsoleHosts>,
        &crate::lockstep::FleetSlotOf,
    )>,
    sessions: Res<Sessions>,
    roster: Option<Res<crate::lockstep::FleetRoster>>,
    fleet_session: Option<Res<crate::lockstep::FleetLockstep>>,
) {
    let solo_roster = crate::lockstep::FleetRoster::default();
    let roster = roster.as_deref().unwrap_or(&solo_roster);
    let no_ratings = std::collections::HashMap::new();
    for (
        ship_config,
        mut control_sources,
        ratings,
        scenario_floor,
        mut hosts,
        console_hosts,
        slot,
    ) in ships.iter_mut()
    {
        // Whose crew this hull answers to. Without FleetLockstep, the local ship
        // follows live `Sessions` — for both ordinary solo play and an
        // independently restored multi-ship save — while retained remote ships
        // stay on their empty frozen crew. In active lockstep, EVERY ship uses
        // admitted ratings so peers apply crew handoffs at the same due tick,
        // never from their machine-local session views (issues #1116, #1119).
        let local_crew =
            crate::lockstep::uses_live_sessions(roster, fleet_session.is_some(), slot.0);
        let frozen_crew = roster.crew_of(slot.0);
        // Placement eligibility belongs to the Console and proposed rating,
        // independent of the candidate's crew-seat eligibility.
        let seat_is_available = |candidate: &crate::core::messages::StationId,
                                 console: &crate::core::messages::ConsoleId,
                                 rating: &str|
         -> bool {
            if local_crew {
                sessions.0.holder_for_station(candidate).is_some_and(|tok| {
                    !sessions.0.is_afk(tok) && sessions.0.is_console_eligible(tok, console, rating)
                })
            } else if fleet_session.is_some() {
                // Backfill is the replicated absence/AFK/release state. The
                // frozen roster seeds boot only: consulting it after a due
                // AssignStationRating would undo a real mid-mission handoff.
                ratings
                    .and_then(|ratings| ratings.0.get(candidate))
                    .map_or_else(
                        || frozen_crew.iter().any(|(id, _)| id == candidate),
                        |rating| rating != crate::ship::rating::BACKFILL_RATING,
                    )
            } else {
                // Independently resumed remote ships have no current crew.
                frozen_crew.iter().any(|(id, _)| id == candidate)
            }
        };
        let config = &ship_config.0;
        let mut resolved = std::collections::BTreeMap::new();
        let mut resolved_consoles = Vec::new();
        for console in &config.consoles {
            let assignment =
                phoenix_sim_gameplay::ship::console_ownership::resolve_assignment_with_eligibility(
                    config,
                    console,
                    |candidate, rating| seat_is_available(candidate, &console.id, rating),
                    ratings.map(|r| &r.0).unwrap_or(&no_ratings),
                    &scenario_floor.0,
                );
            let host = assignment.host.as_ref();
            let rating = &assignment.rating;
            let depths = config.console_system_depths(console, rating);
            for system in config
                .systems
                .iter()
                .filter(|system| system.console.as_ref() == Some(&console.id))
            {
                resolved.remove(&system.id);
                let source = if host.is_none() || system.ai_only {
                    ControlSource::Ai
                } else {
                    depths
                        .get(&system.id)
                        .copied()
                        .unwrap_or(crate::ship::rating::SystemDepth::Detailed)
                        .control_source()
                };
                control_sources.0.set(system.id.clone(), source);
                if let Some(host) = host.as_ref().filter(|_| !system.ai_only) {
                    resolved.insert(system.id.clone(), (*host).clone());
                }
            }
            resolved_consoles.push(assignment);
        }
        if let Some(mut console_hosts) = console_hosts {
            if console_hosts.0 != resolved_consoles {
                console_hosts.0 = resolved_consoles;
            }
        }
        if hosts.0 != resolved {
            hosts.0 = resolved;
        }
    }
}

/// Reduce one addressed Station to the control policy and [`ControlSource`] the
/// Channel-3 router delivers against.
///
/// `pub(crate)` since issue #1438 so the GM Station-workload advisory can ask
/// the SAME question the router asks rather than re-deriving "is a person
/// reading this seat" from the fine Systems and eventually disagreeing with it
/// about the Helm (whose answer is the AXES, not one representative System).
pub(crate) fn station_delivery_policy(
    config: &crate::ship::config::ShipConfig,
    control_sources: &ShipSystemControlSources,
    station: &StationId,
) -> (
    crate::ship::control_source::ControlTickPolicy,
    ControlSource,
) {
    address_delivery_policy(
        config,
        control_sources,
        &CoordinationAddress::Station(station.clone()),
    )
}

fn address_delivery_policy(
    config: &crate::ship::config::ShipConfig,
    control_sources: &ShipSystemControlSources,
    address: &CoordinationAddress,
) -> (
    crate::ship::control_source::ControlTickPolicy,
    ControlSource,
) {
    // Explicit Station addresses retain their established home-seat reduction;
    // movable Console addresses only reduce their own Systems, never cohosts.
    let owns = |system: &crate::core::messages::SystemId| match address {
        CoordinationAddress::Station(station) => config
            .system(system)
            .is_some_and(|entry| config.system_home_station(entry) == Some(station)),
        CoordinationAddress::Console(console) => config
            .console_for_system(system)
            .is_some_and(|owner| &owner.id == console),
        CoordinationAddress::Ship => false,
    };
    if owns(&crate::ship::system_registry::helm_steering_system_id()) {
        if owns(&crate::ship::system_registry::helm_thrust_system_id())
            && helm_axes_operate_ai(control_sources)
        {
            return (
                crate::ship::control_source::control_tick_policy(ControlSource::Ai),
                ControlSource::Ai,
            );
        }
        let representative = crate::ship::system_registry::helm_steering_system_id();
        return (
            control_sources.0.policy_for(&representative),
            control_sources.0.source_for(&representative),
        );
    }

    let weapon_policies: Vec<_> = config
        .systems
        .iter()
        .filter(|system| owns(&system.id))
        .filter(|system| {
            [
                crate::ship::system_registry::PHASER_BANK_KIND,
                crate::ship::system_registry::TORPEDO_TUBE_KIND,
                crate::ship::system_registry::TORPEDO_MAGAZINE_KIND,
            ]
            .contains(&system.kind.as_str())
        })
        .map(|system| control_sources.0.policy_for(&system.id))
        .collect();
    let tactical = match address {
        CoordinationAddress::Station(station) => config.weapons_station().as_ref() == Some(station),
        CoordinationAddress::Console(_) => {
            !weapon_policies.is_empty()
                && owns(&crate::ship::system_registry::tactical_radar_system_id())
        }
        CoordinationAddress::Ship => false,
    };
    if tactical {
        let any_weapon_ai = match address {
            CoordinationAddress::Station(_) => {
                crate::console::weapons::shared::any_tactical_system_operates_ai(
                    control_sources,
                    config,
                )
            }
            _ => weapon_policies.iter().any(|policy| policy.operate_ai),
        };
        let source = if any_weapon_ai {
            ControlSource::Ai
        } else {
            ControlSource::Human
        };
        return (
            crate::ship::control_source::control_tick_policy(source),
            source,
        );
    }

    let policies: Vec<_> = config
        .systems
        .iter()
        .filter(|system| owns(&system.id))
        .map(|system| control_sources.0.policy_for(&system.id))
        .collect();
    let source = coordination::seat_control_source(&policies);
    (
        crate::ship::control_source::control_tick_policy(source),
        source,
    )
}

pub(crate) fn process_coordination_lag(
    tick: Res<SimTick>,
    mut ship_components: Query<
        (
            Entity,
            &ShipConfigComponent,
            &ShipSystemControlSources,
            Option<&crate::ship::components::ConsoleHosts>,
            &mut CoordinationQueue,
            Option<&crate::entities::spawner::EntitySystemHull>,
            Option<&crate::ship::compartment_damage::ShipDamage>,
            Option<&crate::console::repair::server::ShipRepairTeams>,
            Has<LocalShip>,
        ),
        With<crate::server_app::Ship>,
    >,
    sessions: Res<Sessions>,
    mut chatter_writer: MessageWriter<AiChatterEvent>,
    mut delivered_writer: MessageWriter<DeliveredCoordination>,
    mut popup_writer: MessageWriter<OrderedCoordinationPopup>,
) {
    let repair_id = crate::ship::system_registry::repair_system_id();
    let mut popup_order = 0_u64;
    for (
        ship_entity,
        ship_config,
        control_sources,
        console_hosts,
        mut queue,
        entity_hull,
        physical,
        entity_teams,
        is_local,
    ) in ship_components.iter_mut()
    {
        let repair_address = coordination::address_for_system(&ship_config.0, &repair_id);
        // Visibility affects only human RepairRequest popups. Empty/not-yet-due
        // queues, AI deliveries and unrelated payloads need no hull projection.
        // Share one lazy projection across all such popups for this ship/tick.
        let repair_vis = std::cell::OnceCell::new();
        let repair_visibility = |payload: &CoordinationPayload| {
            if !matches!(payload, CoordinationPayload::RepairRequest { .. }) {
                return None;
            }
            repair_vis
                .get_or_init(|| {
                    physical
                        .map(|damage| {
                            crate::console::repair::visibility::HullVisibility::from_physical(
                                damage,
                                &ship_config.0,
                                entity_teams.map(|teams| &teams.0),
                            )
                        })
                        .or_else(|| {
                            entity_hull.map(|hull| {
                                crate::console::repair::visibility::ship_hull_visibility(
                                    &hull.0,
                                    &ship_config.0,
                                    entity_teams,
                                )
                            })
                        })
                })
                .as_ref()
        };

        for msg in queue.0.due_messages(tick.0) {
            let to_label =
                coordination::coordination_addressee_label_for_config(&ship_config.0, &msg.address);
            // Whole-ship delivery is selected by its address, never by looking
            // for a particular payload variant. The authored Station order is
            // retained for deterministic fan-out.
            if msg.address == CoordinationAddress::Ship {
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
                    popup_writer.write(OrderedCoordinationPopup {
                        order: popup_order,
                        token,
                        message: crate::core::messages::ServerMessage::CoordinationPopup {
                            address: msg.address.clone(),
                            payload: coarsen_repair_request(
                                &msg.payload,
                                repair_visibility(&msg.payload),
                                Some(&seat.station),
                            ),
                            presentation: msg.presentation.clone(),
                            sender_label: label.clone(),
                            to_label: to_label.clone(),
                        },
                    });
                    popup_order += 1;
                }
                continue;
            }

            let station_id = match &msg.address {
                CoordinationAddress::Station(station) => {
                    if ship_config.0.station(station).is_none() {
                        continue;
                    }
                    Some(station)
                }
                CoordinationAddress::Console(console) => {
                    if !ship_config
                        .0
                        .consoles
                        .iter()
                        .any(|item| &item.id == console)
                    {
                        continue;
                    }
                    console_hosts
                        .and_then(|hosts| hosts.0.iter().find(|host| &host.console == console))
                        .and_then(|host| host.host.as_ref())
                }
                CoordinationAddress::Ship => unreachable!(),
            };
            let (target_policy, target_control) =
                address_delivery_policy(&ship_config.0, control_sources, &msg.address);
            let action = if !target_policy.operate_ai && !target_policy.accept_human_input {
                coordination::DeliverAction::Consume
            } else {
                coordination::route_coordination(msg.sender_origin, target_control)
            };

            match action {
                coordination::DeliverAction::Consume => {
                    if target_policy.operate_ai {
                        delivered_writer.write(DeliveredCoordination {
                            source_entity: ship_entity,
                            address: msg.address.clone(),
                            payload: msg.payload.clone(),
                            presentation: msg.presentation.clone(),
                            delivery: CoordinationDelivery::Ai,
                        });
                    }

                    // Typed behavior belongs to each receiving domain. The
                    // generic router emits the delivery and never reads or
                    // writes Helm, Tactical, Repair, or Shields private state.
                    if is_local && msg.sender_origin == ControlSource::Ai {
                        let from_label = if msg.sender_label.is_empty() {
                            coordination::CHATTER_SENDER_AI.to_string()
                        } else {
                            msg.sender_label.clone()
                        };
                        chatter_writer.write(AiChatterEvent {
                            from_label,
                            to_label: to_label.clone(),
                            payload: msg.payload.clone(),
                            presentation: msg.presentation.clone(),
                        });
                    }
                }
                coordination::DeliverAction::Suppress => {}
                coordination::DeliverAction::Popup => {
                    // The peer-identical half of a Popup, first (issue #1438).
                    // Every peer learns that this message routed to a PERSON,
                    // with the ship-internal payload, so a receiver that keeps
                    // ship state about the request — Repair's queue is the one
                    // that does — keeps the same state on every host of the
                    // fleet. The popup below is the presenting peer's alone,
                    // which is exactly why the record cannot ride on it.
                    delivered_writer.write(DeliveredCoordination {
                        source_entity: ship_entity,
                        address: msg.address.clone(),
                        payload: msg.payload.clone(),
                        presentation: msg.presentation.clone(),
                        delivery: CoordinationDelivery::HumanRouted,
                    });
                    if !is_local {
                        continue;
                    }
                    let label = if msg.sender_label.is_empty() {
                        coordination::CHATTER_SENDER_AI.to_string()
                    } else {
                        msg.sender_label
                    };
                    if let Some(token) = station_id
                        .and_then(|station| sessions.0.holder_for_station(station))
                        .map(|token| token.to_string())
                    {
                        let payload = if let CoordinationAddress::Console(console) = &msg.address {
                            let mut payload = msg.payload.clone();
                            if let CoordinationPayload::RepairRequest {
                                system_id, deficit, ..
                            } = &mut payload
                            {
                                if !repair_visibility(&msg.payload).is_some_and(|vis| {
                                    vis.console_can_see(&ship_config.0, console, system_id)
                                }) {
                                    *deficit = None;
                                }
                            }
                            payload
                        } else {
                            coarsen_repair_request(
                                &msg.payload,
                                repair_visibility(&msg.payload),
                                station_id,
                            )
                        };
                        if repair_address.as_ref() == Some(&msg.address)
                            && matches!(&payload, CoordinationPayload::RepairRequest { .. })
                        {
                            // Recipient and visibility stay generic concerns.
                            // Repair owns the remaining human escalation rule
                            // and the resulting popup insertion.
                            delivered_writer.write(DeliveredCoordination {
                                source_entity: ship_entity,
                                address: msg.address,
                                payload,
                                presentation: msg.presentation,
                                delivery: CoordinationDelivery::HumanPopup {
                                    token,
                                    sender_label: label,
                                    order: popup_order,
                                },
                            });
                        } else {
                            popup_writer.write(OrderedCoordinationPopup {
                                order: popup_order,
                                token,
                                message: crate::core::messages::ServerMessage::CoordinationPopup {
                                    address: msg.address,
                                    payload,
                                    presentation: msg.presentation,
                                    sender_label: label,
                                    to_label,
                                },
                            });
                        }
                        popup_order += 1;
                    }
                }
            }
        }
    }
}

/// Publish every human Coordination popup in the lag queue's original order.
///
/// Some outcomes arrive here directly from the generic router; Repair reaches
/// the same seam only after its owning receiver has decided whether escalation
/// warrants a popup. Sorting the shared sequence restores their one global
/// order without moving Repair's decision back into the router.
pub(crate) fn flush_coordination_popups(
    mut popups: MessageReader<OrderedCoordinationPopup>,
    mut outbox: ResMut<crate::lobby::LobbyOutbox>,
) {
    let mut popups: Vec<_> = popups.read().cloned().collect();
    popups.sort_by_key(|popup| popup.order);
    for popup in popups {
        outbox.0.push((
            crate::lobby::handler::Target::Token(popup.token),
            popup.message,
        ));
    }
}

#[cfg(test)]
// Fixture ids only (issue #907): a test that needs "some distinct id" has no
// run to reproduce. Production identity is minted by `crate::world_id`, and
// clippy.toml bans `Uuid::new_v4` outside scopes like this one.
#[allow(clippy::disallowed_methods)]
#[path = "coordination_systems_tests.rs"]
mod tests;

use crate::ship::rating::SystemDepthRuntime as _;

#[cfg(test)]
#[path = "coordination_console_tests.rs"]
mod coordination_console_tests;
