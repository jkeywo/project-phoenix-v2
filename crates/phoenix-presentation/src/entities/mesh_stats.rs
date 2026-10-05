//! What one loaded mesh or image costs, as the GPU sees it.
//!
//! Two functions, and they live here rather than in [`crate::perf::mesh`]
//! because there are now two readers with different build shapes: the perf pass
//! measures every shipped `.glb` natively (`--features perf`), and the model
//! viewer measures whatever is on screen inside wasm. A second copy of "how do
//! you count a triangle" is exactly the kind of drift the perf module refuses
//! to allow for file size — the LOD manifest and the asset inventory already
//! share one byte measurement rather than taking two opinions — so the counting
//! is shared the same way.
//!
//! Neither function knows about budgets, baselines or panels. They answer "how
//! many triangles does this mesh draw" and "how many pixels does this image
//! hold", and every caller decides what that means.

use bevy::image::Image;
use bevy::mesh::{Mesh, PrimitiveTopology};

/// Triangles in one loaded mesh.
///
/// Indexed geometry is counted from the index buffer and unindexed from the
/// vertex count, which is what the GPU draws in each case. A topology that
/// draws no triangles contributes none rather than a nonsense third of a line
/// list.
pub fn triangles_in(mesh: &Mesh) -> u64 {
    let drawn = match mesh.indices() {
        Some(indices) => indices.len() as u64,
        None => mesh.count_vertices() as u64,
    };
    match mesh.primitive_topology() {
        PrimitiveTopology::TriangleList => drawn / 3,
        // A strip of n vertices draws n-2 triangles, and fewer than three
        // vertices draws none.
        PrimitiveTopology::TriangleStrip => drawn.saturating_sub(2),
        _ => 0,
    }
}

/// Pixels in one loaded image.
pub fn pixels_in(image: &Image) -> u64 {
    u64::from(image.width()) * u64::from(image.height())
}
