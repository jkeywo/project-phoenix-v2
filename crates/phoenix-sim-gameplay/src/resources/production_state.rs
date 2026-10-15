use super::*;
use phoenix_model::{job::JobId, messages::SystemId};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProductionBatch {
    pub id: JobId,
    pub recipe: String,
    pub born_tick: u64,
    pub progress: f32,
    /// Exactly the ingredients already consumed, not a second spendable stock.
    pub inputs: Vec<ProductionAmount>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProductionBlock {
    Disabled,
    Target,
    Inputs,
    OutputCapacity,
    Work,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProductionOutcome {
    pub id: JobId,
    pub recipe: String,
    pub discarded: bool,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProductionState {
    pub owner: SystemId,
    pub enabled: bool,
    pub selected_recipe: Option<String>,
    pub target: i64,
    pub next_id: u64,
    pub last_tick: u64,
    pub active: Option<ProductionBatch>,
    pub blocked: Option<ProductionBlock>,
    pub last_outcome: Option<ProductionOutcome>,
}
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ProductionTickResult {
    pub completed: Option<ProductionOutcome>,
    /// Bound the next start's same-System work offer to dt minus this amount.
    pub work_offered_secs: f32,
}

impl ProductionState {
    pub fn new(profile: &CompiledProduction) -> Self {
        Self {
            owner: profile.config.system.clone(),
            enabled: true,
            selected_recipe: profile.config.initial_recipe.clone(),
            target: profile.config.initial_target,
            next_id: 1,
            last_tick: 0,
            active: None,
            blocked: None,
            last_outcome: None,
        }
    }
    pub fn claim(&self, id: JobId, store: &str) -> String {
        format!("production/{}/{}/{store}", self.owner.0, id.0)
    }
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }
    pub fn set_target(&mut self, profile: &CompiledProduction, target: i64) -> Result<(), String> {
        profile.validate_target(self.selected_recipe.as_deref(), target)?;
        self.target = target;
        Ok(())
    }

    fn claims(
        &self,
        profile: &CompiledProduction,
        batch: &ProductionBatch,
    ) -> Result<BTreeMap<String, String>, String> {
        Ok(profile
            .recipe(&batch.recipe)?
            .outputs
            .iter()
            .map(|output| (output.store.clone(), self.claim(batch.id, &output.store)))
            .collect())
    }
    pub fn expected_incoming(
        &self,
        profile: &CompiledProduction,
    ) -> Result<IncomingClaims, String> {
        let mut result = IncomingClaims::new();
        if let Some(batch) = &self.active {
            for output in &profile.recipe(&batch.recipe)?.outputs {
                let store = profile.logistics.store(&output.store)?;
                result
                    .entry(store.endpoint.clone())
                    .or_default()
                    .insert(self.claim(batch.id, &output.store), output.amount);
            }
        }
        Ok(result)
    }
    /// Explicit recipe replacement disposes only consumed WIP. Unused inputs
    /// and completed products stay in their real stocks; target reset is explicit.
    pub fn select_recipe(
        &mut self,
        profile: &CompiledProduction,
        recipe: Option<&str>,
        target: i64,
        stocks: &mut EndpointMap,
    ) -> Result<(), String> {
        profile.validate_target(recipe, target)?;
        let mut staged = stocks.clone();
        let discarded = if self.selected_recipe.as_deref() != recipe {
            if let Some(batch) = &self.active {
                apply_production_stock_action(
                    profile,
                    &ProductionStockAction::Discard {
                        recipe: batch.recipe.clone(),
                        claims: self.claims(profile, batch)?,
                    },
                    &mut staged,
                )?;
                Some(ProductionOutcome {
                    id: batch.id,
                    recipe: batch.recipe.clone(),
                    discarded: true,
                })
            } else {
                None
            }
        } else {
            None
        };
        if let Some(outcome) = discarded {
            self.active = None;
            self.last_outcome = Some(outcome);
        }
        self.selected_recipe = recipe.map(str::to_owned);
        self.target = target;
        self.blocked = None;
        *stocks = staged;
        Ok(())
    }
    fn planned_start(
        &self,
        profile: &CompiledProduction,
        tick: u64,
        floors: &BTreeMap<String, i64>,
        stocks: &EndpointMap,
    ) -> Result<Result<(ProductionBatch, EndpointMap), ProductionBlock>, String> {
        if tick < self.last_tick || tick > self.last_tick.saturating_add(1) {
            return Err("production start tick is outside the current frontier".into());
        }
        if self.owner != profile.config.system {
            return Err("production owner differs from fitted System".into());
        }
        if !self.enabled {
            return Ok(Err(ProductionBlock::Disabled));
        }
        if self.active.is_some() {
            return Ok(Err(ProductionBlock::Work));
        }
        let Some(id) = &self.selected_recipe else {
            return Ok(Err(ProductionBlock::Target));
        };
        profile.validate_target(Some(id), self.target)?;
        let recipe = profile.recipe(id)?;
        let primary = profile.logistics.store(&recipe.outputs[0].store)?;
        profile.logistics.validate_stocks(stocks)?;
        let stock = &stocks[&primary.endpoint];
        let occupied = stock
            .stock
            .level
            .checked_add(stock.stock.incoming_total())
            .and_then(|q| q.checked_add(stock.held))
            .ok_or("production primary occupancy overflows")?;
        if occupied
            .checked_add(recipe.outputs[0].amount)
            .is_none_or(|q| q > self.target)
        {
            return Ok(Err(ProductionBlock::Target));
        }
        if self.next_id == 0 || self.next_id == u64::MAX {
            return Err("production job identity exhausted".into());
        }
        let batch = ProductionBatch {
            id: JobId(self.next_id),
            recipe: id.clone(),
            born_tick: tick,
            progress: 0.0,
            inputs: recipe.inputs.clone(),
        };
        for input in &recipe.inputs {
            let store = profile.logistics.store(&input.store)?;
            let floor = *floors
                .get(&input.store)
                .ok_or("production requires each input's Inventory floor")?;
            if floor < 0 || floor > stocks[&store.endpoint].stock.ceiling {
                return Err("invalid production input floor".into());
            }
            let stock = &stocks[&store.endpoint].stock;
            if stock.available() < input.amount || stock.level.saturating_sub(floor) < input.amount
            {
                return Ok(Err(ProductionBlock::Inputs));
            }
        }
        let mut staged = stocks.clone();
        if !apply_production_stock_action(
            profile,
            &ProductionStockAction::Begin {
                recipe: id.clone(),
                claims: self.claims(profile, &batch)?,
                floors: floors.clone(),
            },
            &mut staged,
        )? {
            return Ok(Err(ProductionBlock::OutputCapacity));
        }
        Ok(Ok((batch, staged)))
    }
    /// Truth-side execution check; ordinary AI must decide from permitted views.
    pub fn start_available(
        &self,
        profile: &CompiledProduction,
        tick: u64,
        floors: &BTreeMap<String, i64>,
        stocks: &EndpointMap,
    ) -> Result<bool, String> {
        Ok(self.planned_start(profile, tick, floors, stocks)?.is_ok())
    }
    pub fn try_start<F>(
        &mut self,
        profile: &CompiledProduction,
        tick: u64,
        dt: f32,
        floors: &BTreeMap<String, i64>,
        stocks: &mut EndpointMap,
        mut work: F,
    ) -> Result<bool, String>
    where
        F: FnMut(&SystemId, f32) -> Result<f32, String>,
    {
        valid_dt(dt)?;
        let (mut batch, staged) = match self.planned_start(profile, tick, floors, stocks)? {
            Ok(plan) => plan,
            Err(reason) => {
                self.blocked = Some(reason);
                return Ok(false);
            }
        };
        let seconds = dt.min(profile.recipe(&batch.recipe)?.work_secs);
        if seconds <= 0.0 {
            self.blocked = Some(ProductionBlock::Work);
            return Ok(false);
        }
        let fraction = work(&self.owner, seconds)?;
        valid_fraction(fraction)?;
        batch.progress = seconds * fraction;
        if batch.progress <= 0.0 {
            self.blocked = Some(ProductionBlock::Work);
            return Ok(false);
        }
        *stocks = staged;
        self.active = Some(batch);
        self.next_id += 1;
        self.blocked = None;
        Ok(true)
    }
    pub fn tick<F>(
        &mut self,
        profile: &CompiledProduction,
        tick: u64,
        dt: f32,
        stocks: &mut EndpointMap,
        mut work: F,
    ) -> Result<ProductionTickResult, String>
    where
        F: FnMut(&SystemId, f32) -> Result<f32, String>,
    {
        valid_dt(dt)?;
        if tick <= self.last_tick {
            return Ok(ProductionTickResult::default());
        }
        let mut next = self.clone();
        let mut staged = stocks.clone();
        next.blocked = None;
        let mut result = ProductionTickResult::default();
        if !next.enabled {
            next.blocked = Some(ProductionBlock::Disabled);
        } else if let Some(batch) = &mut next.active {
            if batch.born_tick < tick {
                let recipe = profile.recipe(&batch.recipe)?;
                let seconds = dt.min((recipe.work_secs - batch.progress).max(0.0));
                if seconds > 0.0 {
                    result.work_offered_secs = seconds;
                    let fraction = work(&next.owner, seconds)?;
                    valid_fraction(fraction)?;
                    batch.progress = (batch.progress + seconds * fraction).min(recipe.work_secs);
                    if fraction == 0.0 {
                        next.blocked = Some(ProductionBlock::Work);
                    }
                }
                if batch.progress == recipe.work_secs {
                    let action = ProductionStockAction::Complete {
                        recipe: batch.recipe.clone(),
                        claims: self.claims(profile, batch)?,
                    };
                    if apply_production_stock_action(profile, &action, &mut staged)? {
                        let outcome = ProductionOutcome {
                            id: batch.id,
                            recipe: batch.recipe.clone(),
                            discarded: false,
                        };
                        next.active = None;
                        next.last_outcome = Some(outcome.clone());
                        result.completed = Some(outcome);
                    } else {
                        next.blocked = Some(ProductionBlock::OutputCapacity);
                    }
                }
            }
        }
        next.last_tick = tick;
        *self = next;
        *stocks = staged;
        Ok(result)
    }
    pub fn validate(
        &self,
        profile: &CompiledProduction,
        stocks: &EndpointMap,
        logistics: &crate::resources::logistics::LogisticsState,
    ) -> Result<(), String> {
        if self.owner != profile.config.system || self.next_id == 0 {
            return Err("production continuation owner/identity is invalid".into());
        }
        profile.validate_target(self.selected_recipe.as_deref(), self.target)?;
        if let Some(batch) = &self.active {
            let recipe = profile.recipe(&batch.recipe)?;
            if batch.id.0 == 0
                || batch.id.0 >= self.next_id
                || batch.born_tick > self.last_tick.saturating_add(1)
                || self.selected_recipe.as_deref() != Some(&batch.recipe)
                || batch.inputs != recipe.inputs
                || !batch.progress.is_finite()
                || batch.progress <= 0.0
                || batch.progress > recipe.work_secs
            {
                return Err("production work-in-progress differs from its selected recipe".into());
            }
        }
        if let Some(outcome) = &self.last_outcome {
            profile.recipe(&outcome.recipe)?;
            if outcome.id.0 == 0
                || outcome.id.0 >= self.next_id
                || self
                    .active
                    .as_ref()
                    .is_some_and(|batch| batch.id == outcome.id)
            {
                return Err("production outcome identity is invalid".into());
            }
        }
        if self.blocked == Some(ProductionBlock::OutputCapacity)
            && self.active.as_ref().is_some_and(|batch| {
                batch.progress != profile.recipe(&batch.recipe).map_or(0.0, |r| r.work_secs)
            })
        {
            return Err("production receiving report precedes completed work".into());
        }
        logistics.validate_with_incoming(
            &profile.logistics,
            stocks,
            &self.expected_incoming(profile)?,
        )
    }
}
fn valid_dt(dt: f32) -> Result<(), String> {
    if !dt.is_finite() || dt < 0.0 {
        Err("production dt must be finite and nonnegative".into())
    } else {
        Ok(())
    }
}
fn valid_fraction(fraction: f32) -> Result<(), String> {
    if !fraction.is_finite() || !(0.0..=1.0).contains(&fraction) {
        Err("production funded work fraction must be finite and within 0..1".into())
    } else {
        Ok(())
    }
}
