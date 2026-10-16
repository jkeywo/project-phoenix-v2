use super::*;
use crate::core::messages::{PowerGroupId, StationId};
use crate::ship::config::{
    PowerGroupConfig, StationConfig, StationRatingConfig, SystemInstanceConfig,
};

#[test]
fn active_schema_and_runtime_rows_are_classified_without_generic_mutation() {
    let station = StationConfig {
        id: StationId("helm".into()),
        name: "Helm".into(),
        description: "Fly".into(),
        rank: "Officer".into(),
        short_code: "H".into(),
        ratings: vec![StationRatingConfig {
            detailed_systems: None,
            name: "Full".into(),
            automated_systems: vec![],
            ai_tuning: None,
        }],
        manual_overview: None,
        tutorials: vec![],
        stances: vec![],
    };
    let console = phoenix_model::console::ConsoleConfig {
        id: phoenix_model::identity::ConsoleId("helm-screen".into()),
        home_station: Some(station.id.clone()),
        screen: Some("consoles/helm.html".into()),
        ..Default::default()
    };
    let system = SystemInstanceConfig {
        damage_components: Vec::new(),
        components: Vec::new(),
        channels: Vec::new(),
        console: Some(console.id.clone()),
        id: SystemId("engine".into()),
        kind: "helm-engine".into(),
        ai_only: false,
        power_group: Some(PowerGroupId("engines".into())),
        marker: None,
        config: Some(toml::Value::Table(toml::Table::from_iter([(
            "cooldown_secs".into(),
            toml::Value::Float(3.0),
        )]))),
    };
    let config = ShipConfig {
        stations: vec![station],
        systems: vec![system],
        power_groups: HashMap::from([(
            PowerGroupId("engines".into()),
            PowerGroupConfig {
                label: "Engines".into(),
                default_level: 2,
                min_level: 1,
                max_level: 4,
            },
        )]),
        consoles: vec![console],
        coordination_lag_secs: 2.0,
    };
    let ratings = ActiveStationRatings(HashMap::from([(StationId("helm".into()), "Full".into())]));
    let controls = ControlSourceResolver::default();
    let hull = SystemHull::from_config(&[(SystemId("engine".into()), 100.0)]);
    let authored = EntityConfig {
        hull: Some(crate::entities::config::HullConfig {
            hull_integrity: 0.0,
            system_hull: vec![crate::entities::config::SystemHullEntry {
                system_id: SystemId("engine".into()),
                display_name: Some("Engine".into()),
                max_hp: 100.0,
                damaged_threshold_pct: 0.75,
                disabled_threshold_pct: 0.25,
                debuff_magnitude: 0.15,
            }],
        }),
        repair: Some(crate::entities::config::RepairConfig {
            repair_team_count: 2,
            ..Default::default()
        }),
        captain_console: Some(Default::default()),
        comms_console: Some(Default::default()),
        shield_arcs: vec![crate::entities::config::ShieldArcConfig {
            damage_components: Vec::new(),
            focus_available: true,
            id: "fore".into(),
            label: "Fore".into(),
            center_deg: 0.0,
            width_deg: 180.0,
            energy: Default::default(),
            ai: toml::from_str(include_str!(
                "../../../../../tests/fixtures/shield-arc-ai.toml"
            ))
            .unwrap(),
            max_hp: None,
            regen_per_sec: None,
            offline_duration: None,
            priority: 1,
            frequency: 0.5,
        }],
        ..Default::default()
    };
    let projection = projection([ShipInspectorInputs {
        id: "ship",
        label: "Ship",
        config: &config,
        authored: Some(&authored),
        authored_document: Some("assets/entities/ship.toml"),
        ratings: &ratings,
        controls: &controls,
        hull: Some(&hull),
        physical: None,
        blackboards: None,
    }]);
    let paths: BTreeMap<_, _> = projection
        .fields
        .iter()
        .map(|f| {
            (
                f.id.as_str(),
                (&f.descriptor.live_mutability, f.action_panel.as_deref()),
            )
        })
        .collect();
    assert_eq!(
        paths["system[engine].config.cooldown_secs"].0,
        &LiveMutability::RecreateRequired
    );
    assert_eq!(
        paths["runtime.system[engine].availability_action"].1,
        Some("system")
    );
    assert_eq!(
        paths["runtime.system[engine].health.current_hp"].0,
        &LiveMutability::Derived
    );
    assert_eq!(
        paths["runtime.system[engine].effect_action"].1,
        Some("effect")
    );
    assert!(projection.readings["ship"]
        .values
        .contains_key("station[helm].rating[Full].name"));
    assert_eq!(
        projection.readings["ship"].values["hull.system_hull[engine].max_hp"],
        "100.0"
    );
    assert_eq!(
        projection.readings["ship"].values["repair.repair_team_count"],
        "2"
    );
    assert_eq!(
        projection.readings["ship"].values["captain_console.ai"],
        "not-authored"
    );
    assert_eq!(
        projection.readings["ship"].values["comms_console.selector"],
        "not-authored"
    );
    assert_eq!(
        projection.readings["ship"].values["shield_arc[fore].max_hp"],
        "not-authored"
    );
    for retired in [
        "station[helm].host_order",
        "station[helm].visiting_rating",
        "station[helm].command_target",
        "system[engine].station",
        "system[engine].seek_order",
    ] {
        assert!(!paths.contains_key(retired), "retired descriptor {retired}");
    }
    assert_eq!(
        paths["station[helm].manual_overview"].0,
        &LiveMutability::RecreateRequired
    );
    assert_eq!(
        paths["system[engine].marker"].0,
        &LiveMutability::RecreateRequired
    );
    assert_eq!(
        projection.readings["ship"].values["provenance.document"],
        "assets/entities/ship.toml"
    );
}

#[test]
fn physical_systems_keep_checked_effect_links_and_readonly_condition_detail() {
    let authored = crate::entities::include_resolve::load_entity_config(
        "assets/entities/alliance_cruiser.toml",
    )
    .unwrap();
    let config = authored.ship_config.as_ref().unwrap();
    let damage = crate::ship::compartment_damage::ShipDamage::from_config(&authored)
        .unwrap()
        .unwrap();
    let ratings = ActiveStationRatings::default();
    let controls = ControlSourceResolver::default();
    let projection = projection([ShipInspectorInputs {
        id: "ship",
        label: "Ship",
        config,
        authored: Some(&authored),
        authored_document: Some("assets/entities/alliance_cruiser.toml"),
        ratings: &ratings,
        controls: &controls,
        hull: None,
        physical: Some(&damage),
        blackboards: None,
    }]);
    for system in &config.systems {
        let action = format!("runtime.system[{}].effect_action", system.id.0);
        let row = projection
            .fields
            .iter()
            .find(|row| row.id == action)
            .expect("physical System retains its checked GM scope action");
        assert_eq!(row.descriptor.live_mutability, LiveMutability::NamedAction);
        assert_eq!(row.action_panel.as_deref(), Some("effect"));
    }
    let physical: Vec<_> = projection
        .fields
        .iter()
        .filter(|row| row.id.starts_with("runtime.system[") && row.id.contains(".physical."))
        .collect();
    assert!(!physical.is_empty());
    for row in physical {
        assert_eq!(
            row.descriptor.live_mutability,
            LiveMutability::Derived,
            "{}",
            row.id
        );
        assert!(row.action_panel.is_none());
        assert!(!row.id.ends_with(".dose") && !row.id.ends_with(".last_source"));
    }
    assert!(
        !projection
            .fields
            .iter()
            .any(|row| row.id.contains(".health.current_hp")),
        "equipped entities do not expose a second System HP owner"
    );
    for row in projection.fields.iter().filter(|row| {
        !row.id.starts_with("runtime.")
            && (row.id.starts_with("damage.")
                || row.id.starts_with("interior.")
                || row.id.contains(".damage_component[")
                || row.id.contains(".impact."))
    }) {
        assert_eq!(
            row.descriptor.live_mutability,
            LiveMutability::RecreateRequired,
            "{}",
            row.id
        );
        assert!(row.action_panel.is_none());
    }
}
