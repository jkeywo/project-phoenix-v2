//! Authored independent sensing instruments and permitted passive projections.
use crate::identity::SystemId;
use serde::{Deserialize, Serialize};

/// A typed publication channel owned by one real focused instrument, never equipment.
pub fn scan_blackboard_key_for(owner: &SystemId) -> SystemId {
    SystemId(format!("scan/{}:{}", owner.0.len(), owner.0))
}
pub fn scan_owner_from_blackboard_key(key: &SystemId) -> Option<SystemId> {
    let (size, owner) = key.0.strip_prefix("scan/")?.split_once(':')?;
    let size = size.parse::<usize>().ok()?;
    if owner.len() != size || size == 0 || size > 128 || owner.chars().any(char::is_control) {
        return None;
    }
    let owner = SystemId(owner.into());
    (scan_blackboard_key_for(&owner) == *key).then_some(owner)
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SensingConfig {
    #[serde(default)]
    pub sharing: Vec<crate::sensing_sharing::RadarSharingConfig>,
    #[serde(default)]
    pub control: SensingControlBindings,
    #[serde(default)]
    pub focused: Vec<FocusedConfig>,
    #[serde(default)]
    pub broad: Vec<BroadConfig>,
    #[serde(default)]
    pub passive: Vec<PassiveConfig>,
}
/// Singular machinery actuators may consume any permitted instrument's picture.
/// This binding grants command ownership, never additional observations.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SensingControlBindings {
    pub combat_lock: Option<SystemId>,
    pub science_target: Option<SystemId>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FocusedConfig {
    pub system: SystemId,
    pub work_secs: f32,
    pub history_limit: u16,
    pub unfinished_retention_secs: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub forecast: Option<crate::sensing_forecast::ScanForecastPolicy>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BroadConfig {
    pub system: SystemId,
    pub sample_interval_secs: f32,
    /// Cumulative supported observation time, in nondecreasing order.
    pub identity_secs: f32,
    pub classification_secs: f32,
    pub assessment_secs: f32,
    pub position_error: f32,
    pub drift_error_per_sec: f32,
    pub track_retention_secs: f32,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SensorMountConfig {
    pub facing_deg: f32,
    pub arc_deg: f32,
    pub range: f32,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PassiveConfig {
    pub system: SystemId,
    pub mount: SensorMountConfig,
    pub sample_interval_secs: f32,
    pub min_signal_strength: f32,
    pub bearing_error_rad: f32,
}
fn nonnegative(value: f32) -> bool {
    value.is_finite() && value >= 0.0
}
fn positive(value: f32) -> bool {
    value.is_finite() && value > 0.0
}
fn system_valid(system: &SystemId) -> bool {
    !system.0.is_empty() && system.0.len() <= 128 && !system.0.chars().any(char::is_control)
}
impl SensorMountConfig {
    pub fn validate(&self) -> Result<(), String> {
        if !self.facing_deg.is_finite()
            || !positive(self.range)
            || !positive(self.arc_deg)
            || self.arc_deg > 360.0
        {
            return Err(
                "sensing mount requires finite facing, positive range and arc in (0,360]".into(),
            );
        }
        Ok(())
    }
}
impl SensingConfig {
    pub fn validate(&self) -> Result<(), String> {
        let mut sharing = std::collections::BTreeSet::new();
        for row in &self.sharing {
            if !system_valid(&row.system)
                || !sharing.insert(&row.system)
                || !self
                    .broad
                    .iter()
                    .any(|profile| profile.system == row.system)
            {
                return Err("radar sharing requires distinct actual broad owners".into());
            }
        }
        if self
            .control
            .combat_lock
            .iter()
            .chain(self.control.science_target.iter())
            .any(|id| !system_valid(id))
        {
            return Err("sensing control bindings require actual System identities".into());
        }
        let mut systems = std::collections::BTreeSet::new();
        for system in self
            .focused
            .iter()
            .map(|row| &row.system)
            .chain(self.broad.iter().map(|row| &row.system))
            .chain(self.passive.iter().map(|row| &row.system))
        {
            if !system_valid(system) || !systems.insert(system) {
                return Err("sensing instruments need distinct actual System identities".into());
            }
        }
        if systems.is_empty() {
            return Err("sensing profile must declare at least one independent instrument".into());
        }
        for config in &self.focused {
            if let Some(policy) = &config.forecast {
                policy.validate()?;
            }
            if !positive(config.work_secs)
                || config.history_limit == 0
                || !nonnegative(config.unfinished_retention_secs)
            {
                return Err("focused sensing requires positive work/history and finite nonnegative unfinished retention".into());
            }
        }
        for config in &self.broad {
            if !positive(config.sample_interval_secs)
                || ![
                    config.identity_secs,
                    config.classification_secs,
                    config.assessment_secs,
                    config.position_error,
                    config.drift_error_per_sec,
                    config.track_retention_secs,
                ]
                .into_iter()
                .all(nonnegative)
                || config.identity_secs > config.classification_secs
                || config.classification_secs > config.assessment_secs
            {
                return Err("broad sensing requires finite cadence, nonnegative error/retention and ordered assessment times".into());
            }
        }
        for config in &self.passive {
            config.mount.validate()?;
            if !positive(config.sample_interval_secs)
                || !nonnegative(config.min_signal_strength)
                || !nonnegative(config.bearing_error_rad)
                || config.bearing_error_rad > std::f32::consts::PI
            {
                return Err("passive sensing requires positive cadence, nonnegative signal threshold and bearing error in [0,PI]".into());
            }
        }
        Ok(())
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PassiveSignature {
    #[default]
    Unclassified,
    Reactor,
    Propulsion,
    Weapon,
    ActiveScan,
    Comms,
}
/// A measured bearing, not a secret association with an actual actor. Copying
/// this record preserves sample_id and observed_tick, never creates evidence.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PassiveSignal {
    pub sample_id: String,
    pub observed_tick: u64,
    pub bearing_rad: f32,
    pub bearing_error_rad: f32,
    pub strength: f32,
    pub signature: PassiveSignature,
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PassiveBlackboard {
    pub enabled: bool,
    pub can_operate: bool,
    pub mount: Option<SensorMountConfig>,
    pub last_sample_tick: Option<u64>,
    pub signals: Vec<PassiveSignal>,
}

#[cfg(test)]
#[path = "sensing_tests.rs"]
mod tests;
