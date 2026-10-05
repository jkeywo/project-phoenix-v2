use super::*;
use crate::civilian::traffic::{
    ComplianceState, OrderResponse, RouteCompletion, RouteConfig, RouteLeg,
};

fn route() -> RouteConfig {
    RouteConfig {
        id: "depot_run".into(),
        legs: vec![
            RouteLeg {
                anchor: "depot_north".into(),
                speed: 0.4,
                hold_secs: 0,
            },
            RouteLeg {
                anchor: "depot_south".into(),
                speed: 0.8,
                hold_secs: 0,
            },
        ],
        on_complete: RouteCompletion::Loop,
    }
}

// ── AC2: every travel intent becomes an authored directive, not a mover ──

#[test]
fn a_route_becomes_a_patrol_directive_over_its_anchor_chain() {
    let entry = directive_for(
        &CivilianTravel::Route {
            id: "depot_run".into(),
        },
        &CivilianConfig::default(),
        Some(&route()),
        0.4,
    )
    .expect("a routed civilian has a directive");
    assert_eq!(entry.id, CIVILIAN_ROUTE_OBJECTIVE_ID);
    assert_eq!(entry.directive_kind.as_deref(), Some("Patrol"));
    assert_eq!(entry.directive_anchors, vec!["depot_north", "depot_south"]);
    assert!(entry.directive_loop, "the authored ending travels across");
    assert_eq!(
        entry.target_speed, 0.4,
        "the current leg's authored speed is the directive's speed"
    );
    assert_eq!(entry.use_impulse, Some(true));
    let cruise = directive_for(
        &CivilianTravel::Route {
            id: "depot_run".into(),
        },
        &CivilianConfig {
            use_impulse: Some(false),
            ..Default::default()
        },
        Some(&route()),
        0.18,
    )
    .expect("a convoy route has a directive");
    assert_eq!(cruise.use_impulse, Some(false));
    assert_eq!(cruise.target_speed, 0.18);
}

#[test]
fn an_anchor_divert_becomes_a_reach_and_a_dock_becomes_a_dock() {
    let anchor = directive_for(
        &CivilianTravel::Anchor {
            name: "holding_point".into(),
        },
        &CivilianConfig::default(),
        None,
        0.5,
    )
    .expect("an anchor divert has a directive");
    assert_eq!(anchor.directive_kind.as_deref(), Some("Reach"));
    assert_eq!(anchor.directive_anchor.as_deref(), Some("holding_point"));

    let dock = directive_for(
        &CivilianTravel::Dock {
            structure: "skyhook_depot".into(),
        },
        &CivilianConfig::default(),
        None,
        0.5,
    )
    .expect("a dock order has a directive");
    assert_eq!(dock.directive_kind.as_deref(), Some("Dock"));
    assert_eq!(
        dock.directive_dock_target.as_deref(),
        Some("skyhook_depot"),
        "the structure name travels on the directive, to be resolved by the \
             same navigation-objective path a Destroy target uses"
    );
}

#[test]
fn holding_station_installs_no_helm_relevant_directive_at_all() {
    assert!(
        directive_for(
            &CivilianTravel::Hold,
            &CivilianConfig::default(),
            Some(&route()),
            0.0
        )
        .is_none(),
        "a held civilian is flown by the existing 'no objective ⇒ zero \
             throttle' arm, not by a stop command this slice invented"
    );
}

#[test]
fn a_routed_civilian_with_no_such_route_installs_nothing() {
    assert!(
        directive_for(
            &CivilianTravel::Route {
                id: "no_such_lane".into()
            },
            &CivilianConfig::default(),
            None,
            0.5
        )
        .is_none(),
        "an unresolvable lane must not become an anchorless Patrol the helm \
             would silently ignore"
    );
}

// ── The cursor survives a speed change and is dropped on a real divert ──

#[test]
fn installing_the_same_destination_at_a_new_speed_does_not_invalidate_the_cursor() {
    let mut doctrine = Vec::new();
    assert!(
        install_directive(
            &mut doctrine,
            directive_for(
                &CivilianTravel::Route {
                    id: "depot_run".into()
                },
                &CivilianConfig::default(),
                Some(&route()),
                0.4
            )
        ),
        "the first install is a change"
    );
    assert_eq!(doctrine.len(), 1);
    assert!(
        !install_directive(
            &mut doctrine,
            directive_for(
                &CivilianTravel::Route {
                    id: "depot_run".into()
                },
                &CivilianConfig::default(),
                Some(&route()),
                0.8
            )
        ),
        "the leg's speed moving as the cursor advances must not reset the \
             cursor that moved it"
    );
    assert_eq!(doctrine[0].target_speed, 0.8, "…but the speed still lands");
}

#[test]
fn changing_where_the_civilian_is_going_invalidates_the_cursor() {
    let mut doctrine = Vec::new();
    install_directive(
        &mut doctrine,
        directive_for(
            &CivilianTravel::Route {
                id: "depot_run".into(),
            },
            &CivilianConfig::default(),
            Some(&route()),
            0.4,
        ),
    );
    assert!(install_directive(
        &mut doctrine,
        directive_for(
            &CivilianTravel::Anchor {
                name: "holding_point".into()
            },
            &CivilianConfig::default(),
            None,
            0.5
        )
    ));
    assert!(
        install_directive(&mut doctrine, None),
        "and so does being told to stop"
    );
    assert!(
        doctrine.is_empty(),
        "a held civilian's entry is removed rather than left scoring"
    );
}

#[test]
fn the_civilian_entry_never_disturbs_the_hulls_own_doctrine() {
    let own = DoctrineObjective {
        id: "reach-destination".into(),
        directive_kind: Some("Reach".into()),
        directive_anchor: Some("home".into()),
        base_priority: 30.0,
        ..DoctrineObjective::default()
    };
    let mut doctrine = vec![own.clone()];
    install_directive(
        &mut doctrine,
        directive_for(
            &CivilianTravel::Route {
                id: "depot_run".into(),
            },
            &CivilianConfig::default(),
            Some(&route()),
            0.4,
        ),
    );
    install_directive(&mut doctrine, None);
    assert_eq!(
        doctrine,
        vec![own],
        "installing and removing the civilian entry leaves the hull's own \
             authored doctrine exactly as it was"
    );
}

// ── AC5: the disposition ladder is entity, then faction, then default ──

#[test]
fn an_entity_disposition_wins_over_its_factions_and_absence_falls_all_the_way_back() {
    let faction_uuid = uuid::Uuid::from_u128(7);
    let mut registry = crate::ai::faction::FactionRegistry::new();
    registry.insert(crate::ai::faction::FactionConfig {
        display_name: None,
        uuid: faction_uuid,
        name: "Kestrel Combine".into(),
        enemies: Vec::new(),
        compliance: Some(ComplianceDisposition {
            divert: OrderResponse::Refuse,
            ..ComplianceDisposition::default()
        }),
    });
    let res = crate::entities::config_cache::FactionRegistryResource(registry);
    let member = FactionComponent(faction_uuid);

    let inherited = resolve_disposition(&CivilianConfig::default(), Some(&member), Some(&res));
    assert_eq!(
        inherited.divert,
        OrderResponse::Refuse,
        "a hull that authors nothing takes its faction's temperament"
    );

    let own = CivilianConfig {
        compliance: Some(ComplianceDisposition::default()),
        ..CivilianConfig::default()
    };
    assert_eq!(
        resolve_disposition(&own, Some(&member), Some(&res)).divert,
        OrderResponse::Comply,
        "…and its own table overrides it"
    );

    assert_eq!(
        resolve_disposition(&CivilianConfig::default(), None, Some(&res)),
        ComplianceDisposition::default(),
        "a factionless civilian is a cooperative one"
    );
}

// ── AC6: what "can this still be carried out?" means against a live world ──

#[test]
fn a_dock_target_this_world_does_not_have_does_not_resolve() {
    let mut runtime = WorldContentRuntime::default();
    runtime
        .name_to_uuid
        .insert("skyhook_depot".into(), "uuid-1".into());
    assert!(destination_resolves(
        Some(&CivilianOrder::dock_at("skyhook_depot")),
        &runtime,
        None
    ));
    assert!(!destination_resolves(
        Some(&CivilianOrder::dock_at("a_depot_that_is_gone")),
        &runtime,
        None
    ));
}

#[test]
fn a_hold_always_resolves_and_a_divert_needs_its_destination_declared() {
    let runtime = WorldContentRuntime::default();
    assert!(destination_resolves(
        Some(&CivilianOrder::Hold),
        &runtime,
        None
    ));
    assert!(destination_resolves(None, &runtime, None));
    assert!(!destination_resolves(
        Some(&CivilianOrder::divert_to_anchor("nowhere")),
        &runtime,
        None
    ));
    assert!(!destination_resolves(
        Some(&CivilianOrder::divert_to_route("no_such_lane")),
        &runtime,
        None
    ));
}

// ── The whole loop, on a bare app ──

fn test_app() -> App {
    let mut app = App::new();
    app.init_resource::<WorldContentRuntime>();
    app.init_resource::<crate::sim_tick::SimTick>();
    app.init_resource::<crate::server_app::SimOutbox>();
    app.configure_sets(FixedUpdate, crate::sim_sets::SimSet::Input);
    app.add_plugins(CivilianPlugin);
    app
}

fn spawn_civilian(app: &mut App, uuid: &str, config: CivilianConfig) -> Entity {
    let state = CivilianState::from_config(&config);
    app.world_mut()
        .spawn((
            EntityUuid(uuid.to_string()),
            CivilianSection(config),
            CivilianTraffic(state),
            BehaviourSection(crate::entities::config::BehaviourConfig::default()),
        ))
        .id()
}

/// **AC4/AC6.** A scripted order walks the whole machine on a live app, and
/// the doctrine entry follows it.
#[test]
fn a_queued_order_is_taken_answered_and_installed_as_a_directive() {
    let mut app = test_app();
    let e = spawn_civilian(
        &mut app,
        "civ-1",
        CivilianConfig {
            compliance: Some(ComplianceDisposition {
                ack_secs: 0,
                decide_secs: 0,
                ..ComplianceDisposition::default()
            }),
            ..CivilianConfig::default()
        },
    );
    app.world_mut()
        .resource_mut::<EffectQueue<PendingCivilianOrder>>()
        .0
        .push(PendingCivilianOrder {
            uuid: "civ-1".into(),
            order: CivilianOrder::divert_to_anchor("holding_point"),
        });

    // Tick one takes the order; with zero authored delays it also answers it.
    app.world_mut().run_schedule(FixedUpdate);
    app.world_mut().run_schedule(FixedUpdate);

    let state = app.world().get::<CivilianTraffic>(e).expect("still there");
    assert_eq!(
        state.0.compliance(),
        ComplianceState::NonCompliant,
        "no world config means the anchor resolves nowhere, which is the \
             distinguishable stuck state rather than a silent stall"
    );
    assert!(
        app.world()
            .get::<BehaviourSection>(e)
            .expect("still there")
            .0
            .doctrine
            .is_empty(),
        "…and a stuck civilian holds station, which is no directive at all"
    );
}

#[test]
fn a_correlated_console_order_is_applied_only_by_the_civilian_owner() {
    use crate::core::messages::{AdmittedCommand, SystemId};

    let mut app = test_app();
    let civilian_uuid = uuid::Uuid::from_u128(0x1286).to_string();
    let civilian = spawn_civilian(&mut app, &civilian_uuid, CivilianConfig::default());
    app.world_mut()
        .spawn(AdmittedCommands(vec![AdmittedCommand {
            target: SystemId(crate::ship::system_registry::NAVIGATION_SYSTEM_ID.to_string()),
            payload: SystemControlPayload::OrderCivilian {
                target: civilian_uuid,
                order: CivilianOrder::Hold,
            },
            response_token: Some("nav-holder".into()),
            feedback_correlation: Some(ActionCorrelationId::new("civilian-hold").unwrap()),
        }]));

    app.world_mut().run_schedule(FixedUpdate);

    assert_eq!(
        app.world()
            .get::<CivilianTraffic>(civilian)
            .and_then(|state| state.0.order()),
        Some(&CivilianOrder::Hold),
        "the traffic owner must receive the order before it reports Applied"
    );
    let feedback: Vec<_> = app
        .world()
        .resource::<crate::server_app::SimOutbox>()
        .iter()
        .filter_map(|(target, message)| match message {
            ServerMessage::ActionFeedback {
                correlation,
                outcome,
            } => Some((target.clone(), correlation.as_str().to_string(), *outcome)),
            _ => None,
        })
        .collect();
    assert_eq!(feedback.len(), 1);
    assert!(matches!(
        &feedback[0],
        (crate::lobby::Target::Token(token), correlation, ActionFeedbackOutcome::Applied)
            if token == "nav-holder" && correlation == "civilian-hold"
    ));
}

#[test]
fn correlated_unknown_and_malformed_orders_are_refused_by_the_civilian_owner() {
    use crate::core::messages::{AdmittedCommand, SystemId};

    let mut app = test_app();
    let civilian_uuid = uuid::Uuid::from_u128(0x1287).to_string();
    let civilian = spawn_civilian(&mut app, &civilian_uuid, CivilianConfig::default());
    let command = |target: &str, order: CivilianOrder, correlation: &str| AdmittedCommand {
        target: SystemId(crate::ship::system_registry::NAVIGATION_SYSTEM_ID.to_string()),
        payload: SystemControlPayload::OrderCivilian {
            target: target.to_string(),
            order,
        },
        response_token: Some("nav-holder".into()),
        feedback_correlation: Some(ActionCorrelationId::new(correlation).unwrap()),
    };
    app.world_mut().spawn(AdmittedCommands(vec![
        command("no-such-civilian", CivilianOrder::Hold, "civilian-unknown"),
        command(
            &civilian_uuid,
            CivilianOrder::Divert {
                route: Some("lane-a".into()),
                anchor: Some("anchor-a".into()),
            },
            "civilian-malformed",
        ),
    ]));

    app.world_mut().run_schedule(FixedUpdate);

    let outbox = app.world().resource::<crate::server_app::SimOutbox>();
    let feedback: Vec<_> = outbox
        .iter()
        .filter_map(|(target, message)| match message {
            ServerMessage::ActionFeedback {
                correlation,
                outcome,
            } => Some((target.clone(), correlation.as_str().to_string(), *outcome)),
            _ => None,
        })
        .collect();
    assert_eq!(feedback.len(), 2);
    for expected in ["civilian-unknown", "civilian-malformed"] {
        assert!(feedback.iter().any(|(target, correlation, outcome)| {
            matches!(target, crate::lobby::Target::Token(token) if token == "nav-holder")
                && correlation == expected
                && *outcome == ActionFeedbackOutcome::Refused
        }));
    }
    assert_eq!(
        outbox
            .iter()
            .filter(|(_, message)| matches!(message, ServerMessage::CivilianOrderRejected { .. }))
            .count(),
        2,
        "the existing crew-facing rejection reasons remain alongside terminal feedback"
    );
    assert_eq!(
        app.world()
            .get::<CivilianTraffic>(civilian)
            .expect("civilian remains")
            .0
            .compliance(),
        ComplianceState::Unordered,
        "neither refused occurrence may mutate civilian traffic"
    );
}

#[test]
fn correlated_order_is_refused_when_no_civilian_owner_remains() {
    use crate::core::messages::{AdmittedCommand, SystemId};

    let mut app = test_app();
    let departed_uuid = uuid::Uuid::from_u128(0x1288).to_string();
    app.world_mut()
        .spawn(AdmittedCommands(vec![AdmittedCommand {
            target: SystemId(crate::ship::system_registry::NAVIGATION_SYSTEM_ID.to_string()),
            payload: SystemControlPayload::OrderCivilian {
                target: departed_uuid.clone(),
                order: CivilianOrder::Hold,
            },
            response_token: Some("nav-holder".into()),
            feedback_correlation: Some(ActionCorrelationId::new("civilian-departed").unwrap()),
        }]));

    app.world_mut().run_schedule(FixedUpdate);

    let outbox = app.world().resource::<crate::server_app::SimOutbox>();
    assert!(outbox.iter().any(|(target, message)| matches!(
        (target, message),
        (
            crate::lobby::Target::Token(token),
            ServerMessage::ActionFeedback {
                correlation,
                outcome: ActionFeedbackOutcome::Refused,
            },
        ) if token == "nav-holder" && correlation.as_str() == "civilian-departed"
    )));
    assert!(outbox.iter().any(|(target, message)| matches!(
        (target, message),
        (
            crate::lobby::Target::Token(token),
            ServerMessage::CivilianOrderRejected { target, reason },
        ) if token == "nav-holder"
            && target == &departed_uuid
            && reason == REJECT_UNKNOWN_CIVILIAN
    )));
}

/// **AC3.** An order the host cannot deliver bounces back to the console
/// that sent it, with a reason, rather than vanishing into a queue nobody
/// drains.
#[test]
fn an_undeliverable_order_is_refused_with_a_reason_on_the_senders_own_token() {
    use crate::core::messages::{AdmittedCommand, SystemId};

    let mut app = test_app();
    spawn_civilian(&mut app, "civ-1", CivilianConfig::default());
    // A named entity that is NOT traffic: resolving is not the same as
    // being addressable, and an order to a rock must not queue.
    app.world_mut()
        .resource_mut::<WorldContentRuntime>()
        .name_to_uuid
        .insert("a_rock".into(), "rock-1".into());

    let console = |target: &str, order: CivilianOrder| AdmittedCommand {
        target: SystemId(crate::ship::system_registry::NAVIGATION_SYSTEM_ID.to_string()),
        payload: SystemControlPayload::OrderCivilian {
            target: target.to_string(),
            order,
        },
        response_token: Some("nav-holder".into()),
        feedback_correlation: None,
    };
    app.world_mut().spawn(AdmittedCommands(vec![
        console("nobody", CivilianOrder::Hold),
        console("a_rock", CivilianOrder::Hold),
        console(
            "civ-1",
            CivilianOrder::Divert {
                route: Some("a".into()),
                anchor: Some("b".into()),
            },
        ),
    ]));

    app.world_mut().run_schedule(FixedUpdate);

    let bounced: Vec<(String, String)> = app
        .world()
        .resource::<crate::server_app::SimOutbox>()
        .iter()
        .filter_map(|(target, msg)| match (target, msg) {
            (
                crate::lobby::Target::Token(token),
                crate::core::messages::ServerMessage::CivilianOrderRejected { target, reason },
            ) => Some((token.clone(), format!("{target}:{reason}"))),
            _ => None,
        })
        .collect();
    assert_eq!(
        bounced,
        vec![
            (
                "nav-holder".to_string(),
                format!("nobody:{REJECT_UNKNOWN_CIVILIAN}")
            ),
            (
                "nav-holder".to_string(),
                format!("a_rock:{REJECT_UNKNOWN_CIVILIAN}")
            ),
            (
                "nav-holder".to_string(),
                format!("civ-1:{REJECT_MALFORMED_ORDER}")
            ),
        ],
        "an unknown craft, a craft that is not traffic, and a divert naming two \
             destinations all bounce — on the token the command arrived with"
    );
    let mut q = app.world_mut().query::<&CivilianTraffic>();
    let states: Vec<ComplianceState> = q.iter(app.world()).map(|t| t.0.compliance()).collect();
    assert_eq!(
        states,
        vec![ComplianceState::Unordered],
        "…and the malformed order never reaches the craft it named"
    );
}

#[test]
fn a_world_with_no_civilians_and_no_orders_leaves_the_runtime_alone() {
    let mut app = test_app();
    app.world_mut().run_schedule(FixedUpdate);
    assert!(app
        .world()
        .resource::<EffectQueue<PendingCivilianOrder>>()
        .0
        .is_empty());
}
