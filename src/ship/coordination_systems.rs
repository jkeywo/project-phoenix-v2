use bevy::prelude::*;

use crate::console_bridge::AiChatterEvent;
use crate::core::messages::{ClientMessage, CoordinationAddress, CoordinationPayload, StationId};
use crate::lobby::{InboundMessage, Sessions};
use crate::server_app::LocalShip;
use crate::ship::components::{
    ActiveStationRatings, CoordinationDelivery, CoordinationEnqueue, CoordinationEnqueueCursor,
    CoordinationQueue, DeliveredCoordination, HumanSeekingHosts, OrderedCoordinationPopup,
    ScenarioDetailFloor, ShipConfigComponent, ShipSystemControlSources, VisitingStationHosts,
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
        queue.0.enqueue(QueuedCoordination {
            sender_origin,
            address: address.clone(),
            payload: payload.clone(),
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

fn station_delivery_policy(
    config: &crate::ship::config::ShipConfig,
    control_sources: &ShipSystemControlSources,
    station: &StationId,
) -> (
    crate::ship::control_source::ControlTickPolicy,
    ControlSource,
) {
    let helm_station = coordination::address_for_system(
        config,
        &crate::ship::system_registry::helm_steering_system_id(),
    );
    if helm_station.as_ref() == Some(&CoordinationAddress::Station(station.clone())) {
        if helm_axes_operate_ai(control_sources) {
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

    if config.weapons_station().as_ref() == Some(station) {
        let source = if crate::console::weapons::shared::any_tactical_system_operates_ai(
            control_sources,
            config,
        ) {
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
        .filter(|system| system.station.as_ref() == Some(station))
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
            &mut CoordinationQueue,
            Option<&crate::entities::spawner::EntitySystemHull>,
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
        mut queue,
        entity_hull,
        entity_teams,
        is_local,
    ) in ship_components.iter_mut()
    {
        let repair_address = coordination::address_for_system(&ship_config.0, &repair_id);
        let repair_vis = entity_hull.map(|hull| {
            crate::console::repair::visibility::ship_hull_visibility(
                &hull.0,
                &ship_config.0,
                entity_teams,
            )
        });

        for msg in queue.0.due_messages(tick.0) {
            let to_label = coordination::coordination_addressee_label(&msg.address);
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
                                repair_vis.as_ref(),
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

            let CoordinationAddress::Station(station_id) = &msg.address else {
                continue;
            };
            if ship_config.0.station(station_id).is_none() {
                // An explicit but unknown Station is invalid for this hull. It
                // is not silently widened to the whole ship.
                continue;
            }
            let (target_policy, target_control) =
                station_delivery_policy(&ship_config.0, control_sources, station_id);
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
                    if !is_local {
                        continue;
                    }
                    let label = if msg.sender_label.is_empty() {
                        coordination::CHATTER_SENDER_AI.to_string()
                    } else {
                        msg.sender_label
                    };
                    if let Some(token) = sessions
                        .0
                        .holder_for_station(station_id)
                        .map(|token| token.to_string())
                    {
                        let payload = coarsen_repair_request(
                            &msg.payload,
                            repair_vis.as_ref(),
                            Some(station_id),
                        );
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
