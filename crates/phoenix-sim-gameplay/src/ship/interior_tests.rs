use super::*;

fn graph() -> InteriorGraph {
    let rooms = ["bridge", "engineering", "hangar"];
    let config = InteriorConfig {
        decks: vec![DeckConfig {
            id: "main".into(),
            label: "test.main".into(),
            damage_weight: 1.0,
        }],
        compartments: rooms
            .iter()
            .enumerate()
            .map(|(i, id)| CompartmentConfig {
                id: (*id).into(),
                deck: "main".into(),
                label: format!("test.{id}"),
                x: i as f32 * 5.0,
                z: 0.0,
                width: 4.0,
                depth: 4.0,
                material: "bulkhead".into(),
            })
            .collect(),
        connections: vec![
            ConnectionConfig {
                id: "a".into(),
                from: "bridge".into(),
                to: "engineering".into(),
                travel_secs: 2.0,
                boundary: None,
            },
            ConnectionConfig {
                id: "b".into(),
                from: "engineering".into(),
                to: "hangar".into(),
                travel_secs: 2.0,
                boundary: None,
            },
            ConnectionConfig {
                id: "z".into(),
                from: "bridge".into(),
                to: "hangar".into(),
                travel_secs: 4.0,
                boundary: None,
            },
        ],
        placements: BTreeMap::new(),
        repair_placements: BTreeMap::new(),
        team_homes: vec!["bridge".into()],
    };
    InteriorGraph::compile(&config, &ship(), 1).unwrap()
}
fn ship() -> ShipConfig {
    ShipConfig {
        consoles: vec![],
        stations: vec![],
        systems: vec![],
        power_groups: Default::default(),
        coordination_lag_secs: 2.0,
    }
}
#[test]
fn stable_route_and_sealed_alternative() {
    let graph = graph();
    assert_eq!(
        graph.route("bridge", "hangar", &BTreeSet::new()).unwrap(),
        ["a", "b"]
    );
    assert_eq!(
        graph
            .route("bridge", "hangar", &BTreeSet::from(["a".into()]))
            .unwrap(),
        ["z"]
    );
    assert!(graph
        .route(
            "bridge",
            "hangar",
            &BTreeSet::from(["a".into(), "z".into()])
        )
        .is_none());
}
#[test]
fn committed_crossing_survives_seal_redirect_and_resume() {
    let graph = graph();
    let mut team = graph.initial_teams().remove(0);
    assert!(!graph.advance(&mut team, "hangar", 1.0, &BTreeSet::new()));
    let encoded = serde_json::to_string(&team).unwrap();
    let mut resumed: TeamNavigation = serde_json::from_str(&encoded).unwrap();
    graph.validate_navigation(&[resumed.clone()]).unwrap();
    let blocked = BTreeSet::from(["a".into()]);
    // Redirect cannot teleport backwards off a crossing that was already entered.
    assert!(!graph.advance(&mut resumed, "bridge", 1.0, &blocked));
    assert!(
        matches!(&resumed.location,TeamLocation::Compartment {compartment} if compartment=="engineering")
    );
    assert!(graph.advance(&mut resumed, "bridge", 6.0, &blocked));
    assert!(
        matches!(&resumed.location,TeamLocation::Compartment {compartment} if compartment=="bridge")
    );
}
#[test]
fn malformed_layout_and_crossing_are_rejected() {
    let graph = graph();
    let mut config = graph.config.clone();
    config.connections[0].travel_secs = f32::NAN;
    assert!(InteriorGraph::compile(&config, &ship(), 1).is_err());
    let mut team = graph.initial_teams().remove(0);
    team.location = TeamLocation::Crossing {
        connection: "a".into(),
        from: "bridge".into(),
        to: "hangar".into(),
        elapsed: 0.0,
    };
    assert!(graph.validate_navigation(&[team]).is_err());
}
