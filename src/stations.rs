use std::collections::HashMap;
use serde::Deserialize;

#[cfg(feature = "server")]
use bevy::prelude::Resource;

use crate::messages::Console;

// ── TOML schema types ────────────────────────────────────────────────────────

/// A single station definition within a player-count bucket.
#[derive(Clone, Debug, PartialEq)]
pub struct StationDef {
    pub name: String,
    pub description: String,
    pub consoles: Vec<Console>,
    /// Name of the station that this station promotes to when a player joins.
    pub next: Option<String>,
    /// Name of the station that this station demotes to when a player leaves.
    pub previous: Option<String>,
}

/// The fully-parsed, validated station configuration.
#[cfg_attr(feature = "server", derive(Resource))]
#[derive(Clone, Debug)]
pub struct ShipStations {
    /// Map from player count → ordered list of station definitions.
    pub configs: HashMap<u32, Vec<StationDef>>,
    pub min_players: u32,
    pub max_players: u32,
}

// ── Error type ───────────────────────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq)]
pub enum StationConfigError {
    /// A `next` field names a station that does not exist at count+1.
    DanglingNext {
        count: u32,
        station: String,
        target: String,
    },
    /// A `previous` field names a station that does not exist at count-1.
    DanglingPrevious {
        count: u32,
        station: String,
        target: String,
    },
    /// Two stations at the same player count share the same name.
    DuplicateName { count: u32, name: String },
    /// A console name in the TOML could not be mapped to a `Console` variant.
    UnknownConsole { count: u32, station: String, console: String },
    /// A station's `consoles` list is empty.
    EmptyConsoles { count: u32, station: String },
    /// A player count is outside the declared `min_players`/`max_players` range.
    CountOutOfRange { count: u32, min: u32, max: u32 },
    /// When count+1 has stations but none has the same name as this station and
    /// no explicit `next` was given.
    MissingNext { count: u32, station: String },
    /// The TOML could not be parsed.
    ParseError(String),
}

impl std::fmt::Display for StationConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self)
    }
}

impl std::error::Error for StationConfigError {}

// ── Raw TOML deserialization types ───────────────────────────────────────────

#[derive(Deserialize, Debug)]
struct RawConfig {
    stations: RawStations,
}

#[derive(Deserialize, Debug)]
struct RawStations {
    min_players: u32,
    max_players: u32,
    #[serde(flatten)]
    counts: HashMap<String, Vec<RawStationDef>>,
}

#[derive(Deserialize, Debug)]
struct RawStationDef {
    name: String,
    #[serde(default)]
    description: String,
    consoles: Vec<String>,
    next: Option<String>,
    previous: Option<String>,
}

// ── Console name parsing ─────────────────────────────────────────────────────

fn parse_console(s: &str, count: u32, station: &str) -> Result<Console, StationConfigError> {
    match s {
        "CaptainChair" => Ok(Console::CaptainChair),
        "Helm" => Ok(Console::Helm),
        "Tactical" => Ok(Console::Tactical),
        "Engineering" => Ok(Console::Engineering),
        "Science" => Ok(Console::Science),
        other => Err(StationConfigError::UnknownConsole {
            count,
            station: station.to_string(),
            console: other.to_string(),
        }),
    }
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Parse and validate a TOML string that contains a `[stations]` section.
///
/// The expected schema looks like:
/// ```toml
/// [stations]
/// min_players = 1
/// max_players = 4
///
/// [[stations.1]]
/// name = "Bridge"
/// consoles = ["CaptainChair", "Helm"]
/// ```
pub fn parse_and_validate(toml_str: &str) -> Result<ShipStations, StationConfigError> {
    let raw: RawConfig = toml::from_str(toml_str)
        .map_err(|e| StationConfigError::ParseError(e.to_string()))?;

    let raw_stations = raw.stations;
    let min = raw_stations.min_players;
    let max = raw_stations.max_players;

    // Build the typed map, validating console names and empty-consoles
    let mut configs: HashMap<u32, Vec<StationDef>> = HashMap::new();

    for (key, raw_defs) in &raw_stations.counts {
        let count: u32 = key.parse().map_err(|_| {
            StationConfigError::ParseError(format!("invalid player count key: {key}"))
        })?;

        if count < min || count > max {
            return Err(StationConfigError::CountOutOfRange { count, min, max });
        }

        // Check duplicate names within this count
        let mut seen_names = std::collections::HashSet::new();
        let mut defs = Vec::new();
        for raw_def in raw_defs {
            if !seen_names.insert(raw_def.name.clone()) {
                return Err(StationConfigError::DuplicateName {
                    count,
                    name: raw_def.name.clone(),
                });
            }
            if raw_def.consoles.is_empty() {
                return Err(StationConfigError::EmptyConsoles {
                    count,
                    station: raw_def.name.clone(),
                });
            }
            let consoles = raw_def
                .consoles
                .iter()
                .map(|c| parse_console(c, count, &raw_def.name))
                .collect::<Result<Vec<_>, _>>()?;

            defs.push(StationDef {
                name: raw_def.name.clone(),
                description: raw_def.description.clone(),
                consoles,
                next: raw_def.next.clone(),
                previous: raw_def.previous.clone(),
            });
        }
        configs.insert(count, defs);
    }

    // Validate explicit next/previous references and check MissingNext.
    // Implicit next/previous (same name at adjacent count) are ONLY validated
    // when the field is explicitly set; a station that happens to share a name
    // across counts is allowed — no error if the matching name doesn't exist.
    for count in min..=max {
        let Some(defs) = configs.get(&count) else {
            continue;
        };
        let defs = defs.clone();
        for def in &defs {
            // --- next ---
            if let Some(explicit_next) = &def.next {
                // Explicit next must resolve at count+1.
                let next_count = count + 1;
                match configs.get(&next_count) {
                    Some(next_defs) if next_defs.iter().any(|d| &d.name == explicit_next) => {}
                    _ => {
                        return Err(StationConfigError::DanglingNext {
                            count,
                            station: def.name.clone(),
                            target: explicit_next.clone(),
                        });
                    }
                }
            } else if count < max {
                // No explicit next: check whether count+1 exists and has the
                // same-named station (implicit next). If count+1 exists but the
                // same name is absent → MissingNext.
                if let Some(next_defs) = configs.get(&(count + 1)) {
                    if !next_defs.iter().any(|d| d.name == def.name) {
                        return Err(StationConfigError::MissingNext {
                            count,
                            station: def.name.clone(),
                        });
                    }
                }
                // If count+1 has no entries at all, that is fine — this station
                // simply has no successor.
            }

            // --- previous ---
            if let Some(explicit_prev) = &def.previous {
                // Explicit previous must resolve at count-1.
                let prev_count = count - 1;
                match configs.get(&prev_count) {
                    Some(prev_defs) if prev_defs.iter().any(|d| &d.name == explicit_prev) => {}
                    _ => {
                        return Err(StationConfigError::DanglingPrevious {
                            count,
                            station: def.name.clone(),
                            target: explicit_prev.clone(),
                        });
                    }
                }
            }
            // Implicit previous is informational only; no validation error.
        }
    }

    Ok(ShipStations { configs, min_players: min, max_players: max })
}

/// Look up a station by player count and name. Returns `None` if the count
/// has no stations or the name is not found.
pub fn get_station<'a>(
    stations: &'a ShipStations,
    player_count: u32,
    name: &str,
) -> Option<&'a StationDef> {
    stations
        .configs
        .get(&player_count)
        .and_then(|defs| defs.iter().find(|d| d.name == name))
}

/// Returns `true` when every station at `player_count` has at least one of its
/// consoles represented in `current` (spectators — players with no consoles —
/// do not count).
///
/// `current` is the set of consoles that are currently occupied.
pub fn all_stations_filled(
    stations: &ShipStations,
    player_count: u32,
    current: &[Console],
) -> bool {
    let Some(defs) = stations.configs.get(&player_count) else {
        return false;
    };
    defs.iter()
        .all(|def| def.consoles.iter().any(|c| current.contains(c)))
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── Helper TOML fixtures ─────────────────────────────────────────────────

    fn minimal_toml() -> &'static str {
        r#"
[stations]
min_players = 1
max_players = 1

[[stations.1]]
name = "Bridge"
description = "Single-crew bridge"
consoles = ["CaptainChair", "Helm"]
"#
    }

    fn multi_count_toml() -> &'static str {
        r#"
[stations]
min_players = 1
max_players = 4

[[stations.1]]
name = "Bridge"
consoles = ["CaptainChair", "Helm", "Tactical"]

[[stations.2]]
name = "Bridge"
consoles = ["CaptainChair", "Helm"]

[[stations.2]]
name = "Weapons"
consoles = ["Tactical"]

[[stations.3]]
name = "Bridge"
consoles = ["CaptainChair", "Helm"]

[[stations.3]]
name = "Weapons"
consoles = ["Tactical"]

[[stations.3]]
name = "Ops"
consoles = ["Engineering"]

[[stations.4]]
name = "Bridge"
consoles = ["CaptainChair", "Helm"]

[[stations.4]]
name = "Weapons"
consoles = ["Tactical"]

[[stations.4]]
name = "Ops"
consoles = ["Engineering"]

[[stations.4]]
name = "Science"
consoles = ["Science"]
"#
    }

    // ── Tracer bullet: happy-path parse ──────────────────────────────────────

    #[test]
    fn parse_minimal_toml_succeeds() {
        let result = parse_and_validate(minimal_toml());
        assert!(result.is_ok(), "expected Ok, got {:?}", result);
    }

    #[test]
    fn parsed_stations_has_correct_bounds() {
        let s = parse_and_validate(minimal_toml()).unwrap();
        assert_eq!(s.min_players, 1);
        assert_eq!(s.max_players, 1);
    }

    #[test]
    fn parsed_station_def_fields_are_correct() {
        let s = parse_and_validate(minimal_toml()).unwrap();
        let def = get_station(&s, 1, "Bridge").expect("Bridge at 1 player not found");
        assert_eq!(def.name, "Bridge");
        assert_eq!(def.description, "Single-crew bridge");
        assert_eq!(def.consoles, vec![Console::CaptainChair, Console::Helm]);
    }

    // ── Multi-count happy path ───────────────────────────────────────────────

    #[test]
    fn parse_multi_count_toml_succeeds() {
        let result = parse_and_validate(multi_count_toml());
        assert!(result.is_ok(), "expected Ok, got {:?}", result);
    }

    #[test]
    fn multi_count_has_expected_station_counts() {
        let s = parse_and_validate(multi_count_toml()).unwrap();
        assert_eq!(s.configs[&1].len(), 1);
        assert_eq!(s.configs[&2].len(), 2);
        assert_eq!(s.configs[&3].len(), 3);
        assert_eq!(s.configs[&4].len(), 4);
    }

    // ── get_station lookup ───────────────────────────────────────────────────

    #[test]
    fn get_station_hit() {
        let s = parse_and_validate(multi_count_toml()).unwrap();
        let def = get_station(&s, 3, "Ops");
        assert!(def.is_some());
        assert_eq!(def.unwrap().consoles, vec![Console::Engineering]);
    }

    #[test]
    fn get_station_miss_unknown_name() {
        let s = parse_and_validate(multi_count_toml()).unwrap();
        assert!(get_station(&s, 2, "Nonexistent").is_none());
    }

    #[test]
    fn get_station_miss_unknown_count() {
        let s = parse_and_validate(minimal_toml()).unwrap();
        assert!(get_station(&s, 99, "Bridge").is_none());
    }

    // ── all_stations_filled ───────────────────────────────────────────────────

    #[test]
    fn all_stations_filled_when_all_occupied() {
        let s = parse_and_validate(multi_count_toml()).unwrap();
        let current = vec![Console::CaptainChair, Console::Tactical];
        assert!(all_stations_filled(&s, 2, &current));
    }

    #[test]
    fn all_stations_not_filled_when_one_missing() {
        let s = parse_and_validate(multi_count_toml()).unwrap();
        // Only Bridge occupied; Weapons (Tactical) is not
        let current = vec![Console::CaptainChair];
        assert!(!all_stations_filled(&s, 2, &current));
    }

    #[test]
    fn all_stations_filled_returns_false_for_unknown_count() {
        let s = parse_and_validate(minimal_toml()).unwrap();
        let current = vec![Console::CaptainChair];
        assert!(!all_stations_filled(&s, 99, &current));
    }

    // ── StationConfigError variants ──────────────────────────────────────────

    #[test]
    fn error_duplicate_name_same_count() {
        let toml = r#"
[stations]
min_players = 1
max_players = 1

[[stations.1]]
name = "Bridge"
consoles = ["Helm"]

[[stations.1]]
name = "Bridge"
consoles = ["Tactical"]
"#;
        let err = parse_and_validate(toml).unwrap_err();
        assert!(
            matches!(err, StationConfigError::DuplicateName { count: 1, .. }),
            "unexpected error: {:?}",
            err
        );
    }

    #[test]
    fn error_unknown_console() {
        let toml = r#"
[stations]
min_players = 1
max_players = 1

[[stations.1]]
name = "Bridge"
consoles = ["Torpedoes"]
"#;
        let err = parse_and_validate(toml).unwrap_err();
        assert!(
            matches!(err, StationConfigError::UnknownConsole { .. }),
            "unexpected error: {:?}",
            err
        );
    }

    #[test]
    fn error_empty_consoles() {
        let toml = r#"
[stations]
min_players = 1
max_players = 1

[[stations.1]]
name = "Bridge"
consoles = []
"#;
        let err = parse_and_validate(toml).unwrap_err();
        assert!(
            matches!(err, StationConfigError::EmptyConsoles { count: 1, .. }),
            "unexpected error: {:?}",
            err
        );
    }

    #[test]
    fn error_count_out_of_range() {
        let toml = r#"
[stations]
min_players = 2
max_players = 4

[[stations.1]]
name = "Bridge"
consoles = ["Helm"]
"#;
        let err = parse_and_validate(toml).unwrap_err();
        assert!(
            matches!(err, StationConfigError::CountOutOfRange { count: 1, min: 2, max: 4 }),
            "unexpected error: {:?}",
            err
        );
    }

    #[test]
    fn error_dangling_next() {
        let toml = r#"
[stations]
min_players = 1
max_players = 2

[[stations.1]]
name = "Bridge"
consoles = ["Helm"]
next = "NoSuchStation"

[[stations.2]]
name = "Bridge"
consoles = ["CaptainChair", "Helm"]
"#;
        let err = parse_and_validate(toml).unwrap_err();
        assert!(
            matches!(err, StationConfigError::DanglingNext { count: 1, .. }),
            "unexpected error: {:?}",
            err
        );
    }

    #[test]
    fn error_dangling_previous() {
        let toml = r#"
[stations]
min_players = 1
max_players = 2

[[stations.1]]
name = "Bridge"
consoles = ["Helm"]

[[stations.2]]
name = "Bridge"
consoles = ["CaptainChair", "Helm"]
previous = "NoSuchStation"
"#;
        let err = parse_and_validate(toml).unwrap_err();
        assert!(
            matches!(err, StationConfigError::DanglingPrevious { count: 2, .. }),
            "unexpected error: {:?}",
            err
        );
    }

    #[test]
    fn error_missing_next_when_count_plus_one_has_no_same_named_station() {
        // At count=1 "Alpha" has no explicit next and count=2 has "Beta" only.
        let toml = r#"
[stations]
min_players = 1
max_players = 2

[[stations.1]]
name = "Alpha"
consoles = ["Helm"]

[[stations.2]]
name = "Beta"
consoles = ["CaptainChair"]
"#;
        let err = parse_and_validate(toml).unwrap_err();
        assert!(
            matches!(err, StationConfigError::MissingNext { count: 1, .. }),
            "unexpected error: {:?}",
            err
        );
    }

    #[test]
    fn error_parse_error_on_invalid_toml() {
        let err = parse_and_validate("this is not toml ][").unwrap_err();
        assert!(
            matches!(err, StationConfigError::ParseError(_)),
            "unexpected error: {:?}",
            err
        );
    }

    // ── Implicit next/previous resolution ───────────────────────────────────

    #[test]
    fn implicit_next_resolved_when_same_name_exists_at_next_count() {
        // multi_count_toml has "Bridge" at all counts — should validate fine
        let result = parse_and_validate(multi_count_toml());
        assert!(result.is_ok(), "implicit next resolution failed: {:?}", result);
    }

    #[test]
    fn explicit_next_overrides_implicit() {
        let toml = r#"
[stations]
min_players = 1
max_players = 2

[[stations.1]]
name = "Solo"
consoles = ["Helm"]
next = "Duo"

[[stations.2]]
name = "Duo"
consoles = ["CaptainChair", "Helm"]
"#;
        let result = parse_and_validate(toml);
        assert!(result.is_ok(), "explicit next failed: {:?}", result);
    }

    // ── player_ship.toml integration ─────────────────────────────────────────

    /// Verify that the actual `assets/entities/player_ship.toml` file contains a
    /// valid `[stations]` section that passes `parse_and_validate`.
    #[test]
    fn player_ship_toml_stations_section_is_valid() {
        let toml_str = include_str!("../assets/entities/player_ship.toml");
        let result = parse_and_validate(toml_str);
        assert!(
            result.is_ok(),
            "player_ship.toml stations section is invalid: {:?}",
            result
        );
    }

    /// Verify that the 1-player station at player_ship.toml gives access to
    /// every console (CaptainChair, Helm, Tactical, Engineering).
    #[test]
    fn player_ship_1p_station_covers_all_consoles() {
        let toml_str = include_str!("../assets/entities/player_ship.toml");
        let stations = parse_and_validate(toml_str).unwrap();
        let current = vec![
            Console::CaptainChair,
            Console::Helm,
            Console::Tactical,
            Console::Engineering,
        ];
        // all_stations_filled should return true at 1P when the solo player
        // holds all consoles.
        assert!(
            all_stations_filled(&stations, 1, &current),
            "1P station should be filled when player holds all expected consoles"
        );
    }
}
