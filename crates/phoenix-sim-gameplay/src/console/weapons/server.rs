/// Authored open-fire policies retain their existing alert fact. An explicit
/// Tactical restraint occupies the rung below every authored firing threshold.
/// Incoming flow is independent: charged reserves can operate with zero intake.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WeaponsAlertPosture {
    /// This ship's own [`crate::ship::state::ShipRedAlert`].
    pub red_alert: bool,
    /// Compatibility field name for explicit Tactical restraint.
    pub weapons_cold: bool,
    /// A Command stance directing this weapons Station's alert posture (issue
    /// #1107), overriding the ship's own Red Alert for the fire gate. `None`
    /// means no Command stance is in force → the posture tracks `red_alert`
    /// exactly as before this issue, so an undirected hull stays byte-identical.
    pub stance_high_alert: Option<bool>,
}

/// Existing fact vocabulary keeps its bottom rung for explicit restraint.
/// Every shipped minimum firing threshold is at least zero.
pub const WEAPONS_COLD_ALERT_FACT: f64 = -1.0;

/// Compatibility spelling for existing policy hosts. Only canonical explicit
/// restraint is read; neither the allocation group nor intake is a fire gate.
pub fn system_power_group_is_cold(
    _ship_config: Option<&crate::ship::config::ShipConfig>,
    power: Option<&crate::modifiers::power_system::PowerSystem>,
    _system_id: &crate::core::messages::SystemId,
) -> bool {
    power.is_some_and(|power| power.weapons_restrained())
}

impl WeaponsAlertPosture {
    /// Read the posture from the ship alert and permitted restraint reading.
    /// An absent alert reads as "not at alert" — the same fail-open reading the
    /// hosts used before #1041, so a bare-`App` fixture that spawns none behaves
    /// exactly as it did.
    pub fn from_components(
        red_alert: Option<&crate::ship::state::ShipRedAlert>,
        weapons_cold: bool,
    ) -> Self {
        Self::from_parts(red_alert, weapons_cold, None)
    }

    /// As [`from_components`](Self::from_components) but with a Command stance
    /// override (issue #1107). `stance_high_alert: None` is exactly
    /// `from_components`, so every pre-#1107 call site keeps its behaviour.
    pub fn from_parts(
        red_alert: Option<&crate::ship::state::ShipRedAlert>,
        weapons_cold: bool,
        stance_high_alert: Option<bool>,
    ) -> Self {
        Self {
            red_alert: red_alert.is_some_and(|r| r.0),
            weapons_cold,
            stance_high_alert,
        }
    }

    /// A posture with live guns — the shorthand every test written before #1041
    /// keeps working through, and the one the byte-identical claim rests on.
    pub fn alert(red_alert: bool) -> Self {
        Self {
            red_alert,
            weapons_cold: false,
            stance_high_alert: None,
        }
    }

    /// Explicit restraint takes precedence over alert and Command stance.
    pub fn alert_fact_value(self) -> f64 {
        if self.weapons_cold {
            WEAPONS_COLD_ALERT_FACT
        } else if let Some(high) = self.stance_high_alert {
            if high {
                1.0
            } else {
                0.0
            }
        } else if self.red_alert {
            1.0
        } else {
            0.0
        }
    }
}

#[cfg(test)]
mod energy_restraint_tests {
    use super::*;
    #[test]
    fn zero_intake_is_independent_of_explicit_restraint() {
        let mut power = crate::modifiers::power_system::PowerSystem::default();
        let group = crate::core::messages::PowerGroupId("weapons".into());
        power.set_flow(&group, 0.0).unwrap();
        let system = crate::core::messages::SystemId("fixture-weapons".into());
        assert!(!system_power_group_is_cold(None, Some(&power), &system));
        assert!(power.try_spend_discrete(&system).unwrap());
        power.set_weapons_restrained(true);
        assert!(system_power_group_is_cold(None, Some(&power), &system));
        assert_eq!(
            WeaponsAlertPosture::from_components(None, true).alert_fact_value(),
            -1.0
        );
    }
}
