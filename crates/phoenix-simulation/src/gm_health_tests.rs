use super::*;

/// The four states are decided from four different facts, and a deliberate
/// pause is not allowed to look like a fault — nor to hide one.
#[test]
fn paused_stale_recovering_and_disconnected_are_decided_separately() {
    // Running and keeping up.
    assert_eq!(
        peer_state(false, false, false, Some(0), 6),
        GmHealthState::Live
    );
    // Behind, but inside the barrier's own tolerance.
    assert_eq!(
        peer_state(false, false, false, Some(6), 6),
        GmHealthState::Live
    );
    // Behind far enough that the barrier would withhold.
    assert_eq!(
        peer_state(false, false, false, Some(7), 6),
        GmHealthState::Stale
    );
    // The same lag, while the world is deliberately held, is a pause.
    assert_eq!(
        peer_state(true, false, false, Some(7), 6),
        GmHealthState::Paused
    );
    // A recovery in flight outranks the pause it holds the fleet with.
    assert_eq!(
        peer_state(true, false, true, Some(7), 6),
        GmHealthState::Recovering
    );
    // And a peer that actually left outranks everything, pause included.
    assert_eq!(
        peer_state(true, true, true, None, 6),
        GmHealthState::Disconnected
    );
}

/// Severity ordering is what the panel's one-word summary reads off.
#[test]
fn the_summary_reports_the_worst_thing_present() {
    let mut projection = GmHealthProjection {
        peers: vec![GmPeerHealth {
            id: "ship:a".into(),
            ship: None,
            operators: Vec::new(),
            state: GmHealthState::Live,
            behind_ticks: None,
            local: true,
            restore_waiting: false,
            restore_excluded: false,
        }],
        ..Default::default()
    };
    assert_eq!(projection.worst_state(), GmHealthState::Live);
    projection.paused = true;
    assert_eq!(projection.worst_state(), GmHealthState::Paused);
    projection.stations.push(GmStationHealth {
        id: "a/helm".into(),
        station_id: StationId("helm".into()),
        name: "Helm".into(),
        ship: None,
        operator: "Morgan".into(),
        state: GmHealthState::Disconnected,
    });
    assert_eq!(projection.worst_state(), GmHealthState::Disconnected);
}

/// A recovery in flight reads as a held fleet; only one that gave up is a
/// failure, and neither is ever mistaken for a lost peer.
#[test]
fn a_recovery_explains_itself_differently_while_it_is_still_working() {
    use crate::command_admission::log::HostSlot;
    use crate::lockstep::recovery::RecoveryStatus;
    let working = RecoveryStatus::InProgress {
        divergence_tick: 240,
        boundary_tick: 260,
        recovering: vec![HostSlot(3)],
        resolved: false,
    };
    let alert = recovery_alert(&working);
    assert_eq!(alert.kind, GmHealthAlertKind::RecoveryInProgress);
    assert_eq!(alert.kind.severity(), GmHealthState::Recovering);
    assert_eq!(alert.reason.id, RECOVERY_IN_PROGRESS_REASON);
    assert_eq!(alert.reason.params.get("tick").unwrap(), "260");
    assert_eq!(alert.reason.params.get("peers").unwrap(), "1");
    // The key carries the boundary, so a SECOND recovery at a later
    // boundary is a new condition rather than the same row aging on.
    assert_eq!(alert.key, "recovery:260");

    // A terminal failure names the divergence it could not repair — the
    // tick the recovery itself recorded, whether or not a boundary was ever
    // agreed. Both shapes are checked, because the no-safe-leader path
    // reaches the banner with no boundary at all.
    for boundary_tick in [Some(260), None] {
        let failed = RecoveryStatus::Failed {
            divergence_tick: 240,
            boundary_tick,
        };
        let alert = recovery_alert(&failed);
        assert_eq!(alert.kind, GmHealthAlertKind::RecoveryFailed);
        assert_eq!(alert.kind.severity(), GmHealthState::Disconnected);
        assert_eq!(alert.reason.id, RECOVERY_FAILED_REASON);
        assert_eq!(
            alert.reason.params.get("tick").unwrap(),
            &failed.divergence_tick().to_string(),
            "the unfilterable sentence quotes the recovery's own divergence tick"
        );
        assert_eq!(alert.reason.params.get("tick").unwrap(), "240");
    }
}

/// Only Station-scoped losses become queue rows; the technical conditions
/// with no Station to open stay banner-only.
#[test]
fn station_scoped_alerts_are_the_ones_that_earn_a_queue_row() {
    assert!(GmHealthAlertKind::StationDisconnected.is_station_attention());
    assert!(GmHealthAlertKind::ShipPeerLost.is_station_attention());
    assert!(!GmHealthAlertKind::OperatorDisconnected.is_station_attention());
    assert!(!GmHealthAlertKind::RecoveryInProgress.is_station_attention());
    assert!(!GmHealthAlertKind::RecoveryFailed.is_station_attention());
    // A recovery in flight is a held fleet, not a lost one.
    assert_eq!(
        GmHealthAlertKind::RecoveryInProgress.severity(),
        GmHealthState::Recovering
    );
    assert_eq!(
        GmHealthAlertKind::StationDisconnected.severity(),
        GmHealthState::Disconnected
    );
}
