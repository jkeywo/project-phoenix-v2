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
//! [`StationManualWire::overview`] carries an authored String Id or legacy
//! literal prose from `[[station]] manual_overview`. Rust preserves that value;
//! the client resolves known ids with `wireText` while retaining mod prose.

use crate::core::messages::{StationId, SystemId};
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
    /// Non-numeric capabilities that don't fit an `f64` metric (issue #773):
    /// each carries a machine `code` plus a machine `value_code`. The client
    /// maps the label id (`manual.<kind>.<code>`) and the value id
    /// (`manual.<kind>.<code>.<value_code>`) through `t()` — e.g. the Helm
    /// movement mode renders as a readable capability rather than a bare
    /// number. Empty for every provider that needs only numeric metrics.
    /// `#[serde(default)]` keeps pre-#773 manual round-trips (which never
    /// carried this field) intact.
    #[serde(default)]
    pub capabilities: Vec<SystemManualCapability>,
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

/// A single non-numeric capability in a generated section (issue #773).
///
/// Both `code` and `value_code` are stable MACHINE codes, never English. The
/// client composes the label id `manual.<kind>.<code>` and the value id
/// `manual.<kind>.<code>.<value_code>` and resolves both through `t()`. This
/// is how enum-valued capabilities (e.g. the Helm vertical movement mode)
/// reach the panel without smuggling English through the wire.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SystemManualCapability {
    /// Machine code for the capability (e.g. `"movement_mode"`).
    pub code: String,
    /// Machine code for the current value (e.g. `"planar"`).
    pub value_code: String,
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
    /// Authored String Id or legacy prose from `[[station]] manual_overview`.
    /// Resolved by the client; `None` when the station authored no overview.
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

    /// The default registry with every shipped provider. Issue #772 shipped
    /// exactly one (Shields); #773 registers a provider for every configured
    /// system kind that carries vessel-specific numeric or capability config.
    ///
    /// Kinds with NO numeric config (`captain`, `red_alert`, `viewscreen`, and
    /// the ownerless capability systems) are deliberately absent: those systems
    /// stay overview-only rather than emitting a fabricated section.
    pub fn with_shipped_providers() -> Self {
        use crate::ship::system_registry as kinds;
        let mut registry = Self::new();
        registry.register(kinds::SHIELDS_KIND, Box::new(ShieldsManualProvider));
        registry.register(kinds::PHASER_BANK_KIND, Box::new(PhaserBankManualProvider));
        registry.register(
            kinds::BLASTER_BANK_KIND,
            Box::new(BlasterBankManualProvider),
        );
        registry.register(
            kinds::TORPEDO_TUBE_KIND,
            Box::new(TorpedoTubeManualProvider),
        );
        registry.register(
            kinds::TORPEDO_MAGAZINE_KIND,
            Box::new(TorpedoMagazineManualProvider),
        );
        registry.register(
            kinds::TACTICAL_RADAR_KIND,
            Box::new(TacticalRadarManualProvider),
        );
        registry.register(kinds::SENSORS_KIND, Box::new(SensorsManualProvider));
        registry.register(
            kinds::SENSOR_RADAR_KIND,
            Box::new(SensorRadarManualProvider),
        );
        registry.register(
            kinds::POWER_REACTOR_KIND,
            Box::new(PowerReactorManualProvider),
        );
        registry.register(
            kinds::POWER_BATTERY_KIND,
            Box::new(PowerBatteryManualProvider),
        );
        registry.register(kinds::REPAIR_KIND, Box::new(RepairManualProvider));
        registry.register(kinds::COMMS_KIND, Box::new(CommsManualProvider));
        registry.register(kinds::HELM_THRUST_KIND, Box::new(HelmManualProvider));
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

// ── Shared provider helpers ──────────────────────────────────────────────────

/// Read a TOML value as an `f64`, accepting either float or integer literals.
fn toml_number(v: &toml::Value) -> Option<f64> {
    v.as_float().or_else(|| v.as_integer().map(|i| i as f64))
}

/// Read a keyed `f64` off a provider `extra` table, defaulting to `0.0` when the
/// key (or the whole `extra`) is absent. The manual then reflects whatever the
/// ship actually configured, never a hardcoded gameplay value.
fn extra_number(extra: Option<&toml::Value>, key: &str) -> f64 {
    extra
        .and_then(|v| v.get(key))
        .and_then(toml_number)
        .unwrap_or(0.0)
}

/// Count the systems of a given kind on the ship (topology-derived counts like
/// the shield arc count, tube count, and power-group count).
fn count_kind(ship_config: &ShipConfig, kind: &str) -> usize {
    ship_config
        .systems
        .iter()
        .filter(|s| s.kind == kind)
        .count()
}

/// Build the rating→AI automation footprint for a station, including the
/// implicit `Backfill` rating (all owned systems become AI-operated). Every
/// provider attaches this so the manual shows the owning station's automation
/// mapping alongside the system's numbers.
fn station_automation(
    ship_config: &ShipConfig,
    station_id: &StationId,
) -> Vec<StationRatingAutomation> {
    available_ratings_for_station(ship_config, station_id)
        .into_iter()
        .map(|rating| StationRatingAutomation {
            rating: rating.to_string(),
            automated_systems: resolve_automated_systems(ship_config, station_id, rating)
                .unwrap_or_default(),
        })
        .collect()
}

/// For a per-instance kind (phaser/blaster banks, torpedo tubes) whose one
/// kind-keyed `extra` packs a TOML array of per-instance tables each tagged
/// with a `system_id`, return the table matching this anchor `system`.
fn instance_entry<'a>(
    extra: Option<&'a toml::Value>,
    array_key: &str,
    system: &SystemInstanceConfig,
) -> Option<&'a toml::Value> {
    extra
        .and_then(|v| v.get(array_key))
        .and_then(|v| v.as_array())
        .and_then(|arr| {
            arr.iter().find(|entry| {
                entry.get("system_id").and_then(|s| s.as_str()) == Some(system.id.0.as_str())
            })
        })
}

// ── Shields provider (issue #772) ─────────────────────────────────────────────

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
        let arc_count = count_kind(ship_config, crate::ship::system_registry::SHIELD_ARC_KIND);
        let metrics = vec![
            SystemManualMetric {
                code: "max_hp".into(),
                value: extra_number(extra, "max_hp"),
            },
            SystemManualMetric {
                code: "regen".into(),
                value: extra_number(extra, "regen_per_sec"),
            },
            SystemManualMetric {
                code: "arcs".into(),
                value: arc_count as f64,
            },
        ];
        SystemManualSection {
            kind: crate::ship::system_registry::SHIELDS_KIND.to_string(),
            metrics,
            capabilities: vec![],
            automation: station_automation(ship_config, station_id),
        }
    }
}

// ── Weapons providers (issue #773) ────────────────────────────────────────────

/// Per-bank generated section for a Phaser Bank system. Each `phaser_bank`
/// `[[system]]` is its own configured system, so each contributes its own
/// section reflecting that bank's authored beam range, damage, cooldown and
/// fire arc (looked up by system id in the kind-keyed `extra` array).
pub struct PhaserBankManualProvider;

impl SystemManualProvider for PhaserBankManualProvider {
    fn build(
        &self,
        ship_config: &ShipConfig,
        system: &SystemInstanceConfig,
        station_id: &StationId,
        extra: Option<&toml::Value>,
    ) -> SystemManualSection {
        let entry = instance_entry(extra, "banks", system);
        let metrics = vec![
            SystemManualMetric {
                code: "beam_range".into(),
                value: extra_number(entry, "beam_range"),
            },
            SystemManualMetric {
                code: "beam_damage".into(),
                value: extra_number(entry, "beam_damage_per_sec"),
            },
            SystemManualMetric {
                code: "cooldown".into(),
                value: extra_number(entry, "cooldown_secs"),
            },
            SystemManualMetric {
                code: "fire_arc".into(),
                value: extra_number(entry, "fire_arc_deg"),
            },
        ];
        SystemManualSection {
            kind: crate::ship::system_registry::PHASER_BANK_KIND.to_string(),
            metrics,
            capabilities: vec![],
            automation: station_automation(ship_config, station_id),
        }
    }
}

/// Per-bank generated section for a Blaster Bank system (issue #631/#765). Each
/// bank reflects its authored range, volley count, cooldown, fire arc and
/// declared barrel count.
pub struct BlasterBankManualProvider;

impl SystemManualProvider for BlasterBankManualProvider {
    fn build(
        &self,
        ship_config: &ShipConfig,
        system: &SystemInstanceConfig,
        station_id: &StationId,
        extra: Option<&toml::Value>,
    ) -> SystemManualSection {
        let entry = instance_entry(extra, "banks", system);
        let metrics = vec![
            SystemManualMetric {
                code: "range".into(),
                value: extra_number(entry, "range"),
            },
            SystemManualMetric {
                code: "volley".into(),
                value: extra_number(entry, "volley_count"),
            },
            SystemManualMetric {
                code: "cooldown".into(),
                value: extra_number(entry, "cooldown_secs"),
            },
            SystemManualMetric {
                code: "fire_arc".into(),
                value: extra_number(entry, "fire_arc_deg"),
            },
            SystemManualMetric {
                code: "barrels".into(),
                value: extra_number(entry, "barrel_count"),
            },
        ];
        SystemManualSection {
            kind: crate::ship::system_registry::BLASTER_BANK_KIND.to_string(),
            metrics,
            capabilities: vec![],
            automation: station_automation(ship_config, station_id),
        }
    }
}

/// Per-tube generated section for a Torpedo Tube system. Each tube reflects its
/// authored fire arc, effective load time and volley capacity (with the shared
/// magazine/warhead figures living on the Torpedo Magazine section).
pub struct TorpedoTubeManualProvider;

impl SystemManualProvider for TorpedoTubeManualProvider {
    fn build(
        &self,
        ship_config: &ShipConfig,
        system: &SystemInstanceConfig,
        station_id: &StationId,
        extra: Option<&toml::Value>,
    ) -> SystemManualSection {
        let entry = instance_entry(extra, "tubes", system);
        let metrics = vec![
            SystemManualMetric {
                code: "fire_arc".into(),
                value: extra_number(entry, "fire_arc_deg"),
            },
            SystemManualMetric {
                code: "load_time".into(),
                value: extra_number(entry, "load_time"),
            },
            SystemManualMetric {
                code: "volley_max".into(),
                value: extra_number(entry, "volley_max"),
            },
        ];
        SystemManualSection {
            kind: crate::ship::system_registry::TORPEDO_TUBE_KIND.to_string(),
            metrics,
            capabilities: vec![],
            automation: station_automation(ship_config, station_id),
        }
    }
}

/// Generated section for the single Torpedo Magazine system: shared magazine
/// capacity and warhead damage from `[torpedoes]`, plus the launch-tube count
/// derived from the ship topology.
pub struct TorpedoMagazineManualProvider;

impl SystemManualProvider for TorpedoMagazineManualProvider {
    fn build(
        &self,
        ship_config: &ShipConfig,
        _system: &SystemInstanceConfig,
        station_id: &StationId,
        extra: Option<&toml::Value>,
    ) -> SystemManualSection {
        let tube_count = count_kind(ship_config, crate::ship::system_registry::TORPEDO_TUBE_KIND);
        let metrics = vec![
            SystemManualMetric {
                code: "capacity".into(),
                value: extra_number(extra, "count"),
            },
            SystemManualMetric {
                code: "damage_hull".into(),
                value: extra_number(extra, "damage_hull"),
            },
            SystemManualMetric {
                code: "damage_shields".into(),
                value: extra_number(extra, "damage_shields"),
            },
            SystemManualMetric {
                code: "tubes".into(),
                value: tube_count as f64,
            },
        ];
        SystemManualSection {
            kind: crate::ship::system_registry::TORPEDO_MAGAZINE_KIND.to_string(),
            metrics,
            capabilities: vec![],
            automation: station_automation(ship_config, station_id),
        }
    }
}

// ── Sensors / radar providers (issue #773) ────────────────────────────────────

/// Generated section for the Tactical Radar: the weapons-console radar range.
pub struct TacticalRadarManualProvider;

impl SystemManualProvider for TacticalRadarManualProvider {
    fn build(
        &self,
        ship_config: &ShipConfig,
        _system: &SystemInstanceConfig,
        station_id: &StationId,
        extra: Option<&toml::Value>,
    ) -> SystemManualSection {
        SystemManualSection {
            kind: crate::ship::system_registry::TACTICAL_RADAR_KIND.to_string(),
            metrics: vec![SystemManualMetric {
                code: "range".into(),
                value: extra_number(extra, "range"),
            }],
            capabilities: vec![],
            automation: station_automation(ship_config, station_id),
        }
    }
}

/// Generated section for the Sensors system: the long-range sensor radar range.
pub struct SensorsManualProvider;

impl SystemManualProvider for SensorsManualProvider {
    fn build(
        &self,
        ship_config: &ShipConfig,
        _system: &SystemInstanceConfig,
        station_id: &StationId,
        extra: Option<&toml::Value>,
    ) -> SystemManualSection {
        SystemManualSection {
            kind: crate::ship::system_registry::SENSORS_KIND.to_string(),
            metrics: vec![SystemManualMetric {
                code: "range".into(),
                value: extra_number(extra, "range"),
            }],
            capabilities: vec![],
            automation: station_automation(ship_config, station_id),
        }
    }
}

/// Generated section for the Sensor Radar fine system: the same long-range radar
/// range, made damageable/repairable on its own.
pub struct SensorRadarManualProvider;

impl SystemManualProvider for SensorRadarManualProvider {
    fn build(
        &self,
        ship_config: &ShipConfig,
        _system: &SystemInstanceConfig,
        station_id: &StationId,
        extra: Option<&toml::Value>,
    ) -> SystemManualSection {
        SystemManualSection {
            kind: crate::ship::system_registry::SENSOR_RADAR_KIND.to_string(),
            metrics: vec![SystemManualMetric {
                code: "range".into(),
                value: extra_number(extra, "range"),
            }],
            capabilities: vec![],
            automation: station_automation(ship_config, station_id),
        }
    }
}

// ── Power / repair / comms providers (issue #773) ─────────────────────────────

/// Generated section for the Power Reactor: reactor capacity from `[power]` plus
/// the count of authored power groups (topology-derived).
pub struct PowerReactorManualProvider;

impl SystemManualProvider for PowerReactorManualProvider {
    fn build(
        &self,
        ship_config: &ShipConfig,
        _system: &SystemInstanceConfig,
        station_id: &StationId,
        extra: Option<&toml::Value>,
    ) -> SystemManualSection {
        let metrics = vec![
            SystemManualMetric {
                code: "capacity".into(),
                value: extra_number(extra, "capacity"),
            },
            SystemManualMetric {
                code: "power_groups".into(),
                value: ship_config.power_groups.len() as f64,
            },
        ];
        SystemManualSection {
            kind: crate::ship::system_registry::POWER_REACTOR_KIND.to_string(),
            metrics,
            capabilities: vec![],
            automation: station_automation(ship_config, station_id),
        }
    }
}

/// Generated section for the Power Battery: the emergency-reserve threshold from
/// `[power]` (the reactor carries capacity; the battery carries the reserve).
pub struct PowerBatteryManualProvider;

impl SystemManualProvider for PowerBatteryManualProvider {
    fn build(
        &self,
        ship_config: &ShipConfig,
        _system: &SystemInstanceConfig,
        station_id: &StationId,
        extra: Option<&toml::Value>,
    ) -> SystemManualSection {
        SystemManualSection {
            kind: crate::ship::system_registry::POWER_BATTERY_KIND.to_string(),
            metrics: vec![SystemManualMetric {
                code: "emergency_threshold".into(),
                value: extra_number(extra, "emergency_threshold"),
            }],
            capabilities: vec![],
            automation: station_automation(ship_config, station_id),
        }
    }
}

/// Generated section for the Repair system: team count, repair rate and travel
/// time from `[repair]`.
pub struct RepairManualProvider;

impl SystemManualProvider for RepairManualProvider {
    fn build(
        &self,
        ship_config: &ShipConfig,
        _system: &SystemInstanceConfig,
        station_id: &StationId,
        extra: Option<&toml::Value>,
    ) -> SystemManualSection {
        let metrics = vec![
            SystemManualMetric {
                code: "teams".into(),
                value: extra_number(extra, "repair_team_count"),
            },
            SystemManualMetric {
                code: "rate".into(),
                value: extra_number(extra, "repair_rate_hp_per_sec"),
            },
            SystemManualMetric {
                code: "travel".into(),
                value: extra_number(extra, "travel_duration_secs"),
            },
        ];
        SystemManualSection {
            kind: crate::ship::system_registry::REPAIR_KIND.to_string(),
            metrics,
            capabilities: vec![],
            automation: station_automation(ship_config, station_id),
        }
    }
}

/// Generated section for the Comms system: comms range from `[comms]`.
pub struct CommsManualProvider;

impl SystemManualProvider for CommsManualProvider {
    fn build(
        &self,
        ship_config: &ShipConfig,
        _system: &SystemInstanceConfig,
        station_id: &StationId,
        extra: Option<&toml::Value>,
    ) -> SystemManualSection {
        SystemManualSection {
            kind: crate::ship::system_registry::COMMS_KIND.to_string(),
            metrics: vec![SystemManualMetric {
                code: "range".into(),
                value: extra_number(extra, "range"),
            }],
            capabilities: vec![],
            automation: station_automation(ship_config, station_id),
        }
    }
}

// ── Helm provider (issue #773, AC3) ───────────────────────────────────────────

/// Generated Helm section, anchored on the `helm_thrust` axis (the throttle,
/// always present exactly once on a helm). Reflects the authored `[helm_console]`
/// speeds and the `[helm_capability]` impulse steering multiplier as numeric
/// metrics, plus the vertical movement mode as a non-numeric CAPABILITY
/// (`planar` / `bounded` / `full_3d`) — the effective value, defaulting to
/// `planar` when no `[helm_capability]` block is authored.
pub struct HelmManualProvider;

impl SystemManualProvider for HelmManualProvider {
    fn build(
        &self,
        ship_config: &ShipConfig,
        _system: &SystemInstanceConfig,
        station_id: &StationId,
        extra: Option<&toml::Value>,
    ) -> SystemManualSection {
        let metrics = vec![
            SystemManualMetric {
                code: "max_speed".into(),
                value: extra_number(extra, "max_speed"),
            },
            SystemManualMetric {
                code: "max_reverse_speed".into(),
                value: extra_number(extra, "max_reverse_speed"),
            },
            SystemManualMetric {
                code: "max_yaw_rate".into(),
                value: extra_number(extra, "max_yaw_rate"),
            },
            SystemManualMetric {
                code: "impulse_steering".into(),
                value: extra_number(extra, "impulse_steering_multiplier"),
            },
        ];
        // Movement mode is an enum, not an f64 — carry it as a machine
        // value_code the client maps through `t()`. Absent block ⇒ effective
        // `planar` (the runtime default), so the manual reflects the real value.
        let movement_mode = extra
            .and_then(|v| v.get("movement_mode"))
            .and_then(|v| v.as_str())
            .unwrap_or("planar")
            .to_string();
        SystemManualSection {
            kind: crate::ship::system_registry::HELM_THRUST_KIND.to_string(),
            metrics,
            capabilities: vec![SystemManualCapability {
                code: "movement_mode".into(),
                value_code: movement_mode,
            }],
            automation: station_automation(ship_config, station_id),
        }
    }
}

#[cfg(test)]
#[path = "manual_tests.rs"]
mod tests;
