use super::*;

#[test]
fn selected_resource_choices_preserve_default_and_refuse_ambiguous_definitions() {
    let mut config = config(UmbilicalDirection::Collect, 2.0);
    config.allowed_resources = vec!["coolant".into()];
    assert!(config.validate().is_ok());
    assert_eq!(config.resource_choices(), vec!["reserve_fuel", "coolant"]);
    assert!(config.permits_resource("reserve_fuel"));
    assert!(config.permits_resource("coolant"));
    assert!(!config.permits_resource("antimatter"));
    config.allowed_resources.push("coolant".into());
    assert!(config.validate().is_err());
    config.allowed_resources = vec!["reserve_fuel".into()];
    assert!(config.validate().is_err());
}

fn config(direction: UmbilicalDirection, rate: f32) -> UmbilicalConfig {
    UmbilicalConfig {
        allowed_resources: vec![],
        link: Default::default(),
        capacity: "reserve_fuel".into(),
        rate,
        direction,
        min_power_level: 2,
    }
}

fn ends(op: Option<(i64, i64)>, pa: Option<(i64, i64)>) -> FlowEnds {
    FlowEnds {
        operator: op.map(|(level, headroom)| CapacityEnd { level, headroom }),
        partner: pa.map(|(level, headroom)| CapacityEnd { level, headroom }),
    }
}

fn ctx(carry: f32) -> FlowContext {
    FlowContext {
        docked: true,
        powered: true,
        disabled: false,
        dt: 1.0,
        carry,
    }
}

// ── The config validator ─────────────────────────────────────────────────

#[test]
fn a_valid_config_passes_and_the_unrunnable_ones_are_named() {
    assert!(config(UmbilicalDirection::Deliver, 5.0).validate().is_ok());
    let blank = UmbilicalConfig {
        link: Default::default(),
        capacity: "  ".into(),
        ..config(UmbilicalDirection::Deliver, 5.0)
    };
    assert!(blank.validate().unwrap_err().contains("capacity"));
    assert!(config(UmbilicalDirection::Deliver, 0.0)
        .validate()
        .unwrap_err()
        .contains("rate"));
    let unpowered = UmbilicalConfig {
        min_power_level: 0,
        ..config(UmbilicalDirection::Deliver, 5.0)
    };
    assert!(unpowered
        .validate()
        .unwrap_err()
        .contains("min_power_level"));
}

// ── The gates ────────────────────────────────────────────────────────────

#[test]
fn each_gate_refuses_by_name_most_actionable_first() {
    let full = ends(Some((100, 0)), Some((0, 100)));
    // Disabled beats everything.
    assert_eq!(
        plan_flow(
            &config(UmbilicalDirection::Deliver, 5.0),
            &full,
            &FlowContext {
                disabled: true,
                powered: false,
                docked: false,
                ..ctx(0.0)
            }
        ),
        FlowVerdict::Refused(UmbilicalRefusal::Disabled)
    );
    // Then power.
    assert_eq!(
        plan_flow(
            &config(UmbilicalDirection::Deliver, 5.0),
            &full,
            &FlowContext {
                powered: false,
                docked: false,
                ..ctx(0.0)
            }
        ),
        FlowVerdict::Refused(UmbilicalRefusal::Unpowered)
    );
    // Then the dock.
    assert_eq!(
        plan_flow(
            &config(UmbilicalDirection::Deliver, 5.0),
            &full,
            &FlowContext {
                docked: false,
                ..ctx(0.0)
            }
        ),
        FlowVerdict::Refused(UmbilicalRefusal::Undocked)
    );
    // Then the capacity: a partner that carries none.
    assert_eq!(
        plan_flow(
            &config(UmbilicalDirection::Deliver, 5.0),
            &ends(Some((100, 0)), None),
            &ctx(0.0)
        ),
        FlowVerdict::Refused(UmbilicalRefusal::NoCapacity)
    );
}

// ── The arithmetic, both directions ──────────────────────────────────────

#[test]
fn deliver_moves_operator_to_partner_and_collect_the_other_way() {
    // Deliver: operator (source) has plenty, partner (dest) has room.
    let deliver = plan_flow(
        &config(UmbilicalDirection::Deliver, 5.0),
        &ends(Some((100, 0)), Some((0, 100))),
        &ctx(0.0),
    );
    assert_eq!(
        deliver,
        FlowVerdict::Flowing {
            operator_delta: -5,
            partner_delta: 5,
            carry: 0.0
        }
    );
    // Collect: partner (source) has plenty, operator (dest) has room.
    let collect = plan_flow(
        &config(UmbilicalDirection::Collect, 5.0),
        &ends(Some((0, 100)), Some((100, 0))),
        &ctx(0.0),
    );
    assert_eq!(
        collect,
        FlowVerdict::Flowing {
            operator_delta: 5,
            partner_delta: -5,
            carry: 0.0
        }
    );
}

#[test]
fn the_deltas_always_sum_to_zero() {
    for direction in [UmbilicalDirection::Deliver, UmbilicalDirection::Collect] {
        if let FlowVerdict::Flowing {
            operator_delta,
            partner_delta,
            ..
        } = plan_flow(
            &config(direction, 7.0),
            &ends(Some((100, 100)), Some((100, 100))),
            &ctx(0.0),
        ) {
            assert_eq!(
                operator_delta + partner_delta,
                0,
                "nothing is created or lost"
            );
        } else {
            panic!("expected a flow");
        }
    }
}

// ── Clamping in both directions, no over/undershoot ──────────────────────

#[test]
fn deliver_clamps_at_source_depletion() {
    // The operator (source) has only 3 left though the rate would move 5.
    let v = plan_flow(
        &config(UmbilicalDirection::Deliver, 5.0),
        &ends(Some((3, 0)), Some((0, 100))),
        &ctx(0.0),
    );
    assert_eq!(
        v,
        FlowVerdict::Flowing {
            operator_delta: -3,
            partner_delta: 3,
            carry: 0.0
        },
        "it moves only what the source holds — no undershoot below what's there, no overshoot past empty"
    );
}

#[test]
fn deliver_clamps_at_destination_headroom() {
    // The partner (dest) has room for only 2 though the rate would move 5.
    let v = plan_flow(
        &config(UmbilicalDirection::Deliver, 5.0),
        &ends(Some((100, 0)), Some((10, 2))),
        &ctx(0.0),
    );
    assert_eq!(
        v,
        FlowVerdict::Flowing {
            operator_delta: -2,
            partner_delta: 2,
            carry: 0.0
        },
        "it fills only to the ceiling — no overshoot past headroom"
    );
}

#[test]
fn collect_clamps_at_source_depletion_and_headroom_too() {
    // Collect source is the PARTNER. It has 4; the operator dest has room 2.
    let v = plan_flow(
        &config(UmbilicalDirection::Collect, 9.0),
        &ends(Some((50, 2)), Some((4, 0))),
        &ctx(0.0),
    );
    assert_eq!(
        v,
        FlowVerdict::Flowing {
            operator_delta: 2,
            partner_delta: -2,
            carry: 0.0
        },
        "the tighter of the partner's 4 and the operator's headroom 2 wins — clamps both ways when collecting"
    );
}

#[test]
fn a_depleted_source_moves_nothing_but_keeps_running() {
    let v = plan_flow(
        &config(UmbilicalDirection::Deliver, 5.0),
        &ends(Some((0, 0)), Some((0, 100))),
        &ctx(0.0),
    );
    assert_eq!(
        v,
        FlowVerdict::Flowing {
            operator_delta: 0,
            partner_delta: 0,
            carry: 0.0
        },
        "an empty source is not a refusal — the flow keeps running and simply moves nothing"
    );
}

// ── The carry meters a sub-unit rate ─────────────────────────────────────

#[test]
fn a_sub_unit_rate_accumulates_through_the_carry() {
    // 0.5 units/sec at dt=1: tick one produces 0.5 (moves 0, carries 0.5),
    // tick two produces 1.0 (moves 1, carries 0.0).
    let cfg = config(UmbilicalDirection::Deliver, 0.5);
    let e = ends(Some((100, 0)), Some((0, 100)));
    let first = plan_flow(&cfg, &e, &ctx(0.0));
    assert_eq!(
        first,
        FlowVerdict::Flowing {
            operator_delta: 0,
            partner_delta: 0,
            carry: 0.5
        }
    );
    let FlowVerdict::Flowing { carry, .. } = first else {
        panic!("expected a flow");
    };
    let second = plan_flow(&cfg, &e, &ctx(carry));
    assert_eq!(
        second,
        FlowVerdict::Flowing {
            operator_delta: -1,
            partner_delta: 1,
            carry: 0.0
        },
        "the carried half plus another half makes one whole unit move"
    );
}

#[test]
fn the_carry_never_hoards_a_whole_unit_the_clamp_discarded() {
    // Source has 1, rate would move 5: it moves 1 and discards the rest,
    // and the carry is only the sub-unit remainder (0 here) — no backlog to
    // dump when the source refills.
    let cfg = config(UmbilicalDirection::Deliver, 5.0);
    let v = plan_flow(&cfg, &ends(Some((1, 0)), Some((0, 100))), &ctx(0.0));
    assert_eq!(
        v,
        FlowVerdict::Flowing {
            operator_delta: -1,
            partner_delta: 1,
            carry: 0.0
        }
    );
}

#[test]
fn explicit_transfer_precedes_idle_replenishment_and_preserves_active_human_work() {
    for maintaining in [false, true] {
        for running in [false, true] {
            for eligible_free_port in [false, true] {
                let expected = maintaining.then_some(false);
                assert_eq!(
                    port_maintenance_change(PortMaintenanceFacts {
                        explicit_transfer: true,
                        running,
                        maintaining,
                        eligible_free_port,
                    }),
                    expected,
                    "explicit work must retain selected direction/resource"
                );
            }
        }
    }
    assert_eq!(
        port_maintenance_change(PortMaintenanceFacts {
            explicit_transfer: false,
            running: false,
            maintaining: false,
            eligible_free_port: true,
        }),
        Some(true),
        "idle delegated visits still replenish Main stores"
    );
    for maintaining in [false, true] {
        for eligible_free_port in [false, true] {
            assert_eq!(
                port_maintenance_change(PortMaintenanceFacts {
                    explicit_transfer: false,
                    running: true,
                    maintaining,
                    eligible_free_port,
                }),
                None,
                "delegation must preserve an already active human flow"
            );
        }
    }
    assert_eq!(
        port_maintenance_change(PortMaintenanceFacts {
            explicit_transfer: false,
            running: false,
            maintaining: false,
            eligible_free_port: false,
        }),
        None,
        "unknown or ineligible ports confer no new maintenance intent"
    );
    assert_eq!(
        port_maintenance_change(PortMaintenanceFacts {
            explicit_transfer: false,
            running: false,
            maintaining: true,
            eligible_free_port: true,
        }),
        None,
        "retained maintenance needs no repeated command"
    );
}
