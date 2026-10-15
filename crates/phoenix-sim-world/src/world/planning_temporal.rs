//! Forecasts over real imported leaves. This module cannot dispatch work,
//! reserve machinery, inspect physical state or manufacture outcome evidence.
use super::{CompiledPlanning, Decomposition, PlanChoice, MAX_LEAVES};
use crate::world::activities::ActivityRequest;
use phoenix_model::identity::{StationId, SystemId};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const MAX_FORECAST_SECS: u32 = 86400;
const MAX_PHASES: usize = 16;
const MAX_PROFILES: usize = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttentionKind {
    Active,
    Monitor,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AttentionPhase {
    pub kind: AttentionKind,
    pub offset_secs: u32,
    pub duration_secs: u32,
    pub attention_permille: u16,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActivityForecast {
    pub min_secs: u32,
    pub max_secs: u32,
    pub recovery_secs: u32,
    #[serde(default, rename = "phase")]
    pub phases: Vec<AttentionPhase>,
}
impl ActivityForecast {
    pub fn validate(&self) -> Result<(), String> {
        if self.min_secs == 0
            || self.min_secs > self.max_secs
            || self
                .max_secs
                .checked_add(self.recovery_secs)
                .is_none_or(|sum| sum > MAX_FORECAST_SECS)
            || self.phases.len() > MAX_PHASES
        {
            return Err("forecast requires bounded positive min/max duration and recovery".into());
        }
        let mut end = 0;
        for phase in &self.phases {
            let next = phase
                .offset_secs
                .checked_add(phase.duration_secs)
                .ok_or("forecast phase clock overflow")?;
            if phase.duration_secs == 0
                || phase.offset_secs < end
                || next > self.max_secs
                || !(1..=1000).contains(&phase.attention_permille)
            {
                return Err(
                    "forecast phases must be ordered, disjoint and within maximum duration".into(),
                );
            }
            end = next;
        }
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DifficultyProfile {
    pub min_operators: u16,
    pub deadline_secs: u32,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TemporalPolicy {
    #[serde(rename = "profile")]
    pub profiles: Vec<DifficultyProfile>,
}
impl TemporalPolicy {
    pub fn validate(&self) -> Result<(), String> {
        if self.profiles.is_empty()
            || self.profiles.len() > MAX_PROFILES
            || self.profiles[0].min_operators != 0
            || self.profiles.iter().any(|row| {
                row.min_operators > 128
                    || row.deadline_secs == 0
                    || row.deadline_secs > MAX_FORECAST_SECS
            })
            || self
                .profiles
                .windows(2)
                .any(|rows| rows[0].min_operators >= rows[1].min_operators)
        {
            return Err("temporal profiles require ascending distinct operator thresholds beginning at zero and finite deadlines".into());
        }
        Ok(())
    }
}
/// The accepted input, not a claim about today's participation or authority.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "source", rename_all = "snake_case", deny_unknown_fields)]
pub enum OwnerInput {
    Human { station: StationId, rating: String },
    Ai {},
    Simplified {},
    Unavailable { reason: UnavailableReason },
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnavailableReason {
    MissingSource,
    Offline,
    Unoccupied,
    MissingHost,
    InvalidRating,
}
impl OwnerInput {
    fn station(&self) -> Option<&StationId> {
        match self {
            Self::Human { station, .. } => Some(station),
            _ => None,
        }
    }
    fn validate(&self) -> Result<(), String> {
        if let Self::Human { station, rating } = self {
            if !super::compile::identifier(&station.0)
                || rating.trim().is_empty()
                || rating == "Backfill"
                || rating.len() > 128
                || rating.chars().any(char::is_control)
            {
                return Err("invalid original forecast operator/rating".into());
            }
        }
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ForecastInput {
    /// Actual original Time<Fixed> period, not a later host's current rate.
    pub tick_nanos: u64,
    pub owners: BTreeMap<SystemId, OwnerInput>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ForecastInterval {
    pub start_tick: u64,
    pub end_tick: u64,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TemporalCommitment {
    pub input: ForecastInput,
    pub profile_index: u8,
    pub deadline_tick: u64,
    pub outcome: TemporalOutcome,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub enum TemporalOutcome {
    Committed {
        nodes: BTreeMap<String, ForecastInterval>,
    },
    Refused {
        reason: String,
    },
}
impl TemporalCommitment {
    pub fn nodes(&self) -> Option<&BTreeMap<String, ForecastInterval>> {
        match &self.outcome {
            TemporalOutcome::Committed { nodes } => Some(nodes),
            _ => None,
        }
    }
    pub fn refused(&self) -> bool {
        matches!(self.outcome, TemporalOutcome::Refused { .. })
    }
}
pub const MAX_RECONSIDERATIONS: usize = 8;
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ReconsiderationCause {
    CrewControlChanged { owners: Vec<SystemId> },
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TemporalReconsideration {
    pub tick: u64,
    pub cause: ReconsiderationCause,
    pub decision: TemporalCommitment,
}
impl PlanChoice {
    pub fn effective_temporal(&self) -> Option<&TemporalCommitment> {
        self.temporal_reconsiderations
            .last()
            .map(|revision| &revision.decision)
            .or(self.temporal.as_ref())
    }
}
fn changed_owners(before: &ForecastInput, after: &ForecastInput) -> Vec<SystemId> {
    before
        .owners
        .iter()
        .filter_map(|(owner, value)| {
            (after.owners.get(owner) != Some(value)).then_some(owner.clone())
        })
        .collect()
}
/// Derived minute-area summary. Exact overload checks use phase boundaries.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkloadBucket {
    pub station: StationId,
    pub minute: u16,
    /// Permille * actual nanoseconds, exact even for a partial last minute.
    pub attention_nanos: u128,
}

fn ticks(seconds: u32, nanos: u64) -> Result<u64, String> {
    if nanos == 0 {
        return Err("forecast needs its original positive timestep".into());
    }
    let result = (u128::from(seconds) * 1_000_000_000).div_ceil(u128::from(nanos));
    u64::try_from(result).map_err(|_| "forecast tick conversion overflow".into())
}
fn add(left: u64, right: u64) -> Result<u64, String> {
    left.checked_add(right)
        .ok_or("forecast logical clock overflow".into())
}
fn owner(request: &ActivityRequest) -> &SystemId {
    match request {
        ActivityRequest::Repair { owner, .. } | ActivityRequest::Delivery { owner, .. } => owner,
    }
}
fn same_resource(left: &ActivityRequest, right: &ActivityRequest) -> bool {
    match (left, right) {
        (
            ActivityRequest::Repair {
                actor: a,
                owner: ao,
                team: at,
                ..
            },
            ActivityRequest::Repair {
                actor: b,
                owner: bo,
                team: bt,
                ..
            },
        ) => a == b && ao == bo && at == bt,
        (
            ActivityRequest::Delivery {
                actor: a,
                owner: ao,
                ..
            },
            ActivityRequest::Delivery {
                actor: b,
                owner: bo,
                ..
            },
        ) => a == b && ao == bo,
        _ => false,
    }
}
type AttentionWindow = (u64, u64, u16);
fn windows(
    forecast: &ActivityForecast,
    start: u64,
    nanos: u64,
) -> Result<Vec<AttentionWindow>, String> {
    forecast
        .phases
        .iter()
        .map(|phase| {
            // Round each absolute offset up, so adjacent authored boundaries stay
            // identical rather than accumulating a separate rounded duration.
            Ok((
                add(start, ticks(phase.offset_secs, nanos)?)?,
                add(
                    start,
                    ticks(phase.offset_secs + phase.duration_secs, nanos)?,
                )?,
                phase.attention_permille,
            ))
        })
        .collect()
}
fn fits(windows: impl Iterator<Item = AttentionWindow>) -> bool {
    let mut events = BTreeMap::<u64, i32>::new();
    for (start, end, attention) in windows {
        if start == end {
            continue;
        }
        *events.entry(start).or_default() += i32::from(attention);
        *events.entry(end).or_default() -= i32::from(attention);
    }
    let mut demand = 0;
    events.values().all(|delta| {
        demand += delta;
        demand <= 1000
    })
}

impl CompiledPlanning {
    fn original_input(
        &self,
        graph: &Decomposition,
        chosen_tick: u64,
        input: &ForecastInput,
    ) -> Result<(u8, u64), String> {
        let policy = self
            .config
            .temporal
            .as_ref()
            .ok_or("plan has no temporal policy")?;
        policy.validate()?;
        if input.tick_nanos == 0
            || input.tick_nanos > 1_000_000_000
            || input.owners.len() > MAX_LEAVES
        {
            return Err("forecast input timestep/owner bound invalid".into());
        }
        let required = graph
            .leaves
            .values()
            .map(|leaf| owner(&leaf.request))
            .collect::<BTreeSet<_>>();
        if required.len() != input.owners.len()
            || required.iter().any(|id| !input.owners.contains_key(*id))
        {
            return Err("forecast needs exactly the selected actual owners".into());
        }
        let mut ratings = BTreeMap::new();
        for value in input.owners.values() {
            value.validate()?;
            if let OwnerInput::Human { station, rating } = value {
                if ratings
                    .insert(station, rating)
                    .is_some_and(|previous| previous != rating)
                {
                    return Err(
                        "one original operator cannot have conflicting accepted ratings".into(),
                    );
                }
            }
        }
        let human_count = input
            .owners
            .values()
            .filter_map(OwnerInput::station)
            .collect::<BTreeSet<_>>()
            .len();
        let profile = policy
            .profiles
            .iter()
            .rposition(|row| usize::from(row.min_operators) <= human_count)
            .ok_or("no applicable temporal profile")?;
        Ok((
            profile as u8,
            add(
                chosen_tick,
                ticks(policy.profiles[profile].deadline_secs, input.tick_nanos)?,
            )?,
        ))
    }
    pub fn schedule(
        &self,
        graph: &Decomposition,
        chosen_tick: u64,
        input: ForecastInput,
    ) -> Result<TemporalCommitment, String> {
        let (profile_index, deadline_tick) = self.original_input(graph, chosen_tick, &input)?;
        self.schedule_with_bounds(graph, chosen_tick, input, profile_index, deadline_tick)
    }
    fn schedule_with_bounds(
        &self,
        graph: &Decomposition,
        start_tick: u64,
        input: ForecastInput,
        profile_index: u8,
        deadline_tick: u64,
    ) -> Result<TemporalCommitment, String> {
        self.original_input(graph, start_tick, &input)?;
        if input
            .owners
            .values()
            .any(|value| matches!(value, OwnerInput::Unavailable { .. }))
        {
            return Err(
                "actual owner control/participation is unavailable for the original forecast"
                    .into(),
            );
        }
        if graph.forecasts.len() != graph.leaves.len() {
            return Err(
                "temporal plan requires a forecast for every selected physical leaf".into(),
            );
        }
        let mut nodes = BTreeMap::<String, ForecastInterval>::new();
        let mut attention = BTreeMap::<StationId, Vec<AttentionWindow>>::new();
        while nodes.len() < graph.leaves.len() {
            let node = graph
                .leaves
                .keys()
                .find(|node| {
                    !nodes.contains_key(*node)
                        && graph
                            .dependencies
                            .iter()
                            .filter(|edge| &edge.to == *node)
                            .all(|edge| nodes.contains_key(&edge.from))
                })
                .ok_or("temporal graph contains an unresolved dependency cycle")?;
            let forecast = &graph.forecasts[node];
            forecast.validate()?;
            let duration = ticks(forecast.max_secs + forecast.recovery_secs, input.tick_nanos)?;
            let mut earliest = start_tick;
            for edge in graph.dependencies.iter().filter(|edge| &edge.to == node) {
                earliest = earliest.max(nodes[&edge.from].end_tick);
            }
            let request = &graph.leaves[node].request;
            let station = input.owners[owner(request)].station();
            let old_windows = station
                .and_then(|id| attention.get(id))
                .map_or(&[][..], Vec::as_slice);
            // Every feasible change can begin at earliest or after a previous
            // physical/attention interval ends. No search over every sim tick.
            let mut candidates = BTreeSet::from([earliest]);
            for (previous, interval) in &nodes {
                if same_resource(request, &graph.leaves[previous].request) {
                    candidates.insert(interval.end_tick.max(earliest));
                }
            }
            for (_, end, _) in old_windows {
                for phase in &forecast.phases {
                    let offset = ticks(phase.offset_secs, input.tick_nanos)?;
                    if let Some(start) = end.checked_sub(offset).filter(|start| *start >= earliest)
                    {
                        candidates.insert(start);
                    }
                }
            }
            let mut placement = None;
            for start in candidates {
                let end = add(start, duration)?;
                if end > deadline_tick {
                    continue;
                }
                if nodes.iter().any(|(previous, interval)| {
                    same_resource(request, &graph.leaves[previous].request)
                        && start < interval.end_tick
                        && interval.start_tick < end
                }) {
                    continue;
                }
                let new_windows = windows(forecast, start, input.tick_nanos)?;
                if station.is_some()
                    && !fits(
                        old_windows
                            .iter()
                            .copied()
                            .chain(new_windows.iter().copied()),
                    )
                {
                    continue;
                }
                placement = Some((
                    ForecastInterval {
                        start_tick: start,
                        end_tick: end,
                    },
                    new_windows,
                ));
                break;
            }
            let (interval, new_windows) = placement.ok_or_else(|| {
                format!("infeasible temporal deadline/attention/resource forecast at {node}")
            })?;
            if let Some(station) = station {
                attention
                    .entry(station.clone())
                    .or_default()
                    .extend(new_windows);
            }
            nodes.insert(node.clone(), interval);
        }
        Ok(TemporalCommitment {
            input,
            profile_index,
            deadline_tick,
            outcome: TemporalOutcome::Committed { nodes },
        })
    }
    /// One historical decision, including a bounded truthful refusal. Structural
    /// input/clock errors cannot be laundered into an accepted refusal record.
    pub fn schedule_decision(
        &self,
        graph: &Decomposition,
        chosen_tick: u64,
        input: ForecastInput,
    ) -> Result<TemporalCommitment, String> {
        let (profile_index, deadline_tick) = self.original_input(graph, chosen_tick, &input)?;
        match self.schedule(graph, chosen_tick, input.clone()) {
            Ok(committed) => Ok(committed),
            Err(reason) => Ok(TemporalCommitment {
                input,
                profile_index,
                deadline_tick,
                outcome: TemporalOutcome::Refused {
                    reason: reason
                        .chars()
                        .filter(|ch| !ch.is_control())
                        .take(super::MAX_EXPLANATION_CHARS)
                        .collect(),
                },
            }),
        }
    }
    fn reconsidered_decision(
        &self,
        graph: &Decomposition,
        original: &TemporalCommitment,
        tick: u64,
        input: ForecastInput,
    ) -> Result<TemporalCommitment, String> {
        self.original_input(graph, tick, &input)?;
        if input.tick_nanos != original.input.tick_nanos {
            return Err("reconsideration cannot replace the original simulation timestep".into());
        }
        match self.schedule_with_bounds(
            graph,
            tick,
            input.clone(),
            original.profile_index,
            original.deadline_tick,
        ) {
            Ok(value) => Ok(value),
            Err(reason) => Ok(TemporalCommitment {
                input,
                profile_index: original.profile_index,
                deadline_tick: original.deadline_tick,
                outcome: TemporalOutcome::Refused {
                    reason: reason
                        .chars()
                        .filter(|ch| !ch.is_control())
                        .take(super::MAX_EXPLANATION_CHARS)
                        .collect(),
                },
            }),
        }
    }
    /// A real changed input may recover only unreleased refused work. Original
    /// methods/deadline/profile never change and no physical command runs here.
    pub fn reconsider_schedule(
        &self,
        choice: &mut PlanChoice,
        input: ForecastInput,
        tick: u64,
    ) -> Result<bool, String> {
        let graph = self.derive(choice)?;
        self.validate_schedule(choice, &graph)?;
        let Some(original) = choice.temporal.as_ref() else {
            return Ok(false);
        };
        let previous = choice
            .effective_temporal()
            .expect("original temporal exists");
        if !previous.refused()
            || !choice.released.is_empty()
            || choice.temporal_reconsiderations.len() >= MAX_RECONSIDERATIONS
        {
            return Ok(false);
        }
        let last_tick = choice
            .temporal_reconsiderations
            .last()
            .map_or(choice.chosen_tick, |row| row.tick);
        if tick <= last_tick {
            return Ok(false);
        }
        let owners = changed_owners(&previous.input, &input);
        if owners.is_empty() {
            return Ok(false);
        }
        let decision = self.reconsidered_decision(&graph, original, tick, input)?;
        choice
            .temporal_reconsiderations
            .push(TemporalReconsideration {
                tick,
                cause: ReconsiderationCause::CrewControlChanged { owners },
                decision,
            });
        Ok(true)
    }
    pub fn record_schedule_decision(
        &self,
        choice: &mut PlanChoice,
        input: ForecastInput,
    ) -> Result<(), String> {
        if choice.temporal.is_some() || !choice.released.is_empty() {
            return Err("forecast can be decided only once before mission release".into());
        }
        let graph = self.derive(choice)?;
        choice.temporal = Some(self.schedule_decision(&graph, choice.chosen_tick, input)?);
        Ok(())
    }
    pub fn commit_schedule(
        &self,
        choice: &mut PlanChoice,
        input: ForecastInput,
    ) -> Result<(), String> {
        if choice.temporal.is_some() || !choice.released.is_empty() {
            return Err("forecast can be committed only once before mission release".into());
        }
        let graph = self.derive(choice)?;
        let commitment = self.schedule(&graph, choice.chosen_tick, input)?;
        choice.temporal = Some(commitment);
        Ok(())
    }
    pub fn validate_schedule(
        &self,
        choice: &PlanChoice,
        graph: &Decomposition,
    ) -> Result<(), String> {
        match (&self.config.temporal, &choice.temporal) {
            (None, None) if choice.temporal_reconsiderations.is_empty() => Ok(()),
            (Some(_), Some(original)) => {
                if &self.schedule_decision(graph, choice.chosen_tick, original.input.clone())?
                    != original
                    || choice.temporal_reconsiderations.len() > MAX_RECONSIDERATIONS
                {
                    return Err(
                        "saved original temporal decision disagrees with source/bounds".into(),
                    );
                }
                let mut previous = original;
                let mut previous_tick = choice.chosen_tick;
                for revision in &choice.temporal_reconsiderations {
                    let owners = changed_owners(&previous.input, &revision.decision.input);
                    let ReconsiderationCause::CrewControlChanged { owners: declared } =
                        &revision.cause;
                    if !previous.refused()
                        || revision.tick <= previous_tick
                        || owners.is_empty()
                        || &owners != declared
                        || &self.reconsidered_decision(
                            graph,
                            original,
                            revision.tick,
                            revision.decision.input.clone(),
                        )? != &revision.decision
                    {
                        return Err(
                            "saved temporal reconsideration lacks its exact cause/clock/decision"
                                .into(),
                        );
                    }
                    previous = &revision.decision;
                    previous_tick = revision.tick;
                }
                if (previous.refused() && !choice.released.is_empty())
                    || choice.released.values().any(|tick| *tick < previous_tick)
                {
                    return Err("released work precedes its effective temporal commitment".into());
                }
                Ok(())
            }
            _ => Err("temporal policy/accepted commitment presence mismatch".into()),
        }
    }
    pub fn workload(&self, choice: &PlanChoice) -> Result<Vec<WorkloadBucket>, String> {
        let graph = self.derive(choice)?;
        self.validate_schedule(choice, &graph)?;
        let Some(saved) = choice.effective_temporal() else {
            return Ok(Vec::new());
        };
        let Some(nodes) = saved.nodes() else {
            return Ok(Vec::new());
        };
        let mut buckets = BTreeMap::<(StationId, u16), u128>::new();
        const MINUTE_NANOS: u128 = 60_000_000_000;
        for (node, interval) in nodes {
            let Some(station) = saved.input.owners[owner(&graph.leaves[node].request)].station()
            else {
                continue;
            };
            for (start, end, permille) in windows(
                &graph.forecasts[node],
                interval.start_tick,
                saved.input.tick_nanos,
            )? {
                let mut cursor =
                    u128::from(start - choice.chosen_tick) * u128::from(saved.input.tick_nanos);
                let finish =
                    u128::from(end - choice.chosen_tick) * u128::from(saved.input.tick_nanos);
                while cursor < finish {
                    let minute = cursor / MINUTE_NANOS;
                    let next = finish.min((minute + 1) * MINUTE_NANOS);
                    *buckets
                        .entry((
                            station.clone(),
                            u16::try_from(minute).map_err(|_| "minute forecast bound exceeded")?,
                        ))
                        .or_default() += (next - cursor) * u128::from(permille);
                    cursor = next;
                }
            }
        }
        Ok(buckets
            .into_iter()
            .map(|((station, minute), attention_nanos)| WorkloadBucket {
                station,
                minute,
                attention_nanos,
            })
            .collect())
    }
}
