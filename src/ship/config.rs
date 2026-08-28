use crate::core::messages::{PowerGroupId, StationId, SystemId, TutorialOverlayWire};
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
    crate::ship::system_registry::PHASER_BANK_KIND,
    crate::ship::system_registry::BLASTER_BANK_KIND,
    crate::ship::system_registry::TORPEDO_TUBE_KIND,
    crate::ship::system_registry::TORPEDO_MAGAZINE_KIND,
    crate::ship::system_registry::TACTICAL_RADAR_KIND,
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

    /// The single owning Station shared by every authored System of `kind`.
    ///
    /// Instance ids are author-defined and therefore cannot stand in for a
    /// capability kind. A missing kind, an ownerless instance, or instances
    /// split across Stations is ambiguous and returns `None` rather than
    /// widening the audience.
    pub fn station_for_system_kind(&self, kind: &str) -> Option<StationId> {
        let mut systems = self.systems.iter().filter(|system| system.kind == kind);
        let station = systems.next()?.station.clone()?;
        systems
            .all(|system| system.station.as_ref() == Some(&station))
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
            .find_map(|s| s.station.clone())
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
#[path = "config_tests.rs"]
mod tests;
