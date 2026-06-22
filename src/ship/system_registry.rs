use crate::messages::SystemId;
use std::collections::HashMap;

pub const RED_ALERT_SYSTEM_ID: &str = "red-alert";
pub const RED_ALERT_KIND: &str = "red_alert";
pub const RED_ALERT_AI_CONTROLLER: &str = "red_alert_ai";

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

pub fn red_alert_system_id() -> SystemId {
    SystemId(RED_ALERT_SYSTEM_ID.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
