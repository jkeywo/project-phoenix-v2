use super::*;

/// The exact former owned envelope, independent of the new outgoing alias.
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", content = "data")]
enum OwnedMessage {
    BlackboardUpdate {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        console_scope: Option<Vec<SystemId>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        console_scope_tick: Option<u64>,
        updates: Vec<(SystemId, SystemBlackboard)>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        presentation_generation: Option<u64>,
    },
}

#[test]
fn shared_publication_preserves_owned_json_schema_and_every_optional_envelope_combination() {
    let boards = vec![
        (
            SystemId("flight".into()),
            SystemBlackboard::Helm(HelmBlackboard {
                x: 42.25,
                ..Default::default()
            }),
        ),
        (
            SystemId("repair".into()),
            SystemBlackboard::Repair(RepairBlackboard {
                interior: Some(crate::interior::InteriorConfig {
                    placements: BTreeMap::from([(SystemId("flight".into()), "bridge".into())]),
                    ..Default::default()
                }),
                navigation: vec![crate::interior::TeamNavigation {
                    location: crate::interior::TeamLocation::Compartment {
                        compartment: "bridge".into(),
                    },
                    destination: None,
                    route: vec![],
                    blocked: false,
                }],
                ..Default::default()
            }),
        ),
        (
            SystemId("scan".into()),
            SystemBlackboard::Scan(ScanBlackboard {
                owner: Some(SystemId("scanner".into())),
                ..Default::default()
            }),
        ),
    ];
    for scope in [
        None,
        Some(vec![]),
        Some(boards.iter().map(|(id, _)| id.clone()).collect()),
    ] {
        for tick in [None, Some(0), Some(42)] {
            for generation in [None, Some(0), Some(7)] {
                for updates in [Vec::new(), boards.clone()] {
                    let shared = ServerMessage::BlackboardUpdate {
                        console_scope: scope.clone(),
                        console_scope_tick: tick,
                        updates: updates
                            .iter()
                            .map(|(id, board)| (id.clone(), std::sync::Arc::new(board.clone())))
                            .collect(),
                        presentation_generation: generation,
                    };
                    let owned = OwnedMessage::BlackboardUpdate {
                        console_scope: scope.clone(),
                        console_scope_tick: tick,
                        updates,
                        presentation_generation: generation,
                    };
                    let old_bytes = serde_json::to_vec(&owned).unwrap();
                    assert_eq!(serde_json::to_vec(&shared).unwrap(), old_bytes);
                    assert_eq!(
                        serde_json::from_slice::<OwnedMessage>(&old_bytes).unwrap(),
                        owned
                    );
                    let decoded = serde_json::from_slice::<ServerMessage>(&old_bytes).unwrap();
                    assert_eq!(decoded, shared);
                    assert_eq!(serde_json::to_vec(&decoded).unwrap(), old_bytes);
                }
            }
        }
    }
}
