use super::*;

fn operator(id: &str, name: &str, connected: bool) -> GmOperator {
    GmOperator {
        id: id.into(),
        name: name.into(),
        connected,
        ready: false,
    }
}

#[test]
fn roster_order_is_canonical_and_empty_display_names_are_valid() {
    let roster = GmRoster::try_new(vec![
        operator("gm-2", "", false),
        operator("gm-1", "Morgan", true),
    ])
    .unwrap();

    assert_eq!(
        roster
            .operators()
            .iter()
            .map(|operator| operator.id.as_str())
            .collect::<Vec<_>>(),
        vec!["gm-1", "gm-2"]
    );
}

#[test]
fn duplicate_and_unbounded_rows_are_refused() {
    assert_eq!(
        GmRoster::try_new(vec![
            operator("gm-1", "One", true),
            operator("gm-1", "Two", false),
        ]),
        Err(GmRosterError::DuplicateId)
    );
    assert_eq!(
        GmRoster::try_new(
            (0..=MAX_GM_OPERATORS)
                .map(|index| operator(&format!("gm-{index}"), "", true))
                .collect()
        ),
        Err(GmRosterError::TooManyOperators)
    );
}

#[test]
fn ids_and_names_are_character_bounded() {
    assert_eq!(
        GmRoster::try_new(vec![operator("", "No id", true)]),
        Err(GmRosterError::EmptyId)
    );
    assert_eq!(
        GmRoster::try_new(vec![operator(
            &"x".repeat(MAX_GM_OPERATOR_ID_CHARS + 1),
            "",
            true,
        )]),
        Err(GmRosterError::IdTooLong)
    );
    assert_eq!(
        GmRoster::try_new(vec![operator(
            "gm-1",
            &"x".repeat(MAX_GM_OPERATOR_NAME_CHARS + 1),
            true,
        )]),
        Err(GmRosterError::NameTooLong)
    );
}

#[test]
fn gm_rows_consume_neither_player_readiness_nor_a_ship_slot() {
    let mut sessions = crate::lobby::session::SessionManager::new();
    sessions.register("crew-1".into(), "Alice".into()).unwrap();
    sessions.set_ready("crew-1", true);
    let fleet = crate::lockstep::FleetRoster::default();

    let gms = GmRoster::try_new(vec![operator("gm-1", "Morgan", true)]).unwrap();

    assert_eq!(gms.operators().len(), 1);
    assert!(sessions.all_ready());
    assert_eq!(fleet.len(), 1);
    assert!(fleet.is_solo());
}

#[test]
fn disconnect_and_reconnect_clear_readiness_without_changing_identity() {
    let connected_ready = GmRoster::try_new(vec![GmOperator {
        id: "gm-1".into(),
        name: "Morgan".into(),
        connected: true,
        ready: true,
    }])
    .unwrap();
    let disconnected = GmRoster::try_new(vec![GmOperator {
        id: "gm-1".into(),
        name: "Morgan".into(),
        connected: false,
        ready: true,
    }])
    .unwrap();
    assert!(!disconnected.operators()[0].ready);

    let mut reconnected = connected_ready.clone();
    reconnected.clear_reconnected_readiness(&disconnected);
    assert_eq!(reconnected.operators()[0].id, "gm-1");
    assert!(!reconnected.operators()[0].ready);
}

#[test]
fn gm_readiness_counts_only_connected_rows() {
    let roster = GmRoster::try_new(vec![
        GmOperator {
            ready: true,
            ..operator("gm-1", "One", true)
        },
        GmOperator {
            ready: true,
            ..operator("gm-2", "Two", false)
        },
    ])
    .unwrap();
    assert_eq!(
        roster.readiness_tally(),
        crate::lobby::start_policy::ReadinessTally {
            connected: 1,
            ready: 1
        }
    );
}
