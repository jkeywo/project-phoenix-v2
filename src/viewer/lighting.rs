//! Lighting modes for the viewer.
//!
//! The three modes isolate what each contribution does to a model:
//!
//! - **Off** — no scene lights at all. Only the skybox reaches the surface, so
//!   this shows raw albedo and emissive with nothing else layered on.
//! - **Ambient** — the game's own default fill
//!   ([`crate::render_setup::default_ambient_light`]), which is what most
//!   scenarios actually render with. This is the mode to judge "does the ship
//!   look right in game".
//! - **Directional** — ambient plus a steerable key light, for checking normal
//!   maps, specular response and self-shadowing.

use bevy::prelude::*;

use crate::render_setup::default_ambient_light;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    Off,
    #[default]
    Ambient,
    Directional,
}

impl Mode {
    /// Parse a mode name from a URL parameter or JS call. Unrecognised values
    /// fall back to `Ambient` (the game default) with a warning rather than
    /// panicking a dev tool over a typo.
    pub fn parse(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "off" | "none" => Mode::Off,
            "ambient" => Mode::Ambient,
            "directional" | "dir" => Mode::Directional,
            other => {
                bevy::log::warn!("viewer: unknown lighting mode '{other}', using 'ambient'");
                Mode::Ambient
            }
        }
    }
}

/// Current lighting mode plus the tunable parameters behind it.
#[derive(Resource, Debug, Clone)]
pub struct LightingMode {
    pub mode: Mode,
    pub ambient_color: [f32; 3],
    pub ambient_brightness: f32,
    pub directional_illuminance: f32,
    /// Radians around Y, 0 = light coming from -Z (behind the camera's home).
    pub directional_yaw: f32,
    /// Radians of elevation; positive tips the light upward.
    pub directional_pitch: f32,
}

impl Default for LightingMode {
    fn default() -> Self {
        let ambient = default_ambient_light();
        let srgb = ambient.color.to_srgba();
        Self {
            mode: Mode::default(),
            ambient_color: [srgb.red, srgb.green, srgb.blue],
            ambient_brightness: ambient.brightness,
            // Bevy's own default for a `DirectionalLight`, i.e. what a light
            // authored in an entity TOML gets when it omits `illuminance`.
            directional_illuminance: DirectionalLight::default().illuminance,
            directional_yaw: -0.6,
            directional_pitch: -0.5,
        }
    }
}

/// Marker for lights the viewer owns, so re-applying a mode can clear them
/// without touching anything else in the scene.
#[derive(Component)]
pub struct ViewerLight;

/// Rebuild the scene's lights whenever the mode or its parameters change.
///
/// Ambient light in Bevy is a component on an entity, not a global, so "off"
/// means despawning it rather than zeroing it — a zero-brightness ambient and
/// no ambient at all are the same picture, but despawning keeps the entity
/// count honest about what is contributing.
pub fn apply_lighting(
    mut commands: Commands,
    lighting: Res<LightingMode>,
    existing: Query<Entity, With<ViewerLight>>,
) {
    if !lighting.is_changed() {
        return;
    }
    for entity in &existing {
        commands.entity(entity).despawn();
    }

    let ambient = AmbientLight {
        color: Color::srgb(
            lighting.ambient_color[0],
            lighting.ambient_color[1],
            lighting.ambient_color[2],
        ),
        brightness: lighting.ambient_brightness,
        ..default()
    };

    match lighting.mode {
        Mode::Off => {
            // Deliberately nothing: skybox-only illumination.
        }
        Mode::Ambient => {
            commands.spawn((ViewerLight, ambient));
        }
        Mode::Directional => {
            commands.spawn((ViewerLight, ambient));
            commands.spawn((
                ViewerLight,
                DirectionalLight {
                    illuminance: lighting.directional_illuminance,
                    shadows_enabled: true,
                    ..default()
                },
                Transform::from_rotation(Quat::from_euler(
                    EulerRot::YXZ,
                    lighting.directional_yaw,
                    lighting.directional_pitch,
                    0.0,
                )),
            ));
        }
    }

    bevy::log::info!(
        "viewer lighting: {:?} (ambient {:.0}, directional {:.0})",
        lighting.mode,
        lighting.ambient_brightness,
        lighting.directional_illuminance,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_known_modes() {
        assert_eq!(Mode::parse("off"), Mode::Off);
        assert_eq!(Mode::parse("Ambient"), Mode::Ambient);
        assert_eq!(Mode::parse(" DIRECTIONAL "), Mode::Directional);
    }

    #[test]
    fn unknown_mode_falls_back_to_ambient() {
        assert_eq!(Mode::parse("sideways"), Mode::Ambient);
    }

    #[test]
    fn default_matches_the_games_ambient_fill() {
        let lighting = LightingMode::default();
        assert_eq!(
            lighting.ambient_brightness,
            crate::render_setup::DEFAULT_AMBIENT_BRIGHTNESS
        );
        assert_eq!(lighting.mode, Mode::Ambient);
    }
}
