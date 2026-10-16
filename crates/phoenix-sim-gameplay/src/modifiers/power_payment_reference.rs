use super::*;
impl PowerSystem {
    pub(super) fn work_continuous_reference(
        &mut self,
        system: &SystemId,
        dt: f32,
        activity: f32,
    ) -> Result<crate::resources::energy::EnergyGrant, String> {
        if !dt.is_finite() || dt < 0.0 || !activity.is_finite() || activity < 0.0 {
            return Err("energy work requires finite nonnegative time and activity".into());
        }
        let rate = self
            .definition
            .config()
            .consumers
            .iter()
            .find(|consumer| &consumer.system == system)
            .ok_or("unknown energy consumer")?
            .continuous_cost_per_sec;
        let grant = self.spend_continuous(system, rate * dt * activity)?;
        self.last_work_fraction.insert(
            system.clone(),
            if activity > 0.0 { grant.fraction } else { 0.0 },
        );
        Ok(grant)
    }
}
