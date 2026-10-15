//! One drive's causal order and paid phase frontier. No course, fuel or physics owner.
use phoenix_model::transit::*;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// One ordinary per-System tree chooses whether to begin/cancel. Numeric
/// movement, resource payment and arrival remain mechanical owners.
pub fn bindings() -> crate::ai::state_tree::SupportedBindings {
    crate::ai::state_tree::SupportedBindings {
        facts: [
            "can_operate",
            "request_present",
            "course_available",
            "preparing",
            "active",
        ]
        .map(str::to_owned)
        .into(),
        channels: ["begin", "cancel"].map(str::to_owned).into(),
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TransitOrder {
    pub destination: TransitDestination,
    #[serde(default, skip_serializing_if = "TransitWorldBinding::is_base")]
    pub original_world: TransitWorldBinding,
    pub origin: [f32; 3],
    pub profile: AcceptedLocalJumpProfile,
    pub accepted_tick: u64,
    pub request: Option<TransitRequestKey>,
}
impl TransitOrder {
    pub fn validate(&self, now: u64) -> Result<(), String> {
        self.destination.validate()?;
        self.original_world.validate()?;
        self.profile.validate()?;
        if matches!(self.destination, TransitDestination::World { .. })
            && !self.profile.config.world_travel
        {
            return Err("accepted drive does not support World routes".into());
        }
        if let Some(request) = &self.request {
            request.validate()?;
        }
        if self.accepted_tick > now
            || !finite(self.origin)
            || distance(self.origin, self.destination.point())
                > f64::from(self.profile.config.max_distance)
        {
            return Err("invalid accepted transit origin/range/clock".into());
        }
        Ok(())
    }
}

/// Original static physical inputs used by the private arrival calculation.
/// Simulation binds these IDs/policies to accepted immutable World content.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TransitGravityBody {
    pub body_id: String,
    pub position: [f32; 3],
    pub policy: ArrivalGravityConfig,
}
/// Same-order private continuation evidence, never a public gravity catalogue.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TransitArrivalSolution {
    pub scatter_offset: [f32; 3],
    pub at_center_direction: [f32; 3],
    pub gravity: Vec<TransitGravityBody>,
    pub point: [f32; 3],
}
impl TransitArrivalSolution {
    pub fn resolve(
        order: &TransitOrder,
        scatter_offset: [f32; 3],
        at_center_direction: [f32; 3],
        mut gravity: Vec<TransitGravityBody>,
    ) -> Result<Self, String> {
        gravity.sort_by(|a, b| a.body_id.cmp(&b.body_id));
        let mut receipt = Self {
            scatter_offset,
            at_center_direction,
            gravity,
            point: order.destination.point(),
        };
        receipt.point = receipt.reconstruct(order)?;
        Ok(receipt)
    }
    pub fn validate(&self, order: &TransitOrder) -> Result<(), String> {
        if self.reconstruct(order)? != self.point {
            return Err("saved arrival disagrees with its original deterministic placement".into());
        }
        Ok(())
    }
    fn reconstruct(&self, order: &TransitOrder) -> Result<[f32; 3], String> {
        order.profile.validate()?;
        order.destination.validate()?;
        if !finite(self.scatter_offset)
            || !finite(self.at_center_direction)
            || distance([0.0; 3], self.scatter_offset)
                > f64::from(order.profile.config.scatter_radius)
            || distance([0.0; 3], self.at_center_direction) == 0.0
            || self.gravity.len() > MAX_TRANSIT_GRAVITY_BODIES
            || self
                .gravity
                .windows(2)
                .any(|pair| pair[0].body_id >= pair[1].body_id)
        {
            return Err("invalid bounded scatter/static gravity provenance".into());
        }
        let destination = order.destination.point();
        let mut point = std::array::from_fn(|axis| {
            (f64::from(destination[axis]) + f64::from(self.scatter_offset[axis])) as f32
        });
        if !finite(point)
            || distance(destination, point) > f64::from(order.profile.config.scatter_radius)
        {
            return Err("scatter has no representable point inside its accepted envelope".into());
        }
        for body in &self.gravity {
            if body.body_id.trim().is_empty()
                || body.body_id.len() > 512
                || body.body_id.chars().any(char::is_control)
                || !finite(body.position)
            {
                return Err("invalid original gravity body identity/pose".into());
            }
            body.policy.validate()?;
            let separation = distance(body.position, point);
            let radius = f64::from(body.policy.exclusion_radius)
                * f64::from(order.profile.config.gravity_response);
            if separation >= radius {
                continue;
            }
            let direction: [f64; 3] = if separation == 0.0 {
                self.at_center_direction.map(f64::from)
            } else {
                std::array::from_fn(|axis| f64::from(point[axis]) - f64::from(body.position[axis]))
            };
            let length = libm::sqrt(direction.iter().map(|value| value * value).sum());
            point = std::array::from_fn(|axis| {
                (f64::from(body.position[axis]) + direction[axis] / length * radius) as f32
            });
            if !finite(point) {
                return Err("gravity placement has no finite representable result".into());
            }
        }
        if self.gravity.iter().any(|body| {
            distance(body.position, point)
                < f64::from(body.policy.exclusion_radius)
                    * f64::from(order.profile.config.gravity_response)
        }) {
            return Err(
                "deterministic placement remains inside an accepted gravity exclusion".into(),
            );
        }
        Ok(point)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HandledTransitRequest {
    pub revision: u64,
    pub destination: TransitDestination,
    pub accepted_tick: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(tag = "phase", rename_all = "snake_case", deny_unknown_fields)]
pub enum TransitPhase {
    #[default]
    Idle,
    Preparing {
        order: TransitOrder,
        worked_secs: f64,
    },
    Committed {
        order: TransitOrder,
        commit_tick: u64,
        arrival: TransitArrivalSolution,
    },
    Arrived {
        order: TransitOrder,
        commit_tick: u64,
        arrival: TransitArrivalSolution,
        arrival_tick: u64,
    },
}
impl TransitPhase {
    pub fn order(&self) -> Option<&TransitOrder> {
        match self {
            Self::Idle => None,
            Self::Preparing { order, .. }
            | Self::Committed { order, .. }
            | Self::Arrived { order, .. } => Some(order),
        }
    }
    pub fn active(&self) -> bool {
        matches!(self, Self::Preparing { .. } | Self::Committed { .. })
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TransitState {
    pub phase: TransitPhase,
    pub last_tick: Option<u64>,
    /// Highest accepted revision per actual objective. Never evict to replay old work.
    pub handled: BTreeMap<String, HandledTransitRequest>,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TransitWork {
    Prepare { seconds: f64 },
    Commit,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TransitGrant {
    Preparation { seconds: f64 },
    Commitment { paid: bool },
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransitWait {
    Unavailable,
    Unfunded,
    OutOfRange,
    NoPlacement,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TransitAdvance {
    Unchanged,
    Waiting(TransitWait),
    Prepared,
    Committed,
    Arrived { point: [f32; 3] },
}

impl TransitState {
    pub fn request_is_fresh(&self, key: &TransitRequestKey) -> bool {
        key.validate().is_ok()
            && self.handled.get(&key.objective_id).map_or(
                self.handled.len() < MAX_HANDLED_TRANSIT_OBJECTIVES,
                |held| key.revision > held.revision,
            )
    }
    pub fn begin(&mut self, order: TransitOrder, now: u64) -> Result<(), String> {
        self.validate(now)?;
        order.validate(now)?;
        if self.phase.active() || order.accepted_tick != now {
            return Err(
                "Begin requires an idle/completed actual drive and current accepted order".into(),
            );
        }
        if let Some(key) = &order.request {
            if !self.request_is_fresh(key) {
                return Err(
                    "transit request is handled/old or exceeds the technical retention bound"
                        .into(),
                );
            }
        }
        let mut next = self.clone();
        if let Some(key) = &order.request {
            next.handled.insert(
                key.objective_id.clone(),
                HandledTransitRequest {
                    revision: key.revision,
                    destination: order.destination.clone(),
                    accepted_tick: now,
                },
            );
        }
        next.phase = TransitPhase::Preparing {
            order,
            worked_secs: 0.0,
        };
        next.validate(now)?;
        *self = next;
        Ok(())
    }
    pub fn cancel(&mut self, now: u64) -> Result<(), String> {
        self.validate(now)?;
        if !matches!(self.phase, TransitPhase::Preparing { .. }) {
            return Err("Cancel only affects preparation before paid commitment".into());
        }
        self.phase = TransitPhase::Idle;
        Ok(())
    }
    /// Callbacks operate on staged resource candidates. Apply them only if this
    /// whole transition succeeds; placement never changes physical pose here.
    #[allow(clippy::too_many_arguments)]
    pub fn advance(
        &mut self,
        now: u64,
        dt: f64,
        position: [f32; 3],
        physical_owner: bool,
        operable: bool,
        operation: f32,
        mut work: impl FnMut(TransitWork, &TransitOrder) -> Result<TransitGrant, String>,
        mut placement: impl FnMut(&TransitOrder, [f32; 3]) -> Option<TransitArrivalSolution>,
    ) -> Result<TransitAdvance, String> {
        self.validate(now)?;
        if !dt.is_finite()
            || dt < 0.0
            || !finite(position)
            || !operation.is_finite()
            || !(0.0..=1.0).contains(&operation)
        {
            return Err("transit step requires finite physical time/pose/operation".into());
        }
        if self.last_tick == Some(now) {
            return Ok(TransitAdvance::Unchanged);
        }
        let mut next = self.clone();
        next.last_tick = Some(now);
        let result = match self.phase.clone() {
            TransitPhase::Idle | TransitPhase::Arrived { .. } => TransitAdvance::Unchanged,
            TransitPhase::Committed {
                order,
                commit_tick,
                arrival,
            } => {
                if !physical_owner {
                    TransitAdvance::Waiting(TransitWait::Unavailable)
                } else {
                    let point = arrival.point;
                    next.phase = TransitPhase::Arrived {
                        order,
                        commit_tick,
                        arrival,
                        arrival_tick: now,
                    };
                    TransitAdvance::Arrived { point }
                }
            }
            TransitPhase::Preparing {
                order,
                mut worked_secs,
            } => {
                if !physical_owner || !operable || operation == 0.0 {
                    TransitAdvance::Waiting(TransitWait::Unavailable)
                } else {
                    let required = f64::from(order.profile.config.preparation_work_secs);
                    let requested = (dt * f64::from(operation)).min(required - worked_secs);
                    if requested > 0.0 {
                        let TransitGrant::Preparation { seconds } =
                            work(TransitWork::Prepare { seconds: requested }, &order)?
                        else {
                            return Err("preparation received the wrong actual work receipt".into());
                        };
                        if !seconds.is_finite() || seconds < 0.0 || seconds > requested {
                            return Err(
                                "preparation work receipt exceeds the actual request".into()
                            );
                        }
                        worked_secs = (worked_secs + seconds).min(required);
                    }
                    next.phase = TransitPhase::Preparing {
                        order: order.clone(),
                        worked_secs,
                    };
                    if worked_secs < required {
                        if requested == 0.0 {
                            TransitAdvance::Unchanged
                        } else if matches!(next.phase, TransitPhase::Preparing { worked_secs: value, .. } if value == self.worked_secs())
                        {
                            TransitAdvance::Waiting(TransitWait::Unfunded)
                        } else {
                            TransitAdvance::Prepared
                        }
                    } else if distance(position, order.destination.point())
                        > f64::from(order.profile.config.max_distance)
                    {
                        TransitAdvance::Waiting(TransitWait::OutOfRange)
                    } else if let Some(arrival) = placement(&order, position)
                        .filter(|arrival| arrival.validate(&order).is_ok())
                    {
                        let TransitGrant::Commitment { paid } = work(TransitWork::Commit, &order)?
                        else {
                            return Err("commitment received the wrong actual work receipt".into());
                        };
                        if paid {
                            next.phase = TransitPhase::Committed {
                                order,
                                commit_tick: now,
                                arrival,
                            };
                            TransitAdvance::Committed
                        } else {
                            TransitAdvance::Waiting(TransitWait::Unfunded)
                        }
                    } else {
                        TransitAdvance::Waiting(TransitWait::NoPlacement)
                    }
                }
            }
        };
        next.validate(now)?;
        *self = next;
        Ok(result)
    }
    pub fn worked_secs(&self) -> f64 {
        match self.phase {
            TransitPhase::Preparing { worked_secs, .. } => worked_secs,
            _ => 0.0,
        }
    }
    pub fn validate(&self, now: u64) -> Result<(), String> {
        if self.handled.len() > MAX_HANDLED_TRANSIT_OBJECTIVES
            || self.last_tick.is_some_and(|tick| tick > now)
        {
            return Err("invalid transit retention/step frontier".into());
        }
        for (id, receipt) in &self.handled {
            TransitRequestKey {
                objective_id: id.clone(),
                revision: receipt.revision,
            }
            .validate()?;
            receipt.destination.validate()?;
            if receipt.accepted_tick > now {
                return Err("future transit request receipt".into());
            }
        }
        let Some(order) = self.phase.order() else {
            return Ok(());
        };
        order.validate(now)?;
        if let Some(key) = &order.request {
            let held = self
                .handled
                .get(&key.objective_id)
                .ok_or("accepted transit request lacks its own receipt")?;
            if held.revision != key.revision
                || held.destination != order.destination
                || held.accepted_tick != order.accepted_tick
            {
                return Err("accepted transit request changed meaning or clock".into());
            }
        }
        match &self.phase {
            TransitPhase::Preparing { worked_secs, .. } => {
                if !worked_secs.is_finite()
                    || *worked_secs < 0.0
                    || *worked_secs > f64::from(order.profile.config.preparation_work_secs)
                    || (*worked_secs > 0.0
                        && self.last_tick.is_none_or(|tick| tick < order.accepted_tick))
                {
                    return Err("invalid paid preparation frontier".into());
                }
            }
            TransitPhase::Committed {
                commit_tick,
                arrival,
                ..
            }
            | TransitPhase::Arrived {
                commit_tick,
                arrival,
                ..
            } => {
                if *commit_tick < order.accepted_tick
                    || *commit_tick > now
                    || arrival.validate(order).is_err()
                    || self.last_tick.is_none_or(|tick| tick < *commit_tick)
                {
                    return Err("invalid paid commitment frontier".into());
                }
                if let TransitPhase::Arrived { arrival_tick, .. } = &self.phase {
                    if arrival_tick <= commit_tick
                        || *arrival_tick > now
                        || self.last_tick.is_none_or(|tick| tick < *arrival_tick)
                    {
                        return Err("invalid one-shot arrival frontier".into());
                    }
                }
            }
            TransitPhase::Idle => unreachable!(),
        }
        Ok(())
    }
}
fn finite(point: [f32; 3]) -> bool {
    point.into_iter().all(f32::is_finite)
}
fn distance(a: [f32; 3], b: [f32; 3]) -> f64 {
    libm::sqrt(
        a.into_iter()
            .zip(b)
            .map(|(a, b)| {
                let delta = f64::from(a) - f64::from(b);
                delta * delta
            })
            .sum(),
    )
}

#[cfg(test)]
#[path = "transit_tests.rs"]
mod tests;
