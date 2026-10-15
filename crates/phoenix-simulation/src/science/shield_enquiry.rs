//! Advisory consumers see acquired immutable readings through their actual Console feeds.
use crate::{
    core::messages::{SystemBlackboard, SystemId},
    server_app::ShipSystemBlackboards,
    ship::config::ShipConfig,
};
use phoenix_model::{focused_enquiry::FocusedAim, shield_enquiry::ShieldFinding};

pub fn latest_permitted_finding<'a>(
    config: &'a ShipConfig,
    boards: &'a ShipSystemBlackboards,
    consumer: &SystemId,
    public_subject: &str,
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
            reading.aim == FocusedAim::Shields && reading.subject_uuid == public_subject
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
