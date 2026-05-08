// Pure Rust module for generating randomized asteroid positions.
// No Bevy, no physics engine — input → output design for isolated unit testing.

use rand::rngs::StdRng;
use rand::Rng;
use rand::SeedableRng;

/// Generate randomized asteroid positions within the given bounds.
///
/// # Arguments
/// * `spawn_radius` - The half-extent of the spawn box: positions are within
///   `[-spawn_radius, +spawn_radius]` on X and Z axes (Y is always 0).
/// * `count` - Number of asteroid positions to generate.
/// * `clear_zone_radius` - No asteroid will be placed within this distance of
///   the origin (where the ship spawns).
///
/// Returns a Vec of (x, z) positions (Y is implicitly 0).
pub fn spawn_asteroid_positions(
    spawn_radius: f32,
    count: usize,
    clear_zone_radius: f32,
) -> Vec<(f32, f32)> {
    let seed = spawn_radius as u64 + (count as u64) << 16 + (clear_zone_radius as u64) << 32;
    let mut rng = StdRng::seed_from_u64(seed);
    let mut positions = Vec::with_capacity(count);

    for _ in 0..count {
        let (x, z) = loop {
            let x: f32 = rng.random_range(-spawn_radius..spawn_radius);
            let z: f32 = rng.random_range(-spawn_radius..spawn_radius);
            let dist = (x * x + z * z).sqrt();
            if dist >= clear_zone_radius {
                break (x, z);
            }
        };
        // Check for duplicates (close enough check)
        if positions.iter().any(|(px, pz)| {
            let dx: f32 = x - px;
            let dz: f32 = z - pz;
            (dx * dx + dz * dz).sqrt() < 1.0
        }) {
            continue;
        }
        positions.push((x, z));
    }

    positions
}

/// Generate deterministic UUID strings for `count` asteroids, derived from
/// the same seed used by `spawn_asteroid_positions`. The UUIDs are stable
/// across runs given identical parameters.
pub fn spawn_asteroid_uuids(spawn_radius: f32, count: usize, clear_zone_radius: f32) -> Vec<String> {
    let seed = spawn_radius as u64 + (count as u64) << 16 + (clear_zone_radius as u64) << 32;
    // Use a secondary seed offset so UUID RNG doesn't interfere with position RNG.
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
                0x40 | ((a >> 8) as u8 & 0x0f),
                a as u8,
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

    fn spawner(count: usize) -> Vec<(f32, f32)> {
        spawn_asteroid_positions(150.0, count, 20.0)
    }

    #[test]
    fn returns_exact_count() {
        let positions = spawner(25);
        assert_eq!(positions.len(), 25);
    }

    #[test]
    fn all_positions_within_bounds() {
        let positions = spawner(50);
        for (x, z) in &positions {
            assert!(*x >= -150.0, "x={} is out of bounds", x);
            assert!(*x <= 150.0, "x={} is out of bounds", x);
            assert!(*z >= -150.0, "z={} is out of bounds", z);
            assert!(*z <= 150.0, "z={} is out of bounds", z);
        }
    }

    #[test]
    fn no_asteroid_within_clear_zone() {
        let positions = spawner(50);
        for (x, z) in &positions {
            let dist = (x * x + z * z).sqrt();
            assert!(dist >= 20.0, "Asteroid at ({}, {}) is within clear zone", x, z);
        }
    }

    #[test]
    fn no_duplicate_positions_for_small_counts() {
        let positions = spawner(20);
        // Check no two positions are too close
        for i in 0..positions.len() {
            for j in (i+1)..positions.len() {
                let (x1, z1) = positions[i];
                let (x2, z2) = positions[j];
                let dx = x1 - x2;
                let dz = z1 - z2;
                let dist = (dx * dx + dz * dz).sqrt();
                assert!(dist >= 0.9, "Duplicate positions found: ({},{}) and ({},{})", x1, z1, x2, z2);
            }
        }
    }

    #[test]
    fn clear_zone_prevents_origin_placement() {
        let positions = spawn_asteroid_positions(10.0, 10, 5.0);
        for (x, z) in &positions {
            let dist = (x * x + z * z).sqrt();
            assert!(dist >= 5.0);
        }
    }

    #[test]
    fn zero_count_returns_empty() {
        let positions = spawner(0);
        assert!(positions.is_empty());
    }

    fn uuids(count: usize) -> Vec<String> {
        spawn_asteroid_uuids(150.0, count, 20.0)
    }

    #[test]
    fn uuids_returns_exact_count() {
        let ids = uuids(25);
        assert_eq!(ids.len(), 25);
    }

    #[test]
    fn uuids_are_unique() {
        let ids = uuids(40);
        let mut seen = std::collections::HashSet::new();
        for id in &ids {
            assert!(seen.insert(id.as_str()), "Duplicate UUID: {}", id);
        }
    }

    #[test]
    fn uuids_are_deterministic() {
        let ids_a = uuids(10);
        let ids_b = uuids(10);
        assert_eq!(ids_a, ids_b);
    }

    #[test]
    fn uuids_have_expected_format() {
        let ids = uuids(5);
        for id in &ids {
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
    fn positions_unchanged_after_adding_uuid_generation() {
        // Ensure that adding UUID generation doesn't change the seeding logic
        // for positions — the two are independent.
        let positions_before = spawner(25);
        let _uuids = uuids(25); // run UUID generation
        let positions_after = spawner(25);
        assert_eq!(positions_before, positions_after);
    }
}
