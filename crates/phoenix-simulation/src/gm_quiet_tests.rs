use super::*;
fn counts_as_worked_control(command: &AdmittedCommand) -> bool {
    let mut app = bevy::prelude::App::new();
    crate::server_app::add_simulation_plugins_with(
        &mut app,
        crate::server_app::SimPluginOptions {
            render: false,
            ..Default::default()
        },
    );
    app.add_plugins(crate::world::server::WorldPlugin);
    let registry = app
        .world()
        .resource::<crate::command_admission::AdmittedConsumerRegistry>();
    super::counts_as_worked_control(
        command,
        registry.legacy_feedback_support(&command.target, &command.payload)
            != crate::command_admission::FeedbackSupport::Unsupported,
    )
}

fn world(quiet_time_secs: f32, quiet_time_disabled: bool) -> WorldConfig {
    let mut world = WorldConfig::default();
    world.global.sim_tick_hz = 60.0;
    world.gm_attention.quiet_time_secs = quiet_time_secs;
    world.gm_attention.quiet_time_disabled = quiet_time_disabled;
    world
}

#[test]
fn the_row_appears_only_once_the_authored_interval_has_actually_elapsed() {
    let world = world(DEFAULT_QUIET_SECONDS, false);
    let mut activity = GmCrewActivity::default();
    // 120 s at 60 Hz is 7200 ticks. One tick short is still working hours.
    assert!(quiet_row(Some(&world), 7199, &mut activity).is_none());
    assert!(quiet_row(Some(&world), 7200, &mut activity).is_some());
}

#[test]
fn an_authored_interval_replaces_the_default_in_both_directions() {
    let short = world(30.0, false);
    let mut activity = GmCrewActivity::default();
    assert!(quiet_row(Some(&short), 1799, &mut activity).is_none());
    let row = quiet_row(Some(&short), 1800, &mut activity).expect("30 s elapsed");
    assert_eq!(row.seconds, 30);

    let long = world(300.0, false);
    let mut activity = GmCrewActivity::default();
    assert!(quiet_row(Some(&long), 7200, &mut activity).is_none());
    assert!(quiet_row(Some(&long), 18_000, &mut activity).is_some());
}

#[test]
fn the_disable_is_independent_of_the_duration() {
    let world = world(30.0, true);
    let mut activity = GmCrewActivity::default();
    assert!(quiet_row(Some(&world), 100_000, &mut activity).is_none());
}

#[test]
fn a_second_lull_is_a_fresh_occurrence_a_stale_snooze_cannot_hide() {
    let world = world(DEFAULT_QUIET_SECONDS, false);
    let mut activity = GmCrewActivity::default();
    let first = quiet_row(Some(&world), 7200, &mut activity).expect("first lull");
    // Still the same lull: the id is stable while the condition holds, so a
    // reading position or a focus ring survives the next publish.
    let same = quiet_row(Some(&world), 9000, &mut activity).expect("still quiet");
    assert_eq!(first.id, same.id);

    activity.observe(9001);
    assert!(quiet_row(Some(&world), 9002, &mut activity).is_none());
    let second = quiet_row(Some(&world), 9001 + 7200, &mut activity).expect("second lull");
    assert_ne!(first.id, second.id);
}

#[test]
fn a_slower_tick_rate_still_measures_the_same_seconds() {
    let mut world = world(DEFAULT_QUIET_SECONDS, false);
    world.global.sim_tick_hz = 20.0;
    let mut activity = GmCrewActivity::default();
    assert!(quiet_row(Some(&world), 2399, &mut activity).is_none());
    assert!(quiet_row(Some(&world), 2400, &mut activity).is_some());
}

fn command(target: &str, payload: SystemControlPayload, token: &str) -> AdmittedCommand {
    AdmittedCommand {
        target: crate::core::messages::SystemId(target.into()),
        payload,
        response_token: Some(token.into()),
        feedback_correlation: None,
    }
}

const SEAT: &str = "crew-session-token";

#[test]
fn a_correlated_payload_is_never_counted_as_a_worked_control() {
    // FirePhaser and a Comms response both settle a terminal result, so
    // this source must not count them — otherwise a press the System
    // refused, and an answer the world rejected, would read as effective
    // control.
    assert!(!counts_as_worked_control(&command(
        "phaser-fore",
        SystemControlPayload::FirePhaser,
        SEAT
    )));
    assert!(!counts_as_worked_control(&command(
        crate::ship::system_registry::COMMS_SYSTEM_ID,
        SystemControlPayload::RespondToMessage {
            message_id: "m1".into(),
            response_index: 0,
        },
        SEAT
    )));
}

#[test]
fn a_held_axis_counts_and_a_centred_one_does_not() {
    let steering = crate::ship::system_registry::HELM_STEERING_SYSTEM_ID;
    assert!(counts_as_worked_control(&command(
        steering,
        SystemControlPayload::SetSteering { value: 0.8 },
        SEAT
    )));
    assert!(!counts_as_worked_control(&command(
        steering,
        SystemControlPayload::SetSteering { value: 0.0 },
        SEAT
    )));
}

#[test]
fn an_ai_operators_own_steering_is_not_crew_activity() {
    let steering = crate::ship::system_registry::HELM_STEERING_SYSTEM_ID;
    assert!(!counts_as_worked_control(&command(
        steering,
        SystemControlPayload::SetSteering { value: 0.8 },
        crate::command_admission::ai_emit::AI_BACKFILL_TOKEN
    )));
    assert!(!counts_as_worked_control(&command(
        steering,
        SystemControlPayload::SetSteering { value: 0.8 },
        "ai:uuid-npc-7"
    )));
}

#[test]
fn a_relayed_seat_with_no_reply_address_still_counts() {
    // A command from another host arrives with no token at all. It is a
    // crew member steering; it is not an AI decision.
    let mut relayed = command(
        crate::ship::system_registry::HELM_THRUST_SYSTEM_ID,
        SystemControlPayload::SetThrust { value: 0.4 },
        SEAT,
    );
    relayed.response_token = None;
    assert!(counts_as_worked_control(&relayed));
}

#[test]
fn the_hosts_own_station_rating_replication_is_not_somebody_working() {
    assert!(!counts_as_worked_control(&command(
        "command",
        SystemControlPayload::AssignStationRating {
            station: crate::core::messages::StationId("helm".into()),
            rating: "Backfill".into(),
        },
        SEAT
    )));
}

#[test]
fn an_uncorrelated_seat_control_counts() {
    assert!(counts_as_worked_control(&command(
        "power",
        SystemControlPayload::SetPowerGroupAllocation {
            group: crate::core::messages::PowerGroupId("weapons".into()),
            level: 3,
        },
        SEAT
    )));
}

#[test]
fn observer_preserves_authored_legacy_controls_and_counts_correlations_only_from_results() {
    for (correlated, outcome, expected) in [
        (false, None, 12),
        (true, None, 0),
        (true, Some(ActionFeedbackOutcome::Refused), 0),
        (true, Some(ActionFeedbackOutcome::Applied), 12),
    ] {
        let mut production = App::new();
        crate::server_app::add_simulation_plugins_with(
            &mut production,
            crate::server_app::SimPluginOptions {
                render: false,
                ..Default::default()
            },
        );
        production.add_plugins(crate::world::server::WorldPlugin);
        let registry = production
            .world_mut()
            .remove_resource::<crate::command_admission::AdmittedConsumerRegistry>()
            .unwrap();
        let mut app = App::new();
        app.insert_resource(registry)
            .insert_resource(SimTick(12))
            .init_resource::<GmCrewActivity>()
            .add_message::<OutboundMessage>()
            .add_systems(Update, observe_crew_activity);
        let correlation =
            crate::core::messages::ActionCorrelationId::new("authored-impulse").unwrap();
        let mut action = command(
            "authored-impulse",
            SystemControlPayload::StartImpulseCharge,
            SEAT,
        );
        if correlated {
            action.feedback_correlation = Some(correlation.clone());
        }
        app.world_mut().spawn(AdmittedCommands(vec![action]));
        if let Some(outcome) = outcome {
            app.world_mut()
                .resource_mut::<Messages<OutboundMessage>>()
                .write(crate::command_admission::feedback::action_feedback(
                    SEAT,
                    &correlation,
                    outcome,
                ));
        }
        app.update();
        assert_eq!(
            app.world()
                .resource::<GmCrewActivity>()
                .last_activity_tick(),
            expected
        );
    }
}
