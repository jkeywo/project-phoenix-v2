//! Pure Rust host-derived per-Station importance (issue #1101).
//!
//! No Bevy dependency. Owns the authoritative importance state the Hero Bar
//! renders and the [`SimSnapshot`](crate::core::messages::SimSnapshot) carries, kept
//! strictly apart from health. Two flags with DISTINCT lifecycles:
//!
//! - `unread` — a **one-off** event (an objective completing/failing off-screen
//!   for a Station). Edge-triggered when the objective first reaches a terminal
//!   status, then sticky until the Station is VISITED. The visit clears it, and
//!   because the edge is recorded it never re-raises for the same objective — so
//!   the clear is authoritative, not optimistic (AC2).
//! - `critical` — a **continuing** condition (a raised Red Alert attributed to a
//!   Station). Level-triggered: recomputed from live conditions every ingest, so
//!   it survives a visit (the visit never touches it) and lowers only when the
//!   condition itself resolves (AC3).
//!
//! Keeping `unread` edge-triggered and `critical` level-triggered is what makes
//! the two lifecycles independent: a visit clears the former without disturbing
//! the latter, and a resolved condition lowers the latter without resurrecting
//! the former.
//!
//! This is the SAME structure the visit-clear mutates and the host snapshot
//! builder reads — [`StationImportanceRes`](crate::server_app) wraps it as a
//! Bevy resource, an ingest pass folds objectives + Red Alert into it, and the
//! `StationVisited` drain calls [`StationImportance::visit`].

use crate::core::messages::{ObjectiveStatus, StationId, StationImportanceSnapshot};
use std::collections::{HashMap, HashSet};

/// The two independent importance flags for one Station.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ImportanceFlags {
    /// A one-off unread event, cleared on visit.
    pub unread: bool,
    /// A continuing critical condition, cleared only on resolve.
    pub critical: bool,
}

impl ImportanceFlags {
    /// Whether this Station carries any importance worth broadcasting.
    fn is_set(&self) -> bool {
        self.unread || self.critical
    }
}

/// Authoritative host-side importance state for every Station on the local ship.
///
/// Keyed by `StationId` (which is `Hash`, not `Ord`), so [`Self::snapshots`]
/// sorts on the id to keep the projected byte stream deterministic.
#[derive(Clone, Debug, Default)]
pub struct StationImportance {
    flags: HashMap<StationId, ImportanceFlags>,
    /// The last terminal status seen per objective id, so a completing/failing
    /// objective raises `unread` exactly once — the edge — and a later visit
    /// clears it for good rather than fighting a re-raise every tick.
    seen_terminal: HashMap<String, ObjectiveStatus>,
}

impl StationImportance {
    /// Fold this tick's host state into the importance projection.
    ///
    /// `objectives` is every live objective as `(id, attributed Station, status)`;
    /// `critical_stations` is the set of Stations under a continuing critical
    /// condition right now (e.g. the ship's Red Alert attributed to the core
    /// bucket). `unread` is raised on the terminal edge of an objective and left
    /// sticky; `critical` is recomputed wholesale from `critical_stations`, so a
    /// resolved condition lowers it while a still-set `unread` is untouched.
    pub fn ingest(
        &mut self,
        objectives: impl IntoIterator<Item = (String, StationId, ObjectiveStatus)>,
        critical_stations: impl IntoIterator<Item = StationId>,
    ) {
        for (id, station, status) in objectives {
            let terminal = matches!(status, ObjectiveStatus::Completed | ObjectiveStatus::Failed);
            if !terminal {
                continue;
            }
            // Only the first time this objective is seen at a terminal status:
            // that transition is the one-off event. Re-seeing the same terminal
            // status (every subsequent tick) is not a fresh event, so a visit's
            // clear stands.
            if self.seen_terminal.get(&id) == Some(&status) {
                continue;
            }
            self.seen_terminal.insert(id, status);
            self.flags.entry(station).or_default().unread = true;
        }

        // Level-trigger critical from the live set. Recompute across every
        // Station we track OR that is newly critical, so a lowered condition
        // actually clears rather than latching.
        let critical: HashSet<StationId> = critical_stations.into_iter().collect();
        let stations: Vec<StationId> = self
            .flags
            .keys()
            .cloned()
            .chain(critical.iter().cloned())
            .collect();
        for station in stations {
            let is_critical = critical.contains(&station);
            self.flags.entry(station).or_default().critical = is_critical;
        }
        self.prune();
    }

    /// Clear ONLY the one-off `unread` flag for a visited Station (AC2).
    ///
    /// `critical` is deliberately untouched — a continuing condition survives the
    /// visit and clears only when [`ingest`](Self::ingest) stops reporting it
    /// (AC3).
    pub fn visit(&mut self, station: &StationId) {
        if let Some(flags) = self.flags.get_mut(station) {
            flags.unread = false;
        }
        self.prune();
    }

    /// Drop fully-resolved Stations so they leave the broadcast — a client
    /// rebuilding its map from the projection then clears them authoritatively.
    fn prune(&mut self) {
        self.flags.retain(|_, flags| flags.is_set());
    }

    /// The importance flags for one Station (defaults to neutral when absent).
    /// Test-facing accessor.
    pub fn flags_of(&self, station: &StationId) -> ImportanceFlags {
        self.flags.get(station).copied().unwrap_or_default()
    }

    /// The wire projection: one entry per Station that currently carries
    /// importance, in deterministic Station order. Fully-resolved Stations are
    /// absent by construction (see [`prune`](Self::prune)).
    pub fn snapshots(&self) -> Vec<StationImportanceSnapshot> {
        let mut snaps: Vec<StationImportanceSnapshot> = self
            .flags
            .iter()
            .filter(|(_, flags)| flags.is_set())
            .map(|(station, flags)| StationImportanceSnapshot {
                station: station.clone(),
                unread: flags.unread,
                critical: flags.critical,
            })
            .collect();
        // `StationId` is not `Ord`; sort on the inner string so the byte stream
        // is deterministic regardless of `HashMap` iteration order.
        snaps.sort_by(|a, b| a.station.0.cmp(&b.station.0));
        snaps
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sid(s: &str) -> StationId {
        StationId(s.into())
    }

    fn obj(
        id: &str,
        station: &str,
        status: ObjectiveStatus,
    ) -> (String, StationId, ObjectiveStatus) {
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
}
