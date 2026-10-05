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
        console: None,
        manual_overview: None,
        tutorials: vec![],
        human_seeking: false,
        host_order: vec![],
        visiting_rating: None,
        auxiliary: false,
        command_target: None,
        stances: vec![],
    };
    let system = SystemInstanceConfig {
        id: SystemId("engine".into()),
        kind: "helm-engine".into(),
        station: Some(station.id.clone()),
        ai_only: false,
        human_seeking: false,
        seek_order: vec![],
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
            id: "fore".into(),
            label: "Fore".into(),
            center_deg: 0.0,
            width_deg: 180.0,
            max_hp: None,
            regen_per_sec: None,
            offline_duration: None,
            hull_max_hp: 50.0,
            hull_damaged_threshold_pct: 0.75,
            hull_disabled_threshold_pct: 0.25,
            hull_debuff_magnitude: 0.15,
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
        hull: &hull,
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
