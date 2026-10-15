//! Receiver-local interference factor shared by fictional link consumers.
//! Natural segment attenuation remains at signal_path; this never changes it.

/// Exact legacy environmental denial before any finite equipment exposure.
/// Invalid derived exposure fails closed; accepted processors emit nonnegative
/// finite values and accumulate them in stable source order using f64.
pub fn endpoint_factor(environmental_denial: bool, equipment_exposure: f64) -> f32 {
    if environmental_denial || !equipment_exposure.is_finite() || equipment_exposure < 0.0 {
        return 0.0;
    }
    (1.0 / (1.0 + equipment_exposure)) as f32
}

#[cfg(test)]
#[path = "interference_tests.rs"]
mod tests;
