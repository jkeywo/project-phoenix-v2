use crate::core::messages::{PowerGroupId, StationId, SystemId, TutorialOverlayWire};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ShipConfig {
    #[serde(default, rename = "console")]
    pub consoles: Vec<phoenix_model::console::ConsoleConfig>,
    #[serde(default, rename = "station")]
    pub stations: Vec<StationConfig>,
    #[serde(default, rename = "system")]
    pub systems: Vec<SystemInstanceConfig>,
    #[serde(default)]
    pub power_groups: HashMap<PowerGroupId, PowerGroupConfig>,
    /// Seconds of artificial lag applied to every channel-3 coordination
    /// message (issue #494). Defaults to 2.0 seconds when absent; validation
    /// requires a finite, non-negative value before runtime converts it to
    /// logical ticks.
    #[serde(default = "default_coordination_lag_secs")]
    pub coordination_lag_secs: f32,
}

fn default_coordination_lag_secs() -> f32 {
    2.0
}

/// System kinds that constitute "the weapons suite" for
/// [`ShipConfig::weapons_station`]. Every weapon system on a hull lives on one
/// station by construction, so the first match wins.
const WEAPONS_KINDS: &[&str] = &[
    crate::ship::system_registry::PHASER_BANK_KIND,
    crate::ship::system_registry::BLASTER_BANK_KIND,
    crate::ship::system_registry::TORPEDO_TUBE_KIND,
    crate::ship::system_registry::TORPEDO_MAGAZINE_KIND,
    crate::ship::system_registry::TACTICAL_RADAR_KIND,
];

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StationConfig {
    pub id: StationId,
    pub name: String,
    pub description: String,
    pub rank: String,
    #[serde(default)]
    pub short_code: String,
    #[serde(default, rename = "rating")]
    pub ratings: Vec<StationRatingConfig>,
    /// Authored overview prose for this station's ship manual tab (issue #772).
    ///
    /// Authored `[[station]] manual_overview` accepts a `strings.csv` id or
    /// legacy literal prose. Rust carries it unchanged; the client resolves
    /// known ids with `wireText` and preserves mod-authored prose (#1540).
    /// `#[serde(default)]` so hulls that omit it (and the
    /// `rejects_missing_required_*` tests) keep parsing. See
    /// `crate::ship::manual` for how it is combined with generated sections.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manual_overview: Option<String>,
    /// Contextual tutorial overlays authored on this station (issue #916),
    /// from `[[station.tutorial]]` blocks. Pure carriage: Rust never
    /// interprets the trigger vocabulary — the client's tutorial
    /// state-builder (`gui/tutorial-state.js`) evaluates it. `title`/`text`
    /// hold `strings.csv` ids, enforced by `scripts/check-strings.mjs`.
    #[serde(default, rename = "tutorial", skip_serializing_if = "Vec::is_empty")]
    pub tutorials: Vec<TutorialOverlayWire>,
    /// This station's authored stance catalogue (issue #1107).
    ///
    /// Standard stances plus the two mandatory alert-neutral fallbacks
    /// (`normal_alert_neutral`, `high_alert_neutral`). A Command Console reads
    /// its `command_target`'s catalogue; the target station authors it here.
    /// Empty on stations that are never directed.
    #[serde(default, rename = "stance", skip_serializing_if = "Vec::is_empty")]
    pub stances: Vec<StationStanceConfig>,
}

pub use phoenix_model::console::ConsoleRatingConfig as StationRatingConfig;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SystemInstanceConfig {
    pub id: SystemId,
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub console: Option<crate::core::messages::ConsoleId>,
    #[serde(default)]
    pub ai_only: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub power_group: Option<PowerGroupId>,
    /// Optional rig-marker name for this system instance.
    ///
    /// **Declared but unread**: no runtime path resolves this against a model
    /// rig, so `crate::entities::marker_validate` deliberately excludes it from the
    /// model-marker contract (issue #758) — validating a field nothing
    /// consumes would invent a contract rather than check one. The shipped
    /// hulls carried a placeholder `marker = "ship"` here that named no rig
    /// marker at all; those entries were removed. When a consumer lands, add
    /// these references to `marker_validate::collect_marker_refs` and the
    /// missing/incompatible checks apply for free.
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

/// The level a `[power_groups.<id>]` entry is seeded at when its hull authors
/// no `default_level`. `const` so callers can define their own constants by
/// CALLING this rather than by restating the number next to it.
pub const fn default_power_level() -> u8 {
    2
}

/// The floor a `[power_groups.<id>]` entry gets when its hull authors no
/// `min_level`.
///
/// The authored `min_level` IS the lowest level any operator can command that
/// group to since issue #1395 — `PowerSystem` stores it per group and every
/// clamp in the allocation API reads it — so this is the default a group takes,
/// not a floor every group is held at. A hull may author `0` and mean it: that
/// group can be switched off.
///
/// `pub const` for the same reason as [`default_max_power_level`]: issue #959's
/// budget planner needs "the lowest level this group could be commanded to"
/// when it works out how much of the reactor budget is discretionary, and
/// restating `1` there would let the two drift.
pub use phoenix_model::wire::default_min_power_level;

/// The ceiling a `[power_groups.<id>]` entry gets when its hull authors no
/// `max_level` — and the ceiling the allocation API clamps every group to, so
/// it is also the highest level ANY operator can command a group to.
///
/// `pub const` because the battery-floor validation in
/// [`crate::entities::config`] needs "the highest level this group could be
/// commanded to" for a group whose hull describes no `[power_groups.*]` block
/// at all, and restating `4` next to that check would let the two drift.
pub const fn default_max_power_level() -> u8 {
    4
}

#[derive(Clone, Debug, PartialEq)]
pub enum ShipConfigError {
    ConsoleCommandTargetUnknownStation {
        console: crate::core::messages::ConsoleId,
        target: StationId,
    },
    ConsoleCommandTargetHasNoStances {
        console: crate::core::messages::ConsoleId,
        target: StationId,
    },
    ParseError(String),
    InvalidCoordinationLagSecs {
        value: f32,
    },
    EmptyStations,
    EmptySystems,
    EmptyStationId,
    EmptySystemId,
    EmptySystemKind {
        system: SystemId,
    },
    EmptyPowerGroupId,
    /// A `[power_groups.<id>]` block whose floor is not below its ceiling
    /// (issue #1395).
    ///
    /// `min_level` became a real allocation clamp in that issue, so the pair
    /// now describes a RANGE the reactor enforces rather than two independent
    /// display hints. `min_level == max_level` is a group with one legal level
    /// — a stepper with nothing to step to, which is not a power group — and
    /// `min_level > max_level` is a group with none at all, which would have
    /// `PowerSystem` seed it below its own floor. Both are authoring mistakes
    /// worth refusing at load rather than discovering as a dead console.
    InvalidPowerGroupLevels {
        group: PowerGroupId,
        min_level: u8,
        max_level: u8,
    },
    /// A `[power_groups.<id>]` block whose boot level is outside its own
    /// `[min_level, max_level]` range (issue #1395).
    ///
    /// The reactor clamps the seed into range, so this would not crash — it
    /// would silently spawn the ship somewhere other than where the hull says
    /// it boots, and the difference would surface as a reactor budget that does
    /// not add up against the numbers in the file.
    PowerGroupDefaultOutOfRange {
        group: PowerGroupId,
        default_level: u8,
        min_level: u8,
        max_level: u8,
    },
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
    ConflictingRatingDepth {
        station: StationId,
        rating: String,
        system: SystemId,
    },
    UnsupportedSimplifiedSystem {
        station: StationId,
        rating: String,
        system: SystemId,
    },
    /// A `seek_order` on a system that does not seek. The list would be read
    /// by nothing, so it is a typo rather than a preference.
    SeekOrderWithoutHumanSeeking {
        system: SystemId,
    },
    /// A `seek_order` entry naming a station this hull does not have.
    SeekOrderUnknownStation {
        system: SystemId,
        station: StationId,
    },
    /// The same station twice in one `seek_order`. The second visit could
    /// never fire, so the list does not mean what it looks like.
    SeekOrderDuplicateStation {
        system: SystemId,
        station: StationId,
    },
    /// A `seek_order` that leaves a station off. See the field's doc comment:
    /// the list is the whole walk, so an omitted station is a console the seek
    /// can never reach.
    SeekOrderMissingStation {
        system: SystemId,
        station: StationId,
    },
    /// A `seek_order` that does not begin at the system's own station.
    /// Owner-first is the rule that keeps a hull's own officer at their own
    /// console; authoring it away is never what an author means.
    SeekOrderOwnerNotFirst {
        system: SystemId,
        owner: StationId,
        first: Option<StationId>,
    },
    HostOrderWithoutHumanSeeking {
        station: StationId,
    },
    HostOrderUnknownStation {
        station: StationId,
        host: StationId,
    },
    HostOrderDuplicateStation {
        station: StationId,
        host: StationId,
    },
    MissingVisitingRating {
        station: StationId,
    },
    UnknownVisitingRating {
        station: StationId,
        rating: String,
    },
    /// A `command_target` naming a station this hull does not have (issue #1107).
    CommandTargetUnknownStation {
        station: StationId,
        target: StationId,
    },
    /// A `command_target` whose named station authors no stance catalogue at all
    /// (issue #1107). Command has nothing to list, so it is a content error.
    CommandTargetHasNoStances {
        station: StationId,
        target: StationId,
    },
    /// Two stances share an `id` within one station's catalogue (issue #1107).
    DuplicateStanceId {
        station: StationId,
        stance: String,
    },
    /// A stance catalogue is missing one of the two mandatory alert-neutral
    /// fallbacks, or authors it more than once (issue #1107). Every catalogue
    /// authors exactly one `normal_alert_neutral` and one `high_alert_neutral`.
    StanceCatalogueNeutralCount {
        station: StationId,
        kind: StanceKind,
        found: usize,
    },
    /// A neutral stance whose authored `high_alert` posture disagrees with its
    /// kind (issue #1107): `normal_alert_neutral` must be `false`,
    /// `high_alert_neutral` must be `true`.
    NeutralStancePostureMismatch {
        station: StationId,
        stance: String,
        kind: StanceKind,
    },
    /// A stance catalogue flags more than one stance `ai_engaged` (issue #1109).
    /// The AI Command seat's high-alert choice is a single authored posture, so
    /// at most one stance may carry the flag.
    MultipleAiEngagedStances {
        station: StationId,
    },
    /// A neutral stance is flagged `ai_engaged` (issue #1109). The flag names the
    /// posture an AI Command adopts ABOVE the alert-neutral tracking default, so
    /// it is meaningful only on a `standard` stance.
    AiEngagedStanceNotStandard {
        station: StationId,
        stance: String,
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
            .filter(move |system| self.system_home_station(system) == Some(id))
    }

    /// The single owning Station shared by every authored System of `kind`.
    ///
    /// Instance ids are author-defined and therefore cannot stand in for a
    /// capability kind. A missing kind, an ownerless instance, or instances
    /// split across Stations is ambiguous and returns `None` rather than
    /// widening the audience.
    pub fn station_for_system_kind(&self, kind: &str) -> Option<StationId> {
        let mut systems = self.systems.iter().filter(|system| system.kind == kind);
        let station = self.system_home_station(systems.next()?)?.clone();
        systems
            .all(|system| self.system_home_station(system) == Some(&station))
            .then_some(station)
    }

    /// The station whose holder is authoritative for this ship's weapons.
    ///
    /// Ship-level Tactical operations (SetTarget / SetPhaserMode /
    /// SetPhaserFrequency) and the `WeaponsUpdate` broadcast
    /// need to know who owns the guns. That owner is not always a station
    /// literally named `"tactical"` — a single-station hull (the Courier) puts
    /// its blaster on `"pilot"` — so resolve it from the config instead of
    /// assuming the name.
    ///
    /// Returns `None` for ships with no human weapons owner (NPCs declare no
    /// `station` on any system); callers treat that as "unclaimed".
    pub fn weapons_station(&self) -> Option<StationId> {
        self.systems
            .iter()
            .filter(|s| WEAPONS_KINDS.contains(&s.kind.as_str()))
            .find_map(|s| self.system_home_station(s).cloned())
            .or_else(|| {
                // Legacy/test ships declare a `tactical` station but no fine
                // weapon systems. Preserve the pre-lookup behaviour for them.
                let tactical = StationId(crate::ship::system_registry::TACTICAL_STATION_ID.into());
                self.station(&tactical).map(|_| tactical)
            })
    }

    /// The station whose holder is authoritative for this ship's Sensors
    /// system, mirroring [`weapons_station`](Self::weapons_station)'s
    /// per-ship station resolution.
    ///
    /// Its one production caller was the `auto_hint` claimed/unclaimed gate in
    /// the frequency-hint emitter, which issue #873 deleted: whether a human
    /// session held Sensors decided whether the ship emitted a coordination
    /// fact at all, which is the human/AI branch AGENTS.md rule 6 forbids. The
    /// accessor itself is sound and stays — resolving a hull's Sensors owner
    /// without assuming the station is called `"sensors"` is a real question —
    /// but a new caller should be sure it is asking "who do I address this
    /// to?", never "may this be emitted?".
    ///
    /// Returns `None` for ships with no human Sensors owner (NPCs declare no
    /// `station` on their `sensors` system, if they declare one at all);
    /// callers treat that as "unclaimed".
    pub fn sensors_station(&self) -> Option<StationId> {
        self.systems
            .iter()
            .find(|s| s.kind == crate::ship::system_registry::SENSORS_KIND)
            .and_then(|s| self.system_home_station(s).cloned())
    }

    /// Look up a named rating for a station.
    pub fn rating_for_station<'a>(
        &'a self,
        station_id: &StationId,
        rating_name: &str,
    ) -> Option<&'a StationRatingConfig> {
        self.station(station_id)?
            .ratings
            .iter()
            .find(|r| r.name == rating_name)
    }

    /// Check whether a station's named rating has the given AI rule in its `ai_tuning` table.
    pub fn has_ai_rule(&self, station_id: &StationId, rating_name: &str, rule: &str) -> bool {
        self.rating_for_station(station_id, rating_name)
            .and_then(|r| r.ai_tuning.as_ref())
            .and_then(|t| t.as_table())
            .is_some_and(|tbl| tbl.contains_key(rule))
    }

    pub fn systems_in_power_group<'a>(
        &'a self,
        id: &'a PowerGroupId,
    ) -> impl Iterator<Item = &'a SystemInstanceConfig> + 'a {
        self.systems
            .iter()
            .filter(move |system| system.power_group.as_ref() == Some(id))
    }

    /// The power group one system draws from — its authored
    /// `[[system]].power_group` (issue #1396).
    ///
    /// The inverse of [`Self::systems_in_power_group`], and the reading a
    /// runtime gate wants: given the system a shot is authorised against, which
    /// group's level decides whether it has the power to fire at all. `None` for
    /// a system the hull put on no group — such a system is not on the reactor's
    /// books and no power order can reach it.
    pub fn power_group_for(&self, id: &SystemId) -> Option<&PowerGroupId> {
        self.system(id)
            .and_then(|system| system.power_group.as_ref())
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
        toml::from_str(toml_str).map_err(|e| ShipConfigError::ParseError(format!(
            "{e}\nContent requires explicit Console ownership: add [[console]] definitions and replace system.station with system.console. Use the pack's original game version if it has not been converted."
        )))?;
    validate(&config, registered_system_kinds)?;
    Ok(config)
}

pub fn validate(
    config: &ShipConfig,
    registered_system_kinds: &[&str],
) -> Result<(), ShipConfigError> {
    if !config.coordination_lag_secs.is_finite() || config.coordination_lag_secs < 0.0 {
        return Err(ShipConfigError::InvalidCoordinationLagSecs {
            value: config.coordination_lag_secs,
        });
    }
    // Empty stations is legitimate: NPC ships have no human consoles. Every
    // system on such a config must be `ai_only = true` (enforced below via the
    // OwnerlessSystemWithoutAiOnly check, since no station can own them).
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

    super::console_ownership::validate_consoles(config)?;

    // Stance catalogues and Command targets (issue #1107).
    for station in &config.stations {
        // A station's own stance catalogue: unique ids, exactly one of each
        // neutral fallback (or none at all — an undirected station authors no
        // catalogue), and neutral postures that agree with their kind.
        if !station.stances.is_empty() {
            let mut stance_ids = HashSet::new();
            let mut ai_engaged_count = 0usize;
            for stance in &station.stances {
                if !stance_ids.insert(stance.id.clone()) {
                    return Err(ShipConfigError::DuplicateStanceId {
                        station: station.id.clone(),
                        stance: stance.id.clone(),
                    });
                }
                match stance.kind {
                    StanceKind::NormalAlertNeutral if stance.high_alert => {
                        return Err(ShipConfigError::NeutralStancePostureMismatch {
                            station: station.id.clone(),
                            stance: stance.id.clone(),
                            kind: stance.kind,
                        });
                    }
                    StanceKind::HighAlertNeutral if !stance.high_alert => {
                        return Err(ShipConfigError::NeutralStancePostureMismatch {
                            station: station.id.clone(),
                            stance: stance.id.clone(),
                            kind: stance.kind,
                        });
                    }
                    _ => {}
                }
                // The AI Command high-alert pick (issue #1109): at most one per
                // catalogue, and only on a `standard` stance.
                if stance.ai_engaged {
                    ai_engaged_count += 1;
                    if stance.kind != StanceKind::Standard {
                        return Err(ShipConfigError::AiEngagedStanceNotStandard {
                            station: station.id.clone(),
                            stance: stance.id.clone(),
                        });
                    }
                }
            }
            if ai_engaged_count > 1 {
                return Err(ShipConfigError::MultipleAiEngagedStances {
                    station: station.id.clone(),
                });
            }
            for kind in [StanceKind::NormalAlertNeutral, StanceKind::HighAlertNeutral] {
                let found = station
                    .stances
                    .iter()
                    .filter(|stance| stance.kind == kind)
                    .count();
                if found != 1 {
                    return Err(ShipConfigError::StanceCatalogueNeutralCount {
                        station: station.id.clone(),
                        kind,
                        found,
                    });
                }
            }
        }
    }

    let power_group_ids: HashSet<PowerGroupId> = config.power_groups.keys().cloned().collect();
    for power_group_id in &power_group_ids {
        if power_group_id.0.trim().is_empty() {
            return Err(ShipConfigError::EmptyPowerGroupId);
        }
    }
    // The level trio has to describe a real range now that `min_level` is an
    // allocation clamp and not a drawing hint (issue #1395). Walked in sorted
    // id order rather than in `HashMap` order so a hull with two bad blocks
    // always names the same one.
    let mut power_group_levels: Vec<(&PowerGroupId, &PowerGroupConfig)> =
        config.power_groups.iter().collect();
    power_group_levels.sort_by(|a, b| a.0 .0.cmp(&b.0 .0));
    for (id, group) in power_group_levels {
        if group.min_level >= group.max_level {
            return Err(ShipConfigError::InvalidPowerGroupLevels {
                group: id.clone(),
                min_level: group.min_level,
                max_level: group.max_level,
            });
        }
        if group.default_level < group.min_level || group.default_level > group.max_level {
            return Err(ShipConfigError::PowerGroupDefaultOutOfRange {
                group: id.clone(),
                default_level: group.default_level,
                min_level: group.min_level,
                max_level: group.max_level,
            });
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
        if config.system_home_station(system).is_none()
            && !system.ai_only
            && !config
                .console_for_system(&system.id)
                .is_some_and(|console| console.human_seeking)
        {
            return Err(ShipConfigError::OwnerlessSystemWithoutAiOnly {
                system: system.id.clone(),
            });
        }
        if let Some(station) = config.system_home_station(system) {
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
        system_owner_by_id.insert(
            system.id.clone(),
            config.system_home_station(system).cloned(),
        );
    }

    for station in &config.stations {
        for rating in &station.ratings {
            for system_id in rating
                .automated_systems
                .iter()
                .chain(rating.detailed_systems.iter().flatten())
            {
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
                if rating.automated_systems.contains(system_id)
                    && rating
                        .detailed_systems
                        .as_ref()
                        .is_some_and(|ids| ids.contains(system_id))
                {
                    return Err(ShipConfigError::ConflictingRatingDepth {
                        station: station.id.clone(),
                        rating: rating.name.clone(),
                        system: system_id.clone(),
                    });
                }
            }
        }
    }

    for station in &config.stations {
        for rating in &station.ratings {
            if let Some(detailed) = &rating.detailed_systems {
                for system in config.systems_for_station(&station.id) {
                    // A simplified surface needs summary intent consumed by a
                    // scoped policy. Never silently turn an unsupported system
                    // into either direct control or unconfigurable automation.
                    if !detailed.contains(&system.id)
                        && !rating.automated_systems.contains(&system.id)
                        && system.kind != "repair"
                    {
                        return Err(ShipConfigError::UnsupportedSimplifiedSystem {
                            station: station.id.clone(),
                            rating: rating.name.clone(),
                            system: system.id.clone(),
                        });
                    }
                }
            }
        }
    }

    Ok(())
}

#[cfg(test)]
#[path = "config_tests.rs"]
mod tests;

pub use phoenix_sim_contracts::stance::*;
