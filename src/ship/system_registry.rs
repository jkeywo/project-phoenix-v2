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

pub fn power_system_id() -> SystemId {
    SystemId(POWER_SYSTEM_ID.to_string())
}

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

pub fn helm_impulse_system_id() -> SystemId {
    SystemId(HELM_IMPULSE_SYSTEM_ID.to_string())
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
        assert_eq!(power_system_id().0, POWER_SYSTEM_ID);
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
}
