use super::*;
use phoenix_model::console::{ConsoleFeed, ConsoleObservationKind};
use phoenix_model::messages::{ScanBlackboard, ScanReadingSnapshot, SystemBlackboard};
use phoenix_model::observation::{
    ObservationMetadata, ObservationProvenance, ObservationSource, ObservationSubject,
    ObservationUncertainty,
};
use std::collections::HashMap;
fn fixture() -> ShipConfig {
    toml::from_str(
        r#"
[[station]]
id="seat"
name="Seat"
description="Shared crew seat"
rank=""
[[console]]
id="science"
home_station="seat"
screen="science.html"
[[console.feed]]
source="scan"
kind="Scan"
[[console]]
id="other"
home_station="seat"
screen="science.html"
[[system]]
id="scanner"
kind="sensors"
console="science"
[[system]]
id="other-scanner"
kind="sensors"
console="other"
"#,
    )
    .unwrap()
}
fn boards() -> HashMap<SystemId, SystemBlackboard> {
    let metadata = ObservationMetadata::at(
        17,
        ObservationSubject::Entity {
            uuid: "target".into(),
        },
        ObservationProvenance {
            kind: ObservationSource::Scan,
            system: Some(SystemId("scanner".into())),
            label: String::new(),
        },
        ObservationUncertainty::Quantized {
            field: "condition_fraction".into(),
            step: 0.1,
        },
    );
    HashMap::from([(
        SystemId("scan".into()),
        SystemBlackboard::Scan(ScanBlackboard {
            reading: Some(ScanReadingSnapshot {
                observation: Some(metadata),
                ..Default::default()
            }),
            ..Default::default()
        }),
    )])
}
#[test]
fn cohosted_consoles_share_neither_records_nor_metadata() {
    let config = fixture();
    let boards = boards();
    let source = SystemId("scan".into());
    let permitted = config
        .observations_for_system(&SystemId("scanner".into()), &boards)
        .recorded_observations(&source);
    assert_eq!(permitted.len(), 1);
    assert_eq!(permitted[0].observed_tick, Some(17));
    assert_eq!(permitted[0].age_ticks(40), Some(23));
    assert!(std::ptr::eq(
        permitted[0],
        boards[&source].recorded_observations()[0]
    ));
    assert!(config
        .observations_for_system(&SystemId("other-scanner".into()), &boards)
        .recorded_observations(&source)
        .is_empty());
}
#[test]
fn explicit_typed_feed_can_be_revoked_without_granting_control() {
    let mut config = fixture();
    let boards = boards();
    let source = SystemId("scan".into());
    let reader = SystemId("other-scanner".into());
    config.consoles[1].feeds.push(ConsoleFeed {
        source: source.clone(),
        kind: ConsoleObservationKind::Scan,
    });
    assert_eq!(
        config
            .observations_for_system(&reader, &boards)
            .recorded_observations(&source)
            .len(),
        1
    );
    assert!(crate::command_admission::policy::station_for_system(&config, None, &source).is_none());
    assert_eq!(
        config
            .console_for_system(&SystemId("scanner".into()))
            .unwrap()
            .id,
        ConsoleId("science".into())
    );
    config.consoles[1].feeds[0].kind = ConsoleObservationKind::Repair;
    assert!(config
        .observations_for_system(&reader, &boards)
        .recorded_observations(&source)
        .is_empty());
    config.consoles[1].feeds.clear();
    assert!(config
        .observations_for_system(&reader, &boards)
        .recorded_observations(&source)
        .is_empty());
}
#[test]
fn missing_metadata_or_live_board_never_fabricates_sample_time() {
    let config = fixture();
    let source = SystemId("scan".into());
    let reader = SystemId("scanner".into());
    let mut boards = boards();
    boards.insert(
        source.clone(),
        SystemBlackboard::Scan(ScanBlackboard::default()),
    );
    assert!(config
        .observations_for_system(&reader, &boards)
        .recorded_observations(&source)
        .is_empty());
    boards.insert(source.clone(), SystemBlackboard::Helm(Default::default()));
    assert!(config
        .observations_for_system(&reader, &boards)
        .recorded_observations(&source)
        .is_empty());
    assert!(boards[&source].recorded_observations().is_empty());
}

#[test]
fn sensors_exposes_only_permitted_held_reports_not_live_gauges() {
    use phoenix_model::messages::SensorsBlackboard;
    use phoenix_model::wire::SensorReport;
    let mut config = fixture();
    let reader = SystemId("other-scanner".into());
    let source = SystemId("sensors".into());
    let sample = ObservationMetadata {
        observed_tick: Some(7),
        ..Default::default()
    };
    let mut sensors = SensorsBlackboard {
        radar_range: 1234.0,
        ..Default::default()
    };
    sensors.contact_reports.insert("hidden".into(), None);
    sensors.contact_reports.insert(
        "target".into(),
        Some(SensorReport {
            observation: Some(sample.clone()),
            observed_tick: 7,
            age_ticks: 33,
            source: "public-source".into(),
            name: "target".into(),
            position_mm: [1, 2, 3],
        }),
    );
    let mut boards = HashMap::from([(source.clone(), SystemBlackboard::Sensors(sensors.clone()))]);
    assert!(config
        .observations_for_system(&reader, &boards)
        .recorded_observations(&source)
        .is_empty());
    config.consoles[1].feeds.push(ConsoleFeed {
        source: source.clone(),
        kind: ConsoleObservationKind::Sensors,
    });
    assert_eq!(
        config
            .observations_for_system(&reader, &boards)
            .recorded_observations(&source),
        vec![&sample]
    );
    sensors.contact_reports.clear();
    sensors.radar_range = 9999.0;
    boards.insert(source.clone(), SystemBlackboard::Sensors(sensors));
    assert!(config
        .observations_for_system(&reader, &boards)
        .recorded_observations(&source)
        .is_empty());
}
