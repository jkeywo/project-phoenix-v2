//! Generic measured-contact appearance; never derived from hidden equipment.
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "RawContactProxyConfig")]
pub struct ContactProxyConfig {
    pub minimum_radius: f32,
    pub colour: [f32; 3],
    pub radar_icon: String,
}

impl Default for ContactProxyConfig {
    fn default() -> Self {
        Self {
            minimum_radius: 2.0,
            colour: [0.55, 0.8, 1.0],
            radar_icon: "__gm_basic_contact".into(),
        }
    }
}

#[derive(Deserialize)]
#[serde(default, deny_unknown_fields)]
struct RawContactProxyConfig {
    minimum_radius: f32,
    colour: [f32; 3],
    radar_icon: String,
}

impl Default for RawContactProxyConfig {
    fn default() -> Self {
        let config = ContactProxyConfig::default();
        Self {
            minimum_radius: config.minimum_radius,
            colour: config.colour,
            radar_icon: config.radar_icon,
        }
    }
}

impl TryFrom<RawContactProxyConfig> for ContactProxyConfig {
    type Error = String;

    fn try_from(raw: RawContactProxyConfig) -> Result<Self, Self::Error> {
        if !raw.minimum_radius.is_finite() || raw.minimum_radius <= 0.0 {
            return Err("render.contact_proxy.minimum_radius must be finite and positive".into());
        }
        if raw
            .colour
            .iter()
            .any(|value| !value.is_finite() || !(0.0..=1.0).contains(value))
        {
            return Err(
                "render.contact_proxy.colour requires three finite sRGB values in [0,1]".into(),
            );
        }
        if raw.radar_icon.is_empty() || raw.radar_icon.chars().any(char::is_control) {
            return Err("render.contact_proxy.radar_icon requires a nonempty icon name".into());
        }
        Ok(Self {
            minimum_radius: raw.minimum_radius,
            colour: raw.colour,
            radar_icon: raw.radar_icon,
        })
    }
}

#[cfg(test)]
#[path = "contact_render_config_tests.rs"]
mod tests;
