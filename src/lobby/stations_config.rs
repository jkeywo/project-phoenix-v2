use std::collections::{HashMap, VecDeque};
use serde::{Deserialize, Serialize};
use bevy::prelude::Resource;

use crate::messages::Console;

// ── TOML schema types ────────────────────────────────────────────────────────

/// A single station definition within a player-count bucket.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StationDef {
    pub name: String,
    pub description: String,
    pub consoles: Vec<Console>,
    /// Rank displayed for players at this station (e.g., "Cpt.", "Ltn.")
    pub rank: String,
    /// Short identifier used in UI labels (e.g. "TAC" renders as "STN-TAC").
    #[serde(default)]
    pub short_code: String,
    /// Name of the station that this station promotes to when a player joins.
    pub next: Option<String>,
    /// Name of the station that this station demotes to when a player leaves.
    pub previous: Option<String>,
}

/// Return the default available complexity presets for every console.
pub fn default_complexity_presets() -> HashMap<Console, Vec<String>> {
    let mut m = HashMap::new();
    for c in &[Console::CaptainChair, Console::Helm, Console::Tactical, Console::Repair, Console::Power, Console::Comms] {
        m.insert(c.clone(), vec!["Low".into(), "Full".into()]);
    }
    for c in &[Console::Sensors, Console::Shields, Console::Navigation] {
        m.insert(c.clone(), vec!["Full".into()]);
    }
    m
}

/// The fully-parsed, validated station configuration.
#[derive(Resource, Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ShipStations {
    /// Map from player count → ordered list of station definitions.
    pub configs: HashMap<u32, Vec<StationDef>>,
    pub min_players: u32,
    pub max_players: u32,
    /// Per-console available complexity preset names.
    #[serde(default = "default_complexity_presets")]
    pub complexity_presets: HashMap<Console, Vec<String>>,
}

impl Default for ShipStations {
    fn default() -> Self {
        Self {
            configs: HashMap::new(),
            min_players: 0,
            max_players: 0,
            complexity_presets: default_complexity_presets(),
        }
    }
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
    #[serde(default)]
    rank: String,
    #[serde(default)]
    short_code: String,
    next: Option<String>,
    previous: Option<String>,
}

// ── Console name parsing ─────────────────────────────────────────────────────

fn parse_console(s: &str, count: u32, station: &str) -> Result<Console, StationConfigError> {
    match s {
        "CaptainChair" => Ok(Console::CaptainChair),
        "Helm" => Ok(Console::Helm),
        "Tactical" => Ok(Console::Tactical),
        "Repair" => Ok(Console::Repair),
        "Sensors" => Ok(Console::Sensors),
        "Shields" => Ok(Console::Shields),
        "Navigation" => Ok(Console::Navigation),
        "Power" => Ok(Console::Power),
        "Comms" => Ok(Console::Comms),
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

            let rank = if raw_def.rank.is_empty() {
                if raw_def.consoles.iter().any(|c| c == "CaptainChair") {
                    "Cpt.".to_string()
                } else {
                    "Ltn.".to_string()
                }
            } else {
                raw_def.rank.clone()
            };

            defs.push(StationDef {
                name: raw_def.name.clone(),
                description: raw_def.description.clone(),
                consoles,
                rank,
                short_code: raw_def.short_code.clone(),
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
                // Explicit next must resolve at count+1, unless we are already
                // at max (no higher player-count exists — next is irrelevant).
                if count < max {
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
                // Explicit previous must resolve at count-1, unless we are at
                // min (no lower player-count exists — previous is irrelevant).
                if count > min {
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
            }
            // Implicit previous is informational only; no validation error.
        }
    }

    Ok(ShipStations { configs, min_players: min, max_players: max, complexity_presets: default_complexity_presets() })
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

// ── Assignment types ─────────────────────────────────────────────────────────

/// Maps session token → station name.  A token absent from this map is a
/// spectator.
pub type StationAssignments = HashMap<String, String>;