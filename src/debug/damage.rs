//! Damage-log surface on the structured debug pipeline (issue #1150, PRD #1144).
//!
//! The migration of the legacy `debug_overlay::write_damage_log` text overlay
//! onto the #1145 pipeline. The always-on ring buffer `debug_overlay::DamageLog`
//! is the data source — damage-application sites push to it every tick, on every
//! target, whatever the debug flag says — and [`project_damage`] is a read-only
//! projection of it into a [`DamageDebugPayload`]. The flag-gated publish system
//! encodes that to JSON, keeps it in [`DamageDebugCapture`] for the headless
//! report and the determinism guard, and (on the browser host only) feeds the
//! WASM bridge thread-local the dock's damage renderer reads.
//!
//! # Determinism
//!
//! `DamageLog` is a digest EXCLUSION (declared `StateClass::Timer` at its owning
//! site in `server_app`), and this projection only reads it, so producing the
//! payload can never move the #894 digest — proven by `tests/debug_overlays.rs`.

use bevy::prelude::*;

use crate::debug::payload::{DamageDebugPayload, DamageEntry, DEBUG_SCHEMA_VERSION};
use crate::debug_overlay::DamageLog;

impl crate::debug::catalogue::DebugSurfaceState for crate::debug_overlay::DebugDamageEnabled {
    fn is_enabled(&self) -> bool {
        self.0
    }

    fn set_enabled(&mut self, enabled: bool) {
        self.0 = enabled;
    }
}

/// Module-owned adapter for the damage Debug Surface.
pub const DEBUG_DAMAGE_ADAPTER: crate::debug::catalogue::DebugSurfaceAdapter =
    crate::debug::catalogue::DebugSurfaceAdapter::for_resource::<
        crate::debug_overlay::DebugDamageEnabled,
    >(crate::core::debug_surface::DebugSurface::Damage);

/// The latest damage-log JSON, when capture is enabled (issue #1150).
///
/// The target-agnostic sink, mirroring `debug::StationActivityCapture`: every
/// target keeps the JSON here so the headless report path and the determinism
/// guard can read it without a browser; the browser host ALSO writes the WASM
/// bridge thread-local. `None` until the first publish; never folded.
#[derive(Resource, Default, Debug)]
pub struct DamageDebugCapture(pub Option<String>);

/// Project the damage ring buffer into the wire payload (read-only).
///
/// Preserves the buffer's own newest-first order (`DamageLog::push` pushes to
/// the front), so the dock shows the most recent hit first exactly as the legacy
/// text overlay did.
pub fn project_damage(log: &DamageLog) -> DamageDebugPayload {
    DamageDebugPayload {
        schema_version: DEBUG_SCHEMA_VERSION,
        entries: log
            .entries
            .iter()
            .map(|e| DamageEntry {
                source: e.source.clone(),
                shield_arc: e.shield_arc.clone(),
                amount: e.amount,
            })
            .collect(),
    }
}

/// Project the damage log to JSON when capture is enabled (flag-gated).
///
/// The `DebugDamageEnabled` flag is taken as `Option<Res<..>>` and the projection
/// short-circuits when it is absent or off — see `publish_modifier_debug` for why
/// gating inside the system beats a `run_if` on a possibly-absent flag resource.
///
/// Read-only w.r.t. every folded resource: it reads `DamageLog` (a digest
/// exclusion) and writes only the presentation `DamageDebugCapture` and, on the
/// browser host, the WASM bridge. Running or not therefore cannot move the digest.
pub fn publish_damage_debug(
    enabled: Option<Res<crate::debug_overlay::DebugDamageEnabled>>,
    log: Res<DamageLog>,
    mut capture: ResMut<DamageDebugCapture>,
) {
    if !enabled.map(|f| f.0).unwrap_or(false) {
        return;
    }

    let payload = project_damage(&log);
    let json = crate::core::codec::encode_damage_debug(&payload);

    #[cfg(all(target_arch = "wasm32", feature = "server"))]
    crate::server::bridge::set_damage_log_string(json.clone());

    capture.0 = Some(json);
}

#[cfg(test)]
#[path = "damage_tests.rs"]
mod tests;
