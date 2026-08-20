use crate::messages::{PowerGroupId, StationId, SystemId, TutorialOverlayWire};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ShipConfig {
    #[serde(default, rename = "station")]
    pub stations: Vec<StationConfig>,
    #[serde(default, rename = "system")]
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

/// System kinds that constitute "the weapons suite" for
/// [`ShipConfig::weapons_station`]. Every weapon system on a hull lives on one
/// station by construction, so the first match wins.
const WEAPONS_KINDS: &[&str] = &[
    crate::system_registry::PHASER_BANK_KIND,
    crate::system_registry::BLASTER_BANK_KIND,
    crate::system_registry::TORPEDO_TUBE_KIND,
    crate::system_registry::TORPEDO_MAGAZINE_KIND,
    crate::system_registry::TACTICAL_RADAR_KIND,
];

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StationConfig {
    pub id: StationId,
    pub name: String,
    pub description: String,
    pub rank: String,
    #[serde(default)]
    pub short_code: String,
    #[serde(default, rename = "rating")]
    pub ratings: Vec<StationRatingConfig>,
    /// Console root URL for this station (e.g. "gui/captain-console.html").
    /// When absent, the client falls back to a generic console or the
    /// station has no dedicated UI.
    #[serde(default)]
    pub console: Option<String>,
    /// Authored overview prose for this station's ship manual tab (issue #772).
    ///
    /// This is LITERAL authored English read from `[[station]] manual_overview`
    /// in the ship TOML — the same authored-content precedent as comms response
    /// text and `display_name`, NOT a `strings.csv` id and NOT emitted English
    /// in Rust. `#[serde(default)]` so hulls that omit it (and the
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
    /// Complete-station human seeking (issue #1097). The station keeps its
    /// identity, systems and console while this resolver chooses a directly
    /// held station on which to present it.
    #[serde(default)]
    pub human_seeking: bool,
    /// Compatible direct stations, in preference order, tried after this
    /// station's own active holder. This is deliberately a finite allow-list:
    /// exhaustion means ordinary AI even if another unrelated seat is held.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub host_order: Vec<StationId>,
    /// Rating used while this station is visiting another direct station.
    /// Scenario detail requirements may raise it, but never lower it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visiting_rating: Option<String>,
    /// Auxiliary stations are mounted and resolved like any other station but
    /// are not offered as a separately claimable lobby seat.
    #[serde(default)]
    pub auxiliary: bool,
    /// The proving Station this (Command) station directs (issue #1107).
    ///
    /// Authored on an auxiliary Command station, never hard-coded: it names the
    /// AI-controlled Station whose authored stance catalogue this Command
    /// surface lists and applies. `None` on every ordinary station.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command_target: Option<StationId>,
    /// This station's authored stance catalogue (issue #1107).
    ///
    /// Standard stances plus the two mandatory alert-neutral fallbacks
    /// (`normal_alert_neutral`, `high_alert_neutral`). A Command station reads
    /// its `command_target`'s catalogue; the target station authors it here.
    /// Empty on stations that are never directed.
    #[serde(default, rename = "stance", skip_serializing_if = "Vec::is_empty")]
    pub stances: Vec<StationStanceConfig>,
}

/// One authored stance in a station's catalogue (issue #1107).
///
/// A stance supplies a posture FACT and policy choices to the station's ordinary
/// AI hosts "in the same broad manner that Red Alert currently informs
/// behavior" — it never applies a hidden statistical bonus and never operates a
/// fine System directly. See `docs/gdd/mechanics/command-and-crew-control.md`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StationStanceConfig {
    /// Stable stance id, referenced by `SetStationStance` on the wire.
    pub id: String,
    /// `strings.csv` id for the stance's display label. Carried to the Command
    /// console, which resolves it through `gui/strings.js`; never emitted
    /// English. Empty falls back to the raw id on the console.
    #[serde(default)]
    pub label: String,
    /// Which of the three authored kinds this stance is.
    pub kind: StanceKind,
    /// The alert posture this stance seeds for the station's AI hosts: `true`
    /// behaves as "at high alert" (the migrated Red Alert branch fires), `false`
    /// as "stood down". Validated to agree with `kind` for the two neutral
    /// fallbacks; a `standard` stance authors it freely.
    #[serde(default)]
    pub high_alert: bool,
    /// Stance lifecycle: when `true` the stance persists behind a human handoff
    /// on the directed station; when `false` (the default) it resets to the
    /// appropriate alert-neutral stance. Neutral stances are their own reset
    /// target, so the flag is meaningful only on `standard` stances.
    #[serde(default)]
    pub persist_behind_human: bool,
    /// The posture an AI-operated Command seat adopts for this station when the
    /// ship is at high (red) alert (issue #1109).
    ///
    /// This is the authored answer to "what does an uncrewed Command choose?":
    /// exactly one `standard` stance per catalogue may set it, and an AI Command
    /// operator selects that stance (through the ordinary admitted-order path a
    /// human uses) while the ship is at Red Alert, tracking the alert-neutral
    /// otherwise. Never a hard-coded stance id in Rust — the choice is authored
    /// data on the catalogue itself. Meaningful only on `standard` stances (a
    /// neutral is already the low-alert tracking default); [`validate`] rejects
    /// it on a neutral or on more than one stance.
    #[serde(default)]
    pub ai_engaged: bool,
}

/// The three authored stance kinds (issue #1107). Every station catalogue that
/// exists at all authors exactly one `normal_alert_neutral` and one
/// `high_alert_neutral`; `standard` stances are the authored postures a Command
/// operator can additionally select.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StanceKind {
    /// An authored non-neutral posture (e.g. "weapons free", "escort").
    Standard,
    /// The fallback stance for the normal (not-red) alert level.
    NormalAlertNeutral,
    /// The fallback stance for the high (red) alert level.
    HighAlertNeutral,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StationRatingConfig {
    pub name: String,
    pub automated_systems: Vec<SystemId>,
    /// Per-rating AI tuning parameters (replaces assets/complexity/*.toml).
    /// Keys are AI rule names (e.g. "torpedo_auto_fire", "frequency_match");
    /// values are TOML tables with rule-specific parameters.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ai_tuning: Option<toml::Value>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SystemInstanceConfig {
    pub id: SystemId,
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub station: Option<StationId>,
    #[serde(default)]
    pub ai_only: bool,
    /// Marks a system as human-seeking (pasm decision
    /// `console-complexity-human-seeking-systems`): comms and navigation
    /// always try to be under human control, walking the ship's authored
    /// station order — owner first — until they find a human-held station.
    /// "The seek order only ever chooses among human-held stations: the
    /// mechanism prefers any human over the AI, it never forces a human." With
    /// no human anywhere in the order the system falls back to AI control
    /// exactly as today. See [`crate::ship::coordination::seek_human_host`]
    /// for the pure resolution this flag gates.
    #[serde(default)]
    pub human_seeking: bool,
    /// Optional authored walk for this system's human seek, overriding the
    /// order derived from `station` + the authored `[[station]]` list.
    ///
    /// **Absent is the default and means exactly today's behaviour**: the
    /// system's own `station` first, then the remaining stations in authored
    /// order. Nothing about a hull that authors no `seek_order` changes — the
    /// field is `skip_serializing_if` empty, so such a config also round-trips
    /// byte-for-byte.
    ///
    /// **Present, the list IS the walk, literally and completely.** It is a
    /// PERMUTATION of the hull's stations — every station exactly once, the
    /// system's own `station` first — enforced by [`validate`]. Three reasons
    /// the contract is a permutation and not a prefix or an allow-list:
    ///
    /// * The field is an *order*, so it reorders; it does not filter. A list
    ///   that could omit a station would need a second, invisible rule about
    ///   what happens to the omitted ones, and the TOML would no longer say
    ///   what the seek does.
    /// * Exhaustiveness preserves the decision's own promise — "the mechanism
    ///   prefers any human over the AI". An author who omitted a station could
    ///   drop a system to AI while a human sat at the unlisted console, which
    ///   is the one outcome `human_seeking` exists to avoid.
    /// * Owner-first is not a courtesy, it is the rule that keeps a hull's own
    ///   officer at their own console (see
    ///   [`crate::ship::coordination::seek_human_host`]). Requiring the owner
    ///   at the head, rather than silently prepending it, means the authored
    ///   list can be read top-to-bottom as the literal walk and cannot quietly
    ///   author the invariant away.
    ///
    /// A hull that gains a station therefore fails the load loudly until every
    /// `seek_order` names it — the alternative being a new console the seek
    /// never visits, with no symptom but a system stuck on AI.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub seek_order: Vec<StationId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub power_group: Option<PowerGroupId>,
    /// Optional rig-marker name for this system instance.
    ///
    /// **Declared but unread**: no runtime path resolves this against a model
    /// rig, so `crate::marker_validate` deliberately excludes it from the
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
/// `min_level` — and the floor the allocation API clamps every group to, so it
/// is also the lowest level ANY operator can command a group to.
///
/// `pub const` for the same reason as [`default_max_power_level`]: issue #959's
/// budget planner needs "the lowest level this group could be commanded to"
/// when it works out how much of the reactor budget is discretionary, and
/// restating `1` there would let the two drift.
pub const fn default_min_power_level() -> u8 {
    1
}

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
            .filter(move |system| system.station.as_ref() == Some(id))
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
            .find_map(|s| s.station.clone())
            .or_else(|| {
                // Legacy/test ships declare a `tactical` station but no fine
                // weapon systems. Preserve the pre-lookup behaviour for them.
                let tactical = StationId(crate::system_registry::TACTICAL_STATION_ID.into());
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
            .find(|s| s.kind == crate::system_registry::SENSORS_KIND)
            .and_then(|s| s.station.clone())
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
        if !station.human_seeking
            && (!station.host_order.is_empty() || station.visiting_rating.is_some())
        {
            return Err(ShipConfigError::HostOrderWithoutHumanSeeking {
                station: station.id.clone(),
            });
        }
        if station.human_seeking {
            let Some(visiting_rating) = station.visiting_rating.as_ref() else {
                return Err(ShipConfigError::MissingVisitingRating {
                    station: station.id.clone(),
                });
            };
            if !station
                .ratings
                .iter()
                .any(|rating| &rating.name == visiting_rating)
            {
                return Err(ShipConfigError::UnknownVisitingRating {
                    station: station.id.clone(),
                    rating: visiting_rating.clone(),
                });
            }
        }
    }

    for station in &config.stations {
        let mut seen = HashSet::new();
        for host in &station.host_order {
            let Some(_host_station) = config
                .stations
                .iter()
                .find(|candidate| &candidate.id == host)
            else {
                return Err(ShipConfigError::HostOrderUnknownStation {
                    station: station.id.clone(),
                    host: host.clone(),
                });
            };
            if !seen.insert(host) {
                return Err(ShipConfigError::HostOrderDuplicateStation {
                    station: station.id.clone(),
                    host: host.clone(),
                });
            }
            // A human-seeking Station is a valid compatible host while it is
            // actively directly held. Runtime placement tests that state,
            // rather than rejecting the Station's authored type here; a
            // visiting Station is never recursively eligible.
        }
    }

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

        // A Command target must name a real station that authors a catalogue.
        if let Some(target) = &station.command_target {
            let Some(target_station) = config.station(target) else {
                return Err(ShipConfigError::CommandTargetUnknownStation {
                    station: station.id.clone(),
                    target: target.clone(),
                });
            };
            if target_station.stances.is_empty() {
                return Err(ShipConfigError::CommandTargetHasNoStances {
                    station: station.id.clone(),
                    target: target.clone(),
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
        validate_seek_order(system, &config.stations)?;
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

/// Enforce the `seek_order` contract: authored only on a seeking system, and
/// then a permutation of the hull's stations headed by the system's own.
///
/// The whole contract is checked here rather than folded into the system loop
/// above so the rules read as one paragraph — the field's doc comment argues
/// for each of them, and a reader who wants to know what a `seek_order` may
/// say has exactly one function to read.
fn validate_seek_order(
    system: &SystemInstanceConfig,
    stations: &[StationConfig],
) -> Result<(), ShipConfigError> {
    if system.seek_order.is_empty() {
        return Ok(());
    }
    if !system.human_seeking {
        return Err(ShipConfigError::SeekOrderWithoutHumanSeeking {
            system: system.id.clone(),
        });
    }

    let hull: HashSet<&StationId> = stations.iter().map(|s| &s.id).collect();
    let mut seen: HashSet<&StationId> = HashSet::new();
    for station in &system.seek_order {
        if !hull.contains(station) {
            return Err(ShipConfigError::SeekOrderUnknownStation {
                system: system.id.clone(),
                station: station.clone(),
            });
        }
        if !seen.insert(station) {
            return Err(ShipConfigError::SeekOrderDuplicateStation {
                system: system.id.clone(),
                station: station.clone(),
            });
        }
    }
    // Reported in AUTHORED station order, so a hull that grew two stations
    // names the first of them rather than a hash-order pick.
    if let Some(missing) = stations.iter().find(|s| !seen.contains(&s.id)) {
        return Err(ShipConfigError::SeekOrderMissingStation {
            system: system.id.clone(),
            station: missing.id.clone(),
        });
    }
    // An ownerless system has no owner to lead with; every other rule still
    // applies. (`validate` has already refused an ownerless system that is not
    // `ai_only`, so this arm is only reachable for a config that asks the AI to
    // run a seeking system — contradictory, but not this check's business.)
    if let Some(owner) = &system.station {
        if system.seek_order.first() != Some(owner) {
            return Err(ShipConfigError::SeekOrderOwnerNotFirst {
                system: system.id.clone(),
                owner: owner.clone(),
                first: system.seek_order.first().cloned(),
            });
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
        "sensors",
        // The seek-order fixtures below author a real seeking system, and on
        // every shipped hull that is `comms`.
        "comms",
    ];

    fn valid_toml() -> &'static str {
        r#"
[[station]]
id = "captain"
name = "Captain"
description = "Command the bridge."
rank = "Cpt."
short_code = "CPT"
console = "gui/captain-console.html"
manual_overview = "You command the bridge and set the ship's posture."

[[station.rating]]
name = "Assisted"
automated_systems = ["red-alert", "viewscreen"]

[[station.rating]]
name = "Manual"
automated_systems = []

[[station]]
id = "tactical"
name = "Tactical"
description = "Weapons and threat response."
rank = "Ltn."
short_code = "TAC"
console = "gui/tactical-console.html"

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
station = "captain"
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
    fn station_config_parses_manual_overview_field() {
        let config = parse_ok(valid_toml());

        let captain = config.station(&StationId("captain".into())).unwrap();
        assert_eq!(
            captain.manual_overview.as_deref(),
            Some("You command the bridge and set the ship's posture."),
        );
        // Absent on stations that authored none (issue #772 default).
        let tactical = config.station(&StationId("tactical".into())).unwrap();
        assert_eq!(tactical.manual_overview, None);
    }

    #[test]
    fn station_config_manual_overview_survives_round_trip() {
        let config = parse_ok(valid_toml());
        let encoded = toml::to_string(&config).expect("ship config should serialize");
        let decoded = parse_ok(&encoded);

        assert_eq!(
            decoded
                .station(&StationId("captain".into()))
                .and_then(|s| s.manual_overview.clone()),
            Some("You command the bridge and set the ship's posture.".to_string()),
        );
    }

    // ── Contextual tutorial overlays (issue #916) ─────────────────────────

    /// A ship config authoring `[[station.tutorial]]` blocks on one station:
    /// one per shipped trigger kind, exercising every optional field.
    fn tutorial_toml() -> &'static str {
        r#"
[[station]]
id = "helm"
name = "Helm"
description = "Fly the ship."
rank = "Ltn."

[[station.rating]]
name = "Std"
automated_systems = []

[[station.tutorial]]
id = "helm-welcome"
title = "entity.test.station.helm.tutorial.welcome.title"
text = "entity.test.station.helm.tutorial.welcome.text"
anchor = "helm-radar"
trigger = { kind = "first_visit" }

[[station.tutorial]]
id = "helm-joystick"
title = "entity.test.station.helm.tutorial.joystick.title"
text = "entity.test.station.helm.tutorial.joystick.text"
trigger = { kind = "control_unused", control = "set_helm" }

[[station.tutorial]]
id = "helm-boost"
priority = 10
title = "entity.test.station.helm.tutorial.boost.title"
text = "entity.test.station.helm.tutorial.boost.text"
trigger = { kind = "state", path = "boost_enabled", op = "truthy", control = "set_boost" }

[[station]]
id = "captain"
name = "Captain"
description = "Command."
rank = "Cpt."

[[station.rating]]
name = "Std"
automated_systems = []

[[system]]
id = "helm"
kind = "helm"
station = "helm"

[[system]]
id = "red-alert"
kind = "red_alert"
station = "captain"
"#
    }

    #[test]
    fn station_config_parses_tutorial_overlay_blocks() {
        let config = parse_ok(tutorial_toml());
        let helm = config.station(&StationId("helm".into())).unwrap();
        assert_eq!(helm.tutorials.len(), 3);

        let welcome = &helm.tutorials[0];
        assert_eq!(welcome.id, "helm-welcome");
        assert_eq!(welcome.trigger.kind, "first_visit");
        assert_eq!(welcome.anchor.as_deref(), Some("helm-radar"));
        assert_eq!(welcome.priority, 0, "priority defaults to 0");
        assert_eq!(
            welcome.title,
            "entity.test.station.helm.tutorial.welcome.title"
        );

        let joystick = &helm.tutorials[1];
        assert_eq!(joystick.trigger.kind, "control_unused");
        assert_eq!(joystick.trigger.control.as_deref(), Some("set_helm"));
        assert_eq!(joystick.anchor, None);

        let boost = &helm.tutorials[2];
        assert_eq!(boost.priority, 10);
        assert_eq!(boost.trigger.kind, "state");
        assert_eq!(boost.trigger.path.as_deref(), Some("boost_enabled"));
        assert_eq!(boost.trigger.op.as_deref(), Some("truthy"));
        assert_eq!(boost.trigger.control.as_deref(), Some("set_boost"));

        // A station that authors none keeps an empty list (serde default).
        let captain = config.station(&StationId("captain".into())).unwrap();
        assert!(captain.tutorials.is_empty());
    }

    #[test]
    fn station_config_tutorials_survive_round_trip() {
        let config = parse_ok(tutorial_toml());
        let encoded = toml::to_string(&config).expect("ship config should serialize");
        let decoded = parse_ok(&encoded);
        assert_eq!(decoded, config);
    }

    // ── Human-seeking systems (issue #984) ────────────────────────────────

    #[test]
    fn system_config_parses_human_seeking_flag() {
        let config = parse_ok(valid_toml());

        // Absent on systems that authored none (serde default).
        let red_alert = config.system(&SystemId("red-alert".into())).unwrap();
        assert!(!red_alert.human_seeking);
    }

    #[test]
    fn system_config_human_seeking_survives_round_trip() {
        let toml = r#"
[[station]]
id = "captain"
name = "Captain"
description = "Command the bridge."
rank = "Cpt."

[[station.rating]]
name = "Std"
automated_systems = []

[[system]]
id = "comms"
kind = "sensors"
station = "captain"
human_seeking = true
"#;
        let config = parse_ok(toml);
        assert!(
            config
                .system(&SystemId("comms".into()))
                .unwrap()
                .human_seeking
        );

        let encoded = toml::to_string(&config).expect("ship config should serialize");
        let decoded = parse_ok(&encoded);
        assert_eq!(decoded, config);
    }

    // ── Authored seek order (issue #984) ──────────────────────────────────

    /// Two stations and one seeking system on the first of them, so a
    /// `seek_order` is a two-name permutation and every rule has room to fail.
    fn seek_order_toml(system_block: &str) -> String {
        format!(
            r#"
[[station]]
id = "tactical"
name = "Tactical"
description = "Fight the ship."
rank = "Lt. Cmdr."

[[station.rating]]
name = "Std"
automated_systems = []

[[station]]
id = "engineering"
name = "Engineering"
description = "Keep it running."
rank = "Ltn."

[[station.rating]]
name = "Std"
automated_systems = []

{system_block}
"#
        )
    }

    #[test]
    fn seek_order_is_absent_by_default_and_round_trips_when_authored() {
        // A hull that authors none keeps an empty list AND serialises without
        // the key, so an untouched hull's TOML is byte-for-byte what it was.
        let plain = parse_ok(&seek_order_toml(
            "[[system]]\nid = \"comms\"\nkind = \"comms\"\nstation = \"tactical\"\nhuman_seeking = true\n",
        ));
        let comms = plain.system(&SystemId("comms".into())).unwrap();
        assert!(comms.seek_order.is_empty());
        let encoded = toml::to_string(&plain).expect("ship config should serialize");
        assert!(
            !encoded.contains("seek_order"),
            "an unauthored seek_order must not appear on the way out:\n{encoded}"
        );

        let authored = parse_ok(&seek_order_toml(
            "[[system]]\nid = \"comms\"\nkind = \"comms\"\nstation = \"tactical\"\nhuman_seeking = true\nseek_order = [\"tactical\", \"engineering\"]\n",
        ));
        assert_eq!(
            authored
                .system(&SystemId("comms".into()))
                .unwrap()
                .seek_order,
            vec![
                StationId("tactical".into()),
                StationId("engineering".into())
            ]
        );
        let encoded = toml::to_string(&authored).expect("ship config should serialize");
        assert_eq!(parse_ok(&encoded), authored);
    }

    #[test]
    fn seek_order_rejects_a_station_this_hull_does_not_have() {
        let err = ShipConfig::from_toml(
            &seek_order_toml(
                "[[system]]\nid = \"comms\"\nkind = \"comms\"\nstation = \"tactical\"\nhuman_seeking = true\nseek_order = [\"tactical\", \"engineering\", \"science\"]\n",
            ),
            KINDS,
        );
        assert!(matches!(
            err,
            Err(ShipConfigError::SeekOrderUnknownStation { ref station, .. })
                if station.0 == "science"
        ));
    }

    #[test]
    fn seek_order_rejects_the_same_station_twice() {
        let err = ShipConfig::from_toml(
            &seek_order_toml(
                "[[system]]\nid = \"comms\"\nkind = \"comms\"\nstation = \"tactical\"\nhuman_seeking = true\nseek_order = [\"tactical\", \"tactical\", \"engineering\"]\n",
            ),
            KINDS,
        );
        assert!(matches!(
            err,
            Err(ShipConfigError::SeekOrderDuplicateStation { .. })
        ));
    }

    /// The list is the WHOLE walk, so a station left off is a console the seek
    /// could never reach — refused at load rather than discovered by a crew.
    #[test]
    fn seek_order_rejects_an_incomplete_walk() {
        let err = ShipConfig::from_toml(
            &seek_order_toml(
                "[[system]]\nid = \"comms\"\nkind = \"comms\"\nstation = \"tactical\"\nhuman_seeking = true\nseek_order = [\"tactical\"]\n",
            ),
            KINDS,
        );
        assert!(matches!(
            err,
            Err(ShipConfigError::SeekOrderMissingStation { ref station, .. })
                if station.0 == "engineering"
        ));
    }

    /// Owner-first is the rule that keeps a hull's own officer at their own
    /// console. A complete permutation that starts anywhere else is still wrong.
    #[test]
    fn seek_order_rejects_an_order_that_does_not_start_at_the_owner() {
        let err = ShipConfig::from_toml(
            &seek_order_toml(
                "[[system]]\nid = \"comms\"\nkind = \"comms\"\nstation = \"tactical\"\nhuman_seeking = true\nseek_order = [\"engineering\", \"tactical\"]\n",
            ),
            KINDS,
        );
        assert!(matches!(
            err,
            Err(ShipConfigError::SeekOrderOwnerNotFirst { ref owner, ref first, .. })
                if owner.0 == "tactical" && first.as_ref().map(|s| s.0.as_str()) == Some("engineering")
        ));
    }

    #[test]
    fn seek_order_rejects_a_system_that_does_not_seek() {
        let err = ShipConfig::from_toml(
            &seek_order_toml(
                "[[system]]\nid = \"comms\"\nkind = \"comms\"\nstation = \"tactical\"\nseek_order = [\"tactical\", \"engineering\"]\n",
            ),
            KINDS,
        );
        assert!(matches!(
            err,
            Err(ShipConfigError::SeekOrderWithoutHumanSeeking { .. })
        ));
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
        // Build a config where a system has no station and no ai_only flag.
        // This is done by appending a new orphan system after the valid TOML.
        let toml = format!(
            "{}\n[[system]]\nid = \"orphan\"\nkind = \"viewscreen\"\npower_group = \"ops\"\n",
            valid_toml()
        );

        assert_eq!(
            parse_and_validate(&toml, KINDS),
            Err(ShipConfigError::OwnerlessSystemWithoutAiOnly {
                system: SystemId("orphan".into())
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

    #[test]
    fn station_config_parses_console_field() {
        let config = parse_ok(valid_toml());

        let captain = config.station(&StationId("captain".into())).unwrap();
        assert_eq!(captain.console.as_deref(), Some("gui/captain-console.html"));

        let tactical = config.station(&StationId("tactical".into())).unwrap();
        assert_eq!(
            tactical.console.as_deref(),
            Some("gui/tactical-console.html")
        );
    }

    #[test]
    fn station_config_console_defaults_to_none_when_absent() {
        let toml = valid_toml().replace("console = \"gui/captain-console.html\"\n", "");
        let config = parse_ok(&toml);

        let captain = config.station(&StationId("captain".into())).unwrap();
        assert_eq!(captain.console, None);
    }

    #[test]
    fn station_system_and_console_resolution_for_battleship_style_config() {
        let toml = r#"
[[station]]
id = "captain"
name = "Captain"
description = "Command."
rank = "Cpt."
short_code = "CPT"
console = "gui/captain-console.html"

[[station.rating]]
name = "Std"
automated_systems = []

[[station]]
id = "helm"
name = "Helm"
description = "Pilot."
rank = "Ltn."
short_code = "HLM"
console = "gui/helm-console.html"

[[station.rating]]
name = "Std"
automated_systems = []

[[station]]
id = "tactical"
name = "Tactical"
description = "Weapons."
rank = "Ltn."
short_code = "TAC"
console = "gui/tactical-console.html"

[[station.rating]]
name = "Std"
automated_systems = []

[[station]]
id = "repair"
name = "Repair"
description = "Repair."
rank = "Ltn."
short_code = "ENG"
console = "gui/repair-console.html"

[[station.rating]]
name = "Std"
automated_systems = []

[power_groups.ops]
label = "Operations"
default_level = 2
min_level = 1
max_level = 4

[power_groups.helm]
label = "Propulsion"
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
id = "helm"
kind = "helm"
station = "helm"
power_group = "helm"

[[system]]
id = "tactical"
kind = "phaser_bank"
station = "tactical"
power_group = "weapons"

[[system]]
id = "viewscreen"
kind = "viewscreen"
station = "captain"
power_group = "ops"
"#;
        let config = parse_ok(toml);

        assert_eq!(config.stations.len(), 4);
        assert_eq!(config.systems.len(), 4);

        for station in &config.stations {
            match station.id.0.as_str() {
                "captain" => {
                    assert_eq!(station.console.as_deref(), Some("gui/captain-console.html"));
                    let systems: Vec<_> = config.systems_for_station(&station.id).collect();
                    assert_eq!(systems.len(), 2);
                }
                "helm" => {
                    assert_eq!(station.console.as_deref(), Some("gui/helm-console.html"));
                    let systems: Vec<_> = config.systems_for_station(&station.id).collect();
                    assert_eq!(systems.len(), 1);
                }
                "tactical" => {
                    assert_eq!(
                        station.console.as_deref(),
                        Some("gui/tactical-console.html")
                    );
                    let systems: Vec<_> = config.systems_for_station(&station.id).collect();
                    assert_eq!(systems.len(), 1);
                }
                "repair" => {
                    assert_eq!(station.console.as_deref(), Some("gui/repair-console.html"));
                    let systems: Vec<_> = config.systems_for_station(&station.id).collect();
                    assert_eq!(systems.len(), 0);
                }
                other => panic!("unexpected station id: {other}"),
            }
        }

        let captain_station = config.station(&StationId("captain".into())).unwrap();
        let captain_system_ids: Vec<&str> = config
            .systems_for_station(&captain_station.id)
            .map(|s| s.id.0.as_str())
            .collect();
        assert_eq!(captain_system_ids, vec!["red-alert", "viewscreen"]);
    }

    // ── weapons_station ──────────────────────────────────────────────────

    /// The crewed hulls put their fine weapon systems on a station named
    /// "tactical"; resolving from config must not change that.
    #[test]
    fn weapons_station_resolves_tactical_for_crewed_hull_shape() {
        let toml = r#"
[[station]]
id = "tactical"
name = "Tactical"
description = "Weapons."
rank = "Ltn."

[[system]]
id = "phaser-fore"
kind = "phaser_bank"
station = "tactical"
"#;
        let config = ShipConfig::from_toml(toml, KINDS).unwrap();
        assert_eq!(config.weapons_station(), Some(StationId("tactical".into())));
    }

    /// The Courier puts its blaster on the single "pilot" station. This is the
    /// case the whole lookup exists for.
    #[test]
    fn weapons_station_resolves_pilot_when_weapons_live_on_pilot() {
        let toml = r#"
[[station]]
id = "pilot"
name = "Pilot"
description = "Everything."
rank = "Ltn."

[[system]]
id = "blaster-fore"
kind = "blaster_bank"
station = "pilot"
"#;
        let config = ShipConfig::from_toml(toml, &["blaster_bank"]).unwrap();
        assert_eq!(config.weapons_station(), Some(StationId("pilot".into())));
    }

    /// NPCs declare no `station` on any system — no human owns their guns.
    #[test]
    fn weapons_station_is_none_for_npc_shape() {
        let toml = r#"
[[system]]
id = "phaser-fore"
kind = "phaser_bank"
ai_only = true
"#;
        let config = ShipConfig::from_toml(toml, KINDS).unwrap();
        assert_eq!(config.weapons_station(), None);
    }

    /// Legacy/test ships declare a `tactical` station but no fine weapon
    /// systems. They must keep resolving to it, or the pre-lookup gates change
    /// behaviour.
    #[test]
    fn weapons_station_falls_back_to_tactical_station_without_fine_systems() {
        let toml = r#"
[[station]]
id = "tactical"
name = "Tactical"
description = "Weapons."
rank = "Ltn."

[[system]]
id = "helm"
kind = "helm"
station = "tactical"
"#;
        let config = ShipConfig::from_toml(toml, KINDS).unwrap();
        assert_eq!(config.weapons_station(), Some(StationId("tactical".into())));
    }

    // ── sensors_station ──────────────────────────────────────────────────

    #[test]
    fn sensors_station_resolves_declared_station() {
        let toml = r#"
[[station]]
id = "sensors"
name = "Sensors"
description = "Long-range sensors."
rank = "Ens."

[[system]]
id = "sensors"
kind = "sensors"
station = "sensors"
"#;
        let config = ShipConfig::from_toml(toml, KINDS).unwrap();
        assert_eq!(config.sensors_station(), Some(StationId("sensors".into())));
    }

    /// NPCs declare no `station` on their sensors system — no human owns it.
    #[test]
    fn sensors_station_is_none_for_npc_shape() {
        let toml = r#"
[[system]]
id = "sensors"
kind = "sensors"
ai_only = true
"#;
        let config = ShipConfig::from_toml(toml, KINDS).unwrap();
        assert_eq!(config.sensors_station(), None);
    }

    #[test]
    fn sensors_station_is_none_when_no_sensors_system_declared() {
        let toml = r#"
[[system]]
id = "helm"
kind = "helm"
ai_only = true
"#;
        let config = ShipConfig::from_toml(toml, KINDS).unwrap();
        assert_eq!(config.sensors_station(), None);
    }

    // ── Command stances (issue #1107) ─────────────────────────────────────────

    const STANCE_KINDS: &[&str] = &["red_alert", "sensors", "command"];

    /// A captain, a proving station (sensors) authoring a full stance catalogue,
    /// and an auxiliary Command station directing it, hosted by the captain.
    fn command_toml(catalogue: &str, command_extra: &str) -> String {
        format!(
            r#"
[[station]]
id = "captain"
name = "Captain"
description = "Command the bridge."
rank = "Cpt."

[[station.rating]]
name = "Std"
automated_systems = []

[[station]]
id = "proving"
name = "Proving"
description = "The AI-controlled proving station."
rank = "Ltn."
{catalogue}

[[station.rating]]
name = "Std"
automated_systems = []

[[station]]
id = "command"
name = "Command"
description = "Direct an AI station."
rank = "Cpt."
console = "gui/command-console.html"
auxiliary = true
human_seeking = true
host_order = ["captain"]
visiting_rating = "Std"
command_target = "proving"
{command_extra}

[[station.rating]]
name = "Std"
automated_systems = []

[[system]]
id = "red-alert"
kind = "red_alert"
station = "captain"

[[system]]
id = "sensors"
kind = "sensors"
station = "proving"

[[system]]
id = "command"
kind = "command"
station = "command"
"#
        )
    }

    const FULL_CATALOGUE: &str = r#"
[[station.stance]]
id = "proving-standard"
kind = "standard"
high_alert = true
persist_behind_human = true

[[station.stance]]
id = "proving-normal"
kind = "normal_alert_neutral"

[[station.stance]]
id = "proving-high"
kind = "high_alert_neutral"
high_alert = true
"#;

    fn parse_command(catalogue: &str, extra: &str) -> Result<ShipConfig, ShipConfigError> {
        ShipConfig::from_toml(&command_toml(catalogue, extra), STANCE_KINDS)
    }

    #[test]
    fn command_station_and_stance_catalogue_parse_and_round_trip() {
        let config = parse_command(FULL_CATALOGUE, "").expect("command hull parses");
        let command = config.station(&StationId("command".into())).unwrap();
        assert!(command.auxiliary);
        assert!(command.human_seeking);
        assert_eq!(command.host_order, vec![StationId("captain".into())]);
        assert_eq!(command.command_target, Some(StationId("proving".into())));

        let proving = config.station(&StationId("proving".into())).unwrap();
        assert_eq!(proving.stances.len(), 3);
        assert_eq!(proving.stances[0].id, "proving-standard");
        assert_eq!(proving.stances[0].kind, StanceKind::Standard);
        assert!(proving.stances[0].high_alert);
        // The authored persistence flag (issue #1108 AC1) parses and round-trips;
        // an unauthored stance defaults to non-persistent.
        assert!(proving.stances[0].persist_behind_human);
        assert!(!proving.stances[1].persist_behind_human);
        assert_eq!(proving.stances[1].kind, StanceKind::NormalAlertNeutral);
        assert!(!proving.stances[1].high_alert);
        assert_eq!(proving.stances[2].kind, StanceKind::HighAlertNeutral);

        // A hull that authors no catalogue keeps an empty list and serialises
        // without the key, so untouched hulls round-trip byte-for-byte.
        let encoded = toml::to_string(&config).expect("serialise");
        let decoded = ShipConfig::from_toml(&encoded, STANCE_KINDS).unwrap();
        assert_eq!(decoded, config);
        let captain_encoded =
            toml::to_string(config.station(&StationId("captain".into())).unwrap()).unwrap();
        assert!(!captain_encoded.contains("stance"));
        assert!(!captain_encoded.contains("command_target"));
    }

    #[test]
    fn command_target_must_name_a_real_station() {
        let toml = command_toml(FULL_CATALOGUE, "")
            .replace("command_target = \"proving\"", "command_target = \"ghost\"");
        assert!(matches!(
            ShipConfig::from_toml(&toml, STANCE_KINDS),
            Err(ShipConfigError::CommandTargetUnknownStation { ref target, .. })
                if target.0 == "ghost"
        ));
    }

    #[test]
    fn command_target_must_author_a_catalogue() {
        // Point Command at the captain, which has no stances.
        let toml = command_toml(FULL_CATALOGUE, "").replace(
            "command_target = \"proving\"",
            "command_target = \"captain\"",
        );
        assert!(matches!(
            ShipConfig::from_toml(&toml, STANCE_KINDS),
            Err(ShipConfigError::CommandTargetHasNoStances { ref target, .. })
                if target.0 == "captain"
        ));
    }

    #[test]
    fn catalogue_must_have_exactly_one_of_each_neutral() {
        // Drop the high-alert neutral.
        let missing = r#"
[[station.stance]]
id = "proving-normal"
kind = "normal_alert_neutral"
"#;
        assert!(matches!(
            parse_command(missing, ""),
            Err(ShipConfigError::StanceCatalogueNeutralCount {
                kind: StanceKind::HighAlertNeutral,
                found: 0,
                ..
            })
        ));
    }

    #[test]
    fn neutral_stance_posture_must_agree_with_kind() {
        // normal_alert_neutral authored as high_alert = true.
        let bad = r#"
[[station.stance]]
id = "proving-normal"
kind = "normal_alert_neutral"
high_alert = true

[[station.stance]]
id = "proving-high"
kind = "high_alert_neutral"
high_alert = true
"#;
        assert!(matches!(
            parse_command(bad, ""),
            Err(ShipConfigError::NeutralStancePostureMismatch {
                kind: StanceKind::NormalAlertNeutral,
                ..
            })
        ));
    }

    #[test]
    fn catalogue_rejects_duplicate_stance_ids() {
        let dupe = r#"
[[station.stance]]
id = "proving-normal"
kind = "normal_alert_neutral"

[[station.stance]]
id = "proving-normal"
kind = "high_alert_neutral"
high_alert = true
"#;
        assert!(matches!(
            parse_command(dupe, ""),
            Err(ShipConfigError::DuplicateStanceId { ref stance, .. })
                if stance == "proving-normal"
        ));
    }

    #[test]
    fn the_ai_engaged_flag_parses_and_round_trips() {
        // Issue #1109: a single standard stance may carry the AI Command
        // high-alert pick, and it survives a serialise/parse round-trip.
        let config = parse_command(FULL_CATALOGUE_AI_ENGAGED, "").expect("hull parses");
        let proving = config.station(&StationId("proving".into())).unwrap();
        assert!(proving.stances[0].ai_engaged);
        assert!(!proving.stances[1].ai_engaged);
        let encoded = toml::to_string(&config).expect("serialise");
        let decoded = ShipConfig::from_toml(&encoded, STANCE_KINDS).unwrap();
        assert_eq!(decoded, config);
    }

    #[test]
    fn catalogue_rejects_more_than_one_ai_engaged_stance() {
        // Issue #1109: the AI's high-alert choice is a single authored posture.
        let two = r#"
[[station.stance]]
id = "proving-a"
kind = "standard"
high_alert = true
ai_engaged = true

[[station.stance]]
id = "proving-b"
kind = "standard"
high_alert = true
ai_engaged = true

[[station.stance]]
id = "proving-normal"
kind = "normal_alert_neutral"

[[station.stance]]
id = "proving-high"
kind = "high_alert_neutral"
high_alert = true
"#;
        assert!(matches!(
            parse_command(two, ""),
            Err(ShipConfigError::MultipleAiEngagedStances { .. })
        ));
    }

    #[test]
    fn catalogue_rejects_an_ai_engaged_neutral() {
        // Issue #1109: a neutral is already the tracking default, so it may not
        // be flagged as the engaged posture.
        let bad = r#"
[[station.stance]]
id = "proving-normal"
kind = "normal_alert_neutral"
ai_engaged = true

[[station.stance]]
id = "proving-high"
kind = "high_alert_neutral"
high_alert = true
"#;
        assert!(matches!(
            parse_command(bad, ""),
            Err(ShipConfigError::AiEngagedStanceNotStandard { ref stance, .. })
                if stance == "proving-normal"
        ));
    }

    const FULL_CATALOGUE_AI_ENGAGED: &str = r#"
[[station.stance]]
id = "proving-standard"
kind = "standard"
high_alert = true
ai_engaged = true

[[station.stance]]
id = "proving-normal"
kind = "normal_alert_neutral"

[[station.stance]]
id = "proving-high"
kind = "high_alert_neutral"
high_alert = true
"#;
}
