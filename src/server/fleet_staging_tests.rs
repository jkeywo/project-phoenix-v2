use super::*;

fn roster() -> String {
    r#"{"local":1,"owner":1,"participants":[1],"ships":[{"host":1,"crew":[]}]}"#.to_string()
}

#[test]
fn supersession_preserves_leave_and_rebinds_latest_projections() {
    let mut state = FleetStaging {
        managed: Some(true),
        validation: Some(true),
        ..Default::default()
    };
    assert!(state.queue(FleetLobbyInput::Managed(false)));
    let first: u64 = state.join(&roster()).parse().unwrap();
    let left: u64 = state.leave().parse().unwrap();
    let joined: u64 = state.join(&roster()).parse().unwrap();
    assert!(first < left && left < joined);
    assert!(
        matches!(state.adoptions.front(), Some(PendingFleetAdoption::Leave { generation }) if *generation == left)
    );
    assert!(
        matches!(state.adoptions.back(), Some(PendingFleetAdoption::Join(join)) if join.generation == joined && join.roster_json == roster())
    );
    state.complete(left, true, "");
    assert_eq!(state.status.generation, joined);
    assert!(state.take_inputs(&state.status.clone()).is_none());
    state.complete(joined, true, "");
    let inputs = state.take_inputs(&state.status.clone()).unwrap();
    assert!(matches!(
        inputs.front(),
        Some(FleetLobbyInput::Managed(true))
    ));
    assert!(matches!(
        inputs.back(),
        Some(FleetLobbyInput::Validation(true))
    ));
}

#[test]
fn invalid_join_cancels_pending_join_and_cannot_regress_completion() {
    let mut state = FleetStaging::default();
    let old: u64 = state.join(&roster()).parse().unwrap();
    assert_eq!(state.join("invalid"), "fleet-roster-unreadable");
    assert!(state.adoptions.is_empty());
    state.complete(old, true, "");
    assert_eq!(state.status.status, FleetJoinStatusKind::Refused);
    assert!(state.take_inputs(&state.status.clone()).is_none());
}

#[test]
fn queue_is_bounded_and_retries_preserve_input_order() {
    let mut state = FleetStaging::default();
    for index in 0..MAX_FLEET_LOBBY_INPUTS {
        assert!(state.queue(FleetLobbyInput::Managed(index % 2 == 0)));
    }
    assert!(!state.queue(FleetLobbyInput::Managed(true)));
    let pending = FleetJoinStatus {
        generation: 0,
        status: FleetJoinStatusKind::Pending,
        reason: None,
    };
    assert!(state.take_inputs(&pending).is_none());
    let accepted = FleetJoinStatus {
        status: FleetJoinStatusKind::Accepted,
        ..pending
    };
    let inputs = state.take_inputs(&accepted).unwrap();
    assert_eq!(inputs.len(), MAX_FLEET_LOBBY_INPUTS);
    assert!(matches!(
        inputs.front(),
        Some(FleetLobbyInput::Managed(true))
    ));
    assert!(matches!(
        inputs.back(),
        Some(FleetLobbyInput::Managed(false))
    ));
}
