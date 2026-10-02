use super::*;

fn sid(s: &str) -> StationId {
    StationId(s.into())
}

fn obj(id: &str, station: &str, status: ObjectiveStatus) -> (String, StationId, ObjectiveStatus) {
    (id.into(), sid(station), status)
}

// ── AC2: a one-off unread event is cleared only by a visit ────────────────

#[test]
fn a_completed_objective_marks_its_station_unread() {
    let mut imp = StationImportance::default();
    imp.ingest(
        vec![obj("rescue", "comms", ObjectiveStatus::Completed)],
        vec![],
    );
    assert_eq!(
        imp.flags_of(&sid("comms")),
        ImportanceFlags {
            unread: true,
            critical: false
        }
    );
}

#[test]
fn an_active_objective_marks_nothing() {
    let mut imp = StationImportance::default();
    imp.ingest(
        vec![obj("rescue", "comms", ObjectiveStatus::Active)],
        vec![],
    );
    assert!(imp.snapshots().is_empty());
}

#[test]
fn visiting_clears_the_unread_flag_and_it_stays_cleared() {
    let mut imp = StationImportance::default();
    let done = || vec![obj("rescue", "comms", ObjectiveStatus::Completed)];
    imp.ingest(done(), vec![]);
    assert!(imp.flags_of(&sid("comms")).unread);

    imp.visit(&sid("comms"));
    assert!(!imp.flags_of(&sid("comms")).unread);

    // The objective is STILL Completed on subsequent ticks — re-ingesting it
    // must NOT re-raise unread (the clear is authoritative, not optimistic).
    imp.ingest(done(), vec![]);
    assert!(
        !imp.flags_of(&sid("comms")).unread,
        "a re-seen terminal objective must not resurrect a cleared unread flag"
    );
    assert!(imp.snapshots().is_empty());
}

#[test]
fn a_failed_objective_is_a_one_off_unread_event() {
    let mut imp = StationImportance::default();
    imp.ingest(vec![obj("hold", "helm", ObjectiveStatus::Failed)], vec![]);
    assert!(imp.flags_of(&sid("helm")).unread);
    assert!(!imp.flags_of(&sid("helm")).critical);
}

// ── AC3: a continuing critical condition survives a visit ─────────────────

#[test]
fn a_critical_condition_survives_a_visit_and_clears_only_on_resolve() {
    let mut imp = StationImportance::default();
    imp.ingest(Vec::new(), vec![sid("core")]);
    assert!(imp.flags_of(&sid("core")).critical);

    // Visiting the Station does NOT clear a continuing condition.
    imp.visit(&sid("core"));
    assert!(
        imp.flags_of(&sid("core")).critical,
        "a visit must not clear a continuing critical condition"
    );

    // It clears only when the condition itself resolves (no longer reported).
    imp.ingest(Vec::new(), Vec::new());
    assert!(!imp.flags_of(&sid("core")).critical);
    assert!(imp.snapshots().is_empty());
}

// ── AC1: the two lifecycles are independent, both apart from health ───────

#[test]
fn unread_and_critical_are_independent_on_the_same_station() {
    let mut imp = StationImportance::default();
    // A completed objective (unread) AND a critical condition on the same
    // Station at once.
    imp.ingest(
        vec![obj("rescue", "core", ObjectiveStatus::Completed)],
        vec![sid("core")],
    );
    assert_eq!(
        imp.flags_of(&sid("core")),
        ImportanceFlags {
            unread: true,
            critical: true
        }
    );

    // Visiting clears the one-off unread but leaves the continuing critical.
    imp.visit(&sid("core"));
    assert_eq!(
        imp.flags_of(&sid("core")),
        ImportanceFlags {
            unread: false,
            critical: true
        }
    );

    // Resolving the condition (objective still terminal, so no unread edge)
    // finally clears the Station entirely.
    imp.ingest(
        vec![obj("rescue", "core", ObjectiveStatus::Completed)],
        Vec::new(),
    );
    assert_eq!(imp.flags_of(&sid("core")), ImportanceFlags::default());
    assert!(imp.snapshots().is_empty());
}

#[test]
fn snapshots_are_deterministic_and_omit_resolved_stations() {
    let mut imp = StationImportance::default();
    imp.ingest(
        vec![
            obj("a", "comms", ObjectiveStatus::Completed),
            obj("b", "helm", ObjectiveStatus::Failed),
        ],
        vec![sid("core")],
    );
    let snaps = imp.snapshots();
    // BTreeMap order: comms, core, helm.
    assert_eq!(
        snaps
            .iter()
            .map(|s| s.station.0.as_str())
            .collect::<Vec<_>>(),
        vec!["comms", "core", "helm"]
    );
    assert_eq!(
        snaps.iter().find(|s| s.station.0 == "core").unwrap(),
        &StationImportanceSnapshot {
            station: sid("core"),
            unread: false,
            critical: true
        }
    );
}
