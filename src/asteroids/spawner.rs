// Pure Rust module for generating asteroid positions in a donut-shaped field.
// No Bevy, no physics engine — input → output design for isolated unit testing.

use crate::entity_config::{AsteroidFieldShape, AsteroidTypeRef, GridConfig};
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
    pub fn from_config(cfg: &crate::entity_config::AsteroidFieldConfig) -> Option<Self> {
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
mod tests {
    use super::*;

    #[test]
    fn all_positions_within_outer_radius() {
        let result = generate_donut_field(
            100.0,
            200.0,
            0.005,
            42,
            &["gameplay.toml".to_string()],
            &["cosmetic.toml".to_string()],
        );

        for spawn in &result.spawns {
            let dist = (spawn.x * spawn.x + spawn.z * spawn.z).sqrt();
            assert!(
                dist <= 200.0,
                "Position ({}, {}) is outside outer radius, dist = {}",
                spawn.x,
                spawn.z,
                dist
            );
        }
    }

    #[test]
    fn no_positions_inside_inner_radius() {
        let result = generate_donut_field(
            100.0,
            200.0,
            0.005,
            42,
            &["gameplay.toml".to_string()],
            &["cosmetic.toml".to_string()],
        );

        for spawn in &result.spawns {
            let dist = (spawn.x * spawn.x + spawn.z * spawn.z).sqrt();
            assert!(
                dist >= 100.0,
                "Position ({}, {}) is inside inner radius, dist = {}",
                spawn.x,
                spawn.z,
                dist
            );
        }
    }

    #[test]
    fn count_within_tolerance_of_area_times_density() {
        let inner = 100.0;
        let outer = 200.0;
        let density = 0.005;

        let result = generate_donut_field(
            inner,
            outer,
            density,
            42,
            &["gameplay.toml".to_string()],
            &["cosmetic.toml".to_string()],
        );

        let ring_area = PI * (outer.powi(2) - inner.powi(2));
        let expected = (ring_area * density).round() as usize;
        let actual = result.count;

        // Allow 10% tolerance
        let tolerance = (expected as f32 * 0.1).ceil() as usize;
        assert!(
            (expected >= actual && expected - actual <= tolerance)
                || (actual >= expected && actual - expected <= tolerance)
                || expected == actual,
            "Count {} not within 10% of expected {}",
            actual,
            expected
        );
    }

    #[test]
    fn same_inputs_produce_same_positions() {
        let result_a = generate_donut_field(
            100.0,
            200.0,
            0.005,
            42,
            &["gameplay.toml".to_string()],
            &["cosmetic.toml".to_string()],
        );

        let result_b = generate_donut_field(
            100.0,
            200.0,
            0.005,
            42,
            &["gameplay.toml".to_string()],
            &["cosmetic.toml".to_string()],
        );

        assert_eq!(result_a.spawns, result_b.spawns, "Determinism failed");
    }

    #[test]
    fn different_seed_offset_produces_different_layouts() {
        let result_a = generate_donut_field(
            100.0,
            200.0,
            0.005,
            42,
            &["gameplay.toml".to_string()],
            &["cosmetic.toml".to_string()],
        );

        let result_b = generate_donut_field(
            100.0,
            200.0,
            0.005,
            43,
            &["gameplay.toml".to_string()],
            &["cosmetic.toml".to_string()],
        );

        assert_ne!(
            result_a.spawns, result_b.spawns,
            "Different seeds should produce different layouts"
        );
    }

    #[test]
    fn gameplay_asteroids_come_first() {
        let result = generate_donut_field(
            100.0,
            200.0,
            0.01,
            42,
            &["gameplay1.toml".to_string(), "gameplay2.toml".to_string()],
            &["cosmetic1.toml".to_string()],
        );

        // Find the first cosmetic asteroid
        let first_cosmetic_idx = result
            .spawns
            .iter()
            .position(|s| s.config_path.contains("cosmetic"));

        // Find the last gameplay asteroid
        let last_gameplay_idx = result
            .spawns
            .iter()
            .rev()
            .position(|s| s.config_path.contains("gameplay"))
            .map(|pos| result.spawns.len() - 1 - pos);

        if let (Some(first_cosmetic), Some(last_gameplay)) = (first_cosmetic_idx, last_gameplay_idx)
        {
            assert!(
                last_gameplay < first_cosmetic,
                "Gameplay asteroids should come before cosmetic asteroids"
            );
        }
    }

    #[test]
    fn type_assignment_selects_from_provided_list() {
        let paths = vec![
            "type1.toml".to_string(),
            "type2.toml".to_string(),
            "type3.toml".to_string(),
        ];

        let result = generate_donut_field(100.0, 200.0, 0.01, 42, &paths, &[]);

        for spawn in &result.spawns {
            assert!(
                paths.contains(&spawn.config_path),
                "Type path {} not in provided list",
                spawn.config_path
            );
        }
    }

    #[test]
    fn empty_type_paths_produces_no_asteroids() {
        let result = generate_donut_field(100.0, 200.0, 0.01, 42, &[], &[]);

        assert_eq!(result.count, 0);
        assert!(result.spawns.is_empty());
    }

    #[test]
    fn only_gameplay_types_produces_only_gameplay() {
        let result =
            generate_donut_field(100.0, 200.0, 0.01, 42, &["gameplay.toml".to_string()], &[]);

        for spawn in &result.spawns {
            assert!(
                spawn.config_path.contains("gameplay"),
                "Should only have gameplay types"
            );
        }
    }

    #[test]
    fn only_cosmetic_types_produces_only_cosmetic() {
        let result =
            generate_donut_field(100.0, 200.0, 0.01, 42, &[], &["cosmetic.toml".to_string()]);

        for spawn in &result.spawns {
            assert!(
                spawn.config_path.contains("cosmetic"),
                "Should only have cosmetic types"
            );
        }
    }

    #[test]
    fn uuids_are_deterministic() {
        let uuids_a = generate_donut_uuids(100.0, 200.0, 0.005, 42, 10);
        let uuids_b = generate_donut_uuids(100.0, 200.0, 0.005, 42, 10);
        assert_eq!(uuids_a, uuids_b);
    }

    #[test]
    fn uuids_are_unique() {
        let uuids = generate_donut_uuids(100.0, 200.0, 0.005, 42, 50);
        let mut seen = std::collections::HashSet::new();
        for id in &uuids {
            assert!(seen.insert(id.as_str()), "Duplicate UUID: {}", id);
        }
    }

    #[test]
    fn uuids_have_expected_format() {
        let uuids = generate_donut_uuids(100.0, 200.0, 0.005, 42, 5);
        for id in &uuids {
            let parts: Vec<&str> = id.split('-').collect();
            assert_eq!(parts.len(), 5, "UUID should have 5 parts: {}", id);
            assert_eq!(parts[0].len(), 8);
            assert_eq!(parts[1].len(), 4);
            assert_eq!(parts[2].len(), 4);
            assert_eq!(parts[3].len(), 4);
            assert_eq!(parts[4].len(), 12);
        }
    }

    #[test]
    fn different_seed_produces_different_uuids() {
        let uuids_a = generate_donut_uuids(100.0, 200.0, 0.005, 42, 10);
        let uuids_b = generate_donut_uuids(100.0, 200.0, 0.005, 43, 10);
        assert_ne!(uuids_a, uuids_b);
    }

    #[test]
    fn zero_count_returns_empty() {
        let result = generate_donut_field(
            100.0,
            200.0,
            0.0,
            42,
            &["gameplay.toml".to_string()],
            &["cosmetic.toml".to_string()],
        );
        assert!(result.spawns.is_empty());
        assert_eq!(result.count, 0);
    }

    #[test]
    fn eval_cell_deterministic() {
        let grid = GridConfig {
            resolution: 15.0,
            fill_gameplay: 0.4,
            fill_cosmetic: 0.15,
            uniformity: 0.3,
            noise_freq: 0.02,
            noise_octaves: 3,
            density_noise_freq: 0.01,
            density_noise_octaves: 2,
            jitter: 10.0,
            cosmetic_y_offset: 15.0,
            gameplay_y_variance: 0.0,
            spawn_cells: 10,
            despawn_cells: 12,
        };
        let a = eval_cell(
            0,
            5,
            3,
            &grid,
            100.0,
            200.0,
            &["gameplay.toml".to_string()],
            &[],
        );
        let b = eval_cell(
            0,
            5,
            3,
            &grid,
            100.0,
            200.0,
            &["gameplay.toml".to_string()],
            &[],
        );
        assert_eq!(a, b, "eval_cell must be deterministic");
    }

    #[test]
    fn eval_cell_returns_none_for_failed_density() {
        let grid = GridConfig {
            resolution: 15.0,
            fill_gameplay: 1.0,
            fill_cosmetic: 1.0,
            uniformity: 1.0,
            noise_freq: 0.02,
            noise_octaves: 1,
            density_noise_freq: 0.01,
            density_noise_octaves: 1,
            jitter: 10.0,
            cosmetic_y_offset: 15.0,
            gameplay_y_variance: 0.0,
            spawn_cells: 10,
            despawn_cells: 12,
        };
        let result = eval_cell(
            0,
            5,
            3,
            &grid,
            100.0,
            200.0,
            &["gameplay.toml".to_string()],
            &[],
        );
        assert!(result.is_none(), "Should be None when fill is 1.0");
    }

    #[test]
    fn eval_cell_returns_some_for_gameplay() {
        let grid = GridConfig {
            resolution: 15.0,
            fill_gameplay: 0.0,
            fill_cosmetic: 0.0,
            uniformity: 0.0,
            noise_freq: 0.02,
            noise_octaves: 1,
            density_noise_freq: 0.01,
            density_noise_octaves: 1,
            jitter: 0.0,
            cosmetic_y_offset: 15.0,
            gameplay_y_variance: 0.0,
            spawn_cells: 10,
            despawn_cells: 12,
        };
        let result = eval_cell(
            0,
            5,
            3,
            &grid,
            100.0,
            200.0,
            &["gameplay.toml".to_string()],
            &[],
        );
        assert!(result.is_some(), "Should be Some when fill is 0.0");
        let spawn = result.unwrap();
        assert_eq!(spawn.x, 75.0, "Cell center X = cx * resolution");
        assert_eq!(spawn.z, 45.0, "Cell center Z = cz * resolution");
        assert_eq!(spawn.y, 0.0, "Gameplay Y must be 0 when variance is 0");
        assert!(spawn.config_path.contains("gameplay"));
    }

    #[test]
    fn grid_positions_within_torus_bounds() {
        let grid = GridConfig {
            resolution: 15.0,
            fill_gameplay: 0.4,
            fill_cosmetic: 0.15,
            uniformity: 0.3,
            noise_freq: 0.02,
            noise_octaves: 3,
            density_noise_freq: 0.01,
            density_noise_octaves: 2,
            jitter: 10.0,
            cosmetic_y_offset: 15.0,
            gameplay_y_variance: 0.0,
            spawn_cells: 10,
            despawn_cells: 12,
        };
        let result = generate_grid_field(
            100.0,
            200.0,
            grid,
            42,
            &["gameplay.toml".to_string()],
            &["cosmetic.toml".to_string()],
        );
        for spawn in &result.gameplay {
            let dist = (spawn.x * spawn.x + spawn.z * spawn.z).sqrt();
            assert!(
                (100.0..=200.0).contains(&dist),
                "Gameplay pos ({}, {}) dist={} outside torus [100,200]",
                spawn.x,
                spawn.z,
                dist
            );
        }
        for spawn in &result.cosmetic_upper {
            let dist = (spawn.x * spawn.x + spawn.z * spawn.z).sqrt();
            assert!(
                (100.0..=200.0).contains(&dist),
                "Cosmetic upper pos ({}, {}) dist={} outside torus [100,200]",
                spawn.x,
                spawn.z,
                dist
            );
        }
        for spawn in &result.cosmetic_lower {
            let dist = (spawn.x * spawn.x + spawn.z * spawn.z).sqrt();
            assert!(
                (100.0..=200.0).contains(&dist),
                "Cosmetic lower pos ({}, {}) dist={} outside torus [100,200]",
                spawn.x,
                spawn.z,
                dist
            );
        }
    }

    #[test]
    fn grid_same_params_same_output() {
        let grid = GridConfig {
            resolution: 15.0,
            fill_gameplay: 0.4,
            fill_cosmetic: 0.15,
            uniformity: 0.3,
            noise_freq: 0.02,
            noise_octaves: 3,
            density_noise_freq: 0.01,
            density_noise_octaves: 2,
            jitter: 10.0,
            cosmetic_y_offset: 15.0,
            gameplay_y_variance: 0.0,
            spawn_cells: 10,
            despawn_cells: 12,
        };
        let result_a = generate_grid_field(
            100.0,
            200.0,
            grid.clone(),
            42,
            &["gameplay.toml".to_string()],
            &["cosmetic.toml".to_string()],
        );
        let result_b = generate_grid_field(
            100.0,
            200.0,
            grid,
            42,
            &["gameplay.toml".to_string()],
            &["cosmetic.toml".to_string()],
        );
        assert_eq!(
            result_a.gameplay, result_b.gameplay,
            "Gameplay must be deterministic"
        );
        assert_eq!(
            result_a.cosmetic_upper, result_b.cosmetic_upper,
            "Cosmetic upper must be deterministic"
        );
        assert_eq!(
            result_a.cosmetic_lower, result_b.cosmetic_lower,
            "Cosmetic lower must be deterministic"
        );
        assert_eq!(result_a.count, result_b.count);
    }

    #[test]
    fn grid_cosmetic_y_offsets_correct_sign() {
        let grid = GridConfig {
            resolution: 15.0,
            fill_gameplay: 0.0,
            fill_cosmetic: 1.0,
            uniformity: 0.0,
            noise_freq: 0.02,
            noise_octaves: 1,
            density_noise_freq: 0.01,
            density_noise_octaves: 1,
            jitter: 0.0,
            cosmetic_y_offset: 15.0,
            gameplay_y_variance: 0.0,
            spawn_cells: 10,
            despawn_cells: 12,
        };
        let result =
            generate_grid_field(100.0, 200.0, grid, 42, &[], &["cosmetic.toml".to_string()]);
        for spawn in &result.gameplay {
            assert_eq!(spawn.y, 0.0, "Gameplay Y must be 0");
        }
        for spawn in &result.cosmetic_upper {
            assert!(
                spawn.y > 0.0 && spawn.y <= 15.0,
                "Cosmetic upper Y={} must be in (0, 15]",
                spawn.y
            );
        }
        for spawn in &result.cosmetic_lower {
            assert!(
                spawn.y < 0.0 && spawn.y >= -15.0,
                "Cosmetic lower Y={} must be in [-15, 0)",
                spawn.y
            );
        }
    }

    #[test]
    fn eval_cell_gameplay_y_within_variance() {
        let variance = 0.5;
        let grid = GridConfig {
            resolution: 15.0,
            fill_gameplay: 0.0,
            fill_cosmetic: 0.0,
            uniformity: 0.0,
            noise_freq: 0.02,
            noise_octaves: 1,
            density_noise_freq: 0.01,
            density_noise_octaves: 1,
            jitter: 0.0,
            cosmetic_y_offset: 15.0,
            gameplay_y_variance: variance,
            spawn_cells: 10,
            despawn_cells: 12,
        };
        let result = eval_cell(
            0,
            5,
            3,
            &grid,
            100.0,
            200.0,
            &["gameplay.toml".to_string()],
            &[],
        );
        let spawn = result.expect("Should spawn when fill is 0.0");
        assert!(
            spawn.y.abs() <= variance,
            "Gameplay Y={} must be within variance ±{}",
            spawn.y,
            variance
        );
    }

    // ── Torus shape eligibility tests ──────────────────────────────────────

    #[test]
    fn cell_in_field_none_shape_uses_center_distance() {
        // Cell placement-point at (75, 45): distance ≈ 87.46. With inner=100,
        // the legacy centre-distance test rejects it (87 < 100).
        assert!(!cell_in_field(5, 3, 15.0, 100.0, 200.0, None));

        // Cell placement-point at (150, 150): distance ≈ 212. With outer=200,
        // the legacy centre-distance test rejects it (212 > 200).
        assert!(!cell_in_field(10, 10, 15.0, 100.0, 200.0, None));

        // Cell placement at (75, 75) → dist ≈ 106, inside the [100, 200] annulus.
        assert!(cell_in_field(5, 5, 15.0, 100.0, 200.0, None));
    }

    #[test]
    fn cell_in_field_torus_admits_cell_straddling_inner_radius() {
        // Cell at gx=7, gz=0 with resolution=15:
        //   placement centre = (105, 0)
        //   bbox = [97.5..112.5] × [-7.5..7.5]
        //   nearest corner from origin = (97.5, 0) → dist = 97.5
        //   farthest corner = (112.5, 7.5) → dist ≈ 112.75
        // With inner_radius=100, the bbox straddles the inner boundary.
        // Torus admits it.
        assert!(cell_in_field(
            7,
            0,
            15.0,
            100.0,
            200.0,
            Some(AsteroidFieldShape::Torus)
        ));
    }

    #[test]
    fn cell_in_field_torus_rejects_cell_fully_inside_inner_radius() {
        // Cell at gx=0, gz=0 with resolution=10:
        //   bbox centred at origin = [-5..5] × [-5..5]
        //   farthest corner = (5, 5) → dist ≈ 7.07
        // With inner_radius=50, the entire bbox is inside the inner hole.
        assert!(!cell_in_field(
            0,
            0,
            10.0,
            50.0,
            200.0,
            Some(AsteroidFieldShape::Torus)
        ));
    }

    #[test]
    fn cell_in_field_torus_rejects_cell_fully_outside_outer_radius() {
        // Cell at gx=20, gz=20 with resolution=15:
        //   placement centre = (300, 300)
        //   bbox = [292.5..307.5] × [292.5..307.5]
        //   nearest corner = (292.5, 292.5) → dist ≈ 413.66
        // With outer_radius=200, the nearest corner is well beyond.
        assert!(!cell_in_field(
            20,
            20,
            15.0,
            100.0,
            200.0,
            Some(AsteroidFieldShape::Torus)
        ));
    }

    #[test]
    fn cell_in_field_torus_admits_cell_straddling_outer_radius() {
        // Cell at gx=13, gz=0 with resolution=15:
        //   placement centre = (195, 0)
        //   bbox = [187.5..202.5] × [-7.5..7.5]
        //   nearest corner = (187.5, 0) → dist = 187.5 (inside outer=200)
        //   farthest = (202.5, 7.5) → dist ≈ 202.6 (outside outer=200)
        // Straddles outer boundary. Admitted because nearest is inside.
        assert!(cell_in_field(
            13,
            0,
            15.0,
            100.0,
            200.0,
            Some(AsteroidFieldShape::Torus)
        ));
    }

    #[test]
    fn cell_in_field_torus_admits_cell_containing_origin() {
        // Cell at gx=0, gz=0 with resolution=15:
        //   bbox = [-7.5..7.5] × [-7.5..7.5] (contains origin)
        //   nearest corner distance = 0 (origin is inside the bbox)
        //   With inner_radius=10, the cell straddles the inner boundary
        //   (farthest corner at sqrt(112.5) ≈ 10.6 > 10).
        assert!(cell_in_field(
            0,
            0,
            15.0,
            10.0,
            200.0,
            Some(AsteroidFieldShape::Torus)
        ));
    }

    #[test]
    fn cell_in_field_torus_zero_inner_radius_admits_central_cells() {
        // With inner_radius = 0, no cells are "fully inside" the inner
        // hole (the hole has no area). Admit any cell whose nearest
        // corner is inside outer_radius.
        assert!(cell_in_field(
            0,
            0,
            15.0,
            0.0,
            200.0,
            Some(AsteroidFieldShape::Torus)
        ));
    }

    #[test]
    fn cell_in_field_torus_negative_coords_symmetric() {
        // Symmetry across quadrants: rejection of cells far in -X, -Z.
        assert!(!cell_in_field(
            -20,
            -20,
            15.0,
            100.0,
            200.0,
            Some(AsteroidFieldShape::Torus)
        ));
        // Cell whose bbox crosses the outer boundary on the -X side.
        assert!(cell_in_field(
            -13,
            0,
            15.0,
            100.0,
            200.0,
            Some(AsteroidFieldShape::Torus)
        ));
    }

    #[test]
    fn cell_in_field_operates_in_anchor_relative_space() {
        // PRD #397 fix 5: anchor support is implemented as a pure post-seed
        // translation. `cell_in_field` is anchor-agnostic — it tests cells
        // against the annulus centred on the field-local origin. Callers
        // (the streaming spawner) must convert the player's world position
        // into anchor-relative grid coordinates BEFORE calling it.
        //
        // This test pins the contract: a cell at anchor-relative (0,0)
        // straddles the inner radius (admitted when inner=10), and a cell
        // at anchor-relative (20,20) is far outside (rejected) — regardless
        // of where the anchor lives in world space. If a refactor ever
        // pushes anchor knowledge into `cell_in_field`, this test fails.
        let res = 15.0;

        // A cell at anchor-relative origin: bbox contains (0,0), so it is
        // admitted for a torus that includes the centre (inner=0 or small).
        assert!(
            cell_in_field(0, 0, res, 0.0, 200.0, Some(AsteroidFieldShape::Torus)),
            "anchor-relative origin cell must be admitted when inner_radius=0"
        );

        // A cell at anchor-relative (20,20) (≈ 424 from anchor) is well
        // outside outer=200 — rejected.
        assert!(
            !cell_in_field(20, 20, res, 100.0, 200.0, Some(AsteroidFieldShape::Torus)),
            "anchor-relative (20,20) is ~424 units out; must be rejected by outer_radius=200"
        );

        // A cell at anchor-relative (7,0) straddles inner_radius=100 — admitted.
        assert!(
            cell_in_field(7, 0, res, 100.0, 200.0, Some(AsteroidFieldShape::Torus)),
            "anchor-relative (7,0) straddles inner_radius=100; must be admitted"
        );

        // Symmetric: anchor-relative (-7,0) likewise straddles inner — admitted.
        assert!(
            cell_in_field(-7, 0, res, 100.0, 200.0, Some(AsteroidFieldShape::Torus)),
            "cell_in_field must be symmetric in anchor-relative space"
        );
    }

    #[test]
    fn eval_cell_is_anchor_independent_anchor_is_pure_translation() {
        // PRD #397 fix 5 / AGENTS.md rule 6: the per-cell density seed is
        // `(seed salt, gx, gz)` and MUST NOT include the anchor. The anchor
        // is a pure post-seed translation — since #913 it is applied inside
        // the composed evaluator (`covering_for_cell` translates the lattice
        // cell into field-local space; the returned position is the lattice
        // cell's world centre plus jitter), and `eval_cell` itself remains
        // anchor-free.
        //
        // This test pins three invariants:
        //   1. `eval_cell` takes no anchor parameter (compiles as-is).
        //   2. Two calls with identical (field_idx, gx, gz) return identical
        //      anchor-relative positions, regardless of what anchor a caller
        //      might add later.
        //   3. Translating the returned position by an anchor offset
        //      produces the expected world-space position — modelling the
        //      contract the composed evaluator honours.
        let grid = GridConfig {
            resolution: 15.0,
            fill_gameplay: 0.0,
            fill_cosmetic: 0.0,
            uniformity: 0.0,
            noise_freq: 0.02,
            noise_octaves: 1,
            density_noise_freq: 0.01,
            density_noise_octaves: 1,
            jitter: 0.0,
            cosmetic_y_offset: 15.0,
            gameplay_y_variance: 0.0,
            spawn_cells: 10,
            despawn_cells: 12,
        };

        // `fill_gameplay = 0.0` reliably yields Some (see
        // `eval_cell_returns_some_for_gameplay`). `jitter = 0.0` makes the
        // returned position the exact cell centre — easy to assert against.
        let (field_idx, gx, gz) = (7_u64, 8_i32, 4_i32);
        let anchor_relative = eval_cell(
            field_idx,
            gx,
            gz,
            &grid,
            100.0,
            200.0,
            &["gameplay.toml".to_string()],
            &[],
        )
        .expect("fill_gameplay=0.0 must produce Some");

        // Invariant 1: cell centre is `(gx*res, _, gz*res)` — anchor-relative,
        // anchor is not even an input to eval_cell.
        assert_eq!(anchor_relative.x, gx as f32 * grid.resolution);
        assert_eq!(anchor_relative.z, gz as f32 * grid.resolution);

        // Invariant 2: identical inputs → identical output (anchor-independent).
        let again = eval_cell(
            field_idx,
            gx,
            gz,
            &grid,
            100.0,
            200.0,
            &["gameplay.toml".to_string()],
            &[],
        )
        .expect("second call with identical (field_idx, gx, gz) must also spawn");
        assert_eq!(
            (anchor_relative.x, anchor_relative.y, anchor_relative.z),
            (again.x, again.y, again.z),
            "eval_cell must be anchor-independent: identical (field_idx, gx, gz) → identical anchor-relative position"
        );

        // Invariant 3: post-seed translation by anchor_offset gives the
        // expected world-space position — model of the contract the
        // composed evaluator honours internally: `covering_for_cell`
        // translates the lattice cell into field-local space, and the
        // returned position is the lattice cell's world centre (anchor-
        // independent) plus jitter.
        let anchor_offset = [100.0_f32, 0.0, 100.0];
        let world_x = anchor_relative.x + anchor_offset[0];
        let world_z = anchor_relative.z + anchor_offset[2];
        assert_eq!(
            world_x,
            gx as f32 * grid.resolution + 100.0,
            "world X = anchor-relative X + anchor_offset.x"
        );
        assert_eq!(
            world_z,
            gz as f32 * grid.resolution + 100.0,
            "world Z = anchor-relative Z + anchor_offset.z"
        );
        // Y is anchor-irrelevant (anchor is XZ-only by design).
    }

    #[test]
    fn generate_grid_field_with_shape_none_matches_legacy() {
        // Determinism / back-compat: when shape = None, the new variant
        // must produce identical output to the legacy `generate_grid_field`.
        let grid = GridConfig {
            resolution: 15.0,
            fill_gameplay: 0.4,
            fill_cosmetic: 0.15,
            uniformity: 0.3,
            noise_freq: 0.02,
            noise_octaves: 3,
            density_noise_freq: 0.01,
            density_noise_octaves: 2,
            jitter: 10.0,
            cosmetic_y_offset: 15.0,
            gameplay_y_variance: 0.0,
            spawn_cells: 10,
            despawn_cells: 12,
        };
        let legacy = generate_grid_field(
            100.0,
            200.0,
            grid.clone(),
            42,
            &["gameplay.toml".to_string()],
            &["cosmetic.toml".to_string()],
        );
        let shaped = generate_grid_field_with_shape(
            100.0,
            200.0,
            grid,
            42,
            &["gameplay.toml".to_string()],
            &["cosmetic.toml".to_string()],
            None,
        );
        assert_eq!(legacy.gameplay, shaped.gameplay);
        assert_eq!(legacy.cosmetic_upper, shaped.cosmetic_upper);
        assert_eq!(legacy.cosmetic_lower, shaped.cosmetic_lower);
        assert_eq!(legacy.count, shaped.count);
    }

    #[test]
    fn generate_grid_field_torus_positions_near_annulus() {
        // Torus eligibility admits cells whose bbox overlaps the annulus —
        // including cells whose centre lies just outside [r_min, r_max].
        // The per-cell jitter does not pull such positions back inside the
        // annulus (seed derivation unchanged from legacy). We assert a
        // looser bound: every position lies within one cell width of the
        // annulus.
        let grid = GridConfig {
            resolution: 15.0,
            fill_gameplay: 0.4,
            fill_cosmetic: 0.15,
            uniformity: 0.3,
            noise_freq: 0.02,
            noise_octaves: 3,
            density_noise_freq: 0.01,
            density_noise_octaves: 2,
            jitter: 10.0,
            cosmetic_y_offset: 15.0,
            gameplay_y_variance: 0.0,
            spawn_cells: 10,
            despawn_cells: 12,
        };
        let res = grid.resolution;
        let result = generate_grid_field_with_shape(
            100.0,
            200.0,
            grid,
            42,
            &["gameplay.toml".to_string()],
            &["cosmetic.toml".to_string()],
            Some(AsteroidFieldShape::Torus),
        );
        // Bounded tolerance: one cell diagonal beyond the annulus on either side.
        let tol = res * std::f32::consts::SQRT_2;
        for spawn in result
            .gameplay
            .iter()
            .chain(result.cosmetic_upper.iter())
            .chain(result.cosmetic_lower.iter())
        {
            let dist = (spawn.x * spawn.x + spawn.z * spawn.z).sqrt();
            assert!(
                dist >= 100.0 - tol && dist <= 200.0 + tol,
                "torus pos ({}, {}) dist={} outside [{}, {}]",
                spawn.x,
                spawn.z,
                dist,
                100.0 - tol,
                200.0 + tol,
            );
        }
    }

    #[test]
    fn generate_grid_field_torus_count_ge_legacy_count() {
        // The torus (bbox-overlap) admits a superset of cells compared to
        // the legacy centre-distance test, so the gameplay+cosmetic counts
        // must be ≥ the legacy counts (the same per-cell seed produces the
        // same per-cell outcome for the cells common to both).
        let grid = GridConfig {
            resolution: 15.0,
            fill_gameplay: 0.4,
            fill_cosmetic: 0.15,
            uniformity: 0.3,
            noise_freq: 0.02,
            noise_octaves: 3,
            density_noise_freq: 0.01,
            density_noise_octaves: 2,
            jitter: 10.0,
            cosmetic_y_offset: 15.0,
            gameplay_y_variance: 0.0,
            spawn_cells: 10,
            despawn_cells: 12,
        };
        let legacy = generate_grid_field_with_shape(
            100.0,
            200.0,
            grid.clone(),
            42,
            &["gameplay.toml".to_string()],
            &[],
            None,
        );
        let torus = generate_grid_field_with_shape(
            100.0,
            200.0,
            grid,
            42,
            &["gameplay.toml".to_string()],
            &[],
            Some(AsteroidFieldShape::Torus),
        );
        assert!(
            torus.gameplay.len() >= legacy.gameplay.len(),
            "torus admits a superset of cells: legacy={} torus={}",
            legacy.gameplay.len(),
            torus.gameplay.len(),
        );
    }

    // ── Composed density evaluation (#913) ─────────────────────────────────

    fn cgrid(resolution: f32, fill_gameplay: f32, jitter: f32) -> GridConfig {
        GridConfig {
            resolution,
            fill_gameplay,
            fill_cosmetic: fill_gameplay,
            uniformity: 0.3,
            noise_freq: 0.02,
            noise_octaves: 3,
            density_noise_freq: 0.01,
            density_noise_octaves: 2,
            jitter,
            cosmetic_y_offset: 15.0,
            gameplay_y_variance: 0.0,
            spawn_cells: 10,
            despawn_cells: 12,
        }
    }

    fn contribution(
        weight: f32,
        inner: f32,
        outer: f32,
        anchor: [f32; 3],
        grid: GridConfig,
        gameplay: &[&str],
        cosmetic: &[&str],
    ) -> FieldContribution {
        FieldContribution {
            weight,
            inner_radius: inner,
            outer_radius: outer,
            shape: None,
            anchor_offset: anchor,
            grid,
            gameplay_types: gameplay
                .iter()
                .copied()
                .map(AsteroidTypeRef::from)
                .collect(),
            cosmetic_types: cosmetic
                .iter()
                .copied()
                .map(AsteroidTypeRef::from)
                .collect(),
            shield_pierce: 0.0,
            random_rotation: None,
        }
    }

    /// Same, but with an authored rarity weight per gameplay type (#946).
    fn weighted_contribution(
        inner: f32,
        outer: f32,
        grid: GridConfig,
        gameplay: &[(&str, f32)],
    ) -> FieldContribution {
        FieldContribution {
            weight: 1.0,
            inner_radius: inner,
            outer_radius: outer,
            shape: None,
            anchor_offset: [0.0, 0.0, 0.0],
            grid,
            gameplay_types: gameplay
                .iter()
                .map(|(path, weight)| AsteroidTypeRef::Weighted {
                    path: (*path).to_string(),
                    weight: *weight,
                })
                .collect(),
            cosmetic_types: Vec::new(),
            shield_pierce: 0.0,
            random_rotation: None,
        }
    }

    /// Bit-compat proof: a composition of ONE origin-anchored field is the
    /// legacy evaluator. For every cell the streaming path would have
    /// admitted (`cell_in_field`), the composed result must equal
    /// `eval_cell` with the same seed salt — gameplay layer salt 0 and
    /// cosmetic-upper salt 0x0001_0000_0000 are exactly the seeds the
    /// per-field windows used for field 0, so single-field worlds keep
    /// their pre-#913 layouts bit for bit at this test's resolution (15.0,
    /// where gx*res/res round-trips exactly in f32; not guaranteed for
    /// arbitrary resolutions like 0.1).
    #[test]
    fn composed_single_field_matches_legacy_eval_cell() {
        let grid = cgrid(15.0, 0.4, 10.0);
        let fields = [contribution(
            1.0,
            100.0,
            200.0,
            [0.0, 0.0, 0.0],
            grid.clone(),
            &["gameplay.toml"],
            &["cosmetic.toml"],
        )];
        for gx in -15..=15 {
            for gz in -15..=15 {
                let eligible = cell_in_field(gx, gz, 15.0, 100.0, 200.0, None);

                let composed_gameplay =
                    eval_cell_composed(&fields, 15.0, gx, gz, ComposedLayer::Gameplay);
                let legacy_gameplay = if eligible {
                    eval_cell(
                        0,
                        gx,
                        gz,
                        &grid,
                        100.0,
                        200.0,
                        &["gameplay.toml".to_string()],
                        &[],
                    )
                } else {
                    None
                };
                assert_eq!(
                    composed_gameplay.clone().map(|(s, _)| s),
                    legacy_gameplay,
                    "gameplay parity broken at cell ({gx}, {gz})"
                );
                if let Some((_, idx)) = composed_gameplay {
                    assert_eq!(idx, 0);
                }

                let composed_cosmetic =
                    eval_cell_composed(&fields, 15.0, gx, gz, ComposedLayer::CosmeticUpper);
                let legacy_cosmetic = if eligible {
                    eval_cell(
                        0x0001_0000_0000,
                        gx,
                        gz,
                        &grid,
                        100.0,
                        200.0,
                        &[],
                        &["cosmetic.toml".to_string()],
                    )
                } else {
                    None
                };
                assert_eq!(
                    composed_cosmetic.map(|(s, _)| s),
                    legacy_cosmetic,
                    "cosmetic parity broken at cell ({gx}, {gz})"
                );
            }
        }
    }

    /// The regression the composition exists to fix: in a cell covered by
    /// two fields, the pre-#913 per-field evaluators BOTH spawned (double
    /// spawn); the composed evaluator returns exactly one spawn.
    #[test]
    fn composed_overlap_spawns_exactly_once_where_legacy_doubled() {
        let grid = cgrid(15.0, 0.0, 0.0); // fill 0 → every covered cell spawns
        let a = contribution(
            1.0,
            0.0,
            150.0,
            [0.0, 0.0, 0.0],
            grid.clone(),
            &["a.toml"],
            &[],
        );
        let b = contribution(
            1.0,
            100.0,
            250.0,
            [0.0, 0.0, 0.0],
            grid.clone(),
            &["b.toml"],
            &[],
        );

        // Cell (8, 0) sits at distance 120 — inside both annuli.
        let (gx, gz) = (8, 0);
        assert!(cell_in_field(gx, gz, 15.0, 0.0, 150.0, None));
        assert!(cell_in_field(gx, gz, 15.0, 100.0, 250.0, None));

        // Old world: each field ran its own window and its own eval — two
        // rocks in one cell.
        let legacy_a = eval_cell(0, gx, gz, &grid, 0.0, 150.0, &["a.toml".to_string()], &[]);
        let legacy_b = eval_cell(1, gx, gz, &grid, 100.0, 250.0, &["b.toml".to_string()], &[]);
        assert!(
            legacy_a.is_some() && legacy_b.is_some(),
            "per-field evaluation double-spawns this cell — precondition for the regression"
        );

        // New world: one composed evaluation, one rock.
        let composed = eval_cell_composed(&[a, b], 15.0, gx, gz, ComposedLayer::Gameplay);
        assert!(
            composed.is_some(),
            "the overlap cell must still spawn (fill is 0)"
        );
    }

    /// Same contributions → identical composed field across a full sweep.
    #[test]
    fn composed_evaluator_is_deterministic() {
        let make = || {
            vec![
                contribution(
                    2.0,
                    0.0,
                    150.0,
                    [0.0, 0.0, 0.0],
                    cgrid(15.0, 0.3, 10.0),
                    &["a.toml"],
                    &["ca.toml"],
                ),
                contribution(
                    1.0,
                    100.0,
                    250.0,
                    [50.0, 0.0, -25.0],
                    cgrid(15.0, 0.5, 5.0),
                    &["b.toml"],
                    &["cb.toml"],
                ),
            ]
        };
        let fields_a = make();
        let fields_b = make();
        for layer in [
            ComposedLayer::Gameplay,
            ComposedLayer::CosmeticUpper,
            ComposedLayer::CosmeticLower,
        ] {
            for gx in -18..=18 {
                for gz in -18..=18 {
                    assert_eq!(
                        eval_cell_composed(&fields_a, 15.0, gx, gz, layer),
                        eval_cell_composed(&fields_b, 15.0, gx, gz, layer),
                        "composed evaluation must be deterministic at ({gx}, {gz}, {layer:?})"
                    );
                }
            }
        }
    }

    /// Per-type rarity weights actually bias the draw (issue #946): with the
    /// tiers the shipped fields author — 1.0 / 0.1 / 0.01 — commons must
    /// dominate, uncommons must be clearly rarer, and rares rarer again while
    /// still appearing at all. The counts are exact, not statistical: every
    /// cell is a pure function of its coordinates, so this test either always
    /// passes or always fails.
    #[test]
    fn weighted_types_bias_selection_by_rarity_tier() {
        let fields = [weighted_contribution(
            0.0,
            900.0,
            cgrid(15.0, 0.0, 0.0),
            &[
                ("common.toml", 1.0),
                ("uncommon.toml", 0.1),
                ("rare.toml", 0.01),
            ],
        )];
        let (mut common, mut uncommon, mut rare) = (0, 0, 0);
        for gx in -40..=40 {
            for gz in -40..=40 {
                if let Some((spawn, _)) =
                    eval_cell_composed(&fields, 15.0, gx, gz, ComposedLayer::Gameplay)
                {
                    match spawn.config_path.as_str() {
                        "common.toml" => common += 1,
                        "uncommon.toml" => uncommon += 1,
                        "rare.toml" => rare += 1,
                        other => panic!("unexpected config path {other}"),
                    }
                }
            }
        }
        assert!(
            common + uncommon + rare > 5000,
            "the sample needs to be big enough for a 1:100 tier to show: {common}/{uncommon}/{rare}"
        );
        assert!(rare > 0, "a 1:100 type must still appear at all");
        assert!(
            uncommon > rare * 3,
            "rares must be visibly rarer than uncommons: uncommon={uncommon} rare={rare}"
        );
        assert!(
            common > uncommon * 3,
            "uncommons must be visibly rarer than commons: common={common} uncommon={uncommon}"
        );
    }

    /// Weighting must not cost a draw (issue #946, AGENTS.md rule 8).
    ///
    /// The type pick is resolved from ONE uniform draw mapped onto the
    /// cumulative weights, in the same slot in the per-cell sequence the old
    /// `random_range` occupied. So for any cell, a weighted list and the bare
    /// (unweighted) spelling of the same paths must agree on everything the
    /// draws around the pick decide: the jittered position drawn before it,
    /// and — the load-bearing one — the gameplay Y drawn *after* it. A pick
    /// that consumed a different amount of entropy would shift Y.
    #[test]
    fn weighting_a_type_list_does_not_shift_the_draw_sequence() {
        let mut grid = cgrid(15.0, 0.0, 20.0);
        grid.gameplay_y_variance = 5.0;
        let plain = [contribution(
            1.0,
            0.0,
            900.0,
            [0.0, 0.0, 0.0],
            grid.clone(),
            &["a.toml", "b.toml", "c.toml"],
            &[],
        )];
        let weighted = [weighted_contribution(
            0.0,
            900.0,
            grid,
            &[("a.toml", 1.0), ("b.toml", 0.1), ("c.toml", 0.01)],
        )];
        let mut differed = 0;
        for gx in -20..=20 {
            for gz in -20..=20 {
                let a = eval_cell_composed(&plain, 15.0, gx, gz, ComposedLayer::Gameplay);
                let b = eval_cell_composed(&weighted, 15.0, gx, gz, ComposedLayer::Gameplay);
                match (a, b) {
                    (None, None) => {}
                    (Some((a, _)), Some((b, _))) => {
                        assert_eq!((a.x, a.z), (b.x, b.z), "position at ({gx}, {gz})");
                        assert_eq!(a.y, b.y, "gameplay Y at ({gx}, {gz})");
                        if a.config_path != b.config_path {
                            differed += 1;
                        }
                    }
                    (a, b) => panic!("weighting changed whether ({gx}, {gz}) spawns: {a:?} {b:?}"),
                }
            }
        }
        assert!(
            differed > 0,
            "the weights must still change WHICH type is picked, or this proves nothing"
        );
    }

    /// The pick costs exactly the one draw `random_range` cost (issue #946,
    /// AGENTS.md rule 8).
    ///
    /// `weighting_a_type_list_does_not_shift_the_draw_sequence` above compares
    /// two authored *spellings* — bare paths and weighted entries — and both
    /// run through [`pick_weighted_type`], so it pins that re-weighting an
    /// authored list is free. It cannot pin what the doc comment on
    /// [`pick_weighted_type`] actually claims: that the pick sits in the same
    /// slot, at the same cost, as the pre-#946 `rng.random_range(0..len)`.
    /// A rewrite to rejection sampling or a draw per candidate would keep that
    /// test green and silently move every gameplay Y in every shipped field.
    ///
    /// So this reconstructs the pre-#946 consumption from a bare `StdRng` —
    /// the cell seed, the density draw, the jitter magnitude, then
    /// `random_range(0..len)` where the type pick goes — and asserts the Y that
    /// falls out next is the Y the evaluator produced. Y is the load-bearing
    /// one: it is drawn *after* the pick, so it moves the instant the pick's
    /// entropy cost changes. (X and Z are drawn before it and cannot.)
    #[test]
    fn the_type_pick_costs_the_same_draw_the_old_random_range_did() {
        let mut grid = cgrid(15.0, 0.0, 20.0);
        grid.gameplay_y_variance = 5.0;
        let y_variance = grid.gameplay_y_variance;
        let types: &[(&str, f32)] = &[("a.toml", 1.0), ("b.toml", 0.1), ("c.toml", 0.01)];
        let fields = [weighted_contribution(0.0, 900.0, grid, types)];

        let mut checked = 0;
        for gx in -20..=20 {
            for gz in -20..=20 {
                let Some((spawn, _)) =
                    eval_cell_composed(&fields, 15.0, gx, gz, ComposedLayer::Gameplay)
                else {
                    continue;
                };

                // The cell seed, mixed exactly as `eval_covered_cell` does
                // (the gameplay layer's salt is 0).
                let seed = {
                    let mut s = ComposedLayer::Gameplay.seed_salt();
                    s = s.wrapping_mul(2654435761);
                    s = s.wrapping_add(gx as u64);
                    s = s.wrapping_mul(2654435761);
                    s = s.wrapping_add(gz as u64);
                    s
                };
                let mut rng = StdRng::seed_from_u64(seed);
                // 1. density gate, 2. jitter magnitude — unchanged by #946.
                let _density = rng.random::<f32>();
                let _jitter_magnitude = rng.random::<f32>();
                // 3. THE PRE-#946 TYPE PICK. A single covering field means no
                //    field-selection draw sits between these.
                let _type_index = rng.random_range(0..types.len());
                // 4. gameplay Y.
                let y = (rng.random::<f32>() * 2.0 - 1.0) * y_variance;

                assert_eq!(
                    spawn.y, y,
                    "gameplay Y at ({gx}, {gz}) moved: the weighted pick no longer consumes the \
                     single draw `random_range(0..len)` consumed before issue #946"
                );
                checked += 1;
            }
        }
        assert!(
            checked > 100,
            "the sweep has to actually spawn rocks to pin anything: {checked} cells checked"
        );
    }

    /// Degenerate authoring guard, matching the field-level one: a type list
    /// whose weights are all zero falls back to a uniform draw rather than
    /// dividing by zero or erasing the field.
    #[test]
    fn zero_weighted_type_list_falls_back_to_uniform() {
        let fields = [weighted_contribution(
            0.0,
            900.0,
            cgrid(15.0, 0.0, 0.0),
            &[("a.toml", 0.0), ("b.toml", 0.0)],
        )];
        let mut seen: Vec<String> = Vec::new();
        for gx in -20..=20 {
            for gz in -20..=20 {
                if let Some((spawn, _)) =
                    eval_cell_composed(&fields, 15.0, gx, gz, ComposedLayer::Gameplay)
                {
                    if !seen.contains(&spawn.config_path) {
                        seen.push(spawn.config_path.clone());
                    }
                }
            }
        }
        seen.sort_unstable();
        assert_eq!(
            seen,
            vec!["a.toml".to_string(), "b.toml".to_string()],
            "all-zero weights must draw both types uniformly, not erase the list"
        );
    }

    /// Weight picks the contributing field: a weight-3 field should supply
    /// roughly three times as many rocks as a weight-1 field across a full
    /// overlap, and both must contribute.
    #[test]
    fn composed_weight_biases_field_selection() {
        let grid = cgrid(15.0, 0.0, 0.0);
        let fields = [
            contribution(
                3.0,
                0.0,
                300.0,
                [0.0, 0.0, 0.0],
                grid.clone(),
                &["a.toml"],
                &[],
            ),
            contribution(
                1.0,
                0.0,
                300.0,
                [0.0, 0.0, 0.0],
                grid.clone(),
                &["b.toml"],
                &[],
            ),
        ];
        let mut a_count = 0;
        let mut b_count = 0;
        for gx in -20..=20 {
            for gz in -20..=20 {
                if let Some((spawn, _)) =
                    eval_cell_composed(&fields, 15.0, gx, gz, ComposedLayer::Gameplay)
                {
                    match spawn.config_path.as_str() {
                        "a.toml" => a_count += 1,
                        "b.toml" => b_count += 1,
                        other => panic!("unexpected config path {other}"),
                    }
                }
            }
        }
        assert!(b_count > 0, "the weight-1 field must still contribute");
        assert!(
            a_count > b_count * 2,
            "weight 3 vs 1 should skew selection heavily: a={a_count} b={b_count}"
        );
    }

    /// Degenerate authoring guard: if every covering field is zero-weighted
    /// the blend falls back to uniform instead of dividing by zero.
    #[test]
    fn composed_zero_weights_fall_back_to_uniform() {
        let grid = cgrid(15.0, 0.0, 0.0);
        let fields = [
            contribution(
                0.0,
                0.0,
                300.0,
                [0.0, 0.0, 0.0],
                grid.clone(),
                &["a.toml"],
                &[],
            ),
            contribution(
                0.0,
                0.0,
                300.0,
                [0.0, 0.0, 0.0],
                grid.clone(),
                &["b.toml"],
                &[],
            ),
        ];
        let spawn = eval_cell_composed(&fields, 15.0, 3, 4, ComposedLayer::Gameplay);
        assert!(
            spawn.is_some(),
            "all-zero weights must not erase the field (uniform fallback)"
        );
    }

    /// A contribution anchored away from the origin covers the translated
    /// region and nothing else; positions come back in world space.
    #[test]
    fn composed_anchor_translates_field_coverage() {
        let grid = cgrid(25.0, 0.0, 0.0);
        let fields = [contribution(
            1.0,
            0.0,
            100.0,
            [600.0, 0.0, 0.0],
            grid,
            &["a.toml"],
            &[],
        )];

        // Cell (24, 0) → world (600, 0) → field-local (0, 0): covered.
        let hit = eval_cell_composed(&fields, 25.0, 24, 0, ComposedLayer::Gameplay)
            .expect("cell at the anchor must be covered");
        assert_eq!(
            hit.0.x, 600.0,
            "jitter 0 → spawn at the cell's world-space centre"
        );
        assert_eq!(hit.0.z, 0.0);

        // Cell (0, 0) → field-local (-600, 0): far outside the disc.
        assert!(
            eval_cell_composed(&fields, 25.0, 0, 0, ComposedLayer::Gameplay).is_none(),
            "the world origin is not covered by a field anchored at x=600"
        );
    }

    /// The shared lattice: finest authored resolution, largest authored
    /// spawn/despawn windows; empty composition has no lattice.
    #[test]
    fn composed_lattice_takes_finest_resolution_and_largest_windows() {
        let mut coarse = cgrid(25.0, 0.4, 0.0);
        coarse.spawn_cells = 10;
        coarse.despawn_cells = 12;
        let mut fine = cgrid(10.0, 0.4, 0.0);
        fine.spawn_cells = 30;
        fine.despawn_cells = 32;

        let fields = [
            contribution(1.0, 0.0, 100.0, [0.0; 3], coarse, &["a.toml"], &[]),
            contribution(1.0, 0.0, 100.0, [0.0; 3], fine, &["b.toml"], &[]),
        ];
        let lattice = composed_lattice(&fields).expect("non-empty composition has a lattice");
        assert_eq!(lattice.resolution, 10.0);
        assert_eq!(lattice.spawn_cells, 30);
        assert_eq!(lattice.despawn_cells, 32);

        assert!(composed_lattice(&[]).is_none());
    }
}
