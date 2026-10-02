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
