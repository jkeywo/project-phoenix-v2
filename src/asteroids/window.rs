// Pure Rust module implementing the ring-buffer window logic for grid-based
// asteroid lifecycle. No Bevy imports, fully unit-testable.

/// Result of evaluating a player move — which cells to despawn, which to
/// evaluate for spawn, and whether a full rebuild is required instead.
#[derive(Debug, Clone, PartialEq)]
pub struct WindowDelta {
    pub full_rebuild: bool,
    pub cells_to_despawn: Vec<(i32, i32)>,
    pub cells_to_spawn: Vec<(i32, i32)>,
}

/// Convert a world-space position to an integer grid cell coordinate.
///
/// Uses flooring (towards negative infinity) so that a cell covers
/// `[cell * resolution, (cell + 1) * resolution)`.
pub fn compute_player_grid_cell(world_x: f32, world_z: f32, resolution: f32) -> (i32, i32) {
    let gx = (world_x / resolution).floor() as i32;
    let gz = (world_z / resolution).floor() as i32;
    (gx, gz)
}

/// Map a world-space cell to its ring-buffer slot index, given the player's
/// current grid cell.
///
/// Returns `None` if the cell is out of range (Chebyshev distance from player
/// exceeds `despawn_cells`).
///
/// Slot addressing is by **absolute cell**, not by offset from the player:
/// `target_gx.rem_euclid(size)` / `target_gz.rem_euclid(size)`, where
/// `size = 2 * despawn_cells + 1` is the window's fixed side length. This
/// makes the mapping translation-invariant — a given world cell always lands
/// in the same slot regardless of where the player currently is, as long as
/// the window size hasn't changed (that only happens on `full_rebuild`).
///
/// This matters for the incremental delta path (issue #924): with the prior
/// player-relative addressing (`dx.wrapping_add(d)`), the same world cell
/// mapped to a *different* slot after the player moved even one cell, so
/// despawning old-window cells against the old player position and spawning
/// new-window cells against the new player position left every surviving
/// cell's storage keyed by a slot index that no longer matched the ring's
/// current layout. Ring addressing removes the need to re-address survivors
/// at all: because any two cells simultaneously within Chebyshev distance
/// `despawn_cells` of the *same* center necessarily have distinct residues
/// mod `size` (there are exactly `size` residues and `size` cells across the
/// window's side), no two live cells can ever collide in the same slot, and
/// a cell's slot is stable across every incremental step that doesn't change
/// `size`.
pub fn compute_slot_for_world_cell(
    player_gx: i32,
    player_gz: i32,
    target_gx: i32,
    target_gz: i32,
    despawn_cells: u32,
) -> Option<(usize, usize)> {
    let d = despawn_cells as i32;
    let dx = target_gx - player_gx;
    let dz = target_gz - player_gz;
    if dx.abs() > d || dz.abs() > d {
        return None;
    }
    let size = 2 * d + 1;
    let slot_x = target_gx.rem_euclid(size) as usize;
    let slot_z = target_gz.rem_euclid(size) as usize;
    Some((slot_x, slot_z))
}

/// Evaluate the window delta when the player moves from `(old_gx, old_gz)` to
/// `(new_gx, new_gz)`.
///
/// Returns cells_to_despawn first, cells_to_spawn second. If the jump exceeds
/// `spawn_cells` in either axis, sets `full_rebuild = true`.
pub fn eval_on_player_move(
    old_gx: i32,
    old_gz: i32,
    new_gx: i32,
    new_gz: i32,
    spawn_cells: u32,
    despawn_cells: u32,
) -> WindowDelta {
    let d_cells = despawn_cells as i32;
    let s_cells = spawn_cells as i32;
    let dx = new_gx - old_gx;
    let dz = new_gz - old_gz;

    if dx == 0 && dz == 0 {
        return WindowDelta {
            full_rebuild: false,
            cells_to_despawn: Vec::new(),
            cells_to_spawn: Vec::new(),
        };
    }

    if dx.abs() > s_cells || dz.abs() > s_cells {
        return WindowDelta {
            full_rebuild: true,
            cells_to_despawn: Vec::new(),
            cells_to_spawn: Vec::new(),
        };
    }

    let mut cells_to_despawn = Vec::new();
    let mut cells_to_spawn = Vec::new();

    // Cells in old despawn window but NOT in new despawn window → despawn
    for gx in old_gx - d_cells..=old_gx + d_cells {
        for gz in old_gz - d_cells..=old_gz + d_cells {
            if (gx - new_gx).abs() > d_cells || (gz - new_gz).abs() > d_cells {
                cells_to_despawn.push((gx, gz));
            }
        }
    }

    // Cells in new spawn window but NOT in old spawn window → spawn
    for gx in new_gx - s_cells..=new_gx + s_cells {
        for gz in new_gz - s_cells..=new_gz + s_cells {
            if (gx - old_gx).abs() > s_cells || (gz - old_gz).abs() > s_cells {
                cells_to_spawn.push((gx, gz));
            }
        }
    }

    WindowDelta {
        full_rebuild: false,
        cells_to_despawn,
        cells_to_spawn,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── compute_player_grid_cell ─────────────────────────────────────────

    #[test]
    fn grid_cell_at_origin_is_zero() {
        assert_eq!(compute_player_grid_cell(0.0, 0.0, 50.0), (0, 0));
    }

    #[test]
    fn grid_cell_positive_world_coords() {
        assert_eq!(compute_player_grid_cell(100.0, 200.0, 50.0), (2, 4));
    }

    #[test]
    fn grid_cell_negative_world_coords() {
        assert_eq!(compute_player_grid_cell(-50.0, -75.0, 50.0), (-1, -2));
    }

    #[test]
    fn grid_cell_floors_fractional_input() {
        assert_eq!(compute_player_grid_cell(49.9, 50.1, 50.0), (0, 1));
    }

    #[test]
    fn grid_cell_negative_fractional_floors_away_from_zero() {
        assert_eq!(compute_player_grid_cell(-1.0, -50.1, 50.0), (-1, -2));
    }

    // ── compute_slot_for_world_cell ──────────────────────────────────────

    #[test]
    fn slot_at_player_center_is_target_cell_mod_size() {
        // Ring addressing: slot = target_cell.rem_euclid(size). At the origin
        // cell (0, 0), that's (0, 0) regardless of window size.
        assert_eq!(compute_slot_for_world_cell(0, 0, 0, 0, 2), Some((0, 0)),);
    }

    #[test]
    fn slot_positive_offset_from_player() {
        // size = 2*2+1 = 5; target (2, 1) is within range and rem_euclid(5)
        // is a no-op for small positive values.
        assert_eq!(compute_slot_for_world_cell(0, 0, 2, 1, 2), Some((2, 1)),);
    }

    #[test]
    fn slot_negative_offset_from_player() {
        // size = 5; target (-2, -2) wraps to (3, 3) under rem_euclid(5).
        assert_eq!(compute_slot_for_world_cell(0, 0, -2, -2, 2), Some((3, 3)),);
    }

    #[test]
    fn slot_out_of_range_returns_none() {
        assert_eq!(compute_slot_for_world_cell(0, 0, 3, 0, 2), None,);
    }

    #[test]
    fn slot_out_of_range_negative() {
        assert_eq!(compute_slot_for_world_cell(0, 0, 0, -3, 2), None,);
    }

    #[test]
    fn slot_edge_max_distance_in_range() {
        // size = 5; target (7, 3) rem_euclid(5) → (2, 3).
        assert_eq!(compute_slot_for_world_cell(5, 5, 7, 3, 2), Some((2, 3)),);
    }

    #[test]
    fn slot_is_stable_regardless_of_player_position() {
        // Ring addressing (#924): the same world cell (2, 2) must map to the
        // SAME slot whether the player is at (0, 0) or has moved to (1, 1) —
        // as long as the cell stays in range of both. This is the invariant
        // that lets the incremental delta path skip re-addressing survivors:
        // a cell's slot never moves out from under it just because the
        // player did.
        let slot_a = compute_slot_for_world_cell(0, 0, 2, 2, 2);
        let slot_b = compute_slot_for_world_cell(1, 1, 2, 2, 2);
        assert_eq!(slot_a, Some((2, 2)));
        assert_eq!(slot_b, Some((2, 2)));
        assert_eq!(slot_a, slot_b);
    }

    // ── eval_on_player_move ──────────────────────────────────────────────

    #[test]
    fn no_movement_returns_empty_delta() {
        let delta = eval_on_player_move(0, 0, 0, 0, 1, 2);
        assert!(!delta.full_rebuild);
        assert!(delta.cells_to_despawn.is_empty());
        assert!(delta.cells_to_spawn.is_empty());
    }

    #[test]
    fn single_cell_right_despawns_left_column() {
        // old=(0,0), new=(1,0), spawn=1, despawn=2
        // old despawn: x∈[-2,2], z∈[-2,2]
        // new despawn: x∈[-1,3], z∈[-2,2]
        // Old \ New: x=-2 → 5 cells: [(-2,-2),(-2,-1),(-2,0),(-2,1),(-2,2)]
        let delta = eval_on_player_move(0, 0, 1, 0, 1, 2);
        let mut expected_despawn: Vec<(i32, i32)> = (-2..=2).map(|z| (-2, z)).collect();
        expected_despawn.sort();
        let mut actual_despawn = delta.cells_to_despawn.clone();
        actual_despawn.sort();
        assert_eq!(actual_despawn, expected_despawn);
    }

    #[test]
    fn single_cell_right_spawns_right_column_within_spawn_range() {
        // old=(0,0), new=(1,0), spawn=1, despawn=2
        // new spawn: x∈[0,2], z∈[-1,1]
        // old spawn: x∈[-1,1], z∈[-1,1]
        // New \ Old (spawn): x=2 → 3 cells
        let delta = eval_on_player_move(0, 0, 1, 0, 1, 2);
        let mut expected_spawn: Vec<(i32, i32)> = (-1..=1).map(|z| (2, z)).collect();
        expected_spawn.sort();
        let mut actual_spawn = delta.cells_to_spawn.clone();
        actual_spawn.sort();
        assert_eq!(actual_spawn, expected_spawn);
    }

    #[test]
    fn eval_returns_despawn_list_first_then_spawn_list() {
        // The struct field order is cells_to_despawn before cells_to_spawn.
        // The function contract says despawn list first, spawn list second.
        let delta = eval_on_player_move(0, 0, 1, 0, 1, 2);
        // Check: the delta carries both lists; we also check Vec<Vec> does not
        // intermix by verifying there is no overlap.
        for (gx, gz) in &delta.cells_to_despawn {
            assert!(
                !delta.cells_to_spawn.contains(&(*gx, *gz)),
                "cell ({gx},{gz}) appears in both lists",
            );
        }
    }

    #[test]
    fn large_jump_returns_full_rebuild() {
        let delta = eval_on_player_move(0, 0, 5, 0, 1, 2);
        assert!(delta.full_rebuild);
        assert!(delta.cells_to_despawn.is_empty());
        assert!(delta.cells_to_spawn.is_empty());
    }

    #[test]
    fn large_jump_in_z_returns_full_rebuild() {
        let delta = eval_on_player_move(0, 0, 0, 5, 1, 2);
        assert!(delta.full_rebuild);
    }

    #[test]
    fn single_cell_up_despans_bottom_row() {
        // old=(0,0), new=(0,1), spawn=1, despawn=2
        // old despawn: x∈[-2,2], z∈[-2,2]
        // new despawn: x∈[-2,2], z∈[-1,3]
        // Old \ New: z=-2 → 5 cells
        let delta = eval_on_player_move(0, 0, 0, 1, 1, 2);
        let mut expected: Vec<(i32, i32)> = (-2..=2).map(|x| (x, -2)).collect();
        expected.sort();
        let mut actual = delta.cells_to_despawn.clone();
        actual.sort();
        assert_eq!(actual, expected);
    }

    #[test]
    fn single_cell_right_keeps_hysteresis_cells() {
        // Cells between spawn (1) and despawn (2) neither despawned nor spawned
        // old=(0,0), new=(1,0), spawn=1, despawn=2
        // Cell (3, 0) is in new despawn but not in new spawn → hysteresis zone
        let delta = eval_on_player_move(0, 0, 1, 0, 1, 2);
        assert!(!delta.cells_to_spawn.contains(&(3, 0)));
        assert!(!delta.cells_to_despawn.contains(&(3, 0)));
    }
}
