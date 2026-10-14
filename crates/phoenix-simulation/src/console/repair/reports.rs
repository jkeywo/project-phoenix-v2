//! Reduction of delivered evidence; no machinery or privileged world reads.
use super::server::{RepairQueueEntry, RepairRequestQueue};
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
                    same_source(previous, observation)
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
            existing.tier = existing.tier.max(incoming.tier);
            existing.deficit = existing.deficit.max(incoming.deficit);
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
            ReportReduction::Updated
        } else {
            sort_observations(&mut incoming.observations);
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
