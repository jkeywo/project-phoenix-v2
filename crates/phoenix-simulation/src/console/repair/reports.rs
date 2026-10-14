//! Reduction of delivered evidence; no machinery or privileged world reads.
use super::server::{RepairQueueEntry, RepairRequestQueue, RepairSourceReading};
use crate::core::messages::RepairGroup;
use phoenix_model::observation::ObservationMetadata;

#[derive(Debug, PartialEq, Eq)]
pub(super) enum ReportReduction {
    Raised,
    Updated,
    Resolved,
    Unchanged,
}

fn same_source(a: &ObservationMetadata, b: &ObservationMetadata) -> bool {
    a.subject == b.subject && a.provenance == b.provenance
}
fn same_known_subject(a: &ObservationMetadata, b: &ObservationMetadata) -> bool {
    a.subject != phoenix_model::observation::ObservationSubject::Unknown && a.subject == b.subject
}
pub(crate) fn sort_observations(values: &mut [ObservationMetadata]) {
    values.sort_by(|a, b| {
        a.subject
            .cmp(&b.subject)
            .then_with(|| a.provenance.cmp(&b.provenance))
    });
}
impl RepairRequestQueue {
    pub(super) fn reduce_report(&mut self, mut incoming: RepairQueueEntry) -> ReportReduction {
        let had_evidence = !incoming.observations.is_empty();
        // Diagnosis runs before damage observation in a fixed step. A same-tick
        // report may therefore describe genuinely new damage and must survive.
        let watermark = self
            .recoveries
            .iter()
            .find(|(group, _)| group == &incoming.group)
            .and_then(|(_, observation)| observation.observed_tick);
        incoming.observations.retain(|observation| observation.is_valid()
            && !matches!((observation.observed_tick, watermark), (Some(sample), Some(resolved)) if sample < resolved));
        if had_evidence && incoming.observations.is_empty() {
            return ReportReduction::Unchanged;
        }
        if let Some(existing) = self
            .entries
            .iter_mut()
            .find(|entry| entry.group == incoming.group)
        {
            incoming.observations.retain(|observation| {
                !existing.observations.iter().any(|previous| {
                    (same_source(previous, observation)
                        || same_known_subject(previous, observation))
                        && match (previous.observed_tick, observation.observed_tick) {
                            (Some(old), Some(new)) => old > new,
                            (Some(_), None) => true,
                            _ => false,
                        }
                })
            });
            if had_evidence && incoming.observations.is_empty() {
                return ReportReduction::Unchanged;
            }
            // A later diagnosis of the same identified System supersedes an
            // older symptom regardless of which permitted observer reported it.
            // Equal-tick samples retain the worst reading: damage follows repair.
            existing.readings.retain(|previous| !incoming.observations.iter().any(|next|
                same_known_subject(&previous.source, next)
                    && matches!((previous.source.observed_tick, next.observed_tick), (Some(old), Some(new)) if old < new)));
            existing.observations.retain(|previous| !incoming.observations.iter().any(|next|
                same_known_subject(previous, next)
                    && matches!((previous.observed_tick, next.observed_tick), (Some(old), Some(new)) if old < new)));
            for source in &incoming.observations {
                let mut next = RepairSourceReading {
                    source: source.clone(),
                    tier: incoming.tier,
                    deficit: incoming.deficit,
                };
                if let Some(old) = existing
                    .readings
                    .iter_mut()
                    .find(|old| same_source(&old.source, source))
                {
                    if old.source.observed_tick == next.source.observed_tick {
                        next.tier = next.tier.max(old.tier);
                        next.deficit = next.deficit.max(old.deficit);
                    }
                    *old = next;
                } else {
                    existing.readings.push(next);
                }
            }
            if existing.readings.is_empty() {
                existing.tier = existing.tier.max(incoming.tier);
                existing.deficit = existing.deficit.max(incoming.deficit);
            } else {
                existing.tier = existing
                    .readings
                    .iter()
                    .map(|row| row.tier)
                    .max()
                    .unwrap_or(crate::ship::damage::DamageTier::Operational);
                existing.deficit = existing
                    .readings
                    .iter()
                    .map(|row| row.deficit)
                    .fold(0.0, f32::max);
                existing.readings.sort_by(|a, b| {
                    a.source
                        .subject
                        .cmp(&b.source.subject)
                        .then_with(|| a.source.provenance.cmp(&b.source.provenance))
                });
            }
            if !incoming.station_label.is_empty() {
                existing.station_label = incoming.station_label;
            }
            for observation in incoming.observations {
                if let Some(previous) = existing
                    .observations
                    .iter_mut()
                    .find(|old| same_source(old, &observation))
                {
                    *previous = observation;
                } else {
                    existing.observations.push(observation);
                }
            }
            sort_observations(&mut existing.observations);
            if existing.tier == crate::ship::damage::DamageTier::Operational {
                let group = existing.group.clone();
                let observation = existing
                    .observations
                    .iter()
                    .max_by_key(|row| row.observed_tick)
                    .cloned();
                if let Some(observation) = observation {
                    return self.resolve_report(group, observation);
                }
            }
            ReportReduction::Updated
        } else {
            if incoming.tier == crate::ship::damage::DamageTier::Operational {
                return ReportReduction::Unchanged;
            }
            sort_observations(&mut incoming.observations);
            incoming.readings = incoming
                .observations
                .iter()
                .map(|source| RepairSourceReading {
                    source: source.clone(),
                    tier: incoming.tier,
                    deficit: incoming.deficit,
                })
                .collect();
            self.entries.push(incoming);
            ReportReduction::Raised
        }
    }
    pub(super) fn resolve_report(
        &mut self,
        group: RepairGroup,
        observation: ObservationMetadata,
    ) -> ReportReduction {
        if !observation.is_valid() {
            return ReportReduction::Unchanged;
        }
        if let Some((_, previous)) = self.recoveries.iter().find(|(key, _)| key == &group) {
            if matches!((previous.observed_tick, observation.observed_tick), (Some(old), Some(new)) if old > new)
                || (previous.observed_tick.is_some() && observation.observed_tick.is_none())
            {
                return ReportReduction::Unchanged;
            }
        }
        if self
            .entries
            .iter()
            .filter(|entry| entry.group == group)
            .flat_map(|entry| &entry.observations)
            .any(
                |current| match (current.observed_tick, observation.observed_tick) {
                    (Some(current), Some(recovered)) => current > recovered,
                    (Some(_), None) => true,
                    _ => false,
                },
            )
        {
            return ReportReduction::Unchanged;
        }
        let before = self.entries.len();
        self.entries.retain(|entry| entry.group != group);
        if observation.observed_tick.is_some() {
            if let Some((_, previous)) = self.recoveries.iter_mut().find(|(key, _)| key == &group) {
                *previous = observation;
            } else {
                self.recoveries.push((group, observation));
            }
            self.recoveries.sort_by(|a, b| a.0.cmp(&b.0));
        }
        if before != self.entries.len() {
            ReportReduction::Resolved
        } else {
            ReportReduction::Unchanged
        }
    }
}

#[cfg(test)]
#[path = "reports_tests.rs"]
mod tests;
