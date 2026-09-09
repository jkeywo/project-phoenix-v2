//! Native-only presentation tuning, authored under `[render.native]`.
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct NativeRenderConfig {
    pub flare_intensity: f32,
    pub star_shadows: bool,
    pub shadow_resolution: usize,
    pub shadow_distance: f32,
    pub motes: bool,
    pub mote_count: usize,
    pub mote_half_width: f32,
    pub mote_depth: f32,
    pub mote_width: f32,
    pub mote_streak_per_speed: f32,
    pub mote_tint: [f32; 3],
    pub mote_brightness: [f32; 3],
    pub mote_opacity: f32,
    pub mote_textures: [String; 3],
}

impl Default for NativeRenderConfig {
    fn default() -> Self {
        Self {
            // User-approved lighting-lab selection: motes, shadows, flare 3.
            flare_intensity: 3.0,
            star_shadows: true,
            // [ai] Keep high-detail shadow coverage local to the encounter.
            shadow_resolution: 2048,
            shadow_distance: 1000.0,
            motes: true,
            mote_count: 440,
            mote_half_width: 30.0,
            mote_depth: 100.0,
            mote_width: 0.08,
            mote_streak_per_speed: 0.25,
            mote_tint: [0.7, 0.8, 1.0],
            mote_brightness: [0.7, 0.55, 0.4],
            mote_opacity: 0.45,
            mote_textures: [
                "pfx/space_mote_streak_head.png",
                "pfx/space_mote_streak_soft.png",
                "pfx/space_mote_compact_core.png",
            ]
            .map(str::to_owned),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn old_render_tables_adopt_native_defaults_and_allow_independent_overrides() {
        let old: crate::world::config::RenderConfig = toml::from_str("hdr = true").unwrap();
        assert_eq!(old.native.flare_intensity, 3.0);
        assert!(old.native.motes && old.native.star_shadows);
        let custom: crate::world::config::RenderConfig =
            toml::from_str("[native]\nflare_intensity = 0.0\nmotes = false").unwrap();
        assert_eq!(custom.native.flare_intensity, 0.0);
        assert!(!custom.native.motes);
        assert!(custom.native.star_shadows);
        assert_eq!(
            custom.native.mote_count,
            NativeRenderConfig::default().mote_count
        );
    }
}
