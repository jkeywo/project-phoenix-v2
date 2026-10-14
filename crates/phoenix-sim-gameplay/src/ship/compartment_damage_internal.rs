//! Exact, internal GM incidents: no external armour ray and no second HP pool.
use super::*;

impl DamageState {
    /// Distribute a bounded internal physical incident across actual scoped
    /// compartments. Overflow continues through remaining eligible structure;
    /// the ordinary dose/disable rules apply, and zero HP stays recoverable.
    pub fn apply_internal(
        &mut self,
        definition: &DamageDefinition,
        rooms: &[String],
        amount: f64,
        incident_source: &str,
        rng: &mut Pcg32,
    ) -> Result<f64, String> {
        self.validate(definition)?;
        if !amount.is_finite()
            || amount < 0.0
            || !source(incident_source)
            || rooms.is_empty()
            || rooms
                .iter()
                .any(|room| !definition.rooms.contains_key(room))
        {
            return Err("invalid internal physical damage incident".into());
        }
        let mut eligible: BTreeSet<String> = rooms.iter().cloned().collect();
        let mut remaining = amount;
        let mut applied = 0.0;
        while remaining > 0.0 {
            let total: f64 = eligible.iter().map(|room| self.compartments[room].hp).sum();
            if total <= 0.0 {
                break;
            }
            let mut draw = super::super::damage::unit_f32(rng) as f64 * total;
            let Some(selected) = eligible
                .iter()
                .filter(|room| self.compartments[*room].hp > 0.0)
                .find(|room| {
                    draw -= self.compartments[*room].hp;
                    draw < 0.0
                })
                .or_else(|| {
                    eligible
                        .iter()
                        .rev()
                        .find(|room| self.compartments[*room].hp > 0.0)
                })
                .cloned()
            else {
                break;
            };
            let state = self
                .compartments
                .get_mut(&selected)
                .expect("validated room");
            let loss = state.hp.min(remaining);
            state.hp -= loss;
            if state.hp == 0.0 {
                state.disabled = true;
            }
            self.deposit_dose(
                definition,
                &selected,
                DamageType::Physical,
                loss,
                incident_source,
                &mut Vec::new(),
            );
            remaining = (remaining - loss).max(0.0);
            applied += loss;
            eligible.remove(&selected);
        }
        Ok(applied)
    }
}
