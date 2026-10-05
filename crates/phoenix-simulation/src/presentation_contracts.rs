//! Simulation-facing presentation contracts. No asset, window or renderer types.
use bevy::prelude::*;

/// Optional host preload projection. Missing or unstarted means ready.
#[derive(Resource, Default, Clone, Copy, Debug)]
pub struct PresentationReadiness {
    pub started: bool,
    pub complete: bool,
}

/// Cosmetic impulse history, consumed and pruned by the presentation adapter.
/// Entries retain their original simulation timestamp and hull-equivalent magnitude.
#[derive(Resource, Default)]
pub struct ShakeState {
    pub entries: Vec<(f32, f32)>,
}

/// A disposable Workshop Test's latest requested window visibility.
#[derive(Resource, Default)]
pub struct TestWindowVisibility(pub Option<bool>);
