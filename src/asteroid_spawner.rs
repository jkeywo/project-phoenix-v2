// Pure Rust module for generating asteroid positions in a donut-shaped field.
// No Bevy, no physics engine — input → output design for isolated unit testing.

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
}