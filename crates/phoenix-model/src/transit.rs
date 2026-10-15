//! Closed accepted LocalJump data. Physical work and phase ownership stay in Gameplay.
use serde::{Deserialize, Serialize};

/// Technical retention bound, never an evicting gameplay message capacity.
pub const MAX_HANDLED_TRANSIT_OBJECTIVES: usize = 128;

fn identity(value: &str) -> bool {
    !value.trim().is_empty() && value.len() <= 512 && !value.chars().any(char::is_control)
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TransitRequestKey {
    pub objective_id: String,
    pub revision: u64,
}
impl TransitRequestKey {
    pub fn validate(&self) -> Result<(), String> {
        if !identity(&self.objective_id) {
            return Err("transit request requires a bounded actual objective identity".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TransitFuel {
    /// Actual Logistics Local store bound to this drive's System.
    pub store: String,
    pub amount: i64,
}
impl TransitFuel {
    pub fn validate(&self) -> Result<(), String> {
        if !identity(&self.store) || self.amount <= 0 {
            return Err(
                "transit fuel requires an actual store and positive finite quantity".into(),
            );
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LocalJumpConfig {
    pub preparation_work_secs: f32,
    pub max_distance: f32,
    pub scatter_radius: f32,
    pub fuel: TransitFuel,
    /// Existing per-System text reading; zero is immediate compatibility tuning.
    #[serde(default)]
    pub reading_ticks: u32,
}
impl LocalJumpConfig {
    pub fn validate(&self) -> Result<(), String> {
        if !self.preparation_work_secs.is_finite()
            || self.preparation_work_secs <= 0.0
            || !self.max_distance.is_finite()
            || self.max_distance <= 0.0
            || !self.scatter_radius.is_finite()
            || self.scatter_radius < 0.0
        {
            return Err(
                "LocalJump requires finite positive work/range and nonnegative scatter".into(),
            );
        }
        self.fuel.validate()
    }
}

/// Actual costs frozen with the accepted profile, not a second live power owner.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AcceptedLocalJumpProfile {
    pub config: LocalJumpConfig,
    pub preparation_energy_per_sec: f32,
    pub commit_energy: f32,
}
impl AcceptedLocalJumpProfile {
    pub fn validate(&self) -> Result<(), String> {
        self.config.validate()?;
        if !self.preparation_energy_per_sec.is_finite()
            || self.preparation_energy_per_sec <= 0.0
            || !self.commit_energy.is_finite()
            || self.commit_energy <= 0.0
        {
            return Err(
                "accepted transit work requires actual finite positive energy costs".into(),
            );
        }
        Ok(())
    }
}

/// The accepted own course; point is read from the permitted Navigation board.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum TransitDestination {
    Local {
        point: [f32; 3],
        course_generation: u64,
    },
}
impl TransitDestination {
    pub fn point(&self) -> [f32; 3] {
        match self {
            Self::Local { point, .. } => *point,
        }
    }
    pub fn validate(&self) -> Result<(), String> {
        if self.point().into_iter().any(|value| !value.is_finite()) {
            return Err("transit course requires finite representable geometry".into());
        }
        Ok(())
    }
}

/// Explicit physical arrival policy. Visual size and mass do not grant gravity.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArrivalGravityConfig {
    pub exclusion_radius: f32,
}
impl ArrivalGravityConfig {
    pub fn validate(&self) -> Result<(), String> {
        if !self.exclusion_radius.is_finite() || self.exclusion_radius <= 0.0 {
            return Err("arrival gravity requires a finite positive displacement radius".into());
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "transit_tests.rs"]
mod tests;
