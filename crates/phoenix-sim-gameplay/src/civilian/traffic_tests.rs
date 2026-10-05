use super::*;

/// 60 Hz, the default `[global] sim_tick_hz`.
const HZ: f32 = 60.0;

fn depot_run() -> RouteConfig {
    toml::from_str(
        r#"
id = "depot_run"
on_complete = "loop"

[[leg]]
anchor = "depot_north"
speed = 0.4
hold_secs = 10

[[leg]]
anchor = "depot_south"
"#,
    )
    .expect("the fixture parses")
}

fn cooperative() -> ComplianceDisposition {
    ComplianceDisposition::default()
}

fn stubborn() -> ComplianceDisposition {
    ComplianceDisposition {
        divert: OrderResponse::Refuse,
        refusal_reason: "world.hauler.refuses".to_string(),
        ..ComplianceDisposition::default()
    }
}

fn ordered(state: &mut CivilianState, order: CivilianOrder, disposition: &ComplianceDisposition) {
    state.receive_order(order, disposition, 0, HZ);
}

/// Run `advance` from tick 0 up to and including `until`, collecting every
/// transition — the deterministic shape the headless probe asserts on.
fn run(
    state: &mut CivilianState,
    until: u64,
    resolves: bool,
    disposition: &ComplianceDisposition,
) -> Vec<ComplianceState> {
    let mut seen = Vec::new();
    for tick in 0..=until {
        if let Some(t) = state.advance(tick, resolves, disposition, HZ) {
            seen.push(t.to);
        }
    }
    seen
}

// ── AC1: the route vocabulary parses, and a broken one is refused ──

#[test]
fn an_authored_route_parses_into_an_anchor_chain_and_a_loop_flag() {
    let route = depot_run();
    assert_eq!(
        route.anchor_chain(),
        vec!["depot_north".to_string(), "depot_south".to_string()],
        "the anchor chain is exactly what an AiDirective::Patrol carries"
    );
    assert!(route.loops(), "on_complete = \"loop\" wraps the chain");
    assert_eq!(
        route.leg(0).map(|l| l.speed),
        Some(0.4),
        "an authored per-leg speed survives"
    );
    assert_eq!(
        route.leg(1).map(|l| l.speed),
        Some(default_leg_speed()),
        "…and a leg that authors none takes the parse fallback"
    );
    assert_eq!(
        route.leg(2).map(|l| l.anchor.as_str()),
        Some("depot_north"),
        "a looping route wraps its leg lookup"
    );
}

#[test]
fn a_terminating_route_saturates_at_its_last_leg_instead_of_wrapping() {
    let route = RouteConfig {
        id: "one_way".into(),
        legs: vec![
            RouteLeg {
                anchor: "a".into(),
                speed: 0.5,
                hold_secs: 0,
            },
            RouteLeg {
                anchor: "b".into(),
                speed: 0.5,
                hold_secs: 0,
            },
        ],
        on_complete: RouteCompletion::Terminate,
    };
    assert!(!route.loops());
    assert_eq!(
        route.leg(9).map(|l| l.anchor.as_str()),
        Some("b"),
        "past the end, a terminating route is parked on its final leg"
    );
}

#[test]
fn a_route_that_cannot_mean_anything_is_refused_by_name() {
    let cases: Vec<(RouteConfig, &str)> = vec![
        (
            RouteConfig {
                id: "  ".into(),
                legs: vec![RouteLeg {
                    anchor: "a".into(),
                    speed: 0.5,
                    hold_secs: 0,
                }],
                on_complete: RouteCompletion::Loop,
            },
            "empty id",
        ),
        (
            RouteConfig {
                id: "empty".into(),
                legs: vec![],
                on_complete: RouteCompletion::Loop,
            },
            "no [[route.leg]]",
        ),
        (
            RouteConfig {
                id: "blank_anchor".into(),
                legs: vec![RouteLeg {
                    anchor: "".into(),
                    speed: 0.5,
                    hold_secs: 0,
                }],
                on_complete: RouteCompletion::Loop,
            },
            "empty anchor",
        ),
        (
            RouteConfig {
                id: "too_fast".into(),
                legs: vec![RouteLeg {
                    anchor: "a".into(),
                    speed: 1.5,
                    hold_secs: 0,
                }],
                on_complete: RouteCompletion::Loop,
            },
            "speed above 1.0",
        ),
    ];
    for (route, what) in cases {
        assert!(
            route.validate().is_err(),
            "a route with {what} must be a load error, not a civilian that \
                 silently never goes anywhere"
        );
    }
    assert!(depot_run().validate().is_ok(), "the exemplar is legal");
}

// ── AC2: orders share one evaluation path, and a malformed one is refused ──

#[test]
fn a_divert_naming_both_or_neither_destination_is_refused() {
    assert!(CivilianOrder::divert_to_route("depot_run")
        .validate()
        .is_ok());
    assert!(CivilianOrder::divert_to_anchor("holding_point")
        .validate()
        .is_ok());
    assert!(CivilianOrder::Divert {
        route: Some("a".into()),
        anchor: Some("b".into()),
    }
    .validate()
    .is_err());
    assert!(CivilianOrder::Divert {
        route: None,
        anchor: None,
    }
    .validate()
    .is_err());
    assert!(CivilianOrder::dock_at("").validate().is_err());
    assert!(CivilianOrder::Hold.validate().is_ok());
}

#[test]
fn every_verb_reports_its_own_kind_for_disposition_lookup() {
    assert_eq!(CivilianOrder::Hold.kind(), OrderKind::Hold);
    assert_eq!(
        CivilianOrder::divert_to_route("r").kind(),
        OrderKind::Divert
    );
    assert_eq!(
        CivilianOrder::divert_to_anchor("a").kind(),
        OrderKind::Divert
    );
    assert_eq!(CivilianOrder::dock_at("s").kind(), OrderKind::Dock);
}

// ── AC3/AC4: the compliance machine, one transition at a time ──

#[test]
fn a_cooperative_civilian_walks_received_acknowledged_complying_on_authored_ticks() {
    let mut state = CivilianState::from_config(&CivilianConfig {
        route: Some("depot_run".into()),
        ..CivilianConfig::default()
    });
    let d = cooperative();
    let t = state
        .receive_order(CivilianOrder::Hold, &d, 0, HZ)
        .expect("taking an order is always a transition");
    assert_eq!(t.from, ComplianceState::Unordered);
    assert_eq!(t.to, ComplianceState::Received);
    assert_eq!(
        state.due_tick(),
        120,
        "two authored seconds at 60 Hz is 120 ticks"
    );

    assert!(
        state.advance(119, true, &d, HZ).is_none(),
        "nothing moves before the authored acknowledgement time"
    );
    assert_eq!(
        state.advance(120, true, &d, HZ).map(|t| t.to),
        Some(ComplianceState::Acknowledged)
    );
    assert_eq!(
        state.due_tick(),
        300,
        "…and the decide clock is three more authored seconds"
    );
    assert!(state.advance(299, true, &d, HZ).is_none());
    assert_eq!(
        state.advance(300, true, &d, HZ).map(|t| t.to),
        Some(ComplianceState::Complying)
    );
    assert_eq!(state.travel(), CivilianTravel::Hold, "and it holds station");
    assert!(
        state.advance(301, true, &d, HZ).is_none(),
        "complying is a resting state, not a loop that keeps transitioning"
    );
}

#[test]
fn a_refusing_civilian_answers_with_its_authored_reason_and_keeps_flying_its_route() {
    let mut state = CivilianState::from_config(&CivilianConfig {
        route: Some("depot_run".into()),
        ..CivilianConfig::default()
    });
    let d = stubborn();
    ordered(&mut state, CivilianOrder::divert_to_anchor("far_side"), &d);
    let seen = run(&mut state, 400, true, &d);
    assert_eq!(
        seen,
        vec![ComplianceState::Refused],
        "a refusal is one transition out of received — it never acknowledges \
             its way to complying"
    );
    assert_eq!(
        state.reason(),
        Some("world.hauler.refuses"),
        "the reason is the authored string id, not one the code invented"
    );
    assert_eq!(
        state.travel(),
        CivilianTravel::Route {
            id: "depot_run".into()
        },
        "a refusal is a decision, so the civilian carries on with its own route"
    );
}

#[test]
fn the_same_hull_complies_with_a_verb_it_does_not_refuse() {
    let mut state = CivilianState::default();
    let d = stubborn();
    ordered(&mut state, CivilianOrder::Hold, &d);
    let seen = run(&mut state, 400, true, &d);
    assert_eq!(
        seen,
        vec![ComplianceState::Acknowledged, ComplianceState::Complying],
        "disposition is per verb: this hull refuses diverts, not holds"
    );
}

#[test]
fn a_complied_divert_onto_a_route_becomes_the_civilians_own_route_from_leg_zero() {
    let mut state = CivilianState::from_config(&CivilianConfig {
        route: Some("depot_run".into()),
        ..CivilianConfig::default()
    });
    let d = cooperative();
    state.observe_leg(1, Some(&depot_run()), 0, HZ);
    ordered(
        &mut state,
        CivilianOrder::divert_to_route("storm_detour"),
        &d,
    );
    run(&mut state, 400, true, &d);
    assert_eq!(state.route(), Some("storm_detour"));
    assert_eq!(state.leg(), 0, "a new lane is flown from its first leg");
    assert_eq!(
        state.travel(),
        CivilianTravel::Route {
            id: "storm_detour".into()
        }
    );
}

#[test]
fn a_divert_to_a_bare_anchor_reaches_it_without_becoming_a_route() {
    let mut state = CivilianState::from_config(&CivilianConfig {
        route: Some("depot_run".into()),
        ..CivilianConfig::default()
    });
    let d = cooperative();
    ordered(
        &mut state,
        CivilianOrder::divert_to_anchor("holding_point"),
        &d,
    );
    run(&mut state, 400, true, &d);
    assert_eq!(
        state.travel(),
        CivilianTravel::Anchor {
            name: "holding_point".into()
        }
    );
    assert_eq!(
        state.route(),
        Some("depot_run"),
        "its own lane is still its lane — a holding point is not a route"
    );
}

#[test]
fn a_dock_order_names_the_structure_for_the_adapter_to_close_on() {
    let mut state = CivilianState::default();
    let d = cooperative();
    ordered(&mut state, CivilianOrder::dock_at("skyhook_depot"), &d);
    run(&mut state, 400, true, &d);
    assert_eq!(
        state.travel(),
        CivilianTravel::Dock {
            structure: "skyhook_depot".into()
        }
    );
}

// ── AC6: unable to comply is its own state, not a silent stall ──

#[test]
fn an_order_whose_destination_never_resolves_lands_in_non_compliant_with_a_reason() {
    let mut state = CivilianState::from_config(&CivilianConfig {
        route: Some("depot_run".into()),
        ..CivilianConfig::default()
    });
    let d = cooperative();
    ordered(
        &mut state,
        CivilianOrder::dock_at("a_depot_that_is_gone"),
        &d,
    );
    let seen = run(&mut state, 400, false, &d);
    assert_eq!(
        seen,
        vec![ComplianceState::Acknowledged, ComplianceState::NonCompliant],
        "it agreed and then could not — which is not the same as refusing"
    );
    assert_eq!(state.reason(), Some(REASON_UNABLE));
    assert_eq!(
        state.travel(),
        CivilianTravel::Hold,
        "a stuck civilian stops where it is rather than wandering back onto \
             its lane as if nothing happened"
    );
    assert_ne!(
        state.compliance(),
        ComplianceState::Refused,
        "the whole point of the state is that a console can tell them apart"
    );
}

#[test]
fn a_civilian_mid_order_that_loses_its_destination_falls_out_of_complying() {
    let mut state = CivilianState::default();
    let d = cooperative();
    ordered(&mut state, CivilianOrder::dock_at("skyhook_depot"), &d);
    run(&mut state, 400, true, &d);
    assert_eq!(state.compliance(), ComplianceState::Complying);
    assert_eq!(
        state.advance(401, false, &d, HZ).map(|t| t.to),
        Some(ComplianceState::NonCompliant),
        "the depot going away mid-approach is exactly the case the state exists for"
    );
    assert_eq!(
        state.advance(402, true, &d, HZ).map(|t| t.to),
        Some(ComplianceState::Complying),
        "…and it resumes on its own if the world puts it back, because the \
             crew never withdrew the order"
    );
}

// ── AC1 (per-leg behaviour): the cursor is the leg pointer, dwell is authored ──

#[test]
fn the_cruise_speed_tracks_the_leg_the_cursor_is_on() {
    let route = depot_run();
    let mut state = CivilianState::from_config(&CivilianConfig {
        route: Some("depot_run".into()),
        ..CivilianConfig::default()
    });
    assert_eq!(
        state.cruise_speed(Some(&route), 0),
        0.4,
        "leg 0's authored speed"
    );
    state.observe_leg(1, Some(&route), 0, HZ);
    assert_eq!(
        state.leg(),
        1,
        "the cursor index is mirrored, not recomputed"
    );
    assert!(
        state.is_dwelling(0),
        "leg 0 authored a 10 s dwell, and leaving it starts the clock"
    );
    assert_eq!(
        state.cruise_speed(Some(&route), 0),
        0.0,
        "a dwelling civilian sits still — at zero throttle, on the same \
             directive, so its cursor keeps its place"
    );
    assert!(
        !state.is_dwelling(600),
        "…for exactly the authored ten seconds"
    );
    assert_eq!(
        state.cruise_speed(Some(&route), 600),
        default_leg_speed(),
        "and then flies leg 1 at leg 1's speed"
    );
}

#[test]
fn a_leg_with_no_authored_dwell_flies_straight_through() {
    let route = depot_run();
    let mut state = CivilianState::default();
    state.observe_leg(1, Some(&route), 0, HZ);
    state.observe_leg(0, Some(&route), 700, HZ);
    assert!(
        !state.is_dwelling(700),
        "leg 1 authors no hold_secs, so wrapping past it starts no dwell"
    );
}

// ── AC5: the disposition is authored, including its absence ──

#[test]
fn an_unauthored_disposition_is_a_cooperative_one_and_authoring_is_per_verb() {
    let parsed: ComplianceDisposition = toml::from_str("").expect("an empty table is legal");
    assert_eq!(parsed, ComplianceDisposition::default());
    assert_eq!(parsed.response(OrderKind::Dock), OrderResponse::Comply);

    let authored: ComplianceDisposition = toml::from_str(
        r#"
ack_secs = 1
decide_secs = 1
dock = "refuse"
refusal_reason = "world.convoy.will_not_dock"
"#,
    )
    .expect("the vocabulary parses");
    assert_eq!(authored.response(OrderKind::Dock), OrderResponse::Refuse);
    assert_eq!(authored.response(OrderKind::Hold), OrderResponse::Comply);
    assert_eq!(authored.response(OrderKind::Divert), OrderResponse::Comply);
}

#[test]
fn a_civilian_table_that_cannot_mean_anything_is_refused() {
    assert!(CivilianConfig::default().validate().is_ok());
    assert!(CivilianConfig {
        route: Some("   ".into()),
        ..CivilianConfig::default()
    }
    .validate()
    .is_err());
    assert!(CivilianConfig {
        route_priority: -1.0,
        ..CivilianConfig::default()
    }
    .validate()
    .is_err());
    assert!(CivilianConfig {
        compliance: Some(ComplianceDisposition {
            ack_secs: -1,
            ..ComplianceDisposition::default()
        }),
        ..CivilianConfig::default()
    }
    .validate()
    .is_err());
    assert!(CivilianConfig {
        order_options: vec![
            CivilianOrderOption {
                id: "clear_lane".into(),
                label: "world.test.clear_lane".into(),
                order: CivilianOrder::Hold,
            },
            CivilianOrderOption {
                id: "clear_lane".into(),
                label: "world.test.clear_lane_again".into(),
                order: CivilianOrder::divert_to_route("lee"),
            },
        ],
        ..CivilianConfig::default()
    }
    .validate()
    .is_err());
    assert!(CivilianConfig {
        order_options: vec![CivilianOrderOption {
            id: "bad".into(),
            label: "world.test.bad".into(),
            order: CivilianOrder::Divert {
                route: None,
                anchor: None,
            },
        }],
        ..CivilianConfig::default()
    }
    .validate()
    .is_err());
}

#[test]
fn a_civilian_table_parses_from_its_authored_shape() {
    let parsed: CivilianConfig = toml::from_str(
            r#"
route = "depot_run"
route_priority = 80.0
order_options = [
  { id = "storm_shelter", label = "world.test.storm_shelter", order = { verb = "divert", route = "storm_shelter_run" } },
]

[compliance]
divert = "refuse"
"#,
        )
        .expect("the vocabulary parses");
    assert_eq!(parsed.route.as_deref(), Some("depot_run"));
    assert_eq!(parsed.route_priority, 80.0);
    assert_eq!(parsed.order_options.len(), 1);
    assert_eq!(parsed.order_options[0].id, "storm_shelter");
    assert_eq!(
        parsed.order_options[0].order,
        CivilianOrder::divert_to_route("storm_shelter_run")
    );
    assert_eq!(
        parsed.compliance.expect("authored").divert,
        OrderResponse::Refuse
    );
    assert!(
        toml::from_str::<CivilianConfig>("rout = \"depot_run\"").is_err(),
        "a misspelled key is a load error, not a civilian with no route"
    );
}

// ── Housekeeping: the state survives a round trip and a cancellation ──

#[test]
fn the_live_state_round_trips_through_serde_for_the_snapshot_path() {
    let mut state = CivilianState::from_config(&CivilianConfig {
        route: Some("depot_run".into()),
        ..CivilianConfig::default()
    });
    let d = cooperative();
    ordered(&mut state, CivilianOrder::dock_at("skyhook_depot"), &d);
    run(&mut state, 400, true, &d);
    state.observe_leg(1, Some(&depot_run()), 400, HZ);

    let bytes = toml::to_string(&state).expect("serialises…");
    let back: CivilianState = toml::from_str(&bytes).expect("…and comes back");
    assert_eq!(back, state, "every field a resume needs must survive");
}

#[test]
fn clearing_an_order_returns_the_civilian_to_its_own_route() {
    let mut state = CivilianState::from_config(&CivilianConfig {
        route: Some("depot_run".into()),
        ..CivilianConfig::default()
    });
    let d = cooperative();
    ordered(&mut state, CivilianOrder::Hold, &d);
    run(&mut state, 400, true, &d);
    assert_eq!(state.travel(), CivilianTravel::Hold);
    let t = state.clear_order().expect("there was an order to clear");
    assert_eq!(t.to, ComplianceState::Unordered);
    assert_eq!(
        state.travel(),
        CivilianTravel::Route {
            id: "depot_run".into()
        }
    );
    assert!(
        state.clear_order().is_none(),
        "clearing nothing is not a transition"
    );
}

#[test]
fn a_new_order_replaces_one_still_in_flight() {
    let mut state = CivilianState::default();
    let d = cooperative();
    ordered(&mut state, CivilianOrder::Hold, &d);
    state.advance(120, true, &d, HZ);
    assert_eq!(state.compliance(), ComplianceState::Acknowledged);
    state.receive_order(CivilianOrder::dock_at("skyhook"), &d, 130, HZ);
    assert_eq!(
        state.compliance(),
        ComplianceState::Received,
        "the latest instruction is the operative one"
    );
    assert_eq!(state.order(), Some(&CivilianOrder::dock_at("skyhook")));
}

#[test]
fn a_civilian_with_no_route_and_no_order_holds_station() {
    assert_eq!(CivilianState::default().travel(), CivilianTravel::Hold);
}
