//! Blaster systems (issue #631), extracted from `server.rs` (issue #726).
//!
//! Note: `crate::blaster::BlasterSystem` (the pure-Rust bank model in
//! `src/weapons/blaster.rs`) is a different module from this file — this one
//! holds the Bevy systems and the ECS wrapper resource.

use bevy::prelude::*;

use super::shared::{
    any_blaster_bank_operates_ai, live_entity_xz, system_is_registered, tactical_authorized,
};
use super::{AsteroidDestroyedVfx, ShipDestroyedVfx, WeaponsTarget, DEFAULT_SHIP_EXPLOSION_RADIUS};
use crate::ai_plugin::AiTokenRegistry;
use crate::lobby::{InboundMessage, Sessions, Target, WorldResource};
use crate::messages::{ClientMessage, GamePhase, ServerMessage, SystemControlPayload};
use crate::model_rig::ModelMarkers;
use crate::ship_plugin::ShipSystemControlSources;
use crate::ship_state::ShipPhysics;
use crate::simulation::{AsteroidUuid, GameOverReason, SimOutbox};

/// Wraps the pure-Rust blaster system(s) so they can be used as a Bevy
/// component on each ship entity (issue #631).
///
/// Each element corresponds to one `[[weapons_console.blaster_banks]]` entry.
/// A ship with no blaster banks will have an empty `Vec`.
#[derive(Resource, Component, Clone, Default)]
pub struct BlasterSystemResource(pub Vec<crate::blaster::BlasterSystem>);

/// Handle blaster fire/charge control messages:
///
/// - `ControlSystem { target: "blaster-<id>", payload: FireBlaster }` — legacy
///   alias, behaves as `ChargeBlasterStart` (instant-fire when `charge_time_secs == 0`).
/// - `ControlSystem { target: "blaster-<id>", payload: ChargeBlasterStart }` — begins
///   charge (or instant-fires when `charge_time_secs == 0`, issue #636).
/// - `ControlSystem { target: "blaster-<id>", payload: ChargeBlasterCancel }` — cancels
///   an in-progress charge with no penalty (issue #636).
///
/// Resolves the bank id from the target SystemId, gates on the bank's
/// fine-system policy, then dispatches to the appropriate `BlasterSystem` method.
///
/// Runs in `SimSet::Input`.
pub(crate) fn handle_fire_blaster(
    mut reader: MessageReader<InboundMessage>,
    sessions: Res<Sessions>,
    ai_registry: Option<Res<AiTokenRegistry>>,
    localship_q: Query<
        (Entity, &crate::ship_plugin::ShipConfigComponent),
        With<crate::server_app::LocalShip>,
    >,
    mut ship_q: Query<
        (
            &ShipSystemControlSources,
            &ShipPhysics,
            Option<&WeaponsTarget>,
            &mut BlasterSystemResource,
        ),
        With<crate::server_app::Ship>,
    >,
    asteroid_q: Query<(&AsteroidUuid, &Transform), Without<crate::entity_spawner::EntityUuid>>,
    entity_q: Query<(&crate::entity_spawner::EntityUuid, &Transform), Without<AsteroidUuid>>,
) {
    let local_ship: Option<(Entity, &crate::ship_plugin::ShipConfigComponent)> =
        localship_q.single().ok();

    for ev in reader.read() {
        let ClientMessage::ControlSystem { target, payload } = &ev.msg else {
            continue;
        };

        // Accept FireBlaster (legacy), ChargeBlasterStart, and ChargeBlasterCancel.
        let is_charge_start = matches!(
            payload,
            SystemControlPayload::FireBlaster | SystemControlPayload::ChargeBlasterStart
        );
        let is_charge_cancel = matches!(payload, SystemControlPayload::ChargeBlasterCancel);
        if !is_charge_start && !is_charge_cancel {
            continue;
        }

        // Target must look like "blaster-<bank_id>" — matches `blaster_bank_system_id`.
        let bank_id = match target.0.strip_prefix("blaster-") {
            Some(id) if !id.is_empty() => id.to_string(),
            _ => continue,
        };

        // Resolve shooter entity — AI tokens route through AiTokenRegistry;
        // human tokens route to the LocalShip.
        let shooter_entity: Entity = if ev.token.starts_with("ai:") {
            match ai_registry
                .as_deref()
                .and_then(|r| r.bevy_entity_for_token(&ev.token))
            {
                Some(e) => e,
                None => continue,
            }
        } else {
            match local_ship {
                Some((e, cfg)) if tactical_authorized(&sessions, cfg, &ev.token) => e,
                _ => continue,
            }
        };

        let Ok((control_sources, physics, weapons_target_opt, mut blaster_res)) =
            ship_q.get_mut(shooter_entity)
        else {
            continue;
        };

        // Gate on the bank's fine-system policy. AI tokens require operate_ai;
        // human tokens require accept_human_input.
        let bank_system_id = crate::system_registry::blaster_bank_system_id(&bank_id)
            .filter(|id| system_is_registered(control_sources, id));
        let policy = match &bank_system_id {
            Some(id) => control_sources.0.policy_for(id),
            // Unregistered fine system: the default-source policy — exactly
            // what `policy_for` returns for any unknown id. No coarse
            // `tactical` fallback (issue #801).
            None => crate::ship::control_source::control_tick_policy(
                crate::ship::control_source::ControlSource::default(),
            ),
        };
        let is_ai_token = ev.token.starts_with("ai:");
        let authorized = if is_ai_token {
            policy.operate_ai
        } else {
            policy.accept_human_input
        };
        if !authorized {
            continue;
        }

        // Arc check: resolve the player's locked target and verify it's within the
        // bank's fire arc. Target selection mirrors tick_blaster_auto_fire:
        // the ship's authoritative `WeaponsTarget` lock.
        // AI tokens skip this check — arc enforcement for AI fire is handled by
        // tick_blaster_auto_fire instead.
        if is_charge_start && !is_ai_token {
            let Some(target_uuid) = weapons_target_opt.and_then(|wt| wt.0.clone()) else {
                continue;
            };
            let Some((tx, tz)) = live_entity_xz(&target_uuid, &asteroid_q, &entity_q) else {
                continue;
            };

            // Find the bank to check its arc config.
            let bank_arc_ok = blaster_res
                .0
                .iter()
                .find(|b| b.config.id == bank_id)
                .map(|bank| {
                    let (rx, ry) = crate::weapons::phaser::ship_local(
                        tx,
                        tz,
                        physics.x,
                        physics.z,
                        physics.yaw,
                    );
                    crate::weapons::phaser::in_arc(
                        rx,
                        ry,
                        bank.config.facing_deg,
                        bank.config.fire_arc_deg,
                    )
                })
                .unwrap_or(false);
            if !bank_arc_ok {
                continue;
            }
        }

        // Dispatch to the matching bank.
        if let Some(bank) = blaster_res.0.iter_mut().find(|b| b.config.id == bank_id) {
            if is_charge_start {
                bank.request_charge_start();
            } else {
                bank.request_charge_cancel();
            }
        }
    }
}

/// Auto-fire blaster banks for AI-controlled ships.
///
/// Iterates every ship (`With<Ship>`) — player + NPC — and calls
/// `request_charge_start()` on each blaster bank whose fine-system policy
/// has `operate_ai == true` when the ship has a valid target in range and
/// within the bank's fire arc. Ships whose config declares no `blaster_bank`
/// fine systems fall back to the coarse `tactical.operate_ai` policy.
///
/// Target selection: the ship's [`WeaponsTarget`] lock — the one authoritative
/// surface, whoever set it. (A legacy `ShipAiMemory::target` fallback sat here
/// until #702; it had been redundant for production NPCs since #703.) Range and
/// arc checks use each bank's config values.
pub(crate) fn tick_blaster_auto_fire(
    mut ship_q: Query<
        (
            &ShipSystemControlSources,
            Option<&crate::ship_plugin::ShipConfigComponent>,
            &ShipPhysics,
            Option<&WeaponsTarget>,
            &mut BlasterSystemResource,
        ),
        With<crate::server_app::Ship>,
    >,
    asteroid_q: Query<(&AsteroidUuid, &Transform), Without<crate::entity_spawner::EntityUuid>>,
    entity_q: Query<(&crate::entity_spawner::EntityUuid, &Transform), Without<AsteroidUuid>>,
) {
    let ship_count = ship_q.iter().len();
    #[cfg(target_arch = "wasm32")]
    web_sys::console::log_1(&wasm_bindgen::JsValue::from_str(&format!(
        "[DEBUG] tick_blaster_auto_fire: {} ship(s) in query",
        ship_count
    )));
    #[cfg(not(target_arch = "wasm32"))]
    eprintln!(
        "[DEBUG] tick_blaster_auto_fire: {} ship(s) in query",
        ship_count
    );

    for (control_sources, ship_config_opt, physics, weapons_target_opt, mut blaster_res) in
        ship_q.iter_mut()
    {
        // Gate: only run when at least one blaster bank is AI-controlled.
        let ai_controlled = match ship_config_opt {
            Some(cfg) => any_blaster_bank_operates_ai(control_sources, &cfg.0),
            // No ship config (test-only spawns): derive the gate from the
            // same per-bank fine ids the fire loop uses. No coarse
            // `tactical` fallback (issue #801).
            None => blaster_res.0.iter().any(|bank| {
                crate::system_registry::blaster_bank_system_id(&bank.config.id)
                    .is_some_and(|id| control_sources.0.policy_for(&id).operate_ai)
            }),
        };
        if !ai_controlled {
            #[cfg(target_arch = "wasm32")]
            web_sys::console::log_1(&wasm_bindgen::JsValue::from_str(
                "[DEBUG] tick_blaster_auto_fire: skipped — not AI controlled",
            ));
            #[cfg(not(target_arch = "wasm32"))]
            eprintln!("[DEBUG] tick_blaster_auto_fire: skipped — not AI controlled");
            continue;
        }
        #[cfg(target_arch = "wasm32")]
        web_sys::console::log_1(&wasm_bindgen::JsValue::from_str(
            "[DEBUG] tick_blaster_auto_fire: AI gate passed",
        ));
        #[cfg(not(target_arch = "wasm32"))]
        eprintln!("[DEBUG] tick_blaster_auto_fire: AI gate passed");

        // Target selection: the ship's one authoritative `WeaponsTarget` lock
        // (see `ai_target_selection`). The legacy `ShipAiMemory` fallback is
        // gone with #702.
        let target_uuid: Option<String> = weapons_target_opt.and_then(|wt| wt.0.clone());
        let Some(target_uuid) = target_uuid else {
            #[cfg(target_arch = "wasm32")]
            web_sys::console::log_1(&wasm_bindgen::JsValue::from_str(
                "[DEBUG] tick_blaster_auto_fire: skipped — no target UUID",
            ));
            #[cfg(not(target_arch = "wasm32"))]
            eprintln!("[DEBUG] tick_blaster_auto_fire: skipped — no target UUID");
            continue;
        };
        #[cfg(target_arch = "wasm32")]
        web_sys::console::log_1(&wasm_bindgen::JsValue::from_str(&format!(
            "[DEBUG] tick_blaster_auto_fire: target UUID = {}",
            target_uuid
        )));
        #[cfg(not(target_arch = "wasm32"))]
        eprintln!(
            "[DEBUG] tick_blaster_auto_fire: target UUID = {}",
            target_uuid
        );

        let Some((tx, tz)) = live_entity_xz(&target_uuid, &asteroid_q, &entity_q) else {
            #[cfg(target_arch = "wasm32")]
            web_sys::console::log_1(&wasm_bindgen::JsValue::from_str(&format!(
                "[DEBUG] tick_blaster_auto_fire: skipped — target {} not alive",
                target_uuid
            )));
            #[cfg(not(target_arch = "wasm32"))]
            eprintln!(
                "[DEBUG] tick_blaster_auto_fire: skipped — target {} not alive",
                target_uuid
            );
            continue;
        };
        #[cfg(target_arch = "wasm32")]
        web_sys::console::log_1(&wasm_bindgen::JsValue::from_str(&format!(
            "[DEBUG] tick_blaster_auto_fire: target live at ({:.1}, {:.1})",
            tx, tz
        )));
        #[cfg(not(target_arch = "wasm32"))]
        eprintln!(
            "[DEBUG] tick_blaster_auto_fire: target live at ({:.1}, {:.1})",
            tx, tz
        );

        for bank in blaster_res.0.iter_mut() {
            // Skip banks that are not ready to accept a new fire command.
            if !bank.is_fire_ready() {
                #[cfg(target_arch = "wasm32")]
                web_sys::console::log_1(&wasm_bindgen::JsValue::from_str(
                    "[DEBUG] tick_blaster_auto_fire: bank not fire-ready, skipping",
                ));
                #[cfg(not(target_arch = "wasm32"))]
                eprintln!("[DEBUG] tick_blaster_auto_fire: bank not fire-ready, skipping");
                continue;
            }

            // Range check.
            let dx = tx - physics.x;
            let dz = tz - physics.z;
            let dist_sq = dx * dx + dz * dz;
            let range_sq = bank.config.range * bank.config.range;
            if dist_sq > range_sq {
                #[cfg(target_arch = "wasm32")]
                web_sys::console::log_1(&wasm_bindgen::JsValue::from_str(&format!(
                    "[DEBUG] tick_blaster_auto_fire: bank out of range (dist={:.0} > range={:.0})",
                    dist_sq.sqrt(),
                    range_sq.sqrt()
                )));
                #[cfg(not(target_arch = "wasm32"))]
                eprintln!(
                    "[DEBUG] tick_blaster_auto_fire: bank out of range (dist={:.0} > range={:.0})",
                    dist_sq.sqrt(),
                    range_sq.sqrt()
                );
                continue;
            }

            // Arc check: convert target to ship-local coordinates.
            let (rx, ry) =
                crate::weapons::phaser::ship_local(tx, tz, physics.x, physics.z, physics.yaw);
            if !crate::weapons::phaser::in_arc(
                rx,
                ry,
                bank.config.facing_deg,
                bank.config.fire_arc_deg,
            ) {
                #[cfg(target_arch = "wasm32")]
                web_sys::console::log_1(&wasm_bindgen::JsValue::from_str(
                    &format!("[DEBUG] tick_blaster_auto_fire: bank out of arc (local=({:.1},{:.1}), facing={}, arc={})", rx, ry, bank.config.facing_deg, bank.config.fire_arc_deg),
                ));
                #[cfg(not(target_arch = "wasm32"))]
                eprintln!("[DEBUG] tick_blaster_auto_fire: bank out of arc (local=({:.1},{:.1}), facing={}, arc={})", rx, ry, bank.config.facing_deg, bank.config.fire_arc_deg);
                continue;
            }

            #[cfg(target_arch = "wasm32")]
            web_sys::console::log_1(&wasm_bindgen::JsValue::from_str(
                "[DEBUG] tick_blaster_auto_fire: FIRING!",
            ));
            #[cfg(not(target_arch = "wasm32"))]
            eprintln!("[DEBUG] tick_blaster_auto_fire: FIRING!");
            bank.request_charge_start();
        }
    }
}

/// Tick every ship's `BlasterSystemResource` — advance volley timers and
/// launch projectiles. Emits `ServerMessage::BlasterFired` for each
/// projectile launched. Runs in `SimSet::Physics`.
///
/// # Sanctioned out-of-band `ShipPhysics` writer (issue #699)
///
/// `integrate_ship_physics` is the sole *helm-path* writer of
/// `ShipPhysics.x/z/yaw/forward_speed/lateral_speed/roll`. The recoil impulse
/// (issue #638) accumulates into `forward_speed` directly and is an
/// intentional exception: it is a weapons-fire impulse added on top of
/// whatever the helm integrator produced, not a helm decision. It deliberately
/// does not opt into the debug `HelmPhysicsWriteGuard`. See the writer-policy
/// table on `ShipPhysics` (`src/ship/state.rs`).
pub(crate) fn tick_blaster_system(
    time: Res<Time>,
    mut ship_q: Query<
        (
            Option<&crate::entity_spawner::EntityUuid>,
            &Transform,
            Option<&ModelMarkers>,
            &mut ShipPhysics,
            Option<&WeaponsTarget>,
            &mut BlasterSystemResource,
        ),
        With<crate::server_app::Ship>,
    >,
    asteroid_q: Query<(&AsteroidUuid, &Transform), Without<crate::entity_spawner::EntityUuid>>,
    entity_q: Query<(&crate::entity_spawner::EntityUuid, &Transform), Without<AsteroidUuid>>,
    mut outbox: ResMut<SimOutbox>,
    #[cfg(feature = "server")] mut shake_state: Option<
        ResMut<crate::server::viewscreen_border::ShakeState>,
    >,
) {
    let dt = time.delta_secs();
    let now = time.elapsed_secs();

    // Pre-compute world-space velocity for every ship so the per-ship loop
    // below can look up the target's velocity for intercept prediction.
    // We read via `ship_q.iter()` (shared access to ShipPhysics) before the
    // main loop uses `ship_q.iter_mut()`, avoiding a Bevy SystemParam conflict.
    let ship_velocities: std::collections::HashMap<String, (f32, f32)> = {
        let mut map = std::collections::HashMap::new();
        for (uuid_opt, _, _, physics, _, _) in ship_q.iter() {
            if let Some(uuid) = uuid_opt {
                let vx = physics.forward_speed * physics.yaw.sin();
                let vz = -physics.forward_speed * physics.yaw.cos();
                map.insert(uuid.0.clone(), (vx, vz));
            }
        }
        map
    };

    for (
        source_uuid_opt,
        transform,
        markers_opt,
        mut physics,
        weapons_target_opt,
        mut blaster_res,
    ) in ship_q.iter_mut()
    {
        let source_uuid = source_uuid_opt
            .map(|u| u.0.as_str())
            .unwrap_or("")
            .to_string();

        // Resolve target position and velocity.
        // Ships supply world-space velocity from ShipPhysics; asteroids/objects
        // are stationary (velocity = 0).
        let target_uuid = weapons_target_opt.and_then(|wt| wt.0.clone());
        let (target_x, target_z, target_vx, target_vz) = if let Some(ref uuid) = target_uuid {
            let pos = live_entity_xz(uuid, &asteroid_q, &entity_q);
            let vel = ship_velocities.get(uuid);
            let (vx, vz) = vel.copied().unwrap_or((0.0, 0.0));
            pos.map(|(x, z)| (x, z, vx, vz))
                .unwrap_or((physics.x, physics.z - 100.0, 0.0, 0.0))
        } else {
            let fwd_x = physics.yaw.sin();
            let fwd_z = -physics.yaw.cos();
            (
                physics.x + fwd_x * 100.0,
                physics.z + fwd_z * 100.0,
                0.0,
                0.0,
            )
        };

        for bank in blaster_res.0.iter_mut() {
            let bank_id = bank.config.id.clone();
            let visual_scale = bank.config.visual_scale;
            let recoil_impulse = bank.config.recoil_impulse;
            let screenshake_magnitude = bank.config.screenshake_magnitude;

            // Named rig marker takes priority for the projectile's spawn
            // origin; falls back to ship center when the bank has no marker
            // or the model's sidecar doesn't define it.
            let (origin_x, origin_z) = bank
                .config
                .marker
                .as_deref()
                .and_then(|name| {
                    markers_opt.and_then(|m| m.resolve_world_position(transform, name))
                })
                .map(|pos| (pos.x, pos.z))
                .unwrap_or((physics.x, physics.z));

            let events = bank.tick(
                dt,
                origin_x,
                origin_z,
                physics.yaw,
                target_x,
                target_z,
                target_vx,
                target_vz,
                &source_uuid,
                &mut || uuid::Uuid::new_v4().to_string(),
            );
            for ev in &events {
                // ── Recoil impulse (issue #638) ─────────────────────────────
                // Apply an instantaneous velocity impulse to the firing ship
                // in the direction opposite to the projectile's heading.
                // The physics model is 1D (forward_speed along ship axis), so
                // we project the impulse onto the ship's forward axis and
                // accumulate it into forward_speed. The opposite-to-fire
                // convention is: impulse_dir = heading + π.
                if recoil_impulse > 0.0 {
                    // Ship forward direction in world space: (sin(yaw), -cos(yaw)).
                    // Projectile direction: (sin(heading), -cos(heading)).
                    // Recoil direction = opposite to projectile = -projectile.
                    // Projection of recoil onto ship forward:
                    //   dot((−sin(h), cos(h)), (sin(yaw), −cos(yaw)))
                    //   = −sin(h)·sin(yaw) + cos(h)·(−cos(yaw))
                    //   = −(sin(h)·sin(yaw) + cos(h)·cos(yaw))
                    //   = −cos(h − yaw)
                    let heading = ev.heading;
                    let yaw = physics.yaw;
                    let projection = -(heading - yaw).cos();
                    physics.forward_speed += projection * recoil_impulse;
                }

                // ── Screenshake (issue #638) ─────────────────────────────────
                // Push a synthetic entry into the rolling shake window.
                // The shake system sums hull_damage in the window; we scale
                // screenshake_magnitude so that 1.0 produces a noticeable
                // single-shot kick. The formula:
                //   magnitude = (total_hull / 30.0).min(1.0) * SHAKE_MAX_MAGNITUDE
                // So pushing hull_damage = screenshake_magnitude * 30.0 maps
                // 1.0 → full shake, 0.5 → half shake, etc.
                #[cfg(feature = "server")]
                if screenshake_magnitude > 0.0 {
                    if let Some(ref mut shake) = shake_state {
                        let hull_equiv = screenshake_magnitude * 30.0;
                        shake.entries.push((now, hull_equiv));
                    }
                }

                outbox.0.push((
                    Target::All,
                    ServerMessage::BlasterFired {
                        bank: bank_id.clone(),
                        source_uuid: source_uuid.clone(),
                        projectile_id: ev.projectile_id.clone(),
                        x: ev.x,
                        z: ev.z,
                        heading: ev.heading,
                        visual_scale,
                    },
                ));
            }
        }
    }
}

/// Check blaster projectile hits against all entities and apply shields-first
/// damage. Emits `ServerMessage::BlasterHit`. Runs in `SimSet::Damage`.
///
/// Hit detection uses live ECS Transform positions — the same approach as
/// `build_torpedo_target_snapshot`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn handle_blaster_hits(
    asteroid_q: Query<(&AsteroidUuid, &Transform), Without<crate::entity_spawner::EntityUuid>>,
    entity_q: Query<(&crate::entity_spawner::EntityUuid, &Transform), Without<AsteroidUuid>>,
    mut hit_target_q: Query<(
        Entity,
        Option<&AsteroidUuid>,
        Option<&crate::entity_spawner::EntityUuid>,
        &mut crate::entity_spawner::EntitySystemHull,
        Option<&mut crate::ship::shields::ShipShields>,
        Option<&mut crate::entity_spawner::EntityShipArcHull>,
        bevy::ecs::query::Has<crate::server_app::LocalShip>,
    )>,
    mut blaster_res_q: Query<&mut BlasterSystemResource, With<crate::server_app::Ship>>,
    mut outbox: ResMut<SimOutbox>,
    mut commands: Commands,
    mut next_state: Option<ResMut<NextState<GamePhase>>>,
    mut game_over_reason: Option<ResMut<GameOverReason>>,
    mut world: ResMut<WorldResource>,
    mut destroyed_events: MessageWriter<crate::ai_plugin::AiEntityDestroyed>,
    mut vfx_events: MessageWriter<AsteroidDestroyedVfx>,
    mut ship_vfx_events: MessageWriter<ShipDestroyedVfx>,
    collider_q: Query<&crate::entity_spawner::ColliderSection>,
) {
    // Build target list from live ECS transforms.
    let mut targets: Vec<(String, f32, f32, f32)> = Vec::new();
    for (ast_uuid, transform) in asteroid_q.iter() {
        targets.push((
            ast_uuid.0.clone(),
            transform.translation.x,
            transform.translation.z,
            0.0,
        ));
    }
    for (ent_uuid, transform) in entity_q.iter() {
        targets.push((
            ent_uuid.0.clone(),
            transform.translation.x,
            transform.translation.z,
            0.0,
        ));
    }

    #[derive(Clone)]
    struct BlasterDetonation {
        bank_id: String,
        projectile_id: String,
        target_uuid: String,
        damage: i32,
        shield_pierce: f32,
    }

    let mut detonations: Vec<BlasterDetonation> = Vec::new();
    for mut blaster_res in blaster_res_q.iter_mut() {
        for bank in blaster_res.0.iter_mut() {
            let hits = bank.find_hits(&targets);
            for (proj_id, target_uuid) in hits {
                if let Some(hit_data) = bank.consume_hit(&proj_id) {
                    detonations.push(BlasterDetonation {
                        bank_id: bank.config.id.clone(),
                        projectile_id: proj_id,
                        target_uuid,
                        damage: hit_data.damage,
                        shield_pierce: hit_data.shield_pierce,
                    });
                }
            }
        }
    }

    for det in detonations {
        outbox.0.push((
            Target::All,
            ServerMessage::BlasterHit {
                bank: det.bank_id,
                projectile_id: det.projectile_id,
                target_uuid: det.target_uuid.clone(),
            },
        ));

        // Apply shields-first damage to the matching entity.
        for (entity, ast_uuid, ent_uuid, mut hull_comp, mut shield_comp, mut arc_hull, is_local) in
            hit_target_q.iter_mut()
        {
            let uuid_matches = ast_uuid.map(|u| u.0.as_str()) == Some(det.target_uuid.as_str())
                || ent_uuid.map(|u| u.0.as_str()) == Some(det.target_uuid.as_str());
            if !uuid_matches {
                continue;
            }

            // God mode: local ship takes no damage.
            if is_local && crate::bridge::is_god_mode() {
                outbox.0.push((
                    Target::All,
                    ServerMessage::DamageTaken {
                        hull: 0.0,
                        shield: 0.0,
                    },
                ));
                break;
            }

            let mut hull_damage = det.damage as f32;

            let shield_amount = if let Some(ref mut shields) = shield_comp {
                let all_offline = shields.0.facings.iter().all(|f| !f.is_online());
                if !all_offline {
                    let (pierced, absorbed) = crate::damage::split_damage_for_pierce(
                        det.damage as f32,
                        det.shield_pierce,
                    );
                    let leak = shields.0.apply_damage(absorbed.round() as i32, 0.0);
                    let shielded = (absorbed - leak as f32).max(0.0);
                    hull_damage = pierced + leak as f32;
                    shielded
                } else {
                    0.0
                }
            } else {
                0.0
            };

            if hull_damage > 0.0 {
                let mut rng = rand::rng();
                let (hull_applied, destroyed) =
                    crate::damage::apply_hull_damage(&mut hull_comp.0, hull_damage, &mut rng);
                if let Some(ref mut ah) = arc_hull {
                    ah.0.apply_damage(hull_applied, &mut rng);
                }
                if is_local {
                    outbox.0.push((
                        Target::All,
                        ServerMessage::DamageTaken {
                            hull: hull_applied,
                            shield: shield_amount,
                        },
                    ));
                    if destroyed {
                        outbox.0.push((Target::All, ServerMessage::ShipDestroyed));
                        if let Some(ref mut ns) = next_state {
                            ns.set(GamePhase::GameOver);
                        }
                        if let Some(ref mut reason) = game_over_reason {
                            if reason.0.is_none() {
                                reason.0 = Some("Ship destroyed".into());
                            }
                        }
                    }
                } else if destroyed {
                    commands.entity(entity).try_despawn();
                    // Historically silent for non-local targets — neither the
                    // asteroid ripple nor a client despawn broadcast fired for
                    // a blaster kill. Bring it in line with the phaser/torpedo
                    // paths so every weapon type destroys entities the same
                    // way.
                    let is_asteroid = ast_uuid.is_some();
                    let (hit_x, hit_z) = targets
                        .iter()
                        .find(|(u, ..)| u == &det.target_uuid)
                        .map(|(_, x, z, _)| (*x, *z))
                        .unwrap_or((0.0, 0.0));
                    world.0.entities.retain(|a| a.uuid != det.target_uuid);
                    if is_asteroid {
                        vfx_events.write(AsteroidDestroyedVfx { x: hit_x, z: hit_z });
                        outbox.0.push((
                            Target::All,
                            ServerMessage::AsteroidDestroyed {
                                uuid: det.target_uuid.clone(),
                            },
                        ));
                    } else {
                        destroyed_events.write(crate::ai_plugin::AiEntityDestroyed {
                            entity_uuid: det.target_uuid.clone(),
                        });
                        let radius = collider_q
                            .get(entity)
                            .map(|c| c.0.radius)
                            .unwrap_or(DEFAULT_SHIP_EXPLOSION_RADIUS);
                        ship_vfx_events.write(ShipDestroyedVfx {
                            x: hit_x,
                            z: hit_z,
                            radius,
                        });
                        outbox.0.push((
                            Target::All,
                            ServerMessage::EntityDespawned {
                                uuid: det.target_uuid.clone(),
                            },
                        ));
                    }
                }
            }
            break; // UUID is unique.
        }
    }
}
