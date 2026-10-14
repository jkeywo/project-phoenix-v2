use super::*;
use phoenix_model::{
    interior::{RouteCursor, RouteLocation},
    job::{JobId, JobOutcome},
    messages::SystemId,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryStage {
    Loading,
    Travelling,
    Unloading,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeliveryBatch {
    pub id: JobId,
    pub source: String,
    pub original_destination: String,
    pub destination: String,
    pub amount: i64,
    pub born_tick: u64,
    pub navigation: RouteCursor,
    pub stage: DeliveryStage,
    pub work_progress: f32,
    pub return_requested: bool,
    pub returning: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeliveryOutcome {
    pub id: JobId,
    pub outcome: JobOutcome,
    pub destination: String,
    pub amount: i64,
    pub returned: bool,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LogisticsState {
    pub owner: SystemId,
    pub enabled: bool,
    pub next_id: u64,
    pub last_tick: u64,
    pub goals: BTreeMap<String, i64>,
    pub floors: BTreeMap<String, i64>,
    pub batches: BTreeMap<JobId, DeliveryBatch>,
    pub last_outcome: Option<DeliveryOutcome>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeliveryBlock {
    Disabled,
    Access,
    ReturnCapacity,
    Receiving,
}
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LogisticsTickResult {
    pub completed: Vec<DeliveryOutcome>,
    pub blocked: Vec<(JobId, DeliveryBlock)>,
}

impl LogisticsState {
    pub fn new(profile: &CompiledLogistics) -> Self {
        Self {
            owner: profile.handling().system.clone(),
            enabled: true,
            next_id: 1,
            last_tick: 0,
            goals: profile
                .config
                .stores
                .iter()
                .map(|s| (s.id.clone(), s.target))
                .collect(),
            floors: profile
                .config
                .stores
                .iter()
                .map(|s| (s.id.clone(), s.reserve))
                .collect(),
            batches: BTreeMap::new(),
            last_outcome: None,
        }
    }
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }
    pub fn set_target(
        &mut self,
        profile: &CompiledLogistics,
        store: &str,
        value: i64,
    ) -> Result<(), String> {
        self.check_amount(profile, store, value)?;
        self.goals.insert(store.into(), value);
        Ok(())
    }
    pub fn set_reserve(
        &mut self,
        profile: &CompiledLogistics,
        store: &str,
        value: i64,
    ) -> Result<(), String> {
        self.check_amount(profile, store, value)?;
        self.floors.insert(store.into(), value);
        Ok(())
    }
    fn check_amount(
        &self,
        profile: &CompiledLogistics,
        store: &str,
        value: i64,
    ) -> Result<(), String> {
        if value < 0 || value > profile.endpoints[&profile.store(store)?.endpoint].ceiling {
            return Err("stock intent exceeds actual endpoint capacity".into());
        }
        Ok(())
    }
    pub fn claim(&self, id: JobId) -> String {
        format!("logistics/{}/{}", self.owner.0, id.0)
    }
    pub fn dispatch(
        &mut self,
        profile: &CompiledLogistics,
        tick: u64,
        source: &str,
        destination: &str,
        requested: i64,
        mut transaction: impl FnMut(&LogisticsStockAction) -> Result<i64, String>,
    ) -> Result<JobId, String> {
        if self.owner != profile.handling().system
            || !self.enabled
            || self.batches.len() >= usize::from(profile.handling().max_active)
            || self.next_id == 0
            || self.next_id == u64::MAX
            || tick < self.last_tick
            || tick > self.last_tick.saturating_add(1)
            || requested <= 0
        {
            return Err("logistics dispatch unavailable or invalid".into());
        }
        let from = profile.store(source)?;
        let to = profile.store(destination)?;
        if from.role != StoreRole::Main
            || to.role != StoreRole::Local
            || to.source.as_deref() != Some(source)
            || from.resource != to.resource
        {
            return Err("delivery must follow the approved main-to-local source binding".into());
        }
        let id = JobId(self.next_id);
        let requested = requested.min(profile.handling().max_batch);
        let amount = transaction(&LogisticsStockAction::Dispatch {
            source: source.into(),
            destination: destination.into(),
            requested,
            floor: *self
                .floors
                .get(source)
                .ok_or("missing saved source reserve")?,
            claim: self.claim(id),
        })?;
        if amount <= 0 || amount > requested {
            return Err(
                "delivery has no committed quantity or callback exceeded its request".into(),
            );
        }
        self.next_id += 1;
        self.batches.insert(
            id,
            DeliveryBatch {
                id,
                source: source.into(),
                original_destination: destination.into(),
                destination: destination.into(),
                amount,
                born_tick: tick,
                navigation: RouteCursor {
                    location: RouteLocation::Compartment {
                        compartment: from.compartment.clone(),
                    },
                    destination: None,
                    route: Vec::new(),
                    blocked: false,
                },
                stage: DeliveryStage::Loading,
                work_progress: 0.0,
                return_requested: false,
                returning: false,
            },
        );
        Ok(id)
    }
    pub fn request_return(&mut self, id: JobId) -> Result<(), String> {
        self.batches
            .get_mut(&id)
            .ok_or("unknown active delivery")?
            .return_requested = true;
        Ok(())
    }
    pub fn incoming_for(&self, store: &str) -> i64 {
        self.batches
            .values()
            .filter(|b| b.destination == store)
            .map(|b| b.amount)
            .fold(0i64, i64::saturating_add)
    }
    /// A caller stages Stock/Power owners for the whole operation; callback
    /// failure must discard that staged transaction as well as this candidate.
    pub fn tick(
        &mut self,
        profile: &CompiledLogistics,
        tick: u64,
        dt: f32,
        blocked: &BTreeSet<String>,
        mut transaction: impl FnMut(&LogisticsStockAction) -> Result<i64, String>,
        mut work: impl FnMut(&SystemId, f32) -> Result<f32, String>,
    ) -> Result<LogisticsTickResult, String> {
        if !dt.is_finite() || dt < 0.0 || self.owner != profile.handling().system {
            return Err("invalid logistics step".into());
        }
        if tick <= self.last_tick {
            return Ok(LogisticsTickResult::default());
        }
        let mut staged = self.clone();
        let mut result = LogisticsTickResult::default();
        let ids: Vec<_> = staged.batches.keys().copied().collect();
        for id in ids {
            let claim = staged.claim(id);
            let batch = staged.batches.get_mut(&id).expect("stable job");
            if tick <= batch.born_tick {
                continue;
            }
            if batch.return_requested && !batch.returning {
                let accepted = transaction(&LogisticsStockAction::Retarget {
                    previous: batch.destination.clone(),
                    destination: batch.source.clone(),
                    amount: batch.amount,
                    claim: claim.clone(),
                })?;
                if accepted == batch.amount {
                    batch.destination = batch.source.clone();
                    batch.returning = true;
                    batch.stage = DeliveryStage::Travelling;
                    batch.work_progress = 0.0;
                } else if accepted == 0 {
                    result.blocked.push((id, DeliveryBlock::ReturnCapacity));
                    // Only the already committed crossing may finish while the
                    // supported return waits for compatible receiving space.
                    if staged.enabled {
                        if let RouteLocation::Crossing {
                            connection,
                            to,
                            elapsed,
                            ..
                        } = &batch.navigation.location
                        {
                            let to = to.clone();
                            let remaining = profile
                                .graph
                                .config
                                .connections
                                .iter()
                                .find(|c| c.id == *connection)
                                .ok_or("unknown committed delivery connection")?
                                .travel_secs
                                - *elapsed;
                            advance(
                                profile,
                                batch,
                                &to,
                                dt.min(remaining),
                                blocked,
                                &staged.owner,
                                &mut work,
                            )?;
                        }
                    }
                    continue;
                } else {
                    return Err("return callback changed the batch quantity".into());
                }
            }
            if !staged.enabled {
                result.blocked.push((id, DeliveryBlock::Disabled));
                continue;
            }
            let destination = profile.store(&batch.destination)?;
            let complete = match batch.stage {
                DeliveryStage::Loading | DeliveryStage::Unloading => {
                    let duration = batch.amount as f32 / profile.handling().units_per_sec;
                    let eligible = dt.min((duration - batch.work_progress).max(0.0));
                    if eligible > 0.0 {
                        batch.work_progress = (batch.work_progress
                            + eligible * fund(&staged.owner, eligible, &mut work)?)
                        .min(duration);
                    }
                    if batch.work_progress < duration {
                        false
                    } else if batch.stage == DeliveryStage::Loading {
                        batch.stage = DeliveryStage::Travelling;
                        batch.work_progress = 0.0;
                        false
                    } else {
                        let accepted = transaction(&LogisticsStockAction::Receive {
                            destination: batch.destination.clone(),
                            amount: batch.amount,
                            claim: claim.clone(),
                        })?;
                        if accepted == 0 {
                            result.blocked.push((id, DeliveryBlock::Receiving));
                            false
                        } else if accepted == batch.amount {
                            true
                        } else {
                            return Err("arrival callback changed the batch quantity".into());
                        }
                    }
                }
                DeliveryStage::Travelling => {
                    if advance(
                        profile,
                        batch,
                        &destination.compartment,
                        dt,
                        blocked,
                        &staged.owner,
                        &mut work,
                    )? {
                        batch.stage = DeliveryStage::Unloading;
                        batch.work_progress = 0.0;
                    } else if batch.navigation.blocked {
                        result.blocked.push((id, DeliveryBlock::Access));
                    }
                    false
                }
            };
            if complete {
                let outcome = DeliveryOutcome {
                    id,
                    outcome: if batch.returning {
                        JobOutcome::Cancelled
                    } else {
                        JobOutcome::Completed
                    },
                    destination: batch.destination.clone(),
                    amount: batch.amount,
                    returned: batch.returning,
                };
                staged.batches.remove(&id);
                staged.last_outcome = Some(outcome.clone());
                result.completed.push(outcome);
            }
        }
        staged.last_tick = tick;
        *self = staged;
        Ok(result)
    }

    pub fn validate(
        &self,
        profile: &CompiledLogistics,
        stocks: &BTreeMap<ResourceEndpoint, EndpointStock>,
    ) -> Result<(), String> {
        profile.validate_stocks(stocks)?;
        let keys: BTreeSet<_> = profile.config.stores.iter().map(|s| s.id.clone()).collect();
        if self.owner != profile.handling().system
            || self.next_id == 0
            || self.batches.len() > usize::from(profile.handling().max_active)
            || self.goals.keys().cloned().collect::<BTreeSet<_>>() != keys
            || self.floors.keys().cloned().collect::<BTreeSet<_>>() != keys
        {
            return Err(
                "logistics continuation identity or goal set differs from authored profile".into(),
            );
        }
        for (store, value) in self.goals.iter().chain(&self.floors) {
            self.check_amount(profile, store, *value)?;
        }
        let mut expected: BTreeMap<ResourceEndpoint, BTreeMap<String, i64>> = BTreeMap::new();
        for (id, batch) in &self.batches {
            let from = profile.store(&batch.source)?;
            let original = profile.store(&batch.original_destination)?;
            let to = profile.store(&batch.destination)?;
            let duration = batch.amount as f32 / profile.handling().units_per_sec;
            if id != &batch.id
                || id.0 == 0
                || id.0 >= self.next_id
                || batch.amount <= 0
                || batch.amount > profile.handling().max_batch
                || batch.born_tick > self.last_tick.saturating_add(1)
                || from.role != StoreRole::Main
                || original.role != StoreRole::Local
                || original.source.as_deref() != Some(&from.id)
                || from.resource != original.resource
                || (if batch.returning {
                    &batch.source
                } else {
                    &batch.original_destination
                }) != &batch.destination
                || (batch.returning && !batch.return_requested)
                || !batch.work_progress.is_finite()
                || batch.work_progress < 0.0
                || batch.work_progress > duration
                || (batch.stage == DeliveryStage::Travelling && batch.work_progress != 0.0)
            {
                return Err("invalid saved delivery quantity, stage or source binding".into());
            }
            profile.graph.validate_cursor(&batch.navigation)?;
            if matches!(batch.navigation.location, RouteLocation::External { .. }) {
                return Err("internal cargo cannot use an external team location".into());
            }
            if let RouteLocation::Compartment { compartment } = &batch.navigation.location {
                if batch.stage == DeliveryStage::Loading && compartment != &from.compartment
                    || batch.stage == DeliveryStage::Unloading && compartment != &to.compartment
                {
                    return Err("delivery work is not in its physical compartment".into());
                }
            } else if batch.stage != DeliveryStage::Travelling {
                return Err("loading/unloading cannot happen while crossing".into());
            }
            expected
                .entry(to.endpoint.clone())
                .or_default()
                .insert(self.claim(*id), batch.amount);
        }
        // In 12.1 Logistics is the sole producer of persistent incoming claims.
        // Production (12.3) must add its real owner to combined claim preflight
        // before another namespace is admitted; silently ignoring it is unsafe.
        for store in &profile.config.stores {
            if stocks[&store.endpoint].stock.incoming
                != expected.remove(&store.endpoint).unwrap_or_default()
            {
                return Err("delivery transit and incoming stock claims disagree".into());
            }
        }
        if let Some(last) = &self.last_outcome {
            if last.id.0 == 0
                || last.id.0 >= self.next_id
                || self.batches.contains_key(&last.id)
                || last.amount <= 0
                || last.amount > profile.handling().max_batch
                || profile.store(&last.destination).is_err()
                || last.outcome
                    != (if last.returned {
                        JobOutcome::Cancelled
                    } else {
                        JobOutcome::Completed
                    })
            {
                return Err("invalid saved delivery outcome".into());
            }
        }
        Ok(())
    }
}
fn fund(
    owner: &SystemId,
    dt: f32,
    work: &mut impl FnMut(&SystemId, f32) -> Result<f32, String>,
) -> Result<f32, String> {
    let fraction = work(owner, dt)?;
    if !fraction.is_finite() || !(0.0..=1.0).contains(&fraction) {
        return Err("invalid paid logistics work fraction".into());
    }
    Ok(fraction)
}
fn advance(
    profile: &CompiledLogistics,
    batch: &mut DeliveryBatch,
    destination: &str,
    dt: f32,
    blocked: &BTreeSet<String>,
    owner: &SystemId,
    work: &mut impl FnMut(&SystemId, f32) -> Result<f32, String>,
) -> Result<bool, String> {
    let mut plan = batch.navigation.clone();
    let (arrived, eligible) = profile
        .graph
        .advance_with_time(&mut plan, destination, dt, blocked);
    if eligible == 0.0 {
        batch.navigation = plan;
        return Ok(arrived);
    }
    let fraction = fund(owner, eligible, work)?;
    if fraction == 1.0 {
        // The exact staged route was fully funded. Replaying a summed duration
        // through several f32 subtractions can otherwise leave a false tiny leg.
        batch.navigation = plan;
        return Ok(arrived);
    }
    Ok(profile
        .graph
        .advance_with_time(
            &mut batch.navigation,
            destination,
            eligible * fraction,
            blocked,
        )
        .0)
}
