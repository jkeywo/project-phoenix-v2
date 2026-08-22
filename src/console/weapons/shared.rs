//! Shared utility functions for the weapons console (issue #721).
//!
//! Pure helpers extracted from `server.rs` so other weapons modules can
//! reuse them without pulling in the full system definitions.

use bevy::prelude::*;

use crate::core::messages::SystemId;
use crate::server_app::AsteroidUuid;
use crate::ship_plugin::ShipSystemControlSources;

/// Look up the live (x, z) world position of an entity by its string UUID.
///
/// `WorldResource.0.entities` is a snapshot populated at spawn / first-report
/// time and never updated, so it cannot be used for gameplay decisions
/// involving moving entities (NPC ships, torpedoes, etc.). Always query the
/// live ECS `Transform` instead. Asteroids carry [`AsteroidUuid`]; NPCs and
/// stations carry [`crate::entities::spawner::EntityUuid`]. This helper checks
/// both.
pub(crate) fn live_entity_xz(
    uuid: &str,
    asteroid_q: &Query<(&AsteroidUuid, &Transform), Without<crate::entities::spawner::EntityUuid>>,
    entity_q: &Query<(&crate::entities::spawner::EntityUuid, &Transform), Without<AsteroidUuid>>,
) -> Option<(f32, f32)> {
    for (u, t) in asteroid_q.iter() {
        if u.0 == uuid {
            return Some((t.translation.x, t.translation.z));
        }
    }
    for (u, t) in entity_q.iter() {
        if u.0 == uuid {
            return Some((t.translation.x, t.translation.z));
        }
    }
    None
}

/// The two live-position lookup queries almost every Weapons system carries:
/// asteroids (keyed by [`AsteroidUuid`]) and non-asteroid entities (ships,
/// stations, keyed by [`crate::entities::spawner::EntityUuid`]), bundled as one
/// `SystemParam` (issue #1185).
///
/// This is a **readability** grouping only — the two `Query`s keep the exact
/// shapes and `Without<..>` filters they had as separate parameters, so the
/// system's access set and the schedule it builds are byte-for-byte unchanged.
/// The pair is the input [`live_entity_xz`] resolves an arbitrary target UUID
/// through, and it appears verbatim in `ai_target_selection`'s siblings across
/// `server.rs`, `beam.rs`, and `blaster.rs`; every host destructures it back to
/// its own `asteroid_q` / `entity_q` locals at entry so the body is untouched.
#[derive(bevy::ecs::system::SystemParam)]
pub(crate) struct DirectFireGeometry<'w, 's> {
    /// Asteroids, keyed by [`AsteroidUuid`]; `Without<EntityUuid>` mirrors the
    /// disjoint split every Weapons system already spelled inline.
    pub asteroids: Query<
        'w,
        's,
        (&'static AsteroidUuid, &'static Transform),
        Without<crate::entities::spawner::EntityUuid>,
    >,
    /// Non-asteroid entities (ships, stations), keyed by
    /// [`crate::entities::spawner::EntityUuid`]; `Without<AsteroidUuid>` is the
    /// complementary half of the same split.
    pub entities: Query<
        'w,
        's,
        (
            &'static crate::entities::spawner::EntityUuid,
            &'static Transform,
        ),
        Without<AsteroidUuid>,
    >,
}

/// Ship-level Tactical concerns (SetTarget, SetPhaserMode, SetPhaserFrequency)
/// are gated on "any phaser bank accepts human input"
/// (issue #512, option c). This preserves the "fire when only one bank is
/// alive" semantic.
///
/// Returns `true` when any bank in the ship's `phaser_banks` config has an
/// operable fine system (`accept_human_input == true`), or when the config
/// declares no `phaser_bank` fine systems at all — nothing to gate on, and
/// the per-target admission gate has already authorised the command (the
/// dead coarse `tactical` fallback was deleted by #801).
pub(crate) fn any_bank_accepts_human_input(
    control_sources: &ShipSystemControlSources,
    ship_config: &crate::ship::config::ShipConfig,
) -> bool {
    // Find bank ids from the systems list (fine `phaser_bank` kinds).
    let bank_system_ids: Vec<SystemId> = ship_config
        .systems
        .iter()
        .filter(|s| s.kind == crate::ship::system_registry::PHASER_BANK_KIND)
        .map(|s| s.id.clone())
        .collect();
    if bank_system_ids.is_empty() {
        // No fine banks declared: nothing to gate on — admission on the
        // target system has already authorised the command. (The dead
        // coarse `tactical` fallback was deleted by #801.)
        return true;
    }
    bank_system_ids
        .iter()
        .any(|id| control_sources.0.policy_for(id).accept_human_input)
}

/// True when ANY phaser bank on the ship has an operable fine system whose
/// policy has `operate_ai == true`.
///
/// Used as the ship-level early-skip gate in `ai_phaser_auto_fire` after
/// issue #512 deleted the coarse `[[system]] id = "tactical"` block. Before
/// the fix, the auto-fire loop gated on the coarse tactical policy — that
/// always returned the default `Human` policy for any ship whose config no
/// longer declared the coarse system, so NPCs with fine phaser banks
/// silently stopped auto-firing.
///
/// No coarse fallback: a config with no `phaser_bank` fine systems has no
/// bank that could be AI-operated, so this returns `false`. (The coarse
/// `tactical` fallback that used to sit here was provably dead — the id was
/// declared in zero TOMLs and registered in no resolver — and was deleted
/// by #801.)
pub(crate) fn any_bank_operates_ai(
    control_sources: &ShipSystemControlSources,
    ship_config: &crate::ship::config::ShipConfig,
) -> bool {
    ship_config
        .systems
        .iter()
        .filter(|s| s.kind == crate::ship::system_registry::PHASER_BANK_KIND)
        .any(|s| control_sources.0.policy_for(&s.id).operate_ai)
}

/// True when ANY blaster bank on the ship has an operable fine system whose
/// policy has `operate_ai == true`.
///
/// Used as the ship-level early-skip gate in `tick_blaster_auto_fire`.
///
/// No coarse fallback: a config with no `blaster_bank` fine systems has no
/// bank that could be AI-operated, so this returns `false` (dead coarse
/// `tactical` fallback deleted by #801, as in [`any_bank_operates_ai`]).
pub(crate) fn any_blaster_bank_operates_ai(
    control_sources: &ShipSystemControlSources,
    ship_config: &crate::ship::config::ShipConfig,
) -> bool {
    ship_config
        .systems
        .iter()
        .filter(|s| s.kind == crate::ship::system_registry::BLASTER_BANK_KIND)
        .any(|s| control_sources.0.policy_for(&s.id).operate_ai)
}

/// True when ANY tactical fine system (phaser bank, torpedo tube, or the
/// torpedo magazine) has an operable fine system whose policy has
/// `operate_ai == true`.
///
/// Used as the ship-level early-skip gate in `ai_target_selection` after
/// issue #512 deleted the coarse `[[system]] id = "tactical"` block.
/// Mirrors the shape of `any_bank_operates_ai` but covers the full tactical
/// surface (weapons_target sync + torpedo auto-fire both need to run when
/// any tactical fine system is AI-driven).
///
/// No coarse fallback: a config with no tactical fine systems has nothing
/// that could be AI-operated, so this returns `false` (dead coarse
/// `tactical` fallback deleted by #801, as in [`any_bank_operates_ai`]).
pub(crate) fn any_tactical_system_operates_ai(
    control_sources: &ShipSystemControlSources,
    ship_config: &crate::ship::config::ShipConfig,
) -> bool {
    let tactical_fine_kinds = [
        crate::ship::system_registry::PHASER_BANK_KIND,
        crate::ship::system_registry::TORPEDO_TUBE_KIND,
        crate::ship::system_registry::TORPEDO_MAGAZINE_KIND,
    ];
    ship_config
        .systems
        .iter()
        .filter(|s| tactical_fine_kinds.contains(&s.kind.as_str()))
        .any(|s| control_sources.0.policy_for(&s.id).operate_ai)
}

/// Per-shooter snapshot of everything phase 1 of the beam tick computes:
/// owned copies of the shooter's identity/position, the resolved (possibly
/// LOS-blocked) target, and the pre-computed damage/cooldown numbers, so
/// later phases can apply damage without holding a mutable borrow on the
/// ship query (issue #722).
#[derive(Debug, Clone)]
pub struct ShooterState {
    pub shooter_entity: Entity,
    pub shooter_uuid: String,
    pub shooter_x: f32,
    pub shooter_z: f32,
    pub target_uuid: String,
    pub active_bank: String,
    pub cooldown_secs: f32,
    pub damage_to_apply: i32,
    pub shield_pierce: f32,
    pub end_beam_early: bool,
    pub is_local_shooter: bool,
    pub shooter_phaser_freq: f32,
    /// UUID of the entity that will actually receive damage this tick.
    /// Equals `target_uuid` when LOS is clear; set to the blocker's UUID
    /// when a blocking entity intercepts the beam.
    pub effective_target_uuid: String,
    /// Position of the effective target (for VFX positioning on destruction).
    pub effective_target_x: f32,
    pub effective_target_z: f32,
    /// True when a friendly ship is blocking — beam is absorbed with no
    /// damage applied to anyone this tick.
    pub zero_damage: bool,
}

/// One-tick beam context shared across the beam-tick phases (issue #722).
///
/// Lifecycle: phase 1 (`tick_beams_prepare`, snapshot + cooldown tick) calls
/// [`BeamContext::clear`] at the start of each frame and repopulates the vec;
/// phase 2 (`tick_beams_apply_damage`) mutates it (instagib multiplier,
/// `end_beam_early`) and phase 3 (`tick_beams_tick_lifetimes`) reads it later
/// in the same tick. It carries no state across frames.
#[derive(Resource, Default)]
pub struct BeamContext(pub Vec<ShooterState>);

impl BeamContext {
    /// Empty the per-tick shooter snapshots. Phase 1 calls this at the
    /// start of every frame before repopulating.
    pub fn clear(&mut self) {
        self.0.clear();
    }
}

/// One-tick torpedo target snapshot shared across the torpedo-tick phases
/// (issue #724).
///
/// Lifecycle: `build_torpedo_target_snapshot` calls
/// [`TorpedoTargetSnapshot::clear`] at the start of each frame and
/// repopulates both collections; `tick_torpedo_lifecycle` reads them later
/// in the same tick. It carries no state across frames.
#[derive(Resource, Default)]
pub struct TorpedoTargetSnapshot {
    /// UUID → (x, y, z) positions for torpedo guidance, from live ECS
    /// transforms with a `WorldResource` snapshot fallback. Y threaded for
    /// full-3D torpedo homing (issue #768); `0.0` for Planar entities.
    pub target_positions: std::collections::HashMap<String, (f32, f32, f32)>,
    /// Proximity detonation target list (uuid, x, y, z, radius). Virtual
    /// entities (asteroid-field anchors, region trigger volumes) are
    /// excluded. Y threaded for 3D collision (issue #768).
    pub targets: Vec<(String, f32, f32, f32, f32)>,
}

impl TorpedoTargetSnapshot {
    /// Empty the per-tick target collections. The builder calls this at
    /// the start of every frame before repopulating.
    pub fn clear(&mut self) {
        self.target_positions.clear();
        self.targets.clear();
    }
}

/// True when `system_id` is registered on this ship's `ControlSourceResolver`
/// (either in the `sources` map or in the damage-driven `offline_systems`
/// set). Used to decide whether per-fine-instance gating applies to a
/// message, or whether the default-source policy applies (ships that haven't
/// opted into the fine-system decomposition; issue #801 removed the coarse
/// `tactical` fallback that used to sit behind this check).
pub(crate) fn system_is_registered(
    control_sources: &ShipSystemControlSources,
    system_id: &SystemId,
) -> bool {
    control_sources.0.entries().any(|(id, _)| id == system_id)
        || control_sources.0.is_offline(system_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_shooter() -> ShooterState {
        ShooterState {
            shooter_entity: Entity::PLACEHOLDER,
            shooter_uuid: "shooter-1".to_string(),
            shooter_x: 10.0,
            shooter_z: -5.0,
            target_uuid: "target-1".to_string(),
            active_bank: "phaser_bank_fore".to_string(),
            cooldown_secs: 0.25,
            damage_to_apply: 3,
            shield_pierce: 0.5,
            end_beam_early: false,
            is_local_shooter: true,
            shooter_phaser_freq: 42.0,
            effective_target_uuid: "blocker-1".to_string(),
            effective_target_x: 12.0,
            effective_target_z: -4.0,
            zero_damage: true,
        }
    }

    #[test]
    fn shooter_state_construction_preserves_fields() {
        let s = sample_shooter();
        assert_eq!(s.shooter_entity, Entity::PLACEHOLDER);
        assert_eq!(s.shooter_uuid, "shooter-1");
        assert_eq!(s.shooter_x, 10.0);
        assert_eq!(s.shooter_z, -5.0);
        assert_eq!(s.target_uuid, "target-1");
        assert_eq!(s.active_bank, "phaser_bank_fore");
        assert_eq!(s.cooldown_secs, 0.25);
        assert_eq!(s.damage_to_apply, 3);
        assert_eq!(s.shield_pierce, 0.5);
        assert!(!s.end_beam_early);
        assert!(s.is_local_shooter);
        assert_eq!(s.shooter_phaser_freq, 42.0);
        assert_eq!(s.effective_target_uuid, "blocker-1");
        assert_eq!(s.effective_target_x, 12.0);
        assert_eq!(s.effective_target_z, -4.0);
        assert!(s.zero_damage);
    }

    #[test]
    fn shooter_state_is_cloneable() {
        let s = sample_shooter();
        let c = s.clone();
        assert_eq!(c.shooter_uuid, s.shooter_uuid);
        assert_eq!(c.effective_target_uuid, s.effective_target_uuid);
    }

    #[test]
    fn beam_context_default_is_empty() {
        let ctx = BeamContext::default();
        assert!(ctx.0.is_empty());
    }

    #[test]
    fn beam_context_clear_empties_after_push() {
        let mut ctx = BeamContext::default();
        ctx.0.push(sample_shooter());
        ctx.0.push(sample_shooter());
        assert_eq!(ctx.0.len(), 2);
        ctx.clear();
        assert!(ctx.0.is_empty());
    }

    #[test]
    fn torpedo_target_snapshot_default_is_empty() {
        let snap = TorpedoTargetSnapshot::default();
        assert!(snap.target_positions.is_empty());
        assert!(snap.targets.is_empty());
    }

    #[test]
    fn torpedo_target_snapshot_clear_empties_after_push() {
        let mut snap = TorpedoTargetSnapshot::default();
        snap.target_positions
            .insert("uuid-1".to_string(), (1.0, 2.0, 3.0));
        snap.targets
            .push(("uuid-1".to_string(), 1.0, 2.0, 3.0, 4.0));
        assert_eq!(snap.target_positions.len(), 1);
        assert_eq!(snap.targets.len(), 1);
        snap.clear();
        assert!(snap.target_positions.is_empty());
        assert!(snap.targets.is_empty());
    }
}
