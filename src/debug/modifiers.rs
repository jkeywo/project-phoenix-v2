//! Modifier surface on the structured debug pipeline (issue #1150, PRD #1144).
//!
//! The migration of the legacy `debug_overlay::write_debug_state` text overlay
//! onto the #1145 pipeline. The projection itself is
//! [`crate::modifiers::ShipModifiers::debug_payload`] — it lives on the type so
//! it can read the private modifier tables, exactly as the retired `format_debug`
//! did. This module owns the flag-gated publish: it queries the LocalShip's
//! `ShipModifiers`, encodes its payload to JSON, keeps it in
//! [`ModifierDebugCapture`], and (on the browser host) feeds the WASM bridge the
//! dock's modifier renderer reads.
//!
//! # Determinism
//!
//! `ShipModifiers` is authoritative state, but this is a read-only projection of
//! it — the publish system holds `&ShipModifiers` and writes only the
//! presentation `ModifierDebugCapture` and the WASM bridge, so producing the
//! payload can never move the #894 digest (proven by `tests/debug_overlays.rs`).

use bevy::prelude::*;

use crate::modifiers::ShipModifiers;

/// The latest modifier-debug JSON, when capture is enabled (issue #1150).
///
/// The target-agnostic sink, mirroring `debug::StationActivityCapture`. `None`
/// until the first publish; never folded into the digest.
#[derive(Resource, Default, Debug)]
pub struct ModifierDebugCapture(pub Option<String>);

/// Project the LocalShip's modifiers to JSON when capture is enabled (flag-gated).
///
/// Queries `With<LocalShip>` exactly as the legacy overlay did — the modifier
/// surface has always been the *player's* ship. When there is no LocalShip (a
/// headless run) it publishes an empty-but-versioned payload rather than nothing,
/// so the dock and the determinism guard always have something to read.
///
/// The `DebugOverlayEnabled` flag is taken as `Option<Res<..>>` and the projection
/// short-circuits when it is absent or off: the flag lives in `debug_overlay` and
/// is only inserted on targets that added `DebugOverlayPlugin` (the browser host),
/// while a headless run merely declares it — so gating inside the system, rather
/// than on a `run_if` that would fetch a possibly-absent `Res`, is what keeps this
/// safe on every target. Read-only w.r.t. every folded resource; see the module
/// docs for why capture cannot move the digest.
pub fn publish_modifier_debug(
    enabled: Option<Res<crate::debug_overlay::DebugOverlayEnabled>>,
    modifiers_q: Query<&ShipModifiers, With<crate::server_app::LocalShip>>,
    mut capture: ResMut<ModifierDebugCapture>,
) {
    if !enabled.map(|f| f.0).unwrap_or(false) {
        return;
    }

    let payload = modifiers_q
        .iter()
        .next()
        .map(ShipModifiers::debug_payload)
        .unwrap_or_default();
    let json = crate::core::codec::encode_modifier_debug(&payload);

    #[cfg(all(target_arch = "wasm32", feature = "server"))]
    crate::server::bridge::set_debug_state_string(json.clone());

    capture.0 = Some(json);
}
