use super::*;

const HZ: f32 = 60.0;

fn authored() -> Vec<Deadline> {
    vec![
        Deadline {
            id: "window".into(),
            label: "world.probe.deadline.window.label".into(),
            due_secs: 10,
            visible: true,
        },
        Deadline {
            id: "collapse".into(),
            label: "world.probe.deadline.collapse.label".into(),
            due_secs: 20,
            visible: false,
        },
    ]
}

fn handlers() -> Vec<DeadlineHandler> {
    vec![
        DeadlineHandler {
            deadline_id: "window".into(),
            handler: "on_window".into(),
            source_path: "w.toml#script.setup".into(),
        },
        DeadlineHandler {
            deadline_id: "collapse".into(),
            handler: "on_collapse".into(),
            source_path: "w.toml#script.setup".into(),
        },
    ]
}

fn armed_table() -> (DeadlineTable, Vec<ScheduledCall>) {
    let mut table = DeadlineTable::default();
    let queued = table.arm(&authored(), &handlers(), 0, HZ);
    (table, queued)
}

// ── AC1: a deadline is authored with id, label, due time and visibility ──

#[test]
fn arming_keys_every_authored_deadline_on_a_tick() {
    let (table, queued) = armed_table();
    assert!(
        table.armed,
        "the latch says the mission's deadlines are set"
    );
    assert_eq!(table.records.len(), 2, "one record per authored block");

    let window = table.get("window").expect("the id is the lookup key");
    assert_eq!(window.due_tick, 600, "10s at 60Hz is tick 600");
    assert!(window.visible, "the authored visibility flag travels");
    assert_eq!(window.label, "world.probe.deadline.window.label");
    assert_eq!(window.state, DeadlineState::Pending);

    assert_eq!(
        table.get("collapse").map(|r| r.due_tick),
        Some(1200),
        "20s at 60Hz is tick 1200"
    );
    assert_eq!(
        queued.len(),
        2,
        "arming produces one queued call per deadline, for the EXISTING queue"
    );
    assert_eq!(
        queued[0],
        ScheduledCall {
            fire_tick: 600,
            script_path: "w.toml#script.setup".into(),
            fn_name: "on_window".into(),
            origin_layer: None,
        },
        "the queued work is an ordinary ScheduledCall — no new record type"
    );
}

#[test]
fn a_deadline_with_no_registered_handler_is_not_armed() {
    // Load-time validation blocks this world, so reaching here is the
    // impossible case; dropping the record keeps an unfireable countdown off
    // the panel rather than ticking to zero forever.
    let mut table = DeadlineTable::default();
    let queued = table.arm(&authored(), &handlers()[..1], 0, HZ);
    assert_eq!(queued.len(), 1);
    assert!(table.get("window").is_some());
    assert!(
        table.get("collapse").is_none(),
        "an unhandled deadline is not armed at all"
    );
}

#[test]
fn root_and_layers_may_reuse_a_local_id_without_cross_talk() {
    let deadline = Deadline {
        id: "window".into(),
        label: "deadline.window".into(),
        due_secs: 10,
        visible: true,
    };
    let handler = DeadlineHandler {
        deadline_id: "window".into(),
        handler: "on_window".into(),
        source_path: "shared.rhai".into(),
    };
    let mut table = DeadlineTable::default();
    let root = table.arm(
        std::slice::from_ref(&deadline),
        std::slice::from_ref(&handler),
        0,
        HZ,
    );
    let layer_a = table.arm_scoped(
        std::slice::from_ref(&deadline),
        std::slice::from_ref(&handler),
        30,
        HZ,
        Some("worlds/a.toml"),
    );
    let layer_b = table.arm_scoped(
        std::slice::from_ref(&deadline),
        std::slice::from_ref(&handler),
        60,
        HZ,
        Some("worlds/b.toml"),
    );

    assert_eq!(root[0].origin_layer, None);
    assert_eq!(layer_a[0].origin_layer.as_deref(), Some("worlds/a.toml"));
    assert_eq!(layer_b[0].origin_layer.as_deref(), Some("worlds/b.toml"));
    assert_eq!(table.records.len(), 3);
    assert_eq!(table.records[0].presentation_id(), "window");
    assert_eq!(
        table.records[1].presentation_id(),
        "worlds/a.toml#deadline.window"
    );
    assert_eq!(
        table.records[2].presentation_id(),
        "worlds/b.toml#deadline.window"
    );
    assert_eq!(table.get("window").unwrap().due_tick, 600);
    assert_eq!(
        table
            .get_scoped(Some("worlds/a.toml"), "window")
            .unwrap()
            .due_tick,
        630,
        "layer due_secs is relative to the tick its activation landed"
    );

    table.apply(
        &DeadlineChange {
            id: "window".into(),
            origin_layer: Some("worlds/a.toml".into()),
            mutation: DeadlineMutation::Cancel,
        },
        30,
        HZ,
    );
    assert_eq!(table.get("window").unwrap().state, DeadlineState::Pending);
    assert_eq!(
        table
            .get_scoped(Some("worlds/a.toml"), "window")
            .unwrap()
            .state,
        DeadlineState::Cancelled
    );
    assert_eq!(
        table
            .get_scoped(Some("worlds/b.toml"), "window")
            .unwrap()
            .state,
        DeadlineState::Pending
    );
}

// ── AC4: deadlines are inspectable — remaining time and state ────────────

#[test]
fn remaining_time_counts_down_and_rounds_up() {
    let (table, _) = armed_table();
    assert_eq!(table.remaining_secs("window", 0, HZ), 10);
    assert_eq!(table.remaining_secs("window", 300, HZ), 5);
    assert_eq!(
        table.remaining_secs("window", 599, HZ),
        1,
        "the final second reads 1 until the deadline is genuinely due"
    );
    assert_eq!(table.remaining_secs("window", 600, HZ), 0);
    assert_eq!(
        table.remaining_secs("window", 900, HZ),
        0,
        "a past due tick saturates at zero rather than going negative"
    );
}

#[test]
fn state_and_remaining_tell_cancelled_apart_from_unknown_and_fired() {
    let (mut table, _) = armed_table();
    assert_eq!(table.state_of("window"), "pending");
    assert_eq!(
        table.state_of("no_such_deadline"),
        "unknown",
        "a typo is named as such rather than silently reading as cancelled"
    );
    assert_eq!(table.remaining_secs("no_such_deadline", 0, HZ), NO_DEADLINE);

    table.apply(
        &DeadlineChange {
            id: "collapse".into(),
            origin_layer: None,
            mutation: DeadlineMutation::Cancel,
        },
        0,
        HZ,
    );
    assert_eq!(table.state_of("collapse"), "cancelled");
    assert_eq!(table.remaining_secs("collapse", 0, HZ), NO_DEADLINE);

    let due = vec![table.get("window").unwrap().armed.clone().unwrap()];
    table.note_fired(&due);
    assert_eq!(table.state_of("window"), "fired");
    assert_eq!(
        table.remaining_secs("window", 600, HZ),
        0,
        "a fired deadline has no time left — which is not the same as having no deadline"
    );
}

// ── AC3: slip and cancel take effect on the EXISTING queue ───────────────

#[test]
fn a_slip_retracts_the_old_queued_call_and_pushes_the_new_one() {
    let (mut table, queued) = armed_table();
    let old = queued[0].clone();

    let edit = table
        .apply(
            &DeadlineChange {
                id: "window".into(),
                origin_layer: None,
                mutation: DeadlineMutation::Slip { by_secs: 5 },
            },
            120,
            HZ,
        )
        .expect("a pending deadline slips");

    assert_eq!(
        edit.retract,
        Some(old),
        "the OLD queued call is named for retraction — so it cannot also fire"
    );
    assert_eq!(
        edit.push,
        Some(ScheduledCall {
            fire_tick: 900,
            script_path: "w.toml#script.setup".into(),
            fn_name: "on_window".into(),
            origin_layer: None,
        }),
        "and the replacement is the same unit and fn at the new tick"
    );
    assert_eq!(table.get("window").unwrap().due_tick, 900);
    assert_eq!(
        table.remaining_secs("window", 120, HZ),
        13,
        "the slip is measured from the deadline's own due tick, not from now"
    );
}

#[test]
fn slips_accumulate_and_a_negative_slip_pulls_the_deadline_in() {
    let (mut table, _) = armed_table();
    for _ in 0..3 {
        table.apply(
            &DeadlineChange {
                id: "window".into(),
                origin_layer: None,
                mutation: DeadlineMutation::Slip { by_secs: 5 },
            },
            0,
            HZ,
        );
    }
    assert_eq!(
        table.get("window").unwrap().due_tick,
        600 + 3 * 300,
        "three five-second slips add up"
    );

    table.apply(
        &DeadlineChange {
            id: "window".into(),
            origin_layer: None,
            mutation: DeadlineMutation::Slip { by_secs: -20 },
        },
        0,
        HZ,
    );
    assert_eq!(
        table.get("window").unwrap().due_tick,
        300,
        "and one pulls in"
    );

    // A slip further back than the present clamps to now rather than
    // producing a fire tick in the past.
    table.apply(
        &DeadlineChange {
            id: "window".into(),
            origin_layer: None,
            mutation: DeadlineMutation::Slip { by_secs: -600 },
        },
        250,
        HZ,
    );
    assert_eq!(table.get("window").unwrap().due_tick, 250);
}

#[test]
fn a_cancel_retracts_its_call_and_queues_nothing() {
    let (mut table, queued) = armed_table();
    let edit = table
        .apply(
            &DeadlineChange {
                id: "collapse".into(),
                origin_layer: None,
                mutation: DeadlineMutation::Cancel,
            },
            0,
            HZ,
        )
        .expect("a pending deadline cancels");
    assert_eq!(edit.retract, Some(queued[1].clone()));
    assert_eq!(
        edit.push, None,
        "a cancelled deadline queues no replacement"
    );
    assert_eq!(
        table.get("collapse").unwrap().state,
        DeadlineState::Cancelled
    );
    assert!(
        table.get("collapse").unwrap().armed.is_none(),
        "and holds no queued call to be re-armed by a later restore"
    );
}

#[test]
fn spending_a_deadline_twice_is_a_no_op_rather_than_a_second_edit() {
    let (mut table, _) = armed_table();
    table.apply(
        &DeadlineChange {
            id: "window".into(),
            origin_layer: None,
            mutation: DeadlineMutation::Cancel,
        },
        0,
        HZ,
    );
    assert_eq!(
        table.apply(
            &DeadlineChange {
                id: "window".into(),
                origin_layer: None,
                mutation: DeadlineMutation::Slip { by_secs: 60 },
            },
            0,
            HZ,
        ),
        None,
        "a cancelled deadline cannot be slipped back into existence"
    );
    assert_eq!(
        table.apply(
            &DeadlineChange {
                id: "nope".into(),
                origin_layer: None,
                mutation: DeadlineMutation::Cancel,
            },
            0,
            HZ,
        ),
        None,
        "and an unknown id edits nothing"
    );
}

#[test]
fn a_slipped_deadline_no_longer_matches_its_old_firing() {
    // The whole point of re-keying: the queue drains the OLD call only if
    // the adapter failed to retract it, and even then the record refuses to
    // record a fire it no longer owns.
    let (mut table, queued) = armed_table();
    let stale = queued[0].clone();
    table.apply(
        &DeadlineChange {
            id: "window".into(),
            origin_layer: None,
            mutation: DeadlineMutation::Slip { by_secs: 5 },
        },
        0,
        HZ,
    );
    table.note_fired(&[stale]);
    assert_eq!(
        table.get("window").unwrap().state,
        DeadlineState::Pending,
        "a slipped deadline does not fire at its old time"
    );
}

#[test]
fn note_fired_flips_only_the_deadline_whose_call_actually_drained() {
    let (mut table, queued) = armed_table();
    table.note_fired(&queued[..1]);
    assert_eq!(table.get("window").unwrap().state, DeadlineState::Fired);
    assert!(table.get("window").unwrap().armed.is_none());
    assert_eq!(
        table.get("collapse").unwrap().state,
        DeadlineState::Pending,
        "the other deadline's call did not drain, so it is untouched"
    );
}

#[test]
fn two_deadlines_sharing_a_call_key_fire_one_at_a_time() {
    // Equal `(fire_tick, script_path, fn_name)` keys are legal — two
    // deadlines may share a handler and a tick. Retracting/firing "the first
    // equal one" then moves exactly one record, which is the correct count.
    let authored = vec![
        Deadline {
            id: "a".into(),
            label: String::new(),
            due_secs: 10,
            visible: false,
        },
        Deadline {
            id: "b".into(),
            label: String::new(),
            due_secs: 10,
            visible: false,
        },
    ];
    let handlers = vec![
        DeadlineHandler {
            deadline_id: "a".into(),
            handler: "shared".into(),
            source_path: "w.toml#script.setup".into(),
        },
        DeadlineHandler {
            deadline_id: "b".into(),
            handler: "shared".into(),
            source_path: "w.toml#script.setup".into(),
        },
    ];
    let mut table = DeadlineTable::default();
    let queued = table.arm(&authored, &handlers, 0, HZ);
    assert_eq!(queued[0], queued[1], "the two keys are genuinely equal");

    table.note_fired(&queued[..1]);
    assert_eq!(table.get("a").unwrap().state, DeadlineState::Fired);
    assert_eq!(
        table.get("b").unwrap().state,
        DeadlineState::Pending,
        "one drained call fires one deadline"
    );
}

// ── AC10: the state is serialisable so #863/#864 can persist it ──────────

#[test]
fn the_table_round_trips_through_serialization() {
    let (mut table, queued) = armed_table();
    table.apply(
        &DeadlineChange {
            id: "window".into(),
            origin_layer: None,
            mutation: DeadlineMutation::Slip { by_secs: 30 },
        },
        60,
        HZ,
    );
    table.note_fired(&queued[1..]);

    let json = serde_json::to_string(&table).expect("serialises");
    let restored: DeadlineTable = serde_json::from_str(&json).expect("deserialises");
    assert_eq!(
        restored, table,
        "every field a run moves — due tick, state, and the queued call — round-trips"
    );
}

#[test]
fn a_zero_rate_reports_no_time_rather_than_dividing_by_it() {
    let (table, _) = armed_table();
    assert_eq!(table.remaining_secs("window", 0, 0.0), 0);
}
