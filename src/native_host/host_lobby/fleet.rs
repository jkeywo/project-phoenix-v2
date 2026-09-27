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

#[derive(Resource, Default)]
pub struct NativeFleetForceRequest(pub bool);

#[derive(Resource, Default)]
struct NativeFleetPublication {
    configured: bool,
    last_update: String,
    roster_result: Option<NativeRosterResult>,
    join_request: Option<NativeFleetJoinRequest>,
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
            NativeFleetEvent::WireSend(frame) => {
                if let Some(mut wire) = world.get_resource_mut::<NativeFleetWire>() {
                    wire.0.send(frame);
                }
            }
            NativeFleetEvent::Fault { reason, detail } => {
                world.remove_resource::<NativeFleetGmAdmission>();
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
mod tests {
    use super::*;

    #[test]
    fn rendezvous_override_does_not_commit_an_undecided_landing_to_ownership() {
        assert_eq!(publication_owner(true, false, false), None);
        // The ordinary accepted JoinPeer request selects a member even when
        // the operator provided a local service instead of the public default.
        assert_eq!(publication_owner(true, false, true), Some(false));
    }

    #[test]
    fn selected_world_owners_and_explicit_ship_members_keep_their_roles() {
        assert_eq!(publication_owner(true, true, false), Some(true));
        assert_eq!(publication_owner(false, true, true), Some(false));
        assert_eq!(publication_owner(false, false, false), Some(false));
        assert_eq!(publication_owner(false, false, true), Some(false));
    }
    #[test]
    fn native_publication_holds_ready_crew_in_lobby_before_page_load_and_freeze() {
        use crate::core::messages::{ClientMessage, GamePhase};
        use crate::lobby::{CountdownTimer, InboundMessage, LobbyPlugin, Sessions};
        for fleet in [
            None,
            Some((false, false)),
            Some((true, false)),
            Some((false, true)),
        ] {
            let mut app = App::new();
            app.add_plugins((LobbyPlugin, bevy::time::TimePlugin, NativeFleetPlugin))
                .insert_resource(HostLobbyBridgeResource(super::super::HostLobbyBridge::new()))
                .insert_resource(crate::world::config::WorldConfig::default());
            crate::sim_tick::register_sim_tick(&mut app);
            crate::ship::test_support::drive_one_fixed_step_per_update(
                &mut app,
                std::time::Duration::from_secs(1),
            );
            if let Some((owner, joining)) = fleet {
                app.insert_resource(NativeFleetConfig {
                    base: "http://127.0.0.1:1".into(),
                    origin: "http://localhost".into(),
                    owner,
                    stamp: "test".into(),
                    stamp_valid: false,
                    max_slots: 6,
                    max_name_length: 32,
                    max_ship_path_length: 256,
                    ship_path: String::new(),
                    ship_name: String::new(),
                    gm_name: String::new(),
                    operator_id: "gm".into(),
                    credentials: vec![],
                });
                if joining {
                    app.world_mut()
                        .resource_mut::<NativeFleetPublication>()
                        .join_request = Some(NativeFleetJoinRequest {
                        code: "TEST".into(),
                        role: "ship",
                        reconnect: None,
                    });
                }
            }
            app.world_mut()
                .resource_mut::<Sessions>()
                .0
                .register("crew".into(), "Crew".into())
                .unwrap();
            app.world_mut()
                .resource_mut::<Messages<InboundMessage>>()
                .write(InboundMessage {
                    token: "crew".into(),
                    msg: ClientMessage::SetReady { ready: true },
                });
            app.update();
            if matches!(fleet, Some((true, _)) | Some((_, true))) {
                assert!(app.world().resource::<NativeFleetPublication>().configured);
                assert_eq!(app.world().resource::<CountdownTimer>().remaining_secs, 0.0);
                for _ in 0..6 {
                    app.update();
                }
                assert_eq!(
                    app.world().resource::<State<GamePhase>>().get(),
                    &GamePhase::Lobby
                );
                assert!(
                    !app.world()
                        .resource::<crate::lobby::FleetManagedLobby>()
                        .validation_passed
                );
            } else {
                assert!(
                    app.world().resource::<CountdownTimer>().remaining_secs > 0.0,
                    "ordinary native crew keeps its local countdown"
                );
            }
        }
    }
    #[test]
    fn only_an_admitted_own_gm_identity_seeds_prefreeze_presence() {
        use crate::native_host::session_role::{NativeSessionRole, NativeSessionRoleState};
        let mut world = World::new();
        let mut role = NativeSessionRoleState::default();
        role.request(NativeSessionRole::FleetGameMaster);
        role.pending_code = Some("CODE".into());
        let mut identity = crate::native_host::fleet_identity::NativeFleetIdentity {
            role: "gm".into(),
            operator_id: Some("gm-2".into()),
            reconnect_credential: "private".into(),
            role_preset: None,
            claim: Some("slot-5".into()),
        };
        for status in [None, Some("pending"), Some("refused"), Some("unreachable")] {
            role.join_status = status.map(str::to_owned);
            world.insert_resource(role.clone());
            assert!(admitted_gm_operator(&world, &identity).is_none());
        }
        role.join_status = Some("admitted".into());
        world.insert_resource(role.clone());
        assert_eq!(admitted_gm_operator(&world, &identity).unwrap().id, "gm-2");
        identity.role = "ship".into();
        assert!(admitted_gm_operator(&world, &identity).is_none());
        identity.role = "gm".into();
        identity.operator_id = Some(String::new());
        assert!(admitted_gm_operator(&world, &identity).is_none());
        identity.operator_id = Some("gm-2".into());
        role.request(NativeSessionRole::ShipHost);
        world.insert_resource(role);
        assert!(admitted_gm_operator(&world, &identity).is_none());
    }
}
