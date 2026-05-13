const REPAIR_DURATION: f32 = 30.0;
const COOLDOWN_DURATION: f32 = 10.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TeamSlot {
    Idle,
    Repairing { progress: f32 },
    Cooldown { progress: f32 },
}

impl Default for TeamSlot {
    fn default() -> Self {
        Self::Idle
    }
}

#[derive(Debug, Clone)]
pub struct RepairTeams {
    slots: [TeamSlot; 3],
}

impl RepairTeams {
    pub fn new() -> Self {
        Self {
            slots: [TeamSlot::Idle, TeamSlot::Idle, TeamSlot::Idle],
        }
    }

    pub fn slots(&self) -> &[TeamSlot; 3] {
        &self.slots
    }

    /// Returns the lowest-indexed idle team, or `None` if all are busy.
    pub fn lowest_free_team(&self) -> Option<usize> {
        self.slots.iter().position(|s| matches!(s, TeamSlot::Idle))
    }

    /// Move a team to `Repairing { progress: 0.0 }`.
    /// No-op if the slot is not `Idle` (in-progress repairs are never interrupted).
    pub fn dispatch(&mut self, team_idx: usize) {
        if let Some(slot) = self.slots.get_mut(team_idx) {
            if matches!(slot, TeamSlot::Idle) {
                *slot = TeamSlot::Repairing { progress: 0.0 };
            }
        }
    }

    /// Move a team to `Cooldown { progress: 1.0 }`.
    /// No-op if the slot is not `Idle`.
    pub fn penalise(&mut self, team_idx: usize) {
        if let Some(slot) = self.slots.get_mut(team_idx) {
            if matches!(slot, TeamSlot::Idle) {
                *slot = TeamSlot::Cooldown { progress: 1.0 };
            }
        }
    }

    /// Advance all active timers by `dt` seconds.
    ///
    /// - `Repairing` advances toward 1.0 over 30s of cumulative `dt`.
    /// - `Cooldown` drains toward 0.0 over 10s of cumulative `dt`.
    /// - Teams that finish repairing return to `Idle`.
    /// - Teams that finish cooling down return to `Idle`.
    ///
    /// Returns the set (as a sorted `Vec`) of team indices whose repair
    /// **completed** during this tick, so the caller can credit HP.
    pub fn tick(&mut self, dt: f32) -> Vec<usize> {
        let mut completed = Vec::new();
        for (i, slot) in self.slots.iter_mut().enumerate() {
            match slot {
                TeamSlot::Repairing { progress } => {
                    *progress += dt / REPAIR_DURATION;
                    if *progress >= 1.0 {
                        *slot = TeamSlot::Idle;
                        completed.push(i);
                    }
                }
                TeamSlot::Cooldown { progress } => {
                    *progress -= dt / COOLDOWN_DURATION;
                    if *progress <= 0.0 {
                        *slot = TeamSlot::Idle;
                    }
                }
                TeamSlot::Idle => {}
            }
        }
        completed
    }
}

impl Default for RepairTeams {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EPSILON: f32 = 1e-6;

    fn approx_eq(a: f32, b: f32) -> bool {
        (a - b).abs() < EPSILON
    }

    /// Helper to extract progress from a Repairing slot.
    fn repairing_progress(teams: &RepairTeams, idx: usize) -> f32 {
        match teams.slots[idx] {
            TeamSlot::Repairing { progress } => progress,
            _ => panic!("slot {idx} is not Repairing"),
        }
    }

    /// Helper to extract progress from a Cooldown slot.
    fn cooldown_progress(teams: &RepairTeams, idx: usize) -> f32 {
        match teams.slots[idx] {
            TeamSlot::Cooldown { progress } => progress,
            _ => panic!("slot {idx} is not Cooldown"),
        }
    }

    // ── Default state ────────────────────────────────────────────────────

    #[test]
    fn new_has_three_idle_slots() {
        let teams = RepairTeams::new();
        assert!(teams.slots.iter().all(|s| matches!(s, TeamSlot::Idle)));
        assert_eq!(teams.slots.len(), 3);
    }

    #[test]
    fn default_is_all_idle() {
        let teams: RepairTeams = Default::default();
        assert!(teams.slots.iter().all(|s| matches!(s, TeamSlot::Idle)));
    }

    // ── lowest_free_team ─────────────────────────────────────────────────

    #[test]
    fn lowest_free_team_returns_zero_when_all_idle() {
        let teams = RepairTeams::new();
        assert_eq!(teams.lowest_free_team(), Some(0));
    }

    #[test]
    fn lowest_free_team_skips_busy_teams() {
        let mut teams = RepairTeams::new();
        teams.dispatch(0);
        assert_eq!(teams.lowest_free_team(), Some(1));
    }

    #[test]
    fn lowest_free_team_returns_none_when_all_busy() {
        let mut teams = RepairTeams::new();
        teams.dispatch(0);
        teams.dispatch(1);
        teams.dispatch(2);
        assert_eq!(teams.lowest_free_team(), None);
    }

    #[test]
    fn lowest_free_team_skips_cooldown_team() {
        let mut teams = RepairTeams::new();
        teams.penalise(0);
        assert_eq!(teams.lowest_free_team(), Some(1));
    }

    // ── dispatch ─────────────────────────────────────────────────────────

    #[test]
    fn dispatch_moves_idle_team_to_repairing_with_zero_progress() {
        let mut teams = RepairTeams::new();
        teams.dispatch(0);
        assert!(matches!(teams.slots[0], TeamSlot::Repairing { .. }));
        assert!(approx_eq(repairing_progress(&teams, 0), 0.0));
    }

    #[test]
    fn dispatch_on_non_idle_team_is_no_op() {
        let mut teams = RepairTeams::new();
        teams.dispatch(0); // now Repairing
        teams.dispatch(0); // should be no-op
        assert!(matches!(teams.slots[0], TeamSlot::Repairing { .. }));
        assert!(approx_eq(repairing_progress(&teams, 0), 0.0));
    }

    #[test]
    fn dispatch_on_cooldown_team_is_no_op() {
        let mut teams = RepairTeams::new();
        teams.penalise(0);
        teams.dispatch(0); // should be no-op — team is in cooldown, not idle
        assert!(matches!(teams.slots[0], TeamSlot::Cooldown { .. }));
    }

    // ── penalise ─────────────────────────────────────────────────────────

    #[test]
    fn penalise_moves_idle_team_to_cooldown_with_full_progress() {
        let mut teams = RepairTeams::new();
        teams.penalise(0);
        assert!(matches!(teams.slots[0], TeamSlot::Cooldown { .. }));
        assert!(approx_eq(cooldown_progress(&teams, 0), 1.0));
    }

    #[test]
    fn penalise_on_non_idle_team_is_no_op() {
        let mut teams = RepairTeams::new();
        teams.dispatch(0);
        teams.penalise(0); // should be no-op — team is repairing, not idle
        assert!(matches!(teams.slots[0], TeamSlot::Repairing { .. }));
    }

    // ── tick — repair progression ────────────────────────────────────────

    #[test]
    fn tick_advances_repair_progress_by_dt_over_30s() {
        let mut teams = RepairTeams::new();
        teams.dispatch(0);
        teams.tick(15.0);
        assert!(approx_eq(repairing_progress(&teams, 0), 0.5));
    }

    #[test]
    fn tick_completes_repair_and_returns_team_index() {
        let mut teams = RepairTeams::new();
        teams.dispatch(0);
        let completed = teams.tick(30.0);
        assert!(matches!(teams.slots[0], TeamSlot::Idle));
        assert_eq!(completed, vec![0]);
    }

    #[test]
    fn tick_partial_then_full_completes_repair() {
        let mut teams = RepairTeams::new();
        teams.dispatch(0);
        teams.tick(15.0);
        let completed = teams.tick(15.0);
        assert!(matches!(teams.slots[0], TeamSlot::Idle));
        assert_eq!(completed, vec![0]);
    }

    #[test]
    fn multiple_ticks_accumulate_toward_completion() {
        let mut teams = RepairTeams::new();
        teams.dispatch(0);
        for _ in 0..29 {
            let completed = teams.tick(1.0);
            assert!(completed.is_empty(), "repair should not complete early");
        }
        let completed = teams.tick(1.0);
        assert_eq!(completed, vec![0]);
        assert!(matches!(teams.slots[0], TeamSlot::Idle));
    }

    #[test]
    fn tick_past_completion_does_not_overflow() {
        let mut teams = RepairTeams::new();
        teams.dispatch(0);
        let completed = teams.tick(60.0);
        assert_eq!(completed, vec![0]);
        assert!(matches!(teams.slots[0], TeamSlot::Idle));
    }

    // ── tick — cooldown drain ────────────────────────────────────────────

    #[test]
    fn tick_drains_cooldown_progress_by_dt_over_10s() {
        let mut teams = RepairTeams::new();
        teams.penalise(0);
        teams.tick(5.0);
        assert!(approx_eq(cooldown_progress(&teams, 0), 0.5));
    }

    #[test]
    fn tick_completes_cooldown_and_returns_to_idle() {
        let mut teams = RepairTeams::new();
        teams.penalise(0);
        let completed = teams.tick(10.0);
        assert!(matches!(teams.slots[0], TeamSlot::Idle));
        assert!(completed.is_empty(), "cooldown completion should not return index");
    }

    #[test]
    fn tick_past_cooldown_completion_does_not_underflow() {
        let mut teams = RepairTeams::new();
        teams.penalise(0);
        teams.tick(20.0);
        assert!(matches!(teams.slots[0], TeamSlot::Idle));
    }

    // ── Mixed-state independence ─────────────────────────────────────────

    #[test]
    fn two_teams_can_repair_independently() {
        let mut teams = RepairTeams::new();
        teams.dispatch(0);
        teams.dispatch(1);
        teams.tick(15.0);
        assert!(approx_eq(repairing_progress(&teams, 0), 0.5));
        assert!(approx_eq(repairing_progress(&teams, 1), 0.5));
    }

    #[test]
    fn one_team_repairing_another_on_cooldown_tick_independently() {
        let mut teams = RepairTeams::new();
        teams.dispatch(0); // Repairing
        teams.penalise(1); // Cooldown
        teams.tick(5.0);
        assert!(approx_eq(repairing_progress(&teams, 0), 5.0 / 30.0));
        assert!(approx_eq(cooldown_progress(&teams, 1), 1.0 - 5.0 / 10.0));
    }

    #[test]
    fn idle_team_stays_idle_after_tick() {
        let mut teams = RepairTeams::new();
        teams.dispatch(0);
        teams.tick(15.0);
        assert!(matches!(teams.slots[1], TeamSlot::Idle));
        assert!(matches!(teams.slots[2], TeamSlot::Idle));
    }

    #[test]
    fn staggered_repair_teams_complete_independently() {
        let mut teams = RepairTeams::new();
        teams.dispatch(0);
        teams.tick(5.0);
        teams.dispatch(1);
        teams.tick(25.0); // team 0 had 30s total, completes; team 1 had 25s
        assert!(matches!(teams.slots[0], TeamSlot::Idle));
        assert!(matches!(teams.slots[1], TeamSlot::Repairing { .. }));
        let completed = teams.tick(5.0); // team 1 completes now
        assert_eq!(completed, vec![1]);
    }

    #[test]
    fn multiple_teams_can_complete_in_same_tick() {
        let mut teams = RepairTeams::new();
        teams.dispatch(0);
        teams.dispatch(1);
        let completed = teams.tick(30.0);
        assert_eq!(completed, vec![0, 1]);
        assert!(matches!(teams.slots[0], TeamSlot::Idle));
        assert!(matches!(teams.slots[1], TeamSlot::Idle));
    }

    #[test]
    fn team_that_completes_repair_can_be_dispatched_again() {
        let mut teams = RepairTeams::new();
        teams.dispatch(0);
        teams.tick(30.0);
        // team 0 is now idle; we can dispatch again
        teams.dispatch(0);
        assert!(matches!(teams.slots[0], TeamSlot::Repairing { .. }));
    }
}
