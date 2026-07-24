//! Read-only ship manual content builder (issue #772).
//!
//! Pure and Bevy-free: builds a ship-specific player manual from the
//! station/system topology ([`ShipConfig`]), authored per-station overview
//! prose, and per-system generated sections produced by kind-keyed providers.
//! A sibling Bevy adapter (`lobby/server.rs`) feeds this at the Welcome seam and
//! replicates the result read-only; nothing here touches Bevy.
//!
//! ## Player-visible English policy (AGENTS.md #11)
//!
//! Generated section content carries **no composed English**. Providers emit
//! STRUCTURED data only: stable machine codes (the section `kind`, each metric
//! `code`) plus numeric values and system/rating identifiers. The client maps
//! those codes to `assets/strings/strings.csv` label ids and renders them via
//! `t()`, interpolating the numbers. None of these codes is itself a strings id
//! (ids are dotted, e.g. `manual.shields.max_hp`), so the client's
//! wire-boundary localiser (`localiseTree`) leaves the whole structure
//! untouched and the panel resolves the labels itself.
//!
//! The single exception is [`StationManualWire::overview`]: literal authored
//! prose read from `[[station]] manual_overview` in the ship TOML. This is the
//! same authored-content precedent as comms response text and `display_name` —
//! data read from TOML, NOT emitted English in Rust and NOT a string id.

use crate::messages::{StationId, SystemId};
use crate::ship::config::{ShipConfig, SystemInstanceConfig};
use crate::ship::rating::{available_ratings_for_station, resolve_automated_systems};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ── Wire types ──────────────────────────────────────────────────────────────

/// Structured, generated documentation for one configured system.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SystemManualSection {
    /// Stable section-kind code — the system kind (e.g. `"shields"`). NOT a
    /// strings id: the client maps it to a heading id (`manual.section.<kind>`).
    pub kind: String,
    /// Numeric metrics, each a machine `code` + value. The client maps
    /// `(kind, code)` to a label id (`manual.<kind>.<code>`) and interpolates
    /// the value via `t()`.
    #[serde(default)]
    pub metrics: Vec<SystemManualMetric>,
    /// Rating→AI automation for the owning station: for each authored rating
    /// (plus the implicit `Backfill`), which owned systems become AI-operated.
    /// Derived from [`resolve_automated_systems`].
    #[serde(default)]
    pub automation: Vec<StationRatingAutomation>,
}

/// A single numeric metric in a generated section.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SystemManualMetric {
    /// Machine code (e.g. `"max_hp"`); the client maps it to a strings label id.
    pub code: String,
    /// The configured value, interpolated into the label by the client.
    pub value: f64,
}

/// One rating's automation footprint for a station.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StationRatingAutomation {
    /// Authored rating name (TOML data — an identifier the client maps to a
    /// caption, like the settings-panel rating toggle already does).
    pub rating: String,
    /// System ids automated under this rating (machine ids, not English).
    #[serde(default)]
    pub automated_systems: Vec<SystemId>,
}

/// One station's manual: authored overview + generated system sections.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StationManualWire {
    pub station_id: StationId,
    /// Authored overview prose from `[[station]] manual_overview` (TOML data;
    /// the single authored-English exception — see module docs). `None` when
    /// the station authored no overview.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub overview: Option<String>,
    /// Generated sections, one per owned system that has a registered provider.
    #[serde(default)]
    pub sections: Vec<SystemManualSection>,
}

/// The full ship manual: one entry per authored station.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ShipManualWire {
    #[serde(default)]
    pub stations: Vec<StationManualWire>,
}

// ── Provider registry ───────────────────────────────────────────────────────

/// Per-system-kind generated-section provider. Pure: reads only the ship
/// topology, the anchor system, its owning station, and optional kind-specific
/// `extra` config carrying values that live outside the station/system topology
/// (e.g. `[shields_console.base]`).
///
/// Issue #772 ships exactly one impl ([`ShieldsManualProvider`]); #773 slots in
/// the rest by registering more kinds — no change to the aggregator.
pub trait SystemManualProvider: Send + Sync {
    fn build(
        &self,
        ship_config: &ShipConfig,
        system: &SystemInstanceConfig,
        station_id: &StationId,
        extra: Option<&toml::Value>,
    ) -> SystemManualSection;
}

/// Registry of generated-section providers keyed by system kind.
#[derive(Default)]
pub struct ManualProviderRegistry {
    providers: HashMap<&'static str, Box<dyn SystemManualProvider>>,
}

impl ManualProviderRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a provider for a system kind. Later registrations win.
    pub fn register(&mut self, kind: &'static str, provider: Box<dyn SystemManualProvider>) {
        self.providers.insert(kind, provider);
    }

    /// The provider for a system kind, or `None` when nothing is registered
    /// (those systems render authored-overview-only — never a fabricated
    /// section).
    pub fn provider_for(&self, kind: &str) -> Option<&dyn SystemManualProvider> {
        self.providers.get(kind).map(|b| b.as_ref())
    }

    /// The default registry with every shipped provider. Issue #772 ships
    /// exactly one: Shields. #773 extends this with the remaining systems.
    pub fn with_shipped_providers() -> Self {
        let mut registry = Self::new();
        registry.register(
            crate::system_registry::SHIELDS_KIND,
            Box::new(ShieldsManualProvider),
        );
        registry
    }
}

// ── Aggregator ──────────────────────────────────────────────────────────────

/// Build the ship manual: for EVERY authored station, combine its authored
/// overview with a generated section for each owned system that has a
/// registered provider. Stations whose systems have no provider still appear
/// (overview-only) — AC1: every authored station is represented.
///
/// `system_extras` supplies kind-keyed config that providers need from outside
/// the topology (e.g. `[shields_console.base]` shield HP/regen), extracted by
/// the Bevy adapter from the ship's `EntityConfig`.
pub fn build_ship_manual(
    ship_config: &ShipConfig,
    registry: &ManualProviderRegistry,
    system_extras: &HashMap<String, toml::Value>,
) -> ShipManualWire {
    let stations = ship_config
        .stations
        .iter()
        .map(|station| {
            let sections = ship_config
                .systems_for_station(&station.id)
                .filter_map(|system| {
                    let provider = registry.provider_for(&system.kind)?;
                    let extra = system_extras.get(&system.kind);
                    Some(provider.build(ship_config, system, &station.id, extra))
                })
                .collect();
            StationManualWire {
                station_id: station.id.clone(),
                overview: station.manual_overview.clone(),
                sections,
            }
        })
        .collect();
    ShipManualWire { stations }
}

// ── Shields provider (issue #772 — the first and, for now, only provider) ─────

/// Generated-section provider for the Shields system.
///
/// Uses ACTUAL configured values: the base shield strength / regen from
/// `[shields_console.base]` (handed in via `extra`) and the arc count derived
/// from the ship's synthesised `shield_arc` systems, plus the owning station's
/// rating→AI automation mapping.
pub struct ShieldsManualProvider;

impl SystemManualProvider for ShieldsManualProvider {
    fn build(
        &self,
        ship_config: &ShipConfig,
        _system: &SystemInstanceConfig,
        station_id: &StationId,
        extra: Option<&toml::Value>,
    ) -> SystemManualSection {
        // Arc count: the number of synthesised `shield_arc` systems on the ship
        // (one per `[[shield_arc]]` block).
        let arc_count = ship_config
            .systems
            .iter()
            .filter(|s| s.kind == crate::system_registry::SHIELD_ARC_KIND)
            .count();

        // Base HP / regen from `[shields_console.base]` (via `extra`). Absent
        // sections fall back to 0.0 — the client renders whatever the ship
        // actually configured, never a hardcoded gameplay value.
        let max_hp = extra
            .and_then(|v| v.get("max_hp"))
            .and_then(toml_number)
            .unwrap_or(0.0);
        let regen = extra
            .and_then(|v| v.get("regen_per_sec"))
            .and_then(toml_number)
            .unwrap_or(0.0);

        let metrics = vec![
            SystemManualMetric {
                code: "max_hp".into(),
                value: max_hp,
            },
            SystemManualMetric {
                code: "regen".into(),
                value: regen,
            },
            SystemManualMetric {
                code: "arcs".into(),
                value: arc_count as f64,
            },
        ];

        // Rating→AI mapping for the owning station, including the implicit
        // `Backfill` rating (all owned systems become AI-operated).
        let automation = available_ratings_for_station(ship_config, station_id)
            .into_iter()
            .map(|rating| StationRatingAutomation {
                rating: rating.to_string(),
                automated_systems: resolve_automated_systems(ship_config, station_id, rating)
                    .unwrap_or_default(),
            })
            .collect();

        SystemManualSection {
            kind: crate::system_registry::SHIELDS_KIND.to_string(),
            metrics,
            automation,
        }
    }
}

/// Read a TOML value as an `f64`, accepting either float or integer literals.
fn toml_number(v: &toml::Value) -> Option<f64> {
    v.as_float().or_else(|| v.as_integer().map(|i| i as f64))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ship::config::parse_and_validate;
    use crate::ship::rating::BACKFILL_RATING;

    const KINDS: &[&str] = &[
        "captain",
        "red_alert",
        "shields",
        "shield_arc",
        "sensors",
        "helm_thrust",
    ];

    /// A cruiser-shaped config: a `science` station owning a `shields` system
    /// plus four synthesised `shield_arc` systems (as `entities::config` would
    /// synthesise them from `[[shield_arc]]` blocks), and a bare `captain`
    /// station with no provider-backed system.
    fn ship_toml() -> &'static str {
        r#"
[[station]]
id = "captain"
name = "Captain"
description = "Command."
rank = "Cpt."
manual_overview = "Captain overview prose."

[[station.rating]]
name = "Std"
automated_systems = []

[[station]]
id = "science"
name = "Science"
description = "Sensors and shields."
rank = "Ltn."
manual_overview = "Science overview prose."

[[station.rating]]
name = "Std"
automated_systems = ["sensors"]

[[station.rating]]
name = "Simplified"
automated_systems = ["sensors", "shield-arc-fore", "shield-arc-aft"]

[[system]]
id = "captain"
kind = "captain"
station = "captain"

[[system]]
id = "sensors"
kind = "sensors"
station = "science"

[[system]]
id = "shields-system"
kind = "shields"
station = "science"

[[system]]
id = "shield-arc-fore"
kind = "shield_arc"
station = "science"

[[system]]
id = "shield-arc-port"
kind = "shield_arc"
station = "science"

[[system]]
id = "shield-arc-aft"
kind = "shield_arc"
station = "science"

[[system]]
id = "shield-arc-starboard"
kind = "shield_arc"
station = "science"
"#
    }

    fn parse() -> ShipConfig {
        parse_and_validate(ship_toml(), KINDS).expect("ship config should parse")
    }

    fn shields_base_extras() -> HashMap<String, toml::Value> {
        let mut base = toml::value::Table::new();
        base.insert("max_hp".into(), toml::Value::Integer(100));
        base.insert("regen_per_sec".into(), toml::Value::Integer(2));
        HashMap::from([(
            crate::system_registry::SHIELDS_KIND.to_string(),
            toml::Value::Table(base),
        )])
    }

    fn shields_section(manual: &ShipManualWire) -> &SystemManualSection {
        manual
            .stations
            .iter()
            .find(|s| s.station_id == StationId("science".into()))
            .expect("science station present")
            .sections
            .iter()
            .find(|sec| sec.kind == crate::system_registry::SHIELDS_KIND)
            .expect("shields section present")
    }

    fn metric<'a>(section: &'a SystemManualSection, code: &str) -> &'a SystemManualMetric {
        section
            .metrics
            .iter()
            .find(|m| m.code == code)
            .unwrap_or_else(|| panic!("metric {code} present"))
    }

    #[test]
    fn every_authored_station_is_represented() {
        let manual = build_ship_manual(
            &parse(),
            &ManualProviderRegistry::with_shipped_providers(),
            &shields_base_extras(),
        );
        let ids: Vec<&str> = manual
            .stations
            .iter()
            .map(|s| s.station_id.0.as_str())
            .collect();
        assert_eq!(ids, vec!["captain", "science"]);
    }

    #[test]
    fn station_without_a_provider_appears_overview_only() {
        let manual = build_ship_manual(
            &parse(),
            &ManualProviderRegistry::with_shipped_providers(),
            &shields_base_extras(),
        );
        let captain = manual
            .stations
            .iter()
            .find(|s| s.station_id == StationId("captain".into()))
            .unwrap();
        assert_eq!(captain.overview.as_deref(), Some("Captain overview prose."));
        assert!(
            captain.sections.is_empty(),
            "captain owns no provider-backed system, so it must be overview-only"
        );
    }

    #[test]
    fn station_combines_overview_with_generated_section() {
        let manual = build_ship_manual(
            &parse(),
            &ManualProviderRegistry::with_shipped_providers(),
            &shields_base_extras(),
        );
        let science = manual
            .stations
            .iter()
            .find(|s| s.station_id == StationId("science".into()))
            .unwrap();
        assert_eq!(science.overview.as_deref(), Some("Science overview prose."));
        assert!(
            science
                .sections
                .iter()
                .any(|sec| sec.kind == crate::system_registry::SHIELDS_KIND),
            "science must carry the generated shields section alongside its overview"
        );
    }

    #[test]
    fn shields_section_reflects_configured_values() {
        let manual = build_ship_manual(
            &parse(),
            &ManualProviderRegistry::with_shipped_providers(),
            &shields_base_extras(),
        );
        let section = shields_section(&manual);
        assert_eq!(metric(section, "max_hp").value, 100.0);
        assert_eq!(metric(section, "regen").value, 2.0);
        // Four `[[shield_arc]]` blocks → four synthesised shield_arc systems.
        assert_eq!(metric(section, "arcs").value, 4.0);
    }

    #[test]
    fn shields_section_rating_mapping_matches_resolver() {
        let config = parse();
        let manual = build_ship_manual(
            &config,
            &ManualProviderRegistry::with_shipped_providers(),
            &shields_base_extras(),
        );
        let section = shields_section(&manual);
        let station = StationId("science".into());

        // Every authored rating plus the implicit Backfill is present.
        let ratings: Vec<&str> = section
            .automation
            .iter()
            .map(|a| a.rating.as_str())
            .collect();
        assert_eq!(ratings, vec!["Std", "Simplified", BACKFILL_RATING]);

        // Each row matches resolve_automated_systems exactly.
        for row in &section.automation {
            let expected =
                resolve_automated_systems(&config, &station, &row.rating).expect("rating resolves");
            assert_eq!(
                row.automated_systems, expected,
                "rating {} automation must mirror resolve_automated_systems",
                row.rating
            );
        }

        // Backfill automates every system the station owns.
        let backfill = section
            .automation
            .iter()
            .find(|a| a.rating == BACKFILL_RATING)
            .unwrap();
        assert_eq!(
            backfill.automated_systems,
            vec![
                SystemId("sensors".into()),
                SystemId("shields-system".into()),
                SystemId("shield-arc-fore".into()),
                SystemId("shield-arc-port".into()),
                SystemId("shield-arc-aft".into()),
                SystemId("shield-arc-starboard".into()),
            ]
        );
    }

    #[test]
    fn unregistered_kind_yields_no_section() {
        // An empty registry means even the shields system gets no section —
        // providers are never fabricated.
        let manual = build_ship_manual(
            &parse(),
            &ManualProviderRegistry::new(),
            &shields_base_extras(),
        );
        for station in &manual.stations {
            assert!(
                station.sections.is_empty(),
                "no providers registered, so no sections anywhere"
            );
        }
    }

    #[test]
    fn manual_wire_round_trips_through_json() {
        let manual = build_ship_manual(
            &parse(),
            &ManualProviderRegistry::with_shipped_providers(),
            &shields_base_extras(),
        );
        // serde_json lives only in codec.rs; use toml here for a pure-module
        // round-trip smoke of the wire structs.
        let encoded = toml::to_string(&manual).expect("serialize");
        let decoded: ShipManualWire = toml::from_str(&encoded).expect("deserialize");
        assert_eq!(decoded, manual);
    }
}
