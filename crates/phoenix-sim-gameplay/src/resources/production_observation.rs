use super::*;

/// Ephemeral permitted reports, not another inventory or saved production state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProductionStockObservation {
    pub store: String,
    pub level: i64,
    pub available: i64,
    pub incoming: i64,
    pub held: i64,
    pub reserve: i64,
    pub headroom: i64,
    pub ceiling: i64,
    /// Exact shared free space; per-resource headroom loses integer remainders.
    pub bay_free_space: i64,
}

impl ProductionState {
    /// Policy-only readiness using the selected recipe's permitted reports.
    /// Execution still rechecks and commits actual stocks through `try_start`.
    pub fn start_available_from_report(
        &self,
        profile: &CompiledProduction,
        tick: u64,
        reports: &[ProductionStockObservation],
    ) -> Result<bool, String> {
        if self.owner != profile.config.system
            || tick < self.last_tick
            || tick > self.last_tick.saturating_add(1)
        {
            return Err(
                "production observation owner/tick differs from its current frontier".into(),
            );
        }
        profile.validate_target(self.selected_recipe.as_deref(), self.target)?;
        if !self.enabled || self.active.is_some() || self.selected_recipe.is_none() {
            return Ok(false);
        }
        let recipe = profile.recipe(self.selected_recipe.as_deref().expect("checked selection"))?;
        let relevant: BTreeSet<_> = recipe
            .inputs
            .iter()
            .chain(&recipe.outputs)
            .map(|r| r.store.as_str())
            .collect();
        let mut rows = BTreeMap::new();
        let mut free_space = None;
        let mut reported_used_space = 0i128;
        for row in reports
            .iter()
            .filter(|r| relevant.contains(r.store.as_str()))
        {
            let store = profile.logistics.store(&row.store)?;
            let bay = profile
                .logistics
                .config()
                .storage
                .iter()
                .find(|b| b.compartment == store.compartment)
                .expect("compiled bay");
            let occupied = row
                .level
                .checked_add(row.incoming)
                .and_then(|v| v.checked_add(row.held))
                .ok_or("reported production occupancy overflows")?;
            if rows.insert(row.store.as_str(), row).is_some()
                || row.ceiling != profile.logistics.endpoints()[&store.endpoint].ceiling
                || [
                    row.level,
                    row.available,
                    row.incoming,
                    row.held,
                    row.reserve,
                    row.headroom,
                    row.bay_free_space,
                ]
                .iter()
                .any(|v| *v < 0)
                || row.available > row.level
                || row.reserve > row.ceiling
                || occupied > row.ceiling
                || row.bay_free_space > bay.space
                || row.headroom
                    != (row.ceiling - occupied)
                        .min(row.bay_free_space / bay.unit_space[&store.resource])
                || free_space.is_some_and(|space| space != row.bay_free_space)
            {
                return Err(
                    "production report has inconsistent stock, shared room or identity".into(),
                );
            }
            reported_used_space = reported_used_space
                .checked_add(space_for(profile, &row.store, occupied)?)
                .ok_or("reported production space overflows")?;
            if reported_used_space > i128::from(bay.space - row.bay_free_space) {
                return Err("reported recipe stocks exceed observed occupied bay space".into());
            }
            free_space = Some(row.bay_free_space);
        }
        if rows.len() != relevant.len() {
            return Ok(false);
        }
        let primary = rows[recipe.outputs[0].store.as_str()];
        let occupied = primary
            .level
            .checked_add(primary.incoming)
            .and_then(|v| v.checked_add(primary.held))
            .ok_or("reported primary occupancy overflows")?;
        if occupied
            .checked_add(recipe.outputs[0].amount)
            .is_none_or(|q| q > self.target)
        {
            return Ok(false);
        }
        if self.next_id == 0 || self.next_id == u64::MAX {
            return Err("production job identity exhausted".into());
        }
        let mut released = 0i128;
        let mut required = 0i128;
        for input in &recipe.inputs {
            let row = rows[input.store.as_str()];
            if row.available < input.amount || row.level.saturating_sub(row.reserve) < input.amount
            {
                return Ok(false);
            }
            released = released
                .checked_add(space_for(profile, &input.store, input.amount)?)
                .ok_or("production released space overflows")?;
        }
        for output in &recipe.outputs {
            let row = rows[output.store.as_str()];
            let consumed = recipe
                .inputs
                .iter()
                .find(|i| i.store == output.store)
                .map_or(0, |i| i.amount);
            let occupied_after = row
                .level
                .checked_sub(consumed)
                .and_then(|v| v.checked_add(row.incoming))
                .and_then(|v| v.checked_add(row.held))
                .and_then(|v| v.checked_add(output.amount))
                .ok_or("production output occupancy overflows")?;
            if occupied_after > row.ceiling {
                return Ok(false);
            }
            required = required
                .checked_add(space_for(profile, &output.store, output.amount)?)
                .ok_or("production required space overflows")?;
        }
        Ok(required
            <= i128::from(free_space.expect("nonempty recipe reports"))
                .checked_add(released)
                .ok_or("production available space overflows")?)
    }
}

fn space_for(profile: &CompiledProduction, id: &str, amount: i64) -> Result<i128, String> {
    let store = profile.logistics.store(id)?;
    let bay = profile
        .logistics
        .config()
        .storage
        .iter()
        .find(|b| b.compartment == store.compartment)
        .expect("compiled bay");
    i128::from(amount)
        .checked_mul(i128::from(bay.unit_space[&store.resource]))
        .ok_or_else(|| "production space product overflows".into())
}
