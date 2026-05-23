// Pure Rust module for generating asteroid positions in a donut-shaped field.
// No Bevy, no physics engine — input → output design for isolated unit testing.

use crate::entity_config::GridConfig;
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
            let cell_center_x = (cx as f32) * res;
            let cell_center_z = (cz as f32) * res;
            let dist = (cell_center_x * cell_center_x + cell_center_z * cell_center_z).sqrt();

            if dist < r_min || dist > r_max {
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
}