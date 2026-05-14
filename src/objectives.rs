// Pure Rust module for managing mission objectives.
// No Bevy dependency. Owns all objective state for the running simulation.
//
// Objectives are owned by the scenario that created them. Active objectives
// are removed when that scenario unloads; completed/failed objectives are
// retained until explicitly cleared.
//
// The public surface is intentionally narrow:
//   - `ObjectiveManager::add` — register a new active objective
//   - `ObjectiveManager::complete` — transition active → completed
//   - `ObjectiveManager::fail` — transition active → failed
//   - `ObjectiveManager::unload_scenario` — remove active objectives for a scenario
//   - `ObjectiveManager::sorted_snapshots` — sorted view (mandatory first)
//   - `ObjectiveManager::is_dirty` / `ObjectiveManager::mark_clean` — change tracking
//     so callers can push `ObjectiveSummary` only on change

use crate::messages::{ObjectiveSnapshot, ObjectiveStatus};

// ── Internal record ────────────────────────────────────────────────────────

#[derive(Clone, Debug)]
struct ObjectiveRecord {
    id: String,
    text: String,
    mandatory: bool,
    status: ObjectiveStatus,
    /// The scenario that created this objective (used for scoping).
    scenario_id: String,
}

// ── Manager ────────────────────────────────────────────────────────────────

/// Manages the full lifecycle of mission objectives across scenario boundaries.
#[derive(Clone, Debug, Default)]
pub struct ObjectiveManager {
    objectives: Vec<ObjectiveRecord>,
    dirty: bool,
}

impl ObjectiveManager {
    /// Create an empty `ObjectiveManager`.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a new `Active` objective owned by `scenario_id`.
    ///
    /// If an objective with this `id` already exists it is **not** duplicated;
    /// the call is a no-op and returns `false`. Returns `true` when the
    /// objective was newly inserted.
    pub fn add(&mut self, id: impl Into<String>, text: impl Into<String>, mandatory: bool, scenario_id: impl Into<String>) -> bool {
        let id = id.into();
        if self.objectives.iter().any(|o| o.id == id) {
            return false;
        }
        self.objectives.push(ObjectiveRecord {
            id,
            text: text.into(),
            mandatory,
            status: ObjectiveStatus::Active,
            scenario_id: scenario_id.into(),
        });
        self.dirty = true;
        true
    }

    /// Transition an `Active` objective to `Completed`.
    ///
    /// Returns `true` if the objective was found and transitioned.
    /// If the objective does not exist or is not `Active`, returns `false`.
    pub fn complete(&mut self, id: &str) -> bool {
        if let Some(rec) = self.objectives.iter_mut().find(|o| o.id == id && o.status == ObjectiveStatus::Active) {
            rec.status = ObjectiveStatus::Completed;
            self.dirty = true;
            true
        } else {
            false
        }
    }

    /// Transition an `Active` objective to `Failed`.
    ///
    /// Returns `true` if the objective was found and transitioned.
    /// If the objective does not exist or is not `Active`, returns `false`.
    pub fn fail(&mut self, id: &str) -> bool {
        if let Some(rec) = self.objectives.iter_mut().find(|o| o.id == id && o.status == ObjectiveStatus::Active) {
            rec.status = ObjectiveStatus::Failed;
            self.dirty = true;
            true
        } else {
            false
        }
    }

    /// Remove all `Active` objectives owned by `scenario_id`.
    ///
    /// `Completed` and `Failed` objectives are retained regardless of ownership.
    /// Returns the number of objectives removed.
    pub fn unload_scenario(&mut self, scenario_id: &str) -> usize {
        let before = self.objectives.len();
        self.objectives.retain(|o| {
            o.scenario_id != scenario_id || o.status != ObjectiveStatus::Active
        });
        let removed = before - self.objectives.len();
        if removed > 0 {
            self.dirty = true;
        }
        removed
    }

    /// Returns a sorted snapshot of all objectives: mandatory first (in
    /// insertion order), then optional (in insertion order).
    ///
    /// This is the slice that should be packed into `ObjectiveSummary`.
    pub fn sorted_snapshots(&self) -> Vec<ObjectiveSnapshot> {
        let mandatory: Vec<_> = self.objectives.iter()
            .filter(|o| o.mandatory)
            .map(record_to_snapshot)
            .collect();
        let optional: Vec<_> = self.objectives.iter()
            .filter(|o| !o.mandatory)
            .map(record_to_snapshot)
            .collect();
        mandatory.into_iter().chain(optional).collect()
    }

    /// `true` when the objective list has changed since the last `mark_clean` call.
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// Reset the dirty flag. Call after broadcasting `ObjectiveSummary`.
    pub fn mark_clean(&mut self) {
        self.dirty = false;
    }
}

fn record_to_snapshot(r: &ObjectiveRecord) -> ObjectiveSnapshot {
    ObjectiveSnapshot {
        id: r.id.clone(),
        text: r.text.clone(),
        mandatory: r.mandatory,
        status: r.status.clone(),
    }
}

// ── Unit Tests ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── Cycle 2: empty manager ─────────────────────────────────────────────

    #[test]
    fn empty_manager_returns_no_snapshots() {
        let mgr = ObjectiveManager::new();
        assert!(mgr.sorted_snapshots().is_empty());
    }

    #[test]
    fn empty_manager_is_not_dirty() {
        let mgr = ObjectiveManager::new();
        assert!(!mgr.is_dirty());
    }

    // ── Cycle 3: add objective ─────────────────────────────────────────────

    #[test]
    fn add_objective_appears_in_snapshots_as_active() {
        let mut mgr = ObjectiveManager::new();
        mgr.add("obj-1", "Destroy the convoy", true, "scenario-a");
        let snapshots = mgr.sorted_snapshots();
        assert_eq!(snapshots.len(), 1);
        assert_eq!(snapshots[0].id, "obj-1");
        assert_eq!(snapshots[0].text, "Destroy the convoy");
        assert_eq!(snapshots[0].mandatory, true);
        assert_eq!(snapshots[0].status, ObjectiveStatus::Active);
    }

    #[test]
    fn add_objective_marks_dirty() {
        let mut mgr = ObjectiveManager::new();
        mgr.add("obj-1", "Text", false, "s");
        assert!(mgr.is_dirty());
    }

    #[test]
    fn mark_clean_clears_dirty_flag() {
        let mut mgr = ObjectiveManager::new();
        mgr.add("obj-1", "Text", false, "s");
        mgr.mark_clean();
        assert!(!mgr.is_dirty());
    }

    #[test]
    fn adding_duplicate_id_is_noop() {
        let mut mgr = ObjectiveManager::new();
        let first = mgr.add("obj-1", "First", true, "s");
        let second = mgr.add("obj-1", "Second", false, "s");
        assert!(first);
        assert!(!second);
        assert_eq!(mgr.sorted_snapshots().len(), 1);
        assert_eq!(mgr.sorted_snapshots()[0].text, "First");
    }

    // ── Cycle 4: mandatory objectives sort before optional ─────────────────

    #[test]
    fn mandatory_objectives_sort_before_optional() {
        let mut mgr = ObjectiveManager::new();
        mgr.add("opt-1", "Optional A", false, "s");
        mgr.add("man-1", "Mandatory A", true, "s");
        mgr.add("opt-2", "Optional B", false, "s");
        mgr.add("man-2", "Mandatory B", true, "s");

        let snaps = mgr.sorted_snapshots();
        assert_eq!(snaps.len(), 4);
        // First two should be mandatory.
        assert_eq!(snaps[0].mandatory, true);
        assert_eq!(snaps[1].mandatory, true);
        // Last two should be optional.
        assert_eq!(snaps[2].mandatory, false);
        assert_eq!(snaps[3].mandatory, false);
        // Insertion order within each group.
        assert_eq!(snaps[0].id, "man-1");
        assert_eq!(snaps[1].id, "man-2");
        assert_eq!(snaps[2].id, "opt-1");
        assert_eq!(snaps[3].id, "opt-2");
    }

    // ── Cycle 5: complete_objective ───────────────────────────────────────

    #[test]
    fn complete_transitions_active_to_completed() {
        let mut mgr = ObjectiveManager::new();
        mgr.add("obj-1", "Destroy convoy", true, "s");
        mgr.mark_clean();

        let result = mgr.complete("obj-1");
        assert!(result);
        assert_eq!(mgr.sorted_snapshots()[0].status, ObjectiveStatus::Completed);
        assert!(mgr.is_dirty());
    }

    #[test]
    fn complete_returns_false_for_unknown_id() {
        let mut mgr = ObjectiveManager::new();
        let result = mgr.complete("nonexistent");
        assert!(!result);
        assert!(!mgr.is_dirty());
    }

    #[test]
    fn complete_returns_false_if_already_completed() {
        let mut mgr = ObjectiveManager::new();
        mgr.add("obj-1", "Text", true, "s");
        mgr.complete("obj-1");
        mgr.mark_clean();

        let result = mgr.complete("obj-1");
        assert!(!result);
        assert!(!mgr.is_dirty());
    }

    // ── Cycle 6: fail_objective ────────────────────────────────────────────

    #[test]
    fn fail_transitions_active_to_failed() {
        let mut mgr = ObjectiveManager::new();
        mgr.add("obj-1", "Save the station", true, "s");
        mgr.mark_clean();

        let result = mgr.fail("obj-1");
        assert!(result);
        assert_eq!(mgr.sorted_snapshots()[0].status, ObjectiveStatus::Failed);
        assert!(mgr.is_dirty());
    }

    #[test]
    fn fail_returns_false_for_unknown_id() {
        let mut mgr = ObjectiveManager::new();
        assert!(!mgr.fail("ghost"));
        assert!(!mgr.is_dirty());
    }

    #[test]
    fn fail_returns_false_if_already_failed() {
        let mut mgr = ObjectiveManager::new();
        mgr.add("obj-1", "Text", true, "s");
        mgr.fail("obj-1");
        mgr.mark_clean();
        assert!(!mgr.fail("obj-1"));
        assert!(!mgr.is_dirty());
    }

    // ── Cycle 7: unload_scenario scoping ──────────────────────────────────

    #[test]
    fn unload_scenario_removes_active_objectives_for_that_scenario() {
        let mut mgr = ObjectiveManager::new();
        mgr.add("obj-a", "From A", true, "scenario-a");
        mgr.add("obj-b", "From B", true, "scenario-b");
        mgr.mark_clean();

        let removed = mgr.unload_scenario("scenario-a");
        assert_eq!(removed, 1);
        let snaps = mgr.sorted_snapshots();
        assert_eq!(snaps.len(), 1);
        assert_eq!(snaps[0].id, "obj-b");
        assert!(mgr.is_dirty());
    }

    #[test]
    fn unload_scenario_retains_completed_objectives() {
        let mut mgr = ObjectiveManager::new();
        mgr.add("obj-1", "Text", true, "scenario-a");
        mgr.complete("obj-1");
        mgr.mark_clean();

        let removed = mgr.unload_scenario("scenario-a");
        assert_eq!(removed, 0);
        assert_eq!(mgr.sorted_snapshots().len(), 1);
        assert_eq!(mgr.sorted_snapshots()[0].status, ObjectiveStatus::Completed);
        assert!(!mgr.is_dirty());
    }

    #[test]
    fn unload_scenario_retains_failed_objectives() {
        let mut mgr = ObjectiveManager::new();
        mgr.add("obj-1", "Text", true, "scenario-a");
        mgr.fail("obj-1");
        mgr.mark_clean();

        mgr.unload_scenario("scenario-a");
        assert_eq!(mgr.sorted_snapshots().len(), 1);
        assert_eq!(mgr.sorted_snapshots()[0].status, ObjectiveStatus::Failed);
    }

    #[test]
    fn unload_scenario_does_not_affect_other_scenario_objectives() {
        let mut mgr = ObjectiveManager::new();
        mgr.add("obj-a", "From A", true, "scenario-a");
        mgr.add("obj-b", "From B", true, "scenario-b");

        mgr.unload_scenario("scenario-a");

        let snaps = mgr.sorted_snapshots();
        assert_eq!(snaps.len(), 1);
        assert_eq!(snaps[0].id, "obj-b");
    }

    #[test]
    fn unload_scenario_with_no_active_objectives_returns_zero_and_not_dirty() {
        let mut mgr = ObjectiveManager::new();
        mgr.mark_clean();
        let removed = mgr.unload_scenario("nonexistent");
        assert_eq!(removed, 0);
        assert!(!mgr.is_dirty());
    }
}
