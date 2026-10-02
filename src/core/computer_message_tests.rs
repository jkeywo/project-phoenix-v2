use super::*;

const HZ: f32 = 60.0;

fn request(id: &str, secs: i64) -> ComputerMessageRequest {
    ComputerMessageRequest {
        id: id.into(),
        text: "world.probe.computer_message.reach".into(),
        severity: ComputerMessageSeverity::Advisory,
        duration_secs: secs,
        station: None,
    }
}

// ── ComputerMessageSeverity::parse ────────────────────────────────────

#[test]
fn parse_accepts_every_severity_case_insensitively_and_trimmed() {
    assert_eq!(
        ComputerMessageSeverity::parse("Info"),
        Ok(ComputerMessageSeverity::Info)
    );
    assert_eq!(
        ComputerMessageSeverity::parse(" ADVISORY "),
        Ok(ComputerMessageSeverity::Advisory)
    );
    assert_eq!(
        ComputerMessageSeverity::parse("warning"),
        Ok(ComputerMessageSeverity::Warning)
    );
    assert_eq!(
        ComputerMessageSeverity::parse("CRITICAL"),
        Ok(ComputerMessageSeverity::Critical)
    );
}

#[test]
fn parse_rejects_an_unknown_severity() {
    assert!(ComputerMessageSeverity::parse("urgent").is_err());
    assert!(ComputerMessageSeverity::parse("").is_err());
}

#[test]
fn severity_labels_round_trip_through_parse() {
    for s in [
        ComputerMessageSeverity::Info,
        ComputerMessageSeverity::Advisory,
        ComputerMessageSeverity::Warning,
        ComputerMessageSeverity::Critical,
    ] {
        assert_eq!(ComputerMessageSeverity::parse(s.as_str()), Ok(s));
    }
}

// ── show / supersede ──────────────────────────────────────────────────

#[test]
fn showing_the_first_message_supersedes_nothing() {
    let mut active = ActiveComputerMessage::default();
    let superseded = active.show(&request("hail_debris", 10), 0, HZ);
    assert!(superseded.is_none());
    let current = active.current.as_ref().unwrap();
    assert_eq!(current.id, "hail_debris");
    assert_eq!(current.shown_tick, 0);
    assert_eq!(current.expires_tick, 600, "10s at 60Hz is tick 600");
}

#[test]
fn a_second_message_supersedes_the_first_immediately() {
    let mut active = ActiveComputerMessage::default();
    active.show(&request("first", 100), 0, HZ);
    let superseded = active.show(&request("second", 5), 10, HZ);
    assert_eq!(superseded, Some(SupersededMessage { id: "first".into() }));
    assert_eq!(active.current.as_ref().unwrap().id, "second");
    assert_eq!(active.current.as_ref().unwrap().expires_tick, 10 + 300);
}

#[test]
fn a_superseded_message_is_never_resumed() {
    // Once "second" expires there is nothing left to fall back to — the
    // state holds only ONE message, never a stack.
    let mut active = ActiveComputerMessage::default();
    active.show(&request("first", 100), 0, HZ);
    active.show(&request("second", 5), 0, HZ);
    active.expire_if_due(300);
    assert!(active.current.is_none(), "no resumption of 'first'");
}

#[test]
fn show_carries_severity_and_station_through() {
    let mut active = ActiveComputerMessage::default();
    let req = ComputerMessageRequest {
        id: "charge_ready".into(),
        text: "world.probe.computer_message.charge".into(),
        severity: ComputerMessageSeverity::Critical,
        duration_secs: 8,
        station: Some(StationId("tactical".into())),
    };
    active.show(&req, 0, HZ);
    let current = active.current.as_ref().unwrap();
    assert_eq!(current.severity, ComputerMessageSeverity::Critical);
    assert_eq!(current.station, Some(StationId("tactical".into())));
    assert_eq!(current.text, "world.probe.computer_message.charge");
}

// ── expiry ─────────────────────────────────────────────────────────────

#[test]
fn expiry_does_not_fire_before_its_tick() {
    let mut active = ActiveComputerMessage::default();
    active.show(&request("hail_debris", 10), 0, HZ);
    assert_eq!(active.expire_if_due(599), None, "one tick early");
    assert!(active.current.is_some());
}

#[test]
fn expiry_fires_exactly_on_its_due_tick_and_clears_state() {
    let mut active = ActiveComputerMessage::default();
    active.show(&request("hail_debris", 10), 0, HZ);
    assert_eq!(active.expire_if_due(600), Some("hail_debris".to_string()));
    assert!(active.current.is_none());
}

#[test]
fn expiry_on_an_empty_state_is_a_silent_none() {
    let mut active = ActiveComputerMessage::default();
    assert_eq!(active.expire_if_due(1_000_000), None);
}

#[test]
fn a_zero_or_negative_duration_still_arms_at_least_one_tick_out() {
    // Not reachable in practice — the script boundary rejects a
    // non-positive duration before this is ever constructed — but the
    // pure state machine must not divide-by/collapse-to zero if it ever
    // is.
    let mut active = ActiveComputerMessage::default();
    active.show(&request("degenerate", 0), 5, HZ);
    let current = active.current.as_ref().unwrap();
    assert!(
        current.expires_tick > 5,
        "must not expire on the same tick it was shown"
    );
}

// ── clear ────────────────────────────────────────────────────────────

#[test]
fn clear_on_empty_state_returns_none() {
    let mut active = ActiveComputerMessage::default();
    assert_eq!(active.clear(), None);
}

#[test]
fn clear_returns_the_cleared_id_and_empties_state() {
    let mut active = ActiveComputerMessage::default();
    active.show(&request("mission_wrap", 30), 0, HZ);
    assert_eq!(active.clear(), Some("mission_wrap".to_string()));
    assert!(active.current.is_none());
}

#[test]
fn default_state_is_empty() {
    assert_eq!(ActiveComputerMessage::default().current, None);
}
