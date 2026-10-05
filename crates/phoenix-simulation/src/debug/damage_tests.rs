use super::*;
use crate::debug_overlay::DamageLogEntry;

fn entry(source: &str, arc: Option<&str>, amount: f32) -> DamageLogEntry {
    DamageLogEntry {
        source: source.to_string(),
        shield_arc: arc.map(str::to_string),
        amount,
    }
}

#[test]
fn empty_log_projects_to_an_empty_versioned_payload() {
    let payload = project_damage(&DamageLog::default());
    assert_eq!(payload.schema_version, DEBUG_SCHEMA_VERSION);
    assert!(payload.entries.is_empty());
}

#[test]
fn projection_preserves_newest_first_order_and_facts() {
    let mut log = DamageLog::default();
    log.push(entry("asteroid-42", Some("Fore"), 12.5));
    log.push(entry("region-zone", None, 3.0));
    let payload = project_damage(&log);
    // Newest (region-zone) first, matching the ring buffer.
    assert_eq!(payload.entries.len(), 2);
    assert_eq!(payload.entries[0].source, "region-zone");
    assert_eq!(payload.entries[0].shield_arc, None);
    assert!((payload.entries[0].amount - 3.0).abs() < f32::EPSILON);
    assert_eq!(payload.entries[1].source, "asteroid-42");
    assert_eq!(payload.entries[1].shield_arc, Some("Fore".to_string()));
    assert!((payload.entries[1].amount - 12.5).abs() < f32::EPSILON);
}
