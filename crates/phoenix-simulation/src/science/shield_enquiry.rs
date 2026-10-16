//! Advisory consumers see acquired immutable readings through their actual Console feeds.
use crate::{
    core::messages::{SystemBlackboard, SystemId},
    server_app::ShipSystemBlackboards,
    ship::config::ShipConfig,
};
use phoenix_model::{focused_enquiry::FocusedAim, shield_enquiry::ShieldFinding};

/// Necessary condition only. When false, the existing qualified latest-finding
/// selection cannot return Some for ANY set of public subjects. A malformed
/// finding or an older Some followed by a newer empty reading deliberately
/// passes this preflight; the existing selector still decides the outcome.
/// Reads only this consuming Console's current permitted publications.
pub(crate) fn has_permitted_shield_finding(
    config: &ShipConfig,
    boards: &ShipSystemBlackboards,
    consumer: &SystemId,
) -> bool {
    let view = config.observations_for_system(consumer, &boards.0);
    boards
        .0
        .keys()
        .filter(|source| source.0.starts_with("scan/"))
        .any(|source| {
            let Some(SystemBlackboard::Scan(scan)) = view.get(source) else {
                return false;
            };
            let Some(owner) = super::scan_owner_from_blackboard_key(source) else {
                return false;
            };
            scan.capable
                && scan.owner.as_ref() == Some(&owner)
                && scan.history.iter().any(|reading| {
                    reading.aim == FocusedAim::Shields && reading.shield_finding.is_some()
                })
        })
}

pub fn latest_permitted_finding<'a>(
    config: &'a ShipConfig,
    boards: &'a ShipSystemBlackboards,
    consumer: &SystemId,
    public_subject: &str,
) -> Option<&'a ShieldFinding> {
    latest_permitted_finding_for_subjects(
        config,
        boards,
        consumer,
        &std::collections::BTreeSet::from([public_subject.to_owned()]),
    )
}

/// Callers may correlate only public handles which their own Console currently
/// resolves to its already-authorized target. This never rewrites reading
/// provenance, refreshes a sample, or grants access to another Scan feed.
pub fn latest_permitted_finding_for_subjects<'a>(
    config: &'a ShipConfig,
    boards: &'a ShipSystemBlackboards,
    consumer: &SystemId,
    public_subjects: &std::collections::BTreeSet<String>,
) -> Option<&'a ShieldFinding> {
    let view = config.observations_for_system(consumer, &boards.0);
    let mut readings = Vec::new();
    let mut sources: Vec<_> = boards.0.keys().collect();
    sources.sort();
    for source in sources {
        let Some(owner) = super::scan_owner_from_blackboard_key(source) else {
            continue;
        };
        let Some(SystemBlackboard::Scan(scan)) = view.get(source) else {
            continue;
        };
        if !scan.capable || scan.owner.as_ref() != Some(&owner) {
            continue;
        }
        readings.extend(scan.history.iter().filter(|reading| {
            reading.aim == FocusedAim::Shields && public_subjects.contains(&reading.subject_uuid)
        }));
    }
    let latest = readings.iter().map(|reading| reading.taken_at_tick).max()?;
    let candidates: Vec<_> = readings
        .into_iter()
        .filter(|reading| reading.taken_at_tick == latest)
        .filter_map(|reading| reading.shield_finding.as_ref())
        .collect();
    let finding = *candidates.first()?;
    (finding.validate().is_ok() && candidates.iter().all(|candidate| *candidate == finding))
        .then_some(finding)
}

#[cfg(test)]
#[path = "shield_enquiry_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "shield_enquiry_fixture.rs"]
pub(crate) mod fixture;
