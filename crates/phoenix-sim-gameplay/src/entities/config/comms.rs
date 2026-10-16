//! Entity schema: comms. Public paths remain in the parent module.
use super::*;

/// Config block for an entity's comms reachability.
///
/// Loaded from `[comms]` in entity TOMLs. When present, the entity is
/// reachable by the player's Comms console while inside `range` units of the
/// player ship. The player ship's own `[comms].range` defines how far it can
/// listen. Effective range between two entities is `min(a.range, b.range)`.
///
/// `range` alone says only "this endpoint is range-gated" — it does NOT put the
/// entity on the hail roster. `hailable = true` does (issue #985). The split is
/// deliberate: every shipped warship and station declares a range, so deriving
/// the roster from `range` alone would make every enemy wave ship in
/// `combat_test` hailable. See [`crate::comms::roster`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommsConfig {
    /// Comms range in world units.
    pub range: f32,
    /// Opt in to the hail roster (issue #985). `false` (the default) means the
    /// entity is a range-gated comms endpoint but not something the Comms
    /// officer can call up; the world's `[[comms]]` templates remain the only
    /// thing that puts it on the roster. Set `true` and the live entity is
    /// unioned into the roster for as long as it exists.
    #[serde(default)]
    pub hailable: bool,
    /// Optional player-facing label for the contact row, independent of the
    /// entity's `name` reference id (mirrors `[[comms]] display_name`, issue
    /// #751). `None` falls back to the entity's `EntityName`, then its UUID.
    /// Only meaningful alongside `hailable = true`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    /// Explicit supported radio channels. Absence retains legacy plot hails.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub radio: Option<phoenix_model::comms_channel::CommsRadioConfig>,
}

pub(super) fn validate_radio(config: &EntityConfig) -> Result<(), String> {
    let Some(comms) = &config.comms else {
        return Ok(());
    };
    let Some(radio) = &comms.radio else {
        return Ok(());
    };
    radio.validate(comms.range)?;
    let ship = config
        .ship_config
        .as_ref()
        .ok_or("radio channels require actual System ownership")?;
    // Preserve the existing Comms consumer's single canonical System boundary.
    if radio.system.0 != crate::ship::system_registry::COMMS_SYSTEM_ID
        || ship
            .system(&radio.system)
            .is_none_or(|system| system.kind != crate::ship::system_registry::COMMS_KIND)
        || ship.console_for_system(&radio.system).is_none()
        || config.energy.as_ref().is_none_or(|energy| {
            !energy.consumers.iter().any(|consumer| {
                consumer.system == radio.system
                    && consumer.continuous_cost_per_sec > 0.0
                    && consumer.discrete_cost > 0.0
            })
        })
    {
        return Err("radio channels require the installed Comms System, its controlling Console and real energy consumer".into());
    }
    if let Some(alert) = radio
        .traffic
        .as_ref()
        .and_then(|traffic| traffic.alert.as_ref())
    {
        let permitted = ship
            .console_for_system(&radio.system)
            .is_some_and(|console| {
                console.feeds.iter().any(|feed| {
                    feed.source == alert.source
                        && feed.kind == phoenix_model::console::ConsoleObservationKind::Captain
                })
            });
        let source = ship
            .system(&alert.source)
            .is_some_and(|system| system.kind == crate::ship::system_registry::CAPTAIN_KIND);
        let alerts = ship
            .systems
            .iter()
            .filter(|system| system.kind == crate::ship::system_registry::RED_ALERT_KIND)
            .count();
        let publishers = ship
            .systems
            .iter()
            .filter(|system| system.kind == crate::ship::system_registry::CAPTAIN_KIND)
            .count();
        if !permitted || !source || alerts != 1 || publishers != 1 {
            return Err("contextual traffic requires its exact permitted Captain feed and one actual Alert System".into());
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "comms_tests.rs"]
mod tests;
