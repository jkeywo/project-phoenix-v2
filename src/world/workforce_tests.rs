use super::*;

fn authored() -> Vec<Workforce> {
    vec![
        Workforce {
            id: "skyway_workers".into(),
            label: "world.probe.workforce.workers.label".into(),
            on_strike: true,
            disposition: 30,
        },
        Workforce {
            id: "havelock_operations".into(),
            label: "world.probe.workforce.operator.label".into(),
            on_strike: false,
            disposition: 45,
        },
    ]
}

fn armed() -> WorkforceRegister {
    let mut register = WorkforceRegister::default();
    register.arm(&authored());
    register
}

// ── AC1: two sides, an explicit status and a per-side disposition ────────

#[test]
fn arming_takes_both_sides_status_and_disposition_from_the_world() {
    let register = armed();
    assert_eq!(
        register
            .records
            .iter()
            .map(|r| r.id.as_str())
            .collect::<Vec<_>>(),
        vec!["skyway_workers", "havelock_operations"],
        "in authored order — the world file's order is the one a debrief reads"
    );
    assert!(register.on_strike("skyway_workers"));
    assert!(
        !register.on_strike("havelock_operations"),
        "the operator is not the one who walked out; a slice that could not tell \
             the two sides apart could not have a dispute"
    );
    assert_eq!(register.disposition("skyway_workers"), Some(30));
    assert_eq!(register.disposition("havelock_operations"), Some(45));
}

#[test]
fn a_side_this_world_never_declared_is_working_and_has_no_opinion() {
    let register = armed();
    assert!(
        !register.on_strike("dockers_of_some_other_mission"),
        "a depot template naming a workforce the world has no dispute about must \
             keep working — the template ships in every scenario, the dispute does not"
    );
    assert_eq!(register.disposition("dockers_of_some_other_mission"), None);
}

#[test]
fn arming_writes_both_mirror_flags_for_every_side() {
    let mut register = WorkforceRegister::default();
    let writes = register.arm(&authored());
    assert_eq!(
        writes,
        vec![
            FlagMirror {
                name: "workforce.skyway_workers.on_strike".into(),
                value: 1
            },
            FlagMirror {
                name: "workforce.skyway_workers.disposition".into(),
                value: 30
            },
            FlagMirror {
                name: "workforce.havelock_operations.on_strike".into(),
                value: 0
            },
            FlagMirror {
                name: "workforce.havelock_operations.disposition".into(),
                value: 45
            },
        ],
        "the mirror is written by the caller, from writes this module names, so \
             the flag a script reads cannot drift from the record"
    );
}

#[test]
fn arming_twice_changes_nothing_and_asks_for_no_writes() {
    let mut register = armed();
    register.apply("skyway_workers", WorkforceMutation::Settle);
    assert!(!register.on_strike("skyway_workers"));

    assert!(
        register.arm(&authored()).is_empty(),
        "a resumed mission's first tick must not put a settled strike back on"
    );
    assert!(!register.on_strike("skyway_workers"));
}

#[test]
fn a_world_that_declares_no_side_arms_to_nothing() {
    let mut register = WorkforceRegister::default();
    assert!(register.arm(&[]).is_empty());
    assert!(register.records.is_empty(), "no side was declared");
    assert!(
        register.armed,
        "and it counts as armed, so the system that arms it stops looking"
    );
    // Deliberately NOT `register.is_empty()`, which is the payload's skip
    // predicate and accounts for the latch (see `Self::is_empty`): an armed
    // register is one a save has to carry even with no side in it, or the
    // resumed mission re-arms from the world file. `arm_mission_workforces`
    // never reaches this state — it early-returns on an empty authored table
    // without latching — so nothing shipped writes such a register, but the
    // pure module can and the payload now answers for it.
    assert!(!register.is_empty());
}

// ── AC4: reversible, both ways, with the mirror following ────────────────

#[test]
fn settling_a_strike_clears_the_status_and_its_flag() {
    let mut register = armed();
    assert_eq!(
        register.apply("skyway_workers", WorkforceMutation::Settle),
        Some(FlagMirror {
            name: "workforce.skyway_workers.on_strike".into(),
            value: 0
        })
    );
    assert!(!register.on_strike("skyway_workers"));
}

#[test]
fn a_settled_side_can_walk_out_again() {
    let mut register = armed();
    register.apply("skyway_workers", WorkforceMutation::Settle);
    assert_eq!(
        register.apply("skyway_workers", WorkforceMutation::CallStrike),
        Some(FlagMirror {
            name: "workforce.skyway_workers.on_strike".into(),
            value: 1
        }),
        "nothing about a stoppage is latched, in either direction"
    );
    assert!(register.on_strike("skyway_workers"));
}

#[test]
fn a_mutation_that_changes_nothing_writes_nothing() {
    let mut register = armed();
    assert_eq!(
        register.apply("skyway_workers", WorkforceMutation::CallStrike),
        None,
        "they are already out"
    );
    assert_eq!(
        register.apply("havelock_operations", WorkforceMutation::Settle),
        None,
        "and they never left"
    );
    assert_eq!(
        register.apply("nobody", WorkforceMutation::Settle),
        None,
        "an unknown side settles nothing and writes no flag"
    );
}

#[test]
fn disposition_moves_absolutely_and_clamps_to_the_authored_scale() {
    let mut register = armed();
    assert_eq!(
        register.apply("skyway_workers", WorkforceMutation::SetDisposition(80)),
        Some(FlagMirror {
            name: "workforce.skyway_workers.disposition".into(),
            value: 80
        })
    );
    assert_eq!(register.disposition("skyway_workers"), Some(80));

    register.apply("skyway_workers", WorkforceMutation::SetDisposition(9_000));
    assert_eq!(
        register.disposition("skyway_workers"),
        Some(DISPOSITION_MAX),
        "a runaway negotiation cannot push a side off its own scale"
    );
    register.apply("skyway_workers", WorkforceMutation::SetDisposition(-40));
    assert_eq!(
        register.disposition("skyway_workers"),
        Some(DISPOSITION_MIN)
    );
}

#[test]
fn the_two_facts_move_independently() {
    // A side can go back to work still resenting the crew, and can walk out
    // while thinking well of them. Folding status and disposition into one
    // number would make both of those unsayable.
    let mut register = armed();
    register.apply("skyway_workers", WorkforceMutation::Settle);
    assert!(!register.on_strike("skyway_workers"));
    assert_eq!(register.disposition("skyway_workers"), Some(30));

    register.apply("havelock_operations", WorkforceMutation::SetDisposition(10));
    assert!(!register.on_strike("havelock_operations"));
    assert_eq!(register.disposition("havelock_operations"), Some(10));
}

// ── Authoring guards ─────────────────────────────────────────────────────

#[test]
fn an_empty_id_is_refused_at_load() {
    let workforce = Workforce {
        id: "  ".into(),
        ..Default::default()
    };
    assert!(workforce.validate().unwrap_err().contains("non-empty id"));
}

#[test]
fn a_disposition_off_the_scale_is_refused_at_load() {
    for value in [-1, 101] {
        let workforce = Workforce {
            id: "skyway_workers".into(),
            disposition: value,
            ..Default::default()
        };
        let err = workforce.validate().expect_err("off the 0..=100 scale");
        assert!(err.contains("disposition"), "{err}");
        assert!(err.contains("skyway_workers"), "the error names it: {err}");
    }
    assert!(Workforce {
        id: "skyway_workers".into(),
        disposition: DISPOSITION_MAX,
        ..Default::default()
    }
    .validate()
    .is_ok());
}

#[test]
fn an_unauthored_disposition_defaults_to_the_midpoint() {
    let parsed: Workforce = toml::from_str(r#"id = "skyway_workers""#).expect("parses");
    assert_eq!(parsed.disposition, default_disposition());
    assert!(
        !parsed.on_strike,
        "a side nobody said was out is at work — the block exists to declare a \
             dispute, not to imply one"
    );
    assert!(parsed.label.is_empty());
}

#[test]
fn the_register_round_trips_through_serialization() {
    let mut register = armed();
    register.apply("skyway_workers", WorkforceMutation::Settle);
    register.apply("havelock_operations", WorkforceMutation::SetDisposition(12));

    let json = serde_json::to_string(&register).expect("serialises");
    let restored: WorkforceRegister = serde_json::from_str(&json).expect("deserialises");
    assert_eq!(
        restored, register,
        "every field a run moves — both statuses, both dispositions and the armed \
             latch — comes back"
    );
}
