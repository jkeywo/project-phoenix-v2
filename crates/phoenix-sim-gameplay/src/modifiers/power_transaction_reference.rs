//! Previous clone-and-commit implementation retained only as a test oracle.
use super::*;
impl PowerSystem {
    pub(super) fn tick_energy_reference(
        &mut self,
        dt: f32,
        bindings: &EnergyCoolantBindings,
        available_stock: &BTreeMap<String, i64>,
        effects: &crate::resources::energy_network::EnergyHardwareEffects,
        choose: &mut impl FnMut(usize) -> usize,
    ) -> Result<PowerTickResult, String> {
        if available_stock.values().any(|value| *value < 0) {
            return Err("coolant stocks cannot be negative".into());
        }
        self.validate_current_observations()?;
        if self
            .coolant_credit
            .keys()
            .any(|id| !available_stock.contains_key(id))
        {
            return Err("saved coolant credit references unknown actual capacity".into());
        }
        let mut budgets = BTreeMap::new();
        for (id, available) in available_stock {
            let amount = *available as f64 + self.coolant_credit.get(id).copied().unwrap_or(0.0);
            let mut continuous = amount as f32;
            if continuous as f64 > amount && continuous > 0.0 {
                continuous = f32::from_bits(continuous.to_bits() - 1);
            }
            budgets.insert(id.clone(), continuous);
        }
        let mut next = self.clone();
        let before = next.battery_charge();
        let energy = next.network.tick_with_definition_in_place(
            &next.definition,
            dt,
            bindings,
            &budgets,
            effects,
            choose,
        )?;
        let mut withdrawals = BTreeMap::new();
        for (id, used) in &energy.coolant_used {
            let old = next.coolant_credit.get(id).copied().unwrap_or(0.0);
            let needed = (*used as f64 - old).max(0.0);
            let units = needed.ceil();
            let available = available_stock
                .get(id)
                .copied()
                .ok_or("cooling used an unknown provider")?;
            if !units.is_finite() || units >= i64::MAX as f64 || units > available as f64 {
                return Err("actual coolant stock cannot fund resolved cooling".into());
            }
            let credit = old + units - *used as f64;
            if !credit.is_finite() || credit < 0.0 || credit > 1.0 {
                return Err("coolant accounting escaped its unit bounds".into());
            }
            // Conservative IEEE rounding of sub-precision expenditure never
            // credits an entire extra stock unit or creates coolant.
            let credit = credit.min(f64::from_bits(1.0f64.to_bits() - 1));
            next.coolant_credit.insert(id.clone(), credit);
            if units > 0.0 {
                withdrawals.insert(id.clone(), units as i64);
            }
        }
        if dt > 0.0 {
            next.last_reactor_energy_delta_per_sec = (next.battery_charge() - before) / dt;
            next.last_intake_per_sec = energy
                .consumer_reports
                .iter()
                .map(|(id, report)| (id.clone(), report.accepted_energy / dt))
                .collect();
        }
        next.validate_current_observations()?;
        *self = next;
        Ok(PowerTickResult {
            energy,
            coolant_withdrawals: withdrawals,
        })
    }
}
