use crate::logging::{EntityFilter, LevelFilter, LogCat, LogFilterConfig};
use bevy::prelude::*;

/// The macros must accept a bare `LogFilterConfig`, a reference, and a
/// `Res<_>` without the call site caring which it has.
#[test]
fn macros_expand_over_value_reference_and_res() {
    let cfg = LogFilterConfig {
        default_level: LevelFilter::Trace,
        ..Default::default()
    };
    let e = Entity::from_raw_u32(1).unwrap();

    pwarn!(cfg, LogCat::World, "by value {}", 1);
    let by_ref = &cfg;
    pinfo!(by_ref, LogCat::Ai, "by reference");
    pdebug!(cfg, LogCat::Admit, entity = e, "entity-scoped {}", 2);
    ptrace!(cfg, LogCat::Physics, "trace");
    perror!(cfg, LogCat::Config, entity = e, "error");
}

// Note on what is *not* tested here: whether a suppressed call site skips
// evaluating its format arguments. Under `cargo test` no `tracing`
// subscriber is installed, so `tracing` short-circuits every event on its
// own — a side-effect probe in the format args never runs regardless of
// what this module's gate decides, and such a test would pass vacuously.
// The gate itself is covered directly by the `cat_enabled` /
// `entity_allowed` tests in `filter.rs`; what remains for the macros is
// that they expand correctly and route to the right predicates, below.

/// The trap this design exists to avoid: a system that logs must still run
/// in a bare `App` that never inserted `LogFilterConfig`. Before
/// `AsLogFilter`, adding one `plog!` to a shared system broke 430 existing
/// tests at parameter validation.
#[test]
fn a_logging_system_runs_without_the_resource_inserted() {
    fn logs_things(log: Option<Res<LogFilterConfig>>) {
        pwarn!(log, LogCat::World, "still runs with no resource");
    }
    let mut app = App::new();
    app.add_systems(Update, logs_things);
    app.update(); // Would panic on `Res<LogFilterConfig>`.
}

/// And it must pick up a real config when one *is* present.
#[test]
fn the_resource_is_used_when_present() {
    fn assert_trace_enabled(log: Option<Res<LogFilterConfig>>) {
        let cfg = crate::logging::AsLogFilter::log_filter(&log);
        assert_eq!(cfg.default_level, LevelFilter::Trace);
    }
    let mut app = App::new();
    app.insert_resource(LogFilterConfig {
        default_level: LevelFilter::Trace,
        ..Default::default()
    });
    app.add_systems(Update, assert_trace_enabled);
    app.update();
}

/// Both forms must consult the predicates the macro claims to. Mirrors the
/// exact expressions `__plog_gated!` expands to.
#[test]
fn gate_predicates_match_the_documented_semantics() {
    let cfg = LogFilterConfig {
        default_level: LevelFilter::Trace,
        entity_filter: Some(EntityFilter::new(vec!["Ironveil".into()])),
        ..Default::default()
    };
    let unresolved = Entity::from_raw_u32(9).unwrap();

    // Entity form: category passes, entity does not, so the event is out.
    assert!(cfg.cat_enabled(LogCat::Ai, LevelFilter::Debug));
    assert!(!cfg.entity_allowed(unresolved));

    // Global form checks only the category, so an entity filter must not
    // swallow entity-less events.
    assert!(cfg.cat_enabled(LogCat::Ai, LevelFilter::Debug));
}
