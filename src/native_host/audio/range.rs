//! Optional output dynamics. Only a current gain envelope is retained; there
//! are no delayed samples, pending sounds or simulation inputs in this stage.
use serde::Deserialize;

#[derive(Clone, Copy, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RangeSpec {
    pub threshold_db: f32,
    pub knee_db: f32,
    pub ratio: f32,
    pub attack_seconds: f32,
    pub release_seconds: f32,
    pub browser_compensation: f32,
    pub native_quiet_gain: f32,
    pub ceiling: f32,
}
impl Default for RangeSpec {
    fn default() -> Self {
        toml::from_str(include_str!("../../../assets/audio/reduced-range.toml"))
            .expect("authored-reduced-range-valid")
    }
}

pub struct ReducedRange {
    spec: RangeSpec,
    enabled: bool,
    peak: f32,
    gain: f32,
}
impl Default for ReducedRange {
    fn default() -> Self {
        Self {
            spec: RangeSpec::default(),
            enabled: false,
            peak: 0.0,
            gain: 1.0,
        }
    }
}
impl ReducedRange {
    pub fn enabled(&self) -> bool {
        self.enabled
    }
    pub fn clear(&mut self) {
        self.peak = 0.0;
        self.gain = 1.0;
    }
    pub fn set_enabled(&mut self, enabled: bool) {
        if self.enabled != enabled {
            self.enabled = enabled;
            self.clear();
        }
    }

    /// Linked peak detection preserves the inter-channel balance. A soft knee
    /// reduces large level differences; a separate ceiling bounds attack-time
    /// peaks. Master is applied afterwards and cannot enter the detector.
    pub fn process(&mut self, frame: &mut [f32], rate: u32) {
        if !self.enabled || rate == 0 {
            return;
        }
        let peak = frame
            .iter()
            .fold(0.0_f32, |peak, sample| peak.max(sample.abs()));
        let release = crate::simmath::exp(-1.0 / (rate as f32 * self.spec.release_seconds));
        self.peak = peak.max(self.peak * release);
        let level = 20.0 * crate::simmath::log10(self.peak.max(f32::MIN_POSITIVE));
        let over = level - self.spec.threshold_db;
        let half_knee = self.spec.knee_db * 0.5;
        let reduction = if over <= -half_knee {
            0.0
        } else if over >= half_knee {
            (1.0 / self.spec.ratio - 1.0) * over
        } else {
            (1.0 / self.spec.ratio - 1.0) * (over + half_knee).powi(2) / (2.0 * self.spec.knee_db)
        };
        let target = crate::simmath::powf(10.0, reduction / 20.0);
        let coefficient = if target < self.gain {
            crate::simmath::exp(-1.0 / (rate as f32 * self.spec.attack_seconds))
        } else {
            release
        };
        self.gain = target + coefficient * (self.gain - target);
        let mut gain = self.gain * self.spec.native_quiet_gain;
        if peak * gain > self.spec.ceiling {
            gain = self.spec.ceiling / peak;
        }
        for sample in frame {
            *sample *= gain;
        }
    }
}
