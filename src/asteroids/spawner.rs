// Pure Rust module for generating asteroid positions in a donut-shaped field.
// No Bevy, no physics engine — input → output design for isolated unit testing.

use crate::entities::config::{AsteroidFieldShape, AsteroidTypeRef, GridConfig};
use crate::simmath;
use rand::rngs::StdRng;
use rand::Rng;
use rand::SeedableRng;
use std::f32::consts::PI;

/// A single asteroid spawn definition with position and type path.
#[derive(Clone, Debug, PartialEq)]
pub struct AsteroidSpawn {
    /// X position (meters)
    pub x: f32,
    /// Z position (meters)
    pub z: f32,
    /// Y position (meters)
    pub y: f32,
    /// Path to the entity config for this asteroid
    pub config_path: String,
}

/// Result of generating asteroid spawns for a field.
#[derive(Clone, Debug, PartialEq)]
pub struct DonutFieldResult {
    /// All asteroid spawns
    pub spawns: Vec<AsteroidSpawn>,
    /// Total number of asteroids generated
    pub count: usize,
}

/// Result of grid-based asteroid generation.
#[derive(Clone, Debug, PartialEq)]
pub struct AsteroidGridResult {
    /// Gameplay asteroids (Y=0 plane).
    pub gameplay: Vec<AsteroidSpawn>,
    /// Cosmetic asteroids in the upper layer (Y > 0).
    pub cosmetic_upper: Vec<AsteroidSpawn>,
    /// Cosmetic asteroids in the lower layer (Y < 0).
    pub cosmetic_lower: Vec<AsteroidSpawn>,
    /// Total count across all layers.
    pub count: usize,
}

/// Generate asteroid positions in a donut-shaped ring.
///
/// # Arguments
/// * `inner_radius` - Minimum distance from center (no asteroids inside this radius)
/// * `outer_radius` - Maximum distance from center (all asteroids within this radius)
/// * `density` - Asteroids per square meter of the ring area
/// * `seed_offset` - Seed offset for deterministic but independent layouts
/// * `gameplay_type_paths` - List of config paths for gameplay asteroids
/// * `cosmetic_type_paths` - List of config paths for cosmetic asteroids
///
/// # Returns
/// A `DonutFieldResult` containing all spawns. Gameplay asteroids come first,
/// then cosmetic asteroids. Within each group, order is random but deterministic.
pub fn generate_donut_field(
    inner_radius: f32,
    outer_radius: f32,
    density: f32,
    seed_offset: u64,
    gameplay_type_paths: &[String],
    cosmetic_type_paths: &[String],
) -> DonutFieldResult {
    // Calculate ring area: π * (outer² - inner²)
    let ring_area = PI * (outer_radius.powi(2) - inner_radius.powi(2));

    // Calculate expected count from density and area
    let expected_count = (ring_area * density).round() as usize;

    // Split count between gameplay and cosmetic based on available types
    // If no type paths, no asteroids of that type
    let gameplay_types_available = gameplay_type_paths.len();
    let cosmetic_types_available = cosmetic_type_paths.len();

    let (gameplay_count, cosmetic_count) =
        if gameplay_types_available > 0 && cosmetic_types_available > 0 {
            // Split roughly 70/30 gameplay/cosmetic if both available
            let gameplay = (expected_count * 7) / 10;
            let cosmetic = expected_count - gameplay;
            (gameplay, cosmetic)
        } else if gameplay_types_available > 0 {
            (expected_count, 0)
        } else if cosmetic_types_available > 0 {
            (0, expected_count)
        } else {
            (0, 0)
        };

    let total_count = gameplay_count + cosmetic_count;

    // Generate seed from parameters using a simple hash combination
    let seed = {
        let mut seed: u64 = seed_offset;
        // Combine f32 bits into the seed
        seed = seed.wrapping_add(inner_radius.to_bits() as u64);
        seed = seed.wrapping_add(outer_radius.to_bits() as u64);
        seed = seed.wrapping_add(density.to_bits() as u64);
        // Mix the bits
        seed = seed.wrapping_mul(2654435761); // Golden ratio
        seed
    };

    let mut rng = StdRng::seed_from_u64(seed);
    let mut spawns = Vec::with_capacity(total_count);

    // Generate gameplay asteroids
    for _ in 0..gameplay_count {
        let spawn =
            generate_single_spawn(&mut rng, inner_radius, outer_radius, gameplay_type_paths);
        spawns.push(spawn);
    }

    // Generate cosmetic asteroids
    for _ in 0..cosmetic_count {
        let spawn =
            generate_single_spawn(&mut rng, inner_radius, outer_radius, cosmetic_type_paths);
        spawns.push(spawn);
    }

    DonutFieldResult {
        spawns,
        count: total_count,
    }
}

// ── Composed density evaluation (#913) ──────────────────────────────────

/// One authored asteroid-field entity's contribution to the world's composed
/// density field.
///
/// Every field entity in a world feeds a single evaluator; the streaming
/// lifecycle runs one window pass over a shared world lattice and calls
/// [`eval_cell_composed`] per cell, so overlapping fields blend by `weight`
/// instead of each spawning independently (the pre-#913 double-spawn).
/// All values come from the field's `[asteroid_field]` TOML — nothing here
/// is a Rust-side tunable.
#[derive(Clone, Debug, PartialEq)]
pub struct FieldContribution {
    /// Relative blend weight (`[asteroid_field] weight`, default 1.0).
    pub weight: f32,
    pub inner_radius: f32,
    pub outer_radius: f32,
    pub shape: Option<AsteroidFieldShape>,
    /// Resolved world anchor. The contribution's eligibility annulus and
    /// noise space are translated by this offset; the lattice itself stays
    /// world-anchored.
    pub anchor_offset: [f32; 3],
    pub grid: GridConfig,
    /// Gameplay types with their authored rarity weights (issue #946).
    pub gameplay_types: Vec<AsteroidTypeRef>,
    /// Cosmetic-layer types with their authored rarity weights.
    pub cosmetic_types: Vec<AsteroidTypeRef>,
    /// Carried through so the Bevy spawn site can apply the selected
    /// contribution's collision tuning without a second config lookup.
    pub shield_pierce: f32,
    pub random_rotation: Option<[f32; 3]>,
}

impl FieldContribution {
    /// Build a contribution from an authored `[asteroid_field]` config.
    /// Returns `None` for fields without a `[asteroid_field.grid]` block —
    /// legacy donut-only fields never streamed and still do not.
    pub fn from_config(cfg: &crate::entities::config::AsteroidFieldConfig) -> Option<Self> {
        let grid = cfg.grid.clone()?;
        Some(Self {
            weight: cfg.weight,
            inner_radius: cfg.inner_radius,
            outer_radius: cfg.outer_radius,
            shape: cfg.shape,
            anchor_offset: cfg.anchor_offset,
            grid,
            gameplay_types: cfg.asteroid_type_paths.clone(),
            cosmetic_types: cfg.cosmetic_type_paths.clone(),
            shield_pierce: cfg.shield_pierce,
            random_rotation: cfg.random_rotation,
        })
    }

    fn layer_types(&self, layer: ComposedLayer) -> &[AsteroidTypeRef] {
        match layer {
            ComposedLayer::Gameplay => &self.gameplay_types,
            ComposedLayer::CosmeticUpper | ComposedLayer::CosmeticLower => &self.cosmetic_types,
        }
    }
}

/// Pick one asteroid type from a weighted list, consuming **exactly one**
/// uniform draw (issue #946).
///
/// The pre-rarity code drew `rng.random_range(0..paths.len())` here. The
/// weighted form deliberately keeps the draw *count* and its position in the
/// per-cell sequence identical — one value out of the cell's `StdRng`,
/// mapped onto the cumulative weights — rather than rejection-sampling or
/// drawing per candidate. That is the same shape the field-selection walk in
/// [`eval_covered_cell`] uses, and it is what keeps AGENTS.md rule 8 true:
/// adding, removing or re-weighting entries in an authored type list changes
/// *which* rock a cell gets, never how much entropy the cell consumes, so the
/// draws that follow (gameplay Y) stay in step.
///
/// Degenerate authoring is handled the same way as the field-level weights:
/// negative weights clamp to zero, and an all-zero list falls back to uniform
/// so a designer cannot author a divide-by-zero. Callers guarantee a
/// non-empty list (an empty type list excludes the field from the cell).
fn pick_weighted_type<'a>(types: &'a [AsteroidTypeRef], rng: &mut StdRng) -> &'a str {
    let mut total: f32 = types.iter().map(|t| t.weight().max(0.0)).sum();
    let uniform = total <= 0.0;
    if uniform {
        total = types.len() as f32;
    }
    let weight_of = |t: &AsteroidTypeRef| if uniform { 1.0 } else { t.weight().max(0.0) };

    let mut pick = rng.random::<f32>() * total;
    let mut chosen = types.len() - 1;
    for (i, t) in types.iter().enumerate() {
        let w = weight_of(t);
        if pick < w {
            chosen = i;
            break;
        }
        pick -= w;
    }
    types[chosen].path()
}

/// Which of the three asteroid layers a composed evaluation targets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ComposedLayer {
    Gameplay,
    CosmeticUpper,
    CosmeticLower,
}

impl ComposedLayer {
    /// Per-layer seed salt. These are the pre-existing per-layer seed offsets
    /// the streaming spawner already used (field 0's gameplay seed and the
    /// two cosmetic offsets), kept bit-for-bit for resolutions where
    /// gx*res/res is exact in f32 (all shipped content) so single-field
    /// origin-anchored worlds keep their exact pre-#913 layouts.
    fn seed_salt(self) -> u64 {
        match self {
            ComposedLayer::Gameplay => 0,
            ComposedLayer::CosmeticUpper => 0x0001_0000_0000,
            ComposedLayer::CosmeticLower => 0x0002_0000_0000,
        }
    }

    fn fill(self, grid: &GridConfig) -> f32 {
        match self {
            ComposedLayer::Gameplay => grid.fill_gameplay,
            ComposedLayer::CosmeticUpper | ComposedLayer::CosmeticLower => grid.fill_cosmetic,
        }
    }
}

/// Shared lattice parameters for the composed field.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ComposedLattice {
    pub resolution: f32,
    pub spawn_cells: u32,
    pub despawn_cells: u32,
}

/// Derive the shared world lattice from the authored contributions:
/// the finest authored `resolution` wins (so no field is sampled coarser
/// than its author intended) and the largest authored `spawn_cells` /
/// `despawn_cells` win (so the window always covers the widest authored
/// streaming radius). A single-field world therefore keeps exactly its own
/// authored lattice. Returns `None` when no contributions exist.
pub fn composed_lattice(fields: &[FieldContribution]) -> Option<ComposedLattice> {
    let first = fields.first()?;
    let mut lattice = ComposedLattice {
        resolution: first.grid.resolution,
        spawn_cells: first.grid.spawn_cells,
        despawn_cells: first.grid.despawn_cells,
    };
    for f in &fields[1..] {
        if f.grid.resolution > 0.0 && f.grid.resolution < lattice.resolution {
            lattice.resolution = f.grid.resolution;
        }
        lattice.spawn_cells = lattice.spawn_cells.max(f.grid.spawn_cells);
        lattice.despawn_cells = lattice.despawn_cells.max(f.grid.despawn_cells);
    }
    Some(lattice)
}

/// One field found to cover a lattice cell, with the cell's placement point
/// pre-translated into that field's local (anchor-relative) space.
struct CoveringField {
    idx: usize,
    local_x: f32,
    local_z: f32,
}

fn covering_for_cell(
    fields: &[FieldContribution],
    lattice_resolution: f32,
    cell_gx: i32,
    cell_gz: i32,
    layer: ComposedLayer,
) -> Vec<CoveringField> {
    let wx = cell_gx as f32 * lattice_resolution;
    let wz = cell_gz as f32 * lattice_resolution;
    fields
        .iter()
        .enumerate()
        .filter_map(|(idx, f)| {
            if f.layer_types(layer).is_empty() {
                return None;
            }
            let local_x = wx - f.anchor_offset[0];
            let local_z = wz - f.anchor_offset[2];
            if !region_admits_cell(
                local_x,
                local_z,
                lattice_resolution,
                f.inner_radius,
                f.outer_radius,
                f.shape,
            ) {
                return None;
            }
            Some(CoveringField {
                idx,
                local_x,
                local_z,
            })
        })
        .collect()
}

/// Evaluate one lattice cell of the composed density field.
///
/// A pure function of (authored field configs, cell position, layer): the
/// per-cell seed is `(layer salt, cell_gx, cell_gz)` and every draw comes
/// from the `StdRng` it opens — the deliberate local-seeding policy of
/// AGENTS.md Key Constraint 8. Asteroid terrain intentionally does NOT
/// consume the gated `SimRng` master seed (see the rationale in
/// `asteroids/lifecycle.rs` and `sim_rng.rs`): a rock's existence, position
/// and identity are a function of its cell alone, so two runs agree without
/// threading the RNG resource through the streaming spawner and no ungated
/// entropy source is involved.
///
/// Composition: every covering field contributes its density and fill
/// threshold, blended by weight; a cell spawns at most one rock per layer,
/// so overlapping fields can never double-spawn. On a pass, one covering
/// field is picked by the same weights and supplies the spawn tuning
/// (jitter, type list, Y placement). Returns the spawn in **world space**
/// plus the index of the selected contribution.
pub fn eval_cell_composed(
    fields: &[FieldContribution],
    lattice_resolution: f32,
    cell_gx: i32,
    cell_gz: i32,
    layer: ComposedLayer,
) -> Option<(AsteroidSpawn, usize)> {
    let covering = covering_for_cell(fields, lattice_resolution, cell_gx, cell_gz, layer);
    eval_covered_cell(
        layer.seed_salt(),
        fields,
        &covering,
        cell_gx,
        cell_gz,
        lattice_resolution,
        layer,
    )
}

/// Shared core of [`eval_cell_composed`] and the legacy [`eval_cell`]:
/// the seed, draw order and position math live here exactly once. The
/// caller supplies the covering set (already coverage-tested, or forced for
/// the legacy path which never coverage-tested inside `eval_cell`).
fn eval_covered_cell(
    seed_salt: u64,
    fields: &[FieldContribution],
    covering: &[CoveringField],
    cell_gx: i32,
    cell_gz: i32,
    lattice_resolution: f32,
    layer: ComposedLayer,
) -> Option<(AsteroidSpawn, usize)> {
    if covering.is_empty() {
        return None;
    }

    let seed = {
        let mut s = seed_salt;
        s = s.wrapping_mul(2654435761);
        s = s.wrapping_add(cell_gx as u64);
        s = s.wrapping_mul(2654435761);
        s = s.wrapping_add(cell_gz as u64);
        s
    };
    let mut rng = StdRng::seed_from_u64(seed);
    let raw_rand = rng.random::<f32>();

    // Weighted blend of density and fill across the covering fields.
    // Negative weights clamp to zero; an all-zero covering set falls back to
    // uniform weights so degenerate authoring can never divide by zero.
    let mut total: f32 = covering.iter().map(|c| fields[c.idx].weight.max(0.0)).sum();
    let uniform = total <= 0.0;
    if uniform {
        total = covering.len() as f32;
    }
    let weight_of = |c: &CoveringField| {
        if uniform {
            1.0
        } else {
            fields[c.idx].weight.max(0.0)
        }
    };

    let mut density_acc = 0.0;
    let mut fill_acc = 0.0;
    for c in covering {
        let w = weight_of(c);
        let g = &fields[c.idx].grid;
        // Noise coordinates are the field-local position in *cell units*
        // (local / resolution), matching the legacy per-cell-index sampling
        // bit-for-bit for resolutions where gx*res/res is exact in f32 (all
        // shipped resolutions, e.g. 25.0); not guaranteed for arbitrary
        // resolutions such as 0.1.
        let noise = perlin2d_octaves(
            (c.local_x / lattice_resolution) * g.density_noise_freq,
            (c.local_z / lattice_resolution) * g.density_noise_freq,
            g.density_noise_octaves,
        );
        let normalized_noise = (noise + 1.0) / 2.0;
        let d = raw_rand * g.uniformity + normalized_noise * (1.0 - g.uniformity);
        density_acc += w * d;
        fill_acc += w * layer.fill(g);
    }
    if density_acc / total < fill_acc / total {
        return None;
    }

    // Pick the contributing field by weight. A single covering field is
    // selected without a draw, keeping the single-field draw sequence
    // identical to the pre-composition evaluator.
    let sel = if covering.len() == 1 {
        0
    } else {
        let mut pick = rng.random::<f32>() * total;
        let mut chosen = covering.len() - 1;
        for (i, c) in covering.iter().enumerate() {
            let w = weight_of(c);
            if pick < w {
                chosen = i;
                break;
            }
            pick -= w;
        }
        chosen
    };
    let c = &covering[sel];
    let f = &fields[c.idx];
    let g = &f.grid;

    let jitter = compute_jitter(
        c.local_x,
        c.local_z,
        f.inner_radius,
        f.outer_radius,
        g.jitter,
        g.noise_freq,
        g.noise_octaves,
        &mut rng,
    );
    // The anchor cancels out of the position: world = local + anchor =
    // lattice cell centre. Only eligibility and noise are anchor-relative.
    let x = cell_gx as f32 * lattice_resolution + jitter.0;
    let z = cell_gz as f32 * lattice_resolution + jitter.1;

    let types = f.layer_types(layer);
    match layer {
        ComposedLayer::Gameplay => {
            let config_path = pick_weighted_type(types, &mut rng).to_string();
            let y = (rng.random::<f32>() * 2.0 - 1.0) * g.gameplay_y_variance;
            Some((
                AsteroidSpawn {
                    x,
                    z,
                    y,
                    config_path,
                },
                c.idx,
            ))
        }
        // Draw order (Y before type) matches the legacy cosmetic arm.
        // Both cosmetic layers return a positive Y; the caller negates for
        // the lower layer, as the legacy callers did.
        ComposedLayer::CosmeticUpper | ComposedLayer::CosmeticLower => {
            let y = g.cosmetic_y_offset * (0.5 + rng.random::<f32>() * 0.5);
            let config_path = pick_weighted_type(types, &mut rng).to_string();
            Some((
                AsteroidSpawn {
                    x,
                    z,
                    y,
                    config_path,
                },
                c.idx,
            ))
        }
    }
}

/// Evaluate a single world cell for asteroid content.
/// Returns `Some(AsteroidSpawn)` if this cell passes the density check,
/// or `None` if no asteroid should spawn.
///
/// The density check is deterministic: seeded from `(field_idx, cell_gx, cell_gz)`.
/// When `gameplay_type_paths` is non-empty, checks against `fill_gameplay`.
/// When only `cosmetic_type_paths` is non-empty, checks against `fill_cosmetic`.
///
/// Since #913 this is a thin wrapper over the composed evaluator with a
/// single forced-coverage contribution, so the two paths cannot drift:
/// `field_idx` becomes the seed salt and the annulus is used only for the
/// jitter clamp (eligibility was always the caller's job here).
///
/// The type lists arrive here as bare paths, which is the *unweighted*
/// spelling of [`AsteroidTypeRef`] — every type equally likely. Rarity
/// weights reach the evaluator through [`FieldContribution`], which is what
/// the streaming path builds from the authored TOML (issue #946).
pub fn eval_cell(
    field_idx: u64,
    cell_gx: i32,
    cell_gz: i32,
    grid: &GridConfig,
    inner_radius: f32,
    outer_radius: f32,
    gameplay_type_paths: &[String],
    cosmetic_type_paths: &[String],
) -> Option<AsteroidSpawn> {
    let layer = if !gameplay_type_paths.is_empty() {
        ComposedLayer::Gameplay
    } else if !cosmetic_type_paths.is_empty() {
        ComposedLayer::CosmeticUpper
    } else {
        return None;
    };
    let contribution = FieldContribution {
        weight: 1.0,
        inner_radius,
        outer_radius,
        shape: None,
        anchor_offset: [0.0, 0.0, 0.0],
        grid: grid.clone(),
        gameplay_types: gameplay_type_paths
            .iter()
            .cloned()
            .map(AsteroidTypeRef::from)
            .collect(),
        cosmetic_types: cosmetic_type_paths
            .iter()
            .cloned()
            .map(AsteroidTypeRef::from)
            .collect(),
        shield_pierce: 0.0,
        random_rotation: None,
    };
    let covering = [CoveringField {
        idx: 0,
        local_x: cell_gx as f32 * grid.resolution,
        local_z: cell_gz as f32 * grid.resolution,
    }];
    eval_covered_cell(
        field_idx,
        std::slice::from_ref(&contribution),
        &covering,
        cell_gx,
        cell_gz,
        grid.resolution,
        layer,
    )
    .map(|(spawn, _)| spawn)
}

/// Cell-eligibility test for an asteroid field.
///
/// Returns `true` when the world cell at integer grid coordinates
/// `(cell_gx, cell_gz)` is eligible to contain asteroids for the given
/// field shape.
///
/// - `None`: the legacy disc/annulus test based on the cell **placement
///   point** distance to the world origin. Equivalent to
///   `inner_radius <= dist(placement) <= outer_radius`, where the
///   placement point is `(cell_gx * resolution, cell_gz * resolution)`.
///   Preserves the historical default for asteroid fields whose TOML
///   omits `shape`.
/// - `Some(Torus)`: bounding-box overlap with the annulus. A cell is
///   admitted iff its XZ bounding box overlaps the annulus
///   `[inner_radius, outer_radius]` around the world origin. Cells whose
///   bounding box lies entirely inside `inner_radius`, or whose nearest
///   bbox corner is beyond `outer_radius`, are rejected. All other
///   cells are admitted.
///
/// For torus eligibility the bbox is centred on the cell placement point
/// (the position where `eval_cell` writes the asteroid before jitter):
/// `[gx*res − res/2, gx*res + res/2] × [gz*res − res/2, gz*res + res/2]`.
/// This makes the eligibility test geometrically consistent with the
/// per-asteroid jitter clamp in `compute_jitter`, which assumes the
/// placement point sits inside the annulus and pulls jittered positions
/// back across the boundary if needed.
pub fn cell_in_field(
    cell_gx: i32,
    cell_gz: i32,
    resolution: f32,
    inner_radius: f32,
    outer_radius: f32,
    shape: Option<AsteroidFieldShape>,
) -> bool {
    region_admits_cell(
        cell_gx as f32 * resolution,
        cell_gz as f32 * resolution,
        resolution,
        inner_radius,
        outer_radius,
        shape,
    )
}

/// Position-form twin of [`cell_in_field`]: the same eligibility test, taking
/// the cell's placement point in *field-local* coordinates rather than integer
/// lattice indices. The composed evaluator needs this form because a shared
/// world lattice cell lands at non-integer field-local coordinates whenever a
/// field is anchored away from the origin.
fn region_admits_cell(
    centre_x: f32,
    centre_z: f32,
    resolution: f32,
    inner_radius: f32,
    outer_radius: f32,
    shape: Option<AsteroidFieldShape>,
) -> bool {
    match shape {
        None => {
            let dist = (centre_x * centre_x + centre_z * centre_z).sqrt();
            dist >= inner_radius && dist <= outer_radius
        }
        Some(AsteroidFieldShape::Torus) => {
            let half = resolution * 0.5;
            let min_x = centre_x - half;
            let max_x = centre_x + half;
            let min_z = centre_z - half;
            let max_z = centre_z + half;

            // Squared nearest-corner distance from origin to the cell bbox.
            // If the origin is inside the bbox on an axis, the contribution is 0.
            let nearest_x = if 0.0 < min_x {
                min_x
            } else if 0.0 > max_x {
                max_x
            } else {
                0.0
            };
            let nearest_z = if 0.0 < min_z {
                min_z
            } else if 0.0 > max_z {
                max_z
            } else {
                0.0
            };
            let nearest_sq = nearest_x * nearest_x + nearest_z * nearest_z;

            // Squared farthest-corner distance from origin: the bbox corner
            // whose component magnitudes are maximal on each axis.
            let far_x = if min_x.abs() > max_x.abs() {
                min_x
            } else {
                max_x
            };
            let far_z = if min_z.abs() > max_z.abs() {
                min_z
            } else {
                max_z
            };
            let farthest_sq = far_x * far_x + far_z * far_z;

            let inner_sq = inner_radius * inner_radius;
            let outer_sq = outer_radius * outer_radius;

            // Reject: cell entirely outside outer radius
            // (nearest corner is beyond outer_radius).
            if nearest_sq > outer_sq {
                return false;
            }
            // Reject: cell entirely inside inner hole
            // (farthest corner is still inside inner_radius).
            if farthest_sq < inner_sq {
                return false;
            }
            true
        }
    }
}

/// Generate asteroid positions using a grid + Perlin noise system.
///
/// Grid cells within the bounding box (inner_radius..outer_radius on the XZ plane)
/// are tested for spawn eligibility. Cells outside the torus (inner hole or beyond
/// outer_radius) are skipped. Each passing cell spawns at its center position plus
/// a jitter offset derived from spatial Perlin noise.
///
/// Calls `eval_cell` internally for each cell-layer combination.
pub fn generate_grid_field(
    inner_radius: f32,
    outer_radius: f32,
    grid: GridConfig,
    _seed_offset: u64,
    gameplay_type_paths: &[String],
    cosmetic_type_paths: &[String],
) -> AsteroidGridResult {
    generate_grid_field_with_shape(
        inner_radius,
        outer_radius,
        grid,
        _seed_offset,
        gameplay_type_paths,
        cosmetic_type_paths,
        None,
    )
}

/// Variant of [`generate_grid_field`] that accepts an explicit shape.
///
/// `shape = None` preserves the historical cell-centre eligibility test
/// (back-compat with TOMLs that do not declare a `shape`).
/// `shape = Some(Torus)` uses bbox-overlap eligibility — admit any cell
/// whose XZ bounding box overlaps the annulus.
pub fn generate_grid_field_with_shape(
    inner_radius: f32,
    outer_radius: f32,
    grid: GridConfig,
    _seed_offset: u64,
    gameplay_type_paths: &[String],
    cosmetic_type_paths: &[String],
    shape: Option<AsteroidFieldShape>,
) -> AsteroidGridResult {
    let r_min = inner_radius;
    let r_max = outer_radius;
    let res = grid.resolution;

    let half_extent = r_max;
    let min_cell_x = (-half_extent / res).floor() as i32;
    let max_cell_x = (half_extent / res).floor() as i32;
    let min_cell_z = (-half_extent / res).floor() as i32;
    let max_cell_z = (half_extent / res).floor() as i32;

    let mut gameplay = Vec::new();
    let mut cosmetic_upper = Vec::new();
    let mut cosmetic_lower = Vec::new();

    let mut cell_id: u64 = 0;
    for cx in min_cell_x..=max_cell_x {
        for cz in min_cell_z..=max_cell_z {
            if !cell_in_field(cx, cz, res, r_min, r_max, shape) {
                continue;
            }

            if !gameplay_type_paths.is_empty() {
                if let Some(spawn) = eval_cell(
                    cell_id * 3,
                    cx,
                    cz,
                    &grid,
                    r_min,
                    r_max,
                    gameplay_type_paths,
                    &[],
                ) {
                    gameplay.push(spawn);
                }
            }

            if !cosmetic_type_paths.is_empty() {
                if let Some(spawn) = eval_cell(
                    cell_id * 3 + 1,
                    cx,
                    cz,
                    &grid,
                    r_min,
                    r_max,
                    &[],
                    cosmetic_type_paths,
                ) {
                    cosmetic_upper.push(spawn);
                }
                if let Some(mut spawn) = eval_cell(
                    cell_id * 3 + 2,
                    cx,
                    cz,
                    &grid,
                    r_min,
                    r_max,
                    &[],
                    cosmetic_type_paths,
                ) {
                    spawn.y = -spawn.y;
                    cosmetic_lower.push(spawn);
                }
            }

            cell_id += 1;
        }
    }

    let count = gameplay.len() + cosmetic_upper.len() + cosmetic_lower.len();
    AsteroidGridResult {
        gameplay,
        cosmetic_upper,
        cosmetic_lower,
        count,
    }
}

/// Compute jitter offset using spatial perlin noise, clamped so the final
/// position cannot go outside the [r_min, r_max] torus.
fn compute_jitter(
    cell_center_x: f32,
    cell_center_z: f32,
    r_min: f32,
    r_max: f32,
    jitter: f32,
    freq: f32,
    octaves: u32,
    rng: &mut StdRng,
) -> (f32, f32) {
    let dist = (cell_center_x * cell_center_x + cell_center_z * cell_center_z).sqrt();
    let max_push_inward = if dist > r_min { dist - r_min } else { 0.0 };
    let max_push_outward = r_max - dist;
    // Clamped at zero: a torus-admitted straddling cell can have its centre
    // OUTSIDE the annulus, making `max_push_outward` negative. An unclamped
    // negative budget flipped the jitter direction and moved rocks even when
    // the authored `jitter` was 0 — two rocks could land in one lattice cell.
    // For legacy (shape-omitted) fields the centre is always inside the
    // annulus, so the clamp never engages there.
    let max_jitter = max_push_inward.min(max_push_outward).min(jitter).max(0.0);
    let angle = perlin2d_octaves(cell_center_x * freq, cell_center_z * freq, octaves) * PI;
    let magnitude = rng.random::<f32>() * max_jitter;
    (
        simmath::cos(angle) * magnitude,
        simmath::sin(angle) * magnitude,
    )
}

/// Compute perlin noise with octaves.
fn perlin2d_octaves(x: f32, y: f32, octaves: u32) -> f32 {
    use noise::NoiseFn;
    let source = noise::Perlin::new(0);
    let mut result = 0.0;
    let mut amp = 1.0;
    let mut freq = 1.0;
    let mut max_amp = 0.0;
    for _ in 0..octaves {
        result += source.get([x as f64 * freq, y as f64 * freq]) as f32 * amp;
        max_amp += amp;
        amp *= 0.5;
        freq *= 2.0;
    }
    result / max_amp
}

/// Generate a single asteroid spawn at a random position in the donut.
fn generate_single_spawn(
    rng: &mut StdRng,
    inner_radius: f32,
    outer_radius: f32,
    type_paths: &[String],
) -> AsteroidSpawn {
    // Generate random angle
    let angle: f32 = rng.random_range(0.0..(2.0 * PI));

    // Generate random radius in the ring
    // Use sqrt to get uniform distribution across the area
    let radius_range = outer_radius - inner_radius;
    let radius = inner_radius + (rng.random::<f32>().sqrt() * radius_range);

    // Convert polar to cartesian
    let x = radius * simmath::cos(angle);
    let z = radius * simmath::sin(angle);

    // Select a random type path. The donut generator takes bare paths, which
    // is the unweighted spelling of an authored type list, so the pick is
    // uniform here by construction — rarity weights reach the evaluator
    // through `FieldContribution` (issue #946).
    let config_path = if type_paths.is_empty() {
        String::new()
    } else {
        type_paths[rng.random_range(0..type_paths.len())].clone()
    };

    AsteroidSpawn {
        x,
        z,
        y: 0.0,
        config_path,
    }
}

/// Generate deterministic UUID strings for asteroids in a donut field.
///
/// Uses the same seed as `generate_donut_field` plus a UUID-specific offset
/// to ensure deterministic but independent UUID generation.
pub fn generate_donut_uuids(
    inner_radius: f32,
    outer_radius: f32,
    density: f32,
    seed_offset: u64,
    count: usize,
) -> Vec<String> {
    // Generate seed using a simple hash combination
    let seed = {
        let mut seed: u64 = seed_offset;
        seed = seed.wrapping_add(inner_radius.to_bits() as u64);
        seed = seed.wrapping_add(outer_radius.to_bits() as u64);
        seed = seed.wrapping_add(density.to_bits() as u64);
        seed = seed.wrapping_mul(2654435761); // Golden ratio
        seed
    };

    // Use a secondary seed offset for UUIDs
    let uuid_seed = seed.wrapping_add(0xDEAD_BEEF_CAFE_1234);
    let mut rng = StdRng::seed_from_u64(uuid_seed);

    (0..count)
        .map(|_| {
            let a: u64 = rng.random();
            let b: u64 = rng.random();
            // Format as UUID v4-like string (8-4-4-4-12 hex groups)
            let bytes: [u8; 16] = [
                (a >> 56) as u8, (a >> 48) as u8, (a >> 40) as u8, (a >> 32) as u8,
                (a >> 24) as u8, (a >> 16) as u8,
                // Set version bits (4) in byte 6
                0x40 | ((a >> 8) as u8 & 0x0f), a as u8,
                // Set variant bits in byte 8
                0x80 | ((b >> 56) as u8 & 0x3f),
                (b >> 48) as u8, (b >> 40) as u8, (b >> 32) as u8,
                (b >> 24) as u8, (b >> 16) as u8, (b >> 8) as u8, b as u8,
            ];
            format!(
                "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
                bytes[0], bytes[1], bytes[2], bytes[3],
                bytes[4], bytes[5],
                bytes[6], bytes[7],
                bytes[8], bytes[9],
                bytes[10], bytes[11], bytes[12], bytes[13], bytes[14], bytes[15],
            )
        })
        .collect()
}

#[cfg(test)]
#[path = "spawner_tests.rs"]
mod tests;
