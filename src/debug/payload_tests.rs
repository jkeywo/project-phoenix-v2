use super::*;
use crate::ship::control_source::ControlSource;

#[test]
fn activity_source_maps_from_every_control_source() {
    assert_eq!(
        ActivitySource::from(ControlSource::Human),
        ActivitySource::Human
    );
    assert_eq!(ActivitySource::from(ControlSource::Ai), ActivitySource::Ai);
    assert_eq!(
        ActivitySource::from(ControlSource::Offline),
        ActivitySource::Offline
    );
}

#[test]
fn entry_records_into_the_matching_source_and_totals() {
    let mut entry = StationActivityEntry::default();
    entry.record(ActivitySource::Human);
    entry.record(ActivitySource::Human);
    entry.record(ActivitySource::Ai);
    assert_eq!(entry.human, 2);
    assert_eq!(entry.ai, 1);
    assert_eq!(entry.offline, 0);
    assert_eq!(entry.total(), 3);
}

#[test]
fn default_payload_carries_the_current_schema_version() {
    assert_eq!(
        StationActivityPayload::default().schema_version,
        DEBUG_SCHEMA_VERSION
    );
}

#[test]
fn default_ai_state_payload_carries_the_current_schema_version() {
    let payload = AiStatePayload::default();
    assert_eq!(payload.schema_version, DEBUG_SCHEMA_VERSION);
    assert!(payload.ships.is_empty());
}

#[test]
fn default_console_latency_payload_carries_the_current_schema_version() {
    let payload = ConsoleLatencyPayload::default();
    assert_eq!(payload.schema_version, DEBUG_SCHEMA_VERSION);
    assert!(payload.actions.is_empty());
}

/// A `SimHost` entry must not serialise empty client segments as zeroed
/// distributions — an absent segment is absent, not "0 ms".
#[test]
fn absent_latency_segments_are_omitted_rather_than_zeroed() {
    let entry = ActionLatencyEntry {
        surface: LatencySurface::SimHost,
        action: "FirePhaser".into(),
        count: 3,
        expired: 0,
        input_to_send: None,
        send_to_ack: None,
        input_to_ack: None,
        admit_to_broadcast: Some(LatencySummary {
            count: 3,
            p50_ms: 1.0,
            p75_ms: 2.0,
            max_ms: 3.0,
        }),
    };
    let json = crate::core::codec::encode_console_latency(&ConsoleLatencyPayload {
        schema_version: DEBUG_SCHEMA_VERSION,
        actions: vec![entry],
    });
    assert!(json.contains("admit_to_broadcast"), "{json}");
    assert!(
        !json.contains("input_to_send"),
        "an unmeasured segment must be omitted, not zeroed: {json}"
    );
}
