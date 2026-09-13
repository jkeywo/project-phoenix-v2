//! The canonical GM/scenario seam, including isolation and snapshot continuation.
#![cfg(all(feature = "headless", not(target_arch = "wasm32")))]
use bevy::{ecs::system::RunSystemOnce, prelude::*};
use phoenix::{
    command_admission::{log::ShipKey, HostSlot},
    comms::server::CommsInboxRes,
    core::messages::{CommsMessage, CommsPriority, GamePhase, ViewMode},
    entities::spawner::EntityUuid,
    gm_action::*,
    gm_presentation::*,
    lockstep::FleetSlotOf,
    ship::state::ShipViewMode,
    sim_tick::SimTick,
    world::server::WorldContentRuntime,
};
use project_phoenix as phoenix;

fn bare() -> App {
    let mut app = App::new();
    app.insert_resource(SimTick(42))
        .insert_resource(State::new(GamePhase::InProgress))
        .init_resource::<SimulationPaused>()
        .init_resource::<GmActionJournal>()
        .init_resource::<GmActionLog>()
        .init_resource::<WorldContentRuntime>()
        .init_resource::<CommsInboxRes>();
    for (slot, name) in [(1, "alpha"), (2, "bravo")] {
        app.world_mut().spawn((
            EntityUuid(name.into()),
            phoenix::server_app::Ship,
            FleetSlotOf(HostSlot(slot)),
            ShipViewMode::default(),
        ));
    }
    app
}
fn grant(sequence: u64, tick: u64, ship: &str, cue: PresentationCue) -> GmActionGrant {
    GmActionGrant {
        from: HostSlot(1),
        sequenced_by: HostSlot(1),
        operator_id: "gm".into(),
        correlation: GmActionId::new(format!("presentation-{sequence}")).unwrap(),
        recovery_generation: 0,
        apply_tick: tick,
        order: GmActionOrder::new(HostSlot(1), sequence),
        action: GmAction::Presentation {
            ship: ShipKey(ship.into()),
            cue,
        },
    }
}
fn apply_gm(app: &mut App, grant: GmActionGrant) {
    app.world_mut()
        .resource_mut::<GmActionJournal>()
        .insert(grant)
        .unwrap();
    app.world_mut().run_system_once(apply_due_actions).unwrap();
}
fn title() -> PresentationCue {
    PresentationCue::TitleCard {
        title: "Arrival".into(),
        subtitle: "The signal returns".into(),
        duration_ticks: 20,
    }
}
fn message(recipient: &str) -> CommsMessage {
    let mut msg = CommsMessage::injected(
        "message-one".into(),
        "sender".into(),
        "Lyra".into(),
        "Hold position".into(),
        Default::default(),
        vec![],
        "thread".into(),
        true,
        CommsPriority::Routine,
    );
    msg.recipient_ship = Some(ShipKey(recipient.into()));
    msg
}

#[test]
fn gm_and_scenario_apply_the_same_state_with_canonical_attribution_and_no_cross_ship_change() {
    let mut gm = bare();
    let mut scenario = bare();
    apply_gm(&mut gm, grant(1, 42, "alpha", title()));
    scenario
        .world_mut()
        .run_system_once_with(apply_scenario_command, ("alpha".into(), title()))
        .unwrap();
    assert_eq!(
        gm.world().resource::<WorldContentRuntime>().presentation,
        scenario
            .world()
            .resource::<WorldContentRuntime>()
            .presentation
    );
    assert!(!gm
        .world()
        .resource::<WorldContentRuntime>()
        .presentation
        .contains_key("bravo"));
    let facts = gm.world().resource::<GmActionLog>().entries();
    assert_eq!(facts[0].action_kind, GmActionKind::Presentation);
    assert_eq!(facts[0].operator_id, "gm");
    assert_eq!(facts[0].outcome, GmActionOutcome::Applied);
    assert_eq!(facts[0].target.as_deref(), Some("alpha"));
    // Clearing is absolute, and a second clear is an honest no-op.
    apply_gm(&mut gm, grant(2, 42, "alpha", PresentationCue::ClearCard));
    apply_gm(&mut gm, grant(3, 42, "alpha", PresentationCue::ClearCard));
    assert_eq!(
        gm.world().resource::<GmActionLog>().entries()[2].outcome,
        GmActionOutcome::NoOp
    );
}

#[test]
fn a_held_simulation_publishes_the_cue_result_without_needing_a_fixed_tick() {
    use phoenix::console_bridge::GmEntityProjectionChanged;
    let mut app = bare();
    app.add_plugins(phoenix::gm_projection::GmProjectionPlugin);
    app.insert_resource(phoenix::gm_projection::NativeGmPresentation);
    app.world_mut().resource_mut::<SimulationPaused>().0 = true;
    apply_gm(&mut app, grant(1, 42, "alpha", title()));
    app.world_mut().run_schedule(PostUpdate);
    let projection = app
        .world_mut()
        .resource_mut::<Messages<GmEntityProjectionChanged>>()
        .drain()
        .last()
        .unwrap()
        .payload;
    assert_eq!(
        projection.presentation_results[0].outcome,
        GmActionOutcome::Applied
    );
    assert!(projection.presentation["alpha"].card.is_some());
    assert_eq!(app.world().resource::<SimTick>().0, 42);
    apply_gm(&mut app, grant(2, 42, "alpha", PresentationCue::ClearCard));
    app.world_mut().run_schedule(PostUpdate);
    let projection = app
        .world_mut()
        .resource_mut::<Messages<GmEntityProjectionChanged>>()
        .drain()
        .last()
        .unwrap()
        .payload;
    assert!(projection.presentation.is_empty());
    assert_eq!(projection.presentation_results.len(), 2);
    assert_eq!(app.world().resource::<SimTick>().0, 42);
}

#[test]
fn a_forced_view_survives_crew_requests_then_releases_to_the_latest_crew_selection() {
    let mut app = bare();
    apply_gm(
        &mut app,
        grant(
            1,
            42,
            "alpha",
            PresentationCue::ForceView {
                view: PresentationView::SensorsRadar,
                duration_ticks: 5,
            },
        ),
    );
    app.world_mut().run_system_once(sync_views).unwrap();
    let mut query = app.world_mut().query::<(&EntityUuid, &mut ShipViewMode)>();
    for (id, mut view) in query.iter_mut(app.world_mut()) {
        if id.0 == "alpha" {
            view.request_view_mode(ViewMode::NavigationChart);
            assert_eq!(view.view_mode, ViewMode::SensorsRadar);
            // A fixed-tick reader sees expiry before the frame-side mirror,
            // including the underlying crew choice beneath the old force.
            assert_eq!(
                resolved_view_mode(None, 47, &view),
                ViewMode::NavigationChart
            );
        } else {
            assert_eq!(view.view_mode, ViewMode::default());
        }
    }
    // Same simulation tick across arbitrarily many frame updates: no expiry.
    for _ in 0..3 {
        app.world_mut().run_system_once(prune).unwrap();
        app.world_mut().run_system_once(sync_views).unwrap();
    }
    assert!(
        app.world().resource::<WorldContentRuntime>().presentation["alpha"]
            .forced_view
            .is_some()
    );
    app.world_mut().resource_mut::<SimTick>().0 = 47;
    app.world_mut().run_system_once(prune).unwrap();
    app.world_mut().run_system_once(sync_views).unwrap();
    for (id, view) in query.iter(app.world()) {
        if id.0 == "alpha" {
            assert_eq!(view.view_mode, ViewMode::NavigationChart);
        }
    }
    assert!(app
        .world()
        .resource::<WorldContentRuntime>()
        .presentation
        .is_empty());
}

#[test]
fn incoming_takeover_rechecks_audience_and_orphaning_at_apply_and_render() {
    let mut app = bare();
    app.world_mut()
        .resource_mut::<CommsInboxRes>()
        .0
        .inject(message("alpha"));
    let cue = PresentationCue::IncomingComms {
        message: "message-one".into(),
        duration_ticks: 20,
    };
    apply_gm(&mut app, grant(1, 42, "bravo", cue.clone()));
    assert_eq!(
        app.world().resource::<GmActionLog>().entries()[0].outcome,
        GmActionOutcome::Refused
    );
    apply_gm(&mut app, grant(2, 42, "alpha", cue.clone()));
    let state = app.world().resource::<WorldContentRuntime>().presentation["alpha"].clone();
    let inbox = app.world().resource::<CommsInboxRes>();
    assert_eq!(
        card_wire(Some(&state), 42, "alpha", Some(inbox))
            .unwrap()
            .body,
        "Hold position"
    );
    assert!(card_wire(Some(&state), 42, "bravo", Some(inbox)).is_none());
    assert!(card_wire(Some(&state), 62, "alpha", Some(inbox)).is_none());
    app.world_mut()
        .resource_mut::<CommsInboxRes>()
        .0
        .orphan_sender("sender");
    assert!(card_wire(
        Some(&state),
        42,
        "alpha",
        Some(app.world().resource::<CommsInboxRes>())
    )
    .is_none());
    apply_gm(&mut app, grant(3, 42, "alpha", cue));
    assert_eq!(
        app.world().resource::<GmActionLog>().entries()[2].outcome,
        GmActionOutcome::Refused
    );
}

#[test]
fn camera_cues_require_the_receiving_ships_authoritative_marker_on_both_paths() {
    let mut app = bare();
    let entity = app
        .world_mut()
        .query::<(Entity, &EntityUuid)>()
        .iter(app.world())
        .find(|(_, id)| id.0 == "alpha")
        .unwrap()
        .0;
    app.world_mut().entity_mut(entity).insert(
        phoenix::entities::model_rig::ModelMarkers::from_markers(
            [(
                "camera_aft".into(),
                phoenix::entities::model_rig::Marker {
                    position: [0.0; 3],
                    direction: [0.0, 0.0, 1.0],
                },
            )]
            .into(),
        ),
    );
    let camera = |name: &str| PresentationCue::ForceView {
        view: PresentationView::Camera(name.into()),
        duration_ticks: 10,
    };
    apply_gm(&mut app, grant(1, 42, "alpha", camera("typo")));
    assert_eq!(
        app.world().resource::<GmActionLog>().entries()[0].outcome,
        GmActionOutcome::Refused
    );
    app.world_mut()
        .run_system_once_with(apply_scenario_command, ("alpha".into(), camera("typo")))
        .unwrap();
    assert!(app
        .world()
        .resource::<WorldContentRuntime>()
        .presentation
        .is_empty());
    apply_gm(&mut app, grant(2, 42, "alpha", camera("camera_aft")));
    assert_eq!(
        app.world().resource::<GmActionLog>().entries()[1].outcome,
        GmActionOutcome::Applied
    );
    apply_gm(&mut app, grant(3, 42, "bravo", camera("camera_aft")));
    assert_eq!(
        app.world().resource::<GmActionLog>().entries()[2].outcome,
        GmActionOutcome::Refused
    );
}

#[test]
fn strict_codec_refuses_extra_fields_bad_durations_and_unknown_cue_vocabulary() {
    let prefix =
        r#"{"operator_id":"gm","correlation":"cue","action":"presentation","ship":"alpha","cue":"#;
    for cue in [
        r#"{"title_card":{"title":"Test","subtitle":"","duration_ticks":0}}"#,
        r#"{"force_view":{"view":"hologram","duration_ticks":1}}"#,
        r#"{"title_card":{"title":"Test","subtitle":"","duration_ticks":1,"script":"evil"}}"#,
    ] {
        assert!(
            phoenix::core::codec::decode_gm_action_request(&format!("{prefix}{cue}}}")).is_none()
        );
    }
    let request =
        phoenix::core::codec::decode_gm_action_request(&format!("{prefix}\"release_view\"}}"))
            .unwrap();
    assert!(matches!(
        request.action,
        GmAction::Presentation {
            cue: PresentationCue::ReleaseView,
            ..
        }
    ));
    // Durable enum encoding works: client ViewMode's internal tag is not used.
    let action = grant(
        1,
        42,
        "alpha",
        PresentationCue::ForceView {
            view: PresentationView::Camera("camera_aft".into()),
            duration_ticks: 10,
        },
    );
    let codec = vellum_digest::ShareCodec::new("GM-PRESENTATION-TEST-");
    let bytes = codec.encode(&action).unwrap();
    assert_eq!(codec.decode::<GmActionGrant>(&bytes).unwrap(), action);
}

fn seeded() -> (App, String) {
    let args = phoenix::headless::HeadlessArgs {
        world_path: "assets/worlds/patrol.toml".into(),
        ship_path: "assets/entities/alliance_cruiser.toml".into(),
        seed: Some(1309),
        deterministic: true,
        max_ticks: 260,
        ..Default::default()
    };
    let mut app = phoenix::headless::build_headless_app(&args).unwrap();
    app.finish();
    app.cleanup();
    for _ in 0..90 {
        app.update();
    }
    let (entity, ship) = app
        .world_mut()
        .query_filtered::<(Entity, &EntityUuid), With<phoenix::server_app::LocalShip>>()
        .iter(app.world())
        .map(|(entity, id)| (entity, id.0.clone()))
        .next()
        .unwrap();
    app.world_mut()
        .entity_mut(entity)
        .insert(FleetSlotOf(HostSlot(1)));
    (app, ship)
}

#[test]
fn snapshot_restores_only_the_live_cue_and_preserves_its_original_expiry_and_digest() {
    let (mut app, ship) = seeded();
    let tick = app.world().resource::<SimTick>().0;
    apply_gm(&mut app, grant(1, tick, &ship, title()));
    let saved = phoenix::snapshot::capture(app.world());
    let (mut restored, _) = seeded();
    let report = phoenix::snapshot::restore(restored.world_mut(), &saved);
    assert!(report.is_complete(), "{:?}", report.gaps);
    assert_eq!(
        app.world().resource::<WorldContentRuntime>().presentation,
        restored
            .world()
            .resource::<WorldContentRuntime>()
            .presentation
    );
    let with_cue = phoenix::sim_digest::world_digest(restored.world());
    restored
        .world_mut()
        .resource_mut::<WorldContentRuntime>()
        .presentation
        .clear();
    assert_ne!(
        with_cue,
        phoenix::sim_digest::world_digest(restored.world())
    );
    phoenix::snapshot::restore(restored.world_mut(), &saved);
    let state = &restored
        .world()
        .resource::<WorldContentRuntime>()
        .presentation[&ship];
    assert!(card_wire(Some(state), tick + 20, &ship, None).is_none());
    assert!(card_wire(Some(state), tick + 19, &ship, None).is_some());
}
