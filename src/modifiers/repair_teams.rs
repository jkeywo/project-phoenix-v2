use crate::damage::ConsoleHull;
use crate::messages::{Console, TeamSlot};

/// Seconds a team spends travelling to a console (or returning from one).
const TRAVEL_DURATION: f32 = 5.0;
/// HP restored per second while the team is at the console.
const REPAIR_RATE_HP_PER_SEC: f32 = 0.5; // 1 HP per 2 seconds

/// Pure state machine for all repair teams on the ship.
///
/// Teams are identified by slot index. The number of teams is set at
/// construction time from the ship config (`repair_team_count`).
#[derive(Debug, Clone)]
pub struct RepairTeams {
    slots: Vec<TeamSlot>,
}

impl RepairTeams {
    /// Create a new `RepairTeams` with `count` teams, all idle.
    pub fn new(count: usize) -> Self {
        Self {
            slots: vec![TeamSlot::Idle; count],
        }
    }

    /// Borrow the full slot slice.
    pub fn slots(&self) -> &[TeamSlot] {
        &self.slots
    }

    /// Returns the index of the lowest-numbered idle team, or `None` if all are busy.
    pub fn lowest_free_team(&self) -> Option<usize> {
        self.slots.iter().position(|s| matches!(s, TeamSlot::Idle))
    }

    /// Dispatch the team at `team_idx` to `console`.
    ///
    /// Transitions `Idle → Travelling { console, elapsed: 0.0 }`.
    /// No-op if the slot is not `Idle` (in-progress teams cannot be redirected).
    pub fn dispatch(&mut self, team_idx: usize, console: Console) {
        if let Some(slot) = self.slots.get_mut(team_idx) {
            if matches!(slot, TeamSlot::Idle) {
                *slot = TeamSlot::Travelling { console, elapsed: 0.0 };
            }
        }
    }

    /// Advance all active timers by `dt` seconds.
    ///
    /// - `Travelling` advances its `elapsed` toward `TRAVEL_DURATION` (5s), then
    ///   transitions to `Repairing { elapsed: 0.0 }`. If the target console is
    ///   already at full HP on arrival, the team skips straight to `Returning`.
    /// - `Repairing` calls `hull.restore(console, dt * REPAIR_RATE_HP_PER_SEC)` each
    ///   tick. Once the console is at full HP, the team transitions to `Returning`.
    /// - `Returning` advances its `elapsed` toward `TRAVEL_DURATION` (5s), then
    ///   transitions to `Idle`.
    pub fn tick(&mut self, dt: f32, hull: &mut ConsoleHull) {
        for slot in self.slots.iter_mut() {
            match slot {
                TeamSlot::Travelling { console, elapsed } => {
                    *elapsed += dt;
                    if *elapsed >= TRAVEL_DURATION {
                        let console = console.clone();
                        // Check if console is already at full HP.
                        let is_full = hull.is_at_max(&console);
                        if is_full {
                            *slot = TeamSlot::Returning { elapsed: 0.0 };
                        } else {
                            *slot = TeamSlot::Repairing { console, elapsed: 0.0 };
                        }
                    }
                }
                TeamSlot::Repairing { console, elapsed } => {
                    let hp_to_restore = dt * REPAIR_RATE_HP_PER_SEC;
                    hull.restore(console.clone(), hp_to_restore);
                    *elapsed += dt;
                    // Transition to Returning when the console is fully repaired.
                    if hull.is_at_max(console) {
                        *slot = TeamSlot::Returning { elapsed: 0.0 };
                    }
                }
                TeamSlot::Returning { elapsed } => {
                    *elapsed += dt;
                    if *elapsed >= TRAVEL_DURATION {
                        *slot = TeamSlot::Idle;
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
    use crate::messages::Console;

    fn hull_with_helm(max_hp: f32) -> ConsoleHull {
        ConsoleHull::from_config(&[(Console::Helm, max_hp)])
    }

    fn hull_full() -> ConsoleHull {
        hull_with_helm(25.0)
    }

    fn hull_damaged(current: f32) -> ConsoleHull {
        let mut h = ConsoleHull::from_config(&[(Console::Helm, 25.0)]);
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
        teams.dispatch(0, Console::Helm);
        assert_eq!(teams.lowest_free_team(), Some(1));
    }

    #[test]
    fn lowest_free_team_returns_none_when_all_busy() {
        let mut teams = RepairTeams::new(2);
        teams.dispatch(0, Console::Helm);
        teams.dispatch(1, Console::Tactical);
        assert_eq!(teams.lowest_free_team(), None);
    }

    // ── dispatch ──────────────────────────────────────────────────────────────

    #[test]
    fn dispatch_idle_team_enters_travelling() {
        let mut teams = RepairTeams::new(2);
        teams.dispatch(0, Console::Helm);
        assert!(matches!(&teams.slots()[0], TeamSlot::Travelling { console: Console::Helm, elapsed } if *elapsed == 0.0));
    }

    #[test]
    fn dispatch_non_idle_team_is_noop() {
        let mut teams = RepairTeams::new(2);
        teams.dispatch(0, Console::Helm);
        // Try to redirect — should be ignored
        teams.dispatch(0, Console::Tactical);
        assert!(matches!(&teams.slots()[0], TeamSlot::Travelling { console: Console::Helm, .. }));
    }

    // ── Travelling → Repairing ────────────────────────────────────────────────

    #[test]
    fn travelling_transitions_to_repairing_after_5s() {
        let mut teams = RepairTeams::new(1);
        let mut hull = hull_damaged(20.0); // not at max
        teams.dispatch(0, Console::Helm);
        teams.tick(5.0, &mut hull);
        assert!(matches!(&teams.slots()[0], TeamSlot::Repairing { console: Console::Helm, .. }));
    }

    #[test]
    fn travelling_does_not_transition_before_5s() {
        let mut teams = RepairTeams::new(1);
        let mut hull = hull_damaged(20.0);
        teams.dispatch(0, Console::Helm);
        teams.tick(4.9, &mut hull);
        assert!(matches!(&teams.slots()[0], TeamSlot::Travelling { .. }));
    }

    #[test]
    fn team_arrives_at_full_hp_console_enters_returning() {
        let mut teams = RepairTeams::new(1);
        let mut hull = hull_full(); // console already at full HP
        teams.dispatch(0, Console::Helm);
        teams.tick(5.0, &mut hull);
        assert!(matches!(&teams.slots()[0], TeamSlot::Returning { .. }));
    }

    // ── Repairing → HP restoration ────────────────────────────────────────────

    #[test]
    fn repairing_restores_hp_at_correct_rate() {
        let mut teams = RepairTeams::new(1);
        let mut hull = hull_damaged(0.0); // 0 HP
        teams.dispatch(0, Console::Helm);
        teams.tick(5.0, &mut hull); // travel
        // Now repairing; restore for 2s should give 1 HP
        teams.tick(2.0, &mut hull);
        let hp = hull.current_for(Console::Helm).unwrap();
        assert!((hp - 1.0).abs() < 1e-4, "expected 1 HP after 2s repair, got {hp}");
    }

    #[test]
    fn repairing_transitions_to_returning_when_console_full() {
        let mut teams = RepairTeams::new(1);
        let mut hull = hull_damaged(24.9); // almost full
        teams.dispatch(0, Console::Helm);
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
        teams.dispatch(0, Console::Helm);
        teams.tick(5.0, &mut hull); // travel (arrives full → Returning)
        teams.tick(5.0, &mut hull); // return
        assert!(matches!(&teams.slots()[0], TeamSlot::Idle));
    }

    #[test]
    fn returning_does_not_complete_before_5s() {
        let mut teams = RepairTeams::new(1);
        let mut hull = hull_full();
        teams.dispatch(0, Console::Helm);
        teams.tick(5.0, &mut hull); // travel → Returning
        teams.tick(4.9, &mut hull); // not yet idle
        assert!(matches!(&teams.slots()[0], TeamSlot::Returning { .. }));
    }

    // ── Full lifecycle ────────────────────────────────────────────────────────

    #[test]
    fn full_lifecycle_travel_repair_return_idle() {
        let mut teams = RepairTeams::new(1);
        let mut hull = hull_damaged(0.0); // fully damaged
        teams.dispatch(0, Console::Helm);

        // Travelling
        teams.tick(5.0, &mut hull);
        assert!(matches!(&teams.slots()[0], TeamSlot::Repairing { .. }));

        // Repairing until full (25 HP at 0.5 HP/s = 50s)
        teams.tick(50.0, &mut hull);
        assert!(matches!(&teams.slots()[0], TeamSlot::Returning { .. }));

        // Returning
        teams.tick(5.0, &mut hull);
        assert!(matches!(&teams.slots()[0], TeamSlot::Idle));
    }

    // ── Multiple teams independence ───────────────────────────────────────────

    #[test]
    fn two_teams_operate_independently() {
        let mut hull = ConsoleHull::from_config(&[
            (Console::Helm, 25.0),
            (Console::Tactical, 25.0),
        ]);
        // Damage both consoles
        let mut rng = rand::rng();
        hull.apply_damage(10.0, &mut rng);
        hull.apply_damage(10.0, &mut rng);

        let mut teams = RepairTeams::new(2);
        teams.dispatch(0, Console::Helm);
        teams.dispatch(1, Console::Tactical);

        // Both should be Travelling
        assert!(matches!(&teams.slots()[0], TeamSlot::Travelling { console: Console::Helm, .. }));
        assert!(matches!(&teams.slots()[1], TeamSlot::Travelling { console: Console::Tactical, .. }));

        // After 5s both transition
        teams.tick(5.0, &mut hull);
        let s0 = &teams.slots()[0];
        let s1 = &teams.slots()[1];
        assert!(matches!(s0, TeamSlot::Repairing { console: Console::Helm, .. }) || matches!(s0, TeamSlot::Returning { .. }));
        assert!(matches!(s1, TeamSlot::Repairing { console: Console::Tactical, .. }) || matches!(s1, TeamSlot::Returning { .. }));
    }

    #[test]
    fn non_idle_team_cannot_be_redirected_while_travelling() {
        let mut teams = RepairTeams::new(2);
        teams.dispatch(0, Console::Helm);
        teams.dispatch(0, Console::Tactical); // should be ignored
        assert!(matches!(&teams.slots()[0], TeamSlot::Travelling { console: Console::Helm, .. }));
    }

    #[test]
    fn team_after_returning_can_be_dispatched_again() {
        let mut teams = RepairTeams::new(1);
        let mut hull = hull_full();
        teams.dispatch(0, Console::Helm);
        teams.tick(5.0, &mut hull); // travel → Returning (full HP)
        teams.tick(5.0, &mut hull); // → Idle
        assert!(matches!(&teams.slots()[0], TeamSlot::Idle));
        teams.dispatch(0, Console::Helm);
        assert!(matches!(&teams.slots()[0], TeamSlot::Travelling { .. }));
    }
}
