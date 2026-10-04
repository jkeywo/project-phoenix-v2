use super::*;
use crate::core::messages::{
    ActionCorrelationId, ActionFeedbackOutcome, AdmittedCommands, DeliveryClass, RepairTarget,
    ServerMessage, StationId, SystemControlPayload, SystemId,
};
use crate::lobby::{LobbyPlugin, Target};
use crate::ship::control_source::{ControlSource, ControlSourceResolver};
use crate::ship::test_support::{drive_one_fixed_step_per_update, TEST_TICK};
use crate::ship_plugin::{ShipConfigComponent, ShipSystemControlSources};

/// The token whose player holds the repair station in every fixture here.
/// Deliberately a distinctive string rather than `t1`: the log tests below
/// assert it appears NOWHERE in a recorded entry, and a two-character token
/// could match some unrelated substring by luck.
const HOLDER: &str = "human-session-token";

/// The fixture ship's `EntityUuid`, and therefore the [`log::ShipKey`] the
/// log must record for anything routed to it. Spawned deliberately: a ship
/// without one would exercise the unnamed-key fallback rather than the
/// production shape.
const SHIP_UUID: &str = "uuid-fixture-ship";

/// A one-station hull, so `station_for_system` resolves `repair` → the
/// `repair` station. Same shape as the `policy` unit tests use.
fn config() -> crate::ship::config::ShipConfig {
    crate::ship::config::ShipConfig::from_toml(
        r#"
[[station]]
id = "repair"
name = "Engineering"
description = "Damage control."
rank = "Ltn."

[[system]]
id = "repair"
kind = "repair_control"
station = "repair"
"#,
        &["repair_control"],
    )
    .unwrap()
}

fn sessions_with_repair_holder(token: &str) -> Sessions {
    let mut sm = crate::lobby::session::SessionManager::new();
    sm.register(token.into(), "Engineer".into()).unwrap();
    sm.set_station(token, Some(StationId("repair".into())));
    Sessions(sm)
}

fn dispatch(team_idx: u8) -> SystemControlPayload {
    SystemControlPayload::DispatchRepairTeam {
        team_idx,
        target: RepairTarget::Core,
    }
}

fn sources(source: ControlSource) -> ShipSystemControlSources {
    let mut resolver = ControlSourceResolver::new();
    resolver.set(SystemId("repair".into()), source);
    ShipSystemControlSources(resolver)
}

/// A `LocalShip` whose repair system answers to `source`, plus the
/// admission seam, on the fixed clock and driving one logical tick per
/// `update()`.
fn admission_app(source: ControlSource) -> (App, Entity) {
    let mut app = App::new();
    app.add_plugins(LobbyPlugin)
        .add_plugins(bevy::time::TimePlugin)
        .add_plugins(crate::server_app::AdmissionPlugin);
    crate::sim_tick::register_sim_tick(&mut app);
    app.insert_resource(sessions_with_repair_holder(HOLDER));
    drive_one_fixed_step_per_update(&mut app, TEST_TICK);
    let ship = app
        .world_mut()
        .spawn((
            LocalShip,
            AdmittedCommands::default(),
            ShipConfigComponent(config()),
            sources(source),
            crate::entities::spawner::EntityUuid(SHIP_UUID.into()),
        ))
        .id();
    (app, ship)
}

fn send(app: &mut App, token: &str, payload: SystemControlPayload) {
    app.world_mut()
        .resource_mut::<Messages<InboundMessage>>()
        .write(InboundMessage {
            token: token.into(),
            msg: ClientMessage::ControlSystem {
                target: SystemId("repair".into()),
                payload,
            },
        });
}

fn send_correlated(
    app: &mut App,
    token: &str,
    correlation: &str,
    target: SystemId,
    payload: SystemControlPayload,
) {
    app.world_mut()
        .resource_mut::<Messages<InboundMessage>>()
        .write(InboundMessage {
            token: token.into(),
            msg: ClientMessage::ControlSystemCorrelated {
                correlation: ActionCorrelationId::new(correlation).expect("valid test correlation"),
                target,
                payload,
            },
        });
}

#[test]
fn correlated_action_feedback_allowlist_is_exact_by_target_and_payload() {
    use crate::core::messages::{CameraView, ViewMode};
    let mut app = App::new();
    crate::server_app::add_simulation_plugins_with(
        &mut app,
        crate::server_app::SimPluginOptions {
            render: false,
            ..Default::default()
        },
    );
    app.add_plugins(crate::world::server::WorldPlugin);
    let registry = app.world().resource::<AdmittedConsumerRegistry>();
    let supports_correlated_action_feedback_for_kind =
        |target: &SystemId, payload: &SystemControlPayload, kind: Option<&str>| {
            let mut topology = config();
            topology.systems.clear();
            if let Some(kind) = kind {
                let mut instance = config().systems.remove(0);
                instance.id = target.clone();
                instance.kind = kind.into();
                topology.systems.push(instance);
            }
            registry.feedback_support(target, payload, &topology) == FeedbackSupport::Supported
        };
    let supports_correlated_action_feedback =
        |target: &SystemId, payload: &SystemControlPayload| {
            supports_correlated_action_feedback_for_kind(target, payload, None)
        };

    let red_alert = SystemControlPayload::SetRedAlert { active: true };
    let view = SystemControlPayload::SetView {
        mode: ViewMode::Camera(CameraView::new("camera_fore")),
    };
    let objective = SystemControlPayload::SetObjectivePriority { id: "o1".into() };
    let hail = SystemControlPayload::Hail {
        target_uuid: "contact-1".into(),
    };
    let response = SystemControlPayload::RespondToMessage {
        message_id: "message-1".into(),
        response_index: 0,
    };
    let clear = SystemControlPayload::ClearComms;
    let show = SystemControlPayload::ShowOnScreen {
        message_id: "message-1".into(),
    };
    let select = SystemControlPayload::SelectCommsMessage {
        message_id: "message-1".into(),
    };
    let science_target = SystemControlPayload::SetScienceTarget {
        uuid: "target".into(),
    };
    let scan = SystemControlPayload::ScanTarget {
        uuid: "target".into(),
    };
    let start_impulse = SystemControlPayload::StartImpulseCharge;
    let cancel_impulse = SystemControlPayload::CancelImpulse;
    let set_boost = SystemControlPayload::SetBoost { active: true };
    let toggle_boost = SystemControlPayload::ToggleBoost;
    let dock = SystemControlPayload::Dock;
    let undock = SystemControlPayload::Undock;
    let shield_focus = SystemControlPayload::SetShieldArcFocus { focused: true };
    let waypoint = SystemControlPayload::SetNavigationWaypoint {
        x: 10.0,
        z: -20.0,
        source_uuid: None,
    };
    let clear_waypoint = SystemControlPayload::ClearNavigationWaypoint;
    let civilian_order = SystemControlPayload::OrderCivilian {
        target: "civilian-a".into(),
        order: crate::civilian::CivilianOrder::Hold,
    };

    assert!(supports_correlated_action_feedback(
        &crate::ship::system_registry::red_alert_system_id(),
        &red_alert,
    ));
    assert!(supports_correlated_action_feedback(
        &crate::ship::system_registry::viewscreen_system_id(),
        &view,
    ));
    assert!(supports_correlated_action_feedback(
        &crate::ship::system_registry::captain_system_id(),
        &objective,
    ));
    for payload in [&hail, &response, &clear, &show] {
        assert!(supports_correlated_action_feedback(
            &crate::ship::system_registry::comms_system_id(),
            payload,
        ));
    }
    assert!(supports_correlated_action_feedback(
        &crate::ship::system_registry::sensors_system_id(),
        &science_target,
    ));
    assert!(supports_correlated_action_feedback(
        &crate::ship::system_registry::sensors_system_id(),
        &scan,
    ));
    for payload in [&start_impulse, &cancel_impulse] {
        assert!(supports_correlated_action_feedback(
            &crate::ship::system_registry::helm_impulse_system_id(),
            payload,
        ));
    }
    for payload in [&set_boost, &toggle_boost] {
        assert!(supports_correlated_action_feedback(
            &crate::ship::system_registry::helm_boost_system_id(),
            payload,
        ));
    }
    for payload in [&dock, &undock] {
        assert!(supports_correlated_action_feedback(
            &SystemId(crate::ship::system_registry::DOCK_KIND.into()),
            payload,
        ));
    }
    for (kind, payload) in [
        (
            crate::ship::system_registry::HELM_IMPULSE_KIND,
            SystemControlPayload::StartImpulseCharge,
        ),
        (
            crate::ship::system_registry::HELM_BOOST_KIND,
            SystemControlPayload::SetBoost { active: true },
        ),
        (crate::ship::system_registry::VIEWSCREEN_KIND, view.clone()),
        (crate::ship::system_registry::DOCK_KIND, dock.clone()),
    ] {
        assert!(supports_correlated_action_feedback_for_kind(
            &SystemId("designer-owned-instance".into()),
            &payload,
            Some(kind),
        ));
    }
    for (kind, payload) in [
        (
            crate::ship::system_registry::HELM_THRUST_KIND,
            SystemControlPayload::SetThrust { value: 0.4 },
        ),
        (
            crate::ship::system_registry::HELM_STEERING_KIND,
            SystemControlPayload::SetSteering { value: -0.2 },
        ),
        (
            crate::ship::system_registry::LATERAL_THRUST_KIND,
            SystemControlPayload::LateralThrustInput { lateral: 1.0 },
        ),
    ] {
        assert!(
            !supports_correlated_action_feedback_for_kind(
                &SystemId("designer-owned-instance".into()),
                &payload,
                Some(kind),
            ),
            "continuous {kind} input has no terminal consumer and must stay uncorrelated"
        );
    }
    assert!(supports_correlated_action_feedback(
        &crate::ship::system_registry::shield_arc_system_id("fore").expect("fore"),
        &shield_focus,
    ));
    for payload in [&waypoint, &clear_waypoint, &civilian_order] {
        assert!(supports_correlated_action_feedback(
            &crate::ship::system_registry::navigation_system_id(),
            payload,
        ));
    }
    for (target, payload) in [
        (
            crate::ship::system_registry::power_reactor_system_id(),
            SystemControlPayload::SetPowerGroupAllocation {
                group: crate::core::messages::PowerGroupId("helm".into()),
                level: 2,
            },
        ),
        (
            crate::ship::system_registry::repair_system_id(),
            dispatch(0),
        ),
        (
            crate::ship::system_registry::repair_system_id(),
            SystemControlPayload::SetRepairTargetPriority {
                system_id: SystemId("power-reactor".into()),
            },
        ),
        (
            crate::ship::system_registry::repair_system_id(),
            SystemControlPayload::DispatchExternalRepair,
        ),
        (
            crate::ship::system_registry::repair_system_id(),
            SystemControlPayload::RecallExternalRepair,
        ),
        (
            crate::ship::system_registry::tractor_system_id(),
            SystemControlPayload::EngageTractor,
        ),
        (
            crate::ship::system_registry::tractor_system_id(),
            SystemControlPayload::ReleaseTractor,
        ),
        (
            crate::ship::system_registry::umbilical_system_id(),
            SystemControlPayload::StartTransfer,
        ),
        (
            crate::ship::system_registry::umbilical_system_id(),
            SystemControlPayload::StopTransfer,
        ),
    ] {
        assert!(supports_correlated_action_feedback(&target, &payload));
    }

    assert!(!supports_correlated_action_feedback(
        &crate::ship::system_registry::captain_system_id(),
        &red_alert,
    ));
    assert!(!supports_correlated_action_feedback(
        &crate::ship::system_registry::viewscreen_system_id(),
        &objective,
    ));
    assert!(!supports_correlated_action_feedback(
        &crate::ship::system_registry::captain_system_id(),
        &waypoint,
    ));
    assert!(!supports_correlated_action_feedback(
        &crate::ship::system_registry::viewscreen_system_id(),
        &civilian_order,
    ));
    assert!(!supports_correlated_action_feedback(
        &SystemId("repair".into()),
        &SystemControlPayload::SetRepairPriority {
            team_idx: 0,
            priority: 1,
        },
    ));
    assert!(!supports_correlated_action_feedback(
        &crate::ship::system_registry::repair_system_id(),
        &SystemControlPayload::SetPowerGroupAllocation {
            group: crate::core::messages::PowerGroupId("helm".into()),
            level: 2,
        },
    ));
    assert!(!supports_correlated_action_feedback(
        &crate::ship::system_registry::comms_system_id(),
        &select,
    ));
    assert!(!supports_correlated_action_feedback(
        &crate::ship::system_registry::captain_system_id(),
        &hail,
    ));
    assert!(!supports_correlated_action_feedback(
        &crate::ship::system_registry::captain_system_id(),
        &scan,
    ));
    assert!(!supports_correlated_action_feedback(
        &SystemId("shield-arc-FORE".into()),
        &shield_focus,
    ));
    assert!(!supports_correlated_action_feedback(
        &SystemId("shield-arc-".into()),
        &shield_focus,
    ));
    assert!(!supports_correlated_action_feedback(
        &crate::ship::system_registry::shield_arc_system_id("fore").expect("fore"),
        &cancel_impulse,
    ));
    assert!(!supports_correlated_action_feedback(
        &crate::ship::system_registry::helm_boost_system_id(),
        &start_impulse,
    ));
    assert!(!supports_correlated_action_feedback(
        &crate::ship::system_registry::helm_impulse_system_id(),
        &set_boost,
    ));
    assert!(!supports_correlated_action_feedback(
        &SystemId("berthing-clamps".into()),
        &dock,
    ));
    assert!(!supports_correlated_action_feedback(
        &SystemId(crate::ship::system_registry::DOCK_KIND.into()),
        &view,
    ));
    assert!(!supports_correlated_action_feedback_for_kind(
        &SystemId("designer-owned-instance".into()),
        &dock,
        Some(crate::ship::system_registry::HELM_THRUST_KIND),
    ));
}

fn admitted(app: &mut App, ship: Entity) -> Vec<SystemControlPayload> {
    app.world()
        .entity(ship)
        .get::<AdmittedCommands>()
        .expect("the ship has an AdmittedCommands")
        .0
        .iter()
        .map(|c| c.payload.clone())
        .collect()
}

fn command_log(app: &App) -> &log::CommandLog {
    app.world().resource::<log::CommandLog>()
}

/// The baseline: an accepted command applies on the tick it was admitted
/// on, and the log records it stamped with that same tick.
///
/// `SimTick` advances in `FixedLast`, so the step that admits reads tick 0
/// and the counter reads 1 once the frame is over — which is why the
/// recorded stamp is 0 while `SimTick` afterwards is 1.
#[test]
fn an_admitted_command_applies_on_the_tick_it_is_stamped_for() {
    let (mut app, ship) = admission_app(ControlSource::Human);
    send(&mut app, HOLDER, dispatch(0));
    app.update();

    assert_eq!(admitted(&mut app, ship), vec![dispatch(0)]);
    let entries = command_log(&app).entries();
    assert_eq!(entries.len(), 1, "the accepted command must be recorded");
    assert_eq!(entries[0].tick, 0, "stamped with the tick it applied on");
    assert_eq!(entries[0].payload, dispatch(0));
    assert_eq!(
        entries[0].ship,
        log::ShipKey(SHIP_UUID.into()),
        "the log carries the ROUTED SHIP's uuid, which is what makes the \
             entry re-routable on replay — and not the sender's session token, \
             which is a bearer credential"
    );
    assert!(
        !format!("{entries:?}").contains(HOLDER),
        "the holder's session token must not appear anywhere in the log"
    );
    assert_eq!(app.world().resource::<crate::sim_tick::SimTick>().0, 1);
}

/// The vellum contract's third rule, at the phoenix seam: a command the
/// authority gate refuses reaches neither `AdmittedCommands` nor the log.
#[test]
fn a_rejected_command_never_enters_the_log() {
    let (mut app, ship) = admission_app(ControlSource::Human);
    send(&mut app, "intruder", dispatch(0));
    app.update();

    assert!(admitted(&mut app, ship).is_empty());
    assert!(
        command_log(&app).is_empty(),
        "a refused command in the log would refuse again on replay, where \
             refusal is a hard error"
    );
    assert!(app.world().resource::<log::PendingCommands>().is_empty());
}

#[test]
fn crew_spectator_dead_hull_refuses_controls_while_live_crew_still_controls_own_ship() {
    let (mut dead, wreck) = admission_app(ControlSource::Human);
    let (mut live, ship) = admission_app(ControlSource::Human);
    let id = SystemId("repair".into());
    let mut hull = crate::ship::damage::SystemHull::from_config(&[(id.clone(), 10.0)]);
    hull.set_hp(&id, 0.0);
    dead.world_mut()
        .entity_mut(wreck)
        .insert(crate::entities::spawner::EntitySystemHull(hull));
    send(&mut dead, HOLDER, dispatch(0));
    send(&mut live, HOLDER, dispatch(0));
    dead.update();
    live.update();
    assert!(admitted(&mut dead, wreck).is_empty());
    assert!(command_log(&dead).is_empty());
    assert_eq!(admitted(&mut live, ship), vec![dispatch(0)]);
    let policy = dead
        .world()
        .get::<ShipSystemControlSources>(wreck)
        .unwrap()
        .0
        .policy_for(&id);
    assert!(!policy.accept_human_input && !policy.operate_ai && !policy.coordinate);
    // A repaired availability flag or a reconnect cannot override the hull.
    dead.world_mut()
        .get_mut::<ShipSystemControlSources>(wreck)
        .unwrap()
        .0
        .set_offline(id, false);
    dead.world_mut()
        .resource_mut::<Sessions>()
        .0
        .reconnect(HOLDER);
    send(&mut dead, HOLDER, dispatch(0));
    dead.update();
    assert!(admitted(&mut dead, wreck).is_empty());
}

#[test]
fn correlated_non_red_alert_requests_are_reliably_refused_to_the_origin() {
    let (mut app, ship) = admission_app(ControlSource::Human);
    let mut cursor = app
        .world()
        .resource::<Messages<OutboundMessage>>()
        .get_cursor();
    send_correlated(
        &mut app,
        HOLDER,
        "unsupported-feedback",
        SystemId("repair".into()),
        SystemControlPayload::SetPowerGroupAllocation {
            group: crate::core::messages::PowerGroupId("shields".into()),
            level: 1,
        },
    );
    app.update();

    assert!(admitted(&mut app, ship).is_empty());
    assert!(command_log(&app).is_empty());
    let outbound = app.world().resource::<Messages<OutboundMessage>>();
    let feedback: Vec<_> = cursor.read(outbound).collect();
    assert!(feedback.iter().any(|message| {
        message.target == Target::Token(HOLDER.into())
            && message.delivery == DeliveryClass::Reliable
            && matches!(
                &message.msg,
                ServerMessage::ActionFeedback {
                    correlation,
                    outcome: ActionFeedbackOutcome::Refused,
                } if correlation.as_str() == "unsupported-feedback"
            )
    }));
}

#[test]
fn correlated_continuous_helm_axes_are_refused_once_before_admission() {
    let (mut app, ship) = admission_app(ControlSource::Human);
    let helm_axes = crate::ship::config::ShipConfig::from_toml(
        r#"
[[station]]
id = "repair"
name = "Helm"
description = "Flight control."
rank = "Ltn."

[[system]]
id = "port-main-drive"
kind = "helm_thrust"
station = "repair"

[[system]]
id = "yaw-ring"
kind = "helm_steering"
station = "repair"

[[system]]
id = "translation-ring"
kind = "lateral_thrust"
station = "repair"
"#,
        &[
            crate::ship::system_registry::HELM_THRUST_KIND,
            crate::ship::system_registry::HELM_STEERING_KIND,
            crate::ship::system_registry::LATERAL_THRUST_KIND,
        ],
    )
    .expect("continuous Helm axis fixture is valid");
    app.world_mut()
        .entity_mut(ship)
        .insert(ShipConfigComponent(helm_axes));
    let mut cursor = app
        .world()
        .resource::<Messages<OutboundMessage>>()
        .get_cursor();

    for (correlation, target, payload) in [
        (
            "continuous-thrust",
            "port-main-drive",
            SystemControlPayload::SetThrust { value: 0.5 },
        ),
        (
            "continuous-steering",
            "yaw-ring",
            SystemControlPayload::SetSteering { value: -0.25 },
        ),
        (
            "continuous-lateral",
            "translation-ring",
            SystemControlPayload::LateralThrustInput { lateral: 1.0 },
        ),
    ] {
        send_correlated(
            &mut app,
            HOLDER,
            correlation,
            SystemId(target.into()),
            payload,
        );
    }
    app.update();

    assert!(admitted(&mut app, ship).is_empty());
    assert!(command_log(&app).is_empty());
    assert!(app.world().resource::<log::PendingCommands>().is_empty());
    let outbound = app.world().resource::<Messages<OutboundMessage>>();
    let feedback: Vec<_> = cursor.read(outbound).collect();
    for correlation in [
        "continuous-thrust",
        "continuous-steering",
        "continuous-lateral",
    ] {
        let matching: Vec<_> = feedback
            .iter()
            .filter(|message| {
                message.target == Target::Token(HOLDER.into())
                    && message.delivery == DeliveryClass::Reliable
                    && matches!(
                        &message.msg,
                        ServerMessage::ActionFeedback {
                            correlation: actual,
                            ..
                        } if actual.as_str() == correlation
                    )
            })
            .collect();
        assert_eq!(
            matching.len(),
            1,
            "{correlation} must receive exactly one terminal refusal"
        );
        assert!(matches!(
            &matching[0].msg,
            ServerMessage::ActionFeedback {
                outcome: ActionFeedbackOutcome::Refused,
                ..
            }
        ));
    }
}

#[test]
fn correlated_red_alert_payload_on_wrong_target_is_refused_before_admission() {
    let (mut app, ship) = admission_app(ControlSource::Human);
    let mut cursor = app
        .world()
        .resource::<Messages<OutboundMessage>>()
        .get_cursor();
    // The local console bypasses station authority and `repair` is a real
    // system in this fixture.  Without the exact-pair protocol gate this
    // malformed request would therefore be admitted and logged.
    send_correlated(
        &mut app,
        crate::console_bridge::LOCAL_CONSOLE_TOKEN,
        "wrong-red-alert-target",
        SystemId("repair".into()),
        SystemControlPayload::SetRedAlert { active: true },
    );
    app.update();

    assert!(admitted(&mut app, ship).is_empty());
    assert!(command_log(&app).is_empty());
    assert!(app.world().resource::<log::PendingCommands>().is_empty());
    let outbound = app.world().resource::<Messages<OutboundMessage>>();
    let feedback: Vec<_> = cursor.read(outbound).collect();
    assert!(feedback.iter().any(|message| {
        message.target == Target::Token(crate::console_bridge::LOCAL_CONSOLE_TOKEN.into())
            && message.delivery == DeliveryClass::Reliable
            && matches!(
                &message.msg,
                ServerMessage::ActionFeedback {
                    correlation,
                    outcome: ActionFeedbackOutcome::Refused,
                } if correlation.as_str() == "wrong-red-alert-target"
            )
    }));
}

/// Two commands arriving in one tick keep their arrival order in both the
/// admitted buffer and the log — the log *is* the order.
#[test]
fn arrival_order_within_a_tick_is_the_recorded_order() {
    let (mut app, ship) = admission_app(ControlSource::Human);
    send(&mut app, HOLDER, dispatch(0));
    send(&mut app, HOLDER, dispatch(1));
    app.update();

    assert_eq!(admitted(&mut app, ship), vec![dispatch(0), dispatch(1)]);
    let recorded: Vec<SystemControlPayload> = command_log(&app)
        .entries()
        .iter()
        .map(|e| e.payload.clone())
        .collect();
    assert_eq!(recorded, vec![dispatch(0), dispatch(1)]);
}

/// The future-tick path, which a zero `CommandDelay` otherwise hides: with
/// a delay of two ticks the command is queued at once, stamped for tick 2,
/// and neither applies nor is recorded until tick 2 comes round.
///
/// This is the test that makes "logged commands carry the tick they apply
/// on, and apply on that tick" a claim about the plumbing rather than a
/// tautology about a delay of nought — and, since #1116, that the log is
/// the APPLIED sequence rather than the accepted one. A host writes an
/// entry as the command lands, so two hosts in a fleet write the same log
/// even though each accepted its own crew's commands locally and the
/// other's off a socket, at different moments and in different orders.
#[test]
fn a_delayed_command_waits_for_the_tick_it_is_stamped_for() {
    let (mut app, ship) = admission_app(ControlSource::Human);
    app.insert_resource(log::CommandDelay(2));
    send(&mut app, HOLDER, dispatch(0));

    // Tick 0: queued for tick 2, applied nowhere and written down nowhere.
    app.update();
    assert!(
        admitted(&mut app, ship).is_empty(),
        "a command stamped for tick 2 must not apply on tick 0"
    );
    assert!(
        command_log(&app).is_empty(),
        "and must not be in the log either: the log is what HAS applied, \
             so an entry here would claim a tick 2 that has not happened"
    );
    assert_eq!(app.world().resource::<log::PendingCommands>().len(), 1);

    // Tick 1: still waiting.
    app.update();
    assert!(admitted(&mut app, ship).is_empty());
    assert!(command_log(&app).is_empty());

    // Tick 2: applies, on exactly the tick it was stamped for, and is
    // recorded as it lands.
    app.update();
    assert_eq!(app.world().resource::<crate::sim_tick::SimTick>().0, 3);
    assert_eq!(admitted(&mut app, ship), vec![dispatch(0)]);
    assert!(app.world().resource::<log::PendingCommands>().is_empty());
    assert_eq!(command_log(&app).entries()[0].tick, 2);

    // And applying once records once.
    app.update();
    assert_eq!(
        command_log(&app).entries().len(),
        1,
        "applying must not record a second time"
    );
}

/// Ticks are recorded in non-decreasing order across a run — the smoke-level
/// property a replay driver depends on.
#[test]
fn recorded_ticks_never_go_backwards() {
    let (mut app, _) = admission_app(ControlSource::Human);
    for team in 0..4 {
        send(&mut app, HOLDER, dispatch(team));
        app.update();
    }
    let ticks: Vec<u64> = command_log(&app).entries().iter().map(|e| e.tick).collect();
    assert_eq!(ticks, vec![0, 1, 2, 3]);
    assert!(command_log(&app).ticks_are_monotonic());
}

/// What the probe below saw *inside* the fixed step, per step.
#[derive(Resource, Default)]
struct SameStepWitness {
    /// One entry per fixed step: how many of that step's `AdmittedCommands`
    /// carried the AI decider's emission at the moment the paired applier
    /// would have run.
    seen_per_step: Vec<usize>,
}

/// Option A's load-bearing half: an AI decider's emission lands in
/// `AdmittedCommands` in the same tick it was decided — the guarantee
/// `emit_ai_command` documents — and is *not* recorded.
///
/// It is absent because a replay re-derives it: the log plus the seed
/// regenerate this decision, so logging it would apply it twice.
///
/// # Why the emitter is scheduled rather than called
///
/// The point at issue is "same *tick*", and a tick is a run of the fixed
/// schedule. Driving `update()` and then poking the emitter in with
/// `run_system_cached` proves something weaker and differently shaped: it
/// reads `AdmittedCommands` from *outside* any fixed step, where the buffer
/// happens to survive between steps, so it would keep passing even if
/// admission had cleared the emission away mid-step. Here the emitter and
/// the probe are both registered in `FixedUpdate` `.after(AdmissionSet)`
/// and chained, so the probe stands exactly where the real paired applier
/// stands — after the decider, inside the same step, after that step's
/// admission has done its clear. The assertion is on what the probe
/// recorded, not on what survived to the end of the frame.
#[test]
fn an_ai_emission_keeps_its_same_tick_guarantee_and_stays_out_of_the_log() {
    fn emit(
        mut ships: Query<(
            &ShipSystemControlSources,
            &mut AdmittedCommands,
            Option<&ShipConfigComponent>,
        )>,
        sessions: Res<Sessions>,
    ) {
        for (sources, mut admitted, config) in ships.iter_mut() {
            assert!(
                ai_emit::emit_ai_command(
                    None,
                    SystemId("repair".into()),
                    SystemControlPayload::DispatchRepairTeam {
                        team_idx: 7,
                        target: RepairTarget::Core,
                    },
                    sources,
                    &sessions,
                    config,
                    &mut admitted,
                ),
                "the AI token must be admitted on an AI-controlled system"
            );
        }
    }

    /// Stands where the paired applier stands: same fixed step, after the
    /// decider, after admission.
    fn probe(ships: Query<&AdmittedCommands>, mut witness: ResMut<SameStepWitness>) {
        let seen = ships
            .iter()
            .flat_map(|a| a.0.iter())
            .filter(|c| {
                matches!(
                    c.payload,
                    SystemControlPayload::DispatchRepairTeam { team_idx: 7, .. }
                )
            })
            .count();
        witness.seen_per_step.push(seen);
    }

    let (mut app, _) = admission_app(ControlSource::Ai);
    app.init_resource::<SameStepWitness>()
        .add_systems(FixedUpdate, (emit, probe).chain().after(AdmissionSet));

    // Three steps, so the claim is about every tick rather than about one
    // lucky one — a decider that emitted into a buffer the next tick's
    // admission wiped before the applier ran would show a zero here.
    for _ in 0..3 {
        app.update();
    }

    let seen = &app.world().resource::<SameStepWitness>().seen_per_step;
    assert_eq!(seen.len(), 3, "precondition: three fixed steps ran");
    assert!(
        seen.iter().all(|&n| n == 1),
        "every fixed step must show the decider's emission still in \
             AdmittedCommands when the applier runs, in that same step — got \
             {seen:?}"
    );
    assert!(
        command_log(&app).is_empty(),
        "an AI emission never crossed the network boundary, so the log \
             must not carry it — replay re-derives it from the seed"
    );
}

/// The recorder keys on the *seam*, not on the origin: a command that
/// arrives over the inbound boundary is recorded whatever its token looks
/// like. Branching on `ai:` here would be exactly the human-vs-AI branch
/// AGENTS.md constraint 6 forbids, and it would drop a remote peer's
/// orders — which this instance cannot re-derive — from the log.
#[test]
fn the_recorder_does_not_ask_where_an_inbound_command_came_from() {
    let (mut app, ship) = admission_app(ControlSource::Ai);
    send(&mut app, ai_emit::AI_BACKFILL_TOKEN, dispatch(0));
    app.update();

    assert_eq!(admitted(&mut app, ship), vec![dispatch(0)]);
    assert_eq!(
        command_log(&app).entries().len(),
        1,
        "an inbound command is recorded on arrival, not on its token shape"
    );
}

#[test]
fn installed_owners_settle_authored_actions_once_after_delayed_admission() {
    use crate::ship::system_registry as sr;
    let mut registry_app = App::new();
    crate::server_app::add_simulation_plugins_with(
        &mut registry_app,
        crate::server_app::SimPluginOptions {
            render: false,
            ..Default::default()
        },
    );
    registry_app.add_plugins(crate::world::server::WorldPlugin);
    let registry = registry_app
        .world_mut()
        .remove_resource::<AdmittedConsumerRegistry>()
        .unwrap();
    let (mut app, ship) = admission_app(ControlSource::Human);
    app.insert_resource(registry)
        .insert_resource(log::CommandDelay(2));
    app.add_systems(
        FixedUpdate,
        (
            crate::ship::helm_admission::process_helm_inputs,
            crate::dock::server::handle_dock_commands,
            crate::console::captain::server::handle_set_view,
        )
            .after(AdmissionSet),
    );
    let mut topology = config();
    topology.systems.clear();
    let mut resolver = ControlSourceResolver::new();
    for (id, kind) in [
        ("custom-impulse", sr::HELM_IMPULSE_KIND),
        ("custom-boost", sr::HELM_BOOST_KIND),
        ("custom-dock", sr::DOCK_KIND),
        ("custom-view", sr::VIEWSCREEN_KIND),
        ("custom-captain", sr::CAPTAIN_KIND),
    ] {
        let mut instance = config().systems.remove(0);
        instance.id = SystemId(id.into());
        instance.kind = kind.into();
        resolver.set(instance.id.clone(), ControlSource::Human);
        topology.systems.push(instance);
    }
    let mut dock = crate::dock::server::DockControl::new(
        SystemId("custom-dock".into()),
        crate::dock::mating::DockConfig {
            range: 200.0,
            engage_distance: 400.0,
            approach_speed: 60.0,
            mate_tolerance: 4.0,
            undock_clear_distance: 120.0,
            min_power_level: 2,
        },
        crate::core::messages::PowerGroupId("dock".into()),
    );
    dock.available_target = Some("berth".into());
    app.world_mut().entity_mut(ship).insert((
        ShipConfigComponent(topology),
        ShipSystemControlSources(resolver),
        Transform::default(),
        dock,
        crate::ship::helm::ImpulseCommand::default(),
        crate::ship::helm::BoostCommand::default(),
        crate::ship::components::BoostConfigResource {
            enabled: true,
            ..Default::default()
        },
        crate::server_app::ShipBoost::default(),
        crate::ship::state::ShipViewMode::default(),
    ));
    let mut cursor = app
        .world()
        .resource::<Messages<OutboundMessage>>()
        .get_cursor();
    for (correlation, target, payload) in [
        (
            "impulse",
            "custom-impulse",
            SystemControlPayload::StartImpulseCharge,
        ),
        (
            "boost",
            "custom-boost",
            SystemControlPayload::SetBoost { active: true },
        ),
        ("dock", "custom-dock", SystemControlPayload::Dock),
        (
            "view",
            "custom-view",
            SystemControlPayload::SetView {
                mode: crate::core::messages::ViewMode::Camera(
                    crate::core::messages::CameraView::new("camera_fore"),
                ),
            },
        ),
    ] {
        send_correlated(
            &mut app,
            HOLDER,
            correlation,
            SystemId(target.into()),
            payload,
        );
    }
    app.update();
    assert!(admitted(&mut app, ship).is_empty());
    assert!(command_log(&app).is_empty());
    assert!(!cursor
        .read(app.world().resource::<Messages<OutboundMessage>>())
        .any(|message| matches!(message.msg, ServerMessage::ActionFeedback { .. })));
    app.update();
    app.update();
    let feedback: Vec<_> = cursor
        .read(app.world().resource::<Messages<OutboundMessage>>())
        .cloned()
        .collect();
    for correlation in ["impulse", "boost", "dock", "view"] {
        let outcomes: Vec<_> = feedback
            .iter()
            .filter_map(|message| match &message.msg {
                ServerMessage::ActionFeedback {
                    correlation: actual,
                    outcome,
                } if actual.as_str() == correlation => {
                    assert_eq!(message.target, Target::Token(HOLDER.into()));
                    assert_eq!(message.delivery, DeliveryClass::Reliable);
                    Some(*outcome)
                }
                _ => None,
            })
            .collect();
        assert_eq!(
            outcomes,
            vec![ActionFeedbackOutcome::Applied],
            "{correlation}"
        );
    }
    assert_eq!(command_log(&app).entries().len(), 4);
    app.update();
    assert!(!cursor
        .read(app.world().resource::<Messages<OutboundMessage>>())
        .any(|message| matches!(message.msg, ServerMessage::ActionFeedback { .. })));
    app.insert_resource(log::CommandDelay(0));
    app.world_mut()
        .entity_mut(ship)
        .remove::<crate::ship::helm::ImpulseCommand>();
    app.world_mut()
        .entity_mut(ship)
        .remove::<crate::ship::helm::BoostCommand>();
    app.world_mut()
        .entity_mut(ship)
        .remove::<crate::ship::state::ShipViewMode>();
    for (correlation, target, payload) in [
        (
            "impulse-refused",
            "custom-impulse",
            SystemControlPayload::StartImpulseCharge,
        ),
        (
            "boost-refused",
            "custom-boost",
            SystemControlPayload::SetBoost { active: true },
        ),
        ("dock-refused", "custom-dock", SystemControlPayload::Dock),
        (
            "view-refused",
            "custom-view",
            SystemControlPayload::SetView {
                mode: crate::core::messages::ViewMode::Camera(
                    crate::core::messages::CameraView::new("camera_fore"),
                ),
            },
        ),
    ] {
        send_correlated(
            &mut app,
            HOLDER,
            correlation,
            SystemId(target.into()),
            payload,
        );
    }
    app.update();
    let refused: Vec<_> = cursor
        .read(app.world().resource::<Messages<OutboundMessage>>())
        .cloned()
        .collect();
    for correlation in [
        "impulse-refused",
        "boost-refused",
        "dock-refused",
        "view-refused",
    ] {
        let matches: Vec<_> = refused.iter().filter(|message| matches!(&message.msg,
            ServerMessage::ActionFeedback { correlation: actual, outcome: ActionFeedbackOutcome::Refused }
                if actual.as_str() == correlation)).collect();
        assert_eq!(matches.len(), 1, "{correlation}");
        assert_eq!(matches[0].target, Target::Token(HOLDER.into()));
        assert_eq!(matches[0].delivery, DeliveryClass::Reliable);
    }
    app.world_mut().entity_mut(ship).insert((
        crate::ship::helm::ImpulseCommand::default(),
        crate::ship::helm::BoostCommand::default(),
    ));
    app.world_mut()
        .resource_mut::<Messages<InboundMessage>>()
        .write(InboundMessage {
            token: HOLDER.into(),
            msg: ClientMessage::ControlSystem {
                target: SystemId("custom-boost".into()),
                payload: SystemControlPayload::SetBoost { active: false },
            },
        });
    app.world_mut()
        .get_mut::<ShipSystemControlSources>(ship)
        .unwrap()
        .0
        .set(SystemId("custom-impulse".into()), ControlSource::Ai);
    app.world_mut()
        .resource_mut::<Messages<InboundMessage>>()
        .write(InboundMessage {
            token: ai_emit::AI_BACKFILL_TOKEN.into(),
            msg: ClientMessage::ControlSystem {
                target: SystemId("custom-impulse".into()),
                payload: SystemControlPayload::CancelImpulse,
            },
        });
    app.update();
    assert_eq!(admitted(&mut app, ship).len(), 2);
    assert!(
        !app.world()
            .get::<crate::ship::helm::BoostCommand>(ship)
            .unwrap()
            .0
    );
    assert_eq!(
        app.world()
            .get::<crate::ship::helm::ImpulseCommand>(ship)
            .unwrap()
            .0,
        crate::ship::impulse::ImpulsePhase::Idle
    );
    assert!(!cursor
        .read(app.world().resource::<Messages<OutboundMessage>>())
        .any(|message| matches!(message.msg, ServerMessage::ActionFeedback { .. })));
}

#[test]
fn missing_or_ambiguous_feedback_owner_refuses_before_queue_and_log() {
    use crate::core::messages::SystemControlPayloadDiscriminants as Payload;
    for ambiguous in [false, true] {
        let (mut app, ship) = admission_app(ControlSource::Human);
        if ambiguous {
            let mut registry = app.world_mut().resource_mut::<AdmittedConsumerRegistry>();
            registry.register(
                ConsumerMatcher::exact("repair_control", "repair").with_feedback(
                    FeedbackAddress::MatcherSpelling,
                    &[Payload::DispatchRepairTeam],
                ),
            );
            registry.register(ConsumerMatcher::kind("repair_control").with_feedback(
                FeedbackAddress::DeclaredKindOrCanonical("repair"),
                &[Payload::DispatchRepairTeam],
            ));
        }
        let mut cursor = app
            .world()
            .resource::<Messages<OutboundMessage>>()
            .get_cursor();
        send_correlated(
            &mut app,
            HOLDER,
            "missing-or-ambiguous",
            SystemId("repair".into()),
            dispatch(0),
        );
        app.update();
        assert!(admitted(&mut app, ship).is_empty());
        assert!(command_log(&app).is_empty());
        assert!(app.world().resource::<log::PendingCommands>().is_empty());
        let replies: Vec<_> = cursor.read(app.world().resource::<Messages<OutboundMessage>>()).filter(|message| matches!(&message.msg, ServerMessage::ActionFeedback { correlation, outcome: ActionFeedbackOutcome::Refused } if correlation.as_str() == "missing-or-ambiguous")).collect();
        assert_eq!(replies.len(), 1);
        assert_eq!(replies[0].target, Target::Token(HOLDER.into()));
        assert_eq!(replies[0].delivery, DeliveryClass::Reliable);
    }
}
