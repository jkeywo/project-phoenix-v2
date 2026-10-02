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
    pub stamp_valid: bool,
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

/// Generation zero adopts the boot socket. At most two live role sockets may
/// overlap while a member binds its delegated successor host connection.
#[derive(Resource, Default)]
struct NativeFleetRoleWires {
    sockets: std::collections::BTreeMap<u64, Box<dyn RelaySocket>>,
    opening: std::collections::BTreeMap<u64, NativeFleetDial>,
    last_generation: u64,
    primary_closed: bool,
    adopted: bool,
    terminal: bool,
}

struct NativeFleetDial {
    receiver: std::sync::Mutex<std::sync::mpsc::Receiver<Result<Box<dyn RelaySocket>, String>>>,
    started: std::time::Instant,
    cancelled: bool,
}
impl Drop for NativeFleetRoleWires {
    fn drop(&mut self) {
        for socket in self.sockets.values_mut() {
            socket.close();
        }
    }
}

#[derive(Clone, Serialize)]
struct NativeContinuationResult {
    generation: u64,
    status: serde_json::Value,
}

#[derive(Resource, Default)]
pub struct NativeFleetForceRequest(pub bool);

#[derive(Resource, Default)]
struct NativeFleetPublication {
    configured: bool,
    last_update: String,
    roster_result: Option<NativeRosterResult>,
    join_request: Option<NativeFleetJoinRequest>,
    continuation_result: Option<NativeContinuationResult>,
}

/// Own public identity from the authenticated owner's admitted-member welcome.
/// This supports pre-freeze Ready presentation, never simulation membership.
#[derive(Resource)]
pub(crate) struct NativeFleetGmAdmission(pub crate::gm_roster::GmOperator);

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
    role: &'static str,
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
    WireSend {
        generation: u64,
        frame: String,
    },
    WireAdopt,
    WireOpen {
        generation: u64,
        role: String,
    },
    WireClose(u64),
    Continuation {
        generation: u64,
        request: serde_json::Value,
    },
    ContinuationFrame {
        epoch: u64,
        source: u32,
        raw: String,
    },
    Fault {
        reason: String,
        detail: String,
    },
    GmBootstrap {
        id: u64,
        generation: u64,
        raw: String,
    },
    BeginGmJoin {
        id: u64,
        join_kind: crate::gm_join::GmJoinKind,
        approved_by: u32,
        candidate_host: u32,
        operator_id: String,
    },
    RefuseGmJoin {
        id: u64,
        reason: String,
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
            HostLobbyRecord::FleetWireSend { generation, frame } => {
                Some(NativeFleetEvent::WireSend {
                    generation: *generation,
                    frame: frame.clone(),
                })
            }
            HostLobbyRecord::FleetWireAdopt => Some(NativeFleetEvent::WireAdopt),
            HostLobbyRecord::FleetWireOpen { generation, role } => {
                Some(NativeFleetEvent::WireOpen {
                    generation: *generation,
                    role: role.clone(),
                })
            }
            HostLobbyRecord::FleetWireClose { generation } => {
                Some(NativeFleetEvent::WireClose(*generation))
            }
            HostLobbyRecord::FleetContinuation {
                generation,
                request,
            } => Some(NativeFleetEvent::Continuation {
                generation: *generation,
                request: request.clone(),
            }),
            HostLobbyRecord::FleetContinuationFrame {
                epoch,
                source,
                frame,
            } => Some(NativeFleetEvent::ContinuationFrame {
                epoch: *epoch,
                source: *source,
                raw: frame.clone(),
            }),
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
            HostLobbyRecord::FleetBeginGmJoin {
                id,
                join_kind,
                approved_by,
                candidate_host,
                operator_id,
            } => Some(NativeFleetEvent::BeginGmJoin {
                id: *id,
                join_kind: *join_kind,
                approved_by: *approved_by,
                candidate_host: *candidate_host,
                operator_id: operator_id.clone(),
            }),
            HostLobbyRecord::FleetRefuseGmJoin { id, reason } => {
                Some(NativeFleetEvent::RefuseGmJoin {
                    id: *id,
                    reason: reason.clone(),
                })
            }
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
    recovery: serde_json::Value,
    gm_join: Option<crate::gm_join::GmJoinProgress>,
    continuation_result: Option<NativeContinuationResult>,
    health: Option<crate::gm_health::GmHealthProjection>,
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
            .init_resource::<NativeFleetRoleWires>()
            .init_resource::<NativeFleetForceRequest>()
            .insert_resource(crate::native_host::fleet_identity::NativeFleetIdentityStore::user())
            .add_systems(PreUpdate, poll_wire.before(super::drain_surface_records))
            .add_systems(PreUpdate, apply_events.after(super::drain_surface_records))
            .add_systems(PreUpdate, prepare_managed_lobby.after(apply_events))
            .add_systems(Update, apply_pending_gm_bootstrap)
            .add_systems(
                PostUpdate,
                publish_state.after(crate::gm_health::publish_health_projection),
            );
    }
}

// Fleet admission precedes frozen-roster adoption. Engage the shared start
// policy before FixedUpdate can process a local crew's SetReady, including
// while the retained Ultralight page is still loading.
fn prepare_managed_lobby(world: &mut World) {
    let Some(config) = world.get_resource::<NativeFleetConfig>() else {
        return;
    };
    let selected = (config.owner && world.contains_resource::<crate::world::config::WorldConfig>())
        || world
            .resource::<NativeFleetPublication>()
            .join_request
            .is_some();
    if selected {
        world
            .resource_mut::<crate::lobby::FleetManagedLobby>()
            .set_enabled(true);
    }
}

fn apply_events(world: &mut World) {
    let events = std::mem::take(&mut world.resource_mut::<NativeFleetEvents>().0);
    for event in events {
        match event {
            NativeFleetEvent::Join { code } => {
                world.remove_resource::<NativeFleetGmAdmission>();
                let Some(config) = world.get_resource::<NativeFleetConfig>().cloned() else {
                    continue;
                };
                let mut reconnect = world
                    .resource::<crate::native_host::fleet_identity::NativeFleetIdentityStore>()
                    .load(&code)
                    .unwrap_or_else(|error| {
                        eprintln!(
                            "phoenix-host: native fleet reconnect identity unavailable — {error}"
                        );
                        None
                    });
                let role = if world
                    .resource::<crate::native_host::session_role::NativeSessionRoleState>()
                    .role()
                    == crate::native_host::session_role::NativeSessionRole::ShipHost
                {
                    "ship"
                } else {
                    "gm"
                };
                if role == "ship" {
                    // A native GM capability stored for this code cannot claim
                    // a ship slot or change this explicitly selected role.
                    reconnect = None;
                }
                world.resource_mut::<NativeFleetPublication>().join_request =
                    Some(NativeFleetJoinRequest {
                        code,
                        role,
                        reconnect,
                    });
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
                    world.remove_resource::<NativeFleetGmAdmission>();
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
            NativeFleetEvent::BeginGmJoin {
                id,
                join_kind,
                approved_by,
                candidate_host,
                operator_id,
            } => {
                begin_native_gm_join(
                    world,
                    id,
                    join_kind,
                    approved_by,
                    candidate_host,
                    operator_id,
                );
            }
            NativeFleetEvent::RefuseGmJoin { id, reason } => {
                if id > 0 && reason == "candidate-disconnected" {
                    let _ = crate::gm_join::refuse_join(
                        world,
                        crate::gm_join::GmJoinId(id),
                        crate::gm_join::GmJoinRefusal::CandidateDisconnected,
                    );
                }
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
                let Some(operator) = admitted_gm_operator(world, &identity) else {
                    continue;
                };
                world.insert_resource(NativeFleetGmAdmission(operator));
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
            NativeFleetEvent::WireAdopt => {
                let lanes = world.resource::<NativeFleetRoleWires>();
                if lanes.adopted || lanes.terminal {
                    world.resource_mut::<NativeFleetRoleWires>().terminal = true;
                    close_all_role_wires(world);
                    wire_event(
                        world,
                        0,
                        "fault",
                        Some("native-fleet-surface-reloaded".into()),
                    );
                } else {
                    world.resource_mut::<NativeFleetRoleWires>().adopted = true;
                }
            }
            NativeFleetEvent::WireOpen { generation, role } => {
                open_role_wire(world, generation, &role)
            }
            NativeFleetEvent::WireClose(generation) => close_role_wire(world, generation),
            NativeFleetEvent::WireSend { generation, frame } => {
                if world.resource::<NativeFleetRoleWires>().terminal {
                    continue;
                }
                if generation == 0 && !world.resource::<NativeFleetRoleWires>().primary_closed {
                    if let Some(mut wire) = world.get_resource_mut::<NativeFleetWire>() {
                        wire.0.send(frame);
                    }
                } else if let Some(wire) = world
                    .resource_mut::<NativeFleetRoleWires>()
                    .sockets
                    .get_mut(&generation)
                {
                    wire.send(frame);
                }
            }
            NativeFleetEvent::Continuation {
                generation,
                request,
            } => {
                let status = match serde_json::from_value::<
                    crate::lockstep::continuation::ContinuationRequest,
                >(request)
                {
                    Ok(request) => serde_json::to_value(
                        crate::lockstep::continuation_systems::enqueue(world, request),
                    )
                    .unwrap(),
                    Err(_) => {
                        crate::lockstep::continuation_systems::refuse(
                            world,
                            "malformed-continuation-request",
                        );
                        serde_json::json!({"status":"refused","reason":"malformed-continuation-request"})
                    }
                };
                world
                    .resource_mut::<NativeFleetPublication>()
                    .continuation_result = Some(NativeContinuationResult { generation, status });
            }
            NativeFleetEvent::ContinuationFrame { epoch, source, raw } => {
                if let Some(frame) = crate::core::codec::decode_mesh_frame(&raw) {
                    crate::lockstep::continuation_systems::enqueue_frame(
                        world,
                        epoch,
                        crate::command_admission::HostSlot(source),
                        frame,
                    );
                } else {
                    crate::lockstep::continuation_systems::refuse(
                        world,
                        "malformed-continuation-frame",
                    );
                }
            }
            NativeFleetEvent::Fault { reason, detail } => {
                world.resource_mut::<NativeFleetRoleWires>().terminal = true;
                world.remove_resource::<NativeFleetGmAdmission>();
                close_all_role_wires(world);
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

// Only the private retained surface can emit this record. Its member adapter
// emits Identity after Welcome on its single authenticated owner connection.
// A stored reconnect claim, unadmitted join or ship identity is insufficient.
fn admitted_gm_operator(
    world: &World,
    identity: &crate::native_host::fleet_identity::NativeFleetIdentity,
) -> Option<crate::gm_roster::GmOperator> {
    let role = world.get_resource::<crate::native_host::session_role::NativeSessionRoleState>()?;
    if role.role() != crate::native_host::session_role::NativeSessionRole::FleetGameMaster
        || role.pending_code.is_none()
        || role.join_status.as_deref() != Some("admitted")
        || identity.role != "gm"
        || identity.reconnect_credential.is_empty()
    {
        return None;
    }
    let claim = identity
        .claim
        .as_deref()?
        .strip_prefix("slot-")?
        .parse::<u32>()
        .ok()?;
    if claim == 0 {
        return None;
    }
    let operator =
        crate::gm_roster::GmOperator::new(identity.operator_id.clone()?, "GM".into(), false);
    crate::gm_roster::GmRoster::try_new(vec![operator.clone()]).ok()?;
    Some(operator)
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

/// The same authoritative transaction the browser bridge invokes. The local
/// page has authenticated the capability; Rust still checks owner, identity and
/// departure before it can schedule a pause, and later proves the restore.
fn begin_native_gm_join(
    world: &mut World,
    id: u64,
    kind: crate::gm_join::GmJoinKind,
    approved_by: u32,
    candidate_host: u32,
    operator_id: String,
) {
    use crate::gm_join::{GmJoinCandidate, GmJoinId, GmJoinKind, GmJoinRefusal, GmJoinRuntime};
    if id == 0 {
        return;
    }
    world.init_resource::<GmJoinRuntime>();
    // A --world boot records SaveScenario. A world selected in the native
    // lobby retains the accepted catalogue selection instead.
    let scenario = world
        .get_resource::<crate::save_slots_lifecycle::SaveScenario>()
        .map(|scenario| scenario.0.as_str())
        .filter(|value| !value.is_empty())
        .or_else(|| {
            let catalog =
                world.get_resource::<crate::native_host::world_load::LobbyScenarioCatalog>()?;
            let selection =
                world.get_resource::<crate::native_host::world_load::LobbySelection>()?;
            crate::lobby::scenario_arbiter::world_path_for(&catalog.0, &selection.0)
        })
        .map(str::to_owned);
    let result = if approved_by == 0
        || candidate_host == 0
        || operator_id.is_empty()
        || operator_id.chars().count() > crate::gm_roster::MAX_GM_OPERATOR_ID_CHARS
    {
        Err(GmJoinRefusal::InvalidCandidate)
    } else if let Some(scenario) = scenario {
        let candidate = GmJoinCandidate {
            host: crate::command_admission::HostSlot(candidate_host),
            operator_id,
        };
        match kind {
            GmJoinKind::FirstTime => crate::gm_join::begin_join(
                world,
                GmJoinId(id),
                crate::command_admission::HostSlot(approved_by),
                candidate,
                scenario,
            ),
            GmJoinKind::Reconnect => {
                crate::gm_join::begin_reconnect(world, GmJoinId(id), candidate, scenario)
            }
        }
    } else {
        Err(GmJoinRefusal::TransferFailed)
    };
    if let Err(reason) = result {
        world
            .resource_mut::<GmJoinRuntime>()
            .refuse(GmJoinId(id), reason);
    }
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

fn wire_event(world: &World, generation: u64, event: &str, frame: Option<String>) {
    if let Some(bridge) = world.get_resource::<HostLobbyBridgeResource>() {
        bridge.0.push_fleet_wire(serde_json::json!({"native_wire":{"generation":generation,"event":event,"frame":frame}}).to_string());
    }
}

fn close_role_wire(world: &mut World, generation: u64) {
    if generation == 0 {
        world.resource_mut::<NativeFleetRoleWires>().primary_closed = true;
        if let Some(mut wire) = world.get_resource_mut::<NativeFleetWire>() {
            wire.0.close();
        }
    } else {
        let mut lanes = world.resource_mut::<NativeFleetRoleWires>();
        if let Some(mut wire) = lanes.sockets.remove(&generation) {
            wire.close();
        }
        if let Some(dial) = lanes.opening.get_mut(&generation) {
            dial.cancelled = true;
        }
    }
}

fn close_all_role_wires(world: &mut World) {
    close_role_wire(world, 0);
    let lanes = world.resource::<NativeFleetRoleWires>();
    let generations: Vec<_> = lanes
        .sockets
        .keys()
        .chain(lanes.opening.keys())
        .copied()
        .collect();
    for generation in generations {
        close_role_wire(world, generation);
    }
}

fn open_role_wire(world: &mut World, generation: u64, role: &str) {
    let lanes = world.resource::<NativeFleetRoleWires>();
    let count = lanes.sockets.len()
        + lanes.opening.len()
        + usize::from(!lanes.primary_closed && world.contains_resource::<NativeFleetWire>());
    if lanes.terminal
        || generation == 0
        || generation <= lanes.last_generation
        || count >= 2
        || !matches!(role, "host" | "join")
    {
        wire_event(world, generation, "close", None);
        return;
    }
    world.resource_mut::<NativeFleetRoleWires>().last_generation = generation;
    let Some(config) = world.get_resource::<NativeFleetConfig>() else {
        wire_event(world, generation, "close", None);
        return;
    };
    let (base, origin, role) = (config.base.clone(), config.origin.clone(), role.to_owned());
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
    // A TCP/TLS first dial may block. Never stall the Bevy thread or its
    // continuation hold; retain timed-out jobs in the same two-socket budget
    // until they finish, so retry storms cannot create unbounded threads.
    let spawned = std::thread::Builder::new()
        .name("fleet-role-dial".into())
        .spawn(move || {
            let result = connect_role_wire(&base, &origin, &role);
            if let Err(std::sync::mpsc::SendError(Ok(mut socket))) = sender.send(result) {
                socket.close();
            }
        });
    if spawned.is_err() {
        wire_event(world, generation, "close", None);
        return;
    }
    world.resource_mut::<NativeFleetRoleWires>().opening.insert(
        generation,
        NativeFleetDial {
            receiver: std::sync::Mutex::new(receiver),
            started: std::time::Instant::now(),
            cancelled: false,
        },
    );
}

#[cfg(feature = "host")]
fn connect_role_wire(base: &str, origin: &str, role: &str) -> Result<Box<dyn RelaySocket>, String> {
    let result = if role == "host" {
        crate::native_host::relay_socket::WsRelaySocket::connect(base, origin)
    } else {
        crate::native_host::relay_socket::WsRelaySocket::connect_join(base, origin)
    };
    result
        .map(|wire| Box::new(wire) as Box<dyn RelaySocket>)
        .map_err(|error| error.to_string())
}
#[cfg(not(feature = "host"))]
fn connect_role_wire(
    _base: &str,
    _origin: &str,
    _role: &str,
) -> Result<Box<dyn RelaySocket>, String> {
    Err("this build has no native WebSocket adapter".into())
}

fn poll_role_dials(world: &mut World) {
    let mut completed = Vec::new();
    let mut expired = Vec::new();
    {
        let mut lanes = world.resource_mut::<NativeFleetRoleWires>();
        for (generation, dial) in &mut lanes.opening {
            match dial.receiver.lock().unwrap().try_recv() {
                Ok(result) => completed.push((*generation, dial.cancelled, result)),
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    completed.push((*generation, dial.cancelled, Err("dial-ended".into())))
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {
                    if !dial.cancelled
                        && dial.started.elapsed() >= std::time::Duration::from_secs(15)
                    {
                        dial.cancelled = true;
                        expired.push(*generation);
                    }
                }
            }
        }
    }
    for generation in expired {
        wire_event(world, generation, "close", None);
    }
    for (generation, cancelled, result) in completed {
        world
            .resource_mut::<NativeFleetRoleWires>()
            .opening
            .remove(&generation);
        match result {
            Ok(mut socket) if cancelled => socket.close(),
            Ok(socket) => {
                world
                    .resource_mut::<NativeFleetRoleWires>()
                    .sockets
                    .insert(generation, socket);
                wire_event(world, generation, "open", None);
            }
            Err(_) if !cancelled => wire_event(world, generation, "close", None),
            Err(_) => {}
        }
    }
}

fn poll_wire(world: &mut World) {
    if !world.resource::<NativeFleetPublication>().configured {
        return;
    }
    poll_role_dials(world);
    let mut events = Vec::new();
    if !world.resource::<NativeFleetRoleWires>().primary_closed {
        if let Some(mut wire) = world.get_resource_mut::<NativeFleetWire>() {
            events.extend(
                wire.0
                    .poll()
                    .into_iter()
                    .map(|frame| (0, "frame", Some(frame))),
            );
            if !wire.0.is_open() {
                events.push((0, "close", None));
            }
        }
    }
    for (generation, wire) in &mut world.resource_mut::<NativeFleetRoleWires>().sockets {
        events.extend(
            wire.poll()
                .into_iter()
                .map(|frame| (*generation, "frame", Some(frame))),
        );
        if !wire.is_open() {
            events.push((*generation, "close", None));
        }
    }
    for (generation, event, frame) in events {
        wire_event(world, generation, event, frame);
        if event == "close" {
            close_role_wire(world, generation);
        }
    }
    if world
        .get_resource::<HostLobbyBridgeResource>()
        .is_some_and(|bridge| bridge.0.fleet_faulted())
    {
        close_all_role_wires(world);
    }
}

fn continuation_result(world: &World) -> Option<NativeContinuationResult> {
    let mut result = world
        .resource::<NativeFleetPublication>()
        .continuation_result
        .clone()?;
    if let Some(lane) =
        world.get_resource::<crate::lockstep::continuation_systems::OwnerContinuation>()
    {
        if result
            .status
            .get("generation")
            .and_then(serde_json::Value::as_u64)
            == Some(lane.status().generation)
        {
            result.status = serde_json::to_value(lane.status()).unwrap();
        }
    }
    Some(result)
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
        } else {
            let Some(owner) = publication_owner(
                config.owner,
                world.contains_resource::<crate::world::config::WorldConfig>(),
                world
                    .resource::<NativeFleetPublication>()
                    .join_request
                    .is_some(),
            ) else {
                return;
            };
            let mut selected = config.clone();
            selected.owner = owner;
            if let Ok(json) = serde_json::to_string(&selected) {
                bridge.0.push_fleet_config(json);
                world.resource_mut::<NativeFleetPublication>().configured = true;
            }
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
        recovery: crate::lockstep::diagnostics::recovery_status(
            world
                .get_resource::<crate::lockstep::PendingHostLoss>()
                .map_or(&[], |losses| losses.records()),
            world
                .get_resource::<crate::lockstep::recovery::RecoveryLog>()
                .and_then(|log| log.last()),
            world
                .get_resource::<crate::lockstep::SlotRecoveryLog>()
                .and_then(|log| log.last()),
            world.get_resource::<crate::lockstep::FleetRoster>(),
        ),
        gm_join: world
            .get_resource::<crate::gm_join::GmJoinRuntime>()
            .map(|runtime| runtime.progress().clone()),
        continuation_result: continuation_result(world),
        health: world
            .get_resource::<crate::gm_health::GmHealthWatch>()
            .and_then(|watch| watch.last())
            .cloned(),
        ship: serde_json::json!({ "template_path": ship_path,
            "name": world.resource::<NativeFleetConfig>().ship_name }),
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

/// An explicit rendezvous selects the service, not the landing role. Creating
/// its owner handle before a --lobby operator chooses Join as Peer makes that
/// ordinary join fail: the embedded adapter already owns a different fleet.
/// Wait until world selection commits hosting or an admitted landing request
/// selects membership. Existing --world owners and --fleet-code members keep
/// their original configuration.
fn publication_owner(
    explicit_owner: bool,
    world_selected: bool,
    join_pending: bool,
) -> Option<bool> {
    if join_pending || !explicit_owner {
        Some(false)
    } else if world_selected {
        Some(true)
    } else {
        None
    }
}

#[cfg(test)]
#[path = "fleet_tests.rs"]
mod tests;
