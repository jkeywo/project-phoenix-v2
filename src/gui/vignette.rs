//! `GuiVignette` — full-screen `UiMaterial` overlay driven by `RedAlertIntensity`.
//!
//! The `RedAlertIntensity` resource is written by game logic (pulse, shield flash)
//! each frame.  Two systems consume it:
//!
//! - One writes `RedAlertIntensity` into the `RedAlertVignetteMaterial` uniform.
//! - The `GuiBorder` plugin reads it to swap border textures.
//!
//! Both the phone bezel and the viewscreen border share the same
//! `RedAlertVignetteMaterial` shader but define their own material struct.
//! The client version defined here uses 4 fields (16 bytes) for WGSL std140
//! alignment with the shared WGSL shader.

use bevy::prelude::*;
use bevy::render::render_resource::AsBindGroup;
use bevy::shader::ShaderRef;
use bevy::ui_render::prelude::{MaterialNode, UiMaterial, UiMaterialPlugin};

// ── RedAlertIntensity resource ────────────────────────────────────────────

/// Shared intensity value for the red-alert vignette pulse, 0.0–1.0.
///
/// Written by game logic each frame (pulse function, shield-flash decay, etc.).
/// Read by `drive_vignette_material` to update the shader uniform, and by
/// `GuiBorderPlugin::update_border_textures` to swap corner/edge images.
#[derive(Resource, Default, Debug, Clone, Copy, PartialEq)]
pub struct RedAlertIntensity(pub f32);

// ── Material ──────────────────────────────────────────────────────────────

/// Red-alert vignette material for the client (phone) app.
///
/// Layout: 4×f32 = 16 bytes for WGSL std140 alignment.  The shared WGSL
/// shader expects this layout.
#[derive(AsBindGroup, Asset, TypePath, Debug, Clone)]
pub struct RedAlertVignetteMaterial {
    #[uniform(0)]
    pub intensity: f32,
    #[uniform(0)]
    pub flash_intensity: f32,
    #[uniform(0)]
    pub aspect_ratio: f32,
    #[uniform(0)]
    pub _pad0: f32,
}

impl RedAlertVignetteMaterial {
    /// Create a new material with the given intensity (0.0–1.0) and default
    /// aspect ratio 1.0 (square).  Call [`set_aspect_ratio`] after creation.
    pub fn new(intensity: f32) -> Self {
        Self { intensity, flash_intensity: 0.0, aspect_ratio: 1.0, _pad0: 0.0 }
    }
}

impl UiMaterial for RedAlertVignetteMaterial {
    fn fragment_shader() -> ShaderRef {
        "shaders/red_alert_vignette.wgsl".into()
    }
}

// ── Cache handle ──────────────────────────────────────────────────────────

/// Cached handle to the single `RedAlertVignetteMaterial` instance so the
/// update system can mutate its uniform without a query.
#[derive(Resource, Debug, Clone)]
pub struct VignetteMaterialHandle(pub Handle<RedAlertVignetteMaterial>);

// ── GuiVignette marker ────────────────────────────────────────────────────

/// Marker on the vignette `MaterialNode<RedAlertVignetteMaterial>` entity.
#[derive(Component)]
pub struct GuiVignette;

// ── Widget ────────────────────────────────────────────────────────────────

/// Placeholder struct — the `spawn` method creates the overlay.
pub struct GuiVignetteWidget;

impl GuiVignetteWidget {
    /// Spawn a full-screen vignette overlay.
    ///
    /// Returns the overlay entity.  The overlay is a `MaterialNode` at front
    /// depth, sized to 100% × 100%.
    pub fn spawn(
        commands: &mut Commands,
        material_handle: Handle<RedAlertVignetteMaterial>,
    ) -> Entity {
        commands
            .spawn((
                GuiVignette,
                MaterialNode(material_handle),
                Node {
                    position_type: PositionType::Absolute,
                    top: Val::Px(0.0),
                    left: Val::Px(0.0),
                    width: Val::Percent(100.0),
                    height: Val::Percent(100.0),
                    ..default()
                },
            ))
            .id()
    }
}

// ── Systems ───────────────────────────────────────────────────────────────

/// Each frame: write `RedAlertIntensity` into the cached `RedAlertVignetteMaterial`.
///
/// This system is registered by `GuiVignettePlugin`.  Game logic should write
/// `RedAlertIntensity` before this system runs (use `before` ordering if needed).
fn drive_vignette_material(
    intensity: Option<Res<RedAlertIntensity>>,
    handle: Option<Res<VignetteMaterialHandle>>,
    mut materials: ResMut<Assets<RedAlertVignetteMaterial>>,
) {
    let Some(intensity) = intensity else { return };
    let Some(handle) = handle else { return };
    let Some(material) = materials.get_mut(&handle.0) else { return };
    material.intensity = intensity.0;
}

// ── Plugin ────────────────────────────────────────────────────────────────

/// Sub-plugin for the vignette widget.
///
/// Registers `RedAlertVignetteMaterial` as a `UiMaterial` and installs the
/// `drive_vignette_material` system that keeps the shader uniform in sync
/// with the `RedAlertIntensity` resource.
pub struct GuiVignettePlugin;

impl Plugin for GuiVignettePlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(UiMaterialPlugin::<RedAlertVignetteMaterial>::default())
            .add_systems(Update, drive_vignette_material);
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn intensity_defaults_to_zero() {
        let i = RedAlertIntensity::default();
        assert_eq!(i.0, 0.0);
    }

    #[test]
    fn intensity_can_be_set() {
        let i = RedAlertIntensity(0.75);
        assert_eq!(i.0, 0.75);
    }

    #[test]
    fn intensity_clamps_not_enforced_by_resource() {
        // The type is a newtype wrapper with no clamping.
        // Callers are expected to clamp to 0.0–1.0 themselves.
        let i = RedAlertIntensity(1.5);
        assert_eq!(i.0, 1.5);
    }

    #[test]
    fn material_default_intensity_is_zero() {
        let m = RedAlertVignetteMaterial {
            intensity: 0.0,
            flash_intensity: 0.0,
            aspect_ratio: 1.0,
            _pad0: 0.0,
        };
        assert_eq!(m.intensity, 0.0);
    }

    #[test]
    fn material_holds_values() {
        let m = RedAlertVignetteMaterial {
            intensity: 0.75,
            flash_intensity: 0.0,
            aspect_ratio: 1.0,
            _pad0: 0.0,
        };
        assert_eq!(m.intensity, 0.75);
    }

    #[test]
    fn material_new_uses_aspect_ratio_1() {
        let m = RedAlertVignetteMaterial::new(0.5);
        assert_eq!(m.aspect_ratio, 1.0);
        assert_eq!(m.intensity, 0.5);
    }
}
