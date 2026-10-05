use super::*;

#[test]
fn completion_serves_its_cycle_and_preserves_other_banks() {
    let mut beam = ActiveBeam::default();
    let mut cooldown = PhaserCooldown::default();
    beam.start("a", "target", 1.0, 7.0);
    beam.start("b", "other", 2.0, 0.0);
    beam.bank_slot_mut("a").unwrap().damage_accumulator = 0.75;
    let slot = beam.complete_bank("a", &mut cooldown, 99.0).unwrap();
    assert_eq!(slot.damage_accumulator, 0.75);
    assert_eq!(cooldown.bank_remaining_secs("a"), 7.0);
    assert!(beam.bank_slot_mut("a").is_none());
    assert!(beam.bank_slot_mut("b").is_some());
    beam.complete_bank("b", &mut cooldown, 4.0).unwrap();
    assert_eq!(cooldown.bank_remaining_secs("b"), 4.0);
    assert!(beam.complete_bank("b", &mut cooldown, 99.0).is_none());
    assert_eq!(cooldown.bank_remaining_secs("b"), 4.0);
}
