// Pure Rust module for generating asteroid positions in a donut-shaped field.
// No Bevy, no physics engine — input → output design for isolated unit testing.

use crate::entity_config::{AsteroidFieldShape, GridConfig};
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
    
    let (gameplay_count, cosmetic_count) = if gameplay_types_available > 0 && cosmetic_types_available > 0 {
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
        let spawn = generate_single_spawn(
            &mut rng,
            inner_radius,
            outer_radius,
            gameplay_type_paths,
        );
        spawns.push(spawn);
    }
    
    // Generate cosmetic asteroids
    for _ in 0..cosmetic_count {
        let spawn = generate_single_spawn(
            &mut rng,
            inner_radius,
            outer_radius,
            cosmetic_type_paths,
        );
        spawns.push(spawn);
    }
    
    DonutFieldResult {
        spawns,
        count: total_count,
    }
}

/// Evaluate a single world cell for asteroid content.
/// Returns `Some(AsteroidSpawn)` if this cell passes the density check,
/// or `None` if no asteroid should spawn.
///
/// The density check is deterministic: seeded from `(field_idx, cell_gx, cell_gz)`.
/// When `gameplay_type_paths` is non-empty, checks against `fill_gameplay`.
/// When only `cosmetic_type_paths` is non-empty, checks against `fill_cosmetic`.
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
    let seed = {
        let mut s = field_idx;
        s = s.wrapping_mul(2654435761);
        s = s.wrapping_add(cell_gx as u64);
        s = s.wrapping_mul(2654435761);
        s = s.wrapping_add(cell_gz as u64);
        s
    };
    let mut rng = StdRng::seed_from_u64(seed);

    let cell_center_x = (cell_gx as f32) * grid.resolution;
    let cell_center_z = (cell_gz as f32) * grid.resolution;

    if !gameplay_type_paths.is_empty() {
        let density = compute_density(cell_gx, cell_gz, grid.density_noise_freq, grid.density_noise_octaves, grid.uniformity, &mut rng);
        if density >= grid.fill_gameplay {
            let jitter = compute_jitter(cell_center_x, cell_center_z, inner_radius, outer_radius, grid.jitter, grid.noise_freq, grid.noise_octaves, &mut rng);
            let x = cell_center_x + jitter.0;
            let z = cell_center_z + jitter.1;
            let config_path = gameplay_type_paths[rng.random_range(0..gameplay_type_paths.len())].clone();
            let y = (rng.random::<f32>() * 2.0 - 1.0) * grid.gameplay_y_variance;
            return Some(AsteroidSpawn { x, z, y, config_path });
        }
        return None;
    }

    if !cosmetic_type_paths.is_empty() {
        let density = compute_density(cell_gx, cell_gz, grid.density_noise_freq, grid.density_noise_octaves, grid.uniformity, &mut rng);
        if density >= grid.fill_cosmetic {
            let jitter = compute_jitter(cell_center_x, cell_center_z, inner_radius, outer_radius, grid.jitter, grid.noise_freq, grid.noise_octaves, &mut rng);
            let x = cell_center_x + jitter.0;
            let z = cell_center_z + jitter.1;
            let y_offset = grid.cosmetic_y_offset * (0.5 + rng.random::<f32>() * 0.5);
            let config_path = cosmetic_type_paths[rng.random_range(0..cosmetic_type_paths.len())].clone();
            return Some(AsteroidSpawn { x, z, y: y_offset, config_path });
        }
        return None;
    }

    None
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
    match shape {
        None => {
            let cx = cell_gx as f32 * resolution;
            let cz = cell_gz as f32 * resolution;
            let dist = (cx * cx + cz * cz).sqrt();
            dist >= inner_radius && dist <= outer_radius
        }
        Some(AsteroidFieldShape::Torus) => {
            let half = resolution * 0.5;
            let centre_x = cell_gx as f32 * resolution;
            let centre_z = cell_gz as f32 * resolution;
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
            let far_x = if min_x.abs() > max_x.abs() { min_x } else { max_x };
            let far_z = if min_z.abs() > max_z.abs() { min_z } else { max_z };
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
                if let Some(spawn) = eval_cell(cell_id * 3, cx, cz, &grid, r_min, r_max, gameplay_type_paths, &[]) {
                    gameplay.push(spawn);
                }
            }

            if !cosmetic_type_paths.is_empty() {
                if let Some(spawn) = eval_cell(cell_id * 3 + 1, cx, cz, &grid, r_min, r_max, &[], cosmetic_type_paths) {
                    cosmetic_upper.push(spawn);
                }
                if let Some(mut spawn) = eval_cell(cell_id * 3 + 2, cx, cz, &grid, r_min, r_max, &[], cosmetic_type_paths) {
                    spawn.y = -spawn.y;
                    cosmetic_lower.push(spawn);
                }
            }

            cell_id += 1;
        }
    }

    let count = gameplay.len() + cosmetic_upper.len() + cosmetic_lower.len();
    AsteroidGridResult { gameplay, cosmetic_upper, cosmetic_lower, count }
}

/// Compute the density value for a grid cell using rand + normalized perlin noise.
fn compute_density(
    cell_x: i32,
    cell_z: i32,
    freq: f32,
    octaves: u32,
    uniformity: f32,
    rng: &mut StdRng,
) -> f32 {
    let raw_rand = rng.random::<f32>();
    let noise_sample = perlin2d_octaves(cell_x as f32 * freq, cell_z as f32 * freq, octaves);
    let normalized_noise = (noise_sample + 1.0) / 2.0;
    raw_rand * uniformity + normalized_noise * (1.0 - uniformity)
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
    let max_jitter = max_push_inward.min(max_push_outward).min(jitter);
    let angle = perlin2d_octaves(cell_center_x * freq, cell_center_z * freq, octaves) * PI;
    let magnitude = rng.random::<f32>() * max_jitter;
    (angle.cos() * magnitude, angle.sin() * magnitude)
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
        result += source.get([x as f64 * freq as f64, y as f64 * freq as f64]) as f32 * amp;
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
    let x = radius * angle.cos();
    let z = radius * angle.sin();
    
    // Select a random type path
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

/// Generate deterministic UUID strings for asteroids in a grid field.
///
/// Returns (gameplay_uuids, cosmetic_upper_uuids, cosmetic_lower_uuids).
/// Uses the same seed derivation as generate_grid_field plus a UUID-specific offset.
pub fn generate_grid_uuids(
    inner_radius: f32,
    outer_radius: f32,
    grid: &GridConfig,
    seed_offset: u64,
    gameplay_count: usize,
    cosmetic_upper_count: usize,
    cosmetic_lower_count: usize,
) -> (Vec<String>, Vec<String>, Vec<String>) {
    let seed = {
        let mut seed: u64 = seed_offset;
        seed = seed.wrapping_add(inner_radius.to_bits() as u64);
        seed = seed.wrapping_add(outer_radius.to_bits() as u64);
        seed = seed.wrapping_add(grid.resolution.to_bits() as u64);
        seed = seed.wrapping_mul(2654435761);
        seed
    };

    let uuid_seed = seed.wrapping_add(0xDEAD_BEEF_CAFE_1234);
    let mut rng = StdRng::seed_from_u64(uuid_seed);

    let make_uuid = |rng: &mut StdRng| -> String {
        let a: u64 = rng.random();
        let b: u64 = rng.random();
        let bytes: [u8; 16] = [
            (a >> 56) as u8, (a >> 48) as u8, (a >> 40) as u8, (a >> 32) as u8,
            (a >> 24) as u8, (a >> 16) as u8,
            0x40 | ((a >> 8) as u8 & 0x0f), a as u8,
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
    };

    let gameplay: Vec<String> = (0..gameplay_count).map(|_| make_uuid(&mut rng)).collect();
    let cosmetic_upper: Vec<String> = (0..cosmetic_upper_count).map(|_| make_uuid(&mut rng)).collect();
    let cosmetic_lower: Vec<String> = (0..cosmetic_lower_count).map(|_| make_uuid(&mut rng)).collect();

    (gameplay, cosmetic_upper, cosmetic_lower)
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
        let first_cosmetic_idx = result.spawns.iter()
            .position(|s| s.config_path.contains("cosmetic"));
        
        // Find the last gameplay asteroid
        let last_gameplay_idx = result.spawns.iter()
            .rev()
            .position(|s| s.config_path.contains("gameplay"))
            .map(|pos| result.spawns.len() - 1 - pos);
        
        if let (Some(first_cosmetic), Some(last_gameplay)) = (first_cosmetic_idx, last_gameplay_idx) {
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
        
        let result = generate_donut_field(
            100.0,
            200.0,
            0.01,
            42,
            &paths,
            &[],
        );
        
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
        let result = generate_donut_field(
            100.0,
            200.0,
            0.01,
            42,
            &[],
            &[],
        );
        
        assert_eq!(result.count, 0);
        assert!(result.spawns.is_empty());
    }

    #[test]
    fn only_gameplay_types_produces_only_gameplay() {
        let result = generate_donut_field(
            100.0,
            200.0,
            0.01,
            42,
            &["gameplay.toml".to_string()],
            &[],
        );
        
        for spawn in &result.spawns {
            assert!(
                spawn.config_path.contains("gameplay"),
                "Should only have gameplay types"
            );
        }
    }

    #[test]
    fn only_cosmetic_types_produces_only_cosmetic() {
        let result = generate_donut_field(
            100.0,
            200.0,
            0.01,
            42,
            &[],
            &["cosmetic.toml".to_string()],
        );
        
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
            0, 5, 3, &grid, 100.0, 200.0,
            &["gameplay.toml".to_string()], &[],
        );
        let b = eval_cell(
            0, 5, 3, &grid, 100.0, 200.0,
            &["gameplay.toml".to_string()], &[],
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
            0, 5, 3, &grid, 100.0, 200.0,
            &["gameplay.toml".to_string()], &[],
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
            0, 5, 3, &grid, 100.0, 200.0,
            &["gameplay.toml".to_string()], &[],
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
                dist >= 100.0 && dist <= 200.0,
                "Gameplay pos ({}, {}) dist={} outside torus [100,200]",
                spawn.x, spawn.z, dist
            );
        }
        for spawn in &result.cosmetic_upper {
            let dist = (spawn.x * spawn.x + spawn.z * spawn.z).sqrt();
            assert!(
                dist >= 100.0 && dist <= 200.0,
                "Cosmetic upper pos ({}, {}) dist={} outside torus [100,200]",
                spawn.x, spawn.z, dist
            );
        }
        for spawn in &result.cosmetic_lower {
            let dist = (spawn.x * spawn.x + spawn.z * spawn.z).sqrt();
            assert!(
                dist >= 100.0 && dist <= 200.0,
                "Cosmetic lower pos ({}, {}) dist={} outside torus [100,200]",
                spawn.x, spawn.z, dist
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
        assert_eq!(result_a.gameplay, result_b.gameplay, "Gameplay must be deterministic");
        assert_eq!(result_a.cosmetic_upper, result_b.cosmetic_upper, "Cosmetic upper must be deterministic");
        assert_eq!(result_a.cosmetic_lower, result_b.cosmetic_lower, "Cosmetic lower must be deterministic");
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
        let result = generate_grid_field(
            100.0,
            200.0,
            grid,
            42,
            &[],
            &["cosmetic.toml".to_string()],
        );
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
            0, 5, 3, &grid, 100.0, 200.0,
            &["gameplay.toml".to_string()], &[],
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
        assert!(cell_in_field(7, 0, 15.0, 100.0, 200.0, Some(AsteroidFieldShape::Torus)));
    }

    #[test]
    fn cell_in_field_torus_rejects_cell_fully_inside_inner_radius() {
        // Cell at gx=0, gz=0 with resolution=10:
        //   bbox centred at origin = [-5..5] × [-5..5]
        //   farthest corner = (5, 5) → dist ≈ 7.07
        // With inner_radius=50, the entire bbox is inside the inner hole.
        assert!(!cell_in_field(0, 0, 10.0, 50.0, 200.0, Some(AsteroidFieldShape::Torus)));
    }

    #[test]
    fn cell_in_field_torus_rejects_cell_fully_outside_outer_radius() {
        // Cell at gx=20, gz=20 with resolution=15:
        //   placement centre = (300, 300)
        //   bbox = [292.5..307.5] × [292.5..307.5]
        //   nearest corner = (292.5, 292.5) → dist ≈ 413.66
        // With outer_radius=200, the nearest corner is well beyond.
        assert!(!cell_in_field(20, 20, 15.0, 100.0, 200.0, Some(AsteroidFieldShape::Torus)));
    }

    #[test]
    fn cell_in_field_torus_admits_cell_straddling_outer_radius() {
        // Cell at gx=13, gz=0 with resolution=15:
        //   placement centre = (195, 0)
        //   bbox = [187.5..202.5] × [-7.5..7.5]
        //   nearest corner = (187.5, 0) → dist = 187.5 (inside outer=200)
        //   farthest = (202.5, 7.5) → dist ≈ 202.6 (outside outer=200)
        // Straddles outer boundary. Admitted because nearest is inside.
        assert!(cell_in_field(13, 0, 15.0, 100.0, 200.0, Some(AsteroidFieldShape::Torus)));
    }

    #[test]
    fn cell_in_field_torus_admits_cell_containing_origin() {
        // Cell at gx=0, gz=0 with resolution=15:
        //   bbox = [-7.5..7.5] × [-7.5..7.5] (contains origin)
        //   nearest corner distance = 0 (origin is inside the bbox)
        //   With inner_radius=10, the cell straddles the inner boundary
        //   (farthest corner at sqrt(112.5) ≈ 10.6 > 10).
        assert!(cell_in_field(0, 0, 15.0, 10.0, 200.0, Some(AsteroidFieldShape::Torus)));
    }

    #[test]
    fn cell_in_field_torus_zero_inner_radius_admits_central_cells() {
        // With inner_radius = 0, no cells are "fully inside" the inner
        // hole (the hole has no area). Admit any cell whose nearest
        // corner is inside outer_radius.
        assert!(cell_in_field(0, 0, 15.0, 0.0, 200.0, Some(AsteroidFieldShape::Torus)));
    }

    #[test]
    fn cell_in_field_torus_negative_coords_symmetric() {
        // Symmetry across quadrants: rejection of cells far in -X, -Z.
        assert!(!cell_in_field(-20, -20, 15.0, 100.0, 200.0, Some(AsteroidFieldShape::Torus)));
        // Cell whose bbox crosses the outer boundary on the -X side.
        assert!(cell_in_field(-13, 0, 15.0, 100.0, 200.0, Some(AsteroidFieldShape::Torus)));
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
        // `(field_idx, gx, gz)` and MUST NOT include the anchor. The anchor
        // is applied as a pure post-seed translation at the call site (see
        // `try_spawn_cell` / `check_destroyed_asteroids` in
        // `asteroids/lifecycle.rs`).
        //
        // This test pins three invariants:
        //   1. `eval_cell` takes no anchor parameter (compiles as-is).
        //   2. Two calls with identical (field_idx, gx, gz) return identical
        //      anchor-relative positions, regardless of what anchor a caller
        //      might add later.
        //   3. Translating the returned position by an anchor offset
        //      produces the expected world-space position — modelling the
        //      contract `try_spawn_cell` honours.
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
            field_idx, gx, gz, &grid, 100.0, 200.0,
            &["gameplay.toml".to_string()], &[],
        ).expect("fill_gameplay=0.0 must produce Some");

        // Invariant 1: cell centre is `(gx*res, _, gz*res)` — anchor-relative,
        // anchor is not even an input to eval_cell.
        assert_eq!(anchor_relative.x, gx as f32 * grid.resolution);
        assert_eq!(anchor_relative.z, gz as f32 * grid.resolution);

        // Invariant 2: identical inputs → identical output (anchor-independent).
        let again = eval_cell(
            field_idx, gx, gz, &grid, 100.0, 200.0,
            &["gameplay.toml".to_string()], &[],
        ).expect("second call with identical (field_idx, gx, gz) must also spawn");
        assert_eq!(
            (anchor_relative.x, anchor_relative.y, anchor_relative.z),
            (again.x, again.y, again.z),
            "eval_cell must be anchor-independent: identical (field_idx, gx, gz) → identical anchor-relative position"
        );

        // Invariant 3: post-seed translation by anchor_offset gives the
        // expected world-space position — model of the contract that
        // `try_spawn_cell` applies via `spawn.x + anchor_offset[0]` etc.
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
            100.0, 200.0, grid.clone(), 42,
            &["gameplay.toml".to_string()],
            &["cosmetic.toml".to_string()],
        );
        let shaped = generate_grid_field_with_shape(
            100.0, 200.0, grid, 42,
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
            100.0, 200.0, grid, 42,
            &["gameplay.toml".to_string()],
            &["cosmetic.toml".to_string()],
            Some(AsteroidFieldShape::Torus),
        );
        // Bounded tolerance: one cell diagonal beyond the annulus on either side.
        let tol = res * std::f32::consts::SQRT_2;
        for spawn in result.gameplay.iter()
            .chain(result.cosmetic_upper.iter())
            .chain(result.cosmetic_lower.iter())
        {
            let dist = (spawn.x * spawn.x + spawn.z * spawn.z).sqrt();
            assert!(
                dist >= 100.0 - tol && dist <= 200.0 + tol,
                "torus pos ({}, {}) dist={} outside [{}, {}]",
                spawn.x, spawn.z, dist, 100.0 - tol, 200.0 + tol,
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
            100.0, 200.0, grid.clone(), 42,
            &["gameplay.toml".to_string()],
            &[],
            None,
        );
        let torus = generate_grid_field_with_shape(
            100.0, 200.0, grid, 42,
            &["gameplay.toml".to_string()],
            &[],
            Some(AsteroidFieldShape::Torus),
        );
        assert!(
            torus.gameplay.len() >= legacy.gameplay.len(),
            "torus admits a superset of cells: legacy={} torus={}",
            legacy.gameplay.len(), torus.gameplay.len(),
        );
    }
}