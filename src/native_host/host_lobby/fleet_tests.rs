use super::*;

#[derive(Clone, Default)]
struct TestWire(std::sync::Arc<std::sync::Mutex<(Vec<String>, bool)>>);
impl RelaySocket for TestWire {
    fn poll(&mut self) -> Vec<String> {
        assert!(!self.0.lock().unwrap().1, "closed primary was polled");
        Vec::new()
    }
    fn send(&mut self, text: String) {
        self.0.lock().unwrap().0.push(text);
    }
    fn is_open(&self) -> bool {
        !self.0.lock().unwrap().1
    }
    fn close(&mut self) {
        self.0.lock().unwrap().1 = true;
    }
}
#[test]
fn native_gm_reconnect_record_enters_the_authoritative_pause_transaction() {
    use crate::command_admission::HostSlot;
    use crate::gm_join::{GmJoinProgress, GmJoinRefusal, GmJoinRuntime};
    use crate::lockstep::{FleetGm, FleetLockstep, FleetRoster, FleetShip};
    for (operator, departed, expected) in [
        (
            "wrong-gm",
            true,
            Some(GmJoinRefusal::ReconnectIdentityMismatch),
        ),
        ("gm-2", false, Some(GmJoinRefusal::ReconnectStillConnected)),
        ("gm-2", true, None),
    ] {
        let mut world = wire_world();
        world.insert_resource(
            FleetRoster::with_participants_and_gms(
                vec![FleetShip::new(HostSlot(1))],
                vec![HostSlot(1), HostSlot(2)],
                vec![FleetGm {
                    host: HostSlot(2),
                    operator_id: "gm-2".into(),
                }],
                HostSlot(1),
                HostSlot(1),
            )
            .unwrap(),
        );
        let mut session = FleetLockstep(crate::lockstep::LockstepSession::new(
            HostSlot(1),
            [HostSlot(1), HostSlot(2)],
            2,
        ));
        if departed {
            session.depart(HostSlot(2));
        }
        world.insert_resource(session);
        world.init_resource::<crate::lockstep::MeshOutbox>();
        world.init_resource::<GmJoinRuntime>();
        world.insert_resource(crate::sim_tick::SimTick(10));
        world.insert_resource(crate::save_slots_lifecycle::SaveScenario(
            "assets/worlds/probe_fleet_six_peer.toml".into(),
        ));
        let raw = serde_json::json!({"kind":"fleet_begin_gm_join","id":7,"join_kind":"reconnect",
                "approved_by":1,"candidate_host":2,"operator_id":operator})
        .to_string();
        let record = HostLobbyRecord::decode(&raw)
            .expect("native owner must accept typed GM reconnect requests");
        assert!(world.resource_mut::<NativeFleetEvents>().record(&record));
        apply_events(&mut world);
        match expected {
            Some(reason) => assert_eq!(
                world.resource::<GmJoinRuntime>().progress(),
                &GmJoinProgress::Refused {
                    id: crate::gm_join::GmJoinId(7),
                    reason
                }
            ),
            None => {
                assert!(matches!(
                    world.resource::<GmJoinRuntime>().progress(),
                    GmJoinProgress::AwaitingPause { .. }
                ));
                let frames = world.resource_mut::<crate::lockstep::MeshOutbox>().drain();
                assert!(
                    matches!(frames.as_slice(), [crate::lockstep::MeshFrame::GmJoin(crate::gm_join::GmJoinFrame::Pause(approval))]
                        if approval.candidate.operator_id == "gm-2" && approval.kind == crate::gm_join::GmJoinKind::Reconnect)
                );
            }
        }
    }
}

fn wire_world() -> World {
    let mut world = World::new();
    world.init_resource::<NativeFleetEvents>();
    world.init_resource::<NativeFleetPublication>();
    world.init_resource::<NativeFleetRoleWires>();
    world
}
#[test]
fn generation_scoped_wire_send_and_close_do_not_cross_role_sockets() {
    let mut world = wire_world();
    let initial = TestWire::default();
    let replacement = TestWire::default();
    world.insert_resource(NativeFleetWire(Box::new(initial.clone())));
    world
        .resource_mut::<NativeFleetRoleWires>()
        .sockets
        .insert(1, Box::new(replacement.clone()));
    world.resource_mut::<NativeFleetEvents>().0 = vec![
        NativeFleetEvent::WireSend {
            generation: 0,
            frame: "join".into(),
        },
        NativeFleetEvent::WireSend {
            generation: 1,
            frame: "host".into(),
        },
        NativeFleetEvent::WireClose(0),
        NativeFleetEvent::WireSend {
            generation: 0,
            frame: "stale".into(),
        },
        NativeFleetEvent::WireSend {
            generation: 1,
            frame: "live".into(),
        },
    ];
    apply_events(&mut world);
    assert_eq!(*initial.0.lock().unwrap(), (vec!["join".to_string()], true));
    assert_eq!(
        *replacement.0.lock().unwrap(),
        (vec!["host".to_string(), "live".to_string()], false)
    );
    // Reused generations are rejected before dial.
    world.resource_mut::<NativeFleetRoleWires>().last_generation = 1;
    open_role_wire(&mut world, 1, "host");
    assert!(world.resource::<NativeFleetRoleWires>().opening.is_empty());
}
#[test]
fn surface_reload_is_terminal_and_old_primary_is_never_polled_or_sent() {
    let mut world = wire_world();
    let primary = TestWire::default();
    world.insert_resource(NativeFleetWire(Box::new(primary.clone())));
    world.resource_mut::<NativeFleetEvents>().0 = vec![
        NativeFleetEvent::WireAdopt,
        NativeFleetEvent::WireAdopt,
        NativeFleetEvent::WireSend {
            generation: 0,
            frame: "stale".into(),
        },
    ];
    apply_events(&mut world);
    assert!(world.resource::<NativeFleetRoleWires>().terminal);
    assert_eq!(*primary.0.lock().unwrap(), (Vec::<String>::new(), true));
    // A closed primary is skipped even if it still has unread old frames.
    world.resource_mut::<NativeFleetPublication>().configured = true;
    poll_wire(&mut world);
    open_role_wire(&mut world, 1, "host");
    assert!(world.resource::<NativeFleetRoleWires>().opening.is_empty());
    assert_eq!(world.resource::<NativeFleetRoleWires>().last_generation, 0);
}
#[test]
fn active_and_outstanding_role_sockets_share_one_two_socket_budget() {
    let mut world = wire_world();
    world.insert_resource(NativeFleetWire(Box::new(TestWire::default())));
    world
        .resource_mut::<NativeFleetRoleWires>()
        .sockets
        .insert(1, Box::new(TestWire::default()));
    open_role_wire(&mut world, 2, "host");
    assert_eq!(world.resource::<NativeFleetRoleWires>().last_generation, 0);
    assert!(world.resource::<NativeFleetRoleWires>().opening.is_empty());
}
#[test]
fn timed_out_role_dial_stays_bounded_and_closes_late_success() {
    let mut world = wire_world();
    let (sender, receiver) = std::sync::mpsc::channel();
    world.resource_mut::<NativeFleetRoleWires>().opening.insert(
        1,
        NativeFleetDial {
            receiver: std::sync::Mutex::new(receiver),
            started: std::time::Instant::now() - std::time::Duration::from_secs(16),
            cancelled: false,
        },
    );
    poll_role_dials(&mut world);
    assert!(world.resource::<NativeFleetRoleWires>().opening[&1].cancelled);
    let late = TestWire::default();
    assert!(sender
        .send(Ok(Box::new(late.clone()) as Box<dyn RelaySocket>))
        .is_ok());
    poll_role_dials(&mut world);
    assert!(late.0.lock().unwrap().1);
    assert!(world.resource::<NativeFleetRoleWires>().opening.is_empty());
    assert!(world.resource::<NativeFleetRoleWires>().sockets.is_empty());
}
#[test]
fn continuation_records_publish_runtime_refusal_under_the_request_generation() {
    let mut world = wire_world();
    let record = HostLobbyRecord::decode(r#"{"kind":"fleet_continuation","generation":91,"request":{"op":"begin","epoch":1,"previous_owner":1,"next_owner":2,"participants":[2,3]}}"#).unwrap();
    assert!(world.resource_mut::<NativeFleetEvents>().record(&record));
    apply_events(&mut world);
    let result = continuation_result(&world).unwrap();
    assert_eq!(result.generation, 91);
    assert_eq!(result.status["status"], "refused");
    assert_eq!(result.status["reason"], "not-in-fleet");
}
#[test]
fn malformed_retained_frame_refusal_survives_the_replayed_request() {
    use crate::command_admission::HostSlot;
    use crate::lockstep::continuation_systems::OwnerContinuation;
    let mut world = wire_world();
    world.init_resource::<OwnerContinuation>();
    world
        .resource_mut::<OwnerContinuation>()
        .state
        .begin(
            1,
            HostSlot(1),
            HostSlot(2),
            vec![HostSlot(2), HostSlot(3)],
            HostSlot(1),
            HostSlot(2),
            vec![HostSlot(1), HostSlot(2), HostSlot(3)],
        )
        .unwrap();
    world.resource_mut::<NativeFleetEvents>().0 = vec![
        NativeFleetEvent::ContinuationFrame {
            epoch: 1,
            source: 3,
            raw: "malformed".into(),
        },
        NativeFleetEvent::Continuation {
            generation: 11,
            request: serde_json::json!({"op":"replayed","epoch":1}),
        },
    ];
    apply_events(&mut world);
    let result = continuation_result(&world).unwrap();
    assert_eq!(result.generation, 11);
    assert_eq!(result.status["status"], "refused");
    assert_eq!(result.status["reason"], "malformed-continuation-frame");
    assert!(world.resource::<OwnerContinuation>().held());
}
#[test]
fn continuation_publication_follows_only_its_runtime_generation() {
    use crate::lockstep::continuation::{ContinuationPhase, ContinuationStatus};
    use crate::lockstep::continuation_systems::OwnerContinuation;
    let mut world = wire_world();
    world.init_resource::<OwnerContinuation>();
    let pending = ContinuationStatus {
        generation: 7,
        epoch: 1,
        status: ContinuationPhase::Pending,
        reason: None,
        loss_tick: None,
    };
    world
        .resource_mut::<NativeFleetPublication>()
        .continuation_result = Some(NativeContinuationResult {
        generation: 91,
        status: serde_json::to_value(&pending).unwrap(),
    });
    world.resource_mut::<OwnerContinuation>().state.status = pending;
    assert_eq!(
        continuation_result(&world).unwrap().status["status"],
        "pending"
    );
    {
        let mut lane = world.resource_mut::<OwnerContinuation>();
        lane.state.status.status = ContinuationPhase::Replayed;
        lane.state.status.loss_tick = Some(72);
    }
    let result = continuation_result(&world).unwrap();
    assert_eq!(result.generation, 91);
    assert_eq!(result.status["status"], "replayed");
    assert_eq!(result.status["loss_tick"], 72);
    world
        .resource_mut::<OwnerContinuation>()
        .state
        .status
        .generation = 8;
    assert_eq!(
        continuation_result(&world).unwrap().status["status"],
        "pending"
    );
}

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
