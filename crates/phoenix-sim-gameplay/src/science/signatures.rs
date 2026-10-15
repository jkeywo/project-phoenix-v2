//! Operating signatures from actual reactor output and paid machinery work.
//!
//! Callers supply the current measurement interval and authored System order.
//! This helper owns no history, sensor knowledge or extra simulation state.
use crate::ship::system_registry::{
    BLASTER_BANK_KIND, HELM_BOOST_KIND, HELM_ENGINE_KIND, HELM_THRUST_KIND, PHASER_BANK_KIND,
    SENSORS_KIND, TORPEDO_TUBE_KIND,
};
use phoenix_model::sensing::PassiveSignature;

/// Preserve the existing Passive instrument's strongest-cause measurement.
///
/// `reactor_output` is actual output per second (zero when generation is off).
/// Each machinery entry is its kind and actual emitted work for this interval.
/// Strict comparison retains reactor-first, then authored-order tie handling.
/// The caller validates a positive, finite interval before measuring the scene.
pub fn observed_signature<'a>(
    reactor_output: f64,
    machinery: impl IntoIterator<Item = (&'a str, f64)>,
    interval_secs: f32,
) -> Option<(f32, PassiveSignature)> {
    let mut strength = reactor_output;
    let mut signature = PassiveSignature::Reactor;
    for (kind, emitted_work) in machinery {
        let cause = if [HELM_ENGINE_KIND, HELM_THRUST_KIND, HELM_BOOST_KIND].contains(&kind) {
            PassiveSignature::Propulsion
        } else if kind == SENSORS_KIND {
            PassiveSignature::ActiveScan
        } else if [PHASER_BANK_KIND, BLASTER_BANK_KIND, TORPEDO_TUBE_KIND].contains(&kind) {
            PassiveSignature::Weapon
        } else {
            continue;
        };
        let measured = emitted_work / f64::from(interval_secs);
        if measured > strength {
            strength = measured;
            signature = cause;
        }
    }
    (strength > 0.0).then(|| (strength.min(f64::from(f32::MAX)) as f32, signature))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn actual_reserve_work_remains_visible_with_reactor_off() {
        assert_eq!(
            observed_signature(0.0, [(HELM_ENGINE_KIND, 3.0)], 0.5),
            Some((6.0, PassiveSignature::Propulsion))
        );
        assert_eq!(observed_signature(0.0, [], 0.5), None);
    }

    #[test]
    fn reactor_then_authored_system_order_resolve_equal_causes() {
        assert_eq!(
            observed_signature(8.0, [(PHASER_BANK_KIND, 4.0)], 0.5),
            Some((8.0, PassiveSignature::Reactor))
        );
        assert_eq!(
            observed_signature(1.0, [(SENSORS_KIND, 4.0), (PHASER_BANK_KIND, 4.0)], 0.5),
            Some((8.0, PassiveSignature::ActiveScan))
        );
        assert_eq!(
            observed_signature(1.0, [(PHASER_BANK_KIND, 4.0), (SENSORS_KIND, 4.0)], 0.5),
            Some((8.0, PassiveSignature::Weapon))
        );
    }

    #[test]
    fn unknown_equipment_does_not_change_the_existing_measurement() {
        assert_eq!(
            observed_signature(2.0, [("unknown", 1000.0), (HELM_BOOST_KIND, 2.0)], 0.5),
            Some((4.0, PassiveSignature::Propulsion))
        );
    }

    #[test]
    fn stronger_actual_work_and_finite_output_saturation_are_preserved() {
        assert_eq!(
            observed_signature(
                4.0,
                [(HELM_THRUST_KIND, 3.0), (TORPEDO_TUBE_KIND, 7.0)],
                0.5
            ),
            Some((14.0, PassiveSignature::Weapon))
        );
        assert_eq!(
            observed_signature(f64::from(f32::MAX) * 2.0, [], 0.5),
            Some((f32::MAX, PassiveSignature::Reactor))
        );
    }
}
