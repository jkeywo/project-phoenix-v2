use super::*;
use crate::core::messages::DeliveryClass;
struct Alpha;
struct Zulu;
fn alpha(requests: Res<ReconnectRequests>) -> ReconnectBatch {
    requests
        .0
        .iter()
        .map(|token| {
            if token == "hidden" {
                vec![]
            } else {
                vec![
                    ServerMessage::GameStartCountdown { remaining_secs: 1 },
                    ServerMessage::GameStarted,
                ]
            }
        })
        .collect()
}
fn zulu(requests: Res<ReconnectRequests>) -> ReconnectBatch {
    requests
        .0
        .iter()
        .map(|_| vec![ServerMessage::GameStartCountdown { remaining_secs: 9 }])
        .collect()
}
fn app(reverse: bool) -> App {
    let mut app = App::new();
    app.init_resource::<SimOutbox>();
    test_reconnect_welcomes(&mut app, &[]);
    if reverse {
        register_reconnect_projection::<Zulu, Res<'static, ReconnectRequests>>(
            &mut app,
            "zulu",
            |p| zulu(p.downcast::<Res<ReconnectRequests>>().unwrap()),
        );
        register_reconnect_projection::<Alpha, Res<'static, ReconnectRequests>>(
            &mut app,
            "alpha",
            |p| alpha(p.downcast::<Res<ReconnectRequests>>().unwrap()),
        );
    } else {
        register_reconnect_projection::<Alpha, Res<'static, ReconnectRequests>>(
            &mut app,
            "alpha",
            |p| alpha(p.downcast::<Res<ReconnectRequests>>().unwrap()),
        );
        register_reconnect_projection::<Zulu, Res<'static, ReconnectRequests>>(
            &mut app,
            "zulu",
            |p| zulu(p.downcast::<Res<ReconnectRequests>>().unwrap()),
        );
    }
    app
}
#[test]
fn scheduled_delivery_retains_occurrences_owner_privacy_and_payload_order() {
    let mut observations = Vec::new();
    for reverse in [false, true] {
        let mut app = app(reverse);
        app.finish();
        test_reconnect_welcomes(&mut app, &["second", "hidden", "second", "first"]);
        // Neither a broadcast Welcome nor a different targeted message is
        // a reconnect request. Keep them adjacent to real occurrences.
        let welcome = app
            .world()
            .resource::<crate::lobby::server::LobbyOutbox>()
            .0[0]
            .1
            .clone();
        app.world_mut()
            .resource_mut::<crate::lobby::server::LobbyOutbox>()
            .0
            .extend([
                (Target::All, welcome),
                (Target::Token("ignored".into()), ServerMessage::GameStarted),
            ]);
        app.world_mut().run_schedule(FixedUpdate);
        let output = app.world_mut().resource_mut::<SimOutbox>().drain();
        let mut rows = Vec::new();
        for row in output {
            assert_eq!(row.delivery, DeliveryClass::Snapshot);
            let Target::Token(token) = row.target else {
                panic!("private target required")
            };
            let value = match row.message {
                ServerMessage::GameStartCountdown { remaining_secs } => remaining_secs,
                ServerMessage::GameStarted => 2,
                _ => panic!("unexpected owner payload"),
            };
            rows.push((token, value));
        }
        assert_eq!(
            rows,
            vec![
                ("second".into(), 1),
                ("second".into(), 2),
                ("second".into(), 9),
                ("hidden".into(), 9),
                ("second".into(), 1),
                ("second".into(), 2),
                ("second".into(), 9),
                ("first".into(), 1),
                ("first".into(), 2),
                ("first".into(), 9)
            ]
        );
        assert!(app.world().resource::<ReconnectRequests>().0.is_empty());
        app.world_mut()
            .resource_mut::<crate::lobby::server::LobbyOutbox>()
            .0
            .clear();
        app.world_mut().run_schedule(FixedUpdate);
        assert!(
            app.world_mut()
                .resource_mut::<SimOutbox>()
                .drain()
                .is_empty(),
            "scratch must not replay stale requests"
        );
        observations.push(rows);
    }
    assert_eq!(observations[0], observations[1]);
}
#[test]
fn initialized_reconnect_pipeline_contains_no_exclusive_system() {
    let mut app = app(false);
    app.finish();
    app.world_mut().run_schedule(FixedUpdate);
    let schedules = app.world().resource::<bevy::ecs::schedule::Schedules>();
    let systems: Vec<_> = schedules
        .get(FixedUpdate)
        .unwrap()
        .systems()
        .unwrap()
        .collect();
    assert_eq!(systems.len(), 1, "one collector/capture/delivery node");
    for (_, system) in systems {
        assert_eq!(
            system.name().to_string(),
            "project_phoenix::server_app::broadcast_publish::refresh_caches_on_midgame_reconnect",
            "the single production node retains the exact old full logical name"
        );
        assert!(!system.is_exclusive(), "{} hides its access", system.name());
    }
}
#[test]
#[should_panic(expected = "duplicate reconnect lifecycle key")]
fn duplicate_semantic_owner_cannot_overwrite_output() {
    let mut app = app(false);
    struct Other;
    register_reconnect_projection::<Other, Res<'static, ReconnectRequests>>(
        &mut app,
        "alpha",
        |p| zulu(p.downcast::<Res<ReconnectRequests>>().unwrap()),
    );
}
#[test]
#[should_panic(expected = "duplicate reconnect owner marker")]
fn distinct_keys_cannot_share_mutable_owner_scratch() {
    let mut app = app(false);
    register_reconnect_projection::<Alpha, Res<'static, ReconnectRequests>>(
        &mut app,
        "other",
        |p| alpha(p.downcast::<Res<ReconnectRequests>>().unwrap()),
    );
}
#[test]
#[should_panic(expected = "projector must answer each request ordinal")]
fn missing_occurrence_is_refused_instead_of_shifting_private_output() {
    fn broken(_: Res<ReconnectRequests>) -> ReconnectBatch {
        vec![]
    }
    let mut app = App::new();
    test_reconnect_welcomes(&mut app, &["owner"]);
    register_reconnect_projection::<Alpha, Res<'static, ReconnectRequests>>(
        &mut app,
        "broken",
        |p| broken(p.downcast::<Res<ReconnectRequests>>().unwrap()),
    );
    app.finish();
    app.world_mut().run_schedule(FixedUpdate);
}
#[test]
fn physical_transport_scratch_inherits_the_existing_canonical_owner() {
    use crate::authoritative::StateCensus;
    let app = app(false);
    let census = app.world().resource::<StateCensus>();
    let owner = std::any::type_name::<ReplicationLifecycleRegistry>();
    assert_eq!(
        census.alias_owner(std::any::type_name::<ReconnectRequests>()),
        Some(owner)
    );
}
#[derive(Resource, Default)]
struct SourceA(u32);
#[derive(Component, Default)]
struct SourceB(u32);
fn advance(mut a: ResMut<SourceA>, mut b: Query<&mut SourceB>) {
    a.0 += 1;
    b.single_mut().unwrap().0 = a.0;
}
fn coherent_app() -> App {
    let mut app = App::new();
    app.init_resource::<SourceA>().init_resource::<SimOutbox>();
    test_reconnect_welcomes(&mut app, &[]);
    app.world_mut().spawn(SourceB::default());
    register_reconnect_projection::<Alpha, (Res<'static, ReconnectRequests>, Res<'static, SourceA>)>(
        &mut app,
        "alpha",
        |p| {
            let (requests, source) = p
                .downcast::<(Res<ReconnectRequests>, Res<SourceA>)>()
                .unwrap();
            requests
                .0
                .iter()
                .map(|_| {
                    vec![ServerMessage::GameStartCountdown {
                        remaining_secs: source.0,
                    }]
                })
                .collect()
        },
    );
    register_reconnect_projection::<
        Zulu,
        (
            Res<'static, ReconnectRequests>,
            Query<'static, 'static, &'static SourceB>,
        ),
    >(&mut app, "zulu", |p| {
        let (requests, source) = p
            .downcast::<(Res<ReconnectRequests>, Query<&SourceB>)>()
            .unwrap();
        requests
            .0
            .iter()
            .map(|_| {
                vec![ServerMessage::GameStartCountdown {
                    remaining_secs: source.single().unwrap().0,
                }]
            })
            .collect()
    });
    app
}
#[test]
fn capture_holds_the_exact_concrete_read_union_against_a_cross_owner_writer() {
    let mut app = coherent_app();
    let registered = owners(app.world());
    let mut capture = build_capture(app.world_mut(), registered);
    let capture_access = capture.initialize(app.world_mut());
    let access = capture_access.combined_access();
    let components = app.world().components();
    let a = components.resource_id::<SourceA>().unwrap();
    let b = components.component_id::<SourceB>().unwrap();
    assert!(access.has_resource_read(a));
    assert!(access.has_component_read(b));
    assert!(!access.has_any_write());
    assert!(!access.has_read_all_resources() && !access.has_read_all_components());
    assert!(!capture.is_exclusive());
    let mut writer = IntoSystem::into_system(advance);
    let writer_access = writer.initialize(app.world_mut());
    assert!(
        !capture_access.is_compatible(&writer_access),
        "the scheduler must hold both owners' reads across the whole capture"
    );
}
#[test]
fn one_cohort_cannot_straddle_a_writer_between_owners_or_duplicate_requests() {
    let mut app = coherent_app();
    app.add_systems(FixedUpdate, advance);
    app.finish();
    let mut values = std::collections::BTreeSet::new();
    for _ in 0..16 {
        test_reconnect_welcomes(&mut app, &["one", "two", "one"]);
        app.world_mut().run_schedule(FixedUpdate);
        let rows = app.world_mut().resource_mut::<SimOutbox>().drain();
        assert_eq!(rows.len(), 6);
        let captured: Vec<_> = rows
            .iter()
            .map(|r| {
                let ServerMessage::GameStartCountdown { remaining_secs } = r.message else {
                    panic!("typed observation")
                };
                remaining_secs
            })
            .collect();
        assert!(
            captured.iter().all(|n| *n == captured[0]),
            "all owners/occurrences share one source observation"
        );
        values.insert(captured[0]);
    }
    assert!(values.len() >= 15, "the conflicting writer really advances");
}
#[test]
#[should_panic(expected = "reconnect registration after finalization")]
fn finalized_cohort_refuses_a_late_owner() {
    let mut app = app(false);
    app.finish();
    struct Later;
    register_reconnect_projection::<Later, Res<'static, ReconnectRequests>>(
        &mut app,
        "late",
        |p| alpha(p.downcast::<Res<ReconnectRequests>>().unwrap()),
    );
}
#[test]
fn empty_requests_do_not_invoke_owner_projection() {
    struct Never;
    let mut app = App::new();
    test_reconnect_welcomes(&mut app, &[]);
    register_reconnect_projection::<Never, ()>(&mut app, "never", |_| {
        panic!("empty capture invoked an owner")
    });
    app.finish();
    app.world_mut().run_schedule(FixedUpdate);
    assert!(app.world().resource::<ReconnectRequests>().0.is_empty());
    // A real targeted Welcome outside InProgress must likewise not invoke
    // a projector; the collector clears the cohort at the same boundary.
    test_reconnect_welcomes(&mut app, &["lobby-only"]);
    app.insert_resource(State::new(crate::core::messages::GamePhase::Lobby));
    app.world_mut().run_schedule(FixedUpdate);
    assert!(app.world().resource::<ReconnectRequests>().0.is_empty());
}
