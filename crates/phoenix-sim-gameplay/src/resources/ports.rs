//! Physical-visit quotas and approved paid lot work, never another stock owner.
use phoenix_model::messages::SystemId;
pub use phoenix_model::ports::*;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PortVisit {
    pub visit: u64,
    pub delivered: BTreeMap<String, i64>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PortVisitState {
    /// Tick zero is a real simulation step, distinct from an unvisited owner.
    pub initialized: bool,
    pub last_tick: u64,
    pub visitors: BTreeMap<String, PortVisit>,
}

impl PortVisitState {
    /// The caller supplies actual coupled visitors, not observed contacts.
    /// Undocking removes visit permission; re-entry starts a new bounded visit.
    pub fn refresh(
        &mut self,
        profile: &PortServiceConfig,
        tick: u64,
        visitors: &BTreeSet<String>,
    ) -> Result<(), String> {
        self.validate(profile, tick.max(self.last_tick))?;
        if (self.initialized && tick < self.last_tick) || visitors.iter().any(|id| !valid_uuid(id))
        {
            return Err("port visit refresh requires current tick and actual visitor UUIDs".into());
        }
        if self.initialized && tick == self.last_tick {
            if visitors != &self.visitors.keys().cloned().collect() {
                return Err("port visit membership already refreshed for this tick".into());
            }
            return Ok(());
        }
        let mut next = self.clone();
        next.initialized = true;
        next.last_tick = tick;
        next.visitors.retain(|id, _| visitors.contains(id));
        for id in visitors {
            next.visitors
                .entry(id.clone())
                .or_insert_with(|| PortVisit {
                    visit: tick.max(1),
                    delivered: profile
                        .offers
                        .iter()
                        .map(|offer| (offer.id.clone(), 0))
                        .collect(),
                });
        }
        *self = next;
        Ok(())
    }

    pub fn remaining(
        &self,
        profile: &PortServiceConfig,
        visitor: &str,
        visit: u64,
        offer: &str,
    ) -> Result<i64, String> {
        self.validate(profile, self.last_tick)?;
        let current = self
            .visitors
            .get(visitor)
            .filter(|current| current.visit == visit)
            .ok_or("port permission belongs to a different or ended physical visit")?;
        let terms = profile.offer(offer).ok_or("unknown port offer")?;
        Ok(terms.quota - current.delivered[offer])
    }

    /// Stage this debit together with all actual goods/cost stock owners.
    /// Calling this alone does not perform or authorise any resource transfer.
    pub fn record_delivery(
        &mut self,
        profile: &PortServiceConfig,
        visitor: &str,
        visit: u64,
        offer: &str,
        lots: i64,
    ) -> Result<(), String> {
        let remaining = self.remaining(profile, visitor, visit, offer)?;
        if lots < 0 || lots > remaining {
            return Err("delivered port lots exceed this visitor's remaining quota".into());
        }
        let current = self.visitors.get_mut(visitor).unwrap();
        *current.delivered.get_mut(offer).unwrap() += lots;
        Ok(())
    }

    pub fn validate(&self, profile: &PortServiceConfig, snapshot_tick: u64) -> Result<(), String> {
        profile.validate()?;
        if !self.initialized && (self.last_tick != 0 || !self.visitors.is_empty()) {
            return Err("uninitialized port provider cannot carry visits or a frontier".into());
        }
        if self.last_tick > snapshot_tick {
            return Err("port provider frontier exceeds snapshot tick".into());
        }
        let keys: BTreeSet<_> = profile.offers.iter().map(|offer| &offer.id).collect();
        for (uuid, visit) in &self.visitors {
            if !valid_uuid(uuid)
                || visit.visit == 0
                || visit.visit > self.last_tick.max(1)
                || visit.delivered.keys().collect::<BTreeSet<_>>() != keys
                || profile.offers.iter().any(|offer| {
                    visit.delivered[&offer.id] < 0 || visit.delivered[&offer.id] > offer.quota
                })
            {
                return Err(
                    "port provider has an invalid visitor, visit or delivered-lot quota".into(),
                );
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PortOrderLineProgress {
    pub offer: String,
    pub store: String,
    pub requested_lots: i64,
    pub delivered_lots: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PortOutcome {
    pub tick: u64,
    pub line: Option<usize>,
    pub offered_lots: i64,
    pub moved_lots: i64,
    pub settled: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PortOrderState {
    pub partner: Option<String>,
    pub visit: Option<u64>,
    pub lines: Vec<PortOrderLineProgress>,
    pub active: bool,
    /// Paid fraction of one lot on the first unfinished line, never goods.
    pub fraction: f64,
    pub last_tick: u64,
    pub last_outcome: Option<PortOutcome>,
}

impl PortOrderState {
    /// Admission establishes human/trusted-mission approval and actual partner
    /// permission. This closed planner validates terms, not command authority.
    pub fn approve(
        &mut self,
        profile: &PortServiceConfig,
        partner: &str,
        visit: u64,
        lines: Vec<PortOrderLine>,
    ) -> Result<(), String> {
        profile.validate()?;
        self.validate_settlement()?;
        if !valid_uuid(partner) || visit == 0 || lines.is_empty() {
            return Err("port order requires a physical partner visit and explicit lines".into());
        }
        let mut seen = BTreeSet::new();
        let mut progress = Vec::new();
        for line in lines {
            let offer = profile.offer(&line.offer).ok_or("unknown port offer")?;
            if line.store.trim().is_empty()
                || line.lots <= 0
                || line.lots > offer.quota
                || !seen.insert(line.offer.clone())
            {
                return Err("port order requires unique offers, actual receiving stores and positive lots within quota".into());
            }
            progress.push(PortOrderLineProgress {
                offer: line.offer,
                store: line.store,
                requested_lots: line.lots,
                delivered_lots: 0,
            });
        }
        self.partner = Some(partner.into());
        self.visit = Some(visit);
        self.lines = progress;
        self.active = true;
        self.fraction = 0.0;
        if self.last_tick > 0 {
            self.last_outcome = Some(PortOutcome {
                tick: self.last_tick,
                line: None,
                offered_lots: 0,
                moved_lots: 0,
                settled: true,
            });
        }
        Ok(())
    }

    /// Cancels future work. A current real transfer receipt must still settle.
    pub fn stop(&mut self) {
        self.active = false;
        self.fraction = 0.0;
    }

    pub fn current_line(&self) -> Option<(usize, &PortOrderLineProgress)> {
        self.lines
            .iter()
            .enumerate()
            .find(|(_, line)| line.delivered_lots < line.requested_lots)
    }

    /// The caller preflights independent actual supplies/costs/space and stages
    /// paid energy together with this state. Unit rate is the actual pipe rate.
    #[allow(clippy::too_many_arguments)]
    pub fn advance<F>(
        &mut self,
        profile: &PortServiceConfig,
        system: &SystemId,
        tick: u64,
        dt: f32,
        resource_units_per_sec: f32,
        available_lots: i64,
        mut work: F,
    ) -> Result<i64, String>
    where
        F: FnMut(&SystemId, f32) -> Result<f32, String>,
    {
        self.validate(profile, tick.max(self.last_tick))?;
        if !dt.is_finite()
            || dt < 0.0
            || !resource_units_per_sec.is_finite()
            || resource_units_per_sec <= 0.0
            || available_lots < 0
        {
            return Err("port work requires finite nonnegative time, positive finite unit rate and nonnegative lot supply".into());
        }
        if tick <= self.last_tick {
            return Ok(0);
        }
        let mut next = self.clone();
        next.last_tick = tick;
        let mut offered = 0;
        let current = self
            .current_line()
            .map(|(index, line)| (index, line.clone()));
        if let Some((_, line)) = &current {
            if self.active && available_lots > 0 && dt > 0.0 {
                let limit = available_lots.min(line.requested_lots - line.delivered_lots);
                let units = profile.offer(&line.offer).unwrap().unit_amount;
                let rate = f64::from(resource_units_per_sec) / units as f64;
                let seconds = f64::from(dt).min(((limit as f64 - self.fraction) / rate).max(0.0));
                let mut seconds_f32 = seconds as f32;
                if f64::from(seconds_f32) > seconds {
                    seconds_f32 = f32::from_bits(seconds_f32.to_bits() - 1);
                }
                let possible = self.fraction + rate * f64::from(seconds_f32);
                if !possible.is_finite() || possible >= i64::MAX as f64 {
                    return Err("port work exceeds bounded integer lots".into());
                }
                if seconds_f32 > 0.0 {
                    let funded = work(system, seconds_f32)?;
                    if !funded.is_finite() || !(0.0..=1.0).contains(&funded) {
                        return Err(
                            "port funded work fraction must be finite and within 0..1".into()
                        );
                    }
                    let produced =
                        self.fraction + rate * f64::from(seconds_f32) * f64::from(funded);
                    offered = (produced.floor() as i64).min(limit);
                    next.fraction = produced.fract();
                }
            }
        }
        next.last_outcome = Some(PortOutcome {
            tick,
            line: current.map(|(index, _)| index),
            offered_lots: offered,
            moved_lots: 0,
            settled: offered == 0,
        });
        *self = next;
        Ok(offered)
    }

    pub fn record_receipt(&mut self, tick: u64, moved: i64) -> Result<(), String> {
        let receipt = self
            .last_outcome
            .as_ref()
            .ok_or("port receipt has no work offer")?;
        if tick != self.last_tick
            || receipt.tick != tick
            || moved < 0
            || moved > receipt.offered_lots
            || (receipt.settled && moved != receipt.moved_lots)
        {
            return Err("port receipt differs from its actual lot work offer".into());
        }
        if receipt.settled {
            return Ok(());
        }
        let index = receipt
            .line
            .ok_or("port positive work offer has no order line")?;
        let line = self
            .lines
            .get(index)
            .ok_or("port receipt references an absent order line")?;
        let delivered = line
            .delivered_lots
            .checked_add(moved)
            .filter(|value| *value <= line.requested_lots)
            .ok_or("port receipt exceeds approved order lots")?;
        self.lines[index].delivered_lots = delivered;
        // Paid work belongs to this offer's lot size, never the next offer.
        if delivered == self.lines[index].requested_lots {
            self.fraction = 0.0;
        }
        let receipt = self.last_outcome.as_mut().unwrap();
        receipt.moved_lots = moved;
        receipt.settled = true;
        if self.current_line().is_none() {
            self.active = false;
            self.fraction = 0.0;
        }
        Ok(())
    }

    pub fn validate(&self, profile: &PortServiceConfig, snapshot_tick: u64) -> Result<(), String> {
        profile.validate()?;
        self.validate_settlement()?;
        if self.last_tick == 0
            && (self.fraction != 0.0 || self.lines.iter().any(|line| line.delivered_lots != 0))
        {
            return Err("unstarted port order cannot contain paid work or delivered lots".into());
        }
        if self.last_tick > snapshot_tick
            || !self.fraction.is_finite()
            || !(0.0..1.0).contains(&self.fraction)
        {
            return Err("port order has an invalid work frontier or paid fraction".into());
        }
        match (&self.partner, self.visit) {
            (None, None) if self.lines.is_empty() && !self.active && self.fraction == 0.0 => {}
            (Some(partner), Some(visit))
                if valid_uuid(partner)
                    && visit > 0
                    && visit <= snapshot_tick
                    && !self.lines.is_empty() => {}
            _ => return Err("port order has an invalid partner, visit or selection".into()),
        }
        let mut seen = BTreeSet::new();
        for line in &self.lines {
            let offer = profile
                .offer(&line.offer)
                .ok_or("port order references an unknown offer")?;
            if line.store.trim().is_empty()
                || !seen.insert(&line.offer)
                || line.requested_lots <= 0
                || line.requested_lots > offer.quota
                || line.delivered_lots < 0
                || line.delivered_lots > line.requested_lots
            {
                return Err(
                    "port order line has invalid receiving store or approved/delivered lots".into(),
                );
            }
        }
        if (self.active || self.fraction > 0.0) && self.current_line().is_none() {
            return Err("completed port order cannot retain active work".into());
        }
        if !self.active && self.fraction != 0.0 {
            return Err("cancelled port order cannot retain future paid work".into());
        }
        if let Some(receipt) = &self.last_outcome {
            if let Some(index) = receipt.line {
                let line = self
                    .lines
                    .get(index)
                    .ok_or("port receipt references an absent order line")?;
                if receipt.offered_lots > line.requested_lots
                    || receipt.moved_lots > line.delivered_lots
                {
                    return Err("port receipt exceeds its actual order line".into());
                }
            } else if receipt.offered_lots != 0 || receipt.moved_lots != 0 {
                return Err("positive port receipt requires an order line".into());
            }
        }
        Ok(())
    }

    pub fn validate_settlement(&self) -> Result<(), String> {
        if (self.last_tick == 0) != self.last_outcome.is_none()
            || self.last_outcome.as_ref().is_some_and(|outcome| {
                outcome.tick != self.last_tick
                    || outcome.offered_lots < 0
                    || outcome.moved_lots < 0
                    || outcome.moved_lots > outcome.offered_lots
                    || !outcome.settled
            })
        {
            Err("port lot work has an invalid or unsettled receipt".into())
        } else {
            Ok(())
        }
    }
}

fn valid_uuid(id: &str) -> bool {
    uuid::Uuid::parse_str(id).is_ok()
}

#[cfg(test)]
#[path = "ports_tests.rs"]
mod tests;
