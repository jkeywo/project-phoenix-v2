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
}
