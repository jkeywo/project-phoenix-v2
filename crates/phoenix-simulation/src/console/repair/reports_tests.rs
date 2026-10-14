use super::*;
use crate::core::messages::{StationId, SystemId};
use crate::ship::damage::DamageTier;
use phoenix_model::observation::*;
fn evidence(source: &str, tick: u64) -> ObservationMetadata {
    ObservationMetadata::at(
        tick,
        ObservationSubject::System {
            id: SystemId(source.into()),
        },
        ObservationProvenance {
            kind: ObservationSource::SystemReport,
            system: Some(SystemId(source.into())),
            label: String::new(),
        },
        ObservationUncertainty::Qualitative,
    )
}
fn entry(source: &str, tick: u64) -> RepairQueueEntry {
    RepairQueueEntry {
        observations: vec![evidence(source, tick)],
        group: RepairGroup::Station(StationId("helm".into())),
        station_label: "Helm".into(),
        tier: DamageTier::Damaged,
        deficit: 20.0,
    }
}
#[test]
fn recovery_cannot_erase_newer_evidence_and_delayed_report_cannot_reopen() {
    let mut queue = RepairRequestQueue::default();
    let report = entry("engine", 20);
    assert_eq!(queue.reduce_report(report.clone()), ReportReduction::Raised);
    assert_eq!(
        queue.resolve_report(report.group.clone(), evidence("repair", 19)),
        ReportReduction::Unchanged
    );
    assert_eq!(queue.entries.len(), 1);
    assert_eq!(
        queue.resolve_report(report.group.clone(), evidence("repair", 21)),
        ReportReduction::Resolved
    );
    assert_eq!(queue.reduce_report(report), ReportReduction::Unchanged);
    assert!(queue.entries.is_empty());
    assert_eq!(
        queue.reduce_report(entry("engine", 22)),
        ReportReduction::Raised
    );
}
#[test]
fn contributors_are_distinct_and_stale_source_does_not_change_priority() {
    let mut queue = RepairRequestQueue::default();
    queue.reduce_report(entry("left", 20));
    queue.reduce_report(entry("right", 21));
    let mut stale = entry("left", 19);
    stale.deficit = 900.0;
    stale.tier = DamageTier::Destroyed;
    assert_eq!(queue.reduce_report(stale), ReportReduction::Unchanged);
    assert_eq!(queue.entries[0].deficit, 20.0);
    assert_eq!(queue.entries[0].observations.len(), 2);
    assert_eq!(
        queue.resolve_report(
            queue.entries[0].group.clone(),
            ObservationMetadata::default()
        ),
        ReportReduction::Unchanged
    );
}
#[test]
fn unknown_legacy_reports_keep_unknown_time_and_existing_merge_behavior() {
    let mut queue = RepairRequestQueue::default();
    let mut report = entry("engine", 1);
    report.observations.clear();
    queue.reduce_report(report.clone());
    report.deficit = 30.0;
    queue.reduce_report(report);
    assert_eq!(queue.entries[0].deficit, 30.0);
    assert!(queue.entries[0].observations.is_empty());
    let group = queue.entries[0].group.clone();
    assert_eq!(
        queue.resolve_report(group, ObservationMetadata::default()),
        ReportReduction::Resolved
    );
}

#[test]
fn damage_sampled_after_same_tick_diagnosis_can_reopen_group() {
    let mut queue = RepairRequestQueue::default();
    let report = entry("engine", 20);
    queue.reduce_report(report.clone());
    queue.resolve_report(report.group.clone(), evidence("repair", 21));
    assert_eq!(
        queue.reduce_report(entry("engine", 21)),
        ReportReduction::Raised
    );
}
