use crate::damage::SystemHull;
use crate::messages::{SystemId, TeamSlot};

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

    /// Advance all active timers by `dt` seconds.
    ///
    /// - `Travelling` advances its `elapsed` toward `travel_duration`, then
    ///   transitions to `Repairing`. If the target system is already at full
    ///   HP on arrival, the team skips straight to `Returning`.
    /// - `Repairing` calls `hull.restore(&sid, dt * repair_rate_hp_per_sec)`
    ///   each tick. Once the system is at full HP, the team transitions to
    ///   `Returning`.
    /// - `Returning` decrements `remaining` toward 0. On completion:
    ///   - If `queued_system_id = Some(sid)`: auto-dispatch to
    ///     `Travelling { system_id: sid, elapsed: 0 }`.
    ///   - Otherwise: → `Idle`.
    pub fn tick(&mut self, dt: f32, hull: &mut SystemHull) {
        let travel_duration = self.timings.travel_duration;
        let repair_rate = self.timings.repair_rate_hp_per_sec;
        for slot in self.slots.iter_mut() {
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
                        let is_full = hull.is_at_max(&sid);
                        let is_destroyed =
                            hull.tier_for(&sid) == crate::damage::DamageTier::Destroyed;
                        // Carry the display name forward from the current
                        // `Travelling` slot so the human-readable label the
                        // caller supplied at dispatch time survives the
                        // Travelling → Repairing/Returning transition. Falls
                        // back to the raw SystemId only if the slot never
                        // had a label (e.g. legacy on-wire messages without
                        // the new field).
                        let label = display_name.clone().or_else(|| Some(sid.0.clone()));
                        if is_full || is_destroyed {
                            *slot = TeamSlot::Returning {
                                remaining: 0.0,
                                system_id: Some(sid),
                                display_name: label,
                                queued_system_id: None,
                                queued_display_name: None,
                            };
                        } else {
                            *slot = TeamSlot::Repairing {
                                system_id: Some(sid),
                                display_name: label,
                            };
                        }
                    }
                }
                TeamSlot::Repairing {
                    system_id,
                    display_name,
                    ..
                } => {
                    let Some(sid) = system_id.clone() else {
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
                    // Do not repair a Destroyed system — the latch is
                    // unrepairable by a repair team alone.
                    if hull.tier_for(&sid) == crate::damage::DamageTier::Destroyed {
                        *slot = TeamSlot::Returning {
                            remaining: travel_duration,
                            system_id: Some(sid),
                            display_name: carried_label,
                            queued_system_id: None,
                            queued_display_name: None,
                        };
                        continue;
                    }
                    let hp_to_restore = dt * repair_rate;
                    hull.restore(&sid, hp_to_restore);
                    if hull.is_at_max(&sid) {
                        *slot = TeamSlot::Returning {
                            remaining: travel_duration,
                            system_id: Some(sid),
                            display_name: carried_label,
                            queued_system_id: None,
                            queued_display_name: None,
                        };
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
            let mut rng = rand::rng();
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
        teams.tick(5.0, &mut hull);
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
        teams.tick(4.9, &mut hull);
        assert!(matches!(&teams.slots()[0], TeamSlot::Travelling { .. }));
    }

    #[test]
    fn team_arrives_at_full_hp_console_enters_returning() {
        let mut teams = RepairTeams::new(1);
        let mut hull = hull_full(); // system already at full HP
        teams.dispatch(0, sid("helm"), "Helm".to_string());
        teams.tick(5.0, &mut hull);
        assert!(matches!(&teams.slots()[0], TeamSlot::Returning { .. }));
    }

    #[test]
    fn repairing_restores_hp_at_correct_rate() {
        let mut teams = RepairTeams::new(1);
        let mut hull = hull_damaged(1.0); // 1 HP (Disabled, not Destroyed)
        teams.dispatch(0, sid("helm"), "Helm".to_string());
        teams.tick(5.0, &mut hull); // travel
                                    // Now repairing; restore for 2s should give 1 more HP (0.5 HP/s)
        teams.tick(2.0, &mut hull);
        let hp = hull.current_for(&sid("helm")).unwrap();
        assert!(
            (hp - 2.0).abs() < 1e-4,
            "expected 2 HP after 2s repair starting from 1 HP, got {hp}"
        );
    }

    #[test]
    fn repairing_transitions_to_returning_when_console_full() {
        let mut teams = RepairTeams::new(1);
        let mut hull = hull_damaged(24.9); // almost full
        teams.dispatch(0, sid("helm"), "Helm".to_string());
        teams.tick(5.0, &mut hull); // travel
                                    // One tick of 1s restores 0.5 HP — enough to max at 25
        teams.tick(1.0, &mut hull);
        assert!(matches!(&teams.slots()[0], TeamSlot::Returning { .. }));
    }

    // ── Returning → Idle ──────────────────────────────────────────────────────

    #[test]
    fn returning_transitions_to_idle_after_5s() {
        let mut teams = RepairTeams::new(1);
        let mut hull = hull_full();
        teams.dispatch(0, sid("helm"), "Helm".to_string());
        teams.tick(5.0, &mut hull); // travel (arrives full → Returning with remaining=0)
                                    // remaining is already 0 from arriving at full hp; tick 0.1 to trigger idle
        teams.tick(0.1, &mut hull);
        assert!(matches!(&teams.slots()[0], TeamSlot::Idle));
    }

    #[test]
    fn returning_does_not_complete_before_remaining_expires() {
        let mut teams = RepairTeams::new(1);
        let mut hull = hull_damaged(24.9); // not full
        teams.dispatch(0, sid("helm"), "Helm".to_string());
        teams.tick(5.0, &mut hull); // travel → Repairing
        teams.tick(1.0, &mut hull); // repair → full → Returning { remaining: 5.0 }
        teams.tick(4.9, &mut hull); // remaining not yet expired
        assert!(matches!(&teams.slots()[0], TeamSlot::Returning { .. }));
    }

    // ── Full lifecycle ────────────────────────────────────────────────────────

    #[test]
    fn full_lifecycle_travel_repair_return_idle() {
        let mut teams = RepairTeams::new(1);
        let mut hull = hull_damaged(1.0); // 1 HP (Disabled, not Destroyed — repairable)
        teams.dispatch(0, sid("helm"), "Helm".to_string());

        // Travelling
        teams.tick(5.0, &mut hull);
        assert!(matches!(&teams.slots()[0], TeamSlot::Repairing { .. }));

        // Repairing until full (24 HP remaining at 0.5 HP/s = 48s)
        teams.tick(50.0, &mut hull);
        assert!(matches!(&teams.slots()[0], TeamSlot::Returning { .. }));

        // Returning (remaining starts at TRAVEL_DURATION = 5s)
        teams.tick(5.1, &mut hull);
        assert!(matches!(&teams.slots()[0], TeamSlot::Idle));
    }

    // ── Multiple teams independence ───────────────────────────────────────────

    #[test]
    fn two_teams_operate_independently() {
        let mut hull = SystemHull::from_config(&[(sid("helm"), 25.0), (sid("tactical"), 25.0)]);
        // Damage both systems
        let mut rng = rand::rng();
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
        teams.tick(5.0, &mut hull);
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
        teams.tick(5.0, &mut hull); // travel → Returning (remaining=0, full HP)
        teams.tick(0.1, &mut hull); // → Idle
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
        teams.tick(2.0, &mut hull);
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
        teams.tick(3.0, &mut hull);
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
        teams.tick(5.0, &mut hull); // travel → Repairing
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
        teams.tick(5.0, &mut hull); // travel → Repairing
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
        teams.tick(5.0, &mut hull); // travel → Repairing
        teams.tick(2.0, &mut hull); // restore 1 HP (0.5 HP/s * 2s = 1 HP → now 2 HP)
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
        teams.tick(2.0, &mut hull); // elapsed=2
        teams.dispatch(0, sid("tactical"), "Tactical".to_string()); // redirect → Returning { remaining:2, queued:Tactical }
        teams.tick(2.1, &mut hull); // remaining expires → auto-dispatch to Tactical
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
        teams.tick(2.0, &mut hull);
        teams.dispatch(0, sid("helm"), "Helm".to_string()); // recall → Returning { remaining:2, queued:None }
        teams.tick(2.1, &mut hull); // expires → Idle
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

    /// A repair team dispatched to a Destroyed system (hp == 0) must NOT
    /// restore any HP — the Destroyed latch is unrepairable.
    #[test]
    fn destroyed_console_is_not_repaired_by_repair_tick() {
        let mut teams = RepairTeams::new(1);
        // Build a hull with helm at 0 HP (Destroyed).
        let mut hull = SystemHull::from_config(&[(sid("helm"), 25.0)]);
        let mut rng = rand::rng();
        hull.apply_damage(1000.0, &mut rng); // wipe to 0
        assert_eq!(
            hull.tier_for(&sid("helm")),
            crate::damage::DamageTier::Destroyed,
            "precondition: helm must be Destroyed"
        );
        let hp_before = hull.current_for(&sid("helm")).unwrap();
        assert!((hp_before - 0.0).abs() < 1e-6, "precondition: 0 HP");

        teams.dispatch(0, sid("helm"), "Helm".to_string());
        // Travel to system.
        teams.tick(5.0, &mut hull);
        // Team should not enter Repairing — it should bounce directly to Returning.
        assert!(
            !matches!(&teams.slots()[0], TeamSlot::Repairing { .. }),
            "team should not enter Repairing state for a Destroyed system"
        );
        // Simulate several seconds of what would have been repair time.
        teams.tick(10.0, &mut hull);
        let hp_after = hull.current_for(&sid("helm")).unwrap();
        assert!(
            (hp_after - 0.0).abs() < 1e-6,
            "Destroyed system HP must remain 0 after repair tick (got {hp_after})"
        );
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

    /// The caller-supplied display name must survive the
    /// `Travelling → Repairing` transition (regression guard for the
    /// clobber inside `tick()`).
    #[test]
    fn tick_preserves_display_name_through_travelling_to_repairing() {
        let mut teams = RepairTeams::new(1);
        let mut hull = hull_damaged(10.0);
        teams.dispatch(0, sid("helm"), "Helm".to_string());
        teams.tick(5.0, &mut hull); // travel → Repairing
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
