use super::*;
use crate::ship::control_source::ControlSource;

fn target(ship: &str, station: &str) -> StationPuppetTarget {
    StationPuppetTarget::new(ShipKey(ship.into()), StationId(station.into()))
}

#[test]
fn takeover_is_station_scoped_and_an_exact_retry_is_a_no_op() {
    let mut puppets = StationPuppets::default();
    let helm = target("player-1", "helm");
    let tactical = target("player-1", "tactical");

    assert!(puppets.set_operator(helm.clone(), "gm-2".into(), true));
    assert!(!puppets.set_operator(helm.clone(), "gm-2".into(), true));
    assert!(puppets.is_operated_by(&helm, "gm-2"));
    assert!(!puppets.is_active(&tactical));
}

#[test]
fn equal_gms_share_a_station_and_release_only_their_own_membership() {
    let mut puppets = StationPuppets::default();
    let helm = target("player-1", "helm");

    assert!(puppets.set_operator(helm.clone(), "gm-2".into(), true));
    assert!(puppets.set_operator(helm.clone(), "gm-1".into(), true));
    assert_eq!(puppets.operators(&helm), &["gm-1", "gm-2"]);

    assert!(puppets.set_operator(helm.clone(), "gm-1".into(), false));
    assert!(puppets.is_active(&helm));
    assert_eq!(puppets.operators(&helm), &["gm-2"]);
    assert!(!puppets.set_operator(helm.clone(), "gm-1".into(), false));

    assert!(puppets.set_operator(helm.clone(), "gm-2".into(), false));
    assert!(!puppets.is_active(&helm));
    assert!(puppets.entries().is_empty());
}

#[test]
fn rows_and_operator_membership_are_canonical_not_arrival_ordered() {
    let actions = [
        (target("player-2", "tactical"), "gm-3"),
        (target("player-1", "helm"), "gm-2"),
        (target("player-1", "helm"), "gm-1"),
    ];
    let mut forward = StationPuppets::default();
    let mut reverse = StationPuppets::default();
    for (target, operator) in actions.iter() {
        forward.set_operator(target.clone(), (*operator).into(), true);
    }
    for (target, operator) in actions.iter().rev() {
        reverse.set_operator(target.clone(), (*operator).into(), true);
    }
    assert_eq!(forward, reverse);
}

fn control_test_config() -> crate::ship::config::ShipConfig {
    crate::ship::config::ShipConfig::from_toml(
        r#"
[[station]]
id = "helm"
name = "Helm"
description = ""
rank = ""
console = "helm.html"

[[station.rating]]
name = "Manual"
automated_systems = []

[[station.rating]]
name = "Assisted"
automated_systems = ["impulse-drive"]

[[station]]
id = "tactical"
name = "Tactical"
description = ""
rank = ""
console = "weapons.html"

[[station.rating]]
name = "Manual"
automated_systems = []

[[system]]
id = "helm-thrust"
kind = "helm_thrust"
station = "helm"

[[system]]
id = "impulse-drive"
kind = "helm_impulse"
station = "helm"

[[system]]
id = "phaser"
kind = "phaser_bank"
station = "tactical"

[[system]]
id = "main-view"
kind = "viewscreen"
station = "tactical"

[[system]]
id = "helm-radar"
kind = "helm_radar"
station = "helm"

[[system]]
id = "science-sensors"
kind = "sensors"
station = "tactical"
"#,
        &[
            "helm_thrust",
            "helm_impulse",
            "phaser_bank",
            "viewscreen",
            "helm_radar",
            "sensors",
        ],
    )
    .unwrap()
}

#[test]
fn takeover_suppresses_only_the_selected_station_and_release_restores_backfill() {
    let config = control_test_config();
    let helm = StationId("helm".into());
    let tactical = StationId("tactical".into());
    let mut ratings = ActiveStationRatings::default();
    ratings
        .0
        .insert(helm.clone(), crate::ship::rating::BACKFILL_RATING.into());
    ratings.0.insert(
        tactical.clone(),
        crate::ship::rating::BACKFILL_RATING.into(),
    );
    let mut sources = ShipSystemControlSources::default();
    crate::ship::rating::apply_rating(
        &config,
        &helm,
        crate::ship::rating::BACKFILL_RATING,
        &mut sources.0,
    );
    crate::ship::rating::apply_rating(
        &config,
        &tactical,
        crate::ship::rating::BACKFILL_RATING,
        &mut sources.0,
    );

    let ship = ShipKey("player-1".into());
    let mut app = App::new();
    app.init_resource::<StationPuppets>()
        .init_resource::<PreviousStationPuppetTargets>()
        .add_systems(Update, reconcile_station_puppet_control);
    app.world_mut().spawn((
        crate::server_app::Ship,
        EntityUuid(ship.0.clone()),
        ShipConfigComponent(config),
        sources,
        ratings,
    ));

    let target = StationPuppetTarget::new(ship, helm.clone());
    app.world_mut()
        .resource_mut::<StationPuppets>()
        .set_operator(target.clone(), "gm-1".into(), true);
    app.update();

    let sources = app
        .world_mut()
        .query::<&ShipSystemControlSources>()
        .single(app.world())
        .unwrap();
    assert_eq!(
        sources
            .0
            .source_for(&crate::core::messages::SystemId("helm-thrust".into())),
        ControlSource::Human,
    );
    assert_eq!(
        sources
            .0
            .source_for(&crate::core::messages::SystemId("phaser".into())),
        ControlSource::Ai,
        "a different Backfill Station keeps operating AI",
    );

    app.world_mut()
        .resource_mut::<StationPuppets>()
        .set_operator(target, "gm-1".into(), false);
    app.update();
    let sources = app
        .world_mut()
        .query::<&ShipSystemControlSources>()
        .single(app.world())
        .unwrap();
    assert_eq!(
        sources
            .0
            .source_for(&crate::core::messages::SystemId("helm-thrust".into())),
        ControlSource::Ai,
        "release restores the Station's ordinary Backfill rating",
    );
}

#[test]
fn human_holder_keeps_authority_and_release_restores_the_live_mixed_rating() {
    use crate::core::messages::{SystemControlPayload, SystemId};
    let config = control_test_config();
    let helm = StationId("helm".into());
    let thrust = SystemId("helm-thrust".into());
    let impulse = SystemId("impulse-drive".into());
    let mut sessions = crate::lobby::Sessions(Default::default());
    sessions
        .0
        .register("crew".into(), "Helm player".into())
        .unwrap();
    sessions.0.set_station("crew", Some(helm.clone()));
    let (sources, ratings) = crate::ship::rating::seed_boot_ratings(&config, |station| {
        if station.id == helm {
            "Manual".into()
        } else {
            "Backfill".into()
        }
    });
    let target = target("player-1", "helm");
    let mut app = App::new();
    app.insert_resource(sessions)
        .init_resource::<StationPuppets>()
        .init_resource::<PreviousStationPuppetTargets>()
        .add_systems(Update, reconcile_station_puppet_control);
    let entity = app
        .world_mut()
        .spawn((
            crate::server_app::Ship,
            EntityUuid("player-1".into()),
            crate::lockstep::FleetSlotOf(crate::command_admission::HostSlot(1)),
            ShipConfigComponent(config.clone()),
            ShipSystemControlSources(sources),
            ActiveStationRatings(ratings),
        ))
        .id();
    for operator in ["gm-b", "gm-a"] {
        let action = crate::gm_action::GmAction::SetStationPuppet {
            ship: target.ship.clone(),
            station: helm.clone(),
            active: true,
        };
        assert_eq!(
            validate_station_action_in_world(app.world_mut(), &action, operator),
            Ok(())
        );
        assert!(app
            .world_mut()
            .resource_mut::<StationPuppets>()
            .set_operator(target.clone(), operator.into(), true));
    }
    app.update();
    // A human changes their ordinary rating during takeover. Release must
    // restore THIS live rating, rather than the rating at takeover time.
    app.world_mut()
        .get_mut::<ActiveStationRatings>(entity)
        .unwrap()
        .0
        .insert(helm.clone(), "Assisted".into());
    app.update();
    let sources = app.world().get::<ShipSystemControlSources>(entity).unwrap();
    let sessions = app.world().resource::<crate::lobby::Sessions>();
    assert!(crate::command_admission::is_command_authorized(
        "crew",
        &thrust,
        &SystemControlPayload::SetThrust { value: 0.4 },
        sources,
        sessions,
        &config,
        None,
    ));
    assert!(!sources.0.policy_for(&impulse).operate_ai);
    assert_eq!(sessions.0.station_for_token("crew"), Some(&helm));
    app.world_mut()
        .resource_mut::<StationPuppets>()
        .remove_operator_everywhere("gm-a");
    app.update();
    assert!(
        !app.world()
            .get::<ShipSystemControlSources>(entity)
            .unwrap()
            .0
            .policy_for(&impulse)
            .operate_ai,
        "equal surviving GM retains takeover"
    );
    app.world_mut()
        .resource_mut::<StationPuppets>()
        .remove_operator_everywhere("gm-b");
    app.update();
    let sources = app.world().get::<ShipSystemControlSources>(entity).unwrap();
    assert_eq!(sources.0.source_for(&thrust), ControlSource::Human);
    assert_eq!(sources.0.source_for(&impulse), ControlSource::Ai);
    assert_eq!(
        app.world()
            .resource::<crate::lobby::Sessions>()
            .0
            .station_for_token("crew"),
        Some(&helm)
    );
}

#[test]
fn gm_commands_follow_human_admission_in_canonical_order_and_keep_attribution_sidecar_only() {
    use crate::core::messages::{
        AdmittedCommand, AdmittedCommands, SystemControlPayload, SystemId,
    };

    let config = control_test_config();
    let helm = StationId("helm".into());
    let helm_target = SystemId("helm-thrust".into());
    let mut sources = ShipSystemControlSources::default();
    sources.0.set(helm_target.clone(), ControlSource::Human);
    let mut app = App::new();
    crate::console::helm::dispatch::register_helm_dispatch(&mut app);
    app.insert_resource(crate::sim_tick::SimTick(12))
        .init_resource::<PendingGmStationCommands>()
        .init_resource::<PendingGmStationFeedbackRoutes>()
        .init_resource::<StationPuppetActivity>()
        .add_systems(Update, admit_station_puppet_commands);
    app.world_mut().spawn((
        crate::server_app::Ship,
        EntityUuid("player-1".into()),
        ShipConfigComponent(config),
        sources,
        AdmittedCommands(vec![AdmittedCommand {
            target: helm_target.clone(),
            payload: SystemControlPayload::SetThrust { value: 0.1 },
            response_token: Some("crew-route-only".into()),
            feedback_correlation: None,
        }]),
    ));

    for (sequence, operator, value) in [(3, "gm-2", 0.3), (2, "gm-1", 0.2)] {
        app.world_mut()
            .resource_mut::<PendingGmStationCommands>()
            .push(PendingGmStationCommand {
                tick: 12,
                order: crate::gm_action::GmActionOrder::new(
                    crate::command_admission::HostSlot(sequence as u32),
                    sequence,
                ),
                operator_id: operator.into(),
                correlation: crate::gm_action::GmActionId::new(format!("cmd-{sequence}")).unwrap(),
                ship: ShipKey("player-1".into()),
                station: helm.clone(),
                target: helm_target.clone(),
                payload: SystemControlPayload::SetThrust { value },
            });
    }
    app.update();

    let admitted = app
        .world_mut()
        .query::<&AdmittedCommands>()
        .single(app.world())
        .unwrap();
    let values: Vec<_> = admitted
        .0
        .iter()
        .map(|command| match &command.payload {
            SystemControlPayload::SetThrust { value } => *value,
            _ => panic!("unexpected payload"),
        })
        .collect();
    assert_eq!(values, vec![0.1, 0.2, 0.3]);
    assert_eq!(
        admitted.0[0].response_token.as_deref(),
        Some("crew-route-only")
    );
    assert!(admitted.0[1..]
        .iter()
        .all(|command| command.response_token.is_none()));
    assert_eq!(
        app.world()
            .resource::<StationPuppetActivity>()
            .entries()
            .iter()
            .map(|entry| entry.operator_id.as_str())
            .collect::<Vec<_>>(),
        vec!["gm-1", "gm-2"],
    );
}

#[test]
fn authentic_helm_consumer_settles_gm_correlation_applied_or_refused_exactly_once() {
    use crate::core::messages::{AdmittedCommands, SystemControlPayload, SystemId};
    use crate::gm_action::{
        GmAction, GmActionGrant, GmActionJournal, GmActionKind, GmActionLog, GmActionOutcome,
        GmActionRefusalReason, LoggedGmAction,
    };

    for (with_impulse_owner, registration_mode, expected_outcome, expected_reason) in [
        (true, 1, GmActionOutcome::Applied, None),
        (
            false,
            1,
            GmActionOutcome::Refused,
            Some(GmActionRefusalReason::SystemRefused),
        ),
        (
            true,
            0,
            GmActionOutcome::Refused,
            Some(GmActionRefusalReason::SystemUnavailable),
        ),
        (
            true,
            2,
            GmActionOutcome::Refused,
            Some(GmActionRefusalReason::SystemUnavailable),
        ),
    ] {
        let order = crate::gm_action::GmActionOrder::new(crate::command_admission::HostSlot(2), 1);
        let correlation = crate::gm_action::GmActionId::new(if with_impulse_owner {
            "iframe-impulse-applied"
        } else {
            "iframe-impulse-refused"
        })
        .unwrap();
        let payload = SystemControlPayload::StartImpulseCharge;
        let grant = GmActionGrant {
            from: order.origin,
            sequenced_by: crate::command_admission::HostSlot(1),
            operator_id: "gm-2".into(),
            correlation: correlation.clone(),
            recovery_generation: 0,
            apply_tick: 12,
            order,
            action: GmAction::IssueStationCommand {
                ship: ShipKey("player-1".into()),
                station: StationId("helm".into()),
                target: SystemId("impulse-drive".into()),
                payload: crate::core::codec::canonical_system_command(&payload).unwrap(),
            },
        };
        let mut journal = GmActionJournal::default();
        journal.insert(grant).unwrap();
        journal
            .record_applied_result(LoggedGmAction {
                operator_id: "gm-2".into(),
                correlation: correlation.clone(),
                action_kind: GmActionKind::StationCommand,
                requested_active: true,
                outcome: GmActionOutcome::Pending,
                tick: 12,
                lever: None,
                reason: None,
                order: Some(order),
                target: None,
                effect: None,
                verb: None,
                effect_scope: None,
                objective_verb: None,
                objective_instance_scope: None,
                objective_recipients: None,
                comms_recipients: None,
                observer: None,
                npc_doctrine: None,
                affected: None,
                undo_of: None,
            })
            .unwrap();
        let provisional_log = journal.applied_log();

        let mut app = App::new();
        app.add_message::<crate::lobby::server::OutboundMessage>()
            .insert_resource(crate::sim_tick::SimTick(12))
            .insert_resource(journal)
            .insert_resource(provisional_log)
            .init_resource::<PendingGmStationCommands>()
            .init_resource::<PendingGmStationFeedbackRoutes>()
            .init_resource::<StationPuppetActivity>()
            .add_systems(
                Update,
                (
                    admit_station_puppet_commands,
                    crate::ship::helm_admission::process_helm_inputs,
                    settle_station_puppet_feedback,
                )
                    .chain(),
            );
        app.init_resource::<crate::command_admission::AdmittedConsumerRegistry>();
        if registration_mode != 0 {
            crate::console::helm::dispatch::register_helm_dispatch(&mut app);
        }
        if registration_mode == 2 {
            use crate::command_admission::{
                ConsumerMatcher, FeedbackAddress, RegisterAdmittedConsumer,
            };
            app.register_admitted_consumer(ConsumerMatcher::exact(crate::ship::system_registry::HELM_IMPULSE_KIND, "impulse-drive").with_feedback(FeedbackAddress::MatcherSpelling, &[crate::core::messages::SystemControlPayloadDiscriminants::StartImpulseCharge]));
        }
        let entity = app
            .world_mut()
            .spawn((
                crate::server_app::Ship,
                EntityUuid("player-1".into()),
                ShipConfigComponent(control_test_config()),
                AdmittedCommands::default(),
            ))
            .id();
        if with_impulse_owner {
            app.world_mut()
                .entity_mut(entity)
                .insert(crate::ship::helm::ImpulseCommand::default());
        }
        app.world_mut()
            .resource_mut::<PendingGmStationCommands>()
            .push(PendingGmStationCommand {
                tick: 12,
                order,
                operator_id: "gm-2".into(),
                correlation: correlation.clone(),
                ship: ShipKey("player-1".into()),
                station: StationId("helm".into()),
                target: SystemId("impulse-drive".into()),
                payload,
            });

        app.update();

        let result = &app.world().resource::<GmActionJournal>().applied_results()[0];
        assert_eq!(result.outcome, expected_outcome);
        assert_eq!(result.reason, expected_reason);
        assert_eq!(
            app.world().resource::<GmActionLog>().entries()[0],
            result.clone(),
        );
        assert_eq!(
            app.world()
                .resource::<PendingGmStationFeedbackRoutes>()
                .len(),
            0,
            "the authentic consumer's first terminal answer closes its route",
        );
        let admitted = app.world().get::<AdmittedCommands>(entity).unwrap();
        if registration_mode != 1 {
            assert!(
                admitted.0.is_empty(),
                "missing or ambiguous installed owner must refuse before admission"
            );
            assert!(app
                .world()
                .resource::<StationPuppetActivity>()
                .entries()
                .is_empty());
            let terminal = result.clone();
            app.update();
            assert_eq!(
                app.world().resource::<GmActionJournal>().applied_results()[0],
                terminal
            );
            continue;
        }
        assert_eq!(
            admitted.0[0]
                .feedback_correlation
                .as_ref()
                .map(|value| value.as_str()),
            Some(correlation.as_str()),
        );
        assert!(admitted.0[0]
            .response_token
            .as_deref()
            .is_some_and(|token| token.starts_with(GM_STATION_FEEDBACK_TOKEN_PREFIX)));

        // The admitted buffer intentionally remains populated in this
        // narrow fixture, so its authentic consumer emits the same reply a
        // second time.  With no route left, that duplicate is inert.
        let terminal = result.clone();
        app.update();
        assert_eq!(
            app.world().resource::<GmActionJournal>().applied_results()[0],
            terminal,
        );
        assert_eq!(
            app.world()
                .resource::<PendingGmStationFeedbackRoutes>()
                .len(),
            0,
        );
    }
}

#[test]
fn helm_station_admission_uses_the_set_view_effective_target_and_refuses_a_forged_station() {
    use crate::command_admission::{validate_station_command, StationCommandPolicyFailure};
    use crate::core::messages::{SystemControlPayload, SystemId, ViewMode};

    let config = control_test_config();
    let mut sources = ShipSystemControlSources::default();
    // Authentic takeover begins while Helm remains Backfill. The Station
    // authority substitution accepts AI-rated Systems but never Offline.
    sources
        .0
        .set(SystemId("helm-radar".into()), ControlSource::Ai);
    sources
        .0
        .set(SystemId("science-sensors".into()), ControlSource::Ai);
    let viewscreen = SystemId("main-view".into());
    let helm = StationId("helm".into());

    let admitted = validate_station_command(
        &helm,
        viewscreen.clone(),
        SystemControlPayload::SetView {
            mode: ViewMode::Radar,
        },
        &sources,
        &config,
        None,
    )
    .expect("Radar is authored from Helm's effective target");
    assert_eq!(admitted.target, viewscreen);
    assert!(admitted.response_token.is_none());

    assert_eq!(
        validate_station_command(
            &helm,
            SystemId("main-view".into()),
            SystemControlPayload::SetView {
                mode: ViewMode::ScienceRadar,
            },
            &sources,
            &config,
            None,
        ),
        Err(StationCommandPolicyFailure::SystemOutsideStation),
        "a Helm puppet cannot forge a view whose effective System belongs to Tactical",
    );

    sources
        .0
        .set(SystemId("helm-radar".into()), ControlSource::Offline);
    assert_eq!(
        validate_station_command(
            &helm,
            SystemId("main-view".into()),
            SystemControlPayload::SetView {
                mode: ViewMode::Radar,
            },
            &sources,
            &config,
            None,
        ),
        Err(StationCommandPolicyFailure::SystemUnavailable),
    );
}
