//! Web Audio's positional gain laws for the native room sample renderer.
//! Geometry has already crossed the shared presentation/audience boundary.
//! https://www.w3.org/TR/webaudio-1.0/#panning-algorithm
use crate::audio_config::{BlasterAudio, DistanceModel};

/// Two output channels as a matrix over the source's left/right channels.
#[derive(Clone, Copy, Debug)]
pub struct StereoMatrix(pub [[f32; 2]; 2]);

impl StereoMatrix {
    pub fn apply(self, left: f32, right: f32) -> [f32; 2] {
        [
            self.0[0][0] * left + self.0[0][1] * right,
            self.0[1][0] * left + self.0[1][1] * right,
        ]
    }
}

/// Web Audio clamps distance at refDistance for inverse/exponential, and at
/// both refDistance/maxDistance for linear. The producer separately culls
/// beyond maxDistance for every model, matching the browser allocation rule.
pub fn distance_gain(spec: &BlasterAudio, position: [f32; 3]) -> f32 {
    if position.iter().any(|value| !value.is_finite()) {
        return 0.0;
    }
    let distance = position
        .into_iter()
        .map(|value| value * value)
        .sum::<f32>()
        .sqrt();
    let reference = spec.ref_distance.max(f32::EPSILON);
    let rolloff = spec.rolloff_factor.max(0.0);
    let distance = distance.max(reference);
    let gain = match spec.distance_model {
        DistanceModel::Inverse => reference / (reference + rolloff * (distance - reference)),
        DistanceModel::Exponential => (distance / reference).powf(-rolloff),
        DistanceModel::Linear => {
            let maximum = spec.max_distance.max(reference);
            if maximum == reference {
                1.0
            } else {
                1.0 - rolloff.min(1.0) * (distance.min(maximum) - reference) / (maximum - reference)
            }
        }
    };
    if gain.is_finite() {
        gain.clamp(0.0, 1.0)
    } else {
        0.0
    }
}

/// Exact mono/stereo equal-power PannerNode law, including rear azimuth fold.
/// A stereo source keeps its centre image rather than being collapsed to mono.
pub fn equal_power(position: [f32; 3], source_channels: usize) -> StereoMatrix {
    let [x, _, z] = position;
    let mut azimuth = x.atan2(-z).to_degrees().clamp(-180.0, 180.0);
    if azimuth < -90.0 {
        azimuth = -180.0 - azimuth;
    } else if azimuth > 90.0 {
        azimuth = 180.0 - azimuth;
    }
    let normalised = if source_channels == 1 {
        (azimuth + 90.0) / 180.0
    } else if azimuth <= 0.0 {
        (azimuth + 90.0) / 90.0
    } else {
        azimuth / 90.0
    };
    let (right, left) = (normalised * std::f32::consts::FRAC_PI_2).sin_cos();
    if source_channels == 1 {
        StereoMatrix([[left, 0.0], [right, 0.0]])
    } else if azimuth <= 0.0 {
        StereoMatrix([[1.0, left], [0.0, right]])
    } else {
        StereoMatrix([[left, 0.0], [right, 1.0]])
    }
}
