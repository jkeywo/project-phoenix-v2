use super::*;
use crate::world::deadlines::{Deadline, DeadlineState};
use crate::world::script::engine::runtime_engine;
use rhai::{Dynamic, Map};

const HZ: f32 = 60.0;

fn table() -> DeadlineTable {
    let mut table = DeadlineTable::default();
    table.arm(
        &[
            Deadline {
                id: "window".into(),
                label: "l.window".into(),
                due_secs: 100,
                visible: true,
            },
            Deadline {
                id: "collapse".into(),
                label: "l.collapse".into(),
                due_secs: 200,
                visible: false,
            },
        ],
        &[
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
        ],
        0,
        HZ,
    );
    table
}

/// Run `source`'s `on_x` against a live table, returning what it printed
/// into a flag-free out-param plus the mutations it buffered.
fn run(source: &str, now_tick: u64) -> (Vec<DeadlineChange>, Dynamic) {
    let engine = runtime_engine();
    let ast = engine.compile(source).expect("compiles");
    let deadlines = Deadlines::new(&table(), now_tick, HZ);
    let mut ctx = Map::new();
    ctx.insert("deadlines".into(), Dynamic::from(deadlines.clone()));
    let value = vellum_script::call_fn(&engine, &ast, "t.rhai", "on_x", ctx).expect("runs");
    (deadlines.take_changes(), value)
}

// ── AC4: remaining time and state are readable from script ──────────────

#[test]
fn a_handler_reads_remaining_time_and_state() {
    let (_, value) = run(r#"fn on_x(ctx) { ctx.deadlines.remaining("window") }"#, 0);
    assert_eq!(value.as_int().expect("an INT"), 100);

    let (_, value) = run(
        r#"fn on_x(ctx) { ctx.deadlines.remaining("window") }"#,
        3000,
    );
    assert_eq!(value.as_int().expect("an INT"), 50, "50s in, 50s left");

    let (_, value) = run(r#"fn on_x(ctx) { ctx.deadlines.state("window") }"#, 0);
    assert_eq!(value.into_string().expect("a string"), "pending");

    let (_, value) = run(r#"fn on_x(ctx) { ctx.deadlines.state("nope") }"#, 0);
    assert_eq!(
        value.into_string().expect("a string"),
        "unknown",
        "a typo names itself rather than reading as a cancelled deadline"
    );
}

// ── AC3: slip and cancel are callable from script ────────────────────────

#[test]
fn slip_and_cancel_buffer_mutations_in_authored_order() {
    let (changes, _) = run(
        r#"fn on_x(ctx) {
                 ctx.deadlines.slip("window", 60);
                 ctx.deadlines.cancel("collapse");
               }"#,
        0,
    );
    assert_eq!(
        changes,
        vec![
            DeadlineChange {
                id: "window".into(),
                origin_layer: None,
                mutation: DeadlineMutation::Slip { by_secs: 60 },
            },
            DeadlineChange {
                id: "collapse".into(),
                origin_layer: None,
                mutation: DeadlineMutation::Cancel,
            },
        ]
    );
}

#[test]
fn a_read_after_a_slip_sees_the_new_time_within_the_same_call() {
    // The snapshot-overlay property `Flags` establishes, applied to a
    // handler that slips and then decides what else to do about it.
    let (_, value) = run(
        r#"fn on_x(ctx) {
                 ctx.deadlines.slip("window", 60);
                 ctx.deadlines.remaining("window")
               }"#,
        0,
    );
    assert_eq!(value.as_int().expect("an INT"), 160);

    let (_, value) = run(
        r#"fn on_x(ctx) {
                 ctx.deadlines.cancel("window");
                 ctx.deadlines.state("window")
               }"#,
        0,
    );
    assert_eq!(value.into_string().expect("a string"), "cancelled");
}

#[test]
fn the_live_table_is_untouched_by_a_call() {
    // The call mutates its own snapshot; only the adapter replaying the
    // drained changes moves the real table (and, with it, the queue).
    let live = table();
    let engine = runtime_engine();
    let ast = engine
        .compile(r#"fn on_x(ctx) { ctx.deadlines.cancel("window"); }"#)
        .expect("compiles");
    let deadlines = Deadlines::new(&live, 0, HZ);
    let mut ctx = Map::new();
    ctx.insert("deadlines".into(), Dynamic::from(deadlines.clone()));
    let _ = vellum_script::call_fn(&engine, &ast, "t.rhai", "on_x", ctx).expect("runs");
    assert_eq!(
        live.get("window").expect("still there").state,
        DeadlineState::Pending,
        "a script call never writes the live table directly"
    );
    assert_eq!(deadlines.take_changes().len(), 1);
}

#[test]
fn taking_the_changes_twice_yields_them_once() {
    let deadlines = Deadlines::new(&table(), 0, HZ);
    deadlines.push("window", DeadlineMutation::Cancel);
    assert_eq!(deadlines.take_changes().len(), 1);
    assert!(
        deadlines.take_changes().is_empty(),
        "a drained buffer cannot replay its mutations"
    );
}
