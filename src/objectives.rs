// Pure Rust module for managing mission objectives.
// No Bevy dependency. Owns all objective state for the running simulation.
//
// PRD #342: legacy multi-scenario layering is gone. Objectives live for the
// duration of the session. Completed/failed objectives are retained until
// explicitly cleared.
//
// The public surface is intentionally narrow:
//   - `ObjectiveManager::add` — register a new active objective
//   - `ObjectiveManager::complete` — transition active → completed
//   - `ObjectiveManager::fail` — transition active → failed
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
    targets: Vec<String>,
}

// ── Manager ────────────────────────────────────────────────────────────────

/// Manages the full lifecycle of mission objectives.
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

    /// Add a new `Active` objective.
    ///
    /// If an objective with this `id` already exists it is **not** duplicated;
    /// the call is a no-op and returns `false`. Returns `true` when the
    /// objective was newly inserted.
    pub fn add(
        &mut self,
        id: impl Into<String>,
        text: impl Into<String>,
        mandatory: bool,
        targets: Vec<String>,
    ) -> bool {
        let id = id.into();
        if self.objectives.iter().any(|o| o.id == id) {
            return false;
        }
        self.objectives.push(ObjectiveRecord {
            id,
            text: text.into(),
            mandatory,
            status: ObjectiveStatus::Active,
            targets,
        });
        self.dirty = true;
        true
    }

    /// Transition an `Active` objective to `Completed`.
    ///
    /// Returns `true` if the objective was found and transitioned.
    /// If the objective does not exist or is not `Active`, returns `false`.
    pub fn complete(&mut self, id: &str) -> bool {
        if let Some(rec) = self
            .objectives
            .iter_mut()
            .find(|o| o.id == id && o.status == ObjectiveStatus::Active)
        {
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
        if let Some(rec) = self
            .objectives
            .iter_mut()
            .find(|o| o.id == id && o.status == ObjectiveStatus::Active)
        {
            rec.status = ObjectiveStatus::Failed;
            self.dirty = true;
            true
        } else {
            false
        }
    }

    /// Returns a sorted snapshot of all objectives: mandatory first (in
    /// insertion order), then optional (in insertion order).
    ///
    /// This is the slice that should be packed into `ObjectiveSummary`.
    pub fn sorted_snapshots(&self) -> Vec<ObjectiveSnapshot> {
        let mandatory: Vec<_> = self
            .objectives
            .iter()
            .filter(|o| o.mandatory)
            .map(record_to_snapshot)
            .collect();
        let optional: Vec<_> = self
            .objectives
            .iter()
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
        targets: r.targets.clone(),
    }
}

// ── Unit Tests ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn add_objective_appears_in_snapshots_as_active() {
        let mut mgr = ObjectiveManager::new();
        mgr.add("obj-1", "Destroy the convoy", true, vec![]);
        let snapshots = mgr.sorted_snapshots();
        assert_eq!(snapshots.len(), 1);
        assert_eq!(snapshots[0].id, "obj-1");
        assert_eq!(snapshots[0].text, "Destroy the convoy");
        assert!(snapshots[0].mandatory);
        assert_eq!(snapshots[0].status, ObjectiveStatus::Active);
    }

    #[test]
    fn add_objective_marks_dirty() {
        let mut mgr = ObjectiveManager::new();
        mgr.add("obj-1", "Text", false, vec![]);
        assert!(mgr.is_dirty());
    }

    #[test]
    fn mark_clean_clears_dirty_flag() {
        let mut mgr = ObjectiveManager::new();
        mgr.add("obj-1", "Text", false, vec![]);
        mgr.mark_clean();
        assert!(!mgr.is_dirty());
    }

    #[test]
    fn adding_duplicate_id_is_noop() {
        let mut mgr = ObjectiveManager::new();
        let first = mgr.add("obj-1", "First", true, vec![]);
        let second = mgr.add("obj-1", "Second", false, vec![]);
        assert!(first);
        assert!(!second);
        assert_eq!(mgr.sorted_snapshots().len(), 1);
        assert_eq!(mgr.sorted_snapshots()[0].text, "First");
    }

    #[test]
    fn mandatory_objectives_sort_before_optional() {
        let mut mgr = ObjectiveManager::new();
        mgr.add("opt-1", "Optional A", false, vec![]);
        mgr.add("man-1", "Mandatory A", true, vec![]);
        mgr.add("opt-2", "Optional B", false, vec![]);
        mgr.add("man-2", "Mandatory B", true, vec![]);

        let snaps = mgr.sorted_snapshots();
        assert_eq!(snaps.len(), 4);
        assert!(snaps[0].mandatory);
        assert!(snaps[1].mandatory);
        assert!(!snaps[2].mandatory);
        assert!(!snaps[3].mandatory);
        assert_eq!(snaps[0].id, "man-1");
        assert_eq!(snaps[1].id, "man-2");
        assert_eq!(snaps[2].id, "opt-1");
        assert_eq!(snaps[3].id, "opt-2");
    }

    #[test]
    fn complete_transitions_active_to_completed() {
        let mut mgr = ObjectiveManager::new();
        mgr.add("obj-1", "Destroy convoy", true, vec![]);
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
        mgr.add("obj-1", "Text", true, vec![]);
        mgr.complete("obj-1");
        mgr.mark_clean();

        let result = mgr.complete("obj-1");
        assert!(!result);
        assert!(!mgr.is_dirty());
    }

    #[test]
    fn fail_transitions_active_to_failed() {
        let mut mgr = ObjectiveManager::new();
        mgr.add("obj-1", "Save the station", true, vec![]);
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
        mgr.add("obj-1", "Text", true, vec![]);
        mgr.fail("obj-1");
        mgr.mark_clean();
        assert!(!mgr.fail("obj-1"));
        assert!(!mgr.is_dirty());
    }

    #[test]
    fn add_objective_stores_targets() {
        let mut mgr = ObjectiveManager::new();
        mgr.add(
            "obj-1",
            "Destroy Ironveil",
            true,
            vec!["Ironveil".to_string()],
        );
        let snaps = mgr.sorted_snapshots();
        assert_eq!(snaps[0].targets, vec!["Ironveil".to_string()]);
    }

    #[test]
    fn add_objective_stores_multiple_targets() {
        let mut mgr = ObjectiveManager::new();
        mgr.add(
            "obj-1",
            "Hail Axiom Station or Research Outpost",
            false,
            vec!["Axiom Station".to_string(), "Research Outpost".to_string()],
        );
        let snaps = mgr.sorted_snapshots();
        assert_eq!(
            snaps[0].targets,
            vec!["Axiom Station".to_string(), "Research Outpost".to_string()]
        );
    }

    #[test]
    fn add_objective_without_targets_is_empty() {
        let mut mgr = ObjectiveManager::new();
        mgr.add("obj-1", "Survive", true, vec![]);
        let snaps = mgr.sorted_snapshots();
        assert!(snaps[0].targets.is_empty());
    }

    #[test]
    fn add_objective_without_targets_is_empty_vec() {
        let mut mgr = ObjectiveManager::new();
        mgr.add("obj-1", "Survive", true, vec![]);
        let snaps = mgr.sorted_snapshots();
        assert!(snaps[0].targets.is_empty());
    }
}
