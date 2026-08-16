//! The viewscreen's reference grid: one quad, one custom material, one system
//! that keeps the quad under the local ship.
//!
//! # Why a shader and not a line mesh
//!
//! The lattice is world-locked and effectively infinite, and only a bounded
//! patch of it is ever drawn. Two ways to get there:
//!
//! - **One quad + a fragment shader** that derives line coverage from the
//!   fragment's world position. One draw call, one mesh, no rebuild ever, and
//!   the antialiasing comes free from the screen-space derivative.
//! - **A generated line mesh** re-snapped to the lattice whenever the ship
//!   crosses a cell. That means a vertex buffer proportional to
//!   `patch_radius / minor_spacing` (~160 lines at the shipped numbers),
//!   rebuilt and re-uploaded on a cadence set by how fast the ship is flying,
//!   and a visible lattice-width jump each time it re-snaps.
//!
//! The first, because this repo already has six world-space custom materials on
//! exactly this pattern — `star_surface`, `star_halo`, `planet_surface`,
//! `planet_clouds`, `dust_mote`, `engine_trail`, each an `AsBindGroup` struct
//! plus a WGSL file under `assets/shaders/` registered through a
//! `MaterialPlugin`. There is no new plumbing to add and no first-of-its-kind
//! risk to carry; the mesh fallback would be the unusual choice here, not the
//! safe one.
//!
//! # Why the patch is its own entity
//!
//! It is NOT a child of the ship. A child inherits rotation and translation,
//! and this thing must hold its authored `plane_y` and stay axis-aligned while
//! the ship rolls, pitches and climbs above or below the plane. So it is a
//! sibling that copies the ship's X and Z each frame and ignores everything
//! else about it.
//!
//! # Why nothing here touches the ship
//!
//! The grid attaches NO component to any simulation entity. Its authored table
//! is read once at startup out of the config cache and lives in a resource; the
//! patch is a render entity this module owns outright. Inserting even a
//! zero-sized marker on the player hull would move that hull between
//! archetypes, and Bevy allocates archetype ids in creation order — which is
//! measurably enough to re-order query iteration across every NPC and move the
//! authoritative digest (see `LocalShip`'s note in `server_app.rs`). A render
//! feature has no business being able to do that.
//!
//! The whole module is registered only under `SimPluginOptions::render`, so a
//! headless run never builds any of it and the authored table is inert data.

use bevy::light::{NotShadowCaster, NotShadowReceiver};
use bevy::prelude::*;
use bevy::reflect::TypePath;
use bevy::render::render_resource::AsBindGroup;
use bevy::shader::ShaderRef;

use crate::reference_grid::ReferenceGridConfig;
use crate::server_app::LocalShip;

const REFERENCE_GRID_SHADER: &str = "shaders/reference_grid.wgsl";

/// The hull the viewscreen falls back to when no ship has been selected. There
/// isn't one: a hull nothing can name is a hull whose table nothing can read,
/// and the grid is opt-in. Unlike the radar — which must draw *something* —
/// drawing no grid is a perfectly good answer.
const NO_FALLBACK_HULL: Option<&str> = None;

// ── Resources and components ──────────────────────────────────────────────

/// The local hull's authored `[reference_grid]`, resolved once at startup.
///
/// `None` means the hull authors no table, which is every hull that has not
/// opted in — including every NPC, whose config this never consults in the
/// first place.
#[derive(Resource, Debug, Clone, Copy, PartialEq, Default)]
pub struct ReferenceGridTuning(pub Option<ReferenceGridConfig>);

/// Marks the grid patch entity. Exactly one exists while the local ship does.
#[derive(Component, Debug)]
pub struct ReferenceGridPatch;

// ── Material ──────────────────────────────────────────────────────────────

/// Flat `f32` fields rather than `Vec4`s, following the `star.rs` /
/// `dust_mote` precedent: `AsBindGroup` concatenates same-index uniform fields
/// in declaration order, and scalars sidestep the std140 alignment traps that
/// the WebGL2 backend is least forgiving about. Seventeen live floats plus
/// three `_pad`s — twenty in all, 80 bytes, five whole 16-byte rows, the same
/// explicit-padding idiom `StarHaloMaterial` uses to round out its rows.
/// `plane_y` is NOT here: the plane's height is a transform on the patch
/// entity, and the shader never needs it.
#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
pub struct ReferenceGridMaterial {
    #[uniform(0)]
    pub minor_r: f32,
    #[uniform(0)]
    pub minor_g: f32,
    #[uniform(0)]
    pub minor_b: f32,
    #[uniform(0)]
    pub minor_a: f32,
    #[uniform(0)]
    pub major_r: f32,
    #[uniform(0)]
    pub major_g: f32,
    #[uniform(0)]
    pub major_b: f32,
    #[uniform(0)]
    pub major_a: f32,
    #[uniform(0)]
    pub minor_spacing: f32,
    #[uniform(0)]
    pub major_spacing: f32,
    #[uniform(0)]
    pub minor_half_width_px: f32,
    #[uniform(0)]
    pub major_half_width_px: f32,
    #[uniform(0)]
    pub opacity: f32,
    #[uniform(0)]
    pub patch_radius: f32,
    #[uniform(0)]
    pub fade_start: f32,
    #[uniform(0)]
    pub fade_span: f32,
    #[uniform(0)]
    pub fade_exponent: f32,
    #[uniform(0)]
    pub _pad0: f32,
    #[uniform(0)]
    pub _pad1: f32,
    #[uniform(0)]
    pub _pad2: f32,
}

impl Material for ReferenceGridMaterial {
    fn fragment_shader() -> ShaderRef {
        REFERENCE_GRID_SHADER.into()
    }

    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Blend
    }

    fn specialize(
        _pipeline: &bevy::pbr::MaterialPipeline,
        descriptor: &mut bevy::render::render_resource::RenderPipelineDescriptor,
        _layout: &bevy::mesh::MeshVertexBufferLayoutRef,
        _key: bevy::pbr::MaterialPipelineKey<Self>,
    ) -> Result<(), bevy::render::render_resource::SpecializedMeshPipelineError> {
        // Visible from underneath. The cinematic camera regularly sits below
        // the y = 0 plane, and a back-face-culled patch would blink out at
        // exactly the moment the ship's height above the plane is the thing
        // being read.
        descriptor.primitive.cull_mode = None;
        Ok(())
    }
}

/// Build the material uniform from the authored table.
///
/// The three derived values — `fade_start`, `fade_span` and the half widths —
/// are computed here rather than in WGSL so that the clamping the validator
/// already reasoned about happens once, on the CPU, where it is tested.
pub fn material_from_config(config: &ReferenceGridConfig) -> ReferenceGridMaterial {
    ReferenceGridMaterial {
        minor_r: config.minor_colour[0],
        minor_g: config.minor_colour[1],
        minor_b: config.minor_colour[2],
        minor_a: config.minor_colour[3],
        major_r: config.major_colour[0],
        major_g: config.major_colour[1],
        major_b: config.major_colour[2],
        major_a: config.major_colour[3],
        minor_spacing: config.minor_spacing,
        major_spacing: config.major_spacing,
        minor_half_width_px: config.minor_line_width_px * 0.5,
        major_half_width_px: config.major_line_width_px * 0.5,
        opacity: config.opacity,
        patch_radius: config.patch_radius,
        fade_start: config.fade_start(),
        fade_span: config.fade_span(),
        fade_exponent: config.fade_exponent,
        _pad0: 0.0,
        _pad1: 0.0,
        _pad2: 0.0,
    }
}

// ── Spawn gating ──────────────────────────────────────────────────────────

/// What [`sync_reference_grid_patch`] should do this frame.
///
/// Pulled out of the system so the gating rule — the part with actual policy in
/// it — is decidable without an `App`, a GPU or an asset server.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PatchAction {
    /// Create the patch under this world XZ.
    Spawn(Vec2),
    /// Move the existing patch to this world XZ. Y is not this type's business:
    /// the plane's height is the authored `plane_y`, applied by the system.
    Follow(Vec2),
    /// Remove the patch — the ship it belonged to is gone.
    Despawn,
    /// Leave everything alone.
    Idle,
}

/// The gating rule, entire.
///
/// `ship_xz` is the local ship's world position if one exists. `authored` is
/// whether the local hull's resolved config carries a `[reference_grid]` table.
/// `patch_exists` is whether we have already spawned one.
///
/// Note what this cannot express: there is no input for "which ship". The only
/// position that reaches it is the LOCAL ship's, so an NPC hull carrying an
/// authored table — or a hundred of them — still produces exactly zero grids.
/// That is a structural guarantee rather than a filter that could be forgotten.
pub fn decide_patch_action(
    ship_xz: Option<Vec2>,
    authored: bool,
    patch_exists: bool,
) -> PatchAction {
    match (ship_xz, authored, patch_exists) {
        // No ship: nothing to sit under. Clean up if we left one behind.
        (None, _, true) => PatchAction::Despawn,
        (None, _, false) => PatchAction::Idle,
        // Hull authors no table: no grid, ever, and remove one if the table
        // somehow went away underneath us.
        (Some(_), false, true) => PatchAction::Despawn,
        (Some(_), false, false) => PatchAction::Idle,
        (Some(xz), true, false) => PatchAction::Spawn(xz),
        (Some(xz), true, true) => PatchAction::Follow(xz),
    }
}

// ── Systems ───────────────────────────────────────────────────────────────

/// Resolve the local hull's `[reference_grid]` once, at startup, out of the
/// config cache — the same path `server::radar` reads the viewscreen's radar
/// ranges through, so the grid and the radar can never disagree about which
/// hull the player is flying.
fn resolve_reference_grid_config(
    mut commands: Commands,
    selected_ship: Option<Res<crate::lobby::SelectedShipResource>>,
    log: Option<Res<crate::logging::LogFilterConfig>>,
) {
    let path = selected_ship
        .as_ref()
        .map(|selected| selected.0.as_str())
        .or(NO_FALLBACK_HULL);

    let authored = path.and_then(|path| {
        crate::config_cache::get_cached_entity_config(path).and_then(|config| config.reference_grid)
    });

    match (&path, &authored) {
        (Some(path), Some(_)) => {
            crate::pdebug!(
                log,
                crate::logging::LogCat::Assets,
                "reference grid authored by {path}"
            );
        }
        (Some(path), None) => {
            crate::pdebug!(
                log,
                crate::logging::LogCat::Assets,
                "no [reference_grid] on {path}; the viewscreen draws none"
            );
        }
        (None, _) => {
            crate::pdebug!(
                log,
                crate::logging::LogCat::Assets,
                "no hull selected; the viewscreen draws no reference grid"
            );
        }
    }

    commands.insert_resource(ReferenceGridTuning(authored));
}

/// Keep the patch under the local ship, at the authored `plane_y`.
///
/// Runs every rendered frame rather than on the sim tick: it writes nothing the
/// simulation reads, and a patch that lagged the ship by up to a tick would
/// smear against the very motion it exists to convey.
fn sync_reference_grid_patch(
    mut commands: Commands,
    tuning: Res<ReferenceGridTuning>,
    ship: Query<&Transform, (With<LocalShip>, Without<ReferenceGridPatch>)>,
    mut patch: Query<(Entity, &mut Transform), With<ReferenceGridPatch>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ReferenceGridMaterial>>,
) {
    let ship_xz = ship
        .iter()
        .next()
        .map(|transform| transform.translation.xz());
    let existing = patch.iter_mut().next();

    // The plane rides at the authored `plane_y` — below the hull, so the grid
    // reads as a floor rather than a lattice co-planar with the ship. Absent a
    // table (only reachable via Despawn/Idle) this is never consulted.
    let plane_y = tuning.0.map(|config| config.plane_y).unwrap_or(0.0);

    match decide_patch_action(ship_xz, tuning.0.is_some(), existing.is_some()) {
        PatchAction::Idle => {}
        PatchAction::Despawn => {
            if let Some((entity, _)) = existing {
                commands.entity(entity).despawn();
            }
        }
        PatchAction::Follow(xz) => {
            if let Some((_, mut transform)) = existing {
                transform.translation = Vec3::new(xz.x, plane_y, xz.y);
            }
        }
        PatchAction::Spawn(xz) => {
            let Some(config) = tuning.0 else {
                return;
            };
            let half = config.patch_half_size();
            // `Plane3d` already lies in the XZ plane facing +Y with UVs running
            // 0-1 across it, which is exactly the frame the shader's radial
            // fade expects — so the patch needs no rotation of its own and
            // never acquires one.
            let mesh = meshes.add(Mesh::from(Plane3d::new(Vec3::Y, Vec2::splat(half))));
            let material = materials.add(material_from_config(&config));
            commands.spawn((
                ReferenceGridPatch,
                Mesh3d(mesh),
                MeshMaterial3d(material),
                Transform::from_xyz(xz.x, plane_y, xz.y),
                // Never shadow-casts and never receives: it is a navigation
                // aid drawn in the y = 0 plane, not a surface in the scene.
                NotShadowCaster,
                NotShadowReceiver,
            ));
        }
    }
}

// ── Plugin ────────────────────────────────────────────────────────────────

/// Registered only under `SimPluginOptions::render`. A headless run builds none
/// of this, which is why the authored table cannot reach the simulation.
pub struct ReferenceGridPlugin;

impl Plugin for ReferenceGridPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(MaterialPlugin::<ReferenceGridMaterial>::default())
            .init_resource::<ReferenceGridTuning>()
            .add_systems(Startup, resolve_reference_grid_config)
            .add_systems(Update, sync_reference_grid_patch);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Gating ────────────────────────────────────────────────────────────

    #[test]
    fn an_authored_hull_with_a_local_ship_spawns_a_patch() {
        let action = decide_patch_action(Some(Vec2::new(12.0, -30.0)), true, false);
        assert_eq!(action, PatchAction::Spawn(Vec2::new(12.0, -30.0)));
    }

    #[test]
    fn a_hull_that_authors_no_table_never_spawns_one() {
        assert_eq!(
            decide_patch_action(Some(Vec2::new(12.0, -30.0)), false, false),
            PatchAction::Idle
        );
    }

    #[test]
    fn the_patch_follows_the_ship_once_it_exists() {
        assert_eq!(
            decide_patch_action(Some(Vec2::new(4.0, 5.0)), true, true),
            PatchAction::Follow(Vec2::new(4.0, 5.0))
        );
    }

    #[test]
    fn losing_the_ship_takes_the_patch_with_it() {
        assert_eq!(decide_patch_action(None, true, true), PatchAction::Despawn);
        assert_eq!(decide_patch_action(None, true, false), PatchAction::Idle);
    }

    #[test]
    fn a_table_that_goes_away_takes_an_existing_patch_with_it() {
        assert_eq!(
            decide_patch_action(Some(Vec2::ZERO), false, true),
            PatchAction::Despawn
        );
    }

    /// The structural half of "NPC hulls never carry one": the gate has no
    /// input that could express a non-local ship, so there is no arrangement of
    /// NPCs that reaches a second grid. Asserted by exhausting the input space.
    #[test]
    fn no_input_combination_produces_more_than_one_patch() {
        for authored in [true, false] {
            for patch_exists in [true, false] {
                for ship_xz in [None, Some(Vec2::new(1.0, 2.0))] {
                    let action = decide_patch_action(ship_xz, authored, patch_exists);
                    if patch_exists {
                        assert_ne!(
                            action,
                            PatchAction::Spawn(Vec2::new(1.0, 2.0)),
                            "a second patch was spawned for \
                             ship={ship_xz:?} authored={authored}"
                        );
                    }
                }
            }
        }
    }

    // ── Uniform ───────────────────────────────────────────────────────────

    #[test]
    fn the_uniform_carries_the_authored_table() {
        let config = ReferenceGridConfig::default();
        let material = material_from_config(&config);
        assert_eq!(material.minor_spacing, 10.0);
        assert_eq!(material.major_spacing, 50.0);
        assert_eq!(material.minor_a, config.minor_colour[3]);
        assert_eq!(material.major_a, config.major_colour[3]);
        assert_eq!(material.opacity, config.opacity);
        assert_eq!(material.patch_radius, config.patch_radius);
        assert_eq!(material.fade_exponent, config.fade_exponent);
        // The three pads exist only to round the uniform to whole 16-byte rows;
        // they must carry nothing the shader could read as data.
        assert_eq!(
            (material._pad0, material._pad1, material._pad2),
            (0.0, 0.0, 0.0)
        );
    }

    #[test]
    fn the_uniform_halves_the_authored_pixel_widths() {
        // The shader compares against a distance from the line CENTRE, so it
        // wants half widths. Doing it here means the WGSL carries no arithmetic
        // on authored values at all.
        let config = ReferenceGridConfig {
            minor_line_width_px: 3.0,
            major_line_width_px: 5.0,
            ..Default::default()
        };
        let material = material_from_config(&config);
        assert_eq!(material.minor_half_width_px, 1.5);
        assert_eq!(material.major_half_width_px, 2.5);
    }

    #[test]
    fn the_uniform_fade_span_is_never_zero() {
        // WGSL divides by it unconditionally.
        let config = ReferenceGridConfig {
            fade_band: 0.0,
            ..Default::default()
        };
        assert!(material_from_config(&config).fade_span > 0.0);
    }

    // ── Shipped content ───────────────────────────────────────────────────

    fn shipped_hull(stem: &str) -> crate::entity_config::EntityConfig {
        let path = format!("assets/entities/{stem}.toml");
        crate::entity_includes::load_entity_config(&path)
            .unwrap_or_else(|e| panic!("{stem}.toml must compose and parse: {e}"))
    }

    #[test]
    fn the_player_destroyer_authors_a_grid_that_validates() {
        let config = shipped_hull("alliance_destroyer")
            .reference_grid
            .expect("alliance_destroyer.toml authors [reference_grid]");
        config
            .validate()
            .expect("the shipped table must pass the same validator a load does");
        assert_eq!(config.minor_spacing, 10.0);
        assert_eq!(config.major_spacing, 50.0);
        // [ai] The retuned floor/fade values John signed off on. Pinned so a
        // future edit to the TOML that drops one is caught here.
        assert_eq!(config.plane_y, -0.5);
        assert_eq!(config.fade_band, 250.0);
        assert_eq!(config.fade_exponent, 2.5);
        // The uniform it produces is the one the shader is calibrated against.
        let material = material_from_config(&config);
        assert!(
            material.minor_a <= 0.25 && material.major_a <= 0.25,
            "\"faint\" is carried in alpha; these are what keep it faint"
        );
    }

    /// The other half of "NPC hulls never carry one", checked against the
    /// shipped content rather than against the gate: no hostile hull authors
    /// the table, so even a future bug in the gate has nothing to act on.
    #[test]
    fn no_npc_hull_authors_a_reference_grid() {
        for stem in [
            "ship_harrow_destroyer",
            "ship_harrow_warhawk",
            "ship_requiem_courier",
            "alliance_courier",
        ] {
            assert!(
                shipped_hull(stem).reference_grid.is_none(),
                "{stem}.toml must not author [reference_grid] — the grid is the local \
                 player ship's alone"
            );
        }
    }
}
