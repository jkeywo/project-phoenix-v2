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
#[path = "window_tests.rs"]
mod tests;
