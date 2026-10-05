// The viewscreen's reference grid: the authored `[reference_grid]` table, its
// validation, and the world-lattice maths the shader mirrors.
//
// Space is featureless. With nothing but a skybox behind it a ship under thrust
// reads as stationary, and the crew lose the one cue that tells them the helm
// is doing anything at all. The grid is that cue: a faint lattice lying in the
// `y = 0` plane, LOCKED TO WORLD COORDINATES, that the ship visibly slides over.
//
// Two halves that must not be confused:
//
// - **The lines are world-aligned.** A line sits at every integer multiple of
//   `minor_spacing` along world X and world Z, forever, whatever the ship is
//   doing. That is what makes the ship appear to move; a grid that travelled
//   with the ship would convey nothing.
// - **The drawn patch follows the ship.** Drawing a lattice to infinity is not
//   a finite draw call, so one quad of `patch_radius` is kept centred under the
//   local ship and the lines are faded out over `fade_band` before its edge. The
//   patch is a window onto the lattice, not the lattice itself.
//
// This module has no Bevy dependency — it is fully unit-testable on native.
// The Bevy half (material, quad, follow system) is `server::reference_grid`,
// registered only under `SimPluginOptions::render`.

use serde::{Deserialize, Serialize};

// ── Authored defaults ─────────────────────────────────────────────────────
//
// Every one of these is the value `assets/entities/alliance_destroyer.toml`
// authors explicitly. They live here as well so that a hull which opts into the
// grid with a bare `[reference_grid]` gets the calibrated article rather than a
// zero, and so the numbers have exactly one home when they are ratified.

/// Minor line every 10 world units — the round-number idiom the radar rings
/// already read in.
fn default_minor_spacing() -> f32 {
    10.0
}

/// Major line every 50 world units: five minor cells, which is coarse enough to
/// count at a glance and fine enough that one is almost always in shot.
fn default_major_spacing() -> f32 {
    50.0
}

/// Minor line colour, RGBA, linear 0-1. A desaturated instrument blue at 10%
/// alpha — see the calibration note on [`ReferenceGridConfig`] for why the
/// alpha, not the brightness, is what carries "faint" here.
fn default_minor_colour() -> [f32; 4] {
    [0.45, 0.62, 0.78, 0.10]
}

/// Major line colour, RGBA, linear 0-1. The same hue a touch brighter and at
/// twice the alpha: "slightly brighter", not a second visual language.
fn default_major_colour() -> [f32; 4] {
    [0.55, 0.74, 0.90, 0.20]
}

/// Master opacity multiplier over both line classes. `1.0` — the per-colour
/// alphas above are already the faint values, so this exists to dim the whole
/// grid in one move without editing two colours in step.
fn default_opacity() -> f32 {
    1.0
}

/// How far the drawn patch reaches from the ship, in world units. `400` — far
/// enough that the fade edge is out past where the eye is working during a
/// manoeuvre, near enough that the lattice is still resolvable at the rim.
fn default_patch_radius() -> f32 {
    400.0
}

/// How much of that radius is spent fading to nothing. `250` — [ai] widened
/// from 150 so the fade starts much closer in (at `patch_radius - fade_band`,
/// i.e. 150 world units from the ship rather than 250): the lattice is meant to
/// dissolve *fast* toward the rim, not carry legible lines almost to the edge.
/// The alternative failure — a band too tight — leaves a visible disc
/// travelling with the ship, which would undo the world-locked illusion.
fn default_fade_band() -> f32 {
    250.0
}

/// Exponent applied to the (smoothstepped) radial fade multiplier. `2.5` —
/// [ai]. The fade curve is `smoothstep(...) ^ fade_exponent`; because the base
/// is in `[0, 1]`, an exponent above 1 pulls the whole curve down so the grid
/// is already faint a short way into the band and gone well before the rim,
/// which is the "dissolve harder with distance" John asked for. `1.0` is the
/// plain smoothstep. The endpoints are unaffected — full strength at
/// `fade_start`, exactly zero at the edge — so no exponent ever lights a bright
/// far rim.
fn default_fade_exponent() -> f32 {
    2.5
}

/// World-space height of the grid plane. `-0.5` — [ai]. The grid reads as a
/// floor lying just beneath the ship rather than a lattice co-planar with it;
/// half a world unit is enough to separate the two without the grid pulling
/// away into its own depth. The follow system holds the patch at this `y`.
fn default_plane_y() -> f32 {
    -0.5
}

/// Minor line width in PIXELS. Screen-space rather than world-space on purpose:
/// a world-space width makes near lines slabs and far lines sub-pixel shimmer,
/// and the grid is read at every distance at once.
fn default_minor_line_width_px() -> f32 {
    1.0
}

/// Major line width in pixels. Barely wider than a minor line — the major
/// lines are meant to be distinguished by brightness, with width only
/// reinforcing it.
fn default_major_line_width_px() -> f32 {
    1.6
}

// ── The table ─────────────────────────────────────────────────────────────

/// The `[reference_grid]` table on a hull's entity TOML.
///
/// Absent for every hull that carries no grid, which is every hull but the one
/// the player is flying — an NPC hull never authors this, and the render system
/// only ever consults the LOCAL ship's resolved config, so two authored copies
/// still produce one grid.
///
/// # HDR calibration
///
/// The viewscreen camera renders HDR and tonemaps through `tony_mc_mapface`
/// with bloom thresholded at `1.0` (softness `0.4`, so the knee starts around
/// `0.6`) — see [`crate::world::config::RenderConfig`]. Two consequences for
/// anyone retuning the numbers above:
///
/// - **Keep the RGB components under ~0.6.** They are pre-multiply values in a
///   linear HDR buffer. Pushed past the knee the grid starts to bloom, and a
///   navigation aid that glows is louder than the ships it is meant to sit
///   behind.
/// - **Carry "faint" in the ALPHA, not the brightness.** `tony_mc_mapface` has
///   a filmic toe that lifts near-black, so a dim-but-opaque line survives the
///   display transform far more assertively than the authored number suggests.
///   The composited value these defaults land on is roughly `0.07` linear over
///   empty space, which the transform brings up to a faint but legible grey.
///   Halving an alpha is the reliable way to make the grid quieter; halving an
///   RGB triple mostly changes its hue.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReferenceGridConfig {
    /// World-unit spacing of the minor lattice, on both X and Z.
    #[serde(default = "default_minor_spacing")]
    pub minor_spacing: f32,
    /// World-unit spacing of the major lattice. Must be a whole multiple of
    /// `minor_spacing`, so that every major line lands on a minor line and
    /// brightens it rather than sitting beside it.
    #[serde(default = "default_major_spacing")]
    pub major_spacing: f32,
    /// Minor line colour, linear RGBA 0-1.
    #[serde(default = "default_minor_colour")]
    pub minor_colour: [f32; 4],
    /// Major line colour, linear RGBA 0-1.
    #[serde(default = "default_major_colour")]
    pub major_colour: [f32; 4],
    /// Master opacity over both line classes, 0-1.
    #[serde(default = "default_opacity")]
    pub opacity: f32,
    /// Radius of the drawn patch, in world units, measured from the ship.
    #[serde(default = "default_patch_radius")]
    pub patch_radius: f32,
    /// Width of the fade band inside `patch_radius`. `0` restores a hard edge.
    #[serde(default = "default_fade_band")]
    pub fade_band: f32,
    /// Exponent on the radial fade curve; `>1` dissolves the grid faster with
    /// distance, `1.0` is a plain smoothstep. Must be positive and finite.
    #[serde(default = "default_fade_exponent")]
    pub fade_exponent: f32,
    /// World-space height of the grid plane. The follow system parks the patch
    /// at this `y`; negative sits it below the ship like a floor.
    #[serde(default = "default_plane_y")]
    pub plane_y: f32,
    /// World-space height of the grid plane while the viewscreen is in the
    /// first-person `Camera` mode. There the eye sits at hull height, so a
    /// plane half a unit under the hull slices straight through the view;
    /// dropping it further reads as a floor again. `None` (the default)
    /// keeps `plane_y` in every mode.
    #[serde(default)]
    pub first_person_plane_y: Option<f32>,
    /// Minor line width in pixels.
    #[serde(default = "default_minor_line_width_px")]
    pub minor_line_width_px: f32,
    /// Major line width in pixels.
    #[serde(default = "default_major_line_width_px")]
    pub major_line_width_px: f32,
}

impl Default for ReferenceGridConfig {
    /// Hand-written so it calls the same `default_*` fns serde does — two
    /// copies of these numbers could only ever drift apart.
    fn default() -> Self {
        Self {
            minor_spacing: default_minor_spacing(),
            major_spacing: default_major_spacing(),
            minor_colour: default_minor_colour(),
            major_colour: default_major_colour(),
            opacity: default_opacity(),
            patch_radius: default_patch_radius(),
            fade_band: default_fade_band(),
            fade_exponent: default_fade_exponent(),
            plane_y: default_plane_y(),
            first_person_plane_y: None,
            minor_line_width_px: default_minor_line_width_px(),
            major_line_width_px: default_major_line_width_px(),
        }
    }
}

/// Relative tolerance for "is a whole multiple of". Spacings are authored as
/// round decimals, so anything looser than a rounding wobble is a real mistake.
const MULTIPLE_TOLERANCE: f32 = 1.0e-4;

impl ReferenceGridConfig {
    /// Refuse an authored table that could never draw a readable grid.
    ///
    /// Called from the entity-config deserialiser, beside `[scan]`'s and
    /// `[infrastructure]`'s, so a mistake is a load failure naming the file
    /// rather than a viewscreen that quietly shows nothing for a whole mission.
    pub fn validate(&self) -> Result<(), String> {
        if !self.minor_spacing.is_finite() || self.minor_spacing <= 0.0 {
            return Err(format!(
                "[reference_grid] minor_spacing must be a positive, finite number of world \
                 units (got {}) — a lattice with no cell size has no lines to draw",
                self.minor_spacing
            ));
        }
        if !self.major_spacing.is_finite() || self.major_spacing <= 0.0 {
            return Err(format!(
                "[reference_grid] major_spacing must be a positive, finite number of world \
                 units (got {})",
                self.major_spacing
            ));
        }
        if self.major_spacing < self.minor_spacing {
            return Err(format!(
                "[reference_grid] major_spacing ({}) is finer than minor_spacing ({}) — the \
                 major lattice is the coarse one",
                self.major_spacing, self.minor_spacing
            ));
        }
        let cells = self.major_spacing / self.minor_spacing;
        if (self.major_spacing - cells.round() * self.minor_spacing).abs()
            > self.minor_spacing * MULTIPLE_TOLERANCE
        {
            return Err(format!(
                "[reference_grid] major_spacing ({}) is not a whole multiple of minor_spacing \
                 ({}) — major lines have to land on minor lines and brighten them, not drift \
                 across them",
                self.major_spacing, self.minor_spacing
            ));
        }
        if !self.patch_radius.is_finite() || self.patch_radius <= 0.0 {
            return Err(format!(
                "[reference_grid] patch_radius must be a positive, finite number of world \
                 units (got {}) — a patch with no extent draws nothing",
                self.patch_radius
            ));
        }
        if !self.fade_band.is_finite() || self.fade_band < 0.0 {
            return Err(format!(
                "[reference_grid] fade_band must be zero or a positive, finite number of \
                 world units (got {}); zero means a hard patch edge",
                self.fade_band
            ));
        }
        if self.fade_band > self.patch_radius {
            return Err(format!(
                "[reference_grid] fade_band ({}) is wider than patch_radius ({}) — the fade \
                 has to start inside the patch or the grid never reaches full strength",
                self.fade_band, self.patch_radius
            ));
        }
        if !self.fade_exponent.is_finite() || self.fade_exponent <= 0.0 {
            return Err(format!(
                "[reference_grid] fade_exponent must be a positive, finite number (got {}); it \
                 is the power the radial fade is raised to, and zero or negative would invert \
                 the fade instead of steepening it",
                self.fade_exponent
            ));
        }
        if !self.plane_y.is_finite() {
            return Err(format!(
                "[reference_grid] plane_y must be a finite world-space height (got {})",
                self.plane_y
            ));
        }
        if !(0.0..=1.0).contains(&self.opacity) {
            return Err(format!(
                "[reference_grid] opacity must be within 0-1 (got {})",
                self.opacity
            ));
        }
        for (label, width) in [
            ("minor_line_width_px", self.minor_line_width_px),
            ("major_line_width_px", self.major_line_width_px),
        ] {
            if !width.is_finite() || width <= 0.0 {
                return Err(format!(
                    "[reference_grid] {label} must be a positive, finite pixel width (got \
                     {width}) — a zero-width line is an absent line"
                ));
            }
        }
        for (label, colour) in [
            ("minor_colour", self.minor_colour),
            ("major_colour", self.major_colour),
        ] {
            for component in colour {
                if !(0.0..=1.0).contains(&component) {
                    return Err(format!(
                        "[reference_grid] {label} components must be within 0-1 (got \
                         {component}); the viewscreen is HDR, so an over-range line would \
                         bloom rather than stay faint"
                    ));
                }
            }
        }
        Ok(())
    }

    /// Distance from the patch centre at which the radial fade begins, in world
    /// units. Beyond it the grid ramps to nothing by `patch_radius`.
    ///
    /// One of the three values the material uniform is built from, so the
    /// shader never has to re-derive a clamp the validator already settled.
    pub fn fade_start(&self) -> f32 {
        (self.patch_radius - self.fade_band).clamp(0.0, self.patch_radius)
    }

    /// Width of the fade ramp, floored just above zero so the shader can divide
    /// by it unconditionally. An authored `fade_band` of `0` therefore reads as
    /// a hard edge rather than as a division by zero.
    pub fn fade_span(&self) -> f32 {
        (self.patch_radius - self.fade_start()).max(f32::MIN_POSITIVE)
    }

    /// Half the side length of the quad the patch is drawn on — the patch
    /// radius, since the fade inscribes a circle in the square and the corners
    /// are already faded out by the time they are reached.
    pub fn patch_half_size(&self) -> f32 {
        self.patch_radius
    }
}

// ── Lattice maths ─────────────────────────────────────────────────────────
//
// The three functions below are the REFERENCE IMPLEMENTATION of what
// `assets/shaders/reference_grid.wgsl` computes per fragment. They are written
// out here, and tested here, because the GPU cannot call into this crate and a
// rule that lives only in WGSL is a rule nothing in CI ever evaluates. The WGSL
// carries the same expressions and a comment pointing back at this module; if
// you change one, change the other, and the tests below are what tells you what
// the answer is supposed to be.

/// Distance, in world units, from `coord` to the nearest line of a lattice with
/// the given `spacing`.
///
/// Lines sit at every integer multiple of `spacing` including zero — that is
/// the whole world-locked claim — so the result is in `[0, spacing / 2]`.
///
/// Rust rounds halves away from zero and WGSL rounds them to even. They differ
/// only for a coordinate landing exactly on a cell midpoint, where the distance
/// is `spacing / 2` under either rule: the farthest possible point from a line,
/// which draws nothing.
pub fn distance_to_nearest_line(coord: f32, spacing: f32) -> f32 {
    let cells = coord / spacing;
    (cells - cells.round()).abs() * spacing
}

/// Coverage, 0-1, of a line whose centre is `distance` world units away, drawn
/// `half_width_px` pixels wide either side, where one pixel spans
/// `world_per_px` world units at this fragment.
///
/// The screen-space measure is what antialiases the grid for free: converting
/// the distance into pixels before comparing it to the width means a line seen
/// nearly edge-on covers a fraction of a pixel and dims, instead of aliasing
/// into a moiré pattern the way a fixed world-space width would. On the GPU
/// `world_per_px` is `fwidth()` of the coordinate; here it is a parameter, so
/// the ramp can be tested without one.
pub fn line_coverage(world_distance: f32, half_width_px: f32, world_per_px: f32) -> f32 {
    if !(world_per_px > 0.0 && half_width_px > 0.0) {
        return 0.0;
    }
    let distance_px = world_distance / world_per_px;
    (1.0 - distance_px / half_width_px).clamp(0.0, 1.0)
}

/// Radial fade multiplier, 0-1, for a fragment `distance` world units from the
/// patch centre. Full strength within `fade_start`, smoothstepped to nothing
/// over `fade_span` beyond it, then raised to `fade_exponent` so a value above
/// `1.0` dissolves the grid faster across the band. The endpoints are fixed —
/// `1.0` at `fade_start`, `0.0` at the edge — for any positive exponent, so the
/// steepening never lights a bright far rim.
///
/// `std::f32::powf` rather than `simmath::powf` (issue #908) deliberately: this
/// is a PRESENTATION reference that must match the GPU's `pow()` in
/// `reference_grid.wgsl` expression-for-expression, and its output is never
/// folded into the authoritative digest — the whole module is inert data on a
/// headless run. A deterministic polynomial approximation here would diverge
/// from the very shader this function exists to mirror.
#[allow(clippy::disallowed_methods)]
pub fn radial_fade(
    world_distance: f32,
    fade_start: f32,
    fade_span: f32,
    fade_exponent: f32,
) -> f32 {
    if world_distance >= fade_start + fade_span {
        return 0.0;
    }
    if world_distance <= fade_start {
        return 1.0;
    }
    let t = ((world_distance - fade_start) / fade_span).clamp(0.0, 1.0);
    let smooth = 1.0 - t * t * (3.0 - 2.0 * t);
    smooth.powf(fade_exponent)
}

#[cfg(test)]
#[path = "reference_grid_tests.rs"]
mod tests;
