//! A local GM beside the ship in the same authoritative native simulation.
pub mod bridge;
pub mod document;
pub mod recovery;
pub(crate) mod saves;
pub use saves::publish_outcomes as publish_save_outcomes;
pub mod start;

use super::bridge_display::BridgeLayoutResource;
use crate::console_bridge::*;
use crate::core::{codec, messages::GamePhase};
use crate::gm_action::{NativeGmAuthority, NATIVE_GM_OPERATOR_ID};
use crate::gm_roster::{GmOperator, GmRoster};
use bevy::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Resource, Clone)]
pub struct NativeGmSurface {
    pub bridge: bridge::NativeGmBridge,
    pub url: String,
}

#[derive(Resource, Default)]
pub struct NativeGmLifecycle {
    pub enabled: bool,
    pub ready: bool,
    pub desired_monitor: Option<super::bridge_profile::MonitorIdentity>,
    /// Loss is a pause edge, not a condition that can undo the operator's Resume.
    lost: bool,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum NativeGmRecord {
    Loaded,
    Ready {
        ready: bool,
    },
    ForceStart,
    SurfaceFault,
    RecoveryHostLobby,
    InspectorInterest {
        panels: Vec<crate::gm_projection::GmInspectorKind>,
    },
    ConsoleInterest {
        request: crate::gm_projection::GmConsoleInterest,
    },
    Action {
        request: String,
    },
    Save {
        id: String,
        operation: String,
        name: Option<String>,
    },
}

#[derive(Serialize)]
pub struct NativeGmMetadata {
    pub phase: GamePhase,
    pub host_lobby_unavailable: bool,
    /// The operator bound to this private surface. Fleet GM ids are assigned by
    /// the admitted roster and are not necessarily `native-gm`.
    pub local_operator_id: Option<String>,
    pub role_presets: Vec<crate::world::config::GmRolePresetEntry>,
    pub gms: Vec<GmOperator>,
    pub start_policy: start::NativeGmReadinessTotals,
    pub start_result: Option<crate::lobby::start_policy::StartGrantResult>,
    pub ship_slots: Vec<NativeGmShipSlot>,
}

#[derive(Serialize)]
pub struct NativeGmShipSlot {
    pub id: String,
    pub label: Option<String>,
    pub default_hull: String,
    pub state: &'static str,
    pub can_backfill: bool,
}

pub struct NativeGmPlugin;
impl Plugin for NativeGmPlugin {
    fn build(&self, app: &mut App) {
        use crate::authoritative::{DeclareState, StateClass};
        // Private surface transport and external presence/readiness bookkeeping;
        // canonical actions, rather than this mailbox, enter the world journal.
        app.declare_state::<NativeGmSurface>(StateClass::Timer, "native-local-gm-workspace")
            .declare_state::<NativeGmLifecycle>(StateClass::Timer, "native-local-gm-workspace")
            .declare_state::<NativeGmAuthority>(StateClass::Timer, "native-local-gm-workspace")
            .declare_state::<crate::gm_projection::NativeGmPresentation>(
                StateClass::Presentation,
                "native-local-gm-workspace",
            )
            .init_resource::<NativeGmLifecycle>()
            .init_resource::<NativeGmAuthority>()
            .init_resource::<GmRoster>()
            .add_plugins((
                start::NativeGmStartPlugin,
                crate::gm_projection::GmProjectionPlugin,
                crate::gm_activity::GmActivityPlugin,
                crate::gm_attention::GmAttentionPlugin,
                crate::gm_health::GmHealthPlugin,
                crate::gm_workload::GmWorkloadPlugin,
            ))
            .add_systems(
                PreUpdate,
                // Ready/Unready and a surface failure must reach the roster and
                // pause authority before this frame's fixed tick can launch or
                // advance the simulation. PostUpdate still catches display work.
                (
                    sync_lobby_role_intent,
                    sync_operator_scope,
                    drain_records,
                    sync_presence,
                )
                    .chain()
                    .after(super::host_lobby::drain_surface_records)
                    .before(crate::gm_action::apply_due_actions)
                    .run_if(resource_exists::<NativeGmSurface>),
            )
            .add_systems(
                PostUpdate,
                (sync_presence, feed_projections)
                    .chain()
                    .after(crate::gm_projection::HeldGmProjection)
                    .after(crate::gm_activity::publish_frame_activity)
                    .after(crate::gm_action::publish_session_projection)
                    .after(crate::gm_event::publish_mission_projection)
                    .after(crate::gm_spawn::publish_spawn_projection)
                    .after(crate::gm_comms::publish_comms_projection)
                    .after(crate::gm_attention::publish_attention_projection)
                    .after(crate::gm_health::publish_health_projection)
                    .after(crate::gm_workload::publish_workload_projection)
                    .run_if(resource_exists::<NativeGmSurface>),
            )
            .add_systems(
                Last,
                publish_save_outcomes.run_if(resource_exists::<NativeGmSurface>),
            );
    }
}

/// Commit a lobby role change before the fixed tick evaluates readiness. The
/// surface itself is created later in Update, so every new placement first
/// enters the roster as unready. Keeping this separate from full presence
/// publication also lets the first Loaded record see the enabled role.
fn sync_lobby_role_intent(
    layout: Option<Res<BridgeLayoutResource>>,
    phase: Res<State<GamePhase>>,
    mut state: ResMut<NativeGmLifecycle>,
    mut authority: ResMut<NativeGmAuthority>,
    mut roster: ResMut<GmRoster>,
    mut outbound: MessageWriter<crate::lobby::OutboundMessage>,
    role: Option<Res<super::session_role::NativeSessionRoleState>>,
) {
    if *phase.get() != GamePhase::Lobby {
        return;
    }
    let primary_gm = role.as_ref().is_some_and(|state| {
        state.committed()
            && matches!(
                state.role(),
                super::session_role::NativeSessionRole::StandaloneGameMaster
                    | super::session_role::NativeSessionRole::FleetGameMaster
            )
    });
    let assigned = layout.as_ref().and_then(|layout| {
        if primary_gm {
            Some(layout.layout.viewscreen())
        } else {
            layout.layout.game_master_monitor()
        }
    });
    if state.enabled == assigned.is_some() && state.desired_monitor.as_ref() == assigned {
        return;
    }
    state.enabled = assigned.is_some();
    state.desired_monitor = assigned.cloned();
    state.ready = false;
    authority.connected = false;
    if primary_gm {
        return;
    }
    let mut rows: Vec<_> = roster
        .operators()
        .iter()
        .filter(|gm| gm.id != NATIVE_GM_OPERATOR_ID)
        .cloned()
        .collect();
    if state.enabled {
        rows.push(GmOperator {
            id: NATIVE_GM_OPERATOR_ID.into(),
            name: "GM".into(),
            connected: false,
            ready: false,
        });
    }
    let Ok(replacement) = GmRoster::try_new(rows) else {
        return;
    };
    if *roster != replacement {
        outbound.write(crate::lobby::OutboundMessage {
            target: crate::lobby::Target::All,
            delivery: crate::core::messages::DeliveryClass::Reliable,
            msg: crate::core::messages::ServerMessage::GmRosterChanged {
                gms: replacement.projection(),
            },
        });
        *roster = replacement;
    }
}

fn sync_operator_scope(
    surface: Res<NativeGmSurface>,
    hull: Option<Res<crate::lobby::SelectedShipResource>>,
) {
    if let Some(hull) = hull {
        surface.bridge.set_operator_scope(&hull.0);
    }
}

fn drain_records(world: &mut World) {
    let Some(surface) = world.get_resource::<NativeGmSurface>().cloned() else {
        return;
    };
    for json in surface.bridge.take_records() {
        let Some(record) = codec::decode_native_gm_record(&json) else {
            continue;
        };
        if !world.resource::<NativeGmLifecycle>().enabled {
            continue;
        }
        match record {
            NativeGmRecord::ConsoleInterest { request } if request.valid() => {
                world
                    .resource_mut::<crate::gm_projection::GmConsoleSubscriptions>()
                    .requests
                    .insert(request.consumer, request);
            }
            NativeGmRecord::InspectorInterest { panels } => {
                world
                    .resource_mut::<crate::gm_projection::GmInspectorInterest>()
                    .0 = Some(panels.into_iter().collect());
            }
            NativeGmRecord::Save {
                id,
                operation,
                name,
            } if surface.bridge.live() => {
                saves::request(world, &surface.bridge, id, &operation, name);
            }
            NativeGmRecord::RecoveryHostLobby => {
                recovery::request(world);
            }
            NativeGmRecord::SurfaceFault => {
                surface.bridge.fault();
                break;
            }
            NativeGmRecord::Loaded => surface.bridge.mark_live(),
            NativeGmRecord::Ready { ready }
                if world.resource::<State<GamePhase>>().get() == &GamePhase::Lobby =>
            {
                world.resource_mut::<NativeGmLifecycle>().ready = ready;
            }
            NativeGmRecord::ForceStart
                if surface.bridge.live()
                    && world.resource::<State<GamePhase>>().get() == &GamePhase::Lobby =>
            {
                let fleet_peer = world
                    .get_resource::<super::session_role::NativeSessionRoleState>()
                    .is_some_and(|state| {
                        state.committed()
                            && state.role()
                                == super::session_role::NativeSessionRole::FleetGameMaster
                    });
                if fleet_peer {
                    world
                        .resource_mut::<super::host_lobby::fleet::NativeFleetForceRequest>()
                        .0 = true;
                } else {
                    world
                        .resource_mut::<start::NativeGmStartRequests>()
                        .request();
                }
            }
            NativeGmRecord::Action { request } => {
                let Some(request) = codec::decode_gm_action_request(&request) else {
                    continue;
                };
                let primary_gm = world
                    .get_resource::<super::session_role::NativeSessionRoleState>()
                    .is_some_and(|state| {
                        state.committed()
                            && matches!(
                                state.role(),
                                super::session_role::NativeSessionRole::StandaloneGameMaster
                                    | super::session_role::NativeSessionRole::FleetGameMaster
                            )
                    });
                let result = if primary_gm {
                    crate::gm_action::submit_local(world, request.clone())
                } else {
                    crate::gm_action::submit_native(world, request.clone())
                };
                if let Err(reason) = result {
                    let tick = world.resource::<crate::sim_tick::SimTick>().0;
                    world
                        .resource_mut::<crate::gm_action::LocalGmActionRefusals>()
                        .push(crate::gm_action::LoggedGmAction::refused_request(
                            &request, tick, reason,
                        ));
                }
            }
            _ => {}
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn sync_presence(
    surface: Res<NativeGmSurface>,
    layout: Option<Res<BridgeLayoutResource>>,
    phase: Res<State<GamePhase>>,
    mut state: ResMut<NativeGmLifecycle>,
    mut authority: ResMut<NativeGmAuthority>,
    mut roster: ResMut<GmRoster>,
    mut commands: Commands,
    mut paused: ResMut<crate::gm_action::SimulationPaused>,
    config: Option<Res<crate::world::config::WorldConfig>>,
    mut outbound: MessageWriter<crate::lobby::OutboundMessage>,
    sessions: Option<Res<crate::lobby::Sessions>>,
    starts: Option<Res<start::NativeGmStartRequests>>,
    role: Option<Res<super::session_role::NativeSessionRoleState>>,
    fleet_roster: Option<Res<crate::lockstep::FleetRoster>>,
    frozen_ship_slots: Option<Res<crate::ship_slots::FrozenShipSlots>>,
    admission: Option<Res<super::host_lobby::fleet::NativeFleetGmAdmission>>,
) {
    let primary_gm = role.as_ref().is_some_and(|state| {
        state.committed()
            && matches!(
                state.role(),
                super::session_role::NativeSessionRole::StandaloneGameMaster
                    | super::session_role::NativeSessionRole::FleetGameMaster
            )
    });
    let assigned = layout.as_ref().and_then(|l| {
        if primary_gm {
            Some(l.layout.viewscreen())
        } else {
            l.layout.game_master_monitor()
        }
    });
    if *phase.get() == GamePhase::Lobby {
        state.enabled = assigned.is_some();
        state.desired_monitor = assigned.cloned();
        authority.screen_pause = false;
        state.lost = false;
    } else if assigned.is_some() {
        // An explicit --solo launch may enter its mission before the display
        // adapter adopts the saved bridge layout on its first rendered frame.
        state.enabled = true;
        state.desired_monitor = assigned.cloned();
    }
    let present = assigned.is_some_and(|id| {
        layout
            .as_ref()
            .is_some_and(|l| l.monitors.iter().any(|m| &m.identity == id))
    });
    let failed = surface.bridge.take_failure();
    let connected = state.enabled && present && surface.bridge.live() && !surface.bridge.failed();
    if state.enabled {
        commands.insert_resource(crate::gm_projection::NativeGmPresentation);
    } else {
        commands.remove_resource::<crate::gm_projection::NativeGmPresentation>();
    }
    if !connected || failed {
        state.ready = false;
    }
    if state.enabled
        && (!present || failed || surface.bridge.failed())
        && *phase.get() == GamePhase::InProgress
        && !state.lost
    {
        paused.0 = true;
        authority.screen_pause = true;
        state.lost = true;
    }
    if connected {
        state.lost = false;
    }
    authority.connected = connected;
    let bound_operator = primary_gm
        .then(|| {
            if let Some(fleet) = fleet_roster.as_ref().filter(|fleet| !fleet.is_solo()) {
                let id = fleet.gm_operator(fleet.local())?;
                roster.operators().iter().find(|row| row.id == id).cloned()
            } else {
                // A GM must be able to Ready before the collective roster freezes.
                // Frozen membership always wins; this is only its admitted own row.
                admission
                    .as_ref()
                    .map(|admission| admission.0.clone())
                    .or_else(|| {
                        let fleet = fleet_roster.as_ref()?;
                        let id = fleet.gm_operator(fleet.local())?;
                        roster.operators().iter().find(|row| row.id == id).cloned()
                    })
            }
        })
        .flatten();
    authority.connected = connected && (!primary_gm || bound_operator.is_some());
    let operator_id = bound_operator
        .as_ref()
        .map(|row| row.id.as_str())
        .unwrap_or(NATIVE_GM_OPERATOR_ID);
    let mut rows: Vec<_> = roster
        .operators()
        .iter()
        .filter(|gm| gm.id != operator_id)
        .cloned()
        .collect();
    if state.enabled && (!primary_gm || bound_operator.is_some()) {
        rows.push(GmOperator {
            id: operator_id.into(),
            name: bound_operator
                .as_ref()
                .map_or_else(|| "GM".into(), |row| row.name.clone()),
            connected,
            ready: state.ready,
        });
    }
    let Ok(replacement) = GmRoster::try_new(rows) else {
        authority.connected = false;
        return;
    };
    if *roster != replacement {
        outbound.write(crate::lobby::OutboundMessage {
            target: crate::lobby::Target::All,
            delivery: crate::core::messages::DeliveryClass::Reliable,
            msg: crate::core::messages::ServerMessage::GmRosterChanged {
                gms: replacement.projection(),
            },
        });
        *roster = replacement;
    }
    if let Ok(json) = codec::to_json(&NativeGmMetadata {
        phase: phase.get().clone(),
        host_lobby_unavailable: recovery::host_lobby_unavailable(
            phase.get(),
            state.enabled,
            layout.as_deref(),
        ),
        local_operator_id: (state.enabled && (!primary_gm || bound_operator.is_some()))
            .then(|| operator_id.to_owned()),
        role_presets: config
            .as_ref()
            .map(|c| c.gm_role_presets.clone())
            .unwrap_or_default(),
        gms: roster.projection(),
        start_policy: start::readiness_totals(
            sessions
                .as_deref()
                .map_or_else(Default::default, |s| s.0.readiness_tally()),
            &roster,
        ),
        start_result: starts.as_deref().and_then(|s| s.last_result().cloned()),
        ship_slots: config.as_ref().map_or_else(Vec::new, |config| {
            config
                .effective_ship_slots()
                .iter()
                .map(|slot| {
                    let launched = frozen_ship_slots
                        .as_ref()
                        .and_then(|frozen| frozen.0.iter().find(|row| row.slot_id == slot.id));
                    NativeGmShipSlot {
                        id: slot.id.clone(),
                        label: slot.label.clone(),
                        default_hull: slot.default_ship.clone(),
                        state: match launched.map(|row| row.source) {
                            Some(crate::ship_slots::LaunchSource::Claimed) => "claimed",
                            Some(crate::ship_slots::LaunchSource::Backfill) => "backfill",
                            None => "empty",
                        },
                        can_backfill: *phase.get() == GamePhase::Lobby && launched.is_none(),
                    }
                })
                .collect()
        }),
    }) {
        surface.bridge.publish("metadata", json);
    }
}

#[allow(clippy::too_many_arguments)]
fn feed_projections(
    surface: Res<NativeGmSurface>,
    mut entity: MessageReader<GmEntityProjectionChanged>,
    mut activity: MessageReader<GmActivityFeedChanged>,
    mut station: MessageReader<GmStationProjectionChanged>,
    mut session: MessageReader<GmSessionChanged>,
    mut mission: MessageReader<GmMissionChanged>,
    mut spawn: MessageReader<GmSpawnChanged>,
    mut comms: MessageReader<GmCommsChanged>,
    mut attention: MessageReader<GmAttentionChanged>,
    mut health: MessageReader<GmHealthChanged>,
    mut workload: MessageReader<GmWorkloadChanged>,
) {
    macro_rules! feed {
        ($reader:ident, $channel:literal) => {
            if let Some(message) = $reader.read().last() {
                if let Ok(json) = codec::to_json(&message.payload) {
                    surface.bridge.publish($channel, json);
                }
            }
        };
    }
    feed!(entity, "gm_entity");
    feed!(activity, "gm_activity");
    feed!(station, "gm_station");
    feed!(session, "gm_session");
    feed!(mission, "gm_mission");
    feed!(spawn, "gm_spawn");
    feed!(comms, "gm_comms");
    feed!(attention, "gm_attention");
    feed!(health, "gm_health");
    feed!(workload, "gm_workload");
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
