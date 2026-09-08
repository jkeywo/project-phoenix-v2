//! Fixed-tick Force Start admission for the private native GM surface.
//!
//! The surface queues intent only. This module rechecks its authority at the
//! tick that forwards an accepted request to the existing host start applier.
//! Results use the ordinary attributed activity stream, including refusals.

use bevy::prelude::*;
use serde::Serialize;

use super::{NativeGmLifecycle, NativeGmSurface};
use crate::core::messages::GamePhase;
use crate::gm_action::{NativeGmAuthority, NATIVE_GM_OPERATOR_ID};
use crate::gm_roster::GmRoster;
use crate::lobby::start_policy::{
    evaluate_start_policy, ReadinessTally, StartGrantReason, StartGrantResult, StartGrantStatus,
    StartPolicyDecision, StartPolicyReason, StartTrigger,
};
use crate::lobby::{FleetManagedLobby, Sessions, StartGrantResults};
use crate::server::bridge::PendingForceStart;

/// One coalesced request and one presentation result, retained across surface
/// rebuilds and role changes. Sequence numbers belong to this host lifetime.
#[derive(Resource, Default)]
pub struct NativeGmStartRequests {
    pending: bool,
    sequence: u64,
    last_result: Option<StartGrantResult>,
}

impl NativeGmStartRequests {
    /// Returns false when an identical request is already waiting for a tick.
    pub fn request(&mut self) -> bool {
        !std::mem::replace(&mut self.pending, true)
    }

    pub fn last_result(&self) -> Option<&StartGrantResult> {
        self.last_result.as_ref()
    }
}

/// Private metadata for the shared readiness presentation. Invalid aggregate
/// counts remain explicit rather than displaying an overflowed ready total.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct NativeGmReadinessTotals {
    pub crew: ReadinessTally,
    pub gms: ReadinessTally,
    pub participants: Option<ReadinessTally>,
    pub connected_total: u64,
    pub ready_total: u64,
}

pub fn readiness_totals(crew: ReadinessTally, gms: &GmRoster) -> NativeGmReadinessTotals {
    let gm_tally = gms.readiness_tally();
    NativeGmReadinessTotals {
        crew,
        gms: gm_tally,
        participants: ReadinessTally::try_new(crew.connected, crew.ready)
            .and_then(|crew| crew.checked_add(gm_tally))
            .ok(),
        connected_total: u64::from(crew.connected) + u64::from(gm_tally.connected),
        ready_total: u64::from(crew.ready) + u64::from(gm_tally.ready),
    }
}

pub struct NativeGmStartPlugin;

impl Plugin for NativeGmStartPlugin {
    fn build(&self, app: &mut App) {
        use crate::authoritative::{DeclareState, StateClass};
        app.declare_state::<NativeGmStartRequests>(StateClass::Timer, "native-local-gm-workspace")
            .init_resource::<NativeGmStartRequests>()
            .add_systems(
                FixedUpdate,
                admit_force_start
                    .after(crate::native_host::world_load::NativeWorldLoadSet)
                    .before(crate::server::bridge::apply_force_start),
            );
    }
}

/// Exclusive so all checks and the forwarding latch share one world boundary;
/// no surface-state reader can be separated from its recorded outcome by a
/// different Bevy system. The existing applier still owns preload/phase writes.
pub(crate) fn admit_force_start(world: &mut World) {
    let mut requests = world.resource_mut::<NativeGmStartRequests>();
    if !std::mem::take(&mut requests.pending) {
        return;
    }
    let sequence = requests.sequence.checked_add(1);
    if let Some(sequence) = sequence {
        requests.sequence = sequence;
    }
    let (status, reason) = if sequence.is_none() {
        (
            StartGrantStatus::Refused,
            Some(StartGrantReason::InvalidGrant),
        )
    } else {
        admission(world)
    };
    if status == StartGrantStatus::Applied {
        world.resource_mut::<PendingForceStart>().0 = true;
    }
    let result = StartGrantResult {
        tick: world.resource::<crate::sim_tick::SimTick>().0,
        status,
        operator_id: Some(NATIVE_GM_OPERATOR_ID.to_owned()),
        reason,
        grant_id: sequence.map(|sequence| format!("start-{sequence}")),
    };
    world.resource_mut::<NativeGmStartRequests>().last_result = Some(result.clone());
    world.resource_mut::<StartGrantResults>().push(result);
}

fn admission(world: &World) -> (StartGrantStatus, Option<StartGrantReason>) {
    let refuse = |reason| (StartGrantStatus::Refused, Some(reason));
    let Some(managed) = world.get_resource::<FleetManagedLobby>() else {
        return refuse(StartGrantReason::UnauthorizedGrant);
    };
    // A malformed native/fleet composition must not use the standalone latch
    // to evade the fleet's canonical grant and ordering owner.
    if managed.enabled
        || world.contains_resource::<crate::lockstep::FleetLockstep>()
        || world
            .get_resource::<crate::lockstep::FleetRoster>()
            .is_some_and(|roster| !roster.is_solo() || !roster.gms().is_empty())
    {
        return refuse(StartGrantReason::UnauthorizedGrant);
    }
    if !world
        .get_resource::<NativeGmLifecycle>()
        .is_some_and(|state| state.enabled)
    {
        return refuse(StartGrantReason::UnauthorizedGrant);
    }
    let live = world
        .get_resource::<NativeGmSurface>()
        .is_some_and(|surface| surface.bridge.live() && !surface.bridge.failed());
    let placed = world
        .get_resource::<super::super::bridge_display::BridgeLayoutResource>()
        .is_some_and(|layout| {
            layout.layout.game_master_monitor().is_some_and(|monitor| {
                layout
                    .monitors
                    .iter()
                    .any(|present| &present.identity == monitor)
            })
        });
    if !live
        || !placed
        || !world
            .get_resource::<NativeGmAuthority>()
            .is_some_and(|authority| authority.connected)
    {
        return refuse(StartGrantReason::GmNotConnected);
    }
    if !world
        .get_resource::<State<GamePhase>>()
        .is_some_and(|phase| phase.get() == &GamePhase::Lobby)
        || world.get_resource::<NextState<GamePhase>>().is_some_and(
            |next| matches!(next, NextState::Pending(phase) if phase != &GamePhase::Lobby),
        )
    {
        return (
            StartGrantStatus::NoOp,
            Some(StartGrantReason::AlreadyStarted),
        );
    }
    let Some(gms) = world.get_resource::<GmRoster>() else {
        return refuse(StartGrantReason::GmNotConnected);
    };
    let crew = world
        .get_resource::<Sessions>()
        .map(|sessions| sessions.0.readiness_tally())
        .unwrap_or_default();
    let valid =
        managed.validation_passed && world.contains_resource::<crate::world::config::WorldConfig>();
    match evaluate_start_policy(
        [crew],
        gms,
        valid,
        StartTrigger::Forced {
            operator_id: NATIVE_GM_OPERATOR_ID,
        },
    ) {
        StartPolicyDecision::Start { .. } => (StartGrantStatus::Applied, None),
        StartPolicyDecision::Wait { reason } | StartPolicyDecision::Refused { reason } => {
            refuse(match reason {
                StartPolicyReason::ValidationFailed => StartGrantReason::ValidationFailed,
                StartPolicyReason::GmNotConnected => StartGrantReason::GmNotConnected,
                StartPolicyReason::InvalidReadiness => StartGrantReason::InvalidGrant,
                StartPolicyReason::NoParticipants | StartPolicyReason::NotReady => {
                    StartGrantReason::ReadinessChanged
                }
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gm_roster::GmOperator;
    use crate::native_host::bridge_display::BridgeLayoutResource;
    use crate::native_host::bridge_layout::{BridgeLayout, LayoutAction};
    use crate::native_host::bridge_profile::{identify, RawMonitor};
    use crate::native_host::panes::PaneId;

    fn fixture() -> App {
        let mut app = App::new();
        let monitors = identify(&[
            RawMonitor {
                name: Some("Viewscreen".into()),
                physical_width: 1920,
                physical_height: 1080,
                position_x: 0,
                position_y: 0,
                scale_factor: 1.0,
                primary: true,
            },
            RawMonitor {
                name: Some("GM".into()),
                physical_width: 1920,
                physical_height: 1080,
                position_x: 1920,
                position_y: 0,
                scale_factor: 1.0,
                primary: false,
            },
        ]);
        let layout = BridgeLayout::new(
            monitors.iter().map(|m| m.identity.clone()),
            Vec::new(),
            &monitors[0].identity,
        )
        .unwrap()
        .apply(&LayoutAction::SetGameMaster {
            monitor: Some(monitors[1].identity.clone()),
        })
        .unwrap();
        let bridge = super::super::bridge::NativeGmBridge::default();
        bridge.activate(PaneId(1));
        bridge.mark_live();
        let mut sessions = crate::lobby::session::SessionManager::new();
        sessions.register("crew".into(), "Ada".into()).unwrap();
        app.insert_resource(BridgeLayoutResource {
            layout,
            monitors,
            notices: Vec::new(),
        })
        .insert_resource(NativeGmSurface {
            bridge,
            url: "/gm".into(),
        })
        .init_resource::<NativeGmLifecycle>()
        .insert_resource(NativeGmAuthority {
            connected: true,
            screen_pause: false,
        })
        .insert_resource(
            GmRoster::try_new(vec![GmOperator::new(
                NATIVE_GM_OPERATOR_ID.into(),
                "GM".into(),
                true,
            )])
            .unwrap(),
        )
        .insert_resource(Sessions(sessions))
        .insert_resource(State::new(GamePhase::Lobby))
        .init_resource::<NextState<GamePhase>>()
        .init_resource::<crate::world::config::WorldConfig>()
        .init_resource::<FleetManagedLobby>()
        .init_resource::<PendingForceStart>()
        .init_resource::<StartGrantResults>()
        .insert_resource(crate::sim_tick::SimTick(17))
        .add_plugins(NativeGmStartPlugin);
        app.world_mut().resource_mut::<NativeGmLifecycle>().enabled = true;
        app
    }

    fn request(app: &mut App) -> StartGrantResult {
        app.world_mut()
            .resource_mut::<NativeGmStartRequests>()
            .request();
        app.world_mut().run_schedule(FixedUpdate);
        app.world()
            .resource::<NativeGmStartRequests>()
            .last_result()
            .unwrap()
            .clone()
    }

    #[test]
    fn connected_gm_forces_unready_crew_with_one_attributed_result() {
        let mut app = fixture();
        assert!(!app.world().resource::<Sessions>().0.all_ready());
        assert!(!app.world().resource::<GmRoster>().operators()[0].ready);
        {
            let mut state = app.world_mut().resource_mut::<NativeGmStartRequests>();
            assert!(state.request());
            for _ in 0..100 {
                assert!(!state.request());
            }
        }
        app.world_mut().run_schedule(FixedUpdate);
        assert!(app.world().resource::<PendingForceStart>().0);
        let results = app.world().resource::<StartGrantResults>();
        assert_eq!(results.iter().count(), 1);
        assert_eq!(
            results.iter().next().unwrap(),
            &StartGrantResult {
                tick: 17,
                status: StartGrantStatus::Applied,
                operator_id: Some(NATIVE_GM_OPERATOR_ID.into()),
                reason: None,
                grant_id: Some("start-1".into()),
            }
        );
        app.world_mut().run_schedule(FixedUpdate);
        assert_eq!(
            app.world().resource::<StartGrantResults>().iter().count(),
            1
        );
    }

    #[test]
    fn accepted_request_uses_the_existing_phase_applier() {
        let mut app = fixture();
        app.init_resource::<crate::lobby::LobbyOutbox>()
            .add_systems(FixedUpdate, crate::server::bridge::apply_force_start);
        assert_eq!(request(&mut app).status, StartGrantStatus::Applied);
        assert!(!app.world().resource::<PendingForceStart>().0);
        assert!(matches!(
            app.world().resource::<NextState<GamePhase>>(),
            NextState::Pending(GamePhase::InProgress)
        ));
    }

    #[test]
    fn no_world_and_failed_validation_are_terminal_refusals() {
        let mut app = fixture();
        app.world_mut()
            .remove_resource::<crate::world::config::WorldConfig>();
        let first = request(&mut app);
        assert_eq!(first.reason, Some(StartGrantReason::ValidationFailed));
        assert!(!app.world().resource::<PendingForceStart>().0);
        app.init_resource::<crate::world::config::WorldConfig>();
        app.world_mut().run_schedule(FixedUpdate);
        assert!(!app.world().resource::<PendingForceStart>().0);
        app.world_mut()
            .resource_mut::<FleetManagedLobby>()
            .validation_passed = false;
        let second = request(&mut app);
        assert_eq!(second.reason, Some(StartGrantReason::ValidationFailed));
        assert_eq!(second.grant_id.as_deref(), Some("start-2"));
    }

    #[test]
    fn surface_authority_and_monitor_loss_are_rechecked_on_the_tick() {
        for loss in 0..5 {
            let mut app = fixture();
            app.world_mut()
                .resource_mut::<NativeGmStartRequests>()
                .request();
            match loss {
                0 => app.world().resource::<NativeGmSurface>().bridge.close(),
                1 => app.world().resource::<NativeGmSurface>().bridge.fault(),
                2 => {
                    app.world_mut()
                        .resource_mut::<NativeGmAuthority>()
                        .connected = false
                }
                3 => {
                    app.world_mut()
                        .resource_mut::<BridgeLayoutResource>()
                        .monitors
                        .pop();
                }
                _ => {
                    app.world_mut().remove_resource::<NativeGmSurface>();
                }
            }
            app.world_mut().run_schedule(FixedUpdate);
            let result = app
                .world()
                .resource::<NativeGmStartRequests>()
                .last_result()
                .unwrap();
            assert_eq!(result.reason, Some(StartGrantReason::GmNotConnected));
            assert!(!app.world().resource::<PendingForceStart>().0);
        }
    }

    #[test]
    fn disabled_role_and_fleet_cannot_use_private_standalone_force() {
        for denied in 0..3 {
            let mut app = fixture();
            match denied {
                0 => app.world_mut().resource_mut::<NativeGmLifecycle>().enabled = false,
                1 => app.world_mut().resource_mut::<FleetManagedLobby>().enabled = true,
                _ => {
                    app.insert_resource(crate::lockstep::FleetLockstep(
                        crate::lockstep::LockstepSession::new(
                            crate::command_admission::HostSlot::SOLO,
                            [],
                            0,
                        ),
                    ));
                }
            }
            assert_eq!(
                request(&mut app).reason,
                Some(StartGrantReason::UnauthorizedGrant)
            );
            assert!(!app.world().resource::<PendingForceStart>().0);
        }
    }

    #[test]
    fn roster_admission_and_phase_are_rechecked() {
        let mut app = fixture();
        app.insert_resource(GmRoster::default());
        assert_eq!(
            request(&mut app).reason,
            Some(StartGrantReason::GmNotConnected)
        );
        let mut app = fixture();
        app.insert_resource(State::new(GamePhase::InProgress));
        assert_eq!(
            request(&mut app).reason,
            Some(StartGrantReason::AlreadyStarted)
        );
        assert!(!app.world().resource::<PendingForceStart>().0);
        let mut app = fixture();
        app.world_mut()
            .resource_mut::<NextState<GamePhase>>()
            .set(GamePhase::Loading);
        assert_eq!(request(&mut app).status, StartGrantStatus::NoOp);
    }

    #[test]
    fn readiness_metadata_includes_unready_stationless_crew_and_gms() {
        let app = fixture();
        let totals = readiness_totals(
            app.world().resource::<Sessions>().0.readiness_tally(),
            app.world().resource::<GmRoster>(),
        );
        assert_eq!(
            totals.crew,
            ReadinessTally {
                connected: 1,
                ready: 0
            }
        );
        assert_eq!(
            totals.gms,
            ReadinessTally {
                connected: 1,
                ready: 0
            }
        );
        assert_eq!(
            totals.participants,
            Some(ReadinessTally {
                connected: 2,
                ready: 0
            })
        );
        assert!(readiness_totals(
            ReadinessTally {
                connected: 0,
                ready: 1
            },
            &GmRoster::default()
        )
        .participants
        .is_none());
    }
}
