//! Native host-mesh control plane carried by the retained lobby surface.

use crate::native_host::relay_transport::RelaySocket;
use bevy::prelude::*;
use serde::Serialize;

use super::{HostLobbyBridgeResource, HostLobbyRecord};

#[derive(Resource, Clone, Debug, Serialize)]
pub struct NativeFleetConfig {
    pub base: String,
    pub origin: String,
    pub owner: bool,
    pub stamp: String,
    pub max_slots: usize,
    pub max_name_length: usize,
    pub max_ship_path_length: usize,
    pub ship_path: String,
    pub ship_name: String,
    pub gm_name: String,
    pub operator_id: String,
    /// Host-minted opaque reconnect capabilities. Ultralight is not required
    /// to expose Web Crypto for the protocol to issue equal-GM identities.
    pub credentials: Vec<String>,
}

pub fn mint_reconnect_credentials() -> Vec<String> {
    (0..crate::gm_roster::MAX_GM_OPERATORS)
        .map(|_| {
            crate::native_host::panes::identity::PaneIdentity::mint("fleet-gm")
                .token()
                .to_owned()
        })
        .collect()
}

#[derive(Resource, Default)]
pub struct NativeFleetEvents(Vec<NativeFleetEvent>);

#[derive(Resource)]
pub struct NativeFleetWire(pub Box<dyn RelaySocket>);

#[derive(Resource, Default)]
pub struct NativeFleetForceRequest(pub bool);

#[derive(Resource, Default)]
struct NativeFleetPublication {
    configured: bool,
    last_update: String,
    roster_result: Option<NativeRosterResult>,
    join_request: Option<NativeFleetJoinRequest>,
}

#[derive(Resource)]
struct PendingNativeGmBootstrap {
    id: u64,
    generation: u64,
    roster: crate::lockstep::FleetRoster,
}

#[derive(Clone, Serialize)]
struct NativeRosterResult {
    generation: u64,
    accepted: bool,
    reason: Option<&'static str>,
}

#[derive(Clone, Serialize)]
struct NativeFleetJoinRequest {
    code: String,
    reconnect: Option<crate::native_host::fleet_identity::NativeFleetIdentity>,
}

#[derive(Debug)]
pub enum NativeFleetEvent {
    Join {
        code: String,
    },
    Roster {
        generation: u64,
        raw: String,
    },
    Frame {
        raw: String,
        authenticated_slot: u32,
    },
    StartGrant(serde_json::Value),
    HostLost(u32),
    SlotClaimed(u32),
    WireSend(String),
    Fault {
        reason: String,
        detail: String,
    },
    GmBootstrap {
        id: u64,
        generation: u64,
        raw: String,
    },
    Identity(serde_json::Value),
    ForceResult(serde_json::Value),
    JoinStatus(String),
}

impl NativeFleetEvents {
    pub fn request_join(&mut self, code: String) {
        self.0.push(NativeFleetEvent::Join { code });
    }

    pub fn record(&mut self, record: &HostLobbyRecord) -> bool {
        let event = match record {
            HostLobbyRecord::FleetRoster { generation, roster } => Some(NativeFleetEvent::Roster {
                generation: *generation,
                raw: roster.clone(),
            }),
            HostLobbyRecord::FleetFrame {
                frame,
                authenticated_slot,
            } => Some(NativeFleetEvent::Frame {
                raw: frame.clone(),
                authenticated_slot: *authenticated_slot,
            }),
            HostLobbyRecord::FleetStartGrant { grant } => {
                Some(NativeFleetEvent::StartGrant(grant.clone()))
            }
            HostLobbyRecord::FleetHostLost { slot } => Some(NativeFleetEvent::HostLost(*slot)),
            HostLobbyRecord::FleetSlotClaimed { slot } => {
                Some(NativeFleetEvent::SlotClaimed(*slot))
            }
            HostLobbyRecord::FleetWireSend { frame } => {
                Some(NativeFleetEvent::WireSend(frame.clone()))
            }
            HostLobbyRecord::FleetFault { reason, detail } => Some(NativeFleetEvent::Fault {
                reason: reason.clone(),
                detail: detail.clone(),
            }),
            HostLobbyRecord::FleetGmBootstrap {
                id,
                generation,
                roster,
            } => Some(NativeFleetEvent::GmBootstrap {
                id: *id,
                generation: *generation,
                raw: roster.clone(),
            }),
            HostLobbyRecord::FleetIdentity { identity } => {
                Some(NativeFleetEvent::Identity(identity.clone()))
            }
            HostLobbyRecord::FleetForceResult { result } => {
                Some(NativeFleetEvent::ForceResult(result.clone()))
            }
            HostLobbyRecord::FleetJoinStatus { status } => {
                Some(NativeFleetEvent::JoinStatus(status.clone()))
            }
            HostLobbyRecord::FleetStartPolicy { .. }
            | HostLobbyRecord::FleetGmJoinPending { .. }
            | HostLobbyRecord::FleetGmJoinStatus { .. } => return true,
            _ => return false,
        };
        if let Some(event) = event {
            self.0.push(event);
        }
        true
    }
}

#[derive(Serialize)]
struct NativeFleetUpdate {
    ship: serde_json::Value,
    ship_ready: bool,
    crew: crate::lobby::start_policy::ReadinessTally,
    station_ratings: Vec<(String, String)>,
    gm_ready: bool,
    validation: bool,
    frames: Vec<String>,
    roster_result: Option<NativeRosterResult>,
    force_start: bool,
    join_request: Option<NativeFleetJoinRequest>,
}

pub struct NativeFleetPlugin;

impl Plugin for NativeFleetPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<NativeFleetEvents>()
            .init_resource::<NativeFleetPublication>()
            .init_resource::<NativeFleetForceRequest>()
            .insert_resource(crate::native_host::fleet_identity::NativeFleetIdentityStore::user())
            .add_systems(PreUpdate, poll_wire.before(super::drain_surface_records))
            .add_systems(PreUpdate, apply_events.after(super::drain_surface_records))
            .add_systems(Update, apply_pending_gm_bootstrap)
            .add_systems(PostUpdate, publish_state);
    }
}

fn apply_events(world: &mut World) {
    let events = std::mem::take(&mut world.resource_mut::<NativeFleetEvents>().0);
    for event in events {
        match event {
            NativeFleetEvent::Join { code } => {
                let Some(config) = world.get_resource::<NativeFleetConfig>().cloned() else {
                    continue;
                };
                let reconnect = world
                    .resource::<crate::native_host::fleet_identity::NativeFleetIdentityStore>()
                    .load(&code)
                    .unwrap_or_else(|error| {
                        eprintln!(
                            "phoenix-host: native fleet reconnect identity unavailable — {error}"
                        );
                        None
                    });
                world.resource_mut::<NativeFleetPublication>().join_request =
                    Some(NativeFleetJoinRequest { code, reconnect });
                if let Some(mut wire) = world.get_resource_mut::<NativeFleetWire>() {
                    wire.0.close();
                }
                match connect_join_wire(&config.base, &config.origin) {
                    Ok(wire) => {
                        world.insert_resource(NativeFleetWire(wire));
                    }
                    Err(error) => {
                        world
                            .resource_mut::<crate::native_host::session_role::NativeSessionRoleState>()
                            .join_status = Some("unreachable".into());
                        eprintln!("phoenix-host: native fleet join is unreachable — {error}");
                    }
                }
            }
            NativeFleetEvent::Roster { generation, raw } => {
                let decoded = crate::core::codec::decode_fleet_roster(&raw);
                let accepted = decoded.is_some_and(|(roster, delay)| {
                    let frozen = world
                        .get_resource::<crate::world::config::WorldConfig>()
                        .filter(|config| !config.ship_slots.is_empty())
                        .map(|config| {
                            crate::ship_slots::FrozenShipSlots::from_fleet_roster(
                                &config.ship_slots,
                                &roster,
                            )
                        })
                        .transpose();
                    let Ok(frozen) = frozen else {
                        return false;
                    };
                    let delay = delay.unwrap_or_else(|| crate::lockstep::authored_delay(world));
                    let accepted = crate::lockstep::join_fleet(world, roster, delay);
                    if accepted {
                        if let Some(frozen) = frozen {
                            world.insert_resource(frozen);
                        }
                    }
                    accepted
                });
                if accepted {
                    let mut managed = world.resource_mut::<crate::lobby::FleetManagedLobby>();
                    managed.enabled = true;
                    managed.validation_passed = true;
                }
                world.resource_mut::<NativeFleetPublication>().roster_result =
                    Some(NativeRosterResult {
                        generation,
                        accepted,
                        reason: (!accepted).then_some("fleet-adoption-refused"),
                    });
            }
            NativeFleetEvent::GmBootstrap {
                id,
                generation,
                raw,
            } => {
                let Some((roster, _)) = crate::core::codec::decode_fleet_roster(&raw) else {
                    world.resource_mut::<NativeFleetPublication>().roster_result =
                        Some(NativeRosterResult {
                            generation,
                            accepted: false,
                            reason: Some("invalid-gm-bootstrap"),
                        });
                    continue;
                };
                world.insert_resource(PendingNativeGmBootstrap {
                    id,
                    generation,
                    roster,
                });
            }
            NativeFleetEvent::Identity(identity) => {
                let Some(code) = world
                    .get_resource::<crate::native_host::session_role::NativeSessionRoleState>()
                    .and_then(|role| role.pending_code.clone())
                else {
                    continue;
                };
                let Ok(identity) = serde_json::from_value::<
                    crate::native_host::fleet_identity::NativeFleetIdentity,
                >(identity) else {
                    continue;
                };
                if let Err(error) = world
                    .resource::<crate::native_host::fleet_identity::NativeFleetIdentityStore>()
                    .save(&code, &identity)
                {
                    eprintln!(
                        "phoenix-host: native fleet reconnect identity was not saved — {error}"
                    );
                }
                world
                    .resource_mut::<crate::native_host::session_role::NativeSessionRoleState>()
                    .join_status = Some("admitted".into());
            }
            NativeFleetEvent::JoinStatus(status) => {
                let visible = match status.as_str() {
                    "connecting" | "ready" => "pending",
                    "disconnected" | "error" => "unreachable",
                    _ => status.as_str(),
                };
                world
                    .resource_mut::<crate::native_host::session_role::NativeSessionRoleState>()
                    .join_status = Some(visible.to_string());
                eprintln!("phoenix-host: native fleet join status: {status}");
            }
            NativeFleetEvent::ForceResult(value) => {
                let Ok(result) =
                    serde_json::from_value::<crate::lobby::start_policy::StartGrantResult>(value)
                else {
                    continue;
                };
                world
                    .resource_mut::<crate::native_host::native_gm::start::NativeGmStartRequests>()
                    .record_result(result.clone());
                world
                    .resource_mut::<crate::lobby::StartGrantResults>()
                    .push(result);
            }
            NativeFleetEvent::Frame {
                raw,
                authenticated_slot,
            } => {
                let Some(frame) = crate::core::codec::decode_mesh_frame(&raw) else {
                    continue;
                };
                world
                    .resource_mut::<crate::lockstep::MeshInbox>()
                    .push_from(
                        frame,
                        crate::lockstep::MeshOrigin::Peer(crate::command_admission::HostSlot(
                            authenticated_slot,
                        )),
                    );
            }
            NativeFleetEvent::StartGrant(value) => {
                let Some(grant) = crate::core::codec::decode_start_grant(&value.to_string()) else {
                    continue;
                };
                world
                    .resource_mut::<crate::lobby::PendingStartGrants>()
                    .try_push(grant);
            }
            NativeFleetEvent::HostLost(slot) => {
                let frame = crate::lockstep::order_mesh_inbound(
                    [],
                    [crate::command_admission::HostSlot(slot)],
                )
                .pop();
                if let Some(frame) = frame {
                    world
                        .resource_mut::<crate::lockstep::MeshInbox>()
                        .push_from(frame, crate::lockstep::MeshOrigin::LocalObservation);
                }
            }
            NativeFleetEvent::SlotClaimed(slot) => {
                let owner = world.resource::<crate::lockstep::FleetRoster>().local();
                let tick = world
                    .get_resource::<crate::sim_tick::SimTick>()
                    .map_or(0, |tick| tick.0);
                let claim_seq = world
                    .resource_mut::<crate::lockstep::SlotClaimSequence>()
                    .next_claim();
                let frame =
                    crate::lockstep::MeshFrame::SlotClaim(crate::lockstep::SlotClaimFrame {
                        from: owner,
                        slot: crate::command_admission::HostSlot(slot),
                        claim_seq,
                        tick,
                    });
                world
                    .resource_mut::<crate::lockstep::MeshInbox>()
                    .push_from(frame.clone(), crate::lockstep::MeshOrigin::LocalObservation);
                world
                    .resource_mut::<crate::lockstep::MeshOutbox>()
                    .push(frame);
            }
            NativeFleetEvent::WireSend(frame) => {
                if let Some(mut wire) = world.get_resource_mut::<NativeFleetWire>() {
                    wire.0.send(frame);
                }
            }
            NativeFleetEvent::Fault { reason, detail } => {
                if let Some(mut wire) = world.get_resource_mut::<NativeFleetWire>() {
                    wire.0.close();
                }
                if world
                    .get_resource::<crate::native_host::session_role::NativeSessionRoleState>()
                    .is_some_and(|role| {
                        role.role()
                            == crate::native_host::session_role::NativeSessionRole::FleetGameMaster
                    })
                {
                    world
                        .resource_mut::<crate::native_host::session_role::NativeSessionRoleState>()
                        .join_status = Some(reason.clone());
                }
                eprintln!("phoenix-host: native fleet closed — {reason}: {detail}");
            }
        }
    }
}

#[cfg(feature = "host")]
fn connect_join_wire(base: &str, origin: &str) -> Result<Box<dyn RelaySocket>, String> {
    crate::native_host::relay_socket::WsRelaySocket::connect_join(base, origin)
        .map(|socket| Box::new(socket) as Box<dyn RelaySocket>)
        .map_err(|error| error.to_string())
}

#[cfg(not(feature = "host"))]
fn connect_join_wire(_base: &str, _origin: &str) -> Result<Box<dyn RelaySocket>, String> {
    Err("this build has no native WebSocket adapter".into())
}

fn apply_pending_gm_bootstrap(world: &mut World) {
    if !world.contains_resource::<crate::world::config::WorldConfig>() {
        return;
    }
    let Some(pending) = world.remove_resource::<PendingNativeGmBootstrap>() else {
        return;
    };
    let accepted = crate::gm_join::prepare_candidate_bootstrap(world, pending.roster).is_ok();
    world.resource_mut::<NativeFleetPublication>().roster_result = Some(NativeRosterResult {
        generation: pending.generation,
        accepted,
        reason: (!accepted).then_some("gm-bootstrap-refused"),
    });
    if accepted {
        let _ = pending.id;
    }
}

fn poll_wire(world: &mut World) {
    // The Rust socket may receive its initial `ready` immediately. Leave it in
    // the socket queue until the configuration has been published; otherwise
    // the retained page has no synthetic socket yet and would discard the one
    // frame that prompts `host-open`.
    if !world.resource::<NativeFleetPublication>().configured {
        return;
    }
    let frames = world
        .get_resource_mut::<NativeFleetWire>()
        .map(|mut wire| wire.0.poll())
        .unwrap_or_default();
    if frames.is_empty() {
        return;
    }
    let Some(bridge) = world.get_resource::<HostLobbyBridgeResource>().cloned() else {
        return;
    };
    for frame in frames {
        bridge.0.push_fleet_wire(frame);
    }
    if bridge.0.fleet_faulted() {
        world.resource_mut::<NativeFleetWire>().0.close();
    }
}

fn publish_state(world: &mut World) {
    let Some(bridge) = world.get_resource::<HostLobbyBridgeResource>().cloned() else {
        return;
    };
    let configured = world.resource::<NativeFleetPublication>().configured;
    if let Some(config) = world.get_resource::<NativeFleetConfig>() {
        if configured {
            // The configuration owns one host record; repeating it would ask
            // the embedded surface to open the same fleet every frame.
        } else if let Ok(json) = serde_json::to_string(config) {
            bridge.0.push_fleet_config(json);
            world.resource_mut::<NativeFleetPublication>().configured = true;
        }
    } else {
        return;
    }
    let ship_path = world
        .get_resource::<crate::lobby::SelectedShipResource>()
        .map(|ship| ship.0.clone())
        .unwrap_or_default();
    let crew = world
        .get_resource::<crate::lobby::Sessions>()
        .map(|sessions| sessions.0.readiness_tally())
        .unwrap_or_default();
    let station_ratings = match (
        world.get_resource::<crate::lobby::Sessions>(),
        world.get_resource::<crate::lobby::stations_config::ShipStations>(),
    ) {
        (Some(sessions), Some(stations)) => sessions
            .0
            .lobby_station_ratings(stations)
            .into_iter()
            .map(|(id, rating)| (id.0, rating))
            .collect(),
        _ => Vec::new(),
    };
    let gm_ready = world
        .get_resource::<crate::native_host::native_gm::NativeGmLifecycle>()
        .is_some_and(|gm| gm.enabled && gm.ready);
    let frames = world
        .get_resource_mut::<crate::lockstep::MeshOutbox>()
        .map(|mut outbox| {
            outbox
                .drain()
                .into_iter()
                .filter_map(|frame| crate::core::codec::encode_mesh_frame(&frame).ok())
                .collect()
        })
        .unwrap_or_default();
    let validation = world.contains_resource::<crate::world::config::WorldConfig>();
    let update = NativeFleetUpdate {
        ship: serde_json::json!({ "template_path": ship_path }),
        ship_ready: validation,
        crew,
        station_ratings,
        gm_ready,
        validation,
        frames,
        roster_result: world
            .resource::<NativeFleetPublication>()
            .roster_result
            .clone(),
        force_start: std::mem::take(&mut world.resource_mut::<NativeFleetForceRequest>().0),
        join_request: world
            .resource_mut::<NativeFleetPublication>()
            .join_request
            .take(),
    };
    if let Ok(json) = serde_json::to_string(&update) {
        let mut publication = world.resource_mut::<NativeFleetPublication>();
        if publication.last_update != json {
            publication.last_update.clone_from(&json);
            bridge.0.push_fleet_update(json);
        }
    }
}
