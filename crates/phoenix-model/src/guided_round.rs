//! Frozen physical equipment carried by an actual guided round or burst.
//! Guidance intent, remaining state and paid work retain their runtime owners.
use crate::damage::DamageProfile;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GuidedRoundProfile {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub marking: Option<crate::marking::FrozenMarkingPayload>,
    pub speed: f32,
    pub turn_rate: f32,
    pub lifespan: f32,
    pub detonation_radius: f32,
    pub damage_hull: i32,
    pub damage_shields: i32,
    pub shield_pierce: f32,
    pub impact: DamageProfile,
    pub durability: f32,
    pub seeker: Option<crate::guided_guidance::SeekerProfile>,
    pub receiver: Option<crate::guided_guidance::GuidanceReceiverProfile>,
}

impl GuidedRoundProfile {
    /// Rust Default is a construction convenience, never an accepted profile.
    /// Authored defaults belong at the existing equipment parsing boundary.
    pub fn validate(&self) -> Result<(), String> {
        if ![self.speed, self.lifespan, self.durability]
            .into_iter()
            .all(|value| value.is_finite() && value > 0.0)
            || ![self.turn_rate, self.detonation_radius]
                .into_iter()
                .all(|value| value.is_finite() && value >= 0.0)
            || self.damage_hull < 0
            || self.damage_shields < 0
            || !self.shield_pierce.is_finite()
            || !(0.0..=1.0).contains(&self.shield_pierce)
        {
            return Err(
                "guided launch profile requires finite supported flight, payload and durability"
                    .into(),
            );
        }
        self.impact.validate()?;
        if let Some(marking) = &self.marking {
            marking.validate()?;
            if self.damage_hull != 0
                || self.damage_shields != 0
                || self.detonation_radius != 0.0
                || self.shield_pierce != 0.0
                || self.impact.penetration != 0.0
                || self.impact.emp.is_some()
            {
                return Err(
                    "mark-only rounds cannot carry damage, EMP, penetration or proximity delivery"
                        .into(),
                );
            }
        }
        if let Some(seeker) = &self.seeker {
            seeker.validate()?;
        }
        if let Some(receiver) = &self.receiver {
            receiver.validate()?;
        }
        Ok(())
    }

    /// An active vector entry is neither expired nor already intercepted.
    /// Expiry/removal is completed by the existing projectile owner in its tick.
    pub fn validate_active(
        &self,
        lifespan_remaining: f32,
        durability_remaining: f32,
    ) -> Result<(), String> {
        self.validate()?;
        if !lifespan_remaining.is_finite()
            || lifespan_remaining <= 0.0
            || lifespan_remaining > self.lifespan
            || !durability_remaining.is_finite()
            || durability_remaining <= 0.0
            || durability_remaining > self.durability
        {
            return Err(
                "active guided round remaining life/durability must be within its frozen profile"
                    .into(),
            );
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "guided_round_tests.rs"]
mod tests;
