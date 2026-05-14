use crate::messages::Console;

#[derive(Clone, Debug, PartialEq)]
pub struct PowerSystem {
    pub helm: u8,
    pub weapons: u8,
    /// Power allocation for the Sensors console (drives radar range multiplier).
    pub sensors: u8,
    pub battery_charge: f32,
    pub locked: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PowerConfig {
    pub capacity: f32,
    pub rates: [f32; 6],
    pub emergency_threshold: f32,
}

impl Default for PowerConfig {
    fn default() -> Self {
        Self {
            capacity: 100.0,
            rates: [6.0, 5.0, 4.0, 2.0, -2.0, -6.0],
            emergency_threshold: 25.0,
        }
    }
}

impl Default for PowerSystem {
    fn default() -> Self {
        Self {
            helm: 2,
            weapons: 2,
            sensors: 2,
            battery_charge: 100.0,
            locked: false,
        }
    }
}

impl PowerSystem {
    pub fn new(config: &PowerConfig) -> Self {
        Self {
            helm: 2,
            weapons: 2,
            sensors: 2,
            battery_charge: config.capacity,
            locked: false,
        }
    }

    pub fn total(&self) -> u8 {
        self.helm + self.weapons + self.sensors
    }

    pub fn increase(&mut self, console: Console) {
        if self.locked || self.total() >= 8 {
            return;
        }
        match console {
            Console::Helm if self.helm < 4 => self.helm += 1,
            Console::Tactical if self.weapons < 4 => self.weapons += 1,
            Console::Sensors if self.sensors < 4 => self.sensors += 1,
            _ => {}
        }
    }

    pub fn decrease(&mut self, console: Console) {
        if self.locked {
            return;
        }
        match console {
            Console::Helm if self.helm > 1 => self.helm -= 1,
            Console::Tactical if self.weapons > 1 => self.weapons -= 1,
            Console::Sensors if self.sensors > 1 => self.sensors -= 1,
            _ => {}
        }
    }

    /// Advance the simulation by `dt` seconds. Updates battery charge based on the
    /// current total allocation, handles exhaustion (forced to 1 + lock), and
    /// recovery (unlock at emergency threshold).
    ///
    /// Returns `true` if the `locked` state changed this tick.
    pub fn tick(&mut self, dt: f32, config: &PowerConfig) -> bool {
        let prev_locked = self.locked;
        let total = self.total().clamp(3, 8) as usize;
        let rate = config.rates[total - 3];
        self.battery_charge = (self.battery_charge + rate * dt).clamp(0.0, config.capacity);

        if self.battery_charge <= 0.0 {
            self.helm = 1;
            self.weapons = 1;
            self.sensors = 1;
            self.locked = true;
        } else if self.locked && self.battery_charge >= config.emergency_threshold {
            self.locked = false;
        }

        self.locked != prev_locked
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults() {
        let ps = PowerSystem::default();
        assert_eq!(ps.helm, 2);
        assert_eq!(ps.weapons, 2);
        assert_eq!(ps.sensors, 2);
        assert_eq!(ps.battery_charge, 100.0);
        assert!(!ps.locked);
    }

    #[test]
    fn increase_helm() {
        let mut ps = PowerSystem::default();
        ps.increase(Console::Helm);
        assert_eq!(ps.helm, 3);
    }

    #[test]
    fn increase_weapons() {
        let mut ps = PowerSystem::default();
        ps.increase(Console::Tactical);
        assert_eq!(ps.weapons, 3);
    }

    #[test]
    fn increase_sensors() {
        let mut ps = PowerSystem::default();
        ps.increase(Console::Sensors);
        assert_eq!(ps.sensors, 3);
    }

    #[test]
    fn increase_at_four_is_noop() {
        let mut ps = PowerSystem::default();
        ps.helm = 4;
        ps.increase(Console::Helm);
        assert_eq!(ps.helm, 4);
    }

    #[test]
    fn increase_at_total_cap_eight_is_noop() {
        let mut ps = PowerSystem::default();
        ps.helm = 3;
        ps.weapons = 3;
        ps.sensors = 2;
        // total is 8, sensors can't go to 3
        ps.increase(Console::Sensors);
        assert_eq!(ps.sensors, 2);
    }

    #[test]
    fn increase_when_locked_is_noop() {
        let mut ps = PowerSystem::default();
        ps.locked = true;
        ps.helm = 1;
        ps.increase(Console::Helm);
        assert_eq!(ps.helm, 1);
    }

    #[test]
    fn decrease_helm() {
        let mut ps = PowerSystem::default();
        ps.decrease(Console::Helm);
        assert_eq!(ps.helm, 1);
    }

    #[test]
    fn decrease_weapons() {
        let mut ps = PowerSystem::default();
        ps.decrease(Console::Tactical);
        assert_eq!(ps.weapons, 1);
    }

    #[test]
    fn decrease_sensors() {
        let mut ps = PowerSystem::default();
        ps.decrease(Console::Sensors);
        assert_eq!(ps.sensors, 1);
    }

    #[test]
    fn decrease_at_one_is_noop() {
        let mut ps = PowerSystem::default();
        ps.helm = 1;
        ps.decrease(Console::Helm);
        assert_eq!(ps.helm, 1);
    }

    #[test]
    fn decrease_when_locked_is_noop() {
        let mut ps = PowerSystem::default();
        ps.locked = true;
        ps.decrease(Console::Helm);
        assert_eq!(ps.helm, 2);
    }

    // ── tick ──────────────────────────────────────────────────────────────

    #[test]
    fn tick_discharges_above_base() {
        let config = PowerConfig::default();
        let mut ps = PowerSystem::default();
        ps.helm = 4;
        ps.weapons = 2;
        ps.sensors = 2;
        // total = 8 → rate = -6.0/s
        ps.battery_charge = 100.0;
        ps.tick(1.0, &config);
        assert!((ps.battery_charge - 94.0).abs() < 0.001);
    }

    #[test]
    fn tick_recharges_below_base() {
        let config = PowerConfig::default();
        let mut ps = PowerSystem::default();
        // total = 6 → rate = 2.0/s
        ps.battery_charge = 50.0;
        ps.tick(1.0, &config);
        assert!((ps.battery_charge - 52.0).abs() < 0.001);
    }

    #[test]
    fn tick_cannot_overcharge_beyond_capacity() {
        let config = PowerConfig::default();
        let mut ps = PowerSystem::default();
        // total = 3 → rate = 6.0/s
        ps.helm = 1;
        ps.weapons = 1;
        ps.sensors = 1;
        ps.battery_charge = 99.0;
        ps.tick(1.0, &config);
        assert!((ps.battery_charge - 100.0).abs() < 0.001);
    }

    #[test]
    fn exhaustion_forces_consoles_to_one_and_locks() {
        let config = PowerConfig::default();
        let mut ps = PowerSystem::default();
        // total = 8 → rate = -6.0/s; drain battery completely
        ps.helm = 4;
        ps.weapons = 2;
        ps.sensors = 2;
        ps.battery_charge = 5.0;
        ps.tick(1.0, &config);
        assert_eq!(ps.helm, 1);
        assert_eq!(ps.weapons, 1);
        assert_eq!(ps.sensors, 1);
        assert!(ps.locked);
    }

    #[test]
    fn recovery_unlocks_at_threshold() {
        let config = PowerConfig::default();
        let mut ps = PowerSystem::default();
        ps.helm = 1;
        ps.weapons = 1;
        ps.sensors = 1;
        ps.locked = true;
        ps.battery_charge = 10.0;
        // total = 3 → rate = 6.0/s; charge from 10 to 16 (still below 25)
        ps.tick(1.0, &config);
        assert!(ps.locked);
        // charge from 16 to 22
        ps.tick(1.0, &config);
        assert!(ps.locked);
        // charge from 22 to 28 → crosses 25
        ps.tick(1.0, &config);
        assert!(!ps.locked);
    }

    #[test]
    fn locked_blocks_increase_and_decrease() {
        let config = PowerConfig::default();
        let mut ps = PowerSystem::default();
        // drain battery to force lock
        ps.helm = 4;
        ps.weapons = 2;
        ps.sensors = 2;
        ps.battery_charge = 5.0;
        ps.tick(1.0, &config);
        assert!(ps.locked);

        // Now all are 1; try to increase — should be no-op
        ps.increase(Console::Helm);
        assert_eq!(ps.helm, 1);
        ps.decrease(Console::Helm);
        assert_eq!(ps.helm, 1);
    }

    #[test]
    fn tick_returns_true_when_lock_changes() {
        let config = PowerConfig::default();
        let mut ps = PowerSystem::default();
        ps.helm = 4;
        ps.weapons = 2;
        ps.sensors = 2;
        ps.battery_charge = 5.0;
        // tick that exhausts the battery → locked changes from false to true
        assert!(ps.tick(1.0, &config));
        // next tick: locked stays true
        assert!(!ps.tick(1.0, &config));
    }

    // ── configurable constructor ──────────────────────────────────────────

    #[test]
    fn custom_config() {
        let config = PowerConfig {
            capacity: 50.0,
            rates: [1.0, 1.0, 1.0, -1.0, -2.0, -3.0],
            emergency_threshold: 10.0,
        };
        let ps = PowerSystem::new(&config);
        assert_eq!(ps.helm, 2);
        assert_eq!(ps.weapons, 2);
        assert_eq!(ps.sensors, 2);
        assert!((ps.battery_charge - 50.0).abs() < 0.001);
        assert!(!ps.locked);
    }

    #[test]
    fn increase_sensors_increases_radar_power_slot() {
        let mut ps = PowerSystem::default();
        ps.increase(Console::Sensors);
        assert_eq!(ps.sensors, 3, "Sensors console should drive radar power");
    }

    #[test]
    fn increase_shields_is_noop_on_power_system() {
        let mut ps = PowerSystem::default();
        ps.increase(Console::Shields);
        // Shields has no power slot in the system — should be a no-op
        assert_eq!(ps.helm, 2);
        assert_eq!(ps.weapons, 2);
        assert_eq!(ps.sensors, 2);
    }

    #[test]
    fn increase_navigation_is_noop_on_power_system() {
        let mut ps = PowerSystem::default();
        ps.increase(Console::Navigation);
        // Navigation has no power slot in the system — should be a no-op
        assert_eq!(ps.helm, 2);
        assert_eq!(ps.weapons, 2);
        assert_eq!(ps.sensors, 2);
    }
}
