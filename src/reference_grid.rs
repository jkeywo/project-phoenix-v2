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

/// How much of that radius is spent fading to nothing. `150` — a band wide
/// enough that the patch has no perceptible edge; the alternative is a visible
/// disc travelling with the ship, which would undo the world-locked illusion.
fn default_fade_band() -> f32 {
    150.0
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
/// over `fade_span` beyond it.
pub fn radial_fade(world_distance: f32, fade_start: f32, fade_span: f32) -> f32 {
    if world_distance >= fade_start + fade_span {
        return 0.0;
    }
    if world_distance <= fade_start {
        return 1.0;
    }
    let t = ((world_distance - fade_start) / fade_span).clamp(0.0, 1.0);
    1.0 - t * t * (3.0 - 2.0 * t)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn toml_config(body: &str) -> Result<ReferenceGridConfig, toml::de::Error> {
        toml::from_str(body)
    }

    // ── Parsing ───────────────────────────────────────────────────────────

    #[test]
    fn an_empty_table_is_the_calibrated_default() {
        let cfg = toml_config("").expect("a bare [reference_grid] is legal");
        assert_eq!(cfg, ReferenceGridConfig::default());
        assert_eq!(cfg.minor_spacing, 10.0);
        assert_eq!(cfg.major_spacing, 50.0);
    }

    #[test]
    fn every_field_is_authorable() {
        let cfg = toml_config(
            r#"
            minor_spacing = 4.0
            major_spacing = 20.0
            minor_colour = [0.1, 0.2, 0.3, 0.4]
            major_colour = [0.5, 0.6, 0.7, 0.8]
            opacity = 0.5
            patch_radius = 200.0
            fade_band = 50.0
            minor_line_width_px = 2.0
            major_line_width_px = 3.0
            "#,
        )
        .expect("parses");
        assert_eq!(cfg.minor_spacing, 4.0);
        assert_eq!(cfg.major_spacing, 20.0);
        assert_eq!(cfg.minor_colour, [0.1, 0.2, 0.3, 0.4]);
        assert_eq!(cfg.major_colour, [0.5, 0.6, 0.7, 0.8]);
        assert_eq!(cfg.opacity, 0.5);
        assert_eq!(cfg.patch_radius, 200.0);
        assert_eq!(cfg.fade_band, 50.0);
        assert_eq!(cfg.minor_line_width_px, 2.0);
        assert_eq!(cfg.major_line_width_px, 3.0);
    }

    #[test]
    fn an_unknown_field_is_refused() {
        let err = toml_config("minor_spacing = 10.0\nminor_spaceing = 5.0\n")
            .expect_err("deny_unknown_fields catches the typo");
        assert!(
            err.to_string().contains("minor_spaceing"),
            "the error should name the offending key, got: {err}"
        );
    }

    #[test]
    fn the_default_table_round_trips_through_toml() {
        let encoded = toml::to_string(&ReferenceGridConfig::default()).expect("encodes");
        let decoded: ReferenceGridConfig = toml::from_str(&encoded).expect("re-parses");
        assert_eq!(decoded, ReferenceGridConfig::default());
    }

    // ── Validation ────────────────────────────────────────────────────────

    #[test]
    fn the_shipped_defaults_validate() {
        ReferenceGridConfig::default()
            .validate()
            .expect("the defaults are what a bare table gets, so they must be legal");
    }

    #[test]
    fn a_zero_or_negative_minor_spacing_is_refused() {
        for bad in [0.0, -10.0] {
            let cfg = ReferenceGridConfig {
                minor_spacing: bad,
                ..Default::default()
            };
            let err = cfg
                .validate()
                .expect_err("a zero or negative spacing has no lines to draw");
            assert!(err.contains("minor_spacing"), "spacing {bad}: got {err}");
        }
    }

    #[test]
    fn a_zero_major_spacing_is_refused() {
        let cfg = ReferenceGridConfig {
            major_spacing: 0.0,
            ..Default::default()
        };
        let err = cfg.validate().expect_err("refused");
        assert!(err.contains("major_spacing"), "got: {err}");
    }

    #[test]
    fn a_non_finite_spacing_is_refused() {
        for bad in [f32::NAN, f32::INFINITY] {
            let cfg = ReferenceGridConfig {
                minor_spacing: bad,
                ..Default::default()
            };
            assert!(cfg.validate().is_err(), "{bad} must not pass validation");
        }
    }

    #[test]
    fn a_major_spacing_that_is_not_a_whole_multiple_of_minor_is_refused() {
        let cfg = ReferenceGridConfig {
            minor_spacing: 10.0,
            major_spacing: 35.0,
            ..Default::default()
        };
        let err = cfg.validate().expect_err("3.5 cells is not a lattice");
        assert!(err.contains("whole multiple"), "got: {err}");
    }

    #[test]
    fn a_major_spacing_finer_than_minor_is_refused() {
        let cfg = ReferenceGridConfig {
            minor_spacing: 50.0,
            major_spacing: 10.0,
            ..Default::default()
        };
        let err = cfg
            .validate()
            .expect_err("the major lattice is the coarse one");
        assert!(err.contains("finer"), "got: {err}");
    }

    #[test]
    fn a_major_spacing_equal_to_minor_is_accepted() {
        // One cell per major line is a degenerate but coherent grid: every line
        // is a major line. Nothing about it is unreadable, so it is not the
        // validator's business to refuse it.
        let cfg = ReferenceGridConfig {
            minor_spacing: 10.0,
            major_spacing: 10.0,
            ..Default::default()
        };
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn a_fade_band_wider_than_the_patch_is_refused() {
        let cfg = ReferenceGridConfig {
            patch_radius: 100.0,
            fade_band: 150.0,
            ..Default::default()
        };
        let err = cfg.validate().expect_err("refused");
        assert!(err.contains("fade_band"), "got: {err}");
    }

    #[test]
    fn a_zero_patch_radius_is_refused() {
        let cfg = ReferenceGridConfig {
            patch_radius: 0.0,
            fade_band: 0.0,
            ..Default::default()
        };
        let err = cfg.validate().expect_err("refused");
        assert!(err.contains("patch_radius"), "got: {err}");
    }

    #[test]
    fn an_out_of_range_opacity_is_refused() {
        for bad in [-0.1, 1.5] {
            let cfg = ReferenceGridConfig {
                opacity: bad,
                ..Default::default()
            };
            assert!(cfg.validate().is_err(), "opacity {bad} must be refused");
        }
    }

    #[test]
    fn an_over_range_colour_component_is_refused() {
        let cfg = ReferenceGridConfig {
            minor_colour: [1.0, 0.5, 0.5, 4.0],
            ..Default::default()
        };
        let err = cfg
            .validate()
            .expect_err("an HDR-hot grid line would bloom");
        assert!(err.contains("minor_colour"), "got: {err}");
    }

    #[test]
    fn a_zero_line_width_is_refused() {
        let cfg = ReferenceGridConfig {
            minor_line_width_px: 0.0,
            ..Default::default()
        };
        let err = cfg.validate().expect_err("refused");
        assert!(err.contains("minor_line_width_px"), "got: {err}");
    }

    // ── Derived uniform values ────────────────────────────────────────────

    #[test]
    fn fade_start_is_the_radius_less_the_band() {
        let cfg = ReferenceGridConfig {
            patch_radius: 400.0,
            fade_band: 150.0,
            ..Default::default()
        };
        assert_eq!(cfg.fade_start(), 250.0);
        assert_eq!(cfg.fade_span(), 150.0);
        assert_eq!(cfg.patch_half_size(), 400.0);
    }

    #[test]
    fn a_zero_fade_band_leaves_a_hard_edge_the_shader_can_still_divide_by() {
        let cfg = ReferenceGridConfig {
            patch_radius: 400.0,
            fade_band: 0.0,
            ..Default::default()
        };
        assert_eq!(cfg.fade_start(), 400.0);
        assert!(
            cfg.fade_span() > 0.0,
            "the span is what the shader divides by; it must never be zero"
        );
        assert_eq!(radial_fade(399.0, cfg.fade_start(), cfg.fade_span()), 1.0);
        assert_eq!(radial_fade(400.0, cfg.fade_start(), cfg.fade_span()), 0.0);
    }

    // ── Lattice maths ─────────────────────────────────────────────────────

    #[test]
    fn a_coordinate_on_a_line_is_zero_distance_from_it() {
        for coord in [0.0, 10.0, -10.0, 250.0, -1230.0] {
            assert_eq!(
                distance_to_nearest_line(coord, 10.0),
                0.0,
                "{coord} is a multiple of 10 and so sits on a line"
            );
        }
    }

    #[test]
    fn the_lattice_is_world_locked_not_ship_locked() {
        // The property the whole feature rests on. The function takes NO patch
        // centre and no ship position — a world coordinate alone decides how
        // far it is from a line, so wherever the patch is dragged the lines
        // stay put underneath it and the ship is what appears to move.
        //
        // Checked against hand-computed answers rather than against itself: at
        // 10-unit spacing, 1234.5 is 4.5 past the line at 1230, and 1237.0 is
        // 3.0 short of the line at 1240.
        for (coord, expected) in [
            (1234.5_f32, 4.5_f32),
            (1237.0, 3.0),
            (-1234.5, 4.5),
            (-1237.0, 3.0),
            (0.0, 0.0),
        ] {
            let actual = distance_to_nearest_line(coord, 10.0);
            assert!(
                (actual - expected).abs() < 1.0e-3,
                "at world {coord} expected {expected} from the nearest line, got {actual}"
            );
        }
    }

    #[test]
    fn distance_never_exceeds_half_a_cell() {
        let spacing = 10.0_f32;
        let mut coord = -37.0_f32;
        while coord < 37.0 {
            let d = distance_to_nearest_line(coord, spacing);
            assert!(
                (0.0..=spacing / 2.0 + 1.0e-4).contains(&d),
                "distance {d} out of range at {coord}"
            );
            coord += 0.37;
        }
    }

    #[test]
    fn the_major_lattice_lands_on_minor_lines() {
        // Why validation insists on a whole multiple: every major line has to
        // be a minor line too, or the grid shows paired lines a fraction apart.
        let cfg = ReferenceGridConfig::default();
        let mut major = 0.0_f32;
        while major <= 500.0 {
            assert_eq!(
                distance_to_nearest_line(major, cfg.minor_spacing),
                0.0,
                "major line at {major} does not sit on a minor line"
            );
            major += cfg.major_spacing;
        }
    }

    #[test]
    fn coverage_is_full_on_the_line_and_gone_a_width_away() {
        let world_per_px = 0.5;
        let half_width_px = 1.0;
        assert_eq!(line_coverage(0.0, half_width_px, world_per_px), 1.0);
        // One pixel away in world units is 0.5, and the half-width is one pixel.
        assert_eq!(line_coverage(0.5, half_width_px, world_per_px), 0.0);
        // Half a pixel away is half covered — the antialiasing ramp.
        assert!((line_coverage(0.25, half_width_px, world_per_px) - 0.5).abs() < 1.0e-6);
    }

    #[test]
    fn coverage_stays_bounded_for_a_degenerate_fragment() {
        // A fragment at a grazing angle can report a huge or zero derivative.
        // Neither may produce a NaN that propagates into the blend.
        assert_eq!(line_coverage(1.0, 1.0, 0.0), 0.0);
        assert_eq!(line_coverage(1.0, 0.0, 1.0), 0.0);
        assert_eq!(line_coverage(0.0, 1.0, 1.0e9), 1.0);
    }

    #[test]
    fn a_line_seen_nearly_edge_on_dims_rather_than_aliasing() {
        // As one pixel comes to span more world units, a line 2 units away
        // falls inside fewer pixel widths and so covers MORE of the fragment,
        // rising smoothly to a uniform tint instead of breaking into a moiré
        // pattern. That monotonic climb is the whole antialiasing claim.
        let mut previous = 0.0_f32;
        for world_per_px in [0.1_f32, 0.5, 1.0, 4.0, 20.0] {
            let coverage = line_coverage(2.0, 1.0, world_per_px);
            assert!(
                coverage >= previous - 1.0e-6,
                "coverage fell from {previous} to {coverage} at {world_per_px} world/px"
            );
            assert!(
                (0.0..=1.0).contains(&coverage),
                "coverage {coverage} out of range"
            );
            previous = coverage;
        }
        // The extremes. Up close one pixel spans 0.1 units, so the line is 20
        // pixels away and nowhere near this fragment: nothing drawn. Far off
        // one pixel spans 20 units, so the line is 0.1 of a pixel from centre
        // and covers all but a tenth of the half width.
        assert_eq!(line_coverage(2.0, 1.0, 0.1), 0.0);
        assert!((line_coverage(2.0, 1.0, 20.0) - 0.9).abs() < 1.0e-6);
    }

    #[test]
    fn the_radial_fade_is_full_inside_and_gone_outside() {
        let fade_start = 250.0;
        let fade_span = 150.0;
        assert_eq!(radial_fade(0.0, fade_start, fade_span), 1.0);
        assert_eq!(radial_fade(250.0, fade_start, fade_span), 1.0);
        assert_eq!(radial_fade(400.0, fade_start, fade_span), 0.0);
        assert_eq!(radial_fade(10_000.0, fade_start, fade_span), 0.0);
    }

    #[test]
    fn the_radial_fade_is_monotonic_across_the_band() {
        let (fade_start, fade_span) = (250.0_f32, 150.0_f32);
        let mut previous = 1.0_f32;
        let mut distance = 250.0_f32;
        while distance <= 400.0 {
            let fade = radial_fade(distance, fade_start, fade_span);
            assert!(
                fade <= previous + 1.0e-6,
                "fade rose from {previous} to {fade} at {distance}"
            );
            assert!((0.0..=1.0).contains(&fade), "fade {fade} out of range");
            previous = fade;
            distance += 5.0;
        }
    }

    #[test]
    fn the_fade_is_half_way_through_at_the_middle_of_the_band() {
        // smoothstep's midpoint. Asserted because it is the one point on the
        // curve a re-implementation is most likely to get subtly wrong.
        assert!((radial_fade(325.0, 250.0, 150.0) - 0.5).abs() < 1.0e-6);
    }
}
