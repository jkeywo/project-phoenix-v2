use crate::damage::{DamageTier, SystemHull};
use crate::messages::{StationId, SystemId, TeamSlot};
use crate::ship::config::ShipConfig;

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
mod tests {
    use super::*;

    fn sid(s: &str) -> SystemId {
        SystemId(s.into())
    }

    fn hull_with_helm(max_hp: f32) -> SystemHull {
        SystemHull::from_config(&[(sid("helm"), max_hp)])
    }

    fn hull_full() -> SystemHull {
        hull_with_helm(25.0)
    }

    fn hull_damaged(current: f32) -> SystemHull {
        let mut h = SystemHull::from_config(&[(sid("helm"), 25.0)]);
        // Damage it down to `current` by applying the difference.
        let dmg = 25.0 - current;
        if dmg > 0.0 {
            let mut rng = crate::sim_rng::unseeded_test_rng();
            h.apply_damage(dmg, &mut rng);
        }
        h
    }

    // ── Default state ─────────────────────────────────────────────────────────

    #[test]
    fn new_teams_all_idle() {
        let teams = RepairTeams::new(3);
        assert_eq!(teams.slots().len(), 3);
        assert!(teams.slots().iter().all(|s| matches!(s, TeamSlot::Idle)));
    }

    #[test]
    fn default_has_two_teams() {
        let teams = RepairTeams::default();
        assert_eq!(teams.slots().len(), 2);
        assert!(teams.slots().iter().all(|s| matches!(s, TeamSlot::Idle)));
    }

    // ── lowest_free_team ──────────────────────────────────────────────────────

    #[test]
    fn lowest_free_team_returns_zero_when_all_idle() {
        let teams = RepairTeams::new(2);
        assert_eq!(teams.lowest_free_team(), Some(0));
    }

    #[test]
    fn lowest_free_team_skips_busy_teams() {
        let mut teams = RepairTeams::new(2);
        teams.dispatch(0, sid("helm"), "Helm".to_string());
        assert_eq!(teams.lowest_free_team(), Some(1));
    }

    #[test]
    fn lowest_free_team_returns_none_when_all_busy() {
        let mut teams = RepairTeams::new(2);
        teams.dispatch(0, sid("helm"), "Helm".to_string());
        teams.dispatch(1, sid("tactical"), "Tactical".to_string());
        assert_eq!(teams.lowest_free_team(), None);
    }

    // ── dispatch ──────────────────────────────────────────────────────────────

    #[test]
    fn dispatch_idle_team_enters_travelling() {
        let mut teams = RepairTeams::new(2);
        teams.dispatch(0, sid("helm"), "Helm".to_string());
        let expected = Some(sid("helm"));
        assert!(matches!(
            &teams.slots()[0],
            TeamSlot::Travelling { system_id, elapsed, .. }
                if *system_id == expected && *elapsed == 0.0
        ));
    }

    #[test]
    fn dispatch_non_idle_team_is_noop() {
        // Dispatching to the same system (recall) sets Returning with no queue.
        let mut teams = RepairTeams::new(2);
        teams.dispatch(0, sid("helm"), "Helm".to_string());
        // Recall (same system)
        teams.dispatch(0, sid("helm"), "Helm".to_string());
        assert!(matches!(
            &teams.slots()[0],
            TeamSlot::Returning {
                queued_system_id: None,
                ..
            }
        ));
    }

    // ── Travelling → Repairing ────────────────────────────────────────────────

    #[test]
    fn travelling_transitions_to_repairing_after_5s() {
        let mut teams = RepairTeams::new(1);
        let mut hull = hull_damaged(20.0); // not at max
        teams.dispatch(0, sid("helm"), "Helm".to_string());
        teams.tick(5.0, &mut hull, None);
        let expected = Some(sid("helm"));
        assert!(matches!(
            &teams.slots()[0],
            TeamSlot::Repairing { system_id, .. } if *system_id == expected
        ));
    }

    #[test]
    fn travelling_does_not_transition_before_5s() {
        let mut teams = RepairTeams::new(1);
        let mut hull = hull_damaged(20.0);
        teams.dispatch(0, sid("helm"), "Helm".to_string());
        teams.tick(4.9, &mut hull, None);
        assert!(matches!(&teams.slots()[0], TeamSlot::Travelling { .. }));
    }

    #[test]
    fn team_arrives_at_full_hp_console_enters_returning() {
        let mut teams = RepairTeams::new(1);
        let mut hull = hull_full(); // system already at full HP
        teams.dispatch(0, sid("helm"), "Helm".to_string());
        teams.tick(5.0, &mut hull, None);
        assert!(matches!(&teams.slots()[0], TeamSlot::Returning { .. }));
    }

    #[test]
    fn repairing_restores_hp_at_correct_rate() {
        let mut teams = RepairTeams::new(1);
        let mut hull = hull_damaged(1.0); // 1 HP (Disabled, not Destroyed)
        teams.dispatch(0, sid("helm"), "Helm".to_string());
        teams.tick(5.0, &mut hull, None); // travel
                                          // Now repairing; restore for 2s should give 1 more HP (0.5 HP/s)
        teams.tick(2.0, &mut hull, None);
        let hp = hull.current_for(&sid("helm")).unwrap();
        assert!(
            (hp - 2.0).abs() < 1e-4,
            "expected 2 HP after 2s repair starting from 1 HP, got {hp}"
        );
    }

    /// With no station context (`config: None`) the "system reached max HP"
    /// edge still ends the visit, exactly as it did before the #1013 sweep.
    #[test]
    fn repairing_transitions_to_returning_when_console_full() {
        let mut teams = RepairTeams::new(1);
        let mut hull = hull_damaged(24.9); // almost full
        teams.dispatch(0, sid("helm"), "Helm".to_string());
        teams.tick(5.0, &mut hull, None); // travel
                                          // One tick of 1s restores 0.5 HP — enough to max at 25
        teams.tick(1.0, &mut hull, None);
        assert!(matches!(&teams.slots()[0], TeamSlot::Returning { .. }));
    }

    // ── Returning → Idle ──────────────────────────────────────────────────────

    #[test]
    fn returning_transitions_to_idle_after_5s() {
        let mut teams = RepairTeams::new(1);
        let mut hull = hull_full();
        teams.dispatch(0, sid("helm"), "Helm".to_string());
        teams.tick(5.0, &mut hull, None); // travel (arrives full → Returning with remaining=0)
                                          // remaining is already 0 from arriving at full hp; tick 0.1 to trigger idle
        teams.tick(0.1, &mut hull, None);
        assert!(matches!(&teams.slots()[0], TeamSlot::Idle));
    }

    #[test]
    fn returning_does_not_complete_before_remaining_expires() {
        let mut teams = RepairTeams::new(1);
        let mut hull = hull_damaged(24.9); // not full
        teams.dispatch(0, sid("helm"), "Helm".to_string());
        teams.tick(5.0, &mut hull, None); // travel → Repairing
        teams.tick(1.0, &mut hull, None); // repair → full → Returning { remaining: 5.0 }
        teams.tick(4.9, &mut hull, None); // remaining not yet expired
        assert!(matches!(&teams.slots()[0], TeamSlot::Returning { .. }));
    }

    // ── Full lifecycle ────────────────────────────────────────────────────────

    /// The whole Idle → Travelling → Repairing → Returning → Idle walk, with no
    /// station context and so no sweep. The `Some(config)` counterpart — a team
    /// that keeps working instead of going home — is
    /// `sweep_repairs_every_damaged_system_at_the_station_in_one_visit`.
    #[test]
    fn full_lifecycle_travel_repair_return_idle() {
        let mut teams = RepairTeams::new(1);
        let mut hull = hull_damaged(1.0); // 1 HP (Disabled, not Destroyed — repairable)
        teams.dispatch(0, sid("helm"), "Helm".to_string());

        // Travelling
        teams.tick(5.0, &mut hull, None);
        assert!(matches!(&teams.slots()[0], TeamSlot::Repairing { .. }));

        // Repairing until full (24 HP remaining at 0.5 HP/s = 48s)
        teams.tick(50.0, &mut hull, None);
        assert!(matches!(&teams.slots()[0], TeamSlot::Returning { .. }));

        // Returning (remaining starts at TRAVEL_DURATION = 5s)
        teams.tick(5.1, &mut hull, None);
        assert!(matches!(&teams.slots()[0], TeamSlot::Idle));
    }

    // ── Multiple teams independence ───────────────────────────────────────────

    #[test]
    fn two_teams_operate_independently() {
        let mut hull = SystemHull::from_config(&[(sid("helm"), 25.0), (sid("tactical"), 25.0)]);
        // Damage both systems
        let mut rng = crate::sim_rng::unseeded_test_rng();
        hull.apply_damage(10.0, &mut rng);
        hull.apply_damage(10.0, &mut rng);

        let mut teams = RepairTeams::new(2);
        teams.dispatch(0, sid("helm"), "Helm".to_string());
        teams.dispatch(1, sid("tactical"), "Tactical".to_string());

        // Both should be Travelling to correct sids
        let expected_helm = Some(sid("helm"));
        let expected_tac = Some(sid("tactical"));
        assert!(matches!(
            &teams.slots()[0],
            TeamSlot::Travelling { system_id, .. } if *system_id == expected_helm
        ));
        assert!(matches!(
            &teams.slots()[1],
            TeamSlot::Travelling { system_id, .. } if *system_id == expected_tac
        ));

        // After 5s both transition
        teams.tick(5.0, &mut hull, None);
        let s0 = &teams.slots()[0];
        let s1 = &teams.slots()[1];
        assert!(
            matches!(
                s0,
                TeamSlot::Repairing { system_id, .. } if *system_id == expected_helm
            ) || matches!(s0, TeamSlot::Returning { .. })
        );
        assert!(
            matches!(
                s1,
                TeamSlot::Repairing { system_id, .. } if *system_id == expected_tac
            ) || matches!(s1, TeamSlot::Returning { .. })
        );
    }

    #[test]
    fn non_idle_team_cannot_be_redirected_while_travelling() {
        // Redirect while Travelling to a DIFFERENT system → Returning with queued
        let mut teams = RepairTeams::new(2);
        teams.dispatch(0, sid("helm"), "Helm".to_string());
        teams.dispatch(0, sid("tactical"), "Tactical".to_string()); // redirect to different system
        let expected = Some(sid("tactical"));
        assert!(matches!(
            &teams.slots()[0],
            TeamSlot::Returning { queued_system_id, .. } if *queued_system_id == expected
        ));
    }

    #[test]
    fn team_after_returning_can_be_dispatched_again() {
        let mut teams = RepairTeams::new(1);
        let mut hull = hull_full();
        teams.dispatch(0, sid("helm"), "Helm".to_string());
        teams.tick(5.0, &mut hull, None); // travel → Returning (remaining=0, full HP)
        teams.tick(0.1, &mut hull, None); // → Idle
        assert!(matches!(&teams.slots()[0], TeamSlot::Idle));
        teams.dispatch(0, sid("helm"), "Helm".to_string());
        assert!(matches!(&teams.slots()[0], TeamSlot::Travelling { .. }));
    }

    // ── Redirect / Recall new behaviors ──────────────────────────────────────

    #[test]
    fn redirect_mid_travel_sets_remaining_equal_to_elapsed() {
        let mut teams = RepairTeams::new(1);
        let mut hull = hull_damaged(10.0);
        teams.dispatch(0, sid("helm"), "Helm".to_string());
        // Advance 2s into travel
        teams.tick(2.0, &mut hull, None);
        assert!(
            matches!(&teams.slots()[0], TeamSlot::Travelling { elapsed, .. } if (*elapsed - 2.0).abs() < 1e-4)
        );
        // Redirect to a different system
        teams.dispatch(0, sid("tactical"), "Tactical".to_string());
        // remaining should equal the elapsed (2.0)
        let expected = Some(sid("tactical"));
        assert!(matches!(
            &teams.slots()[0],
            TeamSlot::Returning {
                remaining,
                queued_system_id,
                ..
            } if (*remaining - 2.0).abs() < 1e-4 && *queued_system_id == expected
        ));
    }

    #[test]
    fn recall_mid_travel_sets_returning_no_queue() {
        let mut teams = RepairTeams::new(1);
        let mut hull = hull_damaged(10.0);
        teams.dispatch(0, sid("helm"), "Helm".to_string());
        teams.tick(3.0, &mut hull, None);
        teams.dispatch(0, sid("helm"), "Helm".to_string()); // same system = recall
        assert!(matches!(
            &teams.slots()[0],
            TeamSlot::Returning {
                queued_system_id: None,
                ..
            }
        ));
    }

    #[test]
    fn redirect_while_repairing_sets_returning_with_travel_duration() {
        let mut teams = RepairTeams::new(1);
        let mut hull = hull_damaged(1.0); // 1 HP (Disabled, not Destroyed — repairable)
        teams.dispatch(0, sid("helm"), "Helm".to_string());
        teams.tick(5.0, &mut hull, None); // travel → Repairing
        assert!(matches!(&teams.slots()[0], TeamSlot::Repairing { .. }));
        teams.dispatch(0, sid("tactical"), "Tactical".to_string());
        let expected = Some(sid("tactical"));
        assert!(matches!(
            &teams.slots()[0],
            TeamSlot::Returning {
                remaining,
                queued_system_id,
                ..
            } if (*remaining - 5.0).abs() < 1e-4 && *queued_system_id == expected
        ));
    }

    #[test]
    fn recall_while_repairing_sets_returning_no_queue() {
        let mut teams = RepairTeams::new(1);
        let mut hull = hull_damaged(1.0); // 1 HP (Disabled, not Destroyed — repairable)
        teams.dispatch(0, sid("helm"), "Helm".to_string());
        teams.tick(5.0, &mut hull, None); // travel → Repairing
        teams.dispatch(0, sid("helm"), "Helm".to_string()); // recall
        assert!(matches!(
            &teams.slots()[0],
            TeamSlot::Returning {
                queued_system_id: None,
                ..
            }
        ));
    }

    #[test]
    fn partial_hp_restored_before_recall_is_preserved() {
        let mut teams = RepairTeams::new(1);
        let mut hull = hull_damaged(1.0); // 1 HP (Disabled, not Destroyed — repairable)
        teams.dispatch(0, sid("helm"), "Helm".to_string());
        teams.tick(5.0, &mut hull, None); // travel → Repairing
        teams.tick(2.0, &mut hull, None); // restore 1 HP (0.5 HP/s * 2s = 1 HP → now 2 HP)
        let hp_before_recall = hull.current_for(&sid("helm")).unwrap();
        assert!(
            (hp_before_recall - 2.0).abs() < 1e-4,
            "expected 2 HP before recall, got {hp_before_recall}"
        );
        teams.dispatch(0, sid("helm"), "Helm".to_string()); // recall
                                                            // HP should not have changed
        let hp_after_recall = hull.current_for(&sid("helm")).unwrap();
        assert!((hp_after_recall - hp_before_recall).abs() < 1e-4);
    }

    #[test]
    fn returning_with_queue_auto_dispatches_on_completion() {
        let mut teams = RepairTeams::new(1);
        let mut hull = hull_damaged(10.0);
        teams.dispatch(0, sid("helm"), "Helm".to_string());
        teams.tick(2.0, &mut hull, None); // elapsed=2
        teams.dispatch(0, sid("tactical"), "Tactical".to_string()); // redirect → Returning { remaining:2, queued:Tactical }
        teams.tick(2.1, &mut hull, None); // remaining expires → auto-dispatch to Tactical
        let expected = Some(sid("tactical"));
        assert!(matches!(
            &teams.slots()[0],
            TeamSlot::Travelling {
                system_id,
                elapsed,
                ..
            } if *system_id == expected && *elapsed < 1e-3
        ));
    }

    #[test]
    fn returning_with_no_queue_becomes_idle_on_completion() {
        let mut teams = RepairTeams::new(1);
        let mut hull = hull_damaged(10.0);
        teams.dispatch(0, sid("helm"), "Helm".to_string());
        teams.tick(2.0, &mut hull, None);
        teams.dispatch(0, sid("helm"), "Helm".to_string()); // recall → Returning { remaining:2, queued:None }
        teams.tick(2.1, &mut hull, None); // expires → Idle
        assert!(matches!(&teams.slots()[0], TeamSlot::Idle));
    }

    #[test]
    fn dispatching_team_0_does_not_affect_team_1() {
        let mut teams = RepairTeams::new(2);
        teams.dispatch(0, sid("helm"), "Helm".to_string());
        // team 1 remains Idle
        assert!(matches!(&teams.slots()[1], TeamSlot::Idle));
        // redirect team 0
        teams.dispatch(0, sid("tactical"), "Tactical".to_string());
        assert!(
            matches!(&teams.slots()[1], TeamSlot::Idle),
            "team 1 should be unaffected"
        );
    }

    // ── Destroyed latch tests ─────────────────────────────────────────────────

    /// The direct inverse of the pre-#1013 rule: a repair team dispatched to a
    /// Destroyed system (hp == 0) works on it like any other, and the first
    /// restored HP un-latches the tier — `tier_for` tests `current == 0.0`
    /// before it looks at any threshold, so there is nothing further to clear.
    ///
    /// This is the "un-stuck" acceptance criterion: before #1013 a destroyed
    /// system stayed destroyed forever, because the team bounced off it and
    /// nothing else in the game restores HP.
    #[test]
    fn destroyed_system_is_repaired_and_unlatched_by_repair_tick() {
        let mut teams = RepairTeams::new(1);
        // Build a hull with helm at 0 HP (Destroyed).
        let mut hull = SystemHull::from_config(&[(sid("helm"), 25.0)]);
        let mut rng = crate::sim_rng::unseeded_test_rng();
        hull.apply_damage(1000.0, &mut rng); // wipe to 0
        assert_eq!(
            hull.tier_for(&sid("helm")),
            DamageTier::Destroyed,
            "precondition: helm must be Destroyed"
        );
        assert_eq!(hull.current_for(&sid("helm")), Some(0.0));

        teams.dispatch(0, sid("helm"), "Helm".to_string());
        teams.tick(5.0, &mut hull, None); // travel
        assert!(
            matches!(&teams.slots()[0], TeamSlot::Repairing { .. }),
            "a team must now go to work on a Destroyed system, got {:?}",
            teams.slots()[0]
        );

        teams.tick(2.0, &mut hull, None); // 0.5 HP/s * 2s = 1 HP
        let hp_after = hull.current_for(&sid("helm")).unwrap();
        assert!(
            (hp_after - 1.0).abs() < 1e-4,
            "the team must restore HP to a Destroyed system (got {hp_after})"
        );
        assert_eq!(
            hull.tier_for(&sid("helm")),
            DamageTier::Disabled,
            "any positive HP un-latches Destroyed"
        );
    }

    // ── The station sweep (issue #1013) ───────────────────────────────────────

    /// A ship config built from `(system id, owning station)` pairs. `None`
    /// leaves the system ownerless, which is the `core` bucket group.
    /// Constructed struct-first rather than through TOML: these tests are about
    /// station MEMBERSHIP and nothing else in `ShipConfig` matters to the sweep.
    fn config_with(systems: &[(&str, Option<&str>)]) -> ShipConfig {
        use crate::ship::config::SystemInstanceConfig;
        ShipConfig {
            stations: vec![],
            systems: systems
                .iter()
                .map(|(id, station)| SystemInstanceConfig {
                    id: sid(id),
                    kind: "generic".into(),
                    station: station.map(|s| StationId(s.into())),
                    ai_only: station.is_none(),
                    power_group: None,
                    marker: None,
                    config: None,
                })
                .collect(),
            power_groups: Default::default(),
            coordination_lag_secs: 2.0,
        }
    }

    /// A hull built from `(system id, max hp, current hp)` triples.
    fn hull_at(entries: &[(&str, f32, f32)]) -> SystemHull {
        let mut hull = SystemHull::from_config(
            &entries
                .iter()
                .map(|(id, max, _)| (sid(id), *max))
                .collect::<Vec<_>>(),
        );
        for (id, _, current) in entries {
            hull.set_hp(&sid(id), *current);
        }
        hull
    }

    /// The three-system `helm` station every sweep test below works over, plus
    /// one `tactical` system and one ownerless `core` entry that must never be
    /// swept from `helm`. Tiers (defaults: <0.75 Damaged, <0.25 Disabled,
    /// 0 Destroyed) are therefore, worst first:
    /// `helm-c` Destroyed, `helm-b` Disabled, `helm-a` Damaged.
    fn station_hull() -> SystemHull {
        hull_at(&[
            ("helm-a", 10.0, 7.0),     // Damaged (0.70)
            ("helm-b", 10.0, 2.0),     // Disabled (0.20)
            ("helm-c", 10.0, 0.0),     // Destroyed
            ("tactical-x", 10.0, 1.0), // Disabled, but a different station
            ("core", 10.0, 5.0),       // Damaged, ownerless bucket
        ])
    }

    fn station_config() -> ShipConfig {
        config_with(&[
            ("helm-a", Some("helm")),
            ("helm-b", Some("helm")),
            ("helm-c", Some("helm")),
            ("tactical-x", Some("tactical")),
            // `core` is deliberately absent: the shipped hulls declare it as a
            // `[[hull.system_hull]]` row with no `[[system]]` behind it.
        ])
    }

    /// The system team 0 is currently working on, if any.
    fn repairing_at(teams: &RepairTeams) -> Option<String> {
        match &teams.slots()[0] {
            TeamSlot::Repairing {
                system_id: Some(s), ..
            } => Some(s.0.clone()),
            _ => None,
        }
    }

    /// Drive team 0 in small steps, recording each system it works on in order.
    /// Stops the moment the team goes `Returning` (or runs out of steps), so the
    /// returned flag distinguishes "swept everything then went home" from "was
    /// still working when we gave up".
    fn walk_sweep(
        teams: &mut RepairTeams,
        hull: &mut SystemHull,
        config: Option<&ShipConfig>,
        steps: usize,
    ) -> (Vec<String>, bool) {
        let mut visited: Vec<String> = vec![];
        let mut returned = false;
        for _ in 0..steps {
            teams.tick(0.5, hull, config);
            match &teams.slots()[0] {
                TeamSlot::Repairing {
                    system_id: Some(s), ..
                } if visited.last() != Some(&s.0) => {
                    visited.push(s.0.clone());
                }
                TeamSlot::Returning { .. } => {
                    returned = true;
                    break;
                }
                _ => {}
            }
        }
        (visited, returned)
    }

    /// AC1: one team, one station, three damaged systems — all three are
    /// repaired in worst-first order, in one visit, with no trip home between
    /// them. The team only heads back once the station is clean.
    #[test]
    fn sweep_repairs_every_damaged_system_at_the_station_in_one_visit() {
        let mut teams = RepairTeams::new(1);
        let mut hull = station_hull();
        let config = station_config();
        teams.dispatch(0, sid("helm-c"), "Helm C".to_string());

        let (visited, returned) = walk_sweep(&mut teams, &mut hull, Some(&config), 400);

        assert_eq!(
            visited,
            vec!["helm-c", "helm-b", "helm-a"],
            "the team must work the station worst-first without going home \
             between systems"
        );
        assert!(returned, "the team goes home once the station is clean");
        for id in ["helm-a", "helm-b", "helm-c"] {
            assert!(
                hull.is_at_max(&sid(id)),
                "{id} must be fully repaired by the sweep"
            );
        }
    }

    /// The sweep is bounded by the station: a damaged system another station
    /// owns is not the sweeping team's business, and neither is the ownerless
    /// `core` bucket.
    #[test]
    fn sweep_does_not_cross_into_another_station_or_the_core_bucket() {
        let mut teams = RepairTeams::new(1);
        let mut hull = station_hull();
        let config = station_config();
        teams.dispatch(0, sid("helm-c"), "Helm C".to_string());

        let (visited, returned) = walk_sweep(&mut teams, &mut hull, Some(&config), 400);

        assert!(returned);
        assert!(
            !visited.iter().any(|v| v == "tactical-x" || v == "core"),
            "a helm team must not sweep other stations' work, got {visited:?}"
        );
        assert_eq!(hull.current_for(&sid("tactical-x")), Some(1.0));
        assert_eq!(hull.current_for(&sid("core")), Some(5.0));
    }

    /// The ownerless bucket is a sweep group of its own: a team at `core`
    /// (a hull row with no `[[system]]` behind it) sweeps on to other
    /// station-less systems, and stops at the station boundary the same way.
    #[test]
    fn sweep_covers_the_ownerless_core_bucket_group() {
        let mut teams = RepairTeams::new(1);
        let mut hull = hull_at(&[
            ("core", 10.0, 5.0),       // Damaged, ownerless
            ("aux-sensor", 10.0, 0.0), // Destroyed, ownerless (`ai_only`)
            ("helm-a", 10.0, 2.0),     // Disabled, but owned by `helm`
        ]);
        let config = config_with(&[("aux-sensor", None), ("helm-a", Some("helm"))]);
        teams.dispatch(0, sid("core"), "Core".to_string());

        let (visited, returned) = walk_sweep(&mut teams, &mut hull, Some(&config), 400);

        assert_eq!(
            visited,
            vec!["core", "aux-sensor"],
            "an ownerless-bucket team sweeps the other ownerless systems only"
        );
        assert!(returned);
        assert_eq!(hull.current_for(&sid("helm-a")), Some(2.0));
    }

    /// A single-damaged-system station behaves exactly as it did before the
    /// sweep existed: repair it, then go home.
    #[test]
    fn single_damaged_system_station_still_repairs_one_and_returns() {
        let mut teams = RepairTeams::new(1);
        let mut hull = hull_at(&[("helm-a", 10.0, 2.0), ("helm-b", 10.0, 10.0)]);
        let config = config_with(&[("helm-a", Some("helm")), ("helm-b", Some("helm"))]);
        teams.dispatch(0, sid("helm-a"), "Helm A".to_string());

        let (visited, returned) = walk_sweep(&mut teams, &mut hull, Some(&config), 200);

        assert_eq!(visited, vec!["helm-a"]);
        assert!(returned);
    }

    /// A system that is below max HP but still `Operational` is not swept to:
    /// the sweep's damage predicate is the same `tier != Operational` the AI
    /// dispatch prune uses, so the team never chases work the dispatcher would
    /// not have sent it for.
    #[test]
    fn sweep_ignores_a_below_max_but_operational_system() {
        let mut teams = RepairTeams::new(1);
        // helm-b at 9/10 → ratio 0.9 → Operational despite the missing HP.
        let mut hull = hull_at(&[("helm-a", 10.0, 2.0), ("helm-b", 10.0, 9.0)]);
        let config = config_with(&[("helm-a", Some("helm")), ("helm-b", Some("helm"))]);
        teams.dispatch(0, sid("helm-a"), "Helm A".to_string());

        let (visited, returned) = walk_sweep(&mut teams, &mut hull, Some(&config), 200);

        assert_eq!(visited, vec!["helm-a"]);
        assert!(returned);
        assert_eq!(hull.current_for(&sid("helm-b")), Some(9.0));
    }

    /// Without a ship config there is no station membership to sweep over, so a
    /// team falls back to the pre-#1013 behaviour: fix the one system it was
    /// sent to and walk home, even with more damage sitting next to it.
    #[test]
    fn without_config_a_team_repairs_one_system_and_returns() {
        let mut teams = RepairTeams::new(1);
        let mut hull = station_hull();
        teams.dispatch(0, sid("helm-c"), "Helm C".to_string());

        let (visited, returned) = walk_sweep(&mut teams, &mut hull, None, 400);

        assert_eq!(visited, vec!["helm-c"]);
        assert!(returned);
        assert_eq!(
            hull.current_for(&sid("helm-b")),
            Some(2.0),
            "with no station context the team cannot know helm-b is its neighbour"
        );
    }

    /// A team that arrives to find its target already whole sweeps the station
    /// rather than turning straight around — it is standing right there.
    #[test]
    fn arrival_at_a_whole_system_sweeps_the_station_instead_of_returning() {
        let mut teams = RepairTeams::new(1);
        let mut hull = hull_at(&[("helm-a", 10.0, 10.0), ("helm-b", 10.0, 2.0)]);
        let config = config_with(&[("helm-a", Some("helm")), ("helm-b", Some("helm"))]);
        teams.dispatch(0, sid("helm-a"), "Helm A".to_string());

        teams.tick(5.0, &mut hull, Some(&config));

        assert_eq!(
            repairing_at(&teams).as_deref(),
            Some("helm-b"),
            "arriving at a healthy system must hand off to the station's real \
             work, got {:?}",
            teams.slots()[0]
        );
    }

    /// With nothing else to do at the station, the arrival bounce is unchanged.
    #[test]
    fn arrival_at_a_whole_system_returns_when_the_station_is_clean() {
        let mut teams = RepairTeams::new(1);
        let mut hull = hull_at(&[("helm-a", 10.0, 10.0), ("helm-b", 10.0, 10.0)]);
        let config = config_with(&[("helm-a", Some("helm")), ("helm-b", Some("helm"))]);
        teams.dispatch(0, sid("helm-a"), "Helm A".to_string());

        teams.tick(5.0, &mut hull, Some(&config));

        assert!(matches!(&teams.slots()[0], TeamSlot::Returning { .. }));
    }

    /// The swept-to system carries the hull entry's display name, the same
    /// source `handle_dispatch_repair_team` uses for a console-ordered dispatch.
    ///
    /// The hull is built through `from_config_with_display_names` — the spawner's
    /// own path — so the label and the raw SystemId are DIFFERENT strings.
    /// `from_config` sets `display_name = sid.0`, which makes "labelled from the
    /// hull entry" and "fell back to the raw id" indistinguishable and lets the
    /// fallback pass a test written for the label.
    #[test]
    fn sweep_labels_the_next_system_from_its_hull_entry() {
        use crate::damage::ConsoleTierConfig;
        let mut teams = RepairTeams::new(1);
        let mut hull = SystemHull::from_config_with_display_names(vec![
            (
                sid("helm-a"),
                "Helm Alpha".to_string(),
                10.0,
                ConsoleTierConfig::default(),
            ),
            (
                sid("helm-b"),
                "Helm Beta".to_string(),
                10.0,
                ConsoleTierConfig::default(),
            ),
        ]);
        hull.set_hp(&sid("helm-b"), 2.0);
        let config = config_with(&[("helm-a", Some("helm")), ("helm-b", Some("helm"))]);
        teams.dispatch(0, sid("helm-a"), "Helm Alpha".to_string());
        teams.tick(5.0, &mut hull, Some(&config));

        assert!(
            matches!(
                &teams.slots()[0],
                TeamSlot::Repairing { display_name: Some(d), .. } if d == "Helm Beta"
            ),
            "the swept-to slot must carry the hull entry's display name, not the \
             raw id `helm-b`; got {:?}",
            teams.slots()[0]
        );
    }

    /// A hull row that can never be progressed is not swept to, and — the point
    /// of the guard — two of them do not trap the team forever.
    ///
    /// A `max_hp = 0` row is `Destroyed` (`tier_for` tests `current == 0.0`
    /// first) AND at max HP at the same time, so it satisfies the sweep's damage
    /// predicate while `restore` can never change it. Before the `current < max`
    /// guard the team handed off from one ghost row to the other and back,
    /// forever: never finishing, never returning, and never releasing the slot
    /// for the next dispatch.
    #[test]
    fn sweep_skips_zero_max_hp_rows_and_still_finishes_the_real_work() {
        let mut teams = RepairTeams::new(1);
        let mut hull = hull_at(&[
            ("helm-a", 10.0, 2.0),      // real work: Disabled, repairable
            ("helm-ghost-a", 0.0, 0.0), // permanently Destroyed AND at max
            ("helm-ghost-b", 0.0, 0.0), // ditto — the pair is what livelocked
        ]);
        assert_eq!(
            hull.tier_for(&sid("helm-ghost-a")),
            DamageTier::Destroyed,
            "fixture precondition: a zero-max row reads Destroyed"
        );
        assert!(
            hull.is_at_max(&sid("helm-ghost-a")),
            "fixture precondition: it is simultaneously at max HP"
        );
        let config = config_with(&[
            ("helm-a", Some("helm")),
            ("helm-ghost-a", Some("helm")),
            ("helm-ghost-b", Some("helm")),
        ]);
        teams.dispatch(0, sid("helm-a"), "Helm A".to_string());

        let (visited, returned) = walk_sweep(&mut teams, &mut hull, Some(&config), 400);

        assert_eq!(
            visited,
            vec!["helm-a"],
            "the sweep must only pick work it can progress, got {visited:?}"
        );
        assert!(
            returned,
            "the team must go home once the repairable work is done rather than \
             alternating between the two unfinishable rows forever"
        );
        assert!(hull.is_at_max(&sid("helm-a")));
    }

    /// A dispatch that resolved to a STATION NAME rather than a hull row bounces
    /// exactly as it did before the sweep existed — it does not walk off into
    /// the ownerless `core` bucket.
    ///
    /// `resolve_repair_target` falls back to `SystemId(station_id)` when no
    /// system of the station is repairable. That id is untracked, so `is_at_max`
    /// answers the permissive `true` and the arrival lands on the sweep branch;
    /// `sweep_group` then buckets an id the config does not describe as
    /// OWNERLESS, which is `core`'s own group. Without the hull-row gate the team
    /// sent to `helm` would arrive and start repairing `core`.
    ///
    /// The COLLIDING case — a station name that is also a hull row, which would
    /// pass this gate — is now impossible at the source rather than caught here:
    /// `resolve_repair_target` produces the fallback only for a name the hull
    /// does not track and returns `None` otherwise, so no such dispatch is ever
    /// applied. This test pins the other half of that pair, the arrival that IS
    /// untracked.
    #[test]
    fn arrival_at_a_station_name_returns_instead_of_sweeping_the_core_bucket() {
        let mut teams = RepairTeams::new(1);
        let mut hull = hull_at(&[("core", 10.0, 5.0), ("helm-a", 10.0, 2.0)]);
        let config = config_with(&[("helm-a", Some("helm"))]);
        assert!(
            hull.get(&sid("helm")).is_none(),
            "fixture precondition: `helm` is a station name, not a hull row"
        );
        teams.dispatch(0, sid("helm"), "Helm".to_string());

        let (visited, returned) = walk_sweep(&mut teams, &mut hull, Some(&config), 400);

        assert!(
            visited.is_empty(),
            "a team that arrived at a non-hull id must repair nothing, got {visited:?}"
        );
        assert!(returned, "it must bounce straight back to Returning");
        assert_eq!(
            hull.current_for(&sid("core")),
            Some(5.0),
            "the ownerless core bucket must be untouched — it is not this team's \
             station and the arrival id never belonged to a group at all"
        );
    }

    /// A destroyed system is swept to like any other, and comes out the far
    /// side at full HP and Operational.
    #[test]
    fn sweep_repairs_a_destroyed_neighbour_back_to_operational() {
        let mut teams = RepairTeams::new(1);
        let mut hull = hull_at(&[("helm-a", 10.0, 2.0), ("helm-b", 10.0, 0.0)]);
        let config = config_with(&[("helm-a", Some("helm")), ("helm-b", Some("helm"))]);
        assert_eq!(hull.tier_for(&sid("helm-b")), DamageTier::Destroyed);
        teams.dispatch(0, sid("helm-a"), "Helm A".to_string());

        let (visited, returned) = walk_sweep(&mut teams, &mut hull, Some(&config), 400);

        assert_eq!(
            visited,
            vec!["helm-a", "helm-b"],
            "the team works the system it was actually sent to first — the \
             ranking only chooses among what is LEFT — and then sweeps on to \
             the Destroyed neighbour"
        );
        assert!(returned);
        assert!(hull.is_at_max(&sid("helm-b")));
        assert_eq!(hull.tier_for(&sid("helm-b")), DamageTier::Operational);
    }

    // ── Priority: the sweep's order selector (issue #1013) ────────────────────

    /// Set up a `helm` station with four systems and put team 0 on site at
    /// `helm-a`, so the remaining work ranks `helm-d` (Destroyed),
    /// `helm-c` (Disabled), `helm-b` (Damaged) worst-first.
    fn team_on_site_with_three_jobs_left() -> (RepairTeams, SystemHull, ShipConfig) {
        let mut teams = RepairTeams::new(1);
        let mut hull = hull_at(&[
            ("helm-a", 10.0, 7.0), // Damaged — the system the team is sent to
            ("helm-b", 10.0, 6.0), // Damaged, less hurt than helm-a
            ("helm-c", 10.0, 2.0), // Disabled
            ("helm-d", 10.0, 0.0), // Destroyed
        ]);
        let config = config_with(&[
            ("helm-a", Some("helm")),
            ("helm-b", Some("helm")),
            ("helm-c", Some("helm")),
            ("helm-d", Some("helm")),
        ]);
        teams.dispatch(0, sid("helm-a"), "Helm A".to_string());
        teams.tick(5.0, &mut hull, Some(&config));
        assert_eq!(
            repairing_at(&teams).as_deref(),
            Some("helm-a"),
            "fixture precondition: the team is on site at helm-a"
        );
        (teams, hull, config)
    }

    /// Finish the system team 0 is on (3 HP at most) and land on whatever the
    /// sweep picks next.
    fn finish_current_system(teams: &mut RepairTeams, hull: &mut SystemHull, config: &ShipConfig) {
        teams.tick(30.0, hull, Some(config));
    }

    /// No priority set → the sweep takes the worst remaining job.
    #[test]
    fn sweep_without_priority_takes_the_worst_remaining_job() {
        let (mut teams, mut hull, config) = team_on_site_with_three_jobs_left();
        finish_current_system(&mut teams, &mut hull, &config);
        assert_eq!(repairing_at(&teams).as_deref(), Some("helm-d"));
    }

    /// AC2: `priority` is READ by the sweep. The identical fixture, differing
    /// only in the priority the console set, sends the team to a different
    /// system — 1 to the worst, 2 to the second worst, 3 to the third.
    #[test]
    fn priority_selects_which_remaining_job_the_sweep_takes() {
        for (priority, expected) in [(1_u8, "helm-d"), (2, "helm-c"), (3, "helm-b")] {
            let (mut teams, mut hull, config) = team_on_site_with_three_jobs_left();
            assert!(teams.set_priority(0, priority));
            finish_current_system(&mut teams, &mut hull, &config);
            assert_eq!(
                repairing_at(&teams).as_deref(),
                Some(expected),
                "priority {priority} must select the {priority}-ranked remaining job"
            );
        }
    }

    /// `None` and `0` both mean "the worst job" — the ordinal is 1-based and
    /// clamped at the bottom, so a console that sends 0 does something sane.
    #[test]
    fn priority_zero_means_the_worst_job_like_no_priority_at_all() {
        let (mut teams, mut hull, config) = team_on_site_with_three_jobs_left();
        assert!(teams.set_priority(0, 0));
        finish_current_system(&mut teams, &mut hull, &config);
        assert_eq!(repairing_at(&teams).as_deref(), Some("helm-d"));
    }

    /// A priority past the end of the remaining work clamps to the last job
    /// rather than stranding the team.
    #[test]
    fn priority_beyond_the_remaining_jobs_clamps_to_the_last_one() {
        let (mut teams, mut hull, config) = team_on_site_with_three_jobs_left();
        assert!(teams.set_priority(0, 9));
        finish_current_system(&mut teams, &mut hull, &config);
        assert_eq!(repairing_at(&teams).as_deref(), Some("helm-b"));
    }

    /// AC2, the live version: changing the priority MID-SWEEP re-orders the work
    /// the team has left. The first hand-off takes the worst job (no priority);
    /// the console then taps priority 2, and the second hand-off takes the
    /// second-worst of what remains instead of the worst.
    #[test]
    fn priority_change_mid_sweep_reorders_the_remaining_work() {
        let (mut teams, mut hull, config) = team_on_site_with_three_jobs_left();

        finish_current_system(&mut teams, &mut hull, &config);
        assert_eq!(
            repairing_at(&teams).as_deref(),
            Some("helm-d"),
            "first hand-off, no priority: the worst job"
        );

        assert!(teams.set_priority(0, 2));
        finish_current_system(&mut teams, &mut hull, &config);
        assert_eq!(
            repairing_at(&teams).as_deref(),
            Some("helm-b"),
            "with priority 2 the team must take the SECOND worst of the \
             remaining {{helm-c, helm-b}}, not helm-c"
        );
    }

    /// The priority is a standing instruction about the station, so it survives
    /// the sweep's in-place system hand-off instead of resetting to `None`.
    #[test]
    fn priority_survives_the_sweep_hand_off() {
        let (mut teams, mut hull, config) = team_on_site_with_three_jobs_left();
        assert!(teams.set_priority(0, 2));
        finish_current_system(&mut teams, &mut hull, &config);
        assert!(
            matches!(
                &teams.slots()[0],
                TeamSlot::Repairing {
                    priority: Some(2),
                    ..
                }
            ),
            "got {:?}",
            teams.slots()[0]
        );
    }

    // ── Naming a system instead of an ordinal (issue #1015) ───────────────────
    //
    // The repair console's damaged-systems taps name a SYSTEM; the host pins
    // that system on the team's slot and leaves the ordinal untouched. Every
    // test here therefore asserts on the same observable the #1013 tests do
    // — where the team actually goes at the hand-off — plus the pin the
    // console highlights.

    /// The headline: tapping the third-ranked job sends the team there next,
    /// with no ordinal anywhere near the caller.
    #[test]
    fn prioritise_system_sends_the_sweep_to_the_named_system() {
        let (mut teams, mut hull, config) = team_on_site_with_three_jobs_left();

        assert_eq!(
            teams.prioritise_system(&sid("helm-b"), &hull, &config),
            Some(0),
            "team 0 is the one sweeping helm, so it takes the order"
        );
        finish_current_system(&mut teams, &mut hull, &config);

        assert_eq!(
            repairing_at(&teams).as_deref(),
            Some("helm-b"),
            "the tapped system must be the next job, not the worst one"
        );
    }

    /// A tap stores the SYSTEM and nothing else. It deliberately does NOT
    /// resolve to an ordinal: `priority` is #1013's standing per-team
    /// instruction, and a tap is a one-shot choice about one row, so the two
    /// never write to the same place. (`the_pin_beats_a_stale_ordinal_after_a_re_rank`
    /// below is why storing a rank here would be wrong and not merely redundant.)
    #[test]
    fn prioritise_system_stores_the_pin_and_never_an_ordinal() {
        for target in ["helm-d", "helm-c", "helm-b"] {
            let (mut teams, hull, config) = team_on_site_with_three_jobs_left();
            assert_eq!(
                teams.prioritise_system(&sid(target), &hull, &config),
                Some(0)
            );
            assert!(
                matches!(
                    &teams.slots()[0],
                    TeamSlot::Repairing {
                        priority: None,
                        priority_system_id: Some(pinned),
                        ..
                    } if pinned.0 == target
                ),
                "{target} must be pinned with no ordinal written, got {:?}",
                teams.slots()[0]
            );
        }
    }

    /// A tap leaves an existing standing ordinal alone — the two levers are
    /// independent, and the pin simply outranks the ordinal while it lasts.
    #[test]
    fn prioritise_system_does_not_disturb_a_standing_ordinal() {
        let (mut teams, hull, config) = team_on_site_with_three_jobs_left();
        assert!(teams.set_priority(0, 3));
        teams.prioritise_system(&sid("helm-c"), &hull, &config);
        assert!(
            matches!(
                &teams.slots()[0],
                TeamSlot::Repairing {
                    priority: Some(3),
                    priority_system_id: Some(pinned),
                    ..
                } if pinned.0 == "helm-c"
            ),
            "got {:?}",
            teams.slots()[0]
        );
    }

    /// The console's highlight: the resolved slot echoes WHICH system the
    /// host pinned, because the client cannot re-derive it (issue #737 hides
    /// most of the candidates from it).
    #[test]
    fn prioritise_system_echoes_the_pinned_system_for_the_console() {
        let (mut teams, hull, config) = team_on_site_with_three_jobs_left();
        teams.prioritise_system(&sid("helm-c"), &hull, &config);
        assert!(
            matches!(
                &teams.slots()[0],
                TeamSlot::Repairing { priority_system_id: Some(s), .. } if s.0 == "helm-c"
            ),
            "got {:?}",
            teams.slots()[0]
        );
    }

    /// The pin describes one hand-off and is spent by it — otherwise the
    /// console would keep highlighting a row the team has already arrived at.
    /// The ORDINAL survives untouched, because #1013 makes that a standing
    /// instruction about the station; here it is deliberately set to something
    /// the pin overrules (3 would take `helm-b`), so the assertion that the team
    /// landed on `helm-c` also proves which of the two levers won.
    #[test]
    fn the_priority_pin_clears_at_the_hand_off_but_the_ordinal_does_not() {
        let (mut teams, mut hull, config) = team_on_site_with_three_jobs_left();
        assert!(teams.set_priority(0, 3));
        teams.prioritise_system(&sid("helm-c"), &hull, &config);
        finish_current_system(&mut teams, &mut hull, &config);

        assert_eq!(repairing_at(&teams).as_deref(), Some("helm-c"));
        assert!(
            matches!(
                &teams.slots()[0],
                TeamSlot::Repairing {
                    priority: Some(3),
                    priority_system_id: None,
                    ..
                }
            ),
            "got {:?}",
            teams.slots()[0]
        );
    }

    /// The finding this design exists for: a tap and the hand-off it steers are
    /// separated by however long the current system takes to finish, and combat
    /// damage in between RE-RANKS the group.
    ///
    /// The standing ordinal is set to 3 first, so the pin and the ordinal
    /// fallback disagree instead of coincidentally landing on the same row:
    /// `helm-b` is tapped while it ranks third. It is then blown to Destroyed,
    /// which makes it rank FIRST (tied with `helm-d` on tier and fraction,
    /// winning on the id tiebreak) and pushes `helm-c` into third — exactly
    /// what the stale ordinal 3 would now select. The pin sends the team to
    /// `helm-b` instead, which is the row the player actually asked for, so
    /// the assertion below discriminates the pin from the ordinal fallback
    /// rather than passing either way.
    #[test]
    fn the_pin_beats_a_stale_ordinal_after_a_re_rank() {
        let (mut teams, mut hull, config) = team_on_site_with_three_jobs_left();
        assert!(teams.set_priority(0, 3));
        assert_eq!(
            teams.prioritise_system(&sid("helm-b"), &hull, &config),
            Some(0)
        );

        // Fresh damage between the tap and the hand-off.
        hull.set_hp(&sid("helm-b"), 0.0);
        assert_eq!(hull.tier_for(&sid("helm-b")), DamageTier::Destroyed);
        // What the stale ordinal 3 would now select, spelled out so the test
        // fails loudly if the fixture's ranking ever shifts under it.
        let ranked: Vec<String> = sweep_candidates(&sid("helm-a"), &hull, &config, &[])
            .into_iter()
            .map(|(s, _)| s.0)
            .collect();
        assert_eq!(ranked, vec!["helm-b", "helm-d", "helm-c"]);

        finish_current_system(&mut teams, &mut hull, &config);
        assert_eq!(
            repairing_at(&teams).as_deref(),
            Some("helm-b"),
            "the pinned row must win, not whatever now sits at its old rank"
        );
    }

    /// The pin's only failure mode: the row it names stops being candidate work
    /// before the hand-off. Then — and only then — the standing ordinal decides,
    /// rather than the team stranding itself waiting for a job that is gone.
    #[test]
    fn a_pin_that_leaves_the_candidate_list_falls_back_to_the_ordinal() {
        let (mut teams, mut hull, config) = team_on_site_with_three_jobs_left();
        assert!(teams.set_priority(0, 2));
        assert_eq!(
            teams.prioritise_system(&sid("helm-b"), &hull, &config),
            Some(0)
        );

        // Somebody else finished helm-b, so it is Operational and at max —
        // failing both halves of the candidate test.
        hull.set_hp(&sid("helm-b"), 10.0);

        finish_current_system(&mut teams, &mut hull, &config);
        assert_eq!(
            repairing_at(&teams).as_deref(),
            Some("helm-c"),
            "with the pin gone, ordinal 2 must pick the second of the remaining \
             {{helm-d, helm-c}}"
        );
    }

    /// A tap on a system ANOTHER team is standing on is refused outright, and
    /// crucially does not fall through: before the on-site exclusion existed,
    /// team 0's own system was never team 0's own candidate, so the search moved
    /// on and pinned it on team 1 — pointing a second team at a row already
    /// being worked while the rest of the station waited.
    #[test]
    fn prioritise_system_refuses_a_tap_on_a_system_another_team_is_on_site_at() {
        let mut teams = RepairTeams::new(2);
        let mut hull = hull_at(&[
            ("helm-a", 10.0, 7.0),
            ("helm-b", 10.0, 6.0),
            ("helm-c", 10.0, 2.0),
        ]);
        let config = config_with(&[
            ("helm-a", Some("helm")),
            ("helm-b", Some("helm")),
            ("helm-c", Some("helm")),
        ]);
        teams.dispatch(0, sid("helm-a"), "Helm A".to_string());
        teams.dispatch(1, sid("helm-b"), "Helm B".to_string());
        teams.tick(5.0, &mut hull, Some(&config));

        assert_eq!(
            teams.prioritise_system(&sid("helm-a"), &hull, &config),
            None,
            "team 0's own system is nobody's next job"
        );
        assert!(
            teams.slots().iter().all(|s| matches!(
                s,
                TeamSlot::Repairing {
                    priority_system_id: None,
                    ..
                }
            )),
            "no team may be pinned by a refused tap, got {:?}",
            teams.slots()
        );
    }

    /// The same exclusion protecting #1013's own hand-off: a team that finishes
    /// its system does not walk onto the one its crewmate is standing on. With
    /// only those two systems damaged it has nothing left and goes home.
    #[test]
    fn a_sweep_hand_off_does_not_converge_on_another_teams_system() {
        let mut teams = RepairTeams::new(2);
        let mut hull = hull_at(&[
            ("helm-a", 10.0, 9.5), // team 0: nearly done
            ("helm-b", 10.0, 2.0), // team 1: a long job
        ]);
        let config = config_with(&[("helm-a", Some("helm")), ("helm-b", Some("helm"))]);
        teams.dispatch(0, sid("helm-a"), "Helm A".to_string());
        teams.dispatch(1, sid("helm-b"), "Helm B".to_string());
        teams.tick(5.0, &mut hull, Some(&config));
        assert_eq!(repairing_at(&teams).as_deref(), Some("helm-a"));

        // Long enough for team 0 to finish helm-a, nowhere near long enough for
        // team 1 to finish helm-b.
        teams.tick(2.0, &mut hull, Some(&config));

        assert!(
            matches!(&teams.slots()[0], TeamSlot::Returning { .. }),
            "team 0 must head home rather than pile onto helm-b, got {:?}",
            teams.slots()[0]
        );
        assert!(
            matches!(
                &teams.slots()[1],
                TeamSlot::Repairing { system_id: Some(s), .. } if s.0 == "helm-b"
            ),
            "team 1 keeps its job, got {:?}",
            teams.slots()[1]
        );
    }

    /// A tap on the system the team is already working on is not a candidate —
    /// `next_sweep_target` excludes the current system — so it changes nothing
    /// rather than resolving to some neighbouring rank.
    #[test]
    fn prioritise_system_ignores_a_tap_on_the_system_under_repair() {
        let (mut teams, hull, config) = team_on_site_with_three_jobs_left();
        assert_eq!(
            teams.prioritise_system(&sid("helm-a"), &hull, &config),
            None
        );
        assert!(
            matches!(
                &teams.slots()[0],
                TeamSlot::Repairing {
                    priority: None,
                    priority_system_id: None,
                    ..
                }
            ),
            "got {:?}",
            teams.slots()[0]
        );
    }

    /// A tap on another station's system finds no team standing in that group
    /// and is refused. The sweep is station-bounded, so "prioritise it" has no
    /// meaning until somebody is dispatched there.
    #[test]
    fn prioritise_system_refuses_a_system_outside_every_teams_sweep_group() {
        let mut teams = RepairTeams::new(1);
        let mut hull = station_hull();
        let config = station_config();
        teams.dispatch(0, sid("helm-b"), "Helm B".to_string());
        teams.tick(5.0, &mut hull, Some(&config));
        assert_eq!(repairing_at(&teams).as_deref(), Some("helm-b"));

        assert_eq!(
            teams.prioritise_system(&sid("tactical-x"), &hull, &config),
            None,
            "no team is sweeping `tactical`, so there is nothing to re-order"
        );
        assert_eq!(
            teams.prioritise_system(&sid("core"), &hull, &config),
            None,
            "the ownerless bucket is its own group, and nobody is in it"
        );
    }

    /// A team that has not arrived yet has no sweep to steer: `Travelling`
    /// carries a `priority` field but no candidate list, exactly as
    /// `set_priority` already refuses it.
    #[test]
    fn prioritise_system_refuses_a_team_that_is_still_travelling() {
        let mut teams = RepairTeams::new(1);
        let hull = station_hull();
        let config = station_config();
        teams.dispatch(0, sid("helm-c"), "Helm C".to_string());

        assert_eq!(
            teams.prioritise_system(&sid("helm-a"), &hull, &config),
            None
        );
    }

    /// Two teams in the same group: the order goes to the lowest slot index, so
    /// the outcome is a pure function of state rather than of iteration luck.
    #[test]
    fn prioritise_system_gives_a_shared_group_to_the_lowest_team_index() {
        let mut teams = RepairTeams::new(2);
        let mut hull = hull_at(&[
            ("helm-a", 10.0, 7.0),
            ("helm-b", 10.0, 6.0),
            ("helm-c", 10.0, 2.0),
        ]);
        let config = config_with(&[
            ("helm-a", Some("helm")),
            ("helm-b", Some("helm")),
            ("helm-c", Some("helm")),
        ]);
        teams.dispatch(0, sid("helm-a"), "Helm A".to_string());
        teams.dispatch(1, sid("helm-b"), "Helm B".to_string());
        teams.tick(5.0, &mut hull, Some(&config));

        assert_eq!(
            teams.prioritise_system(&sid("helm-c"), &hull, &config),
            Some(0)
        );
        assert!(
            matches!(
                &teams.slots()[1],
                TeamSlot::Repairing {
                    priority_system_id: None,
                    ..
                }
            ),
            "team 1 must be untouched, got {:?}",
            teams.slots()[1]
        );
    }

    /// The ownerless bucket is steerable too — that is the "fix the hull first"
    /// case the playtest could not express, and core rows are the ones
    /// Engineering can always see.
    #[test]
    fn prioritise_system_works_inside_the_ownerless_core_bucket() {
        let mut teams = RepairTeams::new(1);
        let mut hull = hull_at(&[
            ("core", 10.0, 5.0),         // Damaged — where the team is
            ("aux-sensor", 10.0, 0.0),   // Destroyed — ranks first
            ("hull-plating", 10.0, 4.0), // Damaged — ranks second
        ]);
        let config = config_with(&[("aux-sensor", None), ("hull-plating", None)]);
        teams.dispatch(0, sid("core"), "Core".to_string());
        teams.tick(5.0, &mut hull, Some(&config));
        assert_eq!(repairing_at(&teams).as_deref(), Some("core"));

        assert_eq!(
            teams.prioritise_system(&sid("hull-plating"), &hull, &config),
            Some(0)
        );
        finish_current_system(&mut teams, &mut hull, &config);
        assert_eq!(
            repairing_at(&teams).as_deref(),
            Some("hull-plating"),
            "the tapped core row must beat the Destroyed one that outranks it"
        );
    }

    /// The tier key dominates the damage-fraction key. `helm-disabled` and
    /// `helm-damaged` are a rounding error apart in HP — 2.4 and 2.6 of 10 — but
    /// they land either side of the 0.25 Disabled threshold, and the worse tier
    /// wins even though the Damaged one is barely less hurt.
    #[test]
    fn sweep_prefers_a_worse_tier_over_a_larger_damage_fraction() {
        let config = config_with(&[
            ("helm-here", Some("helm")),
            ("helm-disabled", Some("helm")),
            ("helm-damaged", Some("helm")),
        ]);
        let hull = hull_at(&[
            ("helm-here", 10.0, 5.0),
            ("helm-disabled", 10.0, 2.4), // 0.24 → Disabled, fraction 0.76
            ("helm-damaged", 10.0, 2.6),  // 0.26 → Damaged, fraction 0.74
        ]);
        assert_eq!(hull.tier_for(&sid("helm-disabled")), DamageTier::Disabled);
        assert_eq!(hull.tier_for(&sid("helm-damaged")), DamageTier::Damaged);

        let (winner, _) =
            next_sweep_target(&sid("helm-here"), None, None, &hull, &config, &[]).unwrap();
        assert_eq!(winner.0, "helm-disabled");
    }

    /// Within one tier, the larger damage fraction goes first.
    #[test]
    fn sweep_prefers_the_larger_damage_fraction_within_a_tier() {
        let config = config_with(&[
            ("helm-here", Some("helm")),
            ("helm-worse", Some("helm")),
            ("helm-better", Some("helm")),
        ]);
        let hull = hull_at(&[
            ("helm-here", 10.0, 5.0),
            ("helm-worse", 10.0, 3.0),  // fraction 0.70
            ("helm-better", 10.0, 7.0), // fraction 0.30
        ]);
        assert_eq!(hull.tier_for(&sid("helm-worse")), DamageTier::Damaged);
        assert_eq!(hull.tier_for(&sid("helm-better")), DamageTier::Damaged);

        let (winner, _) =
            next_sweep_target(&sid("helm-here"), None, None, &hull, &config, &[]).unwrap();
        assert_eq!(winner.0, "helm-worse");
    }

    /// A full tie resolves to the smallest system id, so hull iteration order
    /// cannot reach the decision — a repair choice feeds the sim digest like
    /// any other. `helm-tie-b` is declared FIRST in both the hull and the
    /// config, so an order-sensitive comparator would pick it.
    #[test]
    fn sweep_breaks_a_full_tie_on_the_smallest_system_id() {
        let config = config_with(&[
            ("helm-here", Some("helm")),
            ("helm-tie-b", Some("helm")),
            ("helm-tie-a", Some("helm")),
        ]);
        let hull = hull_at(&[
            ("helm-here", 10.0, 5.0),
            ("helm-tie-b", 10.0, 5.0),
            ("helm-tie-a", 10.0, 5.0),
        ]);

        let (first, _) =
            next_sweep_target(&sid("helm-here"), None, None, &hull, &config, &[]).unwrap();
        assert_eq!(first.0, "helm-tie-a");
        let (second, _) =
            next_sweep_target(&sid("helm-here"), Some(2), None, &hull, &config, &[]).unwrap();
        assert_eq!(second.0, "helm-tie-b");
    }

    /// Nothing left at the station → no sweep target, which is what puts the
    /// team on the road home.
    #[test]
    fn next_sweep_target_is_none_when_the_station_is_clean() {
        let config = config_with(&[("helm-here", Some("helm")), ("helm-other", Some("helm"))]);
        let hull = hull_at(&[("helm-here", 10.0, 5.0), ("helm-other", 10.0, 10.0)]);
        assert!(next_sweep_target(&sid("helm-here"), None, None, &hull, &config, &[]).is_none());
    }

    // ── Display-name propagation (regression for reviewer's #617 finding) ──

    /// Dispatch must record the caller-supplied `display_name` on the
    /// resulting `TeamSlot::Travelling`. Regression for the reviewer's
    /// finding on issue #617 that dispatch was regressing display_name to
    /// the raw SystemId string ("helm-engine-port") instead of the
    /// human-readable label ("Engine (Port)") that the pre-#617
    /// `derive_system_fields(&Console)` helper produced.
    #[test]
    fn dispatch_records_supplied_display_name_on_travelling_slot() {
        let mut teams = RepairTeams::new(2);
        teams.dispatch(0, sid("helm"), "Helm".to_string());
        let slot = &teams.slots()[0];
        assert!(
            matches!(
                slot,
                TeamSlot::Travelling { display_name: Some(d), .. }
                    if d == "Helm"
            ),
            "team 0 must be Travelling with display_name = Some(\"Helm\"), got {slot:?}"
        );
    }

    // ── set_priority ─────────────────────────────────────────────────────────

    #[test]
    fn set_priority_repairing_team_sets_priority() {
        let mut teams = RepairTeams::new(2);
        let mut hull = hull_damaged(10.0);
        teams.dispatch(0, sid("helm"), "Helm".to_string());
        teams.tick(5.0, &mut hull, None); // travel → Repairing
        assert!(teams.set_priority(0, 3));
        let slot = &teams.slots()[0];
        assert!(matches!(
            slot,
            TeamSlot::Repairing {
                priority: Some(3),
                ..
            }
        ));
    }

    #[test]
    fn set_priority_idle_team_returns_false() {
        let mut teams = RepairTeams::new(2);
        assert!(!teams.set_priority(0, 3));
    }

    #[test]
    fn set_priority_out_of_range_index_returns_false() {
        let mut teams = RepairTeams::new(1);
        assert!(!teams.set_priority(5, 3));
    }

    #[test]
    fn set_priority_travelling_team_returns_false() {
        let mut teams = RepairTeams::new(2);
        teams.dispatch(0, sid("helm"), "Helm".to_string());
        assert!(!teams.set_priority(0, 3));
    }

    #[test]
    fn set_priority_returning_team_returns_false() {
        let mut teams = RepairTeams::new(1);
        let mut hull = hull_full();
        teams.dispatch(0, sid("helm"), "Helm".to_string());
        teams.tick(5.0, &mut hull, None);
        // After arriving at full hull the team becomes Returning.
        assert!(!teams.set_priority(0, 3));
    }

    /// The caller-supplied display name must survive the
    /// `Travelling → Repairing` transition (regression guard for the
    /// clobber inside `tick()`).
    #[test]
    fn tick_preserves_display_name_through_travelling_to_repairing() {
        let mut teams = RepairTeams::new(1);
        let mut hull = hull_damaged(10.0);
        teams.dispatch(0, sid("helm"), "Helm".to_string());
        teams.tick(5.0, &mut hull, None); // travel → Repairing
        let slot = &teams.slots()[0];
        assert!(
            matches!(
                slot,
                TeamSlot::Repairing { display_name: Some(d), .. }
                    if d == "Helm"
            ),
            "team 0 must be Repairing with display_name preserved as \
             Some(\"Helm\"), got {slot:?}"
        );
    }
}
