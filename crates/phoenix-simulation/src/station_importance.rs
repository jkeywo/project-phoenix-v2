//! Pure Rust host-derived Console importance (issue #1101).
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

use crate::core::messages::{ConsoleId, ObjectiveStatus, StationId, StationImportanceSnapshot};
use phoenix_model::console::ConsoleImportanceSnapshot;
use std::collections::{HashMap, HashSet};

/// Attention follows a Console; unassigned ship-wide events use a distinct bucket.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum AttentionOwner {
    Console(ConsoleId),
    Core,
}

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
    flags: HashMap<AttentionOwner, ImportanceFlags>,
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
        objectives: impl IntoIterator<Item = (String, AttentionOwner, ObjectiveStatus)>,
        critical_stations: impl IntoIterator<Item = AttentionOwner>,
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
        let critical: HashSet<AttentionOwner> = critical_stations.into_iter().collect();
        let stations: Vec<AttentionOwner> = self
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
    pub fn visit(&mut self, station: &AttentionOwner) {
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
    pub fn flags_of(&self, station: &AttentionOwner) -> ImportanceFlags {
        self.flags.get(station).copied().unwrap_or_default()
    }

    /// The wire projection: one entry per Station that currently carries
    /// importance, in deterministic Station order. Fully-resolved Stations are
    /// absent by construction (see [`prune`](Self::prune)).
    pub fn snapshots(&self) -> Vec<ConsoleImportanceSnapshot> {
        let mut snaps: Vec<_> = self
            .flags
            .iter()
            .filter_map(|(owner, flags)| {
                let AttentionOwner::Console(console) = owner else {
                    return None;
                };
                flags.is_set().then(|| ConsoleImportanceSnapshot {
                    console: console.clone(),
                    unread: flags.unread,
                    critical: flags.critical,
                })
            })
            .collect();
        snaps.sort_by(|a, b| a.console.0.cmp(&b.console.0));
        snaps
    }

    /// Compatibility projection only: no second attention lifecycle is stored.
    /// Detached Consoles never borrow the current host's Station notification.
    pub fn station_snapshots(
        &self,
        homes: &HashMap<ConsoleId, StationId>,
    ) -> Vec<StationImportanceSnapshot> {
        let mut combined: HashMap<StationId, ImportanceFlags> = HashMap::new();
        for (owner, flags) in &self.flags {
            let station = match owner {
                AttentionOwner::Core => Some(StationId("core".into())),
                AttentionOwner::Console(console) => homes.get(console).cloned(),
            };
            if let Some(station) = station {
                let aggregate = combined.entry(station).or_default();
                aggregate.unread |= flags.unread;
                aggregate.critical |= flags.critical;
            }
        }
        let mut snaps: Vec<_> = combined
            .into_iter()
            .filter(|(_, flags)| flags.is_set())
            .map(|(station, flags)| StationImportanceSnapshot {
                station,
                unread: flags.unread,
                critical: flags.critical,
            })
            .collect();
        snaps.sort_by(|a, b| a.station.0.cmp(&b.station.0));
        snaps
    }
}

#[cfg(test)]
#[path = "station_importance_tests.rs"]
mod tests;
