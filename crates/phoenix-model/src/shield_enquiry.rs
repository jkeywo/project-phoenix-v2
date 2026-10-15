//! Supported shield response and immutable, paid focused findings.
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShieldFrequencyResponse {
    pub mismatch_loss: f32,
}
impl Default for ShieldFrequencyResponse {
    fn default() -> Self {
        Self { mismatch_loss: 0.5 }
    }
}
impl ShieldFrequencyResponse {
    pub fn validate(&self) -> Result<(), String> {
        if !self.mismatch_loss.is_finite() || !(0.0..=1.0).contains(&self.mismatch_loss) {
            return Err("shield mismatch loss must be finite in [0,1]".into());
        }
        Ok(())
    }
    pub fn multiplier(&self, emitter: f32, actual: f32) -> f32 {
        1.0 - self.mismatch_loss * (emitter - actual).abs().min(1.0)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShieldFrequencyCycle {
    pub frequencies: Vec<f32>,
    pub period_secs: f32,
}
impl ShieldFrequencyCycle {
    pub fn validate(&self) -> Result<(), String> {
        if self.frequencies.len() < 2
            || self.frequencies.len() > 64
            || !self.period_secs.is_finite()
            || self.period_secs <= 0.0
            || std::time::Duration::try_from_secs_f32(self.period_secs)
                .map_or(true, |duration| duration.is_zero())
            || self
                .frequencies
                .iter()
                .any(|v| !v.is_finite() || !(0.0..=1.0).contains(v))
            || self.frequencies.windows(2).any(|v| v[0] == v[1])
        {
            return Err(
                "shield cycle requires 2..64 supported frequencies and finite positive period"
                    .into(),
            );
        }
        Ok(())
    }
    pub fn frequency_at(
        &self,
        anchor: u64,
        tick: u64,
        fixed_period: std::time::Duration,
    ) -> Result<f32, String> {
        self.validate()?;
        if anchor > tick || fixed_period.is_zero() {
            return Err("invalid shield cycle clock".into());
        }
        let period = std::time::Duration::try_from_secs_f32(self.period_secs)
            .map_err(|_| "unrepresentable shield cycle period")?
            .as_nanos();
        // Duration's u64-second range times 64 fits u128. Reduce the product
        // before division so accepted u64 tick frontiers lose no low bits.
        let modulus = period
            .checked_mul(self.frequencies.len() as u128)
            .ok_or("shield cycle period overflow")?;
        let mut ticks = u128::from(tick - anchor);
        let mut step = fixed_period.as_nanos() % modulus;
        let mut age = 0u128;
        while ticks > 0 {
            if ticks & 1 != 0 {
                age = (age + step) % modulus;
            }
            ticks >>= 1;
            step = (step + step) % modulus;
        }
        Ok(self.frequencies[(age / period) as usize])
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShieldFinding {
    /// Fingerprint of the acquired configuration, not a future rotation schedule.
    pub configuration: String,
    pub frequency: f32,
    pub frequency_step: f32,
    pub mismatch_loss: f32,
}
impl ShieldFinding {
    pub fn acquire(frequency: f32, response: ShieldFrequencyResponse, step: f64) -> Option<Self> {
        response.validate().ok()?;
        let precision = step as f32;
        if !frequency.is_finite()
            || !(0.0..=1.0).contains(&frequency)
            || !precision.is_finite()
            || precision <= 0.0
            || precision > 1.0
        {
            return None;
        }
        let sampled = ((f64::from(frequency) / step).round() * step).clamp(0.0, 1.0) as f32;
        let finding = Self {
            configuration: Self::fingerprint(sampled, response.mismatch_loss),
            frequency: sampled,
            frequency_step: precision,
            mismatch_loss: response.mismatch_loss,
        };
        finding.validate().ok()?;
        Some(finding)
    }
    fn fingerprint(frequency: f32, loss: f32) -> String {
        let mut hash = 0xcbf29ce484222325u64;
        for byte in frequency
            .to_bits()
            .to_le_bytes()
            .into_iter()
            .chain(loss.to_bits().to_le_bytes())
        {
            hash = (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3);
        }
        format!("{hash:016x}")
    }
    pub fn validate(&self) -> Result<(), String> {
        if !self.frequency.is_finite()
            || !(0.0..=1.0).contains(&self.frequency)
            || !self.frequency_step.is_finite()
            || self.frequency_step <= 0.0
            || self.frequency_step > 1.0
            || self.configuration != Self::fingerprint(self.frequency, self.mismatch_loss)
        {
            return Err("invalid acquired shield finding".into());
        }
        ShieldFrequencyResponse {
            mismatch_loss: self.mismatch_loss,
        }
        .validate()
    }
}

#[cfg(test)]
#[path = "shield_enquiry_tests.rs"]
mod tests;

/// False is an explicit inherited-equipment tombstone; true is not equipment.
pub fn optional_cycle<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<ShieldFrequencyCycle>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Value {
        Cycle(ShieldFrequencyCycle),
        Disabled(bool),
    }
    match Option::<Value>::deserialize(deserializer)? {
        None | Some(Value::Disabled(false)) => Ok(None),
        Some(Value::Cycle(cycle)) => Ok(Some(cycle)),
        Some(Value::Disabled(true)) => Err(serde::de::Error::custom(
            "author actual shield cycle equipment or false",
        )),
    }
}
