//! System-kind registry and stable `SystemId` helpers.
//!
//! ## SystemId naming convention (pinned by issue #525)
//!
//! Every `SystemId` string follows one of three patterns:
//!
//! | Pattern | Rule | Examples |
//! |---------|------|---------|
//! | **Coarse system** | Lowercase kebab matching the system kind id | `"helm"`, `"tactical"`, `"red-alert"` |
//! | **Fine system** | Kind id + `-` + instance suffix | `"phaser-fore"`, `"torpedo-tube-fore-port"` |
//! | **Ownerless capability** | Bare capability id (lowercase kebab) | `"red-alert"`, `"viewscreen"` |
//!
//! Multi-word ids always use hyphens (`-`), never underscores.
//!
//! ### `red_alert` vs `red-alert` quirk
//!
//! The registry key (`*_KIND` constants) uses snake_case for `red_alert` because
//! Rust identifiers and some legacy map keys historically used underscores, while
//! the wire `*_SYSTEM_ID` value uses kebab (`"red-alert"`). All other systems have
//! identical `*_KIND` and `*_SYSTEM_ID` values. New systems must use the same
//! lowercase-kebab string for both constants to avoid this split.

use crate::messages::SystemId;
use std::collections::HashMap;

// ── Ownerless capability systems ─────────────────────────────────────────────

/// Wire `SystemId` for the Red Alert coarse system.
///
/// Ownerless capability — multi-word kebab id. Registry kind key is `"red_alert"`
/// (snake_case legacy quirk; see module-level doc for details).
pub const RED_ALERT_SYSTEM_ID: &str = "red-alert";
/// Registry kind key for Red Alert (snake_case for legacy reasons; see module doc).
pub const RED_ALERT_KIND: &str = "red_alert";
pub const RED_ALERT_AI_CONTROLLER: &str = "red_alert_ai";

/// Wire `SystemId` for the Viewscreen coarse system.
///
/// Ownerless capability — single-word lowercase id.
pub const VIEWSCREEN_SYSTEM_ID: &str = "viewscreen";
pub const VIEWSCREEN_KIND: &str = "viewscreen";
pub const VIEWSCREEN_AI_CONTROLLER: &str = "viewscreen_ai";

// ── Station-owned coarse systems ─────────────────────────────────────────────

/// Wire `SystemId` for the Helm coarse system.
pub const HELM_SYSTEM_ID: &str = "helm";
pub const HELM_KIND: &str = "helm";
pub const HELM_AI_CONTROLLER: &str = "helm_ai";

/// Wire `SystemId` for the Tactical coarse system.
pub const TACTICAL_SYSTEM_ID: &str = "tactical";
pub const TACTICAL_KIND: &str = "tactical";
pub const TACTICAL_AI_CONTROLLER: &str = "tactical_ai";

/// Wire `SystemId` for the Power coarse system.
pub const POWER_SYSTEM_ID: &str = "power";
pub const POWER_KIND: &str = "power";
pub const POWER_AI_CONTROLLER: &str = "power_ai";

/// Wire `SystemId` for the Sensors coarse system.
pub const SENSORS_SYSTEM_ID: &str = "sensors";
pub const SENSORS_KIND: &str = "sensors";
pub const SENSORS_AI_CONTROLLER: &str = "sensors_ai";

/// Wire `SystemId` for the Navigation coarse system.
pub const NAVIGATION_SYSTEM_ID: &str = "navigation";
pub const NAVIGATION_KIND: &str = "navigation";
pub const NAVIGATION_AI_CONTROLLER: &str = "navigation_ai";

/// Wire `SystemId` for the Shields coarse system.
pub const SHIELDS_SYSTEM_ID: &str = "shields";
pub const SHIELDS_KIND: &str = "shields";
pub const SHIELDS_AI_CONTROLLER: &str = "shields_ai";

/// Wire `SystemId` for the Comms coarse system.
pub const COMMS_SYSTEM_ID: &str = "comms";
pub const COMMS_KIND: &str = "comms";
pub const COMMS_AI_CONTROLLER: &str = "comms_ai";

/// Wire `SystemId` for the Captain coarse system.
pub const CAPTAIN_SYSTEM_ID: &str = "captain";
pub const CAPTAIN_KIND: &str = "captain";
pub const CAPTAIN_AI_CONTROLLER: &str = "captain_ai";

/// Wire `SystemId` for the Repair coarse system.
pub const REPAIR_SYSTEM_ID: &str = "repair";
pub const REPAIR_KIND: &str = "repair";
pub const REPAIR_AI_CONTROLLER: &str = "repair_ai";

// ── Fine-grained Helm systems (issue #511) ────────────────────────────────────

/// Wire `SystemId` for the Helm Joystick fine system.
pub const HELM_JOYSTICK_KIND: &str = "helm_joystick";
pub const HELM_JOYSTICK_SYSTEM_ID: &str = "helm-joystick";
pub const HELM_JOYSTICK_AI_CONTROLLER: &str = "helm_joystick_ai";

/// Wire `SystemId` for the Helm Engine fine systems (port + starboard instances).
pub const HELM_ENGINE_KIND: &str = "helm_engine";
pub const HELM_ENGINE_PORT_SYSTEM_ID: &str = "helm-engine-port";
pub const HELM_ENGINE_STARBOARD_SYSTEM_ID: &str = "helm-engine-starboard";
pub const HELM_ENGINE_AI_CONTROLLER: &str = "helm_engine_ai";

/// Wire `SystemId` for the Helm Radar fine system.
pub const HELM_RADAR_KIND: &str = "helm_radar";
pub const HELM_RADAR_SYSTEM_ID: &str = "helm-radar";
pub const HELM_RADAR_AI_CONTROLLER: &str = "helm_radar_ai";

/// Wire `SystemId` for the Helm Impulse fine system.
pub const HELM_IMPULSE_KIND: &str = "helm_impulse";
pub const HELM_IMPULSE_SYSTEM_ID: &str = "helm-impulse";
pub const HELM_IMPULSE_AI_CONTROLLER: &str = "helm_impulse_ai";

// ── Fine-grained Tactical systems (issue #512) ────────────────────────────────
//
// The coarse `tactical` kind is DELETED from the runtime registry in favour of
// three fine kinds. `TACTICAL_SYSTEM_ID = "tactical"` is retained purely as a
// coordination surface — ship-level operations (SetTarget / SetPhaserMode /
// SetPhaserFrequency / ToggleAutoFire) still address this string but their
// authorisation gate is now "any bank policy accepts human input" (option (c)
// in the issue), so the coarse system no longer needs its own `[[system]]`
// block.

/// Wire `SystemId` for the Phaser Bank fine systems.
///
/// Registered per-instance in TOML (e.g. `"phaser-fore"`, `"phaser-aft"`).
pub const PHASER_BANK_KIND: &str = "phaser_bank";
pub const PHASER_FORE_SYSTEM_ID: &str = "phaser-fore";
pub const PHASER_AFT_SYSTEM_ID: &str = "phaser-aft";
pub const PHASER_BANK_AI_CONTROLLER: &str = "phaser_bank_ai";

/// Wire `SystemId` for the Torpedo Tube fine systems.
///
/// Registered per-instance in TOML (e.g. `"torpedo-tube-fore-port"`).
pub const TORPEDO_TUBE_KIND: &str = "torpedo_tube";
pub const TORPEDO_TUBE_FORE_PORT_SYSTEM_ID: &str = "torpedo-tube-fore-port";
pub const TORPEDO_TUBE_FORE_STARBOARD_SYSTEM_ID: &str = "torpedo-tube-fore-starboard";
pub const TORPEDO_TUBE_AFT_SYSTEM_ID: &str = "torpedo-tube-aft";
pub const TORPEDO_TUBE_AI_CONTROLLER: &str = "torpedo_tube_ai";

/// Wire `SystemId` for the Blaster Bank fine systems (issue #631).
///
/// Registered per-instance in TOML (e.g. `"blaster-fore"`, `"blaster-aft"`).
/// A blaster bank fires straight-flying projectiles in data-driven volleys.
pub const BLASTER_BANK_KIND: &str = "blaster_bank";
pub const BLASTER_BANK_AI_CONTROLLER: &str = "blaster_bank_ai";

/// Wire `SystemId` for the Tactical Radar fine system.
///
/// Mirrors `HELM_RADAR_KIND`/`HELM_RADAR_SYSTEM_ID` — the tactical station's
/// short-range weapons radar, made damageable/repairable like every other
/// fine system.
pub const TACTICAL_RADAR_KIND: &str = "tactical_radar";
pub const TACTICAL_RADAR_SYSTEM_ID: &str = "tactical-radar";
pub const TACTICAL_RADAR_AI_CONTROLLER: &str = "tactical_radar_ai";

/// Wire `SystemId` for the Sensor Radar fine system.
///
/// Mirrors `HELM_RADAR_KIND`/`HELM_RADAR_SYSTEM_ID` — the sensors/science
/// station's long-range radar, made damageable/repairable like every other
/// fine system.
pub const SENSOR_RADAR_KIND: &str = "sensor_radar";
pub const SENSOR_RADAR_SYSTEM_ID: &str = "sensor-radar";
pub const SENSOR_RADAR_AI_CONTROLLER: &str = "sensor_radar_ai";

/// Wire `SystemId` for the Torpedo Magazine fine system (single instance).
///
/// The magazine owns the shared torpedo `count`; tubes claim a round via
/// the channel-2 [`crate::messages::InterSystemPayload::ClaimTorpedoRound`]
/// message. A Disabled/Destroyed magazine refuses claims (no tubes can load
/// even if a round would otherwise be available), and also blocks the fire
/// path so loaded tubes cannot launch.
pub const TORPEDO_MAGAZINE_KIND: &str = "torpedo_magazine";
pub const TORPEDO_MAGAZINE_SYSTEM_ID: &str = "torpedo-magazine";
pub const TORPEDO_MAGAZINE_AI_CONTROLLER: &str = "torpedo_magazine_ai";

// ── Fine-grained Power systems (issue #513) ──────────────────────────────────
//
// The coarse `power` kind is DELETED from the player ship TOML, but
// `POWER_SYSTEM_ID = "power"` remains as a stable string constant so tests
// and legacy readers (e.g. the JS panel's aggregate `blackboards['power']`
// entry) can still address the aggregate surface. All admission /
// allocation logic now targets the fine `power_reactor` kind; battery drain
// (channel-2 `DrainWeaponsBattery`) targets `power_battery`. Both fine
// systems live on the `power` station and are held by the single
// power-station holder — the split is invisible to the human but grants
// per-instance damage semantics (reactor disabled → no allocation input;
// battery disabled → no emergency reserves).

/// Wire `SystemId` for the Power Reactor fine system.
///
/// The reactor OWNS the allocation surface: `SetPowerGroupAllocation`
/// payloads are gated on `policy_for(&power_reactor_system_id())`.
/// A Disabled/Destroyed reactor refuses allocation input via the standard
/// `accept_human_input` gate.
pub const POWER_REACTOR_KIND: &str = "power_reactor";
pub const POWER_REACTOR_SYSTEM_ID: &str = "power-reactor";
pub const POWER_REACTOR_AI_CONTROLLER: &str = "power_reactor_ai";

/// Wire `SystemId` for the Power Battery fine system.
///
/// The battery is the target for `InterSystemPayload::DrainWeaponsBattery`
/// (channel-2). A Disabled/Destroyed battery refuses the drain — the pool
/// is treated as immovable/0-reserves so magazine-style weapons draws
/// (phaser beams) cannot consume from it.
pub const POWER_BATTERY_KIND: &str = "power_battery";
pub const POWER_BATTERY_SYSTEM_ID: &str = "power-battery";
pub const POWER_BATTERY_AI_CONTROLLER: &str = "power_battery_ai";

// ── Fine-grained Shields systems (issue #514) ────────────────────────────────
//
// The coarse `shields` kind is DELETED from the player ship TOML, but
// `SHIELDS_SYSTEM_ID = "shields"` remains as a stable string constant so
// tests and legacy readers (e.g. the JS panel's aggregate
// `blackboards['shields']` entry) can still address the aggregate surface.
// All per-arc admission and per-arc damage now target `shield_arc` fine
// systems registered per-instance in TOML (e.g. `"shield-arc-fore"`).
//
// Ships may declare any number of `[[shield_arc]]` blocks; each block
// auto-generates a corresponding `[[system]]` entry with
// `kind = "shield_arc"` at TOML-parse time.

/// Wire `SystemId` for the Shield Arc fine systems.
///
/// Registered per-instance in TOML (e.g. `"shield-arc-fore"`,
/// `"shield-arc-aft"`). Arc count is variable — a ship declares one
/// `[[shield_arc]]` block per arc, from which the parser synthesises a
/// matching `[[system]]` entry.
pub const SHIELD_ARC_KIND: &str = "shield_arc";
pub const SHIELD_ARC_AI_CONTROLLER: &str = "shield_arc_ai";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AiControllerRegistration {
    name: String,
}

impl AiControllerRegistration {
    pub fn new(name: impl Into<String>) -> Result<Self, SystemRegistryError> {
        let name = name.into();
        if name.trim().is_empty() {
            return Err(SystemRegistryError::EmptyAiControllerName);
        }
        Ok(Self { name })
    }

    pub fn name(&self) -> &str {
        &self.name
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SystemKindRegistration {
    pub kind: String,
    pub ai_controller: AiControllerRegistration,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SystemKindRegistry {
    kinds: HashMap<String, SystemKindRegistration>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SystemRegistryError {
    EmptyKind,
    EmptyAiControllerName,
    DuplicateKind { kind: String },
}

impl std::fmt::Display for SystemRegistryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self)
    }
}

impl std::error::Error for SystemRegistryError {}

impl SystemKindRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_red_alert() -> Result<Self, SystemRegistryError> {
        let mut registry = Self::new();
        registry.register(
            RED_ALERT_KIND,
            AiControllerRegistration::new(RED_ALERT_AI_CONTROLLER)?,
        )?;
        Ok(registry)
    }

    pub fn with_core_systems() -> Result<Self, SystemRegistryError> {
        let mut registry = Self::with_red_alert()?;
        registry.register(
            HELM_KIND,
            AiControllerRegistration::new(HELM_AI_CONTROLLER)?,
        )?;
        registry.register(
            TACTICAL_KIND,
            AiControllerRegistration::new(TACTICAL_AI_CONTROLLER)?,
        )?;
        registry.register(
            POWER_KIND,
            AiControllerRegistration::new(POWER_AI_CONTROLLER)?,
        )?;
        registry.register(
            SENSORS_KIND,
            AiControllerRegistration::new(SENSORS_AI_CONTROLLER)?,
        )?;
        registry.register(
            NAVIGATION_KIND,
            AiControllerRegistration::new(NAVIGATION_AI_CONTROLLER)?,
        )?;
        registry.register(
            SHIELDS_KIND,
            AiControllerRegistration::new(SHIELDS_AI_CONTROLLER)?,
        )?;
        registry.register(
            COMMS_KIND,
            AiControllerRegistration::new(COMMS_AI_CONTROLLER)?,
        )?;
        registry.register(
            CAPTAIN_KIND,
            AiControllerRegistration::new(CAPTAIN_AI_CONTROLLER)?,
        )?;
        registry.register(
            VIEWSCREEN_KIND,
            AiControllerRegistration::new(VIEWSCREEN_AI_CONTROLLER)?,
        )?;
        registry.register(
            REPAIR_KIND,
            AiControllerRegistration::new(REPAIR_AI_CONTROLLER)?,
        )?;
        // Fine-grained Helm systems (issue #511)
        registry.register(
            HELM_JOYSTICK_KIND,
            AiControllerRegistration::new(HELM_JOYSTICK_AI_CONTROLLER)?,
        )?;
        registry.register(
            HELM_ENGINE_KIND,
            AiControllerRegistration::new(HELM_ENGINE_AI_CONTROLLER)?,
        )?;
        registry.register(
            HELM_RADAR_KIND,
            AiControllerRegistration::new(HELM_RADAR_AI_CONTROLLER)?,
        )?;
        registry.register(
            HELM_IMPULSE_KIND,
            AiControllerRegistration::new(HELM_IMPULSE_AI_CONTROLLER)?,
        )?;
        // Fine-grained Tactical systems (issue #512)
        registry.register(
            PHASER_BANK_KIND,
            AiControllerRegistration::new(PHASER_BANK_AI_CONTROLLER)?,
        )?;
        registry.register(
            TORPEDO_TUBE_KIND,
            AiControllerRegistration::new(TORPEDO_TUBE_AI_CONTROLLER)?,
        )?;
        registry.register(
            TORPEDO_MAGAZINE_KIND,
            AiControllerRegistration::new(TORPEDO_MAGAZINE_AI_CONTROLLER)?,
        )?;
        // Blaster bank fine system (issue #631)
        registry.register(
            BLASTER_BANK_KIND,
            AiControllerRegistration::new(BLASTER_BANK_AI_CONTROLLER)?,
        )?;
        // Tactical / sensor radar fine systems
        registry.register(
            TACTICAL_RADAR_KIND,
            AiControllerRegistration::new(TACTICAL_RADAR_AI_CONTROLLER)?,
        )?;
        registry.register(
            SENSOR_RADAR_KIND,
            AiControllerRegistration::new(SENSOR_RADAR_AI_CONTROLLER)?,
        )?;
        // Fine-grained Power systems (issue #513)
        registry.register(
            POWER_REACTOR_KIND,
            AiControllerRegistration::new(POWER_REACTOR_AI_CONTROLLER)?,
        )?;
        registry.register(
            POWER_BATTERY_KIND,
            AiControllerRegistration::new(POWER_BATTERY_AI_CONTROLLER)?,
        )?;
        // Fine-grained Shields systems (issue #514)
        registry.register(
            SHIELD_ARC_KIND,
            AiControllerRegistration::new(SHIELD_ARC_AI_CONTROLLER)?,
        )?;
        Ok(registry)
    }

    pub fn register(
        &mut self,
        kind: impl Into<String>,
        ai_controller: AiControllerRegistration,
    ) -> Result<(), SystemRegistryError> {
        let kind = kind.into();
        if kind.trim().is_empty() {
            return Err(SystemRegistryError::EmptyKind);
        }
        if ai_controller.name.trim().is_empty() {
            return Err(SystemRegistryError::EmptyAiControllerName);
        }
        if self.kinds.contains_key(&kind) {
            return Err(SystemRegistryError::DuplicateKind { kind });
        }
        self.kinds.insert(
            kind.clone(),
            SystemKindRegistration {
                kind,
                ai_controller,
            },
        );
        Ok(())
    }

    pub fn contains(&self, kind: &str) -> bool {
        self.kinds.contains_key(kind)
    }

    pub fn registration(&self, kind: &str) -> Option<&SystemKindRegistration> {
        self.kinds.get(kind)
    }

    pub fn kinds(&self) -> impl Iterator<Item = &str> {
        self.kinds.keys().map(|kind| kind.as_str())
    }
}

// ── SystemId helpers ──────────────────────────────────────────────────────────
//
// Each helper returns a `SystemId` backed by the corresponding `*_SYSTEM_ID`
// constant. Always prefer these helpers over inline `SystemId("helm".into())`
// literals — the helpers are the pinned authoritative source.

pub fn red_alert_system_id() -> SystemId {
    SystemId(RED_ALERT_SYSTEM_ID.to_string())
}

pub fn helm_system_id() -> SystemId {
    SystemId(HELM_SYSTEM_ID.to_string())
}

pub fn tactical_system_id() -> SystemId {
    SystemId(TACTICAL_SYSTEM_ID.to_string())
}

// NOTE: There is no `power_system_id()` helper. The coarse `POWER_SYSTEM_ID`
// string constant is retained only for the aggregate blackboard key (published
// alongside the fine `power-reactor` / `power-battery` blackboards for legacy
// JS readers). All control-input routing must use
// `power_reactor_system_id()` (allocation surface) or
// `power_battery_system_id()` (channel-2 drain target).

pub fn sensors_system_id() -> SystemId {
    SystemId(SENSORS_SYSTEM_ID.to_string())
}

pub fn navigation_system_id() -> SystemId {
    SystemId(NAVIGATION_SYSTEM_ID.to_string())
}

pub fn shields_system_id() -> SystemId {
    SystemId(SHIELDS_SYSTEM_ID.to_string())
}

pub fn comms_system_id() -> SystemId {
    SystemId(COMMS_SYSTEM_ID.to_string())
}

pub fn captain_system_id() -> SystemId {
    SystemId(CAPTAIN_SYSTEM_ID.to_string())
}

pub fn viewscreen_system_id() -> SystemId {
    SystemId(VIEWSCREEN_SYSTEM_ID.to_string())
}

pub fn repair_system_id() -> SystemId {
    SystemId(REPAIR_SYSTEM_ID.to_string())
}

// ── Fine Helm system id helpers (issue #511) ──────────────────────────────────

pub fn helm_joystick_system_id() -> SystemId {
    SystemId(HELM_JOYSTICK_SYSTEM_ID.to_string())
}

pub fn helm_engine_port_system_id() -> SystemId {
    SystemId(HELM_ENGINE_PORT_SYSTEM_ID.to_string())
}

pub fn helm_engine_starboard_system_id() -> SystemId {
    SystemId(HELM_ENGINE_STARBOARD_SYSTEM_ID.to_string())
}

pub fn helm_radar_system_id() -> SystemId {
    SystemId(HELM_RADAR_SYSTEM_ID.to_string())
}

pub fn tactical_radar_system_id() -> SystemId {
    SystemId(TACTICAL_RADAR_SYSTEM_ID.to_string())
}

pub fn sensor_radar_system_id() -> SystemId {
    SystemId(SENSOR_RADAR_SYSTEM_ID.to_string())
}

pub fn helm_impulse_system_id() -> SystemId {
    SystemId(HELM_IMPULSE_SYSTEM_ID.to_string())
}

// ── Fine Tactical system id helpers (issue #512) ──────────────────────────────

pub fn phaser_fore_system_id() -> SystemId {
    SystemId(PHASER_FORE_SYSTEM_ID.to_string())
}

pub fn phaser_aft_system_id() -> SystemId {
    SystemId(PHASER_AFT_SYSTEM_ID.to_string())
}

pub fn torpedo_tube_fore_port_system_id() -> SystemId {
    SystemId(TORPEDO_TUBE_FORE_PORT_SYSTEM_ID.to_string())
}

pub fn torpedo_tube_fore_starboard_system_id() -> SystemId {
    SystemId(TORPEDO_TUBE_FORE_STARBOARD_SYSTEM_ID.to_string())
}

pub fn torpedo_tube_aft_system_id() -> SystemId {
    SystemId(TORPEDO_TUBE_AFT_SYSTEM_ID.to_string())
}

pub fn torpedo_magazine_system_id() -> SystemId {
    SystemId(TORPEDO_MAGAZINE_SYSTEM_ID.to_string())
}

// ── Fine Power system id helpers (issue #513) ─────────────────────────────────

pub fn power_reactor_system_id() -> SystemId {
    SystemId(POWER_REACTOR_SYSTEM_ID.to_string())
}

pub fn power_battery_system_id() -> SystemId {
    SystemId(POWER_BATTERY_SYSTEM_ID.to_string())
}

/// Resolve the `SystemId` for a phaser bank by its TOML `id`.
///
/// The convention is `"phaser-<bank_id>"`, so `"fore"` → `"phaser-fore"`,
/// `"aft"` → `"phaser-aft"`, `"port"` → `"phaser-port"`, etc. Returns
/// `Some` for every non-empty bank id — callers should combine with
/// `system_is_registered` on the ship's `ControlSourceResolver` to
/// distinguish "the fine system is offline" from "the ship never declared
/// a fine system for this bank" (NPC path).
pub fn phaser_bank_system_id(bank_id: &str) -> Option<SystemId> {
    if bank_id.is_empty() {
        return None;
    }
    Some(SystemId(format!("phaser-{bank_id}")))
}

/// Resolve the `SystemId` for a blaster bank by its TOML `id` (issue #631).
///
/// The convention is `"blaster-<bank_id>"`, so `"fore"` → `"blaster-fore"`,
/// `"aft"` → `"blaster-aft"`, etc. Returns `Some` for every non-empty bank id.
/// Underscore-to-hyphen conversion follows the project convention.
pub fn blaster_bank_system_id(bank_id: &str) -> Option<SystemId> {
    if bank_id.is_empty() {
        return None;
    }
    Some(SystemId(format!("blaster-{}", bank_id.replace('_', "-"))))
}

/// Resolve the `SystemId` for a torpedo tube by its TOML `id`.
///
/// The convention is `"torpedo-tube-<tube_id_with_underscores_to_hyphens>"`,
/// so `"fore_port"` → `"torpedo-tube-fore-port"`. Returns `Some` for every
/// non-empty tube id.
pub fn torpedo_tube_system_id(tube_id: &str) -> Option<SystemId> {
    if tube_id.is_empty() {
        return None;
    }
    Some(SystemId(format!(
        "torpedo-tube-{}",
        tube_id.replace('_', "-")
    )))
}

/// Resolve the `SystemId` for a shield arc by its TOML `id`.
///
/// The convention is `"shield-arc-<arc_id_with_underscores_to_hyphens>"`,
/// so `"fore"` → `"shield-arc-fore"`, `"all"` → `"shield-arc-all"`.
/// Returns `Some` for every non-empty arc id. NPCs that declare a single
/// omni arc (id = "all") get their own fine SystemId without needing a
/// match-arm update.
pub fn shield_arc_system_id(arc_id: &str) -> Option<SystemId> {
    if arc_id.is_empty() {
        return None;
    }
    Some(SystemId(format!("shield-arc-{}", arc_id.replace('_', "-"))))
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Stable id string values ───────────────────────────────────────────────
    // These tests pin the naming convention so a rename of a constant breaks CI
    // rather than silently drifting the wire format.

    #[test]
    fn coarse_system_ids_are_lowercase_kebab() {
        let ids = [
            RED_ALERT_SYSTEM_ID,
            HELM_SYSTEM_ID,
            TACTICAL_SYSTEM_ID,
            POWER_SYSTEM_ID,
            SENSORS_SYSTEM_ID,
            NAVIGATION_SYSTEM_ID,
            SHIELDS_SYSTEM_ID,
            COMMS_SYSTEM_ID,
            CAPTAIN_SYSTEM_ID,
            VIEWSCREEN_SYSTEM_ID,
            REPAIR_SYSTEM_ID,
        ];
        for id in ids {
            assert_eq!(
                id,
                id.to_lowercase(),
                "SystemId constant {id:?} is not lowercase"
            );
            assert!(
                !id.contains('_'),
                "SystemId constant {id:?} contains underscore (use hyphen)"
            );
            assert!(!id.is_empty(), "SystemId constant must not be empty");
        }
    }

    #[test]
    fn coarse_system_id_values_are_stable() {
        assert_eq!(RED_ALERT_SYSTEM_ID, "red-alert");
        assert_eq!(HELM_SYSTEM_ID, "helm");
        assert_eq!(TACTICAL_SYSTEM_ID, "tactical");
        assert_eq!(POWER_SYSTEM_ID, "power");
        assert_eq!(SENSORS_SYSTEM_ID, "sensors");
        assert_eq!(NAVIGATION_SYSTEM_ID, "navigation");
        assert_eq!(SHIELDS_SYSTEM_ID, "shields");
        assert_eq!(COMMS_SYSTEM_ID, "comms");
        assert_eq!(CAPTAIN_SYSTEM_ID, "captain");
        assert_eq!(VIEWSCREEN_SYSTEM_ID, "viewscreen");
        assert_eq!(REPAIR_SYSTEM_ID, "repair");
    }

    #[test]
    fn system_id_helpers_return_expected_values() {
        assert_eq!(red_alert_system_id().0, RED_ALERT_SYSTEM_ID);
        assert_eq!(helm_system_id().0, HELM_SYSTEM_ID);
        assert_eq!(tactical_system_id().0, TACTICAL_SYSTEM_ID);
        // No `power_system_id()` helper — see note above the tactical helper.
        // The coarse constant is still pinned by `coarse_system_id_values_are_stable`.
        assert_eq!(sensors_system_id().0, SENSORS_SYSTEM_ID);
        assert_eq!(navigation_system_id().0, NAVIGATION_SYSTEM_ID);
        assert_eq!(shields_system_id().0, SHIELDS_SYSTEM_ID);
        assert_eq!(comms_system_id().0, COMMS_SYSTEM_ID);
        assert_eq!(captain_system_id().0, CAPTAIN_SYSTEM_ID);
        assert_eq!(viewscreen_system_id().0, VIEWSCREEN_SYSTEM_ID);
        assert_eq!(repair_system_id().0, REPAIR_SYSTEM_ID);
    }

    // ── Registry API ─────────────────────────────────────────────────────────

    #[test]
    fn registering_kind_requires_ai_controller_argument() {
        let mut registry = SystemKindRegistry::new();
        let ai = AiControllerRegistration::new("red_alert_ai").unwrap();

        registry.register("red_alert", ai).unwrap();

        assert!(registry.contains("red_alert"));
        assert_eq!(
            registry
                .registration("red_alert")
                .unwrap()
                .ai_controller
                .name(),
            "red_alert_ai"
        );
    }

    #[test]
    fn rejects_empty_ai_controller_name() {
        assert_eq!(
            AiControllerRegistration::new(""),
            Err(SystemRegistryError::EmptyAiControllerName)
        );
    }

    #[test]
    fn rejects_duplicate_kind() {
        let mut registry = SystemKindRegistry::new();
        registry
            .register(
                "red_alert",
                AiControllerRegistration::new("red_alert_ai").unwrap(),
            )
            .unwrap();

        assert_eq!(
            registry.register(
                "red_alert",
                AiControllerRegistration::new("other_ai").unwrap(),
            ),
            Err(SystemRegistryError::DuplicateKind {
                kind: "red_alert".into()
            })
        );
    }

    #[test]
    fn red_alert_registry_has_required_ai_controller() {
        let registry = SystemKindRegistry::with_red_alert().unwrap();

        assert!(registry.contains(RED_ALERT_KIND));
        assert_eq!(
            registry
                .registration(RED_ALERT_KIND)
                .unwrap()
                .ai_controller
                .name(),
            RED_ALERT_AI_CONTROLLER
        );
    }

    #[test]
    fn core_registry_has_helm_ai_controller() {
        let registry = SystemKindRegistry::with_core_systems().unwrap();

        assert!(registry.contains(HELM_KIND));
        assert_eq!(
            registry
                .registration(HELM_KIND)
                .unwrap()
                .ai_controller
                .name(),
            HELM_AI_CONTROLLER
        );
    }

    #[test]
    fn core_registry_has_tactical_ai_controller() {
        let registry = SystemKindRegistry::with_core_systems().unwrap();

        assert!(registry.contains(TACTICAL_KIND));
        assert_eq!(
            registry
                .registration(TACTICAL_KIND)
                .unwrap()
                .ai_controller
                .name(),
            TACTICAL_AI_CONTROLLER
        );
    }

    #[test]
    fn core_registry_has_power_ai_controller() {
        let registry = SystemKindRegistry::with_core_systems().unwrap();

        assert!(registry.contains(POWER_KIND));
        assert_eq!(
            registry
                .registration(POWER_KIND)
                .unwrap()
                .ai_controller
                .name(),
            POWER_AI_CONTROLLER
        );
    }

    #[test]
    fn core_registry_has_sensors_ai_controller() {
        let registry = SystemKindRegistry::with_core_systems().unwrap();

        assert!(registry.contains(SENSORS_KIND));
        assert_eq!(
            registry
                .registration(SENSORS_KIND)
                .unwrap()
                .ai_controller
                .name(),
            SENSORS_AI_CONTROLLER
        );
    }

    #[test]
    fn core_registry_has_navigation_ai_controller() {
        let registry = SystemKindRegistry::with_core_systems().unwrap();

        assert!(registry.contains(NAVIGATION_KIND));
        assert_eq!(
            registry
                .registration(NAVIGATION_KIND)
                .unwrap()
                .ai_controller
                .name(),
            NAVIGATION_AI_CONTROLLER
        );
    }

    #[test]
    fn core_registry_has_shields_ai_controller() {
        let registry = SystemKindRegistry::with_core_systems().unwrap();

        assert!(registry.contains(SHIELDS_KIND));
        assert_eq!(
            registry
                .registration(SHIELDS_KIND)
                .unwrap()
                .ai_controller
                .name(),
            SHIELDS_AI_CONTROLLER
        );
    }

    #[test]
    fn core_registry_has_comms_ai_controller() {
        let registry = SystemKindRegistry::with_core_systems().unwrap();

        assert!(registry.contains(COMMS_KIND));
        assert_eq!(
            registry
                .registration(COMMS_KIND)
                .unwrap()
                .ai_controller
                .name(),
            COMMS_AI_CONTROLLER
        );
    }

    #[test]
    fn core_registry_has_captain_ai_controller() {
        let registry = SystemKindRegistry::with_core_systems().unwrap();

        assert!(registry.contains(CAPTAIN_KIND));
        assert_eq!(
            registry
                .registration(CAPTAIN_KIND)
                .unwrap()
                .ai_controller
                .name(),
            CAPTAIN_AI_CONTROLLER
        );
    }

    #[test]
    fn core_registry_has_viewscreen_ai_controller() {
        let registry = SystemKindRegistry::with_core_systems().unwrap();

        assert!(registry.contains(VIEWSCREEN_KIND));
        assert_eq!(
            registry
                .registration(VIEWSCREEN_KIND)
                .unwrap()
                .ai_controller
                .name(),
            VIEWSCREEN_AI_CONTROLLER
        );
    }

    #[test]
    fn core_registry_has_repair_ai_controller() {
        let registry = SystemKindRegistry::with_core_systems().unwrap();

        assert!(registry.contains(REPAIR_KIND));
        assert_eq!(
            registry
                .registration(REPAIR_KIND)
                .unwrap()
                .ai_controller
                .name(),
            REPAIR_AI_CONTROLLER
        );
    }

    #[test]
    fn register_revalidates_ai_controller() {
        let mut registry = SystemKindRegistry::new();

        assert_eq!(
            registry.register(
                "red_alert",
                AiControllerRegistration {
                    name: String::new()
                },
            ),
            Err(SystemRegistryError::EmptyAiControllerName)
        );
    }

    // ── Fine Helm system tests (issue #511) ───────────────────────────────────

    #[test]
    fn fine_helm_kinds_are_registered() {
        let registry = SystemKindRegistry::with_core_systems().unwrap();

        assert!(
            registry.contains(HELM_JOYSTICK_KIND),
            "helm_joystick not registered"
        );
        assert!(
            registry.contains(HELM_ENGINE_KIND),
            "helm_engine not registered"
        );
        assert!(
            registry.contains(HELM_RADAR_KIND),
            "helm_radar not registered"
        );
        assert!(
            registry.contains(HELM_IMPULSE_KIND),
            "helm_impulse not registered"
        );

        assert_eq!(
            registry
                .registration(HELM_JOYSTICK_KIND)
                .unwrap()
                .ai_controller
                .name(),
            HELM_JOYSTICK_AI_CONTROLLER
        );
        assert_eq!(
            registry
                .registration(HELM_ENGINE_KIND)
                .unwrap()
                .ai_controller
                .name(),
            HELM_ENGINE_AI_CONTROLLER
        );
        assert_eq!(
            registry
                .registration(HELM_RADAR_KIND)
                .unwrap()
                .ai_controller
                .name(),
            HELM_RADAR_AI_CONTROLLER
        );
        assert_eq!(
            registry
                .registration(HELM_IMPULSE_KIND)
                .unwrap()
                .ai_controller
                .name(),
            HELM_IMPULSE_AI_CONTROLLER
        );
    }

    #[test]
    fn fine_helm_system_ids_are_lowercase_kebab() {
        let ids = [
            HELM_JOYSTICK_SYSTEM_ID,
            HELM_ENGINE_PORT_SYSTEM_ID,
            HELM_ENGINE_STARBOARD_SYSTEM_ID,
            HELM_RADAR_SYSTEM_ID,
            HELM_IMPULSE_SYSTEM_ID,
        ];
        for id in ids {
            assert_eq!(
                id,
                id.to_lowercase(),
                "Fine helm SystemId {id:?} is not lowercase"
            );
            assert!(
                !id.contains('_'),
                "Fine helm SystemId {id:?} contains underscore (use hyphen)"
            );
            assert!(!id.is_empty(), "Fine helm SystemId must not be empty");
        }
        assert_eq!(HELM_JOYSTICK_SYSTEM_ID, "helm-joystick");
        assert_eq!(HELM_ENGINE_PORT_SYSTEM_ID, "helm-engine-port");
        assert_eq!(HELM_ENGINE_STARBOARD_SYSTEM_ID, "helm-engine-starboard");
        assert_eq!(HELM_RADAR_SYSTEM_ID, "helm-radar");
        assert_eq!(HELM_IMPULSE_SYSTEM_ID, "helm-impulse");
    }

    #[test]
    fn fine_helm_system_id_helpers_return_expected_values() {
        assert_eq!(helm_joystick_system_id().0, HELM_JOYSTICK_SYSTEM_ID);
        assert_eq!(helm_engine_port_system_id().0, HELM_ENGINE_PORT_SYSTEM_ID);
        assert_eq!(
            helm_engine_starboard_system_id().0,
            HELM_ENGINE_STARBOARD_SYSTEM_ID
        );
        assert_eq!(helm_radar_system_id().0, HELM_RADAR_SYSTEM_ID);
        assert_eq!(helm_impulse_system_id().0, HELM_IMPULSE_SYSTEM_ID);
    }

    // ── Fine Tactical system tests (issue #512) ───────────────────────────────

    #[test]
    fn fine_tactical_kinds_are_registered() {
        let registry = SystemKindRegistry::with_core_systems().unwrap();

        assert!(
            registry.contains(PHASER_BANK_KIND),
            "phaser_bank not registered"
        );
        assert!(
            registry.contains(TORPEDO_TUBE_KIND),
            "torpedo_tube not registered"
        );
        assert!(
            registry.contains(TORPEDO_MAGAZINE_KIND),
            "torpedo_magazine not registered"
        );

        assert_eq!(
            registry
                .registration(PHASER_BANK_KIND)
                .unwrap()
                .ai_controller
                .name(),
            PHASER_BANK_AI_CONTROLLER
        );
        assert_eq!(
            registry
                .registration(TORPEDO_TUBE_KIND)
                .unwrap()
                .ai_controller
                .name(),
            TORPEDO_TUBE_AI_CONTROLLER
        );
        assert_eq!(
            registry
                .registration(TORPEDO_MAGAZINE_KIND)
                .unwrap()
                .ai_controller
                .name(),
            TORPEDO_MAGAZINE_AI_CONTROLLER
        );
    }

    #[test]
    fn fine_tactical_system_ids_are_lowercase_kebab() {
        let ids = [
            PHASER_FORE_SYSTEM_ID,
            PHASER_AFT_SYSTEM_ID,
            TORPEDO_TUBE_FORE_PORT_SYSTEM_ID,
            TORPEDO_TUBE_FORE_STARBOARD_SYSTEM_ID,
            TORPEDO_TUBE_AFT_SYSTEM_ID,
            TORPEDO_MAGAZINE_SYSTEM_ID,
        ];
        for id in ids {
            assert_eq!(
                id,
                id.to_lowercase(),
                "Fine tactical SystemId {id:?} is not lowercase"
            );
            assert!(
                !id.contains('_'),
                "Fine tactical SystemId {id:?} contains underscore (use hyphen)"
            );
            assert!(!id.is_empty(), "Fine tactical SystemId must not be empty");
        }
        assert_eq!(PHASER_FORE_SYSTEM_ID, "phaser-fore");
        assert_eq!(PHASER_AFT_SYSTEM_ID, "phaser-aft");
        assert_eq!(TORPEDO_TUBE_FORE_PORT_SYSTEM_ID, "torpedo-tube-fore-port");
        assert_eq!(
            TORPEDO_TUBE_FORE_STARBOARD_SYSTEM_ID,
            "torpedo-tube-fore-starboard"
        );
        assert_eq!(TORPEDO_TUBE_AFT_SYSTEM_ID, "torpedo-tube-aft");
        assert_eq!(TORPEDO_MAGAZINE_SYSTEM_ID, "torpedo-magazine");
    }

    #[test]
    fn fine_tactical_system_id_helpers_return_expected_values() {
        assert_eq!(phaser_fore_system_id().0, PHASER_FORE_SYSTEM_ID);
        assert_eq!(phaser_aft_system_id().0, PHASER_AFT_SYSTEM_ID);
        assert_eq!(
            torpedo_tube_fore_port_system_id().0,
            TORPEDO_TUBE_FORE_PORT_SYSTEM_ID
        );
        assert_eq!(
            torpedo_tube_fore_starboard_system_id().0,
            TORPEDO_TUBE_FORE_STARBOARD_SYSTEM_ID
        );
        assert_eq!(torpedo_tube_aft_system_id().0, TORPEDO_TUBE_AFT_SYSTEM_ID);
        assert_eq!(torpedo_magazine_system_id().0, TORPEDO_MAGAZINE_SYSTEM_ID);
    }

    #[test]
    fn phaser_bank_system_id_resolves_known_ids() {
        assert_eq!(phaser_bank_system_id("fore"), Some(phaser_fore_system_id()));
        assert_eq!(phaser_bank_system_id("aft"), Some(phaser_aft_system_id()));
        // Derives arbitrary bank ids via the `phaser-<id>` naming convention;
        // NPC ships that declare e.g. "port"/"starboard" get their own fine
        // SystemIds without needing a match-arm update.
        assert_eq!(
            phaser_bank_system_id("port"),
            Some(SystemId("phaser-port".into()))
        );
        assert_eq!(
            phaser_bank_system_id("starboard"),
            Some(SystemId("phaser-starboard".into()))
        );
        assert_eq!(phaser_bank_system_id(""), None);
    }

    #[test]
    fn torpedo_tube_system_id_resolves_known_ids() {
        assert_eq!(
            torpedo_tube_system_id("fore_port"),
            Some(torpedo_tube_fore_port_system_id())
        );
        assert_eq!(
            torpedo_tube_system_id("fore_starboard"),
            Some(torpedo_tube_fore_starboard_system_id())
        );
        assert_eq!(
            torpedo_tube_system_id("aft"),
            Some(torpedo_tube_aft_system_id())
        );
        // Underscore-to-hyphen conversion is the convention.
        assert_eq!(
            torpedo_tube_system_id("dorsal_upper"),
            Some(SystemId("torpedo-tube-dorsal-upper".into()))
        );
        assert_eq!(torpedo_tube_system_id(""), None);
    }

    // ── Fine Blaster system tests (issue #631) ────────────────────────────────

    #[test]
    fn blaster_bank_kind_is_registered() {
        let registry = SystemKindRegistry::with_core_systems().unwrap();
        assert!(
            registry.contains(BLASTER_BANK_KIND),
            "blaster_bank not registered"
        );
        assert_eq!(
            registry
                .registration(BLASTER_BANK_KIND)
                .unwrap()
                .ai_controller
                .name(),
            BLASTER_BANK_AI_CONTROLLER
        );
    }

    #[test]
    fn blaster_bank_kind_constant_is_correct() {
        assert_eq!(BLASTER_BANK_KIND, "blaster_bank");
        assert_eq!(BLASTER_BANK_AI_CONTROLLER, "blaster_bank_ai");
    }

    #[test]
    fn blaster_bank_system_id_resolves_known_ids() {
        assert_eq!(
            blaster_bank_system_id("fore"),
            Some(SystemId("blaster-fore".into()))
        );
        assert_eq!(
            blaster_bank_system_id("aft"),
            Some(SystemId("blaster-aft".into()))
        );
        assert_eq!(
            blaster_bank_system_id("fore_port"),
            Some(SystemId("blaster-fore-port".into()))
        );
        assert_eq!(blaster_bank_system_id(""), None);
    }

    #[test]
    fn blaster_bank_system_ids_are_lowercase_kebab() {
        for bank_id in &["fore", "aft", "port", "starboard", "fore_port"] {
            let sid =
                blaster_bank_system_id(bank_id).expect("non-empty id should produce a SystemId");
            assert_eq!(
                sid.0,
                sid.0.to_lowercase(),
                "SystemId {sid:?} for blaster bank {bank_id:?} is not lowercase"
            );
            assert!(
                !sid.0.contains('_'),
                "SystemId {sid:?} for blaster bank {bank_id:?} contains underscore"
            );
            assert!(
                sid.0.starts_with("blaster-"),
                "SystemId {sid:?} must start with blaster-"
            );
        }
    }

    // ── Fine Tactical/Sensor Radar system tests ───────────────────────────────

    #[test]
    fn radar_kinds_are_registered() {
        let registry = SystemKindRegistry::with_core_systems().unwrap();

        assert!(
            registry.contains(TACTICAL_RADAR_KIND),
            "tactical_radar not registered"
        );
        assert!(
            registry.contains(SENSOR_RADAR_KIND),
            "sensor_radar not registered"
        );

        assert_eq!(
            registry
                .registration(TACTICAL_RADAR_KIND)
                .unwrap()
                .ai_controller
                .name(),
            TACTICAL_RADAR_AI_CONTROLLER
        );
        assert_eq!(
            registry
                .registration(SENSOR_RADAR_KIND)
                .unwrap()
                .ai_controller
                .name(),
            SENSOR_RADAR_AI_CONTROLLER
        );
    }

    #[test]
    fn radar_system_ids_are_lowercase_kebab() {
        let ids = [TACTICAL_RADAR_SYSTEM_ID, SENSOR_RADAR_SYSTEM_ID];
        for id in ids {
            assert_eq!(
                id,
                id.to_lowercase(),
                "Radar SystemId {id:?} is not lowercase"
            );
            assert!(
                !id.contains('_'),
                "Radar SystemId {id:?} contains underscore (use hyphen)"
            );
            assert!(!id.is_empty(), "Radar SystemId must not be empty");
        }
        assert_eq!(TACTICAL_RADAR_SYSTEM_ID, "tactical-radar");
        assert_eq!(SENSOR_RADAR_SYSTEM_ID, "sensor-radar");
    }

    #[test]
    fn radar_system_id_helpers_return_expected_values() {
        assert_eq!(tactical_radar_system_id().0, TACTICAL_RADAR_SYSTEM_ID);
        assert_eq!(sensor_radar_system_id().0, SENSOR_RADAR_SYSTEM_ID);
    }

    // ── Fine Power system tests (issue #513) ──────────────────────────────────

    #[test]
    fn fine_power_kinds_are_registered() {
        let registry = SystemKindRegistry::with_core_systems().unwrap();

        assert!(
            registry.contains(POWER_REACTOR_KIND),
            "power_reactor not registered"
        );
        assert!(
            registry.contains(POWER_BATTERY_KIND),
            "power_battery not registered"
        );

        assert_eq!(
            registry
                .registration(POWER_REACTOR_KIND)
                .unwrap()
                .ai_controller
                .name(),
            POWER_REACTOR_AI_CONTROLLER
        );
        assert_eq!(
            registry
                .registration(POWER_BATTERY_KIND)
                .unwrap()
                .ai_controller
                .name(),
            POWER_BATTERY_AI_CONTROLLER
        );
    }

    #[test]
    fn fine_power_system_ids_are_lowercase_kebab() {
        let ids = [POWER_REACTOR_SYSTEM_ID, POWER_BATTERY_SYSTEM_ID];
        for id in ids {
            assert_eq!(
                id,
                id.to_lowercase(),
                "Fine power SystemId {id:?} is not lowercase"
            );
            assert!(
                !id.contains('_'),
                "Fine power SystemId {id:?} contains underscore (use hyphen)"
            );
            assert!(!id.is_empty(), "Fine power SystemId must not be empty");
        }
        assert_eq!(POWER_REACTOR_SYSTEM_ID, "power-reactor");
        assert_eq!(POWER_BATTERY_SYSTEM_ID, "power-battery");
    }

    #[test]
    fn fine_power_system_id_helpers_return_expected_values() {
        assert_eq!(power_reactor_system_id().0, POWER_REACTOR_SYSTEM_ID);
        assert_eq!(power_battery_system_id().0, POWER_BATTERY_SYSTEM_ID);
    }

    // ── Fine Shields system tests (issue #514) ────────────────────────────────

    #[test]
    fn shield_arcs_registered_in_system_kind_registry() {
        let registry = SystemKindRegistry::with_core_systems().unwrap();
        assert!(
            registry.contains(SHIELD_ARC_KIND),
            "shield_arc not registered"
        );
        assert_eq!(
            registry
                .registration(SHIELD_ARC_KIND)
                .unwrap()
                .ai_controller
                .name(),
            SHIELD_ARC_AI_CONTROLLER
        );
    }

    #[test]
    fn shield_arc_kind_string_is_lowercase_snake() {
        // Kind key uses snake_case per registry convention (matches
        // `phaser_bank`, `power_reactor` etc.).
        assert_eq!(SHIELD_ARC_KIND, "shield_arc");
    }

    #[test]
    fn shield_arc_system_id_helper_returns_expected_shape() {
        assert_eq!(
            shield_arc_system_id("fore"),
            Some(SystemId("shield-arc-fore".into()))
        );
        assert_eq!(
            shield_arc_system_id("port"),
            Some(SystemId("shield-arc-port".into()))
        );
        assert_eq!(
            shield_arc_system_id("aft"),
            Some(SystemId("shield-arc-aft".into()))
        );
        assert_eq!(
            shield_arc_system_id("starboard"),
            Some(SystemId("shield-arc-starboard".into()))
        );
        // Single-omni NPC arc id
        assert_eq!(
            shield_arc_system_id("all"),
            Some(SystemId("shield-arc-all".into()))
        );
        // Underscore-to-hyphen conversion is the convention.
        assert_eq!(
            shield_arc_system_id("dorsal_upper"),
            Some(SystemId("shield-arc-dorsal-upper".into()))
        );
        assert_eq!(shield_arc_system_id(""), None);
    }

    #[test]
    fn shield_arc_kebab_case_conformance() {
        // Every SystemId synthesised for shield arcs must be lowercase kebab.
        for arc_id in &["fore", "port", "aft", "starboard", "all", "dorsal_upper"] {
            let sid = shield_arc_system_id(arc_id).expect("non-empty id should synthesise");
            assert_eq!(
                sid.0,
                sid.0.to_lowercase(),
                "SystemId {sid:?} for arc {arc_id:?} is not lowercase"
            );
            assert!(
                !sid.0.contains('_'),
                "SystemId {sid:?} for arc {arc_id:?} contains underscore (use hyphen)"
            );
            assert!(
                sid.0.starts_with("shield-arc-"),
                "SystemId {sid:?} must start with shield-arc-"
            );
        }
    }
}
