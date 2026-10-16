//! Optional leaf view of the SAME external spans. Never installs systems,
//! conditions, wrappers or new clocks. Its aggregation cost is diagnostic.
use super::{App, BTreeMap, Duration, FixedUpdate, Interval, RawInvocation, Serialize};
use crate::sim_sets::FixedStep;
use bevy::ecs::schedule::SystemSet;
use std::any::Any;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct LeafScheduleRow {
    /// Local graph identity, not a stable identity across builds.
    pub node: String,
    pub name: String,
    /// Exact typed memberships. Empty/multiple memberships are not guessed.
    pub fixed_steps: Vec<String>,
}

fn observe(app: &App) -> Result<Vec<LeafScheduleRow>, String> {
    let schedule = app.get_schedule(FixedUpdate).ok_or("missing FixedUpdate")?;
    let graph = schedule.graph();
    let mut sets = Vec::new();
    for (_, set, _) in graph.system_sets.iter() {
        if let Some(step) = (set as &dyn Any).downcast_ref::<FixedStep>() {
            let members = graph
                .systems_in_set(step.intern())
                .map_err(|error| format!("leaf membership unavailable: {error:?}"))?;
            sets.push((format!("{step:?}"), members));
        }
    }
    sets.sort_by(|a, b| a.0.cmp(&b.0));
    let mut rows = Vec::new();
    for (id, system) in schedule.systems().map_err(|error| format!("{error:?}"))? {
        rows.push(LeafScheduleRow {
            node: format!("{id:?}"),
            name: system.name().as_string(),
            fixed_steps: sets
                .iter()
                .filter(|(_, members)| members.contains(&id))
                .map(|(label, _)| label.clone())
                .collect(),
        });
    }
    rows.sort_by(|a, b| (&a.name, &a.node).cmp(&(&b.name, &b.node)));
    Ok(rows)
}

fn attribution(rows: &[LeafScheduleRow]) -> BTreeMap<String, Option<String>> {
    let mut names: BTreeMap<String, Vec<&LeafScheduleRow>> = BTreeMap::new();
    for row in rows {
        names.entry(row.name.clone()).or_default().push(row);
    }
    names
        .into_iter()
        .map(|(name, rows)| {
            let label = if rows.len() == 1 && rows[0].fixed_steps.len() == 1 {
                Some(rows[0].fixed_steps[0].clone())
            } else {
                None
            };
            (name, label)
        })
        .collect()
}

#[derive(Debug, Serialize)]
pub struct LeafTiming {
    /// None retains unclassified, duplicate, multiple-set or nested work.
    pub fixed_step: Option<String>,
    pub observed_ms: f64,
    pub envelope_ms: f64,
    pub intervals: usize,
}
#[derive(Debug, Serialize)]
pub struct LeafInvocation {
    pub id: u64,
    pub elapsed_ms: f64,
    pub groups: Vec<LeafTiming>,
    pub unattributed_names: BTreeMap<String, usize>,
}

fn reduce(
    raw: &RawInvocation,
    membership: &BTreeMap<String, Option<String>>,
    span_counts: &BTreeMap<String, usize>,
) -> LeafInvocation {
    let mut groups: BTreeMap<Option<String>, Vec<Interval>> = BTreeMap::new();
    let mut unattributed_names = BTreeMap::new();
    for system in &raw.systems {
        let label =
            if system.unattributed || system.ambiguous || span_counts.get(&system.name) != Some(&1)
            {
                None
            } else {
                membership.get(&system.name).cloned().flatten()
            };
        if label.is_none() {
            *unattributed_names.entry(system.name.clone()).or_default() += 1;
        }
        groups.entry(label).or_default().push(system.interval);
    }
    let groups = groups
        .into_iter()
        .map(|(fixed_step, mut intervals)| {
            intervals.sort_by_key(|interval| (interval.start, interval.end));
            let first = intervals[0];
            let end = intervals.iter().map(|interval| interval.end).max().unwrap();
            let mut current = first;
            let mut observed = Duration::ZERO;
            for interval in intervals.iter().skip(1) {
                if interval.start <= current.end {
                    current.end = current.end.max(interval.end);
                } else {
                    observed += current.end - current.start;
                    current = *interval;
                }
            }
            observed += current.end - current.start;
            LeafTiming {
                fixed_step,
                observed_ms: observed.as_secs_f64() * 1000.0,
                envelope_ms: (end - first.start).as_secs_f64() * 1000.0,
                intervals: intervals.len(),
            }
        })
        .collect();
    LeafInvocation {
        id: raw.id,
        elapsed_ms: (raw.interval.end - raw.interval.start).as_secs_f64() * 1000.0,
        groups,
        unattributed_names,
    }
}

/// Separate sidecar; existing PhaseCoverage serialization is unchanged.
#[derive(Default, Debug, Serialize)]
pub struct LeafCoverage {
    pub schedule: Option<Vec<LeafScheduleRow>>,
    pub samples: Vec<LeafInvocation>,
}
impl LeafCoverage {
    pub(super) fn record(
        &mut self,
        app: &App,
        raw: &[RawInvocation],
        span_counts: &BTreeMap<String, usize>,
    ) -> Result<(), String> {
        let rows = observe(app)?;
        if self
            .schedule
            .as_ref()
            .is_some_and(|previous| *previous != rows)
        {
            return Err("FixedUpdate leaf graph changed during capture".into());
        }
        let membership = attribution(&rows);
        self.schedule = Some(rows);
        self.samples
            .extend(raw.iter().map(|raw| reduce(raw, &membership, span_counts)));
        Ok(())
    }
}

#[cfg(test)]
#[path = "phase_leaves_tests.rs"]
mod tests;
