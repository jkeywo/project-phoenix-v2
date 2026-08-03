//! Showing a model's LOD ladder.
//!
//! The viewer reads the same `[[lod]]` chain the game reads — out of the
//! model's own rig sidecar (issue #914) — and picks a level exactly the way the
//! game picks one, through [`crate::entity_config::select_lod`] with its
//! hysteresis. That shared selection is the point of the tool: "does anything
//! pop when I fly past" is a question about `select_lod`, and a viewer that
//! swapped levels by its own rule would answer a different question.
//!
//! Three modes, all driven from the HTML panel:
//!
//! - **Base** — the `.glb` the dropdown names, ladder ignored. What the viewer
//!   always did, and still the right mode for looking at a model as authored.
//! - **Fixed** — one level, held regardless of distance. The mode for judging a
//!   decimated hull: park on LOD 2 and orbit it.
//! - **Auto** — the game's own behaviour. Zoom out and the levels swap at their
//!   real thresholds, with the real margin.
//!
//! Distance is measured from the camera to the origin, where the subject sits.
//! That is the same quantity `update_mesh_lod` measures (camera to entity), so
//! the number on the panel is the number the game would compare against.

use bevy::prelude::*;

use crate::entities::glb_visual::resolve_sidecar_rig;
use crate::entity_config::{select_lod, LodLevel};

use super::subject::{ProceduralLevel, Showing, SubjectState};
use super::{ViewerArgs, ViewerCamera};

/// The renderer's own default emissive multiplier for a general-purpose entity
/// (`update_mesh_lod`), applied here for the same reason: a level that omits
/// `emissive` inherits it rather than rendering unlit.
const DEFAULT_EMISSIVE: f32 = 0.4;

/// Neutral grey for a procedural level whose colour would come from an entity
/// the viewer does not have — a placeholder shown in the absence of an
/// authoritative value, not a gameplay value (Key Constraint 11).
const STAND_IN_COLOUR: [f32; 3] = [0.5, 0.5, 0.5];

/// Tube radius for a torus level that names none, as a fraction of its major
/// radius. Same placeholder reasoning: the entity would have supplied it, and
/// the serde default (`0.0`) would draw nothing at all.
const STAND_IN_TUBE_FRACTION: f32 = 0.25;

/// Which level the viewer is showing, and how it chose it.
#[derive(Resource, Default, Debug, Clone, Copy, PartialEq)]
pub enum LodMode {
    /// Ignore the ladder; render the selected model itself.
    #[default]
    Base,
    /// Follow the camera distance, as the game does.
    Auto,
    /// Hold one level whatever the distance.
    Fixed(usize),
}

impl LodMode {
    /// Parse the panel's mode name. `index` is only read for `fixed`.
    pub fn parse(mode: &str, index: usize) -> LodMode {
        match mode {
            "auto" => LodMode::Auto,
            "fixed" => LodMode::Fixed(index),
            _ => LodMode::Base,
        }
    }

    /// The name the panel and the stats readout use.
    pub fn name(&self) -> &'static str {
        match self {
            LodMode::Base => "base",
            LodMode::Auto => "auto",
            LodMode::Fixed(_) => "fixed",
        }
    }
}

/// The ladder of the model currently selected in the dropdown.
#[derive(Resource, Default)]
pub struct LadderState {
    /// The `(model, variant)` the levels were read from. A change here is what
    /// makes the ladder reload; until the new sidecar resolves (a fetch, on
    /// wasm) the old levels stay up rather than blinking to empty.
    pub source: Option<(String, Option<String>)>,
    /// Levels as authored, near→far. Empty means this model has no ladder.
    pub levels: Vec<LodLevel>,
    /// The level on screen, or `None` when the base model is showing.
    pub current: Option<usize>,
    /// Camera distance to the subject, in world units.
    pub distance: f32,
    /// The model's largest extent, from its rig sidecar. Sizes a procedural
    /// level that names no radius.
    pub extent: Option<f32>,
    /// A strong handle to every GLB level's scene, held for as long as this
    /// ladder is the one on screen.
    ///
    /// The game preloads a whole ladder the frame the sidecar lands
    /// (`discover_sidecar_lod_assets`), so a level swap there is a handle
    /// change and nothing else. The viewer used to load each level the first
    /// time it was shown, which put a fetch — and a visible gap — in the middle
    /// of the switch being judged. Holding the handles makes the viewer's
    /// transition the game's transition.
    pub preloaded: Vec<Handle<Scene>>,
}

impl LadderState {
    /// The level `mode` asks for at `distance`, or `None` for the base model.
    ///
    /// A `Fixed` index past the end of the ladder falls back to the base model
    /// rather than clamping to the last level: the panel holds its mode across
    /// a model switch, and silently showing level 1 when level 3 was asked for
    /// would read as "this model's LOD 3 looks like that".
    pub fn desired(&self, mode: LodMode, distance: f32) -> Option<usize> {
        if self.levels.is_empty() {
            return None;
        }
        match mode {
            LodMode::Base => None,
            LodMode::Auto => Some(select_lod(&self.levels, distance, self.current)),
            LodMode::Fixed(i) => (i < self.levels.len()).then_some(i),
        }
    }

    /// Half the model's largest extent: what a procedural level that names no
    /// radius stands in for.
    fn stand_in_radius(&self) -> f32 {
        self.extent.map(|e| e * 0.5).unwrap_or(1.0)
    }
}

/// Read the selected model's ladder out of its rig sidecar.
///
/// Runs every frame because the sidecar arrives asynchronously on wasm: the
/// first frames after a model switch resolve to `None` while the fetch is in
/// flight, and the previous model's ladder stays on the panel until the new one
/// lands.
/// `asset_server` is optional for the reason `LogFilterConfig` is (AGENTS.md):
/// a bare `Res` fails Bevy's parameter validation in any bare-`App` fixture,
/// and the ladder's *selection* — which is what those fixtures assert on — does
/// not need one. `None` simply skips the preload.
pub fn refresh_ladder(
    args: Res<ViewerArgs>,
    asset_server: Option<Res<AssetServer>>,
    mut ladder: ResMut<LadderState>,
    mut subject: ResMut<SubjectState>,
) {
    let Some(model) = args.model.clone() else {
        // An `?entity=` subject has no ladder of its own to show.
        if ladder.source.is_some() {
            *ladder = LadderState::default();
            subject.showing = Showing::Base;
        }
        return;
    };
    let key = (model, args.variant.clone());
    if ladder.source.as_ref() == Some(&key) {
        return;
    }
    let Some(rig) = resolve_sidecar_rig(&key.0, key.1.as_deref()) else {
        return; // sidecar fetch in flight — try again next frame
    };
    ladder.levels = rig.lod.clone();
    ladder.extent = rig
        .extents
        .as_ref()
        .map(|e| e.size.iter().fold(0.0_f32, |a, b| a.max(*b)));
    ladder.source = Some(key);
    ladder.current = None;
    ladder.preloaded = asset_server
        .map(|server| preload_levels(&server, &ladder.levels))
        .unwrap_or_default();
    // Whatever was on screen belonged to the previous model.
    subject.showing = Showing::Base;
}

/// Request every GLB level of a ladder up front, and keep the handles.
///
/// Dropping these is what would let Bevy release the assets again, so the
/// returned handles are stored on [`LadderState`] rather than discarded.
pub fn preload_levels(asset_server: &AssetServer, levels: &[LodLevel]) -> Vec<Handle<Scene>> {
    levels
        .iter()
        .filter_map(|level| level.model.as_deref())
        .map(|model| {
            // The asset root is `assets/`, but sidecar paths carry the prefix.
            let rel = model.strip_prefix("assets/").unwrap_or(model);
            asset_server.load(format!("{rel}#Scene0"))
        })
        .collect()
}

/// Apply the current mode: swap what the subject shows when the chosen level
/// changes, and keep the panel's distance readout live.
pub fn apply_lod_mode(
    mode: Res<LodMode>,
    mut ladder: ResMut<LadderState>,
    mut subject: ResMut<SubjectState>,
    args: Res<ViewerArgs>,
    cameras: Query<&Transform, With<ViewerCamera>>,
    mut commands: Commands,
) {
    // The subject sits at the origin, so the camera's distance from it is the
    // length of its own translation.
    if let Some(camera) = cameras.iter().next() {
        ladder.distance = camera.translation.length();
    }

    let desired = ladder.desired(*mode, ladder.distance);
    let showing = match desired.and_then(|i| ladder.levels.get(i)) {
        Some(level) => showing_for(level, &args, &ladder),
        None => Showing::Base,
    };
    if ladder.current == desired && subject.showing == showing {
        return;
    }
    ladder.current = desired;
    subject.showing = showing;
    subject.respawn(&mut commands);
}

/// What one ladder level renders as.
///
/// A level declares only what differs from the entity that named the model, and
/// the viewer has no entity — so the fallbacks are the renderer's own defaults
/// and the model's extents. A sphere level with no radius is the shipped
/// asteroid case: it stands in for the whole rock, so half the model's largest
/// extent is the honest stand-in for what the entity would have supplied.
fn showing_for(level: &LodLevel, args: &ViewerArgs, ladder: &LadderState) -> Showing {
    let scale = level.scale.map(Vec3::from_array).unwrap_or(Vec3::ONE);
    if let Some(model) = &level.model {
        return Showing::Glb {
            path: model.clone(),
            // The same fallback the game applies: the level's own variant, else
            // the one the entity (here, the panel) selected.
            variant: level.variant.clone().or_else(|| args.variant.clone()),
            scale,
        };
    }
    let Some(shape) = level.shape else {
        // Neither model nor shape: an invalid level, which the game skips.
        // Showing the base model says so more clearly than an empty screen.
        return Showing::Base;
    };
    let radius = level.radius.unwrap_or_else(|| ladder.stand_in_radius());
    Showing::Shape(ProceduralLevel {
        shape,
        radius,
        size: level.size,
        minor_radius: level
            .minor_radius
            .unwrap_or(radius * STAND_IN_TUBE_FRACTION),
        colour: level
            .colour
            .clone()
            .unwrap_or_else(|| STAND_IN_COLOUR.to_vec()),
        emissive: level.emissive.unwrap_or(DEFAULT_EMISSIVE),
        scale,
        rotation: level
            .rotation
            .map(|r| Quat::from_euler(EulerRot::XYZ, r[0], r[1], r[2]))
            .unwrap_or(Quat::IDENTITY),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entity_config::MeshShape;

    /// Drive `refresh_ladder` then `apply_lod_mode` over the real shipped
    /// sidecars, the way the viewer's schedule does.
    ///
    /// Native, so `resolve_sidecar_rig` reads the files off disk in one call
    /// rather than through the wasm fetch queue — which is what makes the
    /// engine's own view of a ladder assertable at all.
    fn run_schedule(model: &str, variant: Option<&str>, mode: LodMode) -> (usize, Showing) {
        let mut app = App::new();
        app.insert_resource(ViewerArgs {
            model: Some(model.to_string()),
            variant: variant.map(str::to_string),
            entity: None,
            gizmos: false,
        })
        .insert_resource(mode)
        .init_resource::<LadderState>()
        .init_resource::<SubjectState>()
        .add_systems(Update, (refresh_ladder, apply_lod_mode).chain());
        app.world_mut()
            .spawn((ViewerCamera, Transform::from_xyz(0.0, 0.0, 30.0)));
        app.update();

        let levels = app.world().resource::<LadderState>().levels.len();
        let showing = app.world().resource::<SubjectState>().showing.clone();
        (levels, showing)
    }

    /// The bug that made the panel's "view" button do nothing: the asteroids
    /// have no `<stem>.model.toml`, so the base variant resolves to an identity
    /// rig with no ladder — while the panel, which read whichever sidecar
    /// happened to carry one, showed four levels to click on.
    #[test]
    fn the_engine_reads_the_ladder_of_the_variant_it_was_asked_for() {
        let (base, _) = run_schedule("assets/models/asteroid_common_1.glb", None, LodMode::Base);
        assert_eq!(base, 0, "the asteroids ship no base-variant sidecar");

        let (large, _) = run_schedule(
            "assets/models/asteroid_common_1.glb",
            Some("large"),
            LodMode::Base,
        );
        assert_eq!(large, 4, "the large variant carries the four-level ladder");
    }

    #[test]
    fn fixed_mode_puts_that_level_on_screen() {
        let (_, showing) = run_schedule(
            "assets/models/asteroid_common_1.glb",
            Some("large"),
            LodMode::Fixed(2),
        );
        assert_eq!(
            showing,
            Showing::Glb {
                path: "assets/models/asteroid_common_1_lod2.glb".into(),
                variant: Some("large".into()),
                scale: Vec3::ONE,
            }
        );
    }

    /// The far level of every shipped asteroid ladder is a bare sphere.
    #[test]
    fn the_fallback_level_renders_as_a_procedural_shape() {
        let (_, showing) = run_schedule(
            "assets/models/asteroid_common_1.glb",
            Some("large"),
            LodMode::Fixed(3),
        );
        assert!(
            matches!(showing, Showing::Shape(level) if level.shape == MeshShape::Sphere),
            "the last level is `shape = \"sphere\"`",
        );
    }

    #[test]
    fn auto_mode_at_close_range_shows_the_near_level() {
        let (_, showing) = run_schedule(
            "assets/models/asteroid_common_1.glb",
            Some("large"),
            LodMode::Auto,
        );
        assert_eq!(
            showing,
            Showing::Glb {
                path: "assets/models/asteroid_common_1.glb".into(),
                variant: Some("large".into()),
                scale: Vec3::ONE,
            },
            "30 units out is inside the first band",
        );
    }

    fn ladder(levels: Vec<LodLevel>) -> LadderState {
        LadderState {
            levels,
            ..Default::default()
        }
    }

    fn glb(max_distance: Option<f32>, model: &str) -> LodLevel {
        LodLevel {
            max_distance,
            model: Some(model.to_string()),
            ..Default::default()
        }
    }

    #[test]
    fn base_mode_shows_no_level_even_with_a_ladder() {
        let state = ladder(vec![glb(Some(50.0), "a.glb"), glb(None, "b.glb")]);
        assert_eq!(state.desired(LodMode::Base, 10.0), None);
    }

    #[test]
    fn auto_mode_picks_the_band_the_distance_falls_in() {
        let state = ladder(vec![glb(Some(50.0), "a.glb"), glb(None, "b.glb")]);
        assert_eq!(state.desired(LodMode::Auto, 10.0), Some(0));
        assert_eq!(state.desired(LodMode::Auto, 500.0), Some(1));
    }

    #[test]
    fn fixed_mode_holds_its_level_at_any_distance() {
        let state = ladder(vec![glb(Some(50.0), "a.glb"), glb(None, "b.glb")]);
        assert_eq!(state.desired(LodMode::Fixed(1), 1.0), Some(1));
        assert_eq!(state.desired(LodMode::Fixed(1), 9000.0), Some(1));
    }

    /// A mode held across a model switch can name a level the new model has
    /// not got.
    #[test]
    fn a_fixed_level_past_the_end_falls_back_to_the_base_model() {
        let state = ladder(vec![glb(None, "a.glb")]);
        assert_eq!(state.desired(LodMode::Fixed(3), 10.0), None);
    }

    #[test]
    fn a_model_with_no_ladder_has_nothing_to_show_in_any_mode() {
        let state = ladder(vec![]);
        assert_eq!(state.desired(LodMode::Auto, 10.0), None);
        assert_eq!(state.desired(LodMode::Fixed(0), 10.0), None);
    }

    #[test]
    fn mode_names_round_trip() {
        for (name, mode) in [
            ("base", LodMode::Base),
            ("auto", LodMode::Auto),
            ("fixed", LodMode::Fixed(2)),
        ] {
            assert_eq!(LodMode::parse(name, 2), mode);
            assert_eq!(mode.name(), name);
        }
    }

    #[test]
    fn an_unknown_mode_name_is_the_base_model() {
        assert_eq!(LodMode::parse("nonsense", 0), LodMode::Base);
    }

    /// The shipped asteroid ladders end in a bare `shape = "sphere"` with no
    /// radius: it stands in for the whole rock, so it is sized from the model.
    #[test]
    fn a_procedural_level_with_no_radius_is_sized_from_the_model() {
        let mut state = ladder(vec![LodLevel {
            shape: Some(MeshShape::Sphere),
            ..Default::default()
        }]);
        state.extent = Some(8.0);
        let Showing::Shape(level) = showing_for(&state.levels[0], &ViewerArgs::default(), &state)
        else {
            panic!("a shape level renders as a shape");
        };
        assert_eq!(level.radius, 4.0);
    }

    #[test]
    fn a_glb_level_inherits_the_panel_variant_when_it_names_none() {
        let state = ladder(vec![glb(None, "assets/models/x_lod1.glb")]);
        let args = ViewerArgs {
            variant: Some("large".into()),
            ..Default::default()
        };
        assert_eq!(
            showing_for(&state.levels[0], &args, &state),
            Showing::Glb {
                path: "assets/models/x_lod1.glb".into(),
                variant: Some("large".into()),
                scale: Vec3::ONE,
            }
        );
    }

    /// A sphere standing in for a hull wants to be hull-shaped, which is the
    /// whole point of a per-level `scale`.
    #[test]
    fn a_shape_level_carries_its_own_xyz_scale() {
        let state = ladder(vec![LodLevel {
            shape: Some(MeshShape::Sphere),
            radius: Some(2.0),
            scale: Some([3.0, 1.0, 0.5]),
            ..Default::default()
        }]);
        let Showing::Shape(level) = showing_for(&state.levels[0], &ViewerArgs::default(), &state)
        else {
            panic!("a shape level renders as a shape");
        };
        assert_eq!(level.scale, Vec3::new(3.0, 1.0, 0.5));
    }

    /// A shape level's rotation reaches the visual — the game puts it on the
    /// mesh child rather than the entity, whose rotation the sim owns.
    #[test]
    fn a_shape_level_carries_its_own_rotation() {
        let state = ladder(vec![LodLevel {
            shape: Some(MeshShape::Sphere),
            rotation: Some([0.0, std::f32::consts::FRAC_PI_2, 0.0]),
            ..Default::default()
        }]);
        let Showing::Shape(level) = showing_for(&state.levels[0], &ViewerArgs::default(), &state)
        else {
            panic!("a shape level renders as a shape");
        };
        let expected = Quat::from_euler(EulerRot::XYZ, 0.0, std::f32::consts::FRAC_PI_2, 0.0);
        assert!(level.rotation.abs_diff_eq(expected, 1e-6));
    }

    #[test]
    fn a_shape_level_without_a_rotation_is_unrotated() {
        let state = ladder(vec![LodLevel {
            shape: Some(MeshShape::Sphere),
            ..Default::default()
        }]);
        let Showing::Shape(level) = showing_for(&state.levels[0], &ViewerArgs::default(), &state)
        else {
            panic!("a shape level renders as a shape");
        };
        assert_eq!(level.rotation, Quat::IDENTITY);
    }

    /// The colour a level declares is what it renders; omitting it inherits the
    /// entity's, which in the viewer is the neutral stand-in.
    #[test]
    fn a_shape_level_uses_its_declared_colour() {
        let state = ladder(vec![LodLevel {
            shape: Some(MeshShape::Sphere),
            colour: Some(vec![0.5, 0.25, 0.125]),
            ..Default::default()
        }]);
        let Showing::Shape(level) = showing_for(&state.levels[0], &ViewerArgs::default(), &state)
        else {
            panic!("a shape level renders as a shape");
        };
        assert_eq!(level.colour, vec![0.5, 0.25, 0.125]);
    }

    /// A level that declares no scale renders at the entity's own, which in the
    /// viewer is unity — the same "recompute, never accumulate" rule
    /// `update_mesh_lod` follows.
    #[test]
    fn a_level_without_a_scale_renders_unscaled() {
        let state = ladder(vec![glb(None, "assets/models/x.glb")]);
        assert_eq!(
            showing_for(&state.levels[0], &ViewerArgs::default(), &state),
            Showing::Glb {
                path: "assets/models/x.glb".into(),
                variant: None,
                scale: Vec3::ONE,
            }
        );
    }

    /// A level that names its own variant keeps it — that is how a ladder
    /// points at a differently-rigged far level.
    #[test]
    fn a_level_variant_wins_over_the_panel_variant() {
        let mut level = glb(None, "assets/models/x_lod1.glb");
        level.variant = Some("cosmetic".into());
        let state = ladder(vec![level]);
        let args = ViewerArgs {
            variant: Some("large".into()),
            ..Default::default()
        };
        assert_eq!(
            showing_for(&state.levels[0], &args, &state),
            Showing::Glb {
                path: "assets/models/x_lod1.glb".into(),
                variant: Some("cosmetic".into()),
                scale: Vec3::ONE,
            }
        );
    }
}
