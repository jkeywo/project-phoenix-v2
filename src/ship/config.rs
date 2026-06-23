use crate::messages::{PowerGroupId, StationId, SystemId};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ShipConfig {
    #[serde(rename = "station")]
    pub stations: Vec<StationConfig>,
    #[serde(rename = "system")]
    pub systems: Vec<SystemInstanceConfig>,
    #[serde(default)]
    pub power_groups: HashMap<PowerGroupId, PowerGroupConfig>,
    /// Seconds of artificial lag applied to every channel-3 coordination
    /// message (issue #494). Defaults to 2.0 seconds when absent.
    #[serde(default = "default_coordination_lag_secs")]
    pub coordination_lag_secs: f32,
}

fn default_coordination_lag_secs() -> f32 {
    2.0
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StationConfig {
    pub id: StationId,
    pub name: String,
    pub description: String,
    pub rank: String,
    #[serde(default)]
    pub short_code: String,
    pub console: String,
    #[serde(default, rename = "rating")]
    pub ratings: Vec<StationRatingConfig>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StationRatingConfig {
    pub name: String,
    pub automated_systems: Vec<SystemId>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SystemInstanceConfig {
    pub id: SystemId,
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub station: Option<StationId>,
    #[serde(default)]
    pub ai_only: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub power_group: Option<PowerGroupId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub marker: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config: Option<toml::Value>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PowerGroupConfig {
    pub label: String,
    #[serde(default = "default_power_level")]
    pub default_level: u8,
    #[serde(default = "default_min_power_level")]
    pub min_level: u8,
    #[serde(default = "default_max_power_level")]
    pub max_level: u8,
}

fn default_power_level() -> u8 {
    2
}

fn default_min_power_level() -> u8 {
    1
}

fn default_max_power_level() -> u8 {
    4
}

#[derive(Clone, Debug, PartialEq)]
pub enum ShipConfigError {
    ParseError(String),
    EmptyStations,
    EmptySystems,
    EmptyStationId,
    EmptySystemId,
    EmptySystemKind {
        system: SystemId,
    },
    EmptyPowerGroupId,
    ReservedCoreStationId {
        station: StationId,
    },
    DuplicateStationId {
        id: StationId,
    },
    DuplicateSystemId {
        id: SystemId,
    },
    UnknownSystemKind {
        system: SystemId,
        kind: String,
    },
    OwnerlessSystemWithoutAiOnly {
        system: SystemId,
    },
    UnknownStation {
        system: SystemId,
        station: StationId,
    },
    UnknownPowerGroup {
        system: SystemId,
        power_group: PowerGroupId,
    },
    DanglingRatingReference {
        station: StationId,
        rating: String,
        system: SystemId,
    },
    RatingReferencesUnownedSystem {
        station: StationId,
        rating: String,
        system: SystemId,
        owner: Option<StationId>,
    },
    DuplicateRatingName {
        station: StationId,
        rating: String,
    },
}

impl std::fmt::Display for ShipConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self)
    }
}

impl std::error::Error for ShipConfigError {}

impl ShipConfig {
    pub fn from_toml(
        toml_str: &str,
        registered_system_kinds: &[&str],
    ) -> Result<Self, ShipConfigError> {
        parse_ship_config(toml_str, registered_system_kinds)
    }

    pub fn station(&self, id: &StationId) -> Option<&StationConfig> {
        self.stations.iter().find(|station| &station.id == id)
    }

    pub fn system(&self, id: &SystemId) -> Option<&SystemInstanceConfig> {
        self.systems.iter().find(|system| &system.id == id)
    }

    pub fn systems_for_station<'a>(
        &'a self,
        id: &'a StationId,
    ) -> impl Iterator<Item = &'a SystemInstanceConfig> + 'a {
        self.systems
            .iter()
            .filter(move |system| system.station.as_ref() == Some(id))
    }

    /// Find the station whose `console` field matches the given string.
    /// Used to map a `Console` variant back to the owning station config.
    pub fn station_for_console(&self, console_id: &str) -> Option<&StationConfig> {
        self.stations.iter().find(|s| s.console == console_id)
    }

    pub fn systems_in_power_group<'a>(
        &'a self,
        id: &'a PowerGroupId,
    ) -> impl Iterator<Item = &'a SystemInstanceConfig> + 'a {
        self.systems
            .iter()
            .filter(move |system| system.power_group.as_ref() == Some(id))
    }
}

/// Parse and validate the station/system ship config sections from TOML.
///
/// `registered_system_kinds` is the set of code-backed system kinds available
/// to this build. Issue #490 replaces the caller-side list with the real system
/// registry; this verifier already enforces the contract.
pub fn parse_and_validate(
    toml_str: &str,
    registered_system_kinds: &[&str],
) -> Result<ShipConfig, ShipConfigError> {
    ShipConfig::from_toml(toml_str, registered_system_kinds)
}

pub fn parse_ship_config(
    toml_str: &str,
    registered_system_kinds: &[&str],
) -> Result<ShipConfig, ShipConfigError> {
    let config: ShipConfig =
        toml::from_str(toml_str).map_err(|e| ShipConfigError::ParseError(e.to_string()))?;
    validate(&config, registered_system_kinds)?;
    Ok(config)
}

pub fn validate(
    config: &ShipConfig,
    registered_system_kinds: &[&str],
) -> Result<(), ShipConfigError> {
    if config.stations.is_empty() {
        return Err(ShipConfigError::EmptyStations);
    }
    if config.systems.is_empty() {
        return Err(ShipConfigError::EmptySystems);
    }

    let registered_kinds: HashSet<&str> = registered_system_kinds.iter().copied().collect();
    let mut station_ids = HashSet::new();
    for station in &config.stations {
        if station.id.0.trim().is_empty() {
            return Err(ShipConfigError::EmptyStationId);
        }
        if station.id.0 == "core" {
            return Err(ShipConfigError::ReservedCoreStationId {
                station: station.id.clone(),
            });
        }
        if !station_ids.insert(station.id.clone()) {
            return Err(ShipConfigError::DuplicateStationId {
                id: station.id.clone(),
            });
        }

        let mut rating_names = HashSet::new();
        for rating in &station.ratings {
            if !rating_names.insert(rating.name.clone()) {
                return Err(ShipConfigError::DuplicateRatingName {
                    station: station.id.clone(),
                    rating: rating.name.clone(),
                });
            }
        }
    }

    let power_group_ids: HashSet<PowerGroupId> = config.power_groups.keys().cloned().collect();
    for power_group_id in &power_group_ids {
        if power_group_id.0.trim().is_empty() {
            return Err(ShipConfigError::EmptyPowerGroupId);
        }
    }
    let station_id_set: HashSet<StationId> = config.stations.iter().map(|s| s.id.clone()).collect();
    let mut system_ids = HashSet::new();
    let mut system_owner_by_id: HashMap<SystemId, Option<StationId>> = HashMap::new();

    for system in &config.systems {
        if system.id.0.trim().is_empty() {
            return Err(ShipConfigError::EmptySystemId);
        }
        if system.kind.trim().is_empty() {
            return Err(ShipConfigError::EmptySystemKind {
                system: system.id.clone(),
            });
        }
        if !system_ids.insert(system.id.clone()) {
            return Err(ShipConfigError::DuplicateSystemId {
                id: system.id.clone(),
            });
        }
        if !registered_kinds.contains(system.kind.as_str()) {
            return Err(ShipConfigError::UnknownSystemKind {
                system: system.id.clone(),
                kind: system.kind.clone(),
            });
        }
        if system.station.is_none() && !system.ai_only {
            return Err(ShipConfigError::OwnerlessSystemWithoutAiOnly {
                system: system.id.clone(),
            });
        }
        if let Some(station) = &system.station {
            if !station_id_set.contains(station) {
                return Err(ShipConfigError::UnknownStation {
                    system: system.id.clone(),
                    station: station.clone(),
                });
            }
        }
        if let Some(power_group) = &system.power_group {
            if !power_group_ids.contains(power_group) {
                return Err(ShipConfigError::UnknownPowerGroup {
                    system: system.id.clone(),
                    power_group: power_group.clone(),
                });
            }
        }
        system_owner_by_id.insert(system.id.clone(), system.station.clone());
    }

    for station in &config.stations {
        for rating in &station.ratings {
            for system_id in &rating.automated_systems {
                let Some(owner) = system_owner_by_id.get(system_id) else {
                    return Err(ShipConfigError::DanglingRatingReference {
                        station: station.id.clone(),
                        rating: rating.name.clone(),
                        system: system_id.clone(),
                    });
                };
                if owner.as_ref() != Some(&station.id) {
                    return Err(ShipConfigError::RatingReferencesUnownedSystem {
                        station: station.id.clone(),
                        rating: rating.name.clone(),
                        system: system_id.clone(),
                        owner: owner.clone(),
                    });
                }
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const KINDS: &[&str] = &[
        "red_alert",
        "helm",
        "phaser_bank",
        "torpedo_magazine",
        "torpedo_tube",
        "viewscreen",
    ];

    fn valid_toml() -> &'static str {
        r#"
[[station]]
id = "captain"
name = "Captain"
description = "Command the bridge."
rank = "Cpt."
short_code = "CPT"
console = "captain"

[[station.rating]]
name = "Assisted"
automated_systems = ["red-alert"]

[[station.rating]]
name = "Manual"
automated_systems = []

[[station]]
id = "tactical"
name = "Tactical"
description = "Weapons and threat response."
rank = "Ltn."
short_code = "TAC"
console = "tactical"

[[station.rating]]
name = "Assisted"
automated_systems = ["torpedo-magazine", "torpedo-tube-fore-port"]

[power_groups.ops]
label = "Operations"
default_level = 2
min_level = 1
max_level = 4

[power_groups.weapons]
label = "Weapons"
default_level = 2
min_level = 1
max_level = 4

[[system]]
id = "red-alert"
kind = "red_alert"
station = "captain"
power_group = "ops"

[[system]]
id = "phaser-fore"
kind = "phaser_bank"
station = "tactical"
power_group = "weapons"
marker = "phasers_fore"

[system.config]
facing_deg = 0
fire_arc_deg = 270

[[system]]
id = "torpedo-magazine"
kind = "torpedo_magazine"
station = "tactical"
power_group = "weapons"

[[system]]
id = "torpedo-tube-fore-port"
kind = "torpedo_tube"
station = "tactical"
power_group = "weapons"

[[system]]
id = "viewscreen"
kind = "viewscreen"
ai_only = true
power_group = "ops"
"#
    }

    fn parse_ok(toml: &str) -> ShipConfig {
        parse_and_validate(toml, KINDS).expect("ship config should parse")
    }

    #[test]
    fn new_ship_toml_parses_into_typed_model() {
        let config = parse_ok(valid_toml());

        assert_eq!(config.stations.len(), 2);
        assert_eq!(config.systems.len(), 5);
        assert_eq!(config.stations[1].id, StationId("tactical".into()));
        assert_eq!(config.stations[1].ratings[0].name, "Assisted");
        assert_eq!(
            config.stations[1].ratings[0].automated_systems,
            vec![
                SystemId("torpedo-magazine".into()),
                SystemId("torpedo-tube-fore-port".into())
            ]
        );
        assert_eq!(
            config.power_groups[&PowerGroupId("weapons".into())].label,
            "Weapons"
        );
    }

    #[test]
    fn ship_config_round_trips_through_toml() {
        let config = parse_ok(valid_toml());
        let encoded = toml::to_string(&config).expect("ship config should serialize");
        let decoded = parse_ok(&encoded);

        assert_eq!(decoded, config);
    }

    #[test]
    fn accessors_find_stations_systems_and_power_group_members() {
        let config = ShipConfig::from_toml(valid_toml(), KINDS).unwrap();

        assert_eq!(
            config
                .station(&StationId("tactical".into()))
                .map(|s| &s.name),
            Some(&"Tactical".to_string())
        );
        assert_eq!(
            config
                .system(&SystemId("phaser-fore".into()))
                .map(|s| &s.kind),
            Some(&"phaser_bank".to_string())
        );
        assert_eq!(
            config
                .systems_for_station(&StationId("tactical".into()))
                .map(|s| s.id.clone())
                .collect::<Vec<_>>(),
            vec![
                SystemId("phaser-fore".into()),
                SystemId("torpedo-magazine".into()),
                SystemId("torpedo-tube-fore-port".into())
            ]
        );
        assert_eq!(
            config
                .systems_in_power_group(&PowerGroupId("ops".into()))
                .map(|s| s.id.clone())
                .collect::<Vec<_>>(),
            vec![SystemId("red-alert".into()), SystemId("viewscreen".into())]
        );
    }

    #[test]
    fn rejects_ownerless_without_ai_only() {
        let toml = valid_toml().replace("ai_only = true", "ai_only = false");

        assert_eq!(
            parse_and_validate(&toml, KINDS),
            Err(ShipConfigError::OwnerlessSystemWithoutAiOnly {
                system: SystemId("viewscreen".into())
            })
        );
    }

    #[test]
    fn rejects_core_as_station_id() {
        let toml = valid_toml().replace("id = \"captain\"", "id = \"core\"");

        assert_eq!(
            parse_and_validate(&toml, KINDS),
            Err(ShipConfigError::ReservedCoreStationId {
                station: StationId("core".into())
            })
        );
    }

    #[test]
    fn rejects_missing_required_station_description() {
        let toml = valid_toml().replace("description = \"Command the bridge.\"\n", "");

        assert!(matches!(
            parse_and_validate(&toml, KINDS),
            Err(ShipConfigError::ParseError(_))
        ));
    }

    #[test]
    fn rejects_missing_required_station_rank() {
        let toml = valid_toml().replace("rank = \"Cpt.\"\n", "");

        assert!(matches!(
            parse_and_validate(&toml, KINDS),
            Err(ShipConfigError::ParseError(_))
        ));
    }

    #[test]
    fn rejects_missing_required_rating_automated_systems() {
        let toml = valid_toml().replace("automated_systems = []\n", "");

        assert!(matches!(
            parse_and_validate(&toml, KINDS),
            Err(ShipConfigError::ParseError(_))
        ));
    }

    #[test]
    fn rejects_empty_system_id() {
        let toml = valid_toml().replace("id = \"viewscreen\"", "id = \"\"");

        assert_eq!(
            parse_and_validate(&toml, KINDS),
            Err(ShipConfigError::EmptySystemId)
        );
    }

    #[test]
    fn rejects_duplicate_system_id() {
        let toml = valid_toml().replace("id = \"viewscreen\"", "id = \"red-alert\"");

        assert_eq!(
            parse_and_validate(&toml, KINDS),
            Err(ShipConfigError::DuplicateSystemId {
                id: SystemId("red-alert".into())
            })
        );
    }

    #[test]
    fn rejects_dangling_rating_reference() {
        let toml = valid_toml().replace(
            "automated_systems = [\"torpedo-magazine\", \"torpedo-tube-fore-port\"]",
            "automated_systems = [\"torpedo-magazine\", \"missing-system\"]",
        );

        assert_eq!(
            parse_and_validate(&toml, KINDS),
            Err(ShipConfigError::DanglingRatingReference {
                station: StationId("tactical".into()),
                rating: "Assisted".into(),
                system: SystemId("missing-system".into())
            })
        );
    }

    #[test]
    fn rejects_rating_reference_to_unowned_system() {
        let toml = valid_toml().replace(
            "automated_systems = [\"torpedo-magazine\", \"torpedo-tube-fore-port\"]",
            "automated_systems = [\"torpedo-magazine\", \"red-alert\"]",
        );

        assert_eq!(
            parse_and_validate(&toml, KINDS),
            Err(ShipConfigError::RatingReferencesUnownedSystem {
                station: StationId("tactical".into()),
                rating: "Assisted".into(),
                system: SystemId("red-alert".into()),
                owner: Some(StationId("captain".into()))
            })
        );
    }

    #[test]
    fn rejects_unknown_system_kind() {
        let toml = valid_toml().replace("kind = \"viewscreen\"", "kind = \"magic\"");

        assert_eq!(
            parse_and_validate(&toml, KINDS),
            Err(ShipConfigError::UnknownSystemKind {
                system: SystemId("viewscreen".into()),
                kind: "magic".into()
            })
        );
    }

    #[test]
    fn rejects_unknown_power_group() {
        let toml = valid_toml().replace("power_group = \"weapons\"", "power_group = \"missing\"");

        assert_eq!(
            parse_and_validate(&toml, KINDS),
            Err(ShipConfigError::UnknownPowerGroup {
                system: SystemId("phaser-fore".into()),
                power_group: PowerGroupId("missing".into())
            })
        );
    }

    #[test]
    fn rejects_unknown_station_owner() {
        let toml = valid_toml().replace("station = \"captain\"", "station = \"ghost\"");

        assert_eq!(
            parse_and_validate(&toml, KINDS),
            Err(ShipConfigError::UnknownStation {
                system: SystemId("red-alert".into()),
                station: StationId("ghost".into())
            })
        );
    }
}
