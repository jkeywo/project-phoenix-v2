use crate::core::messages::{StationId, SystemId, TeamSlot};
use crate::ship::config::ShipConfig;
use crate::ship::damage::{DamageTier, SystemHull};

/// Tunable timings for the repair-team state machine.
///
/// Sourced from the `[repair]` block in the ship entity TOML (e.g. `assets/entities/alliance_battleship.toml`)
/// via `RepairConfig::to_runtime()` (see `src/entities/config.rs`). Tests
/// and code paths that don't load a ship TOML use `RepairTimings::default()`,
/// which matches the historical hardcoded constants exactly.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RepairTimings {
    /// Seconds a team spends travelling to a console (or returning from one).
    pub travel_duration: f32,
    /// HP restored per second while the team is at the console.
    pub repair_rate_hp_per_sec: f32,
}

impl Default for RepairTimings {
    fn default() -> Self {
        Self {
            travel_duration: 5.0,
            repair_rate_hp_per_sec: 0.5, // 1 HP per 2 seconds
        }
    }
}

/// Pure state machine for all repair teams on the ship.
///
/// Teams are identified by slot index. The number of teams is set at
/// construction time from the ship config (`repair_team_count`).
///
/// After issue #619 the dispatch API keys on [`SystemId`] and every non-Idle
/// variant carries a `system_id` + `display_name`. The legacy `console` /
/// `queued` fields on `TeamSlot` were removed along with the `Console` enum.
#[derive(Debug, Clone)]
pub struct RepairTeams {
    slots: Vec<TeamSlot>,
    timings: RepairTimings,
}

impl RepairTeams {
    /// Create a new `RepairTeams` with `count` teams, all idle, using
    /// the default (hardcoded-baseline) timings.
    pub fn new(count: usize) -> Self {
        Self::new_with_timings(count, RepairTimings::default())
    }

    /// Create a new `RepairTeams` with `count` teams and explicit timings
    /// (typically from `RepairConfig::to_runtime()`).
    pub fn new_with_timings(count: usize, timings: RepairTimings) -> Self {
        Self {
            slots: vec![TeamSlot::Idle; count],
            timings,
        }
    }

    /// Borrow the current timings.
    pub fn timings(&self) -> RepairTimings {
        self.timings
    }

    /// Borrow the full slot slice.
    pub fn slots(&self) -> &[TeamSlot] {
        &self.slots
    }

    /// Replace every team's slot wholesale (issue #862's snapshot restore).
    ///
    /// The team *count* is not restored with it: how many teams a ship has is
    /// authored config the fresh world already rebuilt from TOML, and a save
    /// that disagreed about it is one the content-version gate refuses. Only as
    /// many slots as this ship actually has are written, so a longer stored
    /// list cannot grow a crew.
    pub fn restore_slots(&mut self, slots: &[TeamSlot]) {
        for (slot, stored) in self.slots.iter_mut().zip(slots) {
            *slot = stored.clone();
        }
    }

    /// System ids where a repair team is currently **on site** — i.e. physically
    /// present and working, not en route and not heading home.
    ///
    /// This is the information gate for issue #737: a repair team's travel time
    /// is both a repair delay *and* an information delay, so only
    /// [`TeamSlot::Repairing`] counts. `Travelling` teams have not arrived yet,
    /// and `Returning` teams have left — including the recall case, where a team
    /// recalled from `Travelling` goes straight to `Returning` without ever
    /// passing through `Repairing` and therefore never reveals anything.
    ///
    /// Named here (rather than inlined as a `matches!` at the publisher) so the
    /// host visibility projection and the PASM `onsite-repair-detail-state`
    /// entity can both point at one symbol.
    pub fn on_site_systems(&self) -> impl Iterator<Item = &SystemId> {
        self.slots.iter().filter_map(|slot| match slot {
            TeamSlot::Repairing {
                system_id: Some(sid),
                ..
            } => Some(sid),
            _ => None,
        })
    }

    /// Returns the index of the lowest-numbered idle team, or `None` if all are busy.
    pub fn lowest_free_team(&self) -> Option<usize> {
        self.slots.iter().position(|s| matches!(s, TeamSlot::Idle))
    }

    /// Which teams may be given new internal work, ascending, with `committed`
    /// of them held back for an external operation (issue #1027).
    ///
    /// **The one place "which teams are available" is answered.** The AI
    /// dispatcher and a human at the repair console both read it, so a
    /// field-repair's commitment cannot be undercut by whichever path happened
    /// not to know about it.
    ///
    /// The commitment eats from the **top** of the idle list, so the teams that
    /// remain are still the lowest-numbered ones: the AI's deterministic visit
    /// order is unchanged, and a ship with spare capacity behaves exactly as it
    /// did before commitments existed. Held back rather than dispatched — a
    /// committed team is still `Idle` in every readout, because it has not gone
    /// anywhere. It is simply spoken for.
    pub fn free_team_indices(&self, committed: u8) -> Vec<usize> {
        let mut idle: Vec<usize> = self
            .slots
            .iter()
            .enumerate()
            .filter_map(|(i, slot)| matches!(slot, TeamSlot::Idle).then_some(i))
            .collect();
        idle.truncate(idle.len().saturating_sub(usize::from(committed)));
        idle
    }

    /// Whether `team_idx` is one of the `committed` teams held back for an
    /// external operation (issue #1027), and so may not be given new work.
    ///
    /// Only ever true of an idle team: a team already out on an internal job was
    /// never part of the commitment, and recalling or redirecting it stays the
    /// console's business.
    pub fn is_committed_to_operation(&self, team_idx: usize, committed: u8) -> bool {
        matches!(self.slots.get(team_idx), Some(TeamSlot::Idle))
            && !self.free_team_indices(committed).contains(&team_idx)
    }

    /// Dispatch the team at `team_idx` to the given system.
    ///
    /// `display_name` is the human-readable label for the target used to
    /// populate `TeamSlot::{Travelling,Repairing,Returning}.display_name`
    /// on the wire. Callers must pass a value derived from the caller's
    /// domain knowledge (e.g. the target system's `SystemHull` entry's
    /// `display_name` field). Passing the raw SystemId string is a fallback
    /// of last resort; do not do it if a proper display name is reachable.
    ///
    /// Transition rules:
    /// - `Idle` → `Travelling { system_id, elapsed: 0.0 }`.
    /// - `Travelling { elapsed: t }` to a **different** system (redirect):
    ///   → `Returning { remaining: t, queued_system_id: Some(...) }`.
    /// - `Travelling { elapsed: t }` to the **same** system (recall):
    ///   → `Returning { remaining: t, queued_system_id: None }`.
    /// - `Repairing` to any system (redirect): `remaining = travel_duration`, queued.
    /// - `Repairing` to same system (recall): `remaining = travel_duration`, no queue.
    /// - `Returning` with a queued system: replace the queued system.
    /// - `Returning` with no queue: add the system as queued (or clear if same).
    pub fn dispatch(&mut self, team_idx: usize, new_system: SystemId, display_name: String) {
        let travel_duration = self.timings.travel_duration;
        let Some(slot) = self.slots.get_mut(team_idx) else {
            return;
        };
        let new_label = display_name;
        match slot.clone() {
            TeamSlot::Idle => {
                *slot = TeamSlot::Travelling {
                    system_id: Some(new_system),
                    display_name: Some(new_label),
                    elapsed: 0.0,
                    priority: None,
                };
            }
            TeamSlot::Travelling {
                system_id: current,
                elapsed,
                display_name: current_label,
                ..
            } => {
                let is_same = current.as_ref() == Some(&new_system);
                let (queued_sid, queued_label) = if is_same {
                    (None, None)
                } else {
                    (Some(new_system), Some(new_label))
                };
                *slot = TeamSlot::Returning {
                    remaining: elapsed,
                    system_id: current,
                    display_name: current_label,
                    queued_system_id: queued_sid,
                    queued_display_name: queued_label,
                };
            }
            TeamSlot::Repairing {
                system_id: current,
                display_name: current_label,
                ..
            } => {
                let is_same = current.as_ref() == Some(&new_system);
                let (queued_sid, queued_label) = if is_same {
                    (None, None)
                } else {
                    (Some(new_system), Some(new_label))
                };
                *slot = TeamSlot::Returning {
                    remaining: travel_duration,
                    system_id: current,
                    display_name: current_label,
                    queued_system_id: queued_sid,
                    queued_display_name: queued_label,
                };
            }
            TeamSlot::Returning {
                remaining,
                system_id,
                display_name,
                ..
            } => {
                let queued_sid = Some(new_system);
                let queued_label = Some(new_label);
                *slot = TeamSlot::Returning {
                    remaining,
                    system_id,
                    display_name,
                    queued_system_id: queued_sid,
                    queued_display_name: queued_label,
                };
            }
        }
    }

    /// Set the priority for the team at `team_idx`. Only takes effect when the
    /// team is in `Repairing` state. Returns `true` if the priority was set,
    /// `false` if the team is not in `Repairing` state (or the index is out of
    /// range).
    ///
    /// # What the priority DOES (issue #1013)
    ///
    /// Before #1013 this value was written and never read — a repair team fixed
    /// the one system it was sent to and went home, so there was no second
    /// choice for a priority to steer. Now that an on-site team SWEEPS every
    /// non-Operational system at its station (see [`Self::tick`]), the priority
    /// is the sweep's order selector and [`next_sweep_target`] consumes it:
    /// the remaining work is ranked worst-first and `Some(n)` picks the `n`th
    /// entry of that ranking, 1-based and clamped to the list; `None` (and
    /// `Some(0)`) pick the first. Changing it mid-sweep therefore re-orders the
    /// work the team has left.
    ///
    /// It is deliberately NOT a score added to the ranking: an ordinal pick
    /// over a ranking the host already computes stays a pure function of
    /// observable damage.
    ///
    /// # What sets it (and what does not)
    ///
    /// This is a STANDING PER-TEAM INSTRUCTION about a station ("always work the
    /// second-worst thing"), which is why it survives the hand-off. Nothing in
    /// the UI sends it: issue #1015 replaced the per-team 1/2/3 ordinal buttons
    /// with the damaged-systems list, and a tap on that list writes
    /// [`Self::prioritise_system`]'s PIN instead — a named system, not a rank.
    /// `SystemControlPayload::SetRepairPriority` stays on the wire for anything
    /// that genuinely means "the nth job" (tooling, and any future AI or console
    /// that wants a standing order rather than a one-shot choice).
    pub fn set_priority(&mut self, team_idx: usize, priority: u8) -> bool {
        let Some(slot) = self.slots.get_mut(team_idx) else {
            return false;
        };
        match slot {
            TeamSlot::Repairing { priority: p, .. } => {
                *p = Some(priority);
                true
            }
            _ => false,
        }
    }

    /// Pin `target` as the next job of whichever on-site team already covers
    /// `target`'s station group (issue #1015). Returns the slot index that took
    /// the order, or `None` when no team's sweep can reach that system.
    ///
    /// The repair console's damaged-systems taps name a SYSTEM, not a number,
    /// because the console cannot see enough of the ship to work a number out —
    /// `SystemControlPayload::SetRepairTargetPriority` documents that boundary.
    /// So what is stored here is the system too: `priority_system_id`, which
    /// [`next_sweep_target`] prefers over `priority` whenever the pinned row is
    /// still a candidate at the hand-off.
    ///
    /// # Why the pin and not a resolved ordinal
    ///
    /// Resolving the tap to a RANK here and storing that would be a
    /// knowingly-false UI. A tap and the hand-off it steers are separated by
    /// however long the current system takes to finish, and combat damage in
    /// between re-ranks the group — so an ordinal frozen at tap time can select
    /// a DIFFERENT system by the time it is consumed, while the console goes on
    /// highlighting the pinned row as `[NEXT]`. The pin cannot drift that way:
    /// it names the row, and its only failure mode is leaving the candidate list
    /// altogether, which [`next_sweep_target`]'s ordinal fallback covers.
    ///
    /// `priority` is therefore left alone by a tap. It stays exactly what
    /// [`Self::set_priority`] made it: #1013's standing per-team instruction.
    ///
    /// Candidacy is delegated to [`sweep_candidates`] — the same list the
    /// hand-off picks from, with the same exclusions — so a tap this accepts is
    /// exactly a tap the hand-off can honour.
    ///
    /// Refusals, all silent and all leaving the slot untouched:
    /// - no team is `Repairing` in `target`'s group (nobody is there to steer);
    /// - the team's current system is not a hull row, the same gate
    ///   [`sweep_from`] applies so a station-name dispatch cannot sweep;
    /// - `target` is not among the candidates — it is `Operational`, it is not
    ///   repairable at all, or SOME TEAM IS ALREADY ON SITE AT IT (including the
    ///   team being asked). That last exclusion is what makes a tap on a busy row
    ///   a refusal for the whole ship rather than a fall-through: a team's own
    ///   system was never its own candidate, so before the exclusion existed a
    ///   tap on team 0's current system simply moved on to team 1 and pinned it
    ///   there, converging two teams on one row.
    ///
    /// Ties (two teams sweeping the same group) go to the lowest slot index, so
    /// the choice is a pure function of state like every other repair decision.
    pub fn prioritise_system(
        &mut self,
        target: &SystemId,
        hull: &SystemHull,
        config: &ShipConfig,
    ) -> Option<usize> {
        let target_group = sweep_group(target, config);
        // Snapshot before the search: `sweep_candidates` needs the whole on-site
        // set, and the write at the end needs `self.slots` mutably.
        let occupied: Vec<SystemId> = self.on_site_systems().cloned().collect();
        let idx = self.slots.iter().enumerate().find_map(|(idx, slot)| {
            let TeamSlot::Repairing {
                system_id: Some(current),
                ..
            } = slot
            else {
                return None;
            };
            hull.get(current)?;
            if sweep_group(current, config) != target_group {
                return None;
            }
            sweep_candidates(current, hull, config, &occupied)
                .iter()
                .any(|(sid, _)| sid == target)
                .then_some(idx)
        })?;
        if let Some(TeamSlot::Repairing {
            priority_system_id, ..
        }) = self.slots.get_mut(idx)
        {
            *priority_system_id = Some(target.clone());
        }
        Some(idx)
    }

    /// Advance all active timers by `dt` seconds.
    ///
    /// - `Travelling` advances its `elapsed` toward `travel_duration`, then
    ///   transitions to `Repairing`. If the target system is already at full HP
    ///   on arrival, the team sweeps (below) for other work at the same station
    ///   and only goes `Returning` when there is none.
    /// - `Repairing` calls `hull.restore(&sid, dt * repair_rate_hp_per_sec)`
    ///   each tick. Once the system is at full HP the team SWEEPS: it stays
    ///   `Repairing` and moves to the next system its station still needs
    ///   fixing, and only transitions to `Returning` when the station is clean.
    /// - `Returning` decrements `remaining` toward 0. On completion:
    ///   - If `queued_system_id = Some(sid)`: auto-dispatch to
    ///     `Travelling { system_id: sid, elapsed: 0 }`.
    ///   - Otherwise: → `Idle`.
    ///
    /// # The sweep (issue #1013)
    ///
    /// A team that has walked to a station is AT that station: making it hike
    /// home after one system and be re-dispatched to its neighbour was a state
    /// machine artefact, not a rule anybody wanted. So the "system is at max HP"
    /// edge no longer ends the visit — it hands off to [`next_sweep_target`],
    /// which picks the next non-Operational system in the same station group and
    /// writes it into the SAME `Repairing` slot in place. No new `TeamSlot`
    /// variant is involved, so the wire shape, the `on_site_systems` gate
    /// (issue #737 — a sweeping team's on-site detail simply follows it from
    /// system to system) and the client's slot rendering all carry over
    /// untouched.
    ///
    /// The team's `priority` survives the in-place hand-off rather than being
    /// reset, because it is a standing instruction about the station ("work the
    /// second-worst thing"), not a fact about one system. [`Self::set_priority`]
    /// documents what it selects. The console's `priority_system_id` PIN is the
    /// opposite — one named row, consumed by the hand-off it steers — and
    /// outranks the ordinal while it lasts (issue #1015).
    ///
    /// # No two teams on one system
    ///
    /// A sweep never hands off to a system another team is standing on: the slot
    /// walk below snapshots who is where and passes it to [`sweep_candidates`]
    /// as an exclusion set, kept in step as teams move within the tick. Without
    /// it, two teams sweeping one station converge the moment their timings line
    /// up — both grinding the same row while the rest of the station waits.
    /// [`Self::prioritise_system`] leans on the same exclusion so a console tap
    /// cannot arrange that convergence deliberately.
    ///
    /// `config` supplies the ONLY thing this module cannot derive from a hull:
    /// which systems share a station. Mirrors
    /// [`crate::modifiers::power_system::PowerSystem::tick`]'s `&PowerConfig`
    /// parameter — a pure tick reading pure authored config. `None` means the
    /// caller has no ship config to offer (bare fixtures, and any ship spawned
    /// without a `ShipConfigComponent`); with no station membership there is no
    /// group to sweep, so the team reverts to the pre-#1013 behaviour of fixing
    /// its one system and going home. Repairing a Destroyed system does NOT
    /// depend on `config` — that happens either way.
    ///
    /// The other route back to that pre-#1013 bounce is an arrival that is not a
    /// hull row at all — see [`sweep_from`], the entry point both arms below call
    /// rather than [`next_sweep_target`] directly.
    ///
    /// # Destroyed systems are repairable by the sweep
    ///
    /// The two guards that bounced a team off a `Destroyed` system are gone.
    /// `SystemHull::restore` has never had a tier gate, and `tier_for` checks
    /// `current == 0.0` FIRST, so the first fraction of an HP restored lifts the
    /// latch on its own — a Destroyed system is `Disabled` again the instant the
    /// team touches it. The old rule ("a repair team alone cannot lift the
    /// latch") left destroyed systems permanently stuck with nothing else in the
    /// game able to clear them.
    pub fn tick(&mut self, dt: f32, hull: &mut SystemHull, config: Option<&ShipConfig>) {
        let travel_duration = self.timings.travel_duration;
        let repair_rate = self.timings.repair_rate_hp_per_sec;
        // Which system each team is standing on, indexed by slot. Snapshotted
        // before the `&mut` walk (which cannot ask `self` that question
        // mid-iteration) and updated in place as teams move, so a hand-off later
        // in the same tick sees where an earlier one actually landed rather than
        // where it started.
        let mut on_site: Vec<Option<SystemId>> = self
            .slots
            .iter()
            .map(|slot| match slot {
                TeamSlot::Repairing {
                    system_id: Some(sid),
                    ..
                } => Some(sid.clone()),
                _ => None,
            })
            .collect();
        for (team_idx, slot) in self.slots.iter_mut().enumerate() {
            match slot {
                TeamSlot::Travelling {
                    system_id,
                    elapsed,
                    display_name,
                    ..
                } => {
                    *elapsed += dt;
                    if *elapsed >= travel_duration {
                        let Some(sid) = system_id.clone() else {
                            *slot = TeamSlot::Returning {
                                remaining: 0.0,
                                system_id: None,
                                display_name: None,
                                queued_system_id: None,
                                queued_display_name: None,
                            };
                            continue;
                        };
                        // Carry the display name forward from the current
                        // `Travelling` slot so the human-readable label the
                        // caller supplied at dispatch time survives the
                        // Travelling → Repairing/Returning transition. Falls
                        // back to the raw SystemId only if the slot never
                        // had a label (e.g. legacy on-wire messages without
                        // the new field).
                        let label = display_name.clone().or_else(|| Some(sid.0.clone()));
                        if !hull.is_at_max(&sid) {
                            // Includes a Destroyed target since #1013: the
                            // arrival bounce off `tier == Destroyed` is gone.
                            on_site[team_idx] = Some(sid.clone());
                            *slot = TeamSlot::Repairing {
                                system_id: Some(sid),
                                display_name: label,
                                priority: None,
                                priority_system_id: None,
                            };
                            continue;
                        }
                        // Arrived to find the target already whole — someone
                        // else fixed it, or the dispatch resolved to a healthy
                        // fallback. Sweep the station before walking home; the
                        // team is standing right there. An arriving team has
                        // neither a priority nor a pin yet (both only reach a
                        // `Repairing` slot), so this takes the top-ranked
                        // candidate.
                        let next = {
                            let exclude = occupied_systems(&on_site);
                            sweep_from(&sid, None, None, hull, config, &exclude)
                        };
                        match next {
                            Some((next_sid, next_label)) => {
                                on_site[team_idx] = Some(next_sid.clone());
                                *slot = TeamSlot::Repairing {
                                    system_id: Some(next_sid),
                                    display_name: Some(next_label),
                                    priority: None,
                                    priority_system_id: None,
                                };
                            }
                            None => {
                                on_site[team_idx] = None;
                                *slot = TeamSlot::Returning {
                                    remaining: 0.0,
                                    system_id: Some(sid),
                                    display_name: label,
                                    queued_system_id: None,
                                    queued_display_name: None,
                                };
                            }
                        }
                    }
                }
                TeamSlot::Repairing {
                    system_id,
                    display_name,
                    priority,
                    priority_system_id,
                } => {
                    let Some(sid) = system_id.clone() else {
                        on_site[team_idx] = None;
                        *slot = TeamSlot::Returning {
                            remaining: travel_duration,
                            system_id: None,
                            display_name: None,
                            queued_system_id: None,
                            queued_display_name: None,
                        };
                        continue;
                    };
                    // Carry the display name forward from `Repairing`
                    // through the Returning transition for the same reason
                    // as the Travelling arm above.
                    let carried_label = display_name.clone().or_else(|| Some(sid.0.clone()));
                    // The standing "which of the remaining jobs" instruction,
                    // carried across the sweep hand-off below (issue #1013).
                    let carried_priority = *priority;
                    // The console's one-shot pin: the row Engineering actually
                    // tapped (issue #1015). Outranks the ordinal at the hand-off.
                    let carried_pin = priority_system_id.clone();
                    let hp_to_restore = dt * repair_rate;
                    // No tier gate: a Destroyed system is repaired like any
                    // other, and the first restored HP un-latches it because
                    // `tier_for` tests `current == 0.0` before anything else.
                    hull.restore(&sid, hp_to_restore);
                    if !hull.is_at_max(&sid) {
                        continue;
                    }
                    // This system is whole. Sweep to the next thing the station
                    // needs, in place, without walking home first.
                    let next = {
                        let exclude = occupied_systems(&on_site);
                        sweep_from(
                            &sid,
                            carried_priority,
                            carried_pin.as_ref(),
                            hull,
                            config,
                            &exclude,
                        )
                    };
                    match next {
                        Some((next_sid, next_label)) => {
                            on_site[team_idx] = Some(next_sid.clone());
                            *slot = TeamSlot::Repairing {
                                system_id: Some(next_sid),
                                display_name: Some(next_label),
                                priority: carried_priority,
                                // The console's pin steered THIS hand-off and is
                                // spent (issue #1015). `priority` survives
                                // because it is the standing instruction; the
                                // pin does not, because after the move it would
                                // restate where the team already is.
                                priority_system_id: None,
                            };
                        }
                        None => {
                            on_site[team_idx] = None;
                            *slot = TeamSlot::Returning {
                                remaining: travel_duration,
                                system_id: Some(sid),
                                display_name: carried_label,
                                queued_system_id: None,
                                queued_display_name: None,
                            };
                        }
                    }
                }
                TeamSlot::Returning {
                    remaining,
                    queued_system_id,
                    queued_display_name,
                    ..
                } => {
                    *remaining -= dt;
                    if *remaining <= 0.0 {
                        if let Some(sid) = queued_system_id.take() {
                            let label = queued_display_name.take().unwrap_or_else(|| sid.0.clone());
                            *slot = TeamSlot::Travelling {
                                system_id: Some(sid),
                                display_name: Some(label),
                                elapsed: 0.0,
                                priority: None,
                            };
                        } else {
                            *slot = TeamSlot::Idle;
                        }
                    }
                }
                TeamSlot::Idle => {}
            }
        }
    }
}

impl Default for RepairTeams {
    fn default() -> Self {
        Self::new(2)
    }
}

/// The sweep group a hull system belongs to: `Some(station)` when the ship
/// config gives it an owning station, `None` for the ownerless `core` bucket.
///
/// A hull entry the config does not describe at all is ownerless, not a group of
/// its own — the shipped example being the synthesised `core` entry, which is a
/// `[[hull.system_hull]]` row with no `[[system]]` behind it (a station named
/// `core` is forbidden by `ShipConfig` validation). That is the same rule
/// `HullVisibility::can_see_station` uses to bucket a hull row for the repair
/// console and the same one `damage_sync` uses to address a `RepairRequest`, so
/// "the station a console tapped", "the rows Engineering can see" and "the
/// systems a team sweeps" cannot drift apart.
fn sweep_group(sid: &SystemId, config: &ShipConfig) -> Option<StationId> {
    config.system(sid).and_then(|s| s.station.clone())
}

/// The sweep entry point: [`next_sweep_target`], but only from a system the
/// hull ACTUALLY TRACKS.
///
/// `resolve_repair_target` falls back to `SystemId(station_id)` when a station
/// has no repairable system — the battleship's `helm` station resolves to
/// `SystemId("helm")`, which is a station name and not a hull row. Every hull
/// lookup answers such an id permissively (`is_at_max` → true for an untracked
/// id, `restore` → no-op), which is exactly right for "walk there and find
/// nothing to do": before issue #1013 the team bounced straight to `Returning`.
///
/// Without this guard the sweep inherits that permissive `true` and then asks
/// [`sweep_group`] which station the id belongs to — and an id the config does
/// not describe is deliberately the OWNERLESS `core` bucket, so the team would
/// walk off its station and start repairing `core`. A dispatch that resolved to
/// a station name must bounce exactly as it did before, so the sweep is gated on
/// the arrival being a real hull row rather than on the hull's fallback answers.
///
/// This gate and the fallback are a COMPLEMENTARY PAIR, and both halves are
/// needed: "the hull tracks this row" is not by itself the same predicate as
/// "this is not a station name", because a hull row and a station can share an
/// id (`alliance_cruiser` authors both a `science` hull row and a `science`
/// station). `resolve_repair_target` therefore emits its fallback only for a
/// name the hull does NOT track, so an arrival here that is a hull row is always
/// a genuine system and never a colliding station name that would sweep the
/// wrong group.
fn sweep_from(
    sid: &SystemId,
    priority: Option<u8>,
    pin: Option<&SystemId>,
    hull: &SystemHull,
    config: Option<&ShipConfig>,
    exclude: &[SystemId],
) -> Option<(SystemId, String)> {
    hull.get(sid)?;
    config.and_then(|c| next_sweep_target(sid, priority, pin, hull, c, exclude))
}

/// Flatten [`RepairTeams::tick`]'s per-slot on-site snapshot into the exclusion
/// set [`sweep_candidates`] takes.
///
/// The sweeping team's OWN system is left in deliberately: `sweep_candidates`
/// already drops it as `current`, so filtering it out here would only add a
/// second rule saying the same thing in a place that could fall out of step
/// with the first.
fn occupied_systems(on_site: &[Option<SystemId>]) -> Vec<SystemId> {
    on_site.iter().flatten().cloned().collect()
}

/// Pick the next system for an on-site team to work on, or `None` when nothing
/// else in its station group needs a repair team.
///
/// Returns the chosen `(SystemId, display_name)`. The label comes from the
/// hull entry, exactly as `handle_dispatch_repair_team` sources it for a
/// console-ordered dispatch, so a swept-to system is labelled identically to a
/// dispatched-to one.
///
/// # Candidates
///
/// Every OTHER hull system in the same [`sweep_group`] whose tier is not
/// `Operational`. That predicate is deliberately the same one the AI dispatch
/// prune uses (`console::repair::server::operate_repair_ai`), so the sweep never
/// chases damage the dispatcher would not have sent a team for in the first
/// place — a system merely below max HP but still `Operational` is left alone.
/// `Destroyed` counts as a candidate since issue #1013.
///
/// A candidate must additionally be BELOW max HP, so the team only picks work
/// it can finish. See the guard at the filter for why that is not implied by the
/// tier test: a `max = 0` hull row is Destroyed and at max simultaneously, and a
/// group holding two of them would livelock the sweep.
///
/// # Ranking, and what `priority` selects
///
/// Candidates are ranked worst-first: tier severity descending
/// (`Destroyed` > `Disabled` > `Damaged`), then damage fraction
/// (`1 - current/max`) descending, then `SystemId` ascending. The last key is
/// there purely so the answer cannot depend on hull iteration order — a repair
/// choice feeds the sim digest like any other.
///
/// `priority` is then an ORDINAL PICK over that ranking, 1-based and clamped to
/// the candidate count: `Some(2)` takes the second-worst job, `None` (and
/// `Some(0)`) the worst. This is the semantic issue #1013 gives
/// `TeamSlot::priority` — previously written by `SetRepairPriority` and read by
/// nothing.
///
/// # The pin beats the ordinal
///
/// `pin` is [`RepairTeams::prioritise_system`]'s `priority_system_id`: the row
/// the repair console actually tapped (issue #1015). When it is still on the
/// candidate list it wins outright, ordinal ignored, because it is a statement
/// about THIS system rather than about a position in a ranking that moves under
/// it — every shell that lands between the tap and this hand-off re-ranks the
/// group, and a rank frozen at tap time would quietly select something the
/// console is still labelling `[NEXT]`.
///
/// A pin that is no longer a candidate — repaired to max by someone else, blown
/// off the ship, or taken by another team — falls through to the ordinal rather
/// than stranding the team. That is the one case where the console's highlight
/// and the destination legitimately differ, and it resolves by the pin simply
/// not existing any more.
fn next_sweep_target(
    current: &SystemId,
    priority: Option<u8>,
    pin: Option<&SystemId>,
    hull: &SystemHull,
    config: &ShipConfig,
    exclude: &[SystemId],
) -> Option<(SystemId, String)> {
    let candidates = sweep_candidates(current, hull, config, exclude);
    if candidates.is_empty() {
        return None;
    }
    if let Some(pinned) = pin {
        if let Some(hit) = candidates.iter().find(|(sid, _)| sid == pinned) {
            return Some(hit.clone());
        }
    }
    let rank = usize::from(priority.unwrap_or(1).max(1)).min(candidates.len());
    candidates.into_iter().nth(rank - 1)
}

/// The ranked worst-first list [`next_sweep_target`] picks from, and the list
/// [`RepairTeams::prioritise_system`] looks a console tap up in.
///
/// One function rather than two so "a tap this ship accepts" and "a system this
/// hand-off will go to" cannot be answered by two comparators that drift apart —
/// see [`next_sweep_target`] for the candidate rule and the ranking keys it
/// documents.
///
/// `exclude` is the systems repair teams are currently ON SITE at (issue #737's
/// predicate, [`RepairTeams::on_site_systems`]). Two teams grinding one row
/// while the rest of a station waits is never what anybody ordered, so a system
/// somebody is standing on is not work anybody else can take — and, because this
/// is the one list both the sweep and the console tap consult, that holds for
/// the tap as well as for the automatic hand-off. The caller's own system is
/// dropped as `current` regardless, so passing the whole set including it is
/// correct and is what both callers do.
fn sweep_candidates(
    current: &SystemId,
    hull: &SystemHull,
    config: &ShipConfig,
    exclude: &[SystemId],
) -> Vec<(SystemId, String)> {
    let group = sweep_group(current, config);
    let mut candidates: Vec<(DamageTier, f32, &SystemId, &str)> = hull
        .iter()
        .filter(|(sid, _)| {
            *sid != current
                && sweep_group(sid, config) == group
                && !exclude.iter().any(|busy| busy == *sid)
        })
        .filter_map(|(sid, entry)| {
            // Only work the team can actually PROGRESS. A row already at max HP
            // has nothing to restore, and the two predicates are not redundant:
            // a `max = 0` row is permanently `Destroyed` (`tier_for` tests
            // `current == 0.0` before it looks at any ratio) AND permanently at
            // max, so it is a candidate that can never stop being one. Two of
            // them in a group make a team hand off from one to the other and
            // back forever, never finishing and never going home — a livelock,
            // not a slow repair. `restore` clamps to `max`, so no amount of
            // repair rate changes it.
            if entry.current >= entry.max {
                return None;
            }
            let tier = hull.tier_for(sid);
            if tier == DamageTier::Operational {
                return None;
            }
            let fraction = if entry.max > 0.0 {
                1.0 - entry.current / entry.max
            } else {
                0.0
            };
            Some((tier, fraction, sid, entry.display_name.as_str()))
        })
        .collect();
    candidates.sort_by(|a, b| {
        b.0.cmp(&a.0)
            .then_with(|| b.1.total_cmp(&a.1))
            .then_with(|| a.2.cmp(b.2))
    });
    candidates
        .into_iter()
        .map(|(_, _, sid, label)| (sid.clone(), label.to_string()))
        .collect()
}

#[cfg(test)]
#[path = "repair_teams_tests.rs"]
mod tests;
