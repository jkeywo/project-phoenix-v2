use super::*;

fn ms(source: ModifierSource, slot: ModifierSlot, bonus: f32) -> Modifier {
    Modifier {
        source,
        slot,
        bonus,
    }
}

// ── 1. Empty table → 1.0 for every slot ──────────────────────────────────

#[test]
fn empty_table_returns_identity_for_all_slots() {
    let mods = ShipModifiers::new();
    for slot in ModifierSlot::all() {
        assert_eq!(mods.get(&slot), 1.0, "expected 1.0 for {slot:?}");
    }
}

// ── 2. Single positive bonus ──────────────────────────────────────────────

#[test]
fn single_positive_bonus_gives_one_plus_bonus() {
    let mut mods = ShipModifiers::new();
    mods.add_or_update(ms(
        ModifierSource::ImpulseDrive,
        ModifierSlot::MaxSpeed,
        0.5,
    ));
    assert!((mods.get(&ModifierSlot::MaxSpeed) - 1.5).abs() < 1e-6);
}

// ── 3. Single negative bonus (penalty) ───────────────────────────────────

#[test]
fn single_penalty_gives_one_over_one_plus_abs() {
    let mut mods = ShipModifiers::new();
    mods.add_or_update(ms(
        ModifierSource::ImpulseDrive,
        ModifierSlot::HullDamageTaken,
        -0.5,
    ));
    let expected = 1.0 / 1.5;
    assert!((mods.get(&ModifierSlot::HullDamageTaken) - expected).abs() < 1e-6);
}

// ── 4. Multiple bonuses on the same slot stack additively ─────────────────

#[test]
fn multiple_sources_same_slot_stack_additively() {
    let mut mods = ShipModifiers::new();
    mods.add_or_update(ms(
        ModifierSource::ImpulseDrive,
        ModifierSlot::MaxSpeed,
        0.3,
    ));
    mods.add_or_update(ms(
        ModifierSource::RegionEffect {
            uuid: uuid::Uuid::from_u128(1),
        },
        ModifierSlot::MaxSpeed,
        0.2,
    ));
    assert!((mods.get(&ModifierSlot::MaxSpeed) - 1.5).abs() < 1e-6);
}

// ── 5. Mixed positive + negative bonuses on same slot ────────────────────

#[test]
fn mixed_bonuses_sum_before_formula() {
    let mut mods = ShipModifiers::new();
    mods.add_or_update(ms(
        ModifierSource::ImpulseDrive,
        ModifierSlot::MaxSpeed,
        1.0,
    ));
    mods.add_or_update(ms(
        ModifierSource::RegionEffect {
            uuid: uuid::Uuid::from_u128(2),
        },
        ModifierSlot::MaxSpeed,
        -0.5,
    ));
    // sum = 0.5 → multiplier = 1.5
    assert!((mods.get(&ModifierSlot::MaxSpeed) - 1.5).abs() < 1e-6);
}

// ── 6. Re-adding same source+slot replaces (not stacks) ──────────────────

#[test]
fn readding_same_source_slot_replaces() {
    let mut mods = ShipModifiers::new();
    mods.add_or_update(ms(
        ModifierSource::ImpulseDrive,
        ModifierSlot::MaxSpeed,
        0.5,
    ));
    mods.add_or_update(ms(
        ModifierSource::ImpulseDrive,
        ModifierSlot::MaxSpeed,
        0.1,
    ));
    // Only 0.1 should remain, not 0.6
    assert!((mods.get(&ModifierSlot::MaxSpeed) - 1.1).abs() < 1e-6);
}

// ── 7. Remove restores to identity ───────────────────────────────────────

#[test]
fn remove_existing_modifier_restores_identity() {
    let mut mods = ShipModifiers::new();
    mods.add_or_update(ms(
        ModifierSource::ImpulseDrive,
        ModifierSlot::RadarRange,
        0.5,
    ));
    mods.remove(&ModifierSource::ImpulseDrive, &ModifierSlot::RadarRange);
    assert_eq!(mods.get(&ModifierSlot::RadarRange), 1.0);
}

// ── 8. Remove unknown entry is a no-op ───────────────────────────────────

#[test]
fn remove_unknown_is_noop() {
    let mut mods = ShipModifiers::new();
    // Should not panic
    mods.remove(&ModifierSource::ImpulseDrive, &ModifierSlot::MaxSpeed);
    assert_eq!(mods.get(&ModifierSlot::MaxSpeed), 1.0);
}

// ── 9. Modifiers on different slots don't bleed into each other ───────────

#[test]
fn slot_isolation() {
    let mut mods = ShipModifiers::new();
    mods.add_or_update(ms(
        ModifierSource::ImpulseDrive,
        ModifierSlot::MaxSpeed,
        2.0,
    ));
    // All other slots should still be 1.0
    assert_eq!(mods.get(&ModifierSlot::PhaserDamage), 1.0);
    assert_eq!(mods.get(&ModifierSlot::RepairRate), 1.0);
    assert_eq!(mods.get(&ModifierSlot::RadarRange), 1.0);
}

// ── 10. Region IDs stack as distinct sources ──────────────────────────────

#[test]
fn different_region_ids_stack_as_distinct_sources() {
    let mut mods = ShipModifiers::new();
    mods.add_or_update(ms(
        ModifierSource::RegionEffect {
            uuid: uuid::Uuid::from_u128(3),
        },
        ModifierSlot::MaxSpeed,
        0.2,
    ));
    mods.add_or_update(ms(
        ModifierSource::RegionEffect {
            uuid: uuid::Uuid::from_u128(4),
        },
        ModifierSlot::MaxSpeed,
        0.3,
    ));
    // sum = 0.5 → 1.5
    assert!((mods.get(&ModifierSlot::MaxSpeed) - 1.5).abs() < 1e-6);
}

// ── 11. PowerGroup source stacks per-group ───────────────────────────────

#[test]
fn power_group_source_uses_group_id() {
    let mut mods = ShipModifiers::new();
    mods.add_or_update(ms(
        ModifierSource::PowerGroup(crate::core::messages::PowerGroupId("sensors".into())),
        ModifierSlot::RadarRange,
        1.0,
    ));
    assert!((mods.get(&ModifierSlot::RadarRange) - 2.0).abs() < 1e-6);
}

// ── Determinism guard (issue #965) ────────────────────────────────────

/// Number of independent `ShipModifiers` instances each producer set is
/// summed by. Every one draws its own `RandomState`, so under a hashed
/// table these are 24 different walks of the same keys.
const GUARD_INSTANCES: usize = 24;
/// Number of distinct producer sets the guard tries. One set is not
/// enough: whether a given set's ULP disagreement survives the
/// `1.0 + sum` rounding into the published multiplier depends on where
/// the sum lands in its binade, and not every set shows it.
/// Re-measured directly against the pre-fix code (swap `table` back to a
/// `HashMap`, rerun the two guard tests below, restore the `BTreeMap`):
/// `same_producers_publish_identical_bits_in_every_instance` (the
/// `1.0 + sum` branch) saw 42 of these 48 sets disagree, and
/// `negative_sum_slots_publish_identical_bits_in_every_instance` (the
/// `1.0 / (1.0 + |sum|)` branch) saw 36 of 48. Both figures move a
/// little from run to run — the split depends on the process's random
/// hash seed, same as the bug itself — but in every run the large
/// majority of sets disagree, which is why 48 sets is enough to make the
/// guard reliable without being slow.
const GUARD_SETS: usize = 48;

/// Bonus for producer `j` of set `set_idx` — a spread of ordinary modifier
/// magnitudes in `[-0.5, 0.8)`, none of them exactly representable in
/// binary (the `/101` sees to that), so their partial sums round.
/// Arithmetic rather than a literal table so the guard covers a family of
/// value shapes instead of one lucky tuple.
fn guard_bonus(set_idx: usize, j: usize) -> f32 {
    (((set_idx * 8 + j) * 37 % 101) as f32 / 101.0) * 1.3 - 0.5
}

/// Every `ShipModifiers` holding the SAME producers must publish the SAME
/// BITS for a slot, whatever order those producers arrived in and whichever
/// instance holds them.
///
/// This is the standing guard against unordered float accumulation coming
/// back into the modifier cache (issue #965). It deliberately is not "the
/// multiplier equals a constant": the defect only shows under a *different
/// iteration order*, and any one instance's order is fixed, so a
/// single-instance assertion would have passed throughout the bug's life.
///
/// It gets those different orders honestly. `HashMap::new()` draws a fresh
/// `RandomState` per instance — std seeds a thread-local key once and bumps
/// it on every construction — so a hashed table walks the same eight keys in
/// a different order in each of these instances. That is the same class of
/// disagreement two *processes* see from the per-process seed, which is the
/// one this test cannot itself create and the one that was diverging seeded
/// runs. Insertion order is rotated too, so a table that ordered by
/// insertion rather than by key would also be caught.
///
/// This guard exists alongside `tests/rng_determinism.rs`'s
/// `two_runs_with_the_same_seed_produce_byte_identical_reports` — not
/// because that integration guard is structurally unable to see this
/// class of bug. It is not: `HashMap::new()` reseeds per MAP, not only
/// per process, so that guard's two sequential app builds already
/// construct their `ShipModifiers` tables from fresh, independently
/// seeded `RandomState`s and are just as capable of diverging. It has
/// stayed green through this defect's whole life because its world,
/// `rng_coverage.toml`, declares exactly one region and that region's
/// only effect is a bare `damage_zone`, which `apply_region_effects`
/// does not turn into a modifier at all — so no slot in that run ever
/// picked up the three-or-more producers this defect needs, regardless
/// of table order. This unit guard earns its place for a different
/// reason: it is fast, it is targeted at the one function that matters,
/// and it names the invariant directly instead of hoping a full
/// simulation's damage numbers happen to move.
///
/// Comparing `to_bits()` rather than an epsilon is the point: a single ULP
/// is the whole issue, because a ULP compounds chaotically across a 600 s
/// simulation.
#[test]
fn same_producers_publish_identical_bits_in_every_instance() {
    let mut split_sets: Vec<(usize, Vec<f32>)> = Vec::new();

    for set_idx in 0..GUARD_SETS {
        let mut published: std::collections::BTreeSet<u32> = std::collections::BTreeSet::new();
        for rotation in 0..GUARD_INSTANCES {
            let mut mods = ShipModifiers::new();
            for k in 0..8 {
                let j = (k + rotation) % 8;
                mods.add_or_update(ms(
                    ModifierSource::RegionEffect {
                        uuid: uuid::Uuid::from_u128(j as u128 + 1),
                    },
                    ModifierSlot::MaxSpeed,
                    guard_bonus(set_idx, j),
                ));
            }
            published.insert(mods.get(&ModifierSlot::MaxSpeed).to_bits());
        }
        if published.len() > 1 {
            split_sets.push((
                set_idx,
                published.iter().map(|b| f32::from_bits(*b)).collect(),
            ));
        }
    }

    assert!(
        split_sets.is_empty(),
        "{} of {GUARD_SETS} producer sets published more than one multiplier \
             across {GUARD_INSTANCES} instances holding identical modifiers — the \
             modifier cache is accumulating f32 over an unordered collection again, \
             so two processes running the same seed will diverge. Offenders: {:?}",
        split_sets.len(),
        split_sets
    );
}

/// The same guard for the reciprocal branch of the cache formula: a slot
/// whose producers sum negative goes through `1.0 / (1.0 + |sum|)`, a
/// different rounding path from `1.0 + sum`, and `HullDamageTaken` is one
/// of the real slots that lands there.
#[test]
fn negative_sum_slots_publish_identical_bits_in_every_instance() {
    let mut split = 0usize;
    for set_idx in 0..GUARD_SETS {
        let mut published: std::collections::BTreeSet<u32> = std::collections::BTreeSet::new();
        for rotation in 0..GUARD_INSTANCES {
            let mut mods = ShipModifiers::new();
            for k in 0..8 {
                let j = (k + rotation) % 8;
                mods.add_or_update(ms(
                    ModifierSource::RegionEffect {
                        uuid: uuid::Uuid::from_u128(j as u128 + 1),
                    },
                    ModifierSlot::HullDamageTaken,
                    // Shifted negative so every set sums below zero.
                    guard_bonus(set_idx, j) - 0.35,
                ));
            }
            published.insert(mods.get(&ModifierSlot::HullDamageTaken).to_bits());
        }
        if published.len() > 1 {
            split += 1;
        }
    }
    assert_eq!(
        split, 0,
        "{split} of {GUARD_SETS} negative-sum producer sets published more than \
             one multiplier across instances holding identical modifiers"
    );
}

/// `clear_source` walks the table to decide which `ModifierEvent::Removed`
/// to queue, and those events reach the wire. The sequence must not depend
/// on which instance queued them.
#[test]
fn clear_source_queues_removals_in_the_same_order_in_every_instance() {
    let slots = ModifierSlot::all();
    let mut sequences: std::collections::BTreeSet<Vec<String>> = Default::default();
    for _ in 0..GUARD_INSTANCES {
        let mut mods = ShipModifiers::new();
        for (i, slot) in slots.iter().enumerate() {
            mods.add_or_update(ms(
                ModifierSource::ImpulseDrive,
                slot.clone(),
                0.1 * (i as f32 + 1.0),
            ));
        }
        mods.pending_events.clear();
        mods.clear_source(&ModifierSource::ImpulseDrive);
        sequences.insert(
            mods.pending_events
                .iter()
                .map(|e| format!("{e:?}"))
                .collect(),
        );
    }
    assert_eq!(
        sequences.len(),
        1,
        "clear_source queued its removals in {} different orders — that \
             sequence becomes an outbound message stream, so two processes \
             running the same seed emit different bytes",
        sequences.len()
    );
}

// ── Flag API tests ─────────────────────────────────────────────────────

use crate::core::messages::FlagKind;

#[test]
fn single_source_adds_flag() {
    let mut mods = ShipModifiers::new();
    mods.add_flag(ModifierSource::ImpulseDrive, FlagKind::CommsJammed);
    assert!(mods.has_flag(&FlagKind::CommsJammed));
}

#[test]
fn multiple_sources_or_aggregate() {
    let mut mods = ShipModifiers::new();
    mods.add_flag(ModifierSource::ImpulseDrive, FlagKind::CommsJammed);
    mods.add_flag(
        ModifierSource::RegionEffect {
            uuid: uuid::Uuid::from_u128(1),
        },
        FlagKind::CommsJammed,
    );
    assert!(mods.has_flag(&FlagKind::CommsJammed));
}

#[test]
fn removing_one_source_leaves_flag_set_when_multiple_sources() {
    let mut mods = ShipModifiers::new();
    mods.add_flag(ModifierSource::ImpulseDrive, FlagKind::CommsJammed);
    mods.add_flag(
        ModifierSource::RegionEffect {
            uuid: uuid::Uuid::from_u128(1),
        },
        FlagKind::CommsJammed,
    );
    mods.remove_flag(ModifierSource::ImpulseDrive, FlagKind::CommsJammed);
    assert!(
        mods.has_flag(&FlagKind::CommsJammed),
        "flag should remain because 2nd source still exists"
    );
}

#[test]
fn removing_last_source_clears_flag() {
    let mut mods = ShipModifiers::new();
    mods.add_flag(ModifierSource::ImpulseDrive, FlagKind::SensorBlind);
    mods.remove_flag(ModifierSource::ImpulseDrive, FlagKind::SensorBlind);
    assert!(!mods.has_flag(&FlagKind::SensorBlind));
}

#[test]
fn idempotent_add_does_not_duplicate() {
    let mut mods = ShipModifiers::new();
    mods.add_flag(ModifierSource::ImpulseDrive, FlagKind::CommsJammed);
    mods.add_flag(ModifierSource::ImpulseDrive, FlagKind::CommsJammed);
    assert!(mods.has_flag(&FlagKind::CommsJammed));
}

#[test]
fn removing_unknown_source_is_noop() {
    let mut mods = ShipModifiers::new();
    mods.remove_flag(ModifierSource::ImpulseDrive, FlagKind::CommsJammed);
    assert!(!mods.has_flag(&FlagKind::CommsJammed));
}

#[test]
fn flag_storage_independent_from_numeric_modifiers() {
    let mut mods = ShipModifiers::new();
    mods.add_flag(ModifierSource::ImpulseDrive, FlagKind::CommsJammed);
    mods.add_or_update(ms(
        ModifierSource::ImpulseDrive,
        ModifierSlot::MaxSpeed,
        0.5,
    ));
    // Flag should still be set
    assert!(mods.has_flag(&FlagKind::CommsJammed));
    // Modifier value should be unaffected
    assert!((mods.get(&ModifierSlot::MaxSpeed) - 1.5).abs() < 1e-6);
}

#[test]
fn flags_returns_all_set_flags() {
    let mut mods = ShipModifiers::new();
    mods.add_flag(ModifierSource::ImpulseDrive, FlagKind::CommsJammed);
    mods.add_flag(
        ModifierSource::RegionEffect {
            uuid: uuid::Uuid::from_u128(1),
        },
        FlagKind::SensorBlind,
    );
    let result = mods.flags();
    assert!(result.contains(&FlagKind::CommsJammed));
    assert!(result.contains(&FlagKind::SensorBlind));
    assert_eq!(result.len(), 2);
}

#[test]
fn flags_empty_when_no_flags_set() {
    let mods = ShipModifiers::new();
    assert!(mods.flags().is_empty());
}

// ── clear_source tests ────────────────────────────────────────────────

#[test]
fn clear_source_removes_all_modifiers_and_flags_for_source() {
    let mut mods = ShipModifiers::new();
    mods.add_or_update(ms(
        ModifierSource::ImpulseDrive,
        ModifierSlot::MaxSpeed,
        0.5,
    ));
    mods.add_or_update(ms(
        ModifierSource::ImpulseDrive,
        ModifierSlot::RadarRange,
        0.3,
    ));
    mods.add_flag(ModifierSource::ImpulseDrive, FlagKind::CommsJammed);
    mods.clear_source(&ModifierSource::ImpulseDrive);
    assert_eq!(mods.get(&ModifierSlot::MaxSpeed), 1.0);
    assert_eq!(mods.get(&ModifierSlot::RadarRange), 1.0);
    assert!(!mods.has_flag(&FlagKind::CommsJammed));
}

#[test]
fn clear_source_does_not_affect_other_sources() {
    let mut mods = ShipModifiers::new();
    let region = ModifierSource::RegionEffect {
        uuid: uuid::Uuid::from_u128(10),
    };
    mods.add_or_update(ms(
        ModifierSource::ImpulseDrive,
        ModifierSlot::MaxSpeed,
        0.5,
    ));
    mods.add_or_update(ms(region.clone(), ModifierSlot::MaxSpeed, 0.3));
    mods.add_flag(ModifierSource::ImpulseDrive, FlagKind::CommsJammed);
    mods.add_flag(region.clone(), FlagKind::SensorBlind);
    mods.clear_source(&region);
    // ImpulseDrive modifiers and flags should survive
    assert!((mods.get(&ModifierSlot::MaxSpeed) - 1.5).abs() < 1e-6);
    assert!(mods.has_flag(&FlagKind::CommsJammed));
    // Region modifiers and flags should be gone
    assert!(!mods.has_flag(&FlagKind::SensorBlind));
}

#[test]
fn clear_source_on_unknown_source_is_noop() {
    let mut mods = ShipModifiers::new();
    mods.add_or_update(ms(
        ModifierSource::ImpulseDrive,
        ModifierSlot::MaxSpeed,
        0.5,
    ));
    mods.clear_source(&ModifierSource::RegionEffect {
        uuid: uuid::Uuid::from_u128(99),
    });
    assert!((mods.get(&ModifierSlot::MaxSpeed) - 1.5).abs() < 1e-6);
}

// ── IntModifierSlot ───────────────────────────────────────────────────────

#[test]
fn int_modifier_slot_count_is_correct() {
    // COUNT must equal the number of variants; currently 1 (RepairTeams).
    assert_eq!(IntModifierSlot::COUNT, 1);
}

#[test]
fn repair_teams_slot_index_is_zero() {
    assert_eq!(IntModifierSlot::RepairTeams.index(), 0);
}

// ── add_or_update_int / get_int ───────────────────────────────────────────

#[test]
fn get_int_returns_zero_with_no_modifiers() {
    let mods = ShipModifiers::new();
    assert_eq!(mods.get_int(&IntModifierSlot::RepairTeams), 0);
}

#[test]
fn add_or_update_int_accumulates_bonuses_from_distinct_sources() {
    let mut mods = ShipModifiers::new();
    mods.add_or_update_int(IntModifier {
        source: ModifierSource::ImpulseDrive,
        slot: IntModifierSlot::RepairTeams,
        bonus: 2,
    });
    mods.add_or_update_int(IntModifier {
        source: ModifierSource::RegionEffect {
            uuid: uuid::Uuid::from_u128(1),
        },
        slot: IntModifierSlot::RepairTeams,
        bonus: 3,
    });
    assert_eq!(mods.get_int(&IntModifierSlot::RepairTeams), 5);
}

#[test]
fn same_source_slot_pair_replaces_rather_than_stacks() {
    let mut mods = ShipModifiers::new();
    mods.add_or_update_int(IntModifier {
        source: ModifierSource::ImpulseDrive,
        slot: IntModifierSlot::RepairTeams,
        bonus: 10,
    });
    mods.add_or_update_int(IntModifier {
        source: ModifierSource::ImpulseDrive,
        slot: IntModifierSlot::RepairTeams,
        bonus: 1,
    });
    // Only the latest bonus (1) should survive
    assert_eq!(mods.get_int(&IntModifierSlot::RepairTeams), 1);
}

// ── remove_int ────────────────────────────────────────────────────────────

#[test]
fn remove_int_removes_correct_entry_and_updates_cache() {
    let mut mods = ShipModifiers::new();
    let region = ModifierSource::RegionEffect {
        uuid: uuid::Uuid::from_u128(5),
    };
    mods.add_or_update_int(IntModifier {
        source: ModifierSource::ImpulseDrive,
        slot: IntModifierSlot::RepairTeams,
        bonus: 2,
    });
    mods.add_or_update_int(IntModifier {
        source: region.clone(),
        slot: IntModifierSlot::RepairTeams,
        bonus: 3,
    });
    mods.remove_int(&region, &IntModifierSlot::RepairTeams);
    assert_eq!(mods.get_int(&IntModifierSlot::RepairTeams), 2);
}

#[test]
fn remove_int_unknown_entry_is_noop() {
    let mut mods = ShipModifiers::new();
    mods.add_or_update_int(IntModifier {
        source: ModifierSource::ImpulseDrive,
        slot: IntModifierSlot::RepairTeams,
        bonus: 4,
    });
    mods.remove_int(
        &ModifierSource::RegionEffect {
            uuid: uuid::Uuid::from_u128(99),
        },
        &IntModifierSlot::RepairTeams,
    );
    assert_eq!(mods.get_int(&IntModifierSlot::RepairTeams), 4);
}

#[test]
fn remove_int_all_sources_returns_zero() {
    let mut mods = ShipModifiers::new();
    mods.add_or_update_int(IntModifier {
        source: ModifierSource::ImpulseDrive,
        slot: IntModifierSlot::RepairTeams,
        bonus: 7,
    });
    mods.remove_int(&ModifierSource::ImpulseDrive, &IntModifierSlot::RepairTeams);
    assert_eq!(mods.get_int(&IntModifierSlot::RepairTeams), 0);
}

#[test]
fn int_modifiers_are_independent_from_float_modifiers() {
    let mut mods = ShipModifiers::new();
    mods.add_or_update_int(IntModifier {
        source: ModifierSource::ImpulseDrive,
        slot: IntModifierSlot::RepairTeams,
        bonus: 3,
    });
    mods.add_or_update(ms(
        ModifierSource::ImpulseDrive,
        ModifierSlot::MaxSpeed,
        0.5,
    ));
    assert_eq!(mods.get_int(&IntModifierSlot::RepairTeams), 3);
    assert!((mods.get(&ModifierSlot::MaxSpeed) - 1.5).abs() < 1e-6);
}

#[test]
fn clear_source_on_region_exit_cleans_up_all_region_effects() {
    let mut mods = ShipModifiers::new();
    let region_uuid = uuid::Uuid::from_u128(42);
    let region = ModifierSource::RegionEffect { uuid: region_uuid };
    mods.add_or_update(ms(region.clone(), ModifierSlot::MaxSpeed, -0.2));
    mods.add_or_update(ms(region.clone(), ModifierSlot::PhaserDamage, 0.5));
    mods.add_flag(region.clone(), FlagKind::CommsJammed);
    mods.add_flag(region.clone(), FlagKind::SensorBlind);
    mods.clear_source(&region);
    assert_eq!(mods.get(&ModifierSlot::MaxSpeed), 1.0);
    assert_eq!(mods.get(&ModifierSlot::PhaserDamage), 1.0);
    assert!(!mods.has_flag(&FlagKind::CommsJammed));
    assert!(!mods.has_flag(&FlagKind::SensorBlind));
}

#[test]
fn clear_source_pushes_removed_events() {
    let mut mods = ShipModifiers::new();
    mods.add_or_update(ms(
        ModifierSource::ImpulseDrive,
        ModifierSlot::MaxSpeed,
        0.5,
    ));
    mods.add_or_update(ms(
        ModifierSource::ImpulseDrive,
        ModifierSlot::RadarRange,
        0.3,
    ));
    mods.pending_events.clear();
    mods.clear_source(&ModifierSource::ImpulseDrive);
    assert_eq!(mods.pending_events.len(), 2);
    assert!(mods
        .pending_events
        .iter()
        .all(|e| matches!(e, ModifierEvent::Removed { .. })));
}

// ── debug_payload tests (issue #1150) ──────────────────────────────────
//
// The structured projection that replaced the `format_debug` text stream.
// Asserts the same facts the old text tests did, now as typed payload data
// the observability dock renders.

#[test]
fn debug_payload_empty_has_no_entries_in_any_section() {
    let mods = ShipModifiers::new();
    let p = mods.debug_payload();
    assert_eq!(
        p.schema_version,
        crate::debug::payload::DEBUG_SCHEMA_VERSION
    );
    assert!(p.flags.is_empty(), "no flags on a fresh ship");
    assert!(p.float_modifiers.is_empty(), "no float modifiers");
    assert!(p.int_modifiers.is_empty(), "no int modifiers");
}

#[test]
fn debug_payload_shows_active_flag_with_source() {
    let mut mods = ShipModifiers::new();
    mods.add_flag(ModifierSource::ImpulseDrive, FlagKind::CommsJammed);
    let p = mods.debug_payload();
    assert_eq!(p.flags.len(), 1);
    assert_eq!(p.flags[0].flag, "CommsJammed", "flag name");
    assert_eq!(p.flags[0].sources, vec!["ImpulseDrive".to_string()]);
    // Only a flag was set.
    assert!(p.float_modifiers.is_empty());
    assert!(p.int_modifiers.is_empty());
}

#[test]
fn debug_payload_shows_float_modifier_with_multiplier_and_source() {
    let mut mods = ShipModifiers::new();
    mods.add_or_update(ms(
        ModifierSource::ImpulseDrive,
        ModifierSlot::MaxSpeed,
        0.5,
    ));
    let p = mods.debug_payload();
    assert_eq!(p.float_modifiers.len(), 1);
    let entry = &p.float_modifiers[0];
    assert_eq!(entry.slot, "MaxSpeed", "slot name");
    assert!(
        (entry.multiplier - 1.5).abs() < f32::EPSILON,
        "+0.5 bonus → 1.5× multiplier, got {}",
        entry.multiplier
    );
    assert_eq!(entry.contributions.len(), 1);
    assert_eq!(entry.contributions[0].source, "ImpulseDrive");
    assert!((entry.contributions[0].bonus - 0.5).abs() < f32::EPSILON);
}

#[test]
fn debug_payload_shows_int_modifier_with_sum_and_source() {
    let mut mods = ShipModifiers::new();
    mods.add_or_update_int(IntModifier {
        source: ModifierSource::ImpulseDrive,
        slot: IntModifierSlot::RepairTeams,
        bonus: 2,
    });
    let p = mods.debug_payload();
    assert_eq!(p.int_modifiers.len(), 1);
    let entry = &p.int_modifiers[0];
    assert_eq!(entry.slot, "RepairTeams", "int slot name");
    assert_eq!(entry.sum, 2, "summed total");
    assert_eq!(entry.contributions.len(), 1);
    assert_eq!(entry.contributions[0].source, "ImpulseDrive");
    assert_eq!(entry.contributions[0].bonus, 2);
}

#[test]
fn debug_payload_omits_empty_float_and_int_slots() {
    let mut mods = ShipModifiers::new();
    // Only a flag — no float or int modifiers.
    mods.add_flag(ModifierSource::ImpulseDrive, FlagKind::SensorBlind);
    let p = mods.debug_payload();
    assert_eq!(p.flags.len(), 1);
    assert!(
        p.float_modifiers.is_empty(),
        "no float slot has a producer, so none is listed"
    );
    assert!(
        p.int_modifiers.is_empty(),
        "no int slot has a producer, so none is listed"
    );
}

#[test]
fn debug_payload_sorts_float_contributions_by_source() {
    let mut mods = ShipModifiers::new();
    // Two sources on one slot; expect them sorted by rendered source name.
    mods.add_or_update(ms(
        ModifierSource::ImpulseDrive,
        ModifierSlot::MaxSpeed,
        0.2,
    ));
    mods.add_or_update(ms(
        ModifierSource::TractorLoad,
        ModifierSlot::MaxSpeed,
        -0.1,
    ));
    let p = mods.debug_payload();
    assert_eq!(p.float_modifiers.len(), 1);
    let sources: Vec<&str> = p.float_modifiers[0]
        .contributions
        .iter()
        .map(|c| c.source.as_str())
        .collect();
    assert_eq!(
        sources,
        vec!["ImpulseDrive", "TractorLoad"],
        "contributions must be sorted for deterministic JSON"
    );
}
