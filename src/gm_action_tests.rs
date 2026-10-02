use super::*;

#[test]
fn both_journal_policies_fold_every_latch_and_live_restore() {
    for initially_paused in [false, true] {
        let mut journal = GmActionJournal::default();
        journal.adopt_initial_pause(initially_paused);
        let actions = [
            GmAction::SetSessionPaused {
                active: !initially_paused,
            },
            GmAction::SetStationPuppet {
                ship: crate::command_admission::log::ShipKey("ship".into()),
                station: crate::core::messages::StationId("helm".into()),
                active: true,
            },
            GmAction::FireGmEvent {
                event: "world::fire".into(),
            },
            GmAction::SetEventPaused {
                event: "world::pause".into(),
                active: true,
            },
            GmAction::ArmGmEventSkip {
                event: "world::skip".into(),
            },
        ];
        for (index, action) in actions
            .into_iter()
            .flat_map(|action| [action.clone(), action])
            .enumerate()
        {
            let mut row = grant(1, index as u64 + 1, 1, &format!("latch-{index}"), true);
            row.action = action;
            journal.insert(row).unwrap();
        }
        let mut restore = grant(1, 11, 1, "restore", true);
        restore.action = GmAction::RequestLiveRestore {
            candidate: "candidate".into(),
        };
        journal.insert(restore).unwrap();
        let derived = journal.derived_log_prefix(11);
        assert!(derived.paused);
        assert_eq!(
            derived
                .entries
                .iter()
                .map(|row| row.outcome)
                .collect::<Vec<_>>(),
            [GmActionOutcome::Applied, GmActionOutcome::NoOp]
                .repeat(5)
                .into_iter()
                .chain([GmActionOutcome::Applied])
                .collect::<Vec<_>>()
        );
        assert!(derived.entries.iter().all(|row| row.affected.is_none()));
        assert_eq!(journal.log_prefix(11), derived);
        journal.applied_results = derived.entries.clone();
        assert_eq!(journal.log_prefix(11), derived);
    }
}

#[test]
fn derived_frontier_ignores_recorded_refusals_but_mixed_projection_retains_them() {
    let mut journal = GmActionJournal::default();
    for sequence in 1..=3 {
        journal
            .insert(fire_grant(
                sequence,
                1,
                &format!("fire-{sequence}"),
                "world::event",
            ))
            .unwrap();
    }
    let derived = journal.derived_log_prefix(3);
    let mut refused = derived.entries[0].clone();
    refused.outcome = GmActionOutcome::Refused;
    refused.reason = Some(GmActionRefusalReason::UnknownGmEvent);
    journal.applied_results = vec![refused.clone()];
    let mixed = journal.log_prefix(3);
    assert_eq!(mixed.entries[0], refused);
    assert_eq!(mixed.entries[1].outcome, GmActionOutcome::Applied);
    assert_eq!(mixed.entries[2].outcome, GmActionOutcome::NoOp);
    assert_eq!(journal.derived_log_prefix(3), derived);
    journal.restore_applied_frontier(3).unwrap();
    assert_eq!(journal.applied_log(), derived);
}

fn grant(
    slot: u32,
    sequence: u64,
    apply_tick: u64,
    correlation: &str,
    active: bool,
) -> GmActionGrant {
    let origin = HostSlot(slot);
    GmActionGrant {
        from: origin,
        sequenced_by: HostSlot(1),
        operator_id: format!("gm-{slot}"),
        correlation: GmActionId::new(correlation).unwrap(),
        recovery_generation: 0,
        apply_tick,
        order: GmActionOrder::new(origin, sequence),
        action: GmAction::SetSessionPaused { active },
    }
}

fn station_grant(
    sequence: u64,
    apply_tick: u64,
    correlation: &str,
    action: GmAction,
) -> GmActionGrant {
    GmActionGrant {
        from: HostSlot(1),
        sequenced_by: HostSlot(1),
        operator_id: "gm-1".into(),
        correlation: GmActionId::new(correlation).unwrap(),
        recovery_generation: 0,
        apply_tick,
        order: GmActionOrder::new(HostSlot(1), sequence),
        action,
    }
}

fn station_apply_app(
    tick: u64,
    ratings: crate::ship::components::ActiveStationRatings,
    puppets: crate::gm_puppet::StationPuppets,
    grants: impl IntoIterator<Item = GmActionGrant>,
) -> App {
    let config = crate::ship::config::ShipConfig::from_toml(
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

[[system]]
id = "helm-thrust"
kind = "helm_thrust"
station = "helm"
"#,
        &["helm_thrust"],
    )
    .unwrap();
    let mut sources = crate::ship::components::ShipSystemControlSources::default();
    sources.0.set(
        crate::core::messages::SystemId("helm-thrust".into()),
        crate::ship::control_source::ControlSource::Ai,
    );
    let mut journal = GmActionJournal::default();
    for grant in grants {
        journal.insert(grant).unwrap();
    }
    let mut app = App::new();
    app.insert_resource(crate::sim_tick::SimTick(tick))
        .insert_resource(SimulationPaused(false))
        .insert_resource(journal)
        .init_resource::<GmActionLog>()
        .insert_resource(puppets)
        .init_resource::<crate::gm_puppet::PendingGmStationCommands>()
        .add_systems(Update, apply_due_actions);
    app.world_mut().spawn((
        crate::server_app::Ship,
        crate::entities::spawner::EntityUuid("player-1".into()),
        crate::lockstep::FleetSlotOf(HostSlot(1)),
        crate::ship::components::ShipConfigComponent(config),
        ratings,
        sources,
    ));
    app
}

// -- Firing an authored GM event (issue #1301) ---------------------------

/// The same fixture with the Pause lever declared (issue #1303).
fn pausable_event_state(id: &str, pause: bool) -> crate::world::content::TriggerState {
    let mut state = manual_event_state(id, None, false, true);
    state.trigger.gm_controls.as_mut().expect("controls").pause = pause;
    state
}

fn pause_grant(
    sequence: u64,
    apply_tick: u64,
    operator: &str,
    correlation: &str,
    event: &str,
    active: bool,
) -> GmActionGrant {
    GmActionGrant {
        from: HostSlot(1),
        sequenced_by: HostSlot(1),
        operator_id: operator.into(),
        correlation: GmActionId::new(correlation).unwrap(),
        recovery_generation: 0,
        apply_tick,
        order: GmActionOrder::new(HostSlot(1), sequence),
        action: GmAction::SetEventPaused {
            event: event.into(),
            active,
        },
    }
}

fn paused(app: &App) -> Vec<String> {
    app.world()
        .resource::<crate::world::server::WorldContentRuntime>()
        .paused_gm_events
        .iter()
        .cloned()
        .collect()
}

fn manual_event_state(
    id: &str,
    layer: Option<&str>,
    repeat: bool,
    fire: bool,
) -> crate::world::content::TriggerState {
    let mut trigger =
        crate::world::config::scripted_trigger(crate::world::config::TriggerCondition::Manual);
    trigger.id = Some(id.to_string());
    trigger.repeat = repeat;
    let mut controls = crate::world::config::GmEventControls::fire_only(
        id.to_string(),
        format!("world.gm.event.{id}"),
    );
    controls.fire = fire;
    trigger.gm_controls = Some(controls);
    crate::world::content::TriggerState {
        trigger,
        fired: false,
        origin_layer: layer.map(str::to_string),
        seen_destroyed: Default::default(),
        last_fired_elapsed: None,
    }
}

fn fire_grant(sequence: u64, apply_tick: u64, correlation: &str, event: &str) -> GmActionGrant {
    GmActionGrant {
        from: HostSlot(1),
        sequenced_by: HostSlot(1),
        operator_id: "gm-1".into(),
        correlation: GmActionId::new(correlation).unwrap(),
        recovery_generation: 0,
        apply_tick,
        order: GmActionOrder::new(HostSlot(1), sequence),
        action: GmAction::FireGmEvent {
            event: event.into(),
        },
    }
}

fn fire_app(
    tick: u64,
    states: Vec<crate::world::content::TriggerState>,
    grants: impl IntoIterator<Item = GmActionGrant>,
) -> App {
    let mut journal = GmActionJournal::default();
    for grant in grants {
        journal.insert(grant).unwrap();
    }
    let mut runtime = crate::world::server::WorldContentRuntime::default();
    runtime.triggers.replace_declarative(states);
    let mut app = App::new();
    app.insert_resource(crate::sim_tick::SimTick(tick))
        .insert_resource(SimulationPaused(false))
        .insert_resource(journal)
        .init_resource::<GmActionLog>()
        .insert_resource(runtime)
        .add_systems(Update, apply_due_actions);
    app
}

fn outcomes(app: &App) -> Vec<(GmActionOutcome, Option<GmActionRefusalReason>)> {
    app.world()
        .resource::<GmActionJournal>()
        .applied_results()
        .iter()
        .map(|result| (result.outcome, result.reason))
        .collect()
}

fn armed(app: &App) -> Vec<String> {
    app.world()
        .resource::<crate::world::server::WorldContentRuntime>()
        .pending_gm_event_fires
        .iter()
        .cloned()
        .collect()
}

/// The whole idempotency contract in one run: the first Fire arms the
/// event, and a SECOND Fire -- a different correlation, so not a retry --
/// is a deterministic No-op rather than a second arm.
#[test]
fn a_second_fire_of_an_armed_event_is_a_deterministic_no_op() {
    let mut app = fire_app(
        5,
        vec![manual_event_state("breach", None, false, true)],
        [
            fire_grant(1, 5, "fire-a", "base-world::breach"),
            fire_grant(2, 5, "fire-b", "base-world::breach"),
        ],
    );
    app.update();

    assert_eq!(
        outcomes(&app),
        vec![
            (GmActionOutcome::Applied, None),
            (GmActionOutcome::NoOp, None)
        ]
    );
    assert_eq!(
        armed(&app),
        vec!["base-world::breach".to_string()],
        "one arm, no matter how many GMs pressed the button"
    );
}

/// A spent once-only event revalidates as a No-op at the apply boundary,
/// which is what stops a delayed grant re-running a lifecycle the trigger
/// pipeline has already consumed.
#[test]
fn firing_a_spent_one_shot_event_is_a_no_op_and_a_repeatable_one_re_arms() {
    let mut spent = manual_event_state("breach", None, false, true);
    spent.fired = true;
    let mut reusable = manual_event_state("scan", None, true, true);
    reusable.fired = true;
    let mut app = fire_app(
        9,
        vec![spent, reusable],
        [
            fire_grant(1, 9, "fire-a", "base-world::breach"),
            fire_grant(2, 9, "fire-b", "base-world::scan"),
        ],
    );
    app.update();

    assert_eq!(
        outcomes(&app),
        vec![
            (GmActionOutcome::NoOp, None),
            (GmActionOutcome::Applied, None)
        ]
    );
    assert_eq!(armed(&app), vec!["base-world::scan".to_string()]);
}

/// Unknown, wrong-layer and control-less ids are all the same answer: a
/// canonical refusal, decided against the LIVE table at the apply tick.
#[test]
fn a_fire_that_names_no_operable_event_is_refused_at_the_apply_boundary() {
    let mut app = fire_app(
        2,
        vec![
            manual_event_state("breach", Some("assets/worlds/layer.toml"), false, true),
            manual_event_state("locked", None, false, false),
        ],
        [
            fire_grant(1, 2, "fire-a", "base-world::missing"),
            // Right authored id, wrong layer.
            fire_grant(2, 2, "fire-b", "base-world::breach"),
            // Listed, but declares no Fire control.
            fire_grant(3, 2, "fire-c", "base-world::locked"),
        ],
    );
    app.update();

    assert_eq!(
        outcomes(&app),
        vec![
            (
                GmActionOutcome::Refused,
                Some(GmActionRefusalReason::UnknownGmEvent)
            ),
            (
                GmActionOutcome::Refused,
                Some(GmActionRefusalReason::UnknownGmEvent)
            ),
            (
                GmActionOutcome::Refused,
                Some(GmActionRefusalReason::UnknownGmEvent)
            ),
        ]
    );
    assert!(armed(&app).is_empty());
}

/// Every durable result names the event it fired, so the activity feed and
/// the mission panel can attribute it without re-reading the journal.
#[test]
fn a_fire_result_carries_its_stable_event_identity() {
    let mut app = fire_app(
        1,
        vec![manual_event_state("breach", None, false, true)],
        [fire_grant(1, 1, "fire-a", "base-world::breach")],
    );
    app.update();

    let journal = app.world().resource::<GmActionJournal>();
    let result = &journal.applied_results()[0];
    assert_eq!(result.action_kind, GmActionKind::EventControl);
    assert_eq!(result.target.as_deref(), Some("base-world::breach"));
    assert!(result.requested_active, "a Fire is always a request to act");
    // And the projection seam the mission panel reads selects exactly it.
    let projected = projected_results(
        GmActionKind::EventControl,
        &journal.applied_log(),
        &LocalGmActionRefusals::default(),
    );
    assert_eq!(projected.len(), 1);
    assert_eq!(projected[0].target.as_deref(), Some("base-world::breach"));
    assert!(
        projected_results(
            GmActionKind::SessionPause,
            &journal.applied_log(),
            &LocalGmActionRefusals::default()
        )
        .is_empty(),
        "an event result must not leak into the Session feed"
    );
}

/// A Fire refused BEFORE it could become a grant still names the event it
/// tried to fire, on both lanes that can refuse one: the local browser
/// ingress and the owner's canonical decision. Without the identity the
/// operator gets "someone fired ''" in the feed and the mission panel.
#[test]
fn a_refused_fire_still_names_the_event_on_both_refusal_lanes() {
    let fire = GmAction::FireGmEvent {
        event: "base-world::breach_alarm".into(),
    };

    // Ingress: the operator claim does not match the frozen slot binding,
    // exactly as `drain_gm_action_input` sees it in the browser.
    let mut world = admitted_world();
    let request = GmActionRequest {
        operator_id: "gm-2".into(),
        correlation: GmActionId::new("fire-a").unwrap(),
        action: fire.clone(),
    };
    assert_eq!(
        submit_local(&mut world, request.clone()),
        Err(GmActionRefusalReason::OperatorMismatch)
    );
    let ingress =
        LoggedGmAction::refused_request(&request, 10, GmActionRefusalReason::OperatorMismatch);
    assert_eq!(ingress.action_kind, GmActionKind::EventControl);
    assert_eq!(ingress.target.as_deref(), Some("base-world::breach_alarm"));

    // Owner lane: the same identity survives the replicated refusal frame
    // and the durable fact derived from it.
    let proposal = GmActionProposal {
        from: HostSlot(2),
        operator_id: "gm-1".into(),
        correlation: GmActionId::new("fire-b").unwrap(),
        action: fire,
    };
    let refusal = refusal_for(
        HostSlot(1),
        &proposal,
        11,
        GmActionRefusalReason::JournalFull,
    );
    assert_eq!(refusal.target.as_deref(), Some("base-world::breach_alarm"));
    assert_eq!(
        refusal.logged().target.as_deref(),
        Some("base-world::breach_alarm")
    );

    // And both reach the mission panel's feed as event results, not as
    // targetless rows the Session feed would have to explain.
    let mut refusals = LocalGmActionRefusals::default();
    refusals.push(ingress);
    refusals.push(refusal.logged());
    let projected = projected_results(
        GmActionKind::EventControl,
        &GmActionLog::default(),
        &refusals,
    );
    assert_eq!(projected.len(), 2);
    assert!(projected
        .iter()
        .all(|entry| entry.target.as_deref() == Some("base-world::breach_alarm")));
}

// -- Arming a Skip of the next occurrence (issue #1304) ------------------

/// The Skip lever's counterpart to `manual_event_state`: an ORDINARY
/// condition-bearing event, because a Skip stands in front of an automatic
/// occurrence and a `TriggerCondition::Manual` event has none.
fn skippable_event_state(
    id: &str,
    repeat: bool,
    skip: bool,
) -> crate::world::content::TriggerState {
    let mut state = manual_event_state(id, None, repeat, true);
    state.trigger.condition = crate::world::config::TriggerCondition::OnDestroyed {
        entity_name: "courier".to_string(),
    };
    state.trigger.gm_controls.as_mut().expect("controls").skip = skip;
    state
}

fn skip_grant(sequence: u64, apply_tick: u64, correlation: &str, event: &str) -> GmActionGrant {
    GmActionGrant {
        from: HostSlot(1),
        sequenced_by: HostSlot(1),
        operator_id: "gm-1".into(),
        correlation: GmActionId::new(correlation).unwrap(),
        recovery_generation: 0,
        apply_tick,
        order: GmActionOrder::new(HostSlot(1), sequence),
        action: GmAction::ArmGmEventSkip {
            event: event.into(),
        },
    }
}

fn armed_skips(app: &App) -> Vec<String> {
    app.world()
        .resource::<crate::world::server::WorldContentRuntime>()
        .pending_gm_event_skips
        .iter()
        .cloned()
        .collect()
}

/// The idempotency contract, in the exact words of the acceptance criterion:
/// repeated arm requests are deterministic and report Applied then No-op.
#[test]
fn a_second_skip_arm_of_the_same_event_is_a_deterministic_no_op() {
    let mut app = fire_app(
        5,
        vec![skippable_event_state("evac", false, true)],
        [
            skip_grant(1, 5, "skip-a", "base-world::evac"),
            skip_grant(2, 5, "skip-b", "base-world::evac"),
        ],
    );
    app.update();

    assert_eq!(
        outcomes(&app),
        vec![
            (GmActionOutcome::Applied, None),
            (GmActionOutcome::NoOp, None)
        ]
    );
    assert_eq!(
        armed_skips(&app),
        vec!["base-world::evac".to_string()],
        "one arm consumes one occurrence, no matter how many GMs pressed"
    );
}

/// Unknown ids and events that declare no Skip are the same answer, decided
/// against the LIVE table at the apply tick — `fireable_index`'s rule for
/// `skippable_index`, so an operator gets the same sentence for "nothing
/// answers to that name" and "that event has no such lever".
#[test]
fn a_skip_that_names_no_skippable_event_is_refused_at_the_apply_boundary() {
    let mut app = fire_app(
        2,
        vec![
            skippable_event_state("evac", false, true),
            // Listed and fireable, but declares no Skip control.
            skippable_event_state("lockdown", false, false),
        ],
        [
            skip_grant(1, 2, "skip-a", "base-world::missing"),
            skip_grant(2, 2, "skip-b", "base-world::lockdown"),
            skip_grant(3, 2, "skip-c", "base-world::evac"),
        ],
    );
    app.update();

    assert_eq!(
        outcomes(&app),
        vec![
            (
                GmActionOutcome::Refused,
                Some(GmActionRefusalReason::UnknownGmEvent)
            ),
            (
                GmActionOutcome::Refused,
                Some(GmActionRefusalReason::UnknownGmEvent)
            ),
            (GmActionOutcome::Applied, None),
        ]
    );
    assert_eq!(armed_skips(&app), vec!["base-world::evac".to_string()]);
}

/// A spent once-only event has no next occurrence to stand in front of, so
/// arming a Skip on it is a No-op; a repeatable one always has another.
#[test]
fn skipping_a_spent_one_shot_is_a_no_op_and_a_repeatable_one_arms() {
    let mut spent = skippable_event_state("evac", false, true);
    spent.fired = true;
    let mut reusable = skippable_event_state("sweep", true, true);
    reusable.fired = true;
    let mut app = fire_app(
        9,
        vec![spent, reusable],
        [
            skip_grant(1, 9, "skip-a", "base-world::evac"),
            skip_grant(2, 9, "skip-b", "base-world::sweep"),
        ],
    );
    app.update();

    assert_eq!(
        outcomes(&app),
        vec![
            (GmActionOutcome::NoOp, None),
            (GmActionOutcome::Applied, None)
        ]
    );
    assert_eq!(armed_skips(&app), vec!["base-world::sweep".to_string()]);
}

/// The two levers are orthogonal at the apply boundary as well as in the
/// pipeline: arming one never touches the other's set, and one event can
/// carry both arms at once.
#[test]
fn a_fire_and_a_skip_arm_two_independent_sets_on_one_event() {
    let mut app = fire_app(
        4,
        vec![skippable_event_state("evac", true, true)],
        [
            fire_grant(1, 4, "fire-a", "base-world::evac"),
            skip_grant(2, 4, "skip-a", "base-world::evac"),
        ],
    );
    app.update();

    assert_eq!(
        outcomes(&app),
        vec![
            (GmActionOutcome::Applied, None),
            (GmActionOutcome::Applied, None)
        ],
        "neither lever reduces the other to a No-op"
    );
    assert_eq!(armed(&app), vec!["base-world::evac".to_string()]);
    assert_eq!(armed_skips(&app), vec!["base-world::evac".to_string()]);
}

/// An armed Skip is untouched by the one Pause that exists on this branch:
/// the session pause reducer runs in the same PreUpdate pass and writes
/// nothing but its own flag. (#1303's per-event Pause is the other half of
/// the contract's "an armed skip survives Pause" and lands with that lever.)
#[test]
fn a_session_pause_leaves_an_armed_skip_exactly_where_it_was() {
    let pause = |sequence: u64, correlation: &str, active: bool| GmActionGrant {
        from: HostSlot(1),
        sequenced_by: HostSlot(1),
        operator_id: "gm-1".into(),
        correlation: GmActionId::new(correlation).unwrap(),
        recovery_generation: 0,
        apply_tick: 7,
        order: GmActionOrder::new(HostSlot(1), sequence),
        action: GmAction::SetSessionPaused { active },
    };
    let mut app = fire_app(
        7,
        vec![skippable_event_state("evac", false, true)],
        [
            skip_grant(1, 7, "skip-a", "base-world::evac"),
            pause(2, "pause-a", true),
            pause(3, "resume-a", false),
        ],
    );
    app.update();

    assert_eq!(
        outcomes(&app),
        vec![
            (GmActionOutcome::Applied, None),
            (GmActionOutcome::Applied, None),
            (GmActionOutcome::Applied, None),
        ]
    );
    assert_eq!(
        armed_skips(&app),
        vec!["base-world::evac".to_string()],
        "an armed Skip survives a Pause/Resume cycle"
    );
}

/// Every durable result says WHICH lever produced it, on every lane that
/// can produce one. Without it the activity feed reports a Skip as a Fire:
/// the opposite sentence about the same button.
#[test]
fn a_skip_result_carries_the_lever_on_the_grant_and_both_refusal_lanes() {
    let mut app = fire_app(
        1,
        vec![skippable_event_state("evac", false, true)],
        [
            skip_grant(1, 1, "skip-a", "base-world::evac"),
            fire_grant(2, 1, "fire-a", "base-world::evac"),
        ],
    );
    app.update();

    let journal = app.world().resource::<GmActionJournal>();
    let results = journal.applied_results();
    assert_eq!(results[0].action_kind, GmActionKind::EventControl);
    assert_eq!(
        results[0].lever,
        Some(crate::gm_event::GmEventLever::SkipNext)
    );
    assert_eq!(results[0].target.as_deref(), Some("base-world::evac"));
    assert_eq!(
        results[1].lever, None,
        "a Fire keeps the absent lever every pre-#1304 fact has"
    );
    // Both levers reach the mission panel's ONE result feed, because the
    // contract calls them levers of one control rather than two families.
    assert_eq!(
        projected_results(
            GmActionKind::EventControl,
            &journal.applied_log(),
            &LocalGmActionRefusals::default(),
        )
        .len(),
        2
    );

    let skip = GmAction::ArmGmEventSkip {
        event: "base-world::evac".into(),
    };
    // Ingress lane.
    let request = GmActionRequest {
        operator_id: "gm-2".into(),
        correlation: GmActionId::new("skip-b").unwrap(),
        action: skip.clone(),
    };
    let ingress =
        LoggedGmAction::refused_request(&request, 10, GmActionRefusalReason::OperatorMismatch);
    assert_eq!(ingress.action_kind, GmActionKind::EventControl);
    assert_eq!(ingress.target.as_deref(), Some("base-world::evac"));
    assert_eq!(ingress.lever, Some(crate::gm_event::GmEventLever::SkipNext));

    // Owner lane: the replicated refusal frame and the fact derived from it.
    let refusal = refusal_for(
        HostSlot(1),
        &GmActionProposal {
            from: HostSlot(2),
            operator_id: "gm-1".into(),
            correlation: GmActionId::new("skip-c").unwrap(),
            action: skip,
        },
        11,
        GmActionRefusalReason::JournalFull,
    );
    assert_eq!(refusal.lever, Some(crate::gm_event::GmEventLever::SkipNext));
    assert_eq!(
        refusal.logged().lever,
        Some(crate::gm_event::GmEventLever::SkipNext)
    );
}

/// The pure reducer reaches the same Applied/No-op ladder without a live
/// world, so a peer that reconstructs an applied frontier from grants alone
/// agrees with the one that watched them apply.
#[test]
fn the_pure_reducer_agrees_about_a_repeated_skip_arm() {
    let mut journal = GmActionJournal::default();
    journal
        .insert(skip_grant(1, 1, "skip-a", "base-world::evac"))
        .unwrap();
    journal
        .insert(skip_grant(2, 1, "skip-b", "base-world::evac"))
        .unwrap();
    journal
        .insert(skip_grant(3, 1, "skip-c", "base-world::sweep"))
        .unwrap();
    journal.restore_applied_frontier(3).unwrap();

    let log = journal.applied_log();
    assert_eq!(
        log.entries()
            .iter()
            .map(|entry| entry.outcome)
            .collect::<Vec<_>>(),
        vec![
            GmActionOutcome::Applied,
            GmActionOutcome::NoOp,
            GmActionOutcome::Applied,
        ]
    );
    assert!(log
        .entries()
        .iter()
        .all(|entry| entry.lever == Some(crate::gm_event::GmEventLever::SkipNext)));
}

/// A lever belongs to exactly one family: a Station or Pause refusal that
/// carries one is a malformed frame, not a fact to project.
#[test]
fn a_replicated_refusal_carrying_a_lever_outside_the_event_family_is_refused() {
    let roster = crate::lockstep::FleetRoster::with_participants_and_gms(
        Vec::new(),
        vec![HostSlot(1), HostSlot(2)],
        vec![crate::lockstep::FleetGm {
            host: HostSlot(2),
            operator_id: "gm-1".into(),
        }],
        HostSlot(1),
        HostSlot(1),
    )
    .unwrap();
    let mut refusal = refusal_for(
        HostSlot(1),
        &GmActionProposal {
            from: HostSlot(2),
            operator_id: "gm-1".into(),
            correlation: GmActionId::new("skip-d").unwrap(),
            action: GmAction::ArmGmEventSkip {
                event: "base-world::evac".into(),
            },
        },
        3,
        GmActionRefusalReason::JournalFull,
    );
    assert_eq!(refusal.lever, Some(crate::gm_event::GmEventLever::SkipNext));
    assert_eq!(
        validate_fleet_frame(&GmActionFrame::Refused(refusal.clone()), &roster),
        Ok(())
    );

    refusal.action_kind = GmActionKind::SessionPause;
    refusal.target = None;
    assert_eq!(
        validate_fleet_frame(&GmActionFrame::Refused(refusal), &roster),
        Err(GmActionRefusalReason::InvalidAction),
        "a lever on a family that pulls none could only publish a sentence \
             about an event nobody named"
    );
}

/// A world-less peer (the pure fixtures and the replay harness) refuses
/// rather than panicking or silently succeeding.
/// Issue #1303, the whole set-state contract in one run: the first Pause
/// applies, a REDUNDANT request for the state it is already in is a
/// deterministic No-op whichever GM makes it, and the matching Resume
/// applies. Absolute state, never a toggle — so two GMs pressing at the
/// same apply boundary commit the same answer on every peer regardless of
/// which arrived first.
#[test]
fn pausing_one_event_is_absolute_idempotent_and_attributed() {
    let mut app = fire_app(
        5,
        vec![pausable_event_state("breach", true)],
        [
            pause_grant(1, 5, "gm-1", "pause-1", "base-world::breach", true),
            pause_grant(2, 5, "gm-2", "pause-2", "base-world::breach", true),
            pause_grant(3, 5, "gm-1", "resume-1", "base-world::breach", false),
            pause_grant(4, 5, "gm-2", "resume-2", "base-world::breach", false),
        ],
    );
    app.update();

    assert_eq!(
        outcomes(&app),
        vec![
            (GmActionOutcome::Applied, None),
            (GmActionOutcome::NoOp, None),
            (GmActionOutcome::Applied, None),
            (GmActionOutcome::NoOp, None),
        ],
    );
    assert!(paused(&app).is_empty(), "the run ends resumed");

    // Every durable fact names the operator, the event and the LEVER, so a
    // feed can tell a Resume from a Fire of the same event.
    let results = app
        .world()
        .resource::<GmActionJournal>()
        .applied_results()
        .to_vec();
    assert_eq!(
        results
            .iter()
            .map(|r| (
                r.operator_id.as_str(),
                r.action_kind,
                r.verb,
                r.requested_active,
                r.target.as_deref()
            ))
            .collect::<Vec<_>>(),
        vec![
            (
                "gm-1",
                GmActionKind::EventControl,
                Some(GmEventVerb::Pause),
                true,
                Some("base-world::breach")
            ),
            (
                "gm-2",
                GmActionKind::EventControl,
                Some(GmEventVerb::Pause),
                true,
                Some("base-world::breach")
            ),
            (
                "gm-1",
                GmActionKind::EventControl,
                Some(GmEventVerb::Pause),
                false,
                Some("base-world::breach")
            ),
            (
                "gm-2",
                GmActionKind::EventControl,
                Some(GmEventVerb::Pause),
                false,
                Some("base-world::breach")
            ),
        ],
    );
}

/// The paused set survives between apply boundaries, and the Pause and Fire
/// levers are independent: a paused event still accepts a Fire, which is
/// the whole point of having both (the GM stopped the world's own trigger
/// and now chooses the moment themselves).
#[test]
fn a_paused_event_still_accepts_a_fire() {
    let mut app = fire_app(
        5,
        vec![pausable_event_state("breach", true)],
        [
            pause_grant(1, 5, "gm-1", "pause-1", "base-world::breach", true),
            fire_grant(2, 5, "fire-1", "base-world::breach"),
        ],
    );
    app.update();

    assert_eq!(
        outcomes(&app),
        vec![
            (GmActionOutcome::Applied, None),
            (GmActionOutcome::Applied, None),
        ],
    );
    assert_eq!(paused(&app), vec!["base-world::breach".to_string()]);
    assert_eq!(armed(&app), vec!["base-world::breach".to_string()]);
}

/// The absent-control refusal, at the apply boundary rather than at request
/// time: an event that declares no Pause lever, an id that names no live
/// event at all, and a run with no world are the same answer to a GM —
/// nothing here is pausable under that name at this tick.
#[test]
fn pausing_an_event_that_declares_no_pause_control_is_refused() {
    let mut app = fire_app(
        5,
        vec![pausable_event_state("breach", false)],
        [
            pause_grant(1, 5, "gm-1", "pause-1", "base-world::breach", true),
            pause_grant(2, 5, "gm-1", "pause-2", "base-world::missing", true),
        ],
    );
    app.update();

    assert_eq!(
        outcomes(&app),
        vec![
            (
                GmActionOutcome::Refused,
                Some(GmActionRefusalReason::UnknownGmEvent)
            ),
            (
                GmActionOutcome::Refused,
                Some(GmActionRefusalReason::UnknownGmEvent)
            ),
        ],
    );
    assert!(paused(&app).is_empty());

    // A refusal still names the event AND the lever, on both lanes that can
    // produce one before a grant exists.
    let refused = LoggedGmAction::refused_request(
        &GmActionRequest {
            operator_id: "gm-1".into(),
            correlation: GmActionId::new("pause-local").unwrap(),
            action: GmAction::SetEventPaused {
                event: "base-world::breach".into(),
                active: false,
            },
        },
        9,
        GmActionRefusalReason::WrongPhase,
    );
    assert_eq!(refused.target.as_deref(), Some("base-world::breach"));
    assert_eq!(refused.verb, Some(GmEventVerb::Pause));
    assert!(!refused.requested_active, "and which state it asked for");

    let owner = refusal_for(
        HostSlot(1),
        &GmActionProposal {
            from: HostSlot(2),
            operator_id: "gm-2".into(),
            correlation: GmActionId::new("pause-owner").unwrap(),
            action: GmAction::SetEventPaused {
                event: "base-world::breach".into(),
                active: true,
            },
        },
        9,
        GmActionRefusalReason::UnknownGmEvent,
    );
    assert_eq!(owner.target.as_deref(), Some("base-world::breach"));
    assert_eq!(owner.verb, Some(GmEventVerb::Pause));
    let roster = crate::lockstep::FleetRoster::with_participants_and_gms(
        Vec::new(),
        vec![HostSlot(1), HostSlot(2)],
        vec![
            crate::lockstep::FleetGm {
                host: HostSlot(1),
                operator_id: "gm-1".into(),
            },
            crate::lockstep::FleetGm {
                host: HostSlot(2),
                operator_id: "gm-2".into(),
            },
        ],
        HostSlot(1),
        HostSlot(1),
    )
    .unwrap();
    assert_eq!(
        validate_fleet_frame(&GmActionFrame::Refused(owner.clone()), &roster),
        Ok(())
    );
    // Stripped of its lever, the same frame could only be republished as a
    // refused FIRE on every other GM's feed, so it is malformed.
    let mut verbless = owner;
    verbless.verb = None;
    assert_eq!(
        validate_fleet_frame(&GmActionFrame::Refused(verbless), &roster),
        Err(GmActionRefusalReason::InvalidAction)
    );
}

/// A Pause with no world at all is refused rather than lost, exactly as a
/// Fire is: the reducer runs in the pure journal fixtures and the replay
/// harness without a `WorldContentRuntime`.
#[test]
fn a_pause_without_a_loaded_world_is_refused_rather_than_lost() {
    let mut journal = GmActionJournal::default();
    journal
        .insert(pause_grant(
            1,
            3,
            "gm-1",
            "pause-1",
            "base-world::breach",
            true,
        ))
        .unwrap();
    let mut app = App::new();
    app.insert_resource(crate::sim_tick::SimTick(3))
        .insert_resource(SimulationPaused(false))
        .insert_resource(journal)
        .init_resource::<GmActionLog>()
        .add_systems(Update, apply_due_actions);
    app.update();

    assert_eq!(
        outcomes(&app),
        vec![(
            GmActionOutcome::Refused,
            Some(GmActionRefusalReason::UnknownGmEvent)
        )],
    );
}

/// The pure reducer that reconstructs a log without live world state agrees
/// with the live one about which set-state requests changed anything —
/// which is what makes a restored frontier and a replayed one project the
/// same terminal facts.
#[test]
fn the_derived_reducer_folds_pause_state_the_same_way_the_live_one_does() {
    let mut journal = GmActionJournal::default();
    for grant in [
        pause_grant(1, 4, "gm-1", "pause-1", "base-world::breach", true),
        pause_grant(2, 4, "gm-2", "pause-2", "base-world::breach", true),
        pause_grant(3, 4, "gm-1", "pause-3", "base-world::sweep", true),
        pause_grant(4, 4, "gm-1", "resume-1", "base-world::breach", false),
        pause_grant(5, 4, "gm-1", "resume-2", "base-world::breach", false),
    ] {
        journal.insert(grant).unwrap();
    }
    journal.restore_applied_frontier(5).unwrap();
    let log = journal.applied_log();
    assert_eq!(
        log.entries()
            .iter()
            .map(|entry| entry.outcome)
            .collect::<Vec<_>>(),
        vec![
            GmActionOutcome::Applied,
            GmActionOutcome::NoOp,
            GmActionOutcome::Applied,
            GmActionOutcome::Applied,
            GmActionOutcome::NoOp,
        ],
    );
    assert!(
        log.entries()
            .iter()
            .all(|entry| entry.verb == Some(GmEventVerb::Pause)),
        "every derived fact still names the lever it replayed"
    );
    assert!(!log.paused(), "an event pause is not a session pause");
}

#[test]
fn a_fire_without_a_loaded_world_is_refused_rather_than_lost() {
    let mut journal = GmActionJournal::default();
    journal
        .insert(fire_grant(1, 3, "fire-a", "base-world::breach"))
        .unwrap();
    let mut app = App::new();
    app.insert_resource(crate::sim_tick::SimTick(3))
        .insert_resource(SimulationPaused(false))
        .insert_resource(journal)
        .init_resource::<GmActionLog>()
        .add_systems(Update, apply_due_actions);
    app.update();

    assert_eq!(
        outcomes(&app),
        vec![(
            GmActionOutcome::Refused,
            Some(GmActionRefusalReason::UnknownGmEvent)
        )]
    );
}

// ── GM palette placement (issue #1305) ──────────────────────────────

fn palette(id: &str, variants: &[&str]) -> crate::world::config::GmPaletteEntry {
    crate::world::config::GmPaletteEntry {
        id: id.to_string(),
        label: format!("world.gm.palette.{id}.label"),
        template_path: format!("assets/entities/{id}.toml"),
        name_prefix: None,
        groups: vec!["gm_placed".to_string()],
        variants: variants
            .iter()
            .map(|variant| crate::world::config::GmPaletteVariant {
                id: (*variant).to_string(),
                label: format!("world.gm.palette.{id}.{variant}.label"),
                overrides: None,
            })
            .collect(),
    }
}

fn place_grant(
    sequence: u64,
    apply_tick: u64,
    correlation: &str,
    entry: &str,
    variant: Option<&str>,
    position_mm: [i64; 3],
    heading_mdeg: i32,
) -> GmActionGrant {
    GmActionGrant {
        from: HostSlot(1),
        sequenced_by: HostSlot(1),
        operator_id: "gm-1".into(),
        correlation: GmActionId::new(correlation).unwrap(),
        recovery_generation: 0,
        apply_tick,
        order: GmActionOrder::new(HostSlot(1), sequence),
        action: GmAction::SpawnPaletteEntity {
            palette: entry.into(),
            variant: variant.map(str::to_string),
            position_mm,
            heading_mdeg,
        },
    }
}

fn place_app(
    tick: u64,
    entries: Vec<crate::world::config::GmPaletteEntry>,
    grants: impl IntoIterator<Item = GmActionGrant>,
) -> App {
    let mut journal = GmActionJournal::default();
    for grant in grants {
        journal.insert(grant).unwrap();
    }
    let runtime = crate::world::server::WorldContentRuntime {
        gm_palette: entries,
        ..Default::default()
    };
    let mut app = App::new();
    app.insert_resource(crate::sim_tick::SimTick(tick))
        .insert_resource(SimulationPaused(false))
        .insert_resource(journal)
        .init_resource::<GmActionLog>()
        .insert_resource(runtime)
        .add_systems(Update, apply_due_actions);
    app
}

fn placements(app: &App) -> Vec<crate::gm_spawn::PendingGmSpawn> {
    app.world()
        .resource::<crate::world::server::WorldContentRuntime>()
        .pending_gm_spawns
        .clone()
}

/// The happy path, and the one thing a placement must NOT share with a
/// Fire: two presses of the same palette entry are two hulls, not one arm.
/// Their names come from the canonical sequence, so every peer -- including
/// one that restored mid-run -- agrees which is which.
#[test]
fn a_gm_placement_arms_the_ordinary_spawn_and_two_presses_are_two_hulls() {
    let mut app = place_app(
        5,
        vec![palette("raider", &[])],
        [
            place_grant(
                1,
                5,
                "place-a",
                "raider",
                None,
                [120_000, 0, -40_000],
                90_000,
            ),
            place_grant(2, 5, "place-b", "raider", None, [0, 0, 0], 0),
        ],
    );
    app.update();

    assert_eq!(
        outcomes(&app),
        vec![
            (GmActionOutcome::Applied, None),
            (GmActionOutcome::Applied, None)
        ]
    );
    let armed = placements(&app);
    assert_eq!(armed.len(), 2, "each press places its own hull");
    assert_eq!(armed[0].name, "raider_1");
    assert_eq!(armed[1].name, "raider_2");
    assert_eq!(armed[0].position_mm, [120_000, 0, -40_000]);
    assert_eq!(armed[0].heading_mdeg, 90_000);
}

/// The palette IS the vocabulary: an id nothing authors, and a variant the
/// named entry never declared, are the same refusal -- decided against the
/// LIVE table at the apply tick, not at request time.
#[test]
fn a_placement_outside_the_authored_palette_is_refused_at_the_apply_boundary() {
    let mut app = place_app(
        7,
        vec![palette("raider", &["blood_eagle"])],
        [
            place_grant(1, 7, "place-a", "tender", None, [0, 0, 0], 0),
            place_grant(2, 7, "place-b", "raider", Some("iron_spear"), [0, 0, 0], 0),
            place_grant(
                3,
                7,
                "place-c",
                "assets/entities/ship_harrow_cruiser.toml",
                None,
                [0, 0, 0],
                0,
            ),
            place_grant(4, 7, "place-d", "raider", Some("blood_eagle"), [0, 0, 0], 0),
        ],
    );
    app.update();

    assert_eq!(
        outcomes(&app),
        vec![
            (
                GmActionOutcome::Refused,
                Some(GmActionRefusalReason::UnknownGmPaletteEntry)
            ),
            (
                GmActionOutcome::Refused,
                Some(GmActionRefusalReason::UnknownGmPaletteEntry)
            ),
            (
                GmActionOutcome::Refused,
                Some(GmActionRefusalReason::UnknownGmPaletteEntry)
            ),
            (GmActionOutcome::Applied, None),
        ],
        "an asset path is not a palette id, and neither is an unauthored variant"
    );
    assert_eq!(placements(&app).len(), 1);
}

/// A placement outside the coordinate bound is refused on BOTH authorities
/// that can see it -- the browser's own proposal, and the canonical journal
/// -- so no such grant ever reaches the apply boundary on any peer.
#[test]
fn an_out_of_range_placement_is_refused_before_it_can_become_a_grant() {
    let far = crate::gm_spawn::MAX_GM_SPAWN_COORD_MM + 1;
    let action = GmAction::SpawnPaletteEntity {
        palette: "raider".into(),
        variant: None,
        position_mm: [far, 0, 0],
        heading_mdeg: 0,
    };
    assert_eq!(
        GmActionProposal {
            from: HostSlot(1),
            operator_id: "gm-1".into(),
            correlation: GmActionId::new("place-a").unwrap(),
            action: action.clone(),
        }
        .validate(),
        Err(GmActionRefusalReason::InvalidAction)
    );

    let mut journal = GmActionJournal::default();
    assert!(
        journal
            .insert(place_grant(1, 3, "place-a", "raider", None, [far, 0, 0], 0))
            .is_err(),
        "the canonical journal never holds a grant whose action does not validate"
    );

    // And the heading bound, on the same two authorities.
    assert!(!crate::gm_spawn::placement_is_valid(
        [0, 0, 0],
        crate::gm_spawn::MAX_GM_SPAWN_HEADING_MDEG + 1
    ));
}

/// No world at all: refused under the operator's own correlation rather
/// than armed against a queue nothing will ever drain.
#[test]
fn a_placement_without_a_loaded_world_is_refused_rather_than_lost() {
    let mut journal = GmActionJournal::default();
    journal
        .insert(place_grant(1, 3, "place-a", "raider", None, [0, 0, 0], 0))
        .unwrap();
    let mut app = App::new();
    app.insert_resource(crate::sim_tick::SimTick(3))
        .insert_resource(SimulationPaused(false))
        .insert_resource(journal)
        .init_resource::<GmActionLog>()
        .add_systems(Update, apply_due_actions);
    app.update();

    assert_eq!(
        outcomes(&app),
        vec![(
            GmActionOutcome::Refused,
            Some(GmActionRefusalReason::WorldUnavailable)
        )]
    );
}

/// Every durable fact a placement produces names WHAT was placed, on both
/// lanes that can produce one -- so the activity feed and the panel can say
/// so without re-reading the journal, and `validate_fleet_frame` accepts
/// the replicated refusal.
#[test]
fn a_placement_result_carries_its_stable_palette_identity() {
    let mut app = place_app(
        2,
        vec![palette("raider", &[])],
        [place_grant(1, 2, "place-a", "raider", None, [0, 0, 0], 0)],
    );
    app.update();
    let applied = app.world().resource::<GmActionJournal>().applied_results()[0].clone();
    assert_eq!(applied.action_kind, GmActionKind::WorldSpawn);
    assert_eq!(applied.target.as_deref(), Some("raider"));

    let request = GmActionRequest {
        operator_id: "gm-1".into(),
        correlation: GmActionId::new("place-b").unwrap(),
        action: GmAction::SpawnPaletteEntity {
            palette: "raider".into(),
            variant: None,
            position_mm: [0, 0, 0],
            heading_mdeg: 0,
        },
    };
    let ingress = LoggedGmAction::refused_request(&request, 4, GmActionRefusalReason::WrongPhase);
    assert_eq!(ingress.target.as_deref(), Some("raider"));

    // The frame rule is per-family, not "event control or nothing": a
    // world-spawn refusal MUST name its palette entry and a pause refusal
    // must not name anything.
    assert!(GmActionKind::WorldSpawn.carries_target());
    assert!(!GmActionKind::SessionPause.carries_target());
}

#[test]
fn delayed_takeover_accepts_a_holder_reconnecting_before_the_apply_boundary() {
    let helm = crate::core::messages::StationId("helm".into());
    let ship = crate::command_admission::ShipKey("player-1".into());
    let action = GmAction::SetStationPuppet {
        ship: ship.clone(),
        station: helm.clone(),
        active: true,
    };
    let mut sequenced_ratings = crate::ship::components::ActiveStationRatings::default();
    sequenced_ratings
        .0
        .insert(helm.clone(), crate::ship::rating::BACKFILL_RATING.into());
    // The sequencing snapshot was valid while the holder was disconnected.
    // A reconnect changes the live rating before the delayed grant is due.
    let mut current_ratings = sequenced_ratings;
    current_ratings.0.insert(helm.clone(), "Manual".into());
    let mut app = station_apply_app(
        12,
        current_ratings,
        crate::gm_puppet::StationPuppets::default(),
        [station_grant(1, 12, "delayed-takeover", action)],
    );
    app.update();

    let target = crate::gm_puppet::StationPuppetTarget::new(ship, helm);
    assert!(app
        .world()
        .resource::<crate::gm_puppet::StationPuppets>()
        .is_active(&target));
    let entry = &app.world().resource::<GmActionLog>().entries()[0];
    assert_eq!(entry.outcome, GmActionOutcome::Applied);
    assert_eq!(entry.reason, None);
    assert_eq!(
        app.world().resource::<GmActionJournal>().applied_results(),
        app.world().resource::<GmActionLog>().entries(),
        "the shared takeover is durable journal state",
    );
}

#[test]
fn recovery_generation_keeps_old_work_stale_after_rejoin_and_accepts_new_work() {
    let helm = crate::core::messages::StationId("helm".into());
    let ship = crate::command_admission::ShipKey("player-1".into());
    let mut ratings = crate::ship::components::ActiveStationRatings::default();
    ratings
        .0
        .insert(helm.clone(), crate::ship::rating::BACKFILL_RATING.into());
    let mut new_takeover = station_grant(
        2,
        13,
        "new-incarnation-takeover",
        GmAction::SetStationPuppet {
            ship: ship.clone(),
            station: helm.clone(),
            active: true,
        },
    );
    new_takeover.recovery_generation = 1;
    let mut app = station_apply_app(
        12,
        ratings,
        crate::gm_puppet::StationPuppets::default(),
        [
            station_grant(
                1,
                12,
                "old-incarnation-takeover",
                GmAction::SetStationPuppet {
                    ship: ship.clone(),
                    station: helm.clone(),
                    active: true,
                },
            ),
            new_takeover,
        ],
    );
    app.world_mut()
        .resource_mut::<GmActionJournal>()
        .record_slot_recovery(HostSlot(1), 10)
        .unwrap();
    let mut session = crate::lockstep::LockstepSession::new(HostSlot(2), [HostSlot(1)], 0);
    session.depart(HostSlot(1));
    session.rejoin(HostSlot(1), 10);
    assert!(!session.has_departed(HostSlot(1)), "fixture crossed rejoin");
    app.world_mut()
        .insert_resource(crate::lockstep::FleetLockstep(session));

    app.update();
    let first = &app.world().resource::<GmActionLog>().entries()[0];
    assert_eq!(first.outcome, GmActionOutcome::Refused);
    assert_eq!(first.reason, Some(GmActionRefusalReason::NotGameMaster));

    app.world_mut().resource_mut::<crate::sim_tick::SimTick>().0 = 13;
    app.update();
    assert_eq!(
        app.world()
            .resource::<GmActionLog>()
            .entries()
            .iter()
            .map(|entry| (entry.outcome, entry.reason))
            .collect::<Vec<_>>(),
        vec![
            (
                GmActionOutcome::Refused,
                Some(GmActionRefusalReason::NotGameMaster),
            ),
            (GmActionOutcome::Applied, None),
        ],
    );
    assert!(app
        .world()
        .resource::<crate::gm_puppet::StationPuppets>()
        .is_active(&crate::gm_puppet::StationPuppetTarget::new(ship, helm)));
}

#[test]
fn recovery_boundary_preserves_same_tick_order_and_round_trips() {
    let helm = crate::core::messages::StationId("helm".into());
    let ship = crate::command_admission::ShipKey("player-1".into());
    let mut ratings = crate::ship::components::ActiveStationRatings::default();
    ratings
        .0
        .insert(helm.clone(), crate::ship::rating::BACKFILL_RATING.into());
    let mut recovered_release = station_grant(
        2,
        10,
        "recovered-boundary-release",
        GmAction::SetStationPuppet {
            ship: ship.clone(),
            station: helm.clone(),
            active: false,
        },
    );
    recovered_release.recovery_generation = 1;
    let mut app = station_apply_app(
        10,
        ratings,
        crate::gm_puppet::StationPuppets::default(),
        [
            station_grant(
                1,
                10,
                "old-boundary-takeover",
                GmAction::SetStationPuppet {
                    ship: ship.clone(),
                    station: helm.clone(),
                    active: true,
                },
            ),
            recovered_release,
        ],
    );
    app.world_mut()
        .resource_mut::<GmActionJournal>()
        .record_slot_recovery(HostSlot(1), 10)
        .unwrap();
    app.update();
    assert_eq!(
        app.world()
            .resource::<GmActionLog>()
            .entries()
            .iter()
            .map(|entry| entry.outcome)
            .collect::<Vec<_>>(),
        [GmActionOutcome::Applied, GmActionOutcome::Applied],
    );
    assert!(!app
        .world()
        .resource::<crate::gm_puppet::StationPuppets>()
        .is_active(&crate::gm_puppet::StationPuppetTarget::new(ship, helm)));

    let journal = app.world().resource::<GmActionJournal>();
    let text = ron::to_string(journal).unwrap();
    let restored: GmActionJournal = ron::from_str(&text).unwrap();
    assert_eq!(&restored, journal);
}

#[test]
fn owner_stamps_new_work_with_the_recovery_generation_and_boundary() {
    let mut journal = GmActionJournal::default();
    assert_eq!(journal.record_slot_recovery(HostSlot(2), 30), Ok(1));
    let proposal = GmActionProposal {
        from: HostSlot(2),
        operator_id: "gm-2".into(),
        correlation: GmActionId::new("after-recovery").unwrap(),
        action: GmAction::SetSessionPaused { active: true },
    };
    let grant = sequence_owner_proposal(
        &mut journal,
        &proposal,
        HostSlot(1),
        20,
        20,
        false,
        false,
        false,
    )
    .unwrap();
    assert_eq!(grant.recovery_generation, 1);
    assert_eq!(grant.apply_tick, 30);
}

#[test]
fn same_tick_station_command_and_release_keep_canonical_admission_order() {
    use crate::core::messages::{StationId, SystemControlPayload, SystemId};

    let ship = crate::command_admission::ShipKey("player-1".into());
    let helm = StationId("helm".into());
    let puppet_target = crate::gm_puppet::StationPuppetTarget::new(ship.clone(), helm.clone());
    let command = || GmAction::IssueStationCommand {
        ship: ship.clone(),
        station: helm.clone(),
        target: SystemId("helm-thrust".into()),
        payload: crate::core::codec::canonical_system_command(&SystemControlPayload::SetThrust {
            value: 0.75,
        })
        .unwrap(),
    };
    let release = || GmAction::SetStationPuppet {
        ship: ship.clone(),
        station: helm.clone(),
        active: false,
    };
    let mut ratings = crate::ship::components::ActiveStationRatings::default();
    ratings
        .0
        .insert(helm.clone(), crate::ship::rating::BACKFILL_RATING.into());

    let mut puppets = crate::gm_puppet::StationPuppets::default();
    puppets.set_operator(puppet_target.clone(), "gm-1".into(), true);
    let mut command_first = station_apply_app(
        20,
        ratings.clone(),
        puppets.clone(),
        [
            station_grant(1, 20, "command-first", command()),
            station_grant(2, 20, "release-second", release()),
        ],
    );
    command_first.update();
    assert_eq!(
        command_first
            .world()
            .resource::<crate::gm_puppet::PendingGmStationCommands>()
            .entries()
            .len(),
        1,
        "a later same-tick release cannot retroactively drop an admitted command",
    );
    assert!(!command_first
        .world()
        .resource::<crate::gm_puppet::StationPuppets>()
        .is_active(&puppet_target));
    assert_eq!(
        command_first
            .world()
            .resource::<GmActionLog>()
            .entries()
            .iter()
            .map(|entry| (entry.outcome, entry.reason))
            .collect::<Vec<_>>(),
        vec![
            (GmActionOutcome::Applied, None),
            (GmActionOutcome::Applied, None),
        ],
    );

    let mut release_first = station_apply_app(
        20,
        ratings,
        puppets,
        [
            station_grant(1, 20, "release-first", release()),
            station_grant(2, 20, "command-second", command()),
        ],
    );
    release_first.update();
    assert!(release_first
        .world()
        .resource::<crate::gm_puppet::PendingGmStationCommands>()
        .entries()
        .is_empty());
    assert_eq!(
        release_first
            .world()
            .resource::<GmActionLog>()
            .entries()
            .iter()
            .map(|entry| (entry.outcome, entry.reason))
            .collect::<Vec<_>>(),
        vec![
            (GmActionOutcome::Applied, None),
            (
                GmActionOutcome::Refused,
                Some(GmActionRefusalReason::StationNotPuppeted),
            ),
        ],
    );
}

#[test]
fn applied_no_op_and_resume_are_explicit_terminal_facts() {
    let mut journal = GmActionJournal::default();
    journal.insert(grant(1, 1, 10, "pause", true)).unwrap();
    journal.insert(grant(1, 2, 10, "duplicate", true)).unwrap();
    journal.insert(grant(1, 3, 10, "resume", false)).unwrap();

    let log = journal.log_through(10);
    assert!(!log.paused());
    assert_eq!(
        log.entries()
            .iter()
            .map(|entry| entry.outcome)
            .collect::<Vec<_>>(),
        vec![
            GmActionOutcome::Applied,
            GmActionOutcome::NoOp,
            GmActionOutcome::Applied,
        ]
    );
}

#[test]
fn owner_closes_a_released_boundary_before_a_late_concurrent_proposal() {
    let owner = HostSlot(1);
    let mut canonical = GmActionJournal::default();
    canonical.adopt_initial_pause(true);
    let resume = GmActionProposal {
        from: HostSlot(1),
        operator_id: "gm-1".into(),
        correlation: GmActionId::new("resume").unwrap(),
        action: GmAction::SetSessionPaused { active: false },
    };
    let resume =
        sequence_owner_proposal(&mut canonical, &resume, owner, 20, 20, true, false, false)
            .expect("owner sequences resume");
    assert_eq!(resume.apply_tick, 20);

    // This proposal was concurrent in product time but reached the owner
    // after the releasing commit. It cannot mutate boundary 20 retroactively.
    let late_pause = GmActionProposal {
        from: HostSlot(2),
        operator_id: "gm-2".into(),
        correlation: GmActionId::new("late-pause").unwrap(),
        action: GmAction::SetSessionPaused { active: true },
    };
    let late_pause = sequence_owner_proposal(
        &mut canonical,
        &late_pause,
        owner,
        20,
        20,
        true,
        false,
        false,
    )
    .expect("owner sequences late proposal");
    assert_eq!(late_pause.apply_tick, 21);

    let mut early_delivery = GmActionJournal::default();
    early_delivery.adopt_initial_pause(true);
    early_delivery.insert(resume.clone()).unwrap();
    assert!(!early_delivery.log_through(20).paused());
    early_delivery.insert(late_pause.clone()).unwrap();

    let mut batched_delivery = GmActionJournal::default();
    batched_delivery.adopt_initial_pause(true);
    batched_delivery.insert(resume).unwrap();
    batched_delivery.insert(late_pause).unwrap();
    assert_eq!(early_delivery, batched_delivery);
    assert!(!early_delivery.log_through(20).paused());
    assert!(early_delivery.log_through(21).paused());
}

#[test]
fn a_replicated_first_resume_adopts_the_same_paused_baseline_as_the_owner() {
    let owner = HostSlot(1);
    let proposal = GmActionProposal {
        from: HostSlot(2),
        operator_id: "gm-2".into(),
        correlation: GmActionId::new("resume-technical-hold").unwrap(),
        action: GmAction::SetSessionPaused { active: false },
    };
    let mut canonical = GmActionJournal::default();
    let resume =
        sequence_owner_proposal(&mut canonical, &proposal, owner, 20, 20, true, true, false)
            .expect("the owner sequences the typed Resume at the stopped boundary");

    let mut receiver = GmActionJournal::default();
    insert_replicated_grant(&mut receiver, true, resume)
        .expect("the authenticated owner grant is admitted");

    assert_eq!(receiver, canonical);
    assert_eq!(
        receiver.log_through(20).entries()[0].outcome,
        GmActionOutcome::Applied
    );
    assert!(!receiver.log_through(20).paused());
}

#[test]
fn technical_join_hold_sequences_resume_at_the_stopped_tick() {
    let owner = HostSlot(1);
    let mut canonical = GmActionJournal::default();
    canonical
        .insert(grant(2, 1, 10, "old-pause", true))
        .unwrap();
    canonical
        .insert(grant(2, 2, 10, "old-resume", false))
        .unwrap();
    let resume = GmActionProposal {
        from: HostSlot(3),
        operator_id: "gm-3".into(),
        correlation: GmActionId::new("join-hold-resume").unwrap(),
        action: GmAction::SetSessionPaused { active: false },
    };

    let grant = sequence_owner_proposal(&mut canonical, &resume, owner, 42, 99, true, true, false)
        .expect("the technical hold accepts an explicit Resume");
    assert_eq!(
        grant.apply_tick, 42,
        "the action cannot wait for a future tick the join hold forbids"
    );
}

#[test]
fn exact_wire_retransmission_is_inert_but_key_reuse_is_refused() {
    let mut journal = GmActionJournal::default();
    let original = grant(1, 1, 10, "same", true);
    assert_eq!(
        journal.insert(original.clone()),
        Ok(GmActionInsert::Inserted)
    );
    assert_eq!(
        journal.insert(original.clone()),
        Ok(GmActionInsert::Duplicate)
    );
    let mut conflicting = original;
    conflicting.action = GmAction::SetSessionPaused { active: false };
    assert_eq!(
        journal.insert(conflicting),
        Err(GmActionRefusalReason::ConflictingGrant)
    );
    assert_eq!(journal.len(), 1);
}

#[test]
fn contiguous_owner_sequence_cannot_move_its_apply_boundary_backwards() {
    let mut journal = GmActionJournal::default();
    journal
        .insert(grant(1, 1, 10, "first-boundary", true))
        .unwrap();
    assert_eq!(
        journal.insert(grant(1, 2, 9, "backwards-boundary", false)),
        Err(GmActionRefusalReason::NonContiguousSequence)
    );
    assert_eq!(journal.len(), 1);
}

#[test]
fn correlation_is_an_operator_scoped_idempotency_key() {
    let mut journal = GmActionJournal::default();
    let original = grant(1, 1, 10, "same", true);
    journal.insert(original).unwrap();

    let same_operator_new_order = grant(1, 2, 11, "same", true);
    assert_eq!(
        journal.insert(same_operator_new_order),
        Err(GmActionRefusalReason::ConflictingGrant)
    );

    let same_text_other_operator = grant(2, 2, 11, "same", false);
    assert_eq!(
        journal.insert(same_text_other_operator),
        Ok(GmActionInsert::Inserted)
    );
    assert_eq!(journal.len(), 2);
}

#[test]
fn future_grant_remains_pending_until_its_exact_tick() {
    let mut journal = GmActionJournal::default();
    journal.insert(grant(1, 1, 33, "future", true)).unwrap();
    assert!(journal.log_through(32).entries().is_empty());
    assert!(!journal.log_through(32).paused());
    assert!(journal.log_through(33).paused());
}

#[test]
fn proposal_grant_and_refusal_have_distinct_frozen_authorities() {
    let roster = crate::lockstep::FleetRoster::with_participants_and_gms(
        Vec::new(),
        vec![HostSlot(1), HostSlot(2)],
        vec![
            crate::lockstep::FleetGm {
                host: HostSlot(1),
                operator_id: "gm-1".into(),
            },
            crate::lockstep::FleetGm {
                host: HostSlot(2),
                operator_id: "gm-2".into(),
            },
        ],
        HostSlot(1),
        HostSlot(1),
    )
    .unwrap();
    let proposal = GmActionProposal {
        from: HostSlot(2),
        operator_id: "gm-2".into(),
        correlation: GmActionId::new("auth-proposal").unwrap(),
        action: GmAction::SetSessionPaused { active: true },
    };
    assert_eq!(
        validate_fleet_frame(&GmActionFrame::Proposal(proposal.clone()), &roster),
        Ok(())
    );
    let mut forged_proposal = proposal.clone();
    forged_proposal.operator_id = "gm-1".into();
    assert_eq!(
        validate_fleet_frame(&GmActionFrame::Proposal(forged_proposal), &roster),
        Err(GmActionRefusalReason::OperatorMismatch)
    );

    let granted = GmActionGrant {
        from: proposal.from,
        sequenced_by: HostSlot(1),
        operator_id: proposal.operator_id.clone(),
        correlation: proposal.correlation.clone(),
        recovery_generation: 0,
        apply_tick: 7,
        order: GmActionOrder::new(proposal.from, 1),
        action: proposal.action.clone(),
    };
    assert_eq!(
        validate_fleet_frame(&GmActionFrame::Granted(granted.clone()), &roster),
        Ok(())
    );
    let mut forged_grant = granted.clone();
    forged_grant.sequenced_by = HostSlot(2);
    assert_eq!(
        validate_fleet_frame(&GmActionFrame::Granted(forged_grant), &roster),
        Err(GmActionRefusalReason::OriginMismatch)
    );

    let refused = GmActionRefusal {
        sequenced_by: HostSlot(1),
        requester: HostSlot(2),
        operator_id: "gm-2".into(),
        correlation: GmActionId::new("auth-refusal").unwrap(),
        effect_scope: None,
        objective_verb: None,
        objective_instance_scope: None,
        objective_recipients: None,
        comms_recipients: None,
        observer: None,
        npc_doctrine: None,
        action_kind: GmActionKind::SessionPause,
        requested_active: true,
        tick: 7,
        reason: GmActionRefusalReason::JournalFull,
        target: None,
        verb: None,
        lever: None,
    };
    assert_eq!(
        validate_fleet_frame(&GmActionFrame::Refused(refused.clone()), &roster),
        Ok(())
    );
    // A replicated refusal must carry exactly the target its family has:
    // an event-control refusal names its event, and no other family does.
    let mut targetless_fire = refused.clone();
    targetless_fire.action_kind = GmActionKind::EventControl;
    assert_eq!(
        validate_fleet_frame(&GmActionFrame::Refused(targetless_fire.clone()), &roster),
        Err(GmActionRefusalReason::InvalidAction)
    );
    targetless_fire.target = Some("base-world::breach_alarm".into());
    // And the LEVER travels with the family too (issue #1303): without it
    // this frame could only be republished as a refused Fire.
    assert_eq!(
        validate_fleet_frame(&GmActionFrame::Refused(targetless_fire.clone()), &roster),
        Err(GmActionRefusalReason::InvalidAction)
    );
    targetless_fire.verb = Some(GmEventVerb::Fire);
    assert_eq!(
        validate_fleet_frame(&GmActionFrame::Refused(targetless_fire.clone()), &roster),
        Ok(())
    );
    let mut refused_pause = targetless_fire.clone();
    refused_pause.verb = Some(GmEventVerb::Pause);
    refused_pause.requested_active = false;
    assert_eq!(
        validate_fleet_frame(&GmActionFrame::Refused(refused_pause), &roster),
        Ok(())
    );
    let mut targeted_pause = refused.clone();
    targeted_pause.target = Some("base-world::breach_alarm".into());
    assert_eq!(
        validate_fleet_frame(&GmActionFrame::Refused(targeted_pause), &roster),
        Err(GmActionRefusalReason::InvalidAction)
    );
    // A session-pause refusal carrying an event-control lever invents a
    // control its family does not have.
    let mut levered_session_pause = refused.clone();
    levered_session_pause.verb = Some(GmEventVerb::Fire);
    assert_eq!(
        validate_fleet_frame(&GmActionFrame::Refused(levered_session_pause), &roster),
        Err(GmActionRefusalReason::InvalidAction)
    );

    let mut forged_refusal = refused;
    forged_refusal.sequenced_by = HostSlot(2);
    assert_eq!(
        validate_fleet_frame(&GmActionFrame::Refused(forged_refusal), &roster),
        Err(GmActionRefusalReason::OriginMismatch)
    );
}

#[test]
fn full_lane_has_one_ordered_resume_escape_and_never_accepts_it_early() {
    let mut journal = GmActionJournal::default();
    for sequence in 1..=MAX_GM_ACTIONS_PER_RUN as u64 {
        journal
            .insert(grant(1, sequence, 10, &format!("fill-{sequence}"), true))
            .unwrap();
    }
    assert!(journal.log_through(10).paused());
    let escape = grant(
        1,
        MAX_STORED_GM_ACTIONS_PER_RUN as u64,
        10,
        "escape-resume",
        false,
    );
    journal.insert(escape).expect("bounded resume escape");
    assert_eq!(journal.len(), MAX_STORED_GM_ACTIONS_PER_RUN);
    assert!(!journal.log_through(10).paused());
    assert_eq!(
        journal.insert(grant(
            1,
            MAX_STORED_GM_ACTIONS_PER_RUN as u64 + 1,
            11,
            "past-bound",
            true,
        )),
        Err(GmActionRefusalReason::JournalFull)
    );

    let mut skewed = GmActionJournal::default();
    for sequence in 1..MAX_GM_ACTIONS_PER_RUN as u64 {
        skewed
            .insert(grant(1, sequence, 10, &format!("skew-{sequence}"), true))
            .unwrap();
    }
    let early_escape = grant(
        1,
        MAX_STORED_GM_ACTIONS_PER_RUN as u64,
        10,
        "early-escape",
        false,
    );
    assert_eq!(
        skewed.insert(early_escape),
        Err(GmActionRefusalReason::NonContiguousSequence),
        "an impossible transport reorder fails closed before it can consume capacity"
    );
}

#[test]
fn standalone_restored_pause_has_a_working_typed_resume() {
    let mut world = admitted_world();
    world.remove_resource::<crate::lockstep::FleetLockstep>();
    world.resource_mut::<SimulationPaused>().0 = true;
    let result = submit_local(
        &mut world,
        GmActionRequest {
            operator_id: "gm-1".into(),
            correlation: GmActionId::new("restored-resume").unwrap(),
            action: GmAction::SetSessionPaused { active: false },
        },
    )
    .expect("the preserved local GM binding remains authoritative");
    let GmActionSubmission::Granted(grant) = result else {
        panic!("standalone owner should sequence immediately");
    };
    assert_eq!(grant.apply_tick, 10);
    let log = world.resource::<GmActionJournal>().log_through(10);
    assert!(!log.paused());
    assert_eq!(log.entries()[0].outcome, GmActionOutcome::Applied);
}

#[test]
fn station_command_projection_preserves_exact_terminal_correlations_and_outcomes() {
    let station_applied = LoggedGmAction {
        operator_id: "gm-1".into(),
        correlation: GmActionId::new("iframe-applied").unwrap(),
        action_kind: GmActionKind::StationCommand,
        requested_active: true,
        outcome: GmActionOutcome::Applied,
        tick: 41,
        reason: None,
        order: Some(GmActionOrder::new(HostSlot(1), 1)),
        target: None,
        effect: None,
        verb: None,
        lever: None,
        effect_scope: None,
        objective_verb: None,
        objective_instance_scope: None,
        objective_recipients: None,
        comms_recipients: None,
        observer: None,
        npc_doctrine: None,
        affected: None,
        undo_of: None,
    };
    let pause = LoggedGmAction {
        operator_id: "gm-1".into(),
        correlation: GmActionId::new("pause-other-surface").unwrap(),
        action_kind: GmActionKind::SessionPause,
        requested_active: true,
        outcome: GmActionOutcome::Applied,
        tick: 40,
        reason: None,
        order: Some(GmActionOrder::new(HostSlot(1), 0)),
        target: None,
        effect: None,
        verb: None,
        lever: None,
        effect_scope: None,
        objective_verb: None,
        objective_instance_scope: None,
        objective_recipients: None,
        comms_recipients: None,
        observer: None,
        npc_doctrine: None,
        affected: None,
        undo_of: None,
    };
    let station_refused = LoggedGmAction::refused(
        "gm-1".into(),
        GmActionId::new("iframe-refused").unwrap(),
        GmActionKind::StationCommand,
        true,
        42,
        GmActionRefusalReason::StationNotPuppeted,
    );
    let station_pending = LoggedGmAction {
        operator_id: "gm-1".into(),
        correlation: GmActionId::new("iframe-still-pending").unwrap(),
        action_kind: GmActionKind::StationCommand,
        requested_active: true,
        outcome: GmActionOutcome::Pending,
        tick: 43,
        reason: None,
        order: Some(GmActionOrder::new(HostSlot(1), 2)),
        target: None,
        effect: None,
        verb: None,
        lever: None,
        effect_scope: None,
        objective_verb: None,
        objective_instance_scope: None,
        objective_recipients: None,
        comms_recipients: None,
        observer: None,
        npc_doctrine: None,
        affected: None,
        undo_of: None,
    };
    let log = GmActionLog {
        entries: vec![pause, station_applied.clone(), station_pending],
        paused: true,
    };
    let mut supplemental = LocalGmActionRefusals::default();
    supplemental.push(station_refused.clone());

    assert_eq!(
        projected_results(GmActionKind::StationCommand, &log, &supplemental),
        [station_applied, station_refused]
    );
}

#[test]
fn pending_is_valid_only_for_a_station_command_waiting_on_its_consumer() {
    let mut journal = GmActionJournal::default();
    let pause = grant(1, 1, 41, "pause-cannot-pend", true);
    journal.insert(pause.clone()).unwrap();
    assert_eq!(
        journal.record_applied_result(LoggedGmAction {
            operator_id: pause.operator_id,
            correlation: pause.correlation,
            action_kind: GmActionKind::SessionPause,
            requested_active: true,
            outcome: GmActionOutcome::Pending,
            tick: pause.apply_tick,
            reason: None,
            order: Some(pause.order),
            target: None,
            lever: None,
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
        }),
        Err("pending GM result is not a Station command"),
    );
    assert_eq!(journal.applied_grants(), 0);
}

#[test]
fn retry_reprojects_an_exact_terminal_fact_after_the_feed_bounds_it_out() {
    let mut world = admitted_world();
    world.insert_resource(crate::sim_tick::SimTick(500));
    let mut journal = GmActionJournal::default();
    for sequence in 1..=140 {
        journal
            .insert(grant(
                1,
                sequence,
                sequence,
                &format!("result-{sequence}"),
                sequence % 2 == 1,
            ))
            .unwrap();
    }
    let log = journal.log_through(500);
    let bounded = projection(
        false,
        &log,
        &LocalGmActionRefusals::default(),
        None,
        None,
        60.0,
        None,
    );
    assert!(bounded
        .results
        .iter()
        .all(|entry| entry.correlation.as_str() != "result-1"));
    world.insert_resource(journal);
    world.insert_resource(log);

    assert!(matches!(
        submit_local(
            &mut world,
            GmActionRequest {
                operator_id: "gm-1".into(),
                correlation: GmActionId::new("result-1").unwrap(),
                action: GmAction::SetSessionPaused { active: true },
            }
        ),
        Ok(GmActionSubmission::Replayed(_))
    ));
    let retried = projection(
        false,
        world.resource::<GmActionLog>(),
        world.resource::<LocalGmActionRefusals>(),
        None,
        None,
        60.0,
        None,
    );
    let exact = retried
        .results
        .iter()
        .find(|entry| entry.correlation.as_str() == "result-1")
        .expect("retried old fact is pinned into the bounded projection");
    assert_eq!(exact.tick, 1);
    assert_eq!(exact.outcome, GmActionOutcome::Applied);
}

#[test]
#[cfg(all(feature = "server", not(target_arch = "wasm32")))]
fn native_gm_admission_is_private_attributed_and_never_creates_a_fleet_peer() {
    let mut world = admitted_world();
    world.insert_resource(crate::lockstep::FleetRoster::default());
    world.remove_resource::<crate::lockstep::FleetLockstep>();
    world.insert_resource(NativeGmAuthority {
        connected: true,
        screen_pause: false,
    });
    world.insert_resource(
        crate::gm_roster::GmRoster::try_new(vec![crate::gm_roster::GmOperator::new(
            NATIVE_GM_OPERATOR_ID.into(),
            "GM".into(),
            true,
        )])
        .unwrap(),
    );
    let request = GmActionRequest {
        operator_id: NATIVE_GM_OPERATOR_ID.into(),
        correlation: GmActionId::new("native-pause").unwrap(),
        action: GmAction::SetSessionPaused { active: true },
    };
    assert_eq!(
        submit_local(&mut world, request.clone()),
        Err(GmActionRefusalReason::NotGameMaster)
    );
    let GmActionSubmission::Granted(grant) = submit_native(&mut world, request.clone()).unwrap()
    else {
        panic!("native request must be sequenced");
    };
    assert_eq!(grant.operator_id, NATIVE_GM_OPERATOR_ID);
    assert_eq!(grant.apply_tick, 10);
    assert!(matches!(
        submit_native(&mut world, request.clone()),
        Ok(GmActionSubmission::Replayed(_))
    ));
    assert!(world
        .resource_mut::<crate::lockstep::MeshOutbox>()
        .drain()
        .is_empty());
    assert!(world.resource::<crate::lockstep::FleetRoster>().is_solo());
    assert!(world
        .resource::<crate::lockstep::FleetRoster>()
        .gms()
        .is_empty());
    assert!(!world.contains_resource::<crate::lockstep::FleetLockstep>());
    let mut spoof = request.clone();
    spoof.operator_id = "crew".into();
    assert_eq!(
        submit_native(&mut world, spoof),
        Err(GmActionRefusalReason::OperatorMismatch)
    );
    world.resource_mut::<NativeGmAuthority>().connected = false;
    assert_eq!(
        submit_native(&mut world, request),
        Err(GmActionRefusalReason::NotGameMaster)
    );
}

#[test]
#[cfg(all(feature = "server", not(target_arch = "wasm32")))]
fn native_screen_hold_stops_an_empty_lane_and_requires_connected_explicit_resume() {
    use bevy::ecs::system::RunSystemOnce;
    let mut world = admitted_world();
    world.insert_resource(crate::lockstep::FleetRoster::default());
    world.remove_resource::<crate::lockstep::FleetLockstep>();
    world.insert_resource(NativeGmAuthority {
        connected: false,
        screen_pause: true,
    });
    world.insert_resource(Time::<Virtual>::default());
    world.insert_resource(Time::<Fixed>::default());
    world.resource_mut::<SimulationPaused>().0 = true;
    world.run_system_once(apply_due_actions).unwrap();
    assert!(world.resource::<Time<Virtual>>().is_paused());
    world.insert_resource(
        crate::gm_roster::GmRoster::try_new(vec![crate::gm_roster::GmOperator::new(
            NATIVE_GM_OPERATOR_ID.into(),
            "GM".into(),
            true,
        )])
        .unwrap(),
    );
    let resume = GmActionRequest {
        operator_id: NATIVE_GM_OPERATOR_ID.into(),
        correlation: GmActionId::new("explicit-native-resume").unwrap(),
        action: GmAction::SetSessionPaused { active: false },
    };
    assert_eq!(
        submit_native(&mut world, resume.clone()),
        Err(GmActionRefusalReason::NotGameMaster)
    );
    world.resource_mut::<NativeGmAuthority>().connected = true;
    world.run_system_once(apply_due_actions).unwrap();
    assert!(
        world.resource::<SimulationPaused>().0,
        "availability alone cannot resume"
    );
    let GmActionSubmission::Granted(grant) = submit_native(&mut world, resume).unwrap() else {
        panic!("connected private GM sequences explicit resume");
    };
    assert_eq!(
        grant.apply_tick, 10,
        "Resume uses the stopped logical boundary"
    );
    world.run_system_once(apply_due_actions).unwrap();
    assert!(!world.resource::<SimulationPaused>().0);
    assert!(!world.resource::<NativeGmAuthority>().screen_pause);
    assert!(!world.resource::<Time<Virtual>>().is_paused());
    let fact = &world.resource::<GmActionLog>().entries()[0];
    assert_eq!(fact.operator_id, NATIVE_GM_OPERATOR_ID);
    assert_eq!(fact.outcome, GmActionOutcome::Applied);
}

#[test]
fn a_player_ship_backfill_is_sequenced_and_applied_on_the_stopped_lobby_boundary() {
    use crate::world::config::{AvailableShipEntry, ShipSlotConfig, UnclaimedSlotPolicy};
    use bevy::ecs::system::RunSystemOnce;

    let mut world = admitted_world();
    world.insert_resource(State::new(crate::core::messages::GamePhase::Lobby));
    world.insert_resource(NextState::<crate::core::messages::GamePhase>::Unchanged);
    world.insert_resource(crate::ship_slots::FrozenShipSlots::default());
    let mut config = crate::world::config::WorldConfig::default();
    config.ship_slots.push(ShipSlotConfig {
        id: "wing".into(),
        label: Some("Wing ship".into()),
        ships: vec![AvailableShipEntry {
            template_path: "wing.toml".into(),
            label: None,
        }],
        default_ship: "wing.toml".into(),
        unclaimed: UnclaimedSlotPolicy::Absent,
    });
    world.insert_resource(config);

    let GmActionSubmission::Granted(grant) = submit_local(
        &mut world,
        GmActionRequest {
            operator_id: "gm-1".into(),
            correlation: GmActionId::new("backfill-wing").unwrap(),
            action: GmAction::BackfillShipSlot {
                slot: "wing".into(),
            },
        },
    )
    .unwrap() else {
        panic!("the fleet owner sequences its lobby action");
    };
    assert_eq!(
        grant.apply_tick, 10,
        "the stopped lobby clock cannot advance"
    );

    world.run_system_once(apply_due_actions).unwrap();
    let frozen = world.resource::<crate::ship_slots::FrozenShipSlots>();
    assert_eq!(frozen.0.len(), 1);
    assert_eq!(frozen.0[0].slot_id, "wing");
    assert_eq!(
        frozen.0[0].source,
        crate::ship_slots::LaunchSource::Backfill
    );
    let fact = &world.resource::<GmActionLog>().entries()[0];
    assert_eq!(fact.action_kind, GmActionKind::ShipSlotBackfill);
    assert_eq!(fact.target.as_deref(), Some("wing"));
    assert_eq!(fact.outcome, GmActionOutcome::Applied);
}

fn admitted_world() -> World {
    let mut world = World::new();
    let slot = HostSlot(1);
    world.insert_resource(
        crate::lockstep::FleetRoster::with_participants_and_gms(
            Vec::new(),
            vec![slot],
            vec![crate::lockstep::FleetGm {
                host: slot,
                operator_id: "gm-1".into(),
            }],
            slot,
            slot,
        )
        .unwrap(),
    );
    world.insert_resource(crate::lockstep::FleetLockstep(
        crate::lockstep::LockstepSession::new(slot, [slot], 6),
    ));
    world.insert_resource(
        crate::gm_roster::GmRoster::try_new(vec![crate::gm_roster::GmOperator::new(
            "gm-1".into(),
            "Morgan".into(),
            true,
        )])
        .unwrap(),
    );
    world.insert_resource(crate::sim_tick::SimTick(10));
    world.insert_resource(SimulationPaused(false));
    world.insert_resource(GmActionJournal::default());
    world.insert_resource(GmActionLog::default());
    world.insert_resource(LocalGmActionRefusals::default());
    world.insert_resource(LastGmSessionProjection::default());
    world.insert_resource(crate::lockstep::MeshOutbox::default());
    world
}

#[test]
fn typed_contact_ingress_and_replay_do_not_require_station_capabilities() {
    use bevy::ecs::system::RunSystemOnce;
    let mut live = admitted_world();
    let request = crate::core::codec::decode_gm_action_request(
            r#"{"operator_id":"gm-1","correlation":"contact-without-console","action":"set_contact_override","ship":"observer","target":"target","mode":"conceal"}"#,
        )
        .unwrap();
    let GmActionSubmission::Granted(grant) = submit_local(&mut live, request).unwrap() else {
        panic!("an admitted contact action must reach its own apply validator");
    };
    let replayed: GmActionGrant =
        serde_json::from_str(&serde_json::to_string(&grant).unwrap()).unwrap();
    let mut replay = admitted_world();
    replay
        .resource_mut::<GmActionJournal>()
        .insert(replayed)
        .unwrap();
    for world in [&mut live, &mut replay] {
        // A valid contact observer need not offer any authentic Station.
        world.spawn((
            crate::server_app::Ship,
            crate::lockstep::FleetSlotOf(HostSlot(2)),
            crate::entities::spawner::EntityUuid("observer".into()),
        ));
        world.spawn(crate::entities::spawner::EntityUuid("target".into()));
        world.init_resource::<crate::world::server::WorldContentRuntime>();
        world.resource_mut::<crate::sim_tick::SimTick>().0 = grant.apply_tick;
        world.run_system_once(apply_due_actions).unwrap();
        assert_eq!(
            world.resource::<GmActionLog>().entries()[0].outcome,
            GmActionOutcome::Applied
        );
        assert_eq!(
            crate::gm_contact::mode(
                &world
                    .resource::<crate::world::server::WorldContentRuntime>()
                    .contact_overrides,
                "observer",
                "target"
            ),
            crate::gm_contact::ContactMode::Conceal,
        );
    }
    assert_eq!(
        live.resource::<GmActionLog>().entries(),
        replay.resource::<GmActionLog>().entries()
    );
}

#[test]
fn local_admission_binds_identity_and_reuses_the_cached_grant() {
    let mut world = admitted_world();
    let request = GmActionRequest {
        operator_id: "gm-1".into(),
        correlation: GmActionId::new("pause-once").unwrap(),
        action: GmAction::SetSessionPaused { active: true },
    };
    let first = submit_local(&mut world, request.clone()).unwrap();
    let retried = submit_local(&mut world, request).unwrap();
    let GmActionSubmission::Granted(first) = first else {
        panic!("owner must grant its local proposal");
    };
    assert_eq!(retried, GmActionSubmission::Replayed(first));
    assert_eq!(world.resource::<GmActionJournal>().len(), 1);
    assert_eq!(
        world
            .resource::<crate::lockstep::MeshOutbox>()
            .pending_frames()
            .len(),
        1,
        "a local retry reuses the cached fact instead of duplicating the wire action"
    );

    let conflicting = GmActionRequest {
        operator_id: "gm-1".into(),
        correlation: GmActionId::new("pause-once").unwrap(),
        action: GmAction::SetSessionPaused { active: false },
    };
    assert_eq!(
        submit_local(&mut world, conflicting),
        Err(GmActionRefusalReason::ConflictingGrant)
    );
}

#[test]
fn local_admission_refuses_spoofed_or_disconnected_operators() {
    let mut world = admitted_world();
    let spoofed = GmActionRequest {
        operator_id: "gm-2".into(),
        correlation: GmActionId::new("spoofed").unwrap(),
        action: GmAction::SetSessionPaused { active: true },
    };
    assert_eq!(
        submit_local(&mut world, spoofed),
        Err(GmActionRefusalReason::OperatorMismatch)
    );

    world.insert_resource(
        crate::gm_roster::GmRoster::try_new(vec![crate::gm_roster::GmOperator::new(
            "gm-1".into(),
            "Morgan".into(),
            false,
        )])
        .unwrap(),
    );
    let disconnected = GmActionRequest {
        operator_id: "gm-1".into(),
        correlation: GmActionId::new("disconnected").unwrap(),
        action: GmAction::SetSessionPaused { active: true },
    };
    assert_eq!(
        submit_local(&mut world, disconnected),
        Err(GmActionRefusalReason::NotGameMaster)
    );
}

#[test]
fn local_session_pause_is_admitted_only_during_an_active_run() {
    let mut world = admitted_world();
    world.insert_resource(State::new(crate::core::messages::GamePhase::Lobby));
    let request = GmActionRequest {
        operator_id: "gm-1".into(),
        correlation: GmActionId::new("lobby-pause").unwrap(),
        action: GmAction::SetSessionPaused { active: true },
    };
    assert_eq!(
        submit_local(&mut world, request.clone()),
        Err(GmActionRefusalReason::WrongPhase)
    );
    assert!(world.resource::<GmActionJournal>().is_empty());
    assert!(world
        .resource::<crate::lockstep::MeshOutbox>()
        .pending_frames()
        .is_empty());

    world.insert_resource(State::new(crate::core::messages::GamePhase::InProgress));
    world.insert_resource(NextState::Pending(crate::core::messages::GamePhase::Lobby));
    assert_eq!(
        submit_local(&mut world, request.clone()),
        Err(GmActionRefusalReason::WrongPhase),
        "a same-frame accepted ReturnToLobby closes ingress before State changes"
    );
    world.insert_resource(NextState::<crate::core::messages::GamePhase>::Unchanged);
    assert!(matches!(
        submit_local(&mut world, request),
        Ok(GmActionSubmission::Granted(_))
    ));
}

// -- Directed world effects (issue #1310) --------------------------------

fn effect_grant(
    sequence: u64,
    apply_tick: u64,
    correlation: &str,
    target: &str,
    effect: crate::gm_effect::GmDirectEffectKind,
    amount_milli_hp: u32,
) -> GmActionGrant {
    GmActionGrant {
        from: HostSlot(1),
        sequenced_by: HostSlot(1),
        operator_id: "gm-1".into(),
        correlation: GmActionId::new(correlation).unwrap(),
        recovery_generation: 0,
        apply_tick,
        order: GmActionOrder::new(HostSlot(1), sequence),
        action: GmAction::ApplyDirectEffect {
            target: target.into(),
            scope: crate::gm_effect::GmDirectEffectScope::Entity,
            effect,
            amount_milli_hp,
        },
    }
}

/// A world of hulls, plus any number of hull-less entities a GM might also
/// select off the same map.
///
/// `hull_less` spawns BOTH shapes a hull-less entity takes in production,
/// because they reach the reducer down different paths and must come out
/// with the same answer: `"<uuid>"` carries an `EntitySystemHull` that
/// declares no systems (an authored empty `[hull]`), and
/// `"<uuid>-componentless"` carries no `EntitySystemHull` component at all
/// — the shape `HullSpawn` produces for every template with no `[hull]`
/// section, which is what a nav beacon and a planet actually are.
fn effect_app(
    tick: u64,
    hulls: &[(&str, f32, f32)],
    hull_less: &[&str],
    grants: impl IntoIterator<Item = GmActionGrant>,
) -> App {
    let mut journal = GmActionJournal::default();
    for grant in grants {
        journal.insert(grant).unwrap();
    }
    let mut app = App::new();
    app.insert_resource(crate::sim_tick::SimTick(tick))
        .insert_resource(SimulationPaused(false))
        .insert_resource(journal)
        .init_resource::<GmActionLog>()
        .init_resource::<crate::gm_effect::PendingGmDirectEffects>()
        .add_systems(Update, apply_due_actions);
    for (uuid, max, current) in hulls {
        let system = crate::core::messages::SystemId("captain".into());
        let mut hull = crate::ship::damage::SystemHull::from_config(&[(system.clone(), *max)]);
        hull.set_hp(&system, *current);
        app.world_mut().spawn((
            crate::entities::spawner::EntityUuid((*uuid).into()),
            crate::entities::spawner::EntitySystemHull(hull),
        ));
    }
    for uuid in hull_less {
        app.world_mut().spawn((
            crate::entities::spawner::EntityUuid((*uuid).into()),
            crate::entities::spawner::EntitySystemHull(crate::ship::damage::SystemHull::default()),
        ));
        app.world_mut()
            .spawn(crate::entities::spawner::EntityUuid(format!(
                "{uuid}-componentless"
            )));
    }
    app
}

/// [`effect_grant`] aimed at less than the whole hull (issue #1311).
fn scoped_effect_grant(
    sequence: u64,
    apply_tick: u64,
    correlation: &str,
    target: &str,
    scope: crate::gm_effect::GmDirectEffectScope,
    effect: crate::gm_effect::GmDirectEffectKind,
    amount_milli_hp: u32,
) -> GmActionGrant {
    let mut grant = effect_grant(
        sequence,
        apply_tick,
        correlation,
        target,
        effect,
        amount_milli_hp,
    );
    if let GmAction::ApplyDirectEffect { scope: slot, .. } = &mut grant.action {
        *slot = scope;
    }
    grant
}

fn effect_station(id: &str) -> crate::gm_effect::GmDirectEffectScope {
    crate::gm_effect::GmDirectEffectScope::Station(crate::core::messages::StationId(id.into()))
}

fn effect_system(id: &str) -> crate::gm_effect::GmDirectEffectScope {
    crate::gm_effect::GmDirectEffectScope::System(crate::core::messages::SystemId(id.into()))
}

/// A world with ONE stationed hull, so a narrowed scope has real authored
/// ownership to resolve against (issue #1311).
///
/// The ship config is parsed from TOML for the reason
/// `gm_effect::tests::ship_config` is: the ownership under test has to be
/// the same `[[system]] station = "..."` field a shipped hull authors, read
/// the same way.
///
/// A row with max `0.0` is authored in the config but NOT tracked by the
/// hull — the shape every Alliance radar has, a `[[system]]` with no
/// `[[hull.system_hull]]` entry. It exists to be owned by a Station and can
/// never be damaged or repaired.
fn stationed_effect_app(
    tick: u64,
    uuid: &str,
    systems: &[(&str, f32, f32, Option<&str>)],
    grants: impl IntoIterator<Item = GmActionGrant>,
) -> App {
    let mut app = effect_app(tick, &[], &[], grants);
    let mut toml = String::new();
    let mut stations: Vec<&str> = systems
        .iter()
        .filter_map(|(_, _, _, station)| *station)
        .collect();
    stations.sort_unstable();
    stations.dedup();
    for station in stations {
        toml.push_str(&format!(
            r#"
[[station]]
id = "{station}"
name = "station.{station}.display_name"
description = "station.{station}.description"
rank = "Lieutenant"
"#
        ));
    }
    let mut hull = crate::ship::damage::SystemHull::from_config(
        &systems
            .iter()
            .filter(|(_, max, _, _)| *max > 0.0)
            .map(|(id, max, _, _)| (crate::core::messages::SystemId((*id).into()), *max))
            .collect::<Vec<_>>(),
    );
    for (id, max, current, station) in systems {
        if *max > 0.0 {
            hull.set_hp(&crate::core::messages::SystemId((*id).into()), *current);
        }
        toml.push_str(&format!(
            r#"
[[system]]
id = "{id}"
kind = "{id}"
"#
        ));
        if let Some(station) = station {
            toml.push_str(&format!("station = \"{station}\"\n"));
        }
    }
    app.world_mut().spawn((
        crate::entities::spawner::EntityUuid(uuid.into()),
        crate::entities::spawner::EntitySystemHull(hull),
        crate::ship::components::ShipConfigComponent(
            toml::from_str(&toml).expect("a well-formed authoring fixture"),
        ),
    ));
    app
}

fn armed_effects(app: &App) -> Vec<crate::gm_effect::PendingGmDirectEffect> {
    app.world()
        .resource::<crate::gm_effect::PendingGmDirectEffects>()
        .entries()
        .to_vec()
}

fn effect_results(app: &App) -> Vec<Option<crate::gm_effect::GmDirectEffectResult>> {
    app.world()
        .resource::<GmActionJournal>()
        .applied_results()
        .iter()
        .map(|result| result.effect)
        .collect()
}

/// The whole resolve-then-arm contract: a valid amount arms exactly what
/// the durable result claims, stated in the same unit.
#[test]
fn a_direct_hit_arms_the_amount_its_durable_result_reports() {
    let mut app = effect_app(
        5,
        &[("npc-1", 100.0, 100.0)],
        &[],
        [effect_grant(
            1,
            5,
            "hit-a",
            "npc-1",
            crate::gm_effect::GmDirectEffectKind::Damage,
            25_000,
        )],
    );
    app.update();

    assert_eq!(outcomes(&app), vec![(GmActionOutcome::Applied, None)]);
    let armed = armed_effects(&app);
    assert_eq!(armed.len(), 1);
    assert_eq!(armed[0].target, "npc-1");
    assert_eq!(armed[0].amount_milli_hp, 25_000);
    assert_eq!(armed[0].tick, 5);
    assert_eq!(
        effect_results(&app)[0],
        Some(crate::gm_effect::GmDirectEffectResult {
            kind: crate::gm_effect::GmDirectEffectKind::Damage,
            applied_milli_hp: 25_000,
            discarded_milli_hp: 0,
            destroyed: false,
        })
    );
}

/// Healing beyond the maxima is clamped at the apply tick and the discarded
/// remainder is REPORTED rather than silently absorbed -- and the arm
/// carries only what will actually land.
#[test]
fn a_heal_beyond_the_maxima_clamps_and_reports_the_discarded_overflow() {
    let mut app = effect_app(
        3,
        &[("npc-1", 100.0, 60.0)],
        &[],
        [effect_grant(
            1,
            3,
            "heal-a",
            "npc-1",
            crate::gm_effect::GmDirectEffectKind::Heal,
            250_000,
        )],
    );
    app.update();

    assert_eq!(outcomes(&app), vec![(GmActionOutcome::Applied, None)]);
    assert_eq!(armed_effects(&app)[0].amount_milli_hp, 40_000);
    assert_eq!(
        effect_results(&app)[0],
        Some(crate::gm_effect::GmDirectEffectResult {
            kind: crate::gm_effect::GmDirectEffectKind::Heal,
            applied_milli_hp: 40_000,
            discarded_milli_hp: 210_000,
            destroyed: false,
        })
    );
}

/// Damage that empties the hull is reported as lethal BEFORE the damage
/// phase runs -- the metadata a confirmation surface previews from.
#[test]
fn damage_that_empties_the_hull_is_reported_lethal_at_the_apply_boundary() {
    let mut app = effect_app(
        1,
        &[("npc-1", 100.0, 30.0)],
        &[],
        [effect_grant(
            1,
            1,
            "kill",
            "npc-1",
            crate::gm_effect::GmDirectEffectKind::Damage,
            90_000,
        )],
    );
    app.update();

    let result = effect_results(&app)[0].expect("a resolved effect");
    assert!(result.destroyed);
    assert_eq!(result.applied_milli_hp, 30_000);
    assert_eq!(result.discarded_milli_hp, 60_000);
}

/// Nothing to change is a No-op on every peer, not a refusal and not a
/// silent success: a full hull cannot be healed and a wreck cannot be
/// damaged further.
#[test]
fn an_effect_with_nothing_to_change_is_a_deterministic_no_op() {
    let mut app = effect_app(
        2,
        &[("full", 100.0, 100.0), ("wreck", 100.0, 0.0)],
        &[],
        [
            effect_grant(
                1,
                2,
                "heal-full",
                "full",
                crate::gm_effect::GmDirectEffectKind::Heal,
                5_000,
            ),
            effect_grant(
                2,
                2,
                "hit-wreck",
                "wreck",
                crate::gm_effect::GmDirectEffectKind::Damage,
                5_000,
            ),
        ],
    );
    app.update();

    assert_eq!(
        outcomes(&app),
        vec![(GmActionOutcome::NoOp, None), (GmActionOutcome::NoOp, None)]
    );
    assert!(
        armed_effects(&app).is_empty(),
        "a No-op arms no work for the damage phase"
    );
    for result in effect_results(&app) {
        let result = result.expect("a No-op still reports what it discarded");
        assert_eq!(result.applied_milli_hp, 0);
        assert_eq!(result.discarded_milli_hp, 5_000);
    }
}

/// Every stale-or-undamageable target shape is a canonical refusal decided
/// against the LIVE world at the apply tick, never at request time — and
/// the two refusals say DIFFERENT things. Only an identity nothing carries
/// is `UnknownEntity`; a beacon a GM can see and select is refused as
/// undamageable whether its template authored an empty `[hull]` or, as
/// every shipped beacon and planet does, no `[hull]` section at all.
#[test]
fn a_vanished_or_hull_less_target_is_refused_at_the_apply_boundary() {
    let mut app = effect_app(
        4,
        &[("npc-1", 100.0, 100.0)],
        &["beacon"],
        [
            effect_grant(
                1,
                4,
                "gone",
                "npc-missing",
                crate::gm_effect::GmDirectEffectKind::Damage,
                1_000,
            ),
            effect_grant(
                2,
                4,
                "beacon",
                "beacon",
                crate::gm_effect::GmDirectEffectKind::Damage,
                1_000,
            ),
            effect_grant(
                3,
                4,
                "beacon-no-hull-section",
                "beacon-componentless",
                crate::gm_effect::GmDirectEffectKind::Damage,
                1_000,
            ),
        ],
    );
    app.update();

    assert_eq!(
        outcomes(&app),
        vec![
            (
                GmActionOutcome::Refused,
                Some(GmActionRefusalReason::UnknownEntity)
            ),
            (
                GmActionOutcome::Refused,
                Some(GmActionRefusalReason::TargetNotDamageable)
            ),
            (
                GmActionOutcome::Refused,
                Some(GmActionRefusalReason::TargetNotDamageable)
            ),
        ]
    );
    assert!(armed_effects(&app).is_empty());
}

/// Two GMs pressing the same target on the same boundary each get an
/// HONEST answer: the second is measured against what the first left, so
/// the durable results describe two different hits rather than the same one
/// twice, and only one of them can claim the kill.
#[test]
fn simultaneous_effects_on_one_target_resolve_against_each_other() {
    let mut app = effect_app(
        7,
        &[("npc-1", 100.0, 100.0)],
        &[],
        [
            effect_grant(
                1,
                7,
                "hit-a",
                "npc-1",
                crate::gm_effect::GmDirectEffectKind::Damage,
                60_000,
            ),
            effect_grant(
                2,
                7,
                "hit-b",
                "npc-1",
                crate::gm_effect::GmDirectEffectKind::Damage,
                60_000,
            ),
        ],
    );
    app.update();

    assert_eq!(
        armed_effects(&app)
            .iter()
            .map(|effect| (effect.order.sequence, effect.amount_milli_hp))
            .collect::<Vec<_>>(),
        vec![(1, 60_000), (2, 40_000)],
        "the damage phase drains these in order and the second only has 40 left"
    );
    let results = effect_results(&app);
    assert!(!results[0].expect("resolved").destroyed);
    let second = results[1].expect("resolved");
    assert!(second.destroyed, "the second press is the one that kills");
    assert_eq!(second.discarded_milli_hp, 20_000);
}

/// A narrowed scope is resolved and CLAMPED against only what it names, so
/// the durable result an operator reads describes the Station they aimed at
/// rather than the hull it sits in (issue #1311).
///
/// 25 points at a Station holding 20 is lethal to that Station and
/// discards 5 — the same arithmetic the whole-hull path performs, asked of
/// a smaller total. `destroyed` stays false because the entity survives:
/// emptying a Station is not sinking a ship.
#[test]
fn a_station_scope_resolves_and_clamps_against_only_its_own_systems() {
    let mut app = stationed_effect_app(
        5,
        "npc-1",
        &[
            ("impulse-drive", 40.0, 15.0, Some("helm")),
            ("manoeuvre-thrusters", 20.0, 5.0, Some("helm")),
            ("phaser-bank", 40.0, 40.0, Some("tactical")),
        ],
        [scoped_effect_grant(
            1,
            5,
            "hit-helm",
            "npc-1",
            effect_station("helm"),
            crate::gm_effect::GmDirectEffectKind::Damage,
            25_000,
        )],
    );
    app.update();

    assert_eq!(outcomes(&app), vec![(GmActionOutcome::Applied, None)]);
    assert_eq!(
        effect_results(&app)[0].expect("resolved"),
        crate::gm_effect::GmDirectEffectResult {
            kind: crate::gm_effect::GmDirectEffectKind::Damage,
            applied_milli_hp: 20_000,
            discarded_milli_hp: 5_000,
            destroyed: false,
        },
        "the clamp is the STATION's 20 points, and the entity survives it"
    );
    assert_eq!(
        armed_effects(&app)
            .iter()
            .map(|effect| (effect.scope.clone(), effect.amount_milli_hp))
            .collect::<Vec<_>>(),
        vec![(effect_station("helm"), 20_000)],
        "the arm carries the scope so the damage phase restricts the same way"
    );
    assert_eq!(
        app.world()
            .resource::<GmActionJournal>()
            .applied_results()
            .iter()
            .map(|result| result.effect_scope.clone())
            .collect::<Vec<_>>(),
        vec![Some(effect_station("helm"))],
        "the durable fact says WHICH Station, so the feed cannot claim the hull"
    );
}

/// Every way a narrowed scope can name nothing, told apart at the apply
/// tick and against the LIVE world — because a Station a layer unloaded
/// between the press and the boundary is exactly as absent as one that was
/// never authored.
///
/// The three answers are deliberately different sentences. `UnknownStation`
/// and `UnknownSystem` say nothing answers to that name;
/// `TargetNotDamageable` says something does, but this hull tracks none of
/// it — the scoped spelling of the refusal a beacon already gets.
#[test]
fn a_scope_that_names_nothing_damageable_is_refused_at_the_apply_boundary() {
    let mut app = stationed_effect_app(
        4,
        "npc-1",
        &[
            ("impulse-drive", 40.0, 40.0, Some("helm")),
            // Authored under `science` but NOT tracked by the hull, the
            // shape every Alliance radar has.
            ("nav-radar", 0.0, 0.0, Some("science")),
        ],
        [
            scoped_effect_grant(
                1,
                4,
                "no-station",
                "npc-1",
                effect_station("engineering"),
                crate::gm_effect::GmDirectEffectKind::Damage,
                1_000,
            ),
            scoped_effect_grant(
                2,
                4,
                "no-system",
                "npc-1",
                effect_system("torpedo-tube"),
                crate::gm_effect::GmDirectEffectKind::Damage,
                1_000,
            ),
            scoped_effect_grant(
                3,
                4,
                "no-damageable-system",
                "npc-1",
                effect_station("science"),
                crate::gm_effect::GmDirectEffectKind::Heal,
                1_000,
            ),
            // A Station scope aimed at a hull that authors no stations at
            // all is refused rather than silently widened to the whole
            // hull: `effect_app`'s bare `npc-2` carries no ship config.
            scoped_effect_grant(
                4,
                4,
                "no-config",
                "npc-2",
                effect_station("helm"),
                crate::gm_effect::GmDirectEffectKind::Damage,
                1_000,
            ),
        ],
    );
    // The config-less hull the fourth grant aims at.
    app.world_mut().spawn((
        crate::entities::spawner::EntityUuid("npc-2".into()),
        crate::entities::spawner::EntitySystemHull(crate::ship::damage::SystemHull::from_config(
            &[(crate::core::messages::SystemId("captain".into()), 100.0)],
        )),
    ));
    app.update();

    assert_eq!(
        outcomes(&app),
        vec![
            (
                GmActionOutcome::Refused,
                Some(GmActionRefusalReason::UnknownStation)
            ),
            (
                GmActionOutcome::Refused,
                Some(GmActionRefusalReason::UnknownSystem)
            ),
            (
                GmActionOutcome::Refused,
                Some(GmActionRefusalReason::TargetNotDamageable)
            ),
            (
                GmActionOutcome::Refused,
                Some(GmActionRefusalReason::UnknownStation)
            ),
        ]
    );
    assert_eq!(
        app.world()
            .resource::<GmActionLog>()
            .entries()
            .iter()
            .map(|fact| fact.effect_scope.clone())
            .collect::<Vec<_>>(),
        vec![
            Some(effect_station("engineering")),
            Some(effect_system("torpedo-tube")),
            Some(effect_station("science")),
            Some(effect_station("helm"))
        ],
        "refusals retain the requested narrowing even when it cannot resolve",
    );
    assert!(
        armed_effects(&app).is_empty(),
        "a refusal arms no work for the damage phase"
    );
}

#[test]
fn refused_scoped_effects_keep_the_requested_scope_on_every_result_lane() {
    for scope in [effect_station("helm"), effect_system("impulse-drive")] {
        let grant = scoped_effect_grant(
            1,
            4,
            "scope-refusal",
            "gone",
            scope.clone(),
            crate::gm_effect::GmDirectEffectKind::Damage,
            1_000,
        );
        let mut reconstructed = GmActionJournal::default();
        reconstructed.insert(grant.clone()).unwrap();
        reconstructed.restore_applied_frontier(1).unwrap();
        assert_eq!(
            reconstructed.applied_log().entries()[0].effect_scope,
            Some(scope.clone()),
            "reconstructing a grant frontier preserves its requested scope"
        );
        let request = GmActionRequest {
            operator_id: grant.operator_id.clone(),
            correlation: grant.correlation.clone(),
            action: grant.action.clone(),
        };
        let ingress =
            LoggedGmAction::refused_request(&request, 4, GmActionRefusalReason::WrongPhase);
        assert_eq!(ingress.effect_scope, Some(scope.clone()));
        let refusal = refusal_for(
            HostSlot(1),
            &GmActionProposal {
                from: HostSlot(1),
                operator_id: request.operator_id,
                correlation: request.correlation,
                action: request.action,
            },
            4,
            GmActionRefusalReason::WrongPhase,
        );
        assert_eq!(refusal.logged().effect_scope, Some(scope.clone()));
        let frame = crate::lockstep::MeshFrame::GmAction(GmActionFrame::Refused(refusal));
        let encoded = crate::core::codec::encode_mesh_frame(&frame).unwrap();
        assert_eq!(
            crate::core::codec::decode_mesh_frame(&encoded),
            Some(frame.clone())
        );
        let crate::lockstep::MeshFrame::GmAction(GmActionFrame::Refused(mut malformed)) = frame
        else {
            unreachable!()
        };
        malformed.action_kind = GmActionKind::SessionPause;
        let malformed = crate::lockstep::MeshFrame::GmAction(GmActionFrame::Refused(malformed));
        let encoded = crate::core::codec::encode_mesh_frame(&malformed).unwrap();
        assert!(
            crate::core::codec::decode_mesh_frame(&encoded).is_none(),
            "a non-effect refusal cannot claim a narrowed effect scope"
        );
        for target in ["gone", "beacon", "beacon-componentless"] {
            let mut grant = grant.clone();
            if let GmAction::ApplyDirectEffect { target: slot, .. } = &mut grant.action {
                *slot = target.into();
            }
            let mut app = effect_app(4, &[], &["beacon"], [grant]);
            app.update();
            let fact = &app.world().resource::<GmActionLog>().entries()[0];
            assert_eq!(fact.outcome, GmActionOutcome::Refused);
            assert_eq!(fact.effect_scope, Some(scope.clone()), "{target}");
        }
    }
}

/// A System scope resolves against exactly one System's totals, and a
/// System already at zero asked to take more damage is the same No-op an
/// empty hull is — not a refusal, because the scope named something real.
#[test]
fn a_system_scope_measures_one_system_and_an_empty_one_is_a_no_op() {
    let mut app = stationed_effect_app(
        6,
        "npc-1",
        &[
            ("impulse-drive", 40.0, 0.0, Some("helm")),
            ("phaser-bank", 40.0, 40.0, Some("tactical")),
        ],
        [
            scoped_effect_grant(
                1,
                6,
                "hit-dead-system",
                "npc-1",
                effect_system("impulse-drive"),
                crate::gm_effect::GmDirectEffectKind::Damage,
                5_000,
            ),
            scoped_effect_grant(
                2,
                6,
                "heal-full-system",
                "npc-1",
                effect_system("phaser-bank"),
                crate::gm_effect::GmDirectEffectKind::Heal,
                5_000,
            ),
        ],
    );
    app.update();

    assert_eq!(
        outcomes(&app),
        vec![(GmActionOutcome::NoOp, None), (GmActionOutcome::NoOp, None)],
        "an empty System asked for more damage, and a full one asked to heal"
    );
    for result in effect_results(&app) {
        let result = result.expect("a No-op still reports what it discarded");
        assert_eq!(result.applied_milli_hp, 0);
        assert_eq!(result.discarded_milli_hp, 5_000);
    }
    assert!(armed_effects(&app).is_empty());
}

/// Two GMs pressing OVERLAPPING scopes on one boundary each get an honest
/// answer, and a third pressing a disjoint scope is unaffected by either
/// (issue #1311).
///
/// The second press is measured against what the first left inside the
/// scope they share; the fourth measures the whole hull, which overlaps
/// everything and so carries both. Nothing here mutates the world — the
/// reducer resolves against the arm queue, which is what makes the answers
/// identical on every peer.
#[test]
fn simultaneous_scoped_effects_resolve_against_each_other_but_not_across_scopes() {
    let mut app = stationed_effect_app(
        7,
        "npc-1",
        &[
            ("impulse-drive", 40.0, 40.0, Some("helm")),
            ("manoeuvre-thrusters", 20.0, 20.0, Some("helm")),
            ("phaser-bank", 40.0, 40.0, Some("tactical")),
        ],
        [
            scoped_effect_grant(
                1,
                7,
                "helm-a",
                "npc-1",
                effect_station("helm"),
                crate::gm_effect::GmDirectEffectKind::Damage,
                50_000,
            ),
            scoped_effect_grant(
                2,
                7,
                "helm-b",
                "npc-1",
                effect_station("helm"),
                crate::gm_effect::GmDirectEffectKind::Damage,
                50_000,
            ),
            scoped_effect_grant(
                3,
                7,
                "tactical",
                "npc-1",
                effect_station("tactical"),
                crate::gm_effect::GmDirectEffectKind::Damage,
                40_000,
            ),
        ],
    );
    app.update();

    assert_eq!(
        armed_effects(&app)
            .iter()
            .map(|effect| (effect.order.sequence, effect.amount_milli_hp))
            .collect::<Vec<_>>(),
        vec![(1, 50_000), (2, 10_000), (3, 40_000)],
        "the second helm press only has 10 of that Station's 60 points left, \
             and the tactical press has its own 40 untouched by either"
    );
    let results = effect_results(&app);
    assert_eq!(results[1].expect("resolved").discarded_milli_hp, 40_000);
    assert!(
        !results[0].expect("resolved").destroyed && !results[1].expect("resolved").destroyed,
        "emptying a Station is not sinking a ship while another Station is alive"
    );
    assert!(
        results[2].expect("resolved").destroyed,
        "destruction is a whole-hull fact, so the press that empties the LAST \
             living Systems is the kill even though its clamp was a Station's"
    );

    // Mixed KINDS across scopes (#1311 round-1 review). A whole-hull heal
    // lands an unknown share of itself inside any one Station — the damage
    // phase's keyed generator decides — so a Station DAMAGE press behind it
    // may not count those points as damageable. Measured against the
    // un-healed Station total, its clamp is what the hull will honour
    // whichever Systems the heal chose.
    let mut mixed = stationed_effect_app(
        7,
        "npc-1",
        &[
            ("impulse-drive", 40.0, 10.0, Some("helm")),
            ("phaser-bank", 40.0, 10.0, Some("tactical")),
        ],
        [
            scoped_effect_grant(
                1,
                7,
                "heal-hull",
                "npc-1",
                crate::gm_effect::GmDirectEffectScope::Entity,
                crate::gm_effect::GmDirectEffectKind::Heal,
                40_000,
            ),
            scoped_effect_grant(
                2,
                7,
                "damage-helm",
                "npc-1",
                effect_station("helm"),
                crate::gm_effect::GmDirectEffectKind::Damage,
                50_000,
            ),
        ],
    );
    mixed.update();

    assert_eq!(
        armed_effects(&mixed)
            .iter()
            .map(|effect| (effect.order.sequence, effect.amount_milli_hp))
            .collect::<Vec<_>>(),
        vec![(1, 40_000), (2, 10_000)],
        "the Station press is bounded by the 10 points that Station holds \
             UN-healed: the 40-point hull heal may land entirely on tactical"
    );
    let mixed_results = effect_results(&mixed);
    assert_eq!(
        mixed_results[1].expect("resolved").discarded_milli_hp,
        40_000,
        "the rest is honestly reported as discarded rather than promised"
    );
    assert!(
        !mixed_results[1].expect("resolved").destroyed,
        "10 points off a hull the queue projects at 60 is no kill"
    );
}

/// The typed vocabulary: what a direct effect is, what it names, and what
/// it refuses without ever reaching the world.
#[test]
fn a_direct_effect_names_its_entity_and_refuses_an_empty_request() {
    let action = GmAction::ApplyDirectEffect {
        target: "npc-1".into(),
        scope: crate::gm_effect::GmDirectEffectScope::Entity,
        effect: crate::gm_effect::GmDirectEffectKind::Damage,
        amount_milli_hp: 1,
    };
    assert_eq!(action.kind(), GmActionKind::DirectEffect);
    assert!(GmActionKind::DirectEffect.carries_target());
    assert_eq!(action.target_id(), Some("npc-1"));
    assert_eq!(action.ship_key(), None);
    assert!(action.requested_active());
    assert_eq!(action.requested_pause(), None);
    assert_eq!(action.validate(), Ok(()));

    let empty = GmAction::ApplyDirectEffect {
        target: "npc-1".into(),
        scope: crate::gm_effect::GmDirectEffectScope::Entity,
        effect: crate::gm_effect::GmDirectEffectKind::Damage,
        amount_milli_hp: 0,
    };
    assert_eq!(empty.validate(), Err(GmActionRefusalReason::InvalidAction));

    let nameless = GmAction::ApplyDirectEffect {
        target: String::new(),
        scope: crate::gm_effect::GmDirectEffectScope::Entity,
        effect: crate::gm_effect::GmDirectEffectKind::Heal,
        amount_milli_hp: 10,
    };
    assert_eq!(
        nameless.validate(),
        Err(GmActionRefusalReason::InvalidAction)
    );

    // A narrowed scope is the same action, so it validates the same way —
    // and its id's SHAPE is checked here rather than only against the live
    // hull, for the palette id's reason: an unbounded or empty Station key
    // is a malformed action, not an unknown Station, and must never reach
    // the canonical journal to be told apart at an apply tick (#1311).
    for scope in [effect_station("helm"), effect_system("impulse-drive")] {
        assert_eq!(
            GmAction::ApplyDirectEffect {
                target: "npc-1".into(),
                scope,
                effect: crate::gm_effect::GmDirectEffectKind::Damage,
                amount_milli_hp: 1,
            }
            .validate(),
            Ok(())
        );
    }
    for scope in [
        effect_station(""),
        effect_system(""),
        effect_station(&"x".repeat(4096)),
        effect_system("helm\u{0}"),
    ] {
        assert_eq!(
            GmAction::ApplyDirectEffect {
                target: "npc-1".into(),
                scope,
                effect: crate::gm_effect::GmDirectEffectKind::Damage,
                amount_milli_hp: 1,
            }
            .validate(),
            Err(GmActionRefusalReason::InvalidAction)
        );
    }
}

fn comms_scope_request() -> GmActionRequest {
    use crate::command_admission::log::ShipKey;
    GmActionRequest {
        operator_id: "gm-2".into(),
        correlation: GmActionId::new("comms-scope").unwrap(),
        action: GmAction::TransmitComms {
            transmission: crate::gm_comms::GmCommsTransmission {
                sender: "speaker".into(),
                route: "private".into(),
                recipients: vec![ShipKey("ship-a".into()), ShipKey("ship-b".into())],
                content: crate::gm_comms::GmCommsContent::Literal {
                    text: "Exact text".into(),
                },
            },
        },
    }
}

#[test]
fn comms_scope_survives_local_and_replicated_refusals_and_rejects_malformed_frames() {
    use crate::lockstep::frame::MeshFrame;
    let request = comms_scope_request();
    let local = LoggedGmAction::refused_request(&request, 12, GmActionRefusalReason::WrongPhase);
    assert_eq!(local.comms_recipients, request.action.comms_recipients());
    let local_json = serde_json::to_string(&local).unwrap();
    assert_eq!(
        serde_json::from_str::<LoggedGmAction>(&local_json).unwrap(),
        local
    );
    let proposal = GmActionProposal {
        from: HostSlot(2),
        operator_id: request.operator_id,
        correlation: request.correlation,
        action: request.action,
    };
    let refusal = refusal_for(
        HostSlot(1),
        &proposal,
        12,
        GmActionRefusalReason::WrongPhase,
    );
    assert_eq!(refusal.logged(), local);
    let roster = crate::lockstep::FleetRoster::with_participants_and_gms(
        Vec::new(),
        vec![HostSlot(1), HostSlot(2)],
        vec![crate::lockstep::FleetGm {
            host: HostSlot(2),
            operator_id: "gm-2".into(),
        }],
        HostSlot(1),
        HostSlot(1),
    )
    .unwrap();
    let frame = GmActionFrame::Refused(refusal.clone());
    assert_eq!(validate_fleet_frame(&frame, &roster), Ok(()));
    let wire = crate::core::codec::encode_mesh_frame(&MeshFrame::GmAction(frame.clone())).unwrap();
    assert_eq!(
        crate::core::codec::decode_mesh_frame(&wire),
        Some(MeshFrame::GmAction(frame))
    );
    for malformed in [serde_json::json!("ship-a"), serde_json::json!([7])] {
        let mut body: serde_json::Value = serde_json::from_str(&wire).unwrap();
        body["d"]["comms_recipients"] = malformed;
        assert!(crate::core::codec::decode_mesh_frame(&body.to_string()).is_none());
    }
    let mut absent: serde_json::Value = serde_json::from_str(&wire).unwrap();
    absent["d"]
        .as_object_mut()
        .unwrap()
        .remove("comms_recipients");
    assert!(crate::core::codec::decode_mesh_frame(&absent.to_string()).is_none());
    for recipients in [
        None,
        Some(vec![]),
        Some(vec!["ship-a".into(), "ship-a".into()]),
        Some(vec!["ship-b".into(), "ship-a".into()]),
        Some(vec![String::new()]),
        Some(vec!["x".repeat(129)]),
        Some(vec!["ship\n".into()]),
        Some((0..33).map(|i| format!("ship-{i:02}")).collect()),
    ] {
        let bad = GmActionRefusal {
            comms_recipients: recipients,
            ..refusal.clone()
        };
        assert_eq!(
            validate_fleet_frame(&GmActionFrame::Refused(bad.clone()), &roster),
            Err(GmActionRefusalReason::InvalidAction)
        );
        let wire = crate::core::codec::encode_mesh_frame(&MeshFrame::GmAction(
            GmActionFrame::Refused(bad),
        ))
        .unwrap();
        assert!(crate::core::codec::decode_mesh_frame(&wire).is_none());
    }
    let limit = GmActionRefusal {
        comms_recipients: Some(
            (0..32)
                .map(|i| format!("{i:02}{}", "x".repeat(126)))
                .collect(),
        ),
        ..refusal.clone()
    };
    assert_eq!(
        validate_fleet_frame(&GmActionFrame::Refused(limit), &roster),
        Ok(())
    );
    // No other action family may acquire an audience, nor should its old
    // serialization gain even a null key when this field is absent.
    let pause = GmActionProposal {
        action: GmAction::SetSessionPaused { active: true },
        ..proposal
    };
    let old = refusal_for(HostSlot(1), &pause, 12, GmActionRefusalReason::WrongPhase);
    let old_wire = crate::core::codec::encode_mesh_frame(&MeshFrame::GmAction(
        GmActionFrame::Refused(old.clone()),
    ))
    .unwrap();
    assert!(!old_wire.contains("comms_recipients"));
    assert!(!serde_json::to_string(&old.logged())
        .unwrap()
        .contains("comms_recipients"));
    let decoded: LoggedGmAction =
        serde_json::from_str(&serde_json::to_string(&old.logged()).unwrap()).unwrap();
    assert_eq!(decoded.comms_recipients, None);
    let bad = GmActionRefusal {
        comms_recipients: refusal.comms_recipients,
        ..old
    };
    assert_eq!(
        validate_fleet_frame(&GmActionFrame::Refused(bad.clone()), &roster),
        Err(GmActionRefusalReason::InvalidAction)
    );
    let bad_wire =
        crate::core::codec::encode_mesh_frame(&MeshFrame::GmAction(GmActionFrame::Refused(bad)))
            .unwrap();
    assert!(crate::core::codec::decode_mesh_frame(&bad_wire).is_none());
}

#[test]
fn comms_scope_is_checked_against_the_canonical_grant_on_restore() {
    let request = comms_scope_request();
    let mut journal = GmActionJournal::default();
    let mut grant = station_grant(1, 12, "comms-scope", request.action);
    grant.operator_id = request.operator_id;
    journal.insert(grant).unwrap();
    journal.restore_applied_frontier(1).unwrap();
    assert_eq!(
        journal.applied_results()[0].comms_recipients,
        Some(vec!["ship-a".into(), "ship-b".into()])
    );
    let stored = serde_json::to_value(&journal).unwrap();
    assert_eq!(
        serde_json::from_value::<GmActionJournal>(stored.clone()).unwrap(),
        journal
    );
    for metadata in [
        serde_json::Value::Null,
        serde_json::json!(["ship-a"]),
        serde_json::json!(["ship-b", "ship-a"]),
        serde_json::json!(["ship-a", "ship-c"]),
    ] {
        let mut changed = stored.clone();
        changed["applied_results"][0]["comms_recipients"] = metadata;
        assert!(serde_json::from_value::<GmActionJournal>(changed).is_err());
    }
    let mut absent = stored;
    absent["applied_results"][0]
        .as_object_mut()
        .unwrap()
        .remove("comms_recipients");
    assert!(serde_json::from_value::<GmActionJournal>(absent).is_err());
}

/// A replicated refusal must name the entity it refused, for the same
/// reason an event-control refusal must name its event.
#[test]
fn a_targetless_direct_effect_refusal_is_a_malformed_frame() {
    let roster = crate::lockstep::FleetRoster::with_participants_and_gms(
        Vec::new(),
        vec![HostSlot(1), HostSlot(2)],
        vec![
            crate::lockstep::FleetGm {
                host: HostSlot(1),
                operator_id: "gm-1".into(),
            },
            crate::lockstep::FleetGm {
                host: HostSlot(2),
                operator_id: "gm-2".into(),
            },
        ],
        HostSlot(1),
        HostSlot(1),
    )
    .unwrap();
    let refusal = GmActionRefusal {
        sequenced_by: HostSlot(1),
        requester: HostSlot(2),
        operator_id: "gm-2".into(),
        correlation: GmActionId::new("effect-refusal").unwrap(),
        effect_scope: None,
        objective_verb: None,
        objective_instance_scope: None,
        objective_recipients: None,
        comms_recipients: None,
        observer: None,
        npc_doctrine: None,
        action_kind: GmActionKind::DirectEffect,
        requested_active: true,
        tick: 3,
        reason: GmActionRefusalReason::UnknownEntity,
        lever: None,
        target: None,
        verb: None,
    };
    assert_eq!(
        validate_fleet_frame(&GmActionFrame::Refused(refusal.clone()), &roster),
        Err(GmActionRefusalReason::InvalidAction)
    );
    let named = GmActionRefusal {
        target: Some("npc-1".into()),
        ..refusal
    };
    assert_eq!(
        validate_fleet_frame(&GmActionFrame::Refused(named), &roster),
        Ok(())
    );
}
