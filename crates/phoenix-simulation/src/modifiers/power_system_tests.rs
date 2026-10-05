#![allow(clippy::field_reassign_with_default)]

use super::*;

#[test]
fn continuation_retains_existing_wire_fields_and_allocation_order() {
    let wire =
        r#"(allocations:[("weapons",3),("helm",1),("shields",2)],battery_charge:12.5,locked:true)"#;
    let saved: PowerState = ron::from_str(wire).unwrap();
    let mut reactor = PowerSystem::default();
    reactor.restore_continuation(&saved);
    assert_eq!(reactor.capture_continuation(), saved);
    assert_eq!(
        ron::to_string(&reactor.capture_continuation()).unwrap(),
        wire
    );
    let old_default: PowerState = ron::from_str("(battery_charge:0.0)").unwrap();
    assert_eq!(old_default, PowerState::default());
}

#[test]
fn continuation_replaces_bootstrap_and_preserves_exhaustion_and_recovery_ticks() {
    let config = PowerConfig::default();
    for (allocations, charge, locked) in [
        (
            vec![(weapons(), 4), (helm(), 3), (shields(), 1)],
            0.001,
            false,
        ),
        (vec![(shields(), 1), (weapons(), 1), (helm(), 1)], 0.0, true),
        (
            vec![(helm(), 2), (weapons(), 2), (shields(), 2)],
            config.capacity,
            false,
        ),
    ] {
        let mut live = PowerSystem::new(&config);
        live.restore(&allocations, charge, locked);
        let saved = live.capture_continuation();
        let mut resumed = PowerSystem::new(&config);
        resumed.restore(
            &[(helm(), 4), (shields(), 4), (weapons(), 4)],
            13.0,
            !locked,
        );
        assert_ne!(resumed.capture_continuation(), saved);
        resumed.restore_continuation(&saved);
        assert_eq!(resumed.capture_continuation(), saved);
        for _ in 0..200 {
            assert_eq!(resumed.tick(0.1, &config), live.tick(0.1, &config));
            assert_eq!(resumed.capture_continuation(), live.capture_continuation());
        }
    }
}

fn helm() -> PowerGroupId {
    PowerGroupId(HELM_POWER_GROUP.into())
}
fn weapons() -> PowerGroupId {
    PowerGroupId(WEAPONS_POWER_GROUP.into())
}
fn shields() -> PowerGroupId {
    PowerGroupId(SHIELDS_POWER_GROUP.into())
}

#[test]
fn strike_reserve_conserves_explicit_allocations_and_resumes_without_decay() {
    let reserve = PowerGroupId("strike-reserve".into());
    let config = PowerConfig {
        strike_reserve: Some(StrikeReserveConfig {
            ai_enable_at: None,
            group: reserve.0.clone(),
            units_per_level: 2.0,
            weapons: Default::default(),
        }),
        ..PowerConfig::default()
    };
    let groups = [
        AuthoredPowerGroup::at_default_floor(helm(), 2),
        AuthoredPowerGroup::at_default_floor(weapons(), 2),
        AuthoredPowerGroup::at_default_floor(shields(), 2),
        AuthoredPowerGroup {
            id: reserve.clone(),
            level: 0,
            floor: 0,
        },
    ];
    let mut power = PowerSystem::from_authored_groups(&config, &groups);
    power.tick(10.0, &config);
    assert_eq!(
        power.battery_charge, 0.0,
        "unused generation is not passive capture"
    );
    power.set_group_allocation(&reserve, 4).unwrap();
    assert_eq!(
        power.level_for(&reserve),
        2,
        "demands leave only two charging pips"
    );
    power.tick(1.0, &config);
    assert_eq!(power.battery_charge, 4.0);
    power.set_group_allocation(&helm(), 1).unwrap();
    power.set_group_allocation(&reserve, 3).unwrap();
    power.tick(2.0, &config);
    assert_eq!(
        power.battery_charge, 16.0,
        "reducing propulsion funds faster charging"
    );
    power.set_group_allocation(&reserve, 0).unwrap();
    power.tick(500.0, &config);
    assert_eq!(power.battery_charge, 16.0, "zero charging holds reserve");
    let saved = power.capture_continuation();
    let mut resumed = PowerSystem::from_authored_groups(&config, &groups);
    resumed.restore_continuation(&saved);
    let bids = [AllocationBid {
        group: reserve.clone(),
        want: 3,
        max_level: 4,
        floor: 0,
        rule_priority: 20,
    }];
    for (id, level) in plan_allocation(&power, &bids) {
        power.set_group_allocation(&id, level).unwrap();
        resumed.set_group_allocation(&id, level).unwrap();
    }
    assert_eq!(
        power.level_for(&reserve),
        3,
        "Backfill can restart zero charging"
    );
    power.tick(3.0, &config);
    resumed.tick(3.0, &config);
    assert_eq!(power.capture_continuation(), resumed.capture_continuation());
    power.tick(500.0, &config);
    assert_eq!(power.battery_charge, config.capacity);
    power.battery_charge = 0.0;
    assert!(!power.tick(0.0, &config));
    assert!(!power.locked());
}
/// A battery that does not move on its own, for explicit charge fixtures.
fn still_config() -> PowerConfig {
    PowerConfig {
        rates: [0.0; 6],
        ..PowerConfig::default()
    }
}

#[test]
fn defaults() {
    let ps = PowerSystem::default();
    assert_eq!(ps.level_for(&helm()), 2);
    assert_eq!(ps.level_for(&weapons()), 2);
    assert_eq!(ps.level_for(&shields()), 2);
    assert_eq!(ps.battery_charge, 100.0);
}

#[test]
fn increase_helm() {
    let mut ps = PowerSystem::default();
    ps.increase(&helm());
    assert_eq!(ps.level_for(&helm()), 3);
}

#[test]
fn increase_weapons() {
    let mut ps = PowerSystem::default();
    ps.increase(&weapons());
    assert_eq!(ps.level_for(&weapons()), 3);
}

#[test]
fn increase_shields() {
    let mut ps = PowerSystem::default();
    ps.increase(&shields());
    assert_eq!(ps.level_for(&shields()), 3);
}

#[test]
fn increase_at_four_is_noop() {
    let mut ps = PowerSystem::default();
    ps.set_group_allocation(&helm(), 4).unwrap();
    ps.increase(&helm());
    assert_eq!(ps.level_for(&helm()), 4);
}

#[test]
fn increase_at_total_cap_eight_is_noop() {
    let mut ps = PowerSystem::default();
    ps.set_group_allocation(&helm(), 3).unwrap();
    ps.set_group_allocation(&weapons(), 3).unwrap();
    // shields is 2 → total = 8
    assert_eq!(ps.total(), 8);
    ps.increase(&shields());
    assert_eq!(ps.level_for(&shields()), 2);
}

/// While the reactor is locked out after a brownout, `increase` is a no-op:
/// the operator cannot spend power the reserve cannot pay for.
#[test]
fn increase_when_locked_is_noop() {
    let config = still_config();
    let mut ps = PowerSystem::default();
    // Flatten the battery: the reactor locks and forces every group to 1.
    ps.battery_charge = 0.0;
    ps.tick(1.0, &config);
    assert!(ps.locked());
    assert_eq!(ps.total(), 3, "every group slammed to 1");

    ps.increase(&helm());
    assert_eq!(ps.level_for(&helm()), 1, "locked reactor refuses the spend");
}

#[test]
fn decrease_helm() {
    let mut ps = PowerSystem::default();
    ps.decrease(&helm());
    assert_eq!(ps.level_for(&helm()), 1);
}

#[test]
fn decrease_weapons() {
    let mut ps = PowerSystem::default();
    ps.decrease(&weapons());
    assert_eq!(ps.level_for(&weapons()), 1);
}

#[test]
fn decrease_shields() {
    let mut ps = PowerSystem::default();
    ps.decrease(&shields());
    assert_eq!(ps.level_for(&shields()), 1);
}

#[test]
fn decrease_at_one_is_noop() {
    let mut ps = PowerSystem::default();
    ps.set_group_allocation(&helm(), 1).unwrap();
    ps.decrease(&helm());
    assert_eq!(ps.level_for(&helm()), 1);
}

/// While locked, `decrease` is a no-op too: the allocation controls are
/// frozen outright until the reserve recovers.
#[test]
fn decrease_when_locked_is_noop() {
    let config = still_config();
    let mut ps = PowerSystem::default();
    ps.set_group_allocation(&helm(), 3).unwrap();
    ps.battery_charge = 0.0;
    ps.tick(1.0, &config);
    assert!(ps.locked());
    assert_eq!(
        ps.level_for(&helm()),
        1,
        "helm slammed to 1 by the brownout"
    );

    ps.decrease(&helm());
    assert_eq!(
        ps.level_for(&helm()),
        1,
        "locked reactor refuses the change"
    );
}

// ── tick ──────────────────────────────────────────────────────────────

#[test]
fn tick_discharges_above_base() {
    let config = PowerConfig::default();
    let mut ps = PowerSystem::default();
    ps.set_group_allocation(&helm(), 4).unwrap();
    // total = 8 → rate = -6.0/s
    ps.battery_charge = 100.0;
    ps.tick(1.0, &config);
    assert!((ps.battery_charge - 94.0).abs() < 0.001);
}

#[test]
fn alliance_reactor_has_six_free_pips_and_refuses_a_ninth() {
    let config = PowerConfig {
        strike_reserve: None,
        capacity: 70.0,
        rates: [5.0, 4.0, 3.0, 2.0, -2.0, -5.0],
        sustainable_total: 6,
        max_commanded_total: 8,
        emergency_threshold: 20.0,
    };
    let mut power = PowerSystem::new(&config);
    assert_eq!(power.total(), 6);
    assert!(power.is_charging(&config));

    power.increase(&helm());
    assert_eq!(power.total(), 7);
    assert!(power.is_draining(&config));
    power.increase(&weapons());
    assert_eq!(power.total(), 8);
    assert!(power.is_draining(&config));
    power.increase(&shields());
    assert_eq!(power.total(), 8, "a ninth pip is refused");
}

#[test]
fn tick_recharges_below_base() {
    let config = PowerConfig::default();
    let mut ps = PowerSystem::default();
    // total = 6 → rate = 2.0/s
    ps.battery_charge = 50.0;
    ps.tick(1.0, &config);
    assert!((ps.battery_charge - 52.0).abs() < 0.001);
}

#[test]
fn tick_cannot_overcharge_beyond_capacity() {
    let config = PowerConfig::default();
    let mut ps = PowerSystem::default();
    // total = 3 → rate = 6.0/s
    ps.set_group_allocation(&helm(), 1).unwrap();
    ps.set_group_allocation(&weapons(), 1).unwrap();
    ps.set_group_allocation(&shields(), 1).unwrap();
    ps.battery_charge = 99.0;
    ps.tick(1.0, &config);
    assert!((ps.battery_charge - 100.0).abs() < 0.001);
}

// ── exhaustion lock ───────────────────────────────────────────────────

/// **Exhaustion.** Nothing degrades until the battery hits zero; then every
/// group is taken down to 1 in the same instant and the reactor locks. There
/// is no graceful per-group floor — a player who drains the reserve loses
/// the lot. Replaces the issue-#952 floor ladder this reverts.
///
/// Down, never up: `exhaustion_forces_one_but_never_raises_a_cold_group`
/// below covers the direction this one cannot see, since every group here
/// starts warm.
#[test]
fn exhaustion_forces_groups_to_one_and_locks() {
    let config = still_config();
    let mut ps = PowerSystem::default();
    ps.set_group_allocation(&helm(), 3).unwrap();
    ps.set_group_allocation(&weapons(), 3).unwrap();

    // Above zero: the standing order is untouched, whatever the charge.
    ps.battery_charge = 5.0;
    assert!(!ps.tick(1.0, &config));
    assert_eq!(ps.level_for(&helm()), 3);
    assert_eq!(ps.level_for(&weapons()), 3);
    assert!(!ps.locked());

    // Zero: the whole reactor browns out at once.
    ps.battery_charge = 0.0;
    assert!(ps.tick(1.0, &config), "the lock engaged");
    assert_eq!(ps.level_for(&helm()), 1);
    assert_eq!(ps.level_for(&weapons()), 1);
    assert_eq!(ps.level_for(&shields()), 1);
    assert!(ps.locked());
}

// ── Issue #1395: a group its hull authored at min_level 0 ────────────────

/// The Alliance shape since #1395: weapons may be taken cold, helm and
/// shields may not.
fn reactor_with_coldable_weapons() -> PowerSystem {
    PowerSystem::from_authored_groups(
        &PowerConfig::default(),
        &[
            seed(helm(), 2),
            coldable_seed(weapons(), 2),
            seed(shields(), 2),
        ],
    )
}

/// **The floor is the group's own.** A hull that authored `min_level = 0`
/// for weapons can have that group ordered to 0; one that authored 1 for
/// helm cannot, and the same command against helm stops at 1.
#[test]
fn a_group_authored_at_zero_can_be_commanded_cold_and_its_neighbours_cannot() {
    let mut ps = reactor_with_coldable_weapons();
    assert_eq!(ps.floor_for(&weapons()), 0);
    assert_eq!(ps.floor_for(&helm()), GROUP_LEVEL_MIN);

    ps.set_group_allocation(&weapons(), 0).unwrap();
    assert_eq!(ps.level_for(&weapons()), 0);
    assert!(ps.is_group_cold(&weapons()));

    ps.set_group_allocation(&helm(), 0).unwrap();
    assert_eq!(
        ps.level_for(&helm()),
        1,
        "helm authored a floor of 1 and the same order is clamped to it"
    );
    assert!(!ps.is_group_cold(&helm()));
}

/// `is_group_cold` is about a group this reactor HAS. `level_for` returns 0
/// for a group it has never heard of, so the bare level test would read
/// every hull without a weapons group as having cold weapons — the exact
/// opposite of what a fire gate wants to hear.
#[test]
fn an_untracked_group_is_not_cold() {
    let ps = reactor_with_coldable_weapons();
    let unknown = PowerGroupId("tractor".into());
    assert_eq!(ps.level_for(&unknown), 0);
    assert!(!ps.is_group_cold(&unknown));
    assert_eq!(
        ps.floor_for(&unknown),
        GROUP_LEVEL_MIN,
        "and its floor reads as the conservative default, not as 0"
    );
}

/// `decrease` walks a coldable group all the way off, one step at a time,
/// and stops there.
#[test]
fn decrease_stops_at_the_groups_own_floor() {
    let mut ps = reactor_with_coldable_weapons();
    for _ in 0..4 {
        ps.decrease(&weapons());
    }
    assert_eq!(ps.level_for(&weapons()), 0, "weapons authored a floor of 0");

    for _ in 0..4 {
        ps.decrease(&shields());
    }
    assert_eq!(
        ps.level_for(&shields()),
        1,
        "shields authored a floor of 1 and holds there"
    );
}

/// **Exhaustion takes power away; it does not hand it out.** The lock still
/// forces every warm group to 1, but a group the crew switched off stays
/// off — otherwise a flat battery would be the thing that put the guns back
/// on, undoing a standing order nobody had cancelled.
#[test]
fn exhaustion_forces_one_but_never_raises_a_cold_group() {
    let config = still_config();
    let mut ps = reactor_with_coldable_weapons();
    ps.set_group_allocation(&weapons(), 0).unwrap();
    ps.set_group_allocation(&helm(), 4).unwrap();

    ps.battery_charge = 0.0;
    assert!(ps.tick(1.0, &config), "the lock engaged");
    assert_eq!(ps.level_for(&helm()), 1, "a warm group is forced down to 1");
    assert_eq!(ps.level_for(&shields()), 1);
    assert_eq!(ps.level_for(&weapons()), 0, "a cold group is left cold");
    assert!(ps.is_group_cold(&weapons()));
    assert!(ps.locked());
}

/// **The resume bug.** `restore` clamped to the global minimum, so a ship
/// saved with its weapons cold came back at level 1 with live guns. It now
/// clamps to the group's own floor.
#[test]
fn restore_reinstates_a_cold_group() {
    let mut ps = reactor_with_coldable_weapons();
    assert_eq!(
        ps.level_for(&weapons()),
        2,
        "precondition: the fresh reactor is warm, so what comes back below \
             cannot be a bootstrap coincidence"
    );

    ps.restore(&[(helm(), 3), (weapons(), 0), (shields(), 2)], 40.0, false);

    assert_eq!(ps.level_for(&weapons()), 0);
    assert!(ps.is_group_cold(&weapons()));
    assert_eq!(ps.level_for(&helm()), 3);
    assert_eq!(ps.battery_charge, 40.0);
}

/// The floors are the HULL's, not the save's. A payload carrying a level
/// under a group's authored floor is still lifted to it — the ship the
/// resume boots is the ship the file describes, and a save cannot re-author
/// what an officer is allowed to command.
#[test]
fn restore_still_lifts_a_group_below_its_own_floor() {
    let mut ps = reactor_with_coldable_weapons();
    ps.restore(&[(helm(), 0), (weapons(), 0), (shields(), 2)], 40.0, false);
    assert_eq!(ps.level_for(&helm()), 1, "helm's authored floor is 1");
    assert_eq!(ps.level_for(&weapons()), 0, "weapons' authored floor is 0");
}

/// Recovery: once locked, the reactor stays locked until the charge climbs
/// back to `emergency_threshold`, and only then do the controls unfreeze.
#[test]
fn recovery_unlocks_at_the_emergency_threshold() {
    let config = still_config(); // emergency_threshold 25
    let mut ps = PowerSystem::default();
    ps.battery_charge = 0.0;
    ps.tick(1.0, &config);
    assert!(ps.locked());

    // Below the threshold: still locked, controls still frozen.
    ps.battery_charge = 20.0;
    assert!(
        !ps.tick(1.0, &config),
        "no edge — still under the threshold"
    );
    assert!(ps.locked());
    ps.increase(&helm());
    assert_eq!(ps.level_for(&helm()), 1, "frozen below the threshold");

    // At the threshold: the lock releases.
    ps.battery_charge = 25.0;
    assert!(ps.tick(1.0, &config), "the lock released");
    assert!(!ps.locked());
    ps.increase(&helm());
    assert_eq!(ps.level_for(&helm()), 2, "controls live again");
}

/// Dropping to 1 across the board lowers the draw, so a bottomed-out reactor
/// recharges instead of sitting flat for ever.
#[test]
fn a_locked_reactor_recharges_on_the_minimum_draw() {
    let config = PowerConfig::default(); // rates[0] (total 3) = +6/s
    let mut ps = PowerSystem::default();
    ps.set_group_allocation(&helm(), 4).unwrap(); // total 8 → -6/s
    ps.battery_charge = 2.0;

    // This tick flattens the battery and locks: groups go to 1, total 3.
    ps.tick(1.0, &config);
    assert_eq!(ps.battery_charge, 0.0);
    assert!(ps.locked());
    assert_eq!(ps.total(), 3);
    // Next tick draws at the locked total of 3 → +6/s.
    ps.tick(1.0, &config);
    assert!(
        (ps.battery_charge - 6.0).abs() < 0.001,
        "got {}",
        ps.battery_charge
    );
}

/// `is_charging` is not `!is_draining`: at a rate of exactly zero the
/// reserve is frozen, and the gauge must claim neither.
#[test]
fn a_zero_rate_is_neither_draining_nor_charging() {
    let config = PowerConfig {
        rates: [3.0, 2.0, 1.0, 0.0, -1.0, -3.0],
        ..PowerConfig::default()
    };
    let ps = PowerSystem::default(); // total 6 → rates[3] = 0.0
    assert_eq!(ps.total(), 6);
    assert!(!ps.is_draining(&config));
    assert!(
        !ps.is_charging(&config),
        "a frozen reserve must not paint the pulsing CHARGING indicator"
    );
}

/// The tick's return value is the lock-changed edge, which
/// `ship::power::tick_power_brownout_advisory` hangs its debounce off.
#[test]
fn tick_returns_true_when_lock_changes() {
    let config = still_config();
    let mut ps = PowerSystem::default();
    ps.battery_charge = 0.0;
    assert!(ps.tick(1.0, &config), "locked out this tick");
    assert!(!ps.tick(1.0, &config), "still locked — no edge");
    ps.battery_charge = 100.0;
    assert!(ps.tick(1.0, &config), "unlocked this tick");
}

// ── configurable constructor ──────────────────────────────────────────

#[test]
fn custom_config() {
    let config = PowerConfig {
        strike_reserve: None,
        capacity: 50.0,
        rates: [1.0, 1.0, 1.0, -1.0, -2.0, -3.0],
        sustainable_total: 5,
        max_commanded_total: 8,
        emergency_threshold: 10.0,
    };
    let ps = PowerSystem::new(&config);
    assert_eq!(ps.level_for(&helm()), 2);
    assert_eq!(ps.level_for(&weapons()), 2);
    assert_eq!(ps.level_for(&shields()), 2);
    assert!((ps.battery_charge - 50.0).abs() < 0.001);
}

/// Re-authored from `increase_on_unknown_group_is_noop`, which asserted
/// that `"shields"` was an unknown group. That is exactly backwards since
/// issue #952: `shields` IS one of the three canonical groups now, and
/// `sensors` is the one that no longer exists. Left as it was the test
/// would have gone on passing while asserting nothing — the increase it
/// aimed at an "unknown" group would have quietly moved a real one, and the
/// three follow-up assertions read the two groups it did not touch.
#[test]
fn increase_on_unknown_group_is_noop() {
    let mut ps = PowerSystem::default();
    // Neither sensors nor navigation are seeded as power groups.
    ps.increase(&PowerGroupId("sensors".into()));
    assert_eq!(ps.level_for(&helm()), 2);
    assert_eq!(ps.level_for(&weapons()), 2);
    assert_eq!(ps.level_for(&shields()), 2);

    ps.increase(&PowerGroupId("navigation".into()));
    assert_eq!(ps.level_for(&helm()), 2);
    assert_eq!(ps.level_for(&weapons()), 2);
    assert_eq!(ps.level_for(&shields()), 2);
    assert_eq!(ps.total(), 6, "no stray group was created either");
}

#[test]
fn set_group_allocation_updates_named_group() {
    let mut ps = PowerSystem::default();

    ps.set_group_allocation(&weapons(), 3).unwrap();

    assert_eq!(ps.level_for(&weapons()), 3);
    assert_eq!(ps.level_for(&helm()), 2);
    assert_eq!(ps.level_for(&shields()), 2);
}

#[test]
fn set_group_allocation_rejects_unknown_group() {
    let mut ps = PowerSystem::default();
    let group = PowerGroupId("life-support".into());

    assert_eq!(
        ps.set_group_allocation(&group, 3),
        Err(PowerAllocationError::UnknownGroup(group))
    );
    assert_eq!(ps.total(), 6);
}

#[test]
fn channel_1_read_exposes_power_without_mutation_access() {
    let mut ps = PowerSystem::default();
    ps.set_group_allocation(&helm(), 4).unwrap();
    let state = ps.read_state();
    let channel_1 = Channel1Read::new(&state);

    assert_eq!(channel_1.power_level(&helm()), Some(4));
    assert_eq!(channel_1.power_level(&PowerGroupId("unknown".into())), None);
    assert_eq!(ps.level_for(&helm()), 4);
}

// ── Budget-aware allocation planning (issue #959) ────────────────────────

fn auxiliary() -> PowerGroupId {
    PowerGroupId("auxiliary".into())
}

/// A bid at the shipped fleet's ordering: every elevation rule authored at
/// priority 10, ties falling back to the caller's group order. The group
/// authors no floor of its own, so it takes the parse default.
fn bid(group: PowerGroupId, want: u8, rule_priority: i32) -> AllocationBid {
    AllocationBid {
        group,
        want,
        max_level: crate::ship::config::default_max_power_level(),
        floor: GROUP_LEVEL_MIN,
        rule_priority,
    }
}

/// The same bid for a group whose hull authored `min_level = 0` — one that
/// may be taken cold (issue #1395).
fn coldable_bid(group: PowerGroupId, want: u8, rule_priority: i32) -> AllocationBid {
    AllocationBid {
        floor: 0,
        ..bid(group, want, rule_priority)
    }
}

/// A seed entry for a group that authors no floor.
fn seed(group: PowerGroupId, level: u8) -> AuthoredPowerGroup {
    AuthoredPowerGroup::at_default_floor(group, level)
}

/// A seed entry for a group whose hull authored `min_level = 0`.
fn coldable_seed(group: PowerGroupId, level: u8) -> AuthoredPowerGroup {
    AuthoredPowerGroup {
        id: group,
        level,
        floor: 0,
    }
}

/// The four-group Alliance shape: `ops` outside the canonical trio, seeded
/// at 1, with nothing in the policy bidding for it.
fn reactor_with_auxiliary_group() -> PowerSystem {
    PowerSystem::from_authored_groups(
        &PowerConfig::default(),
        &[
            seed(helm(), 2),
            seed(weapons(), 2),
            seed(shields(), 1),
            seed(auxiliary(), 1),
        ],
    )
}

/// **Allocate within the max.** The shipped combat-stations allocation —
/// helm 3 / weapons 3 against `ops` 1 and `shields` 1 — spends the budget
/// exactly, and every planned level is inside the group's own ceiling.
#[test]
fn plan_allocation_spends_the_budget_without_exceeding_it() {
    let ps = reactor_with_auxiliary_group();
    let plan = plan_allocation(&ps, &[bid(helm(), 3, 10), bid(weapons(), 3, 10)]);

    let mut after = ps.clone();
    for (group, level) in &plan {
        after.set_group_allocation(group, *level).unwrap();
    }
    assert_eq!(after.commanded_level_for(&helm()), 3);
    assert_eq!(after.commanded_level_for(&weapons()), 3);
    assert_eq!(after.commanded_total(), ps.max_commanded_total());
    assert!(
        plan.iter().all(|(_, l)| *l <= GROUP_LEVEL_MAX),
        "no planned level may exceed the per-group ceiling"
    );
}

/// A bid over the group's OWN authored `max_level` is trimmed by the
/// planner, not by the applier. Trimming it downstream is what made the
/// difference invisible: the applier's clamp is silent, so the decider went
/// on asking for a level the hull had already ruled out.
#[test]
fn plan_allocation_trims_a_bid_to_the_groups_authored_max_level() {
    let mut ps = reactor_with_auxiliary_group();
    ps.set_group_allocation(&weapons(), 1).unwrap();
    let capped = AllocationBid {
        max_level: 2,
        ..bid(weapons(), 4, 10)
    };
    assert_eq!(
        plan_allocation(&ps, std::slice::from_ref(&capped)),
        vec![(weapons(), 2)],
        "the bid asked for 4 and the hull's own max_level is 2"
    );

    // And once it is there, the trimmed bid settles: nothing further is
    // planned, so the ceiling cannot become a re-emit loop of its own.
    ps.set_group_allocation(&weapons(), 2).unwrap();
    assert!(plan_allocation(&ps, &[capped]).is_empty());
}

/// **Budget collision.** Three groups asking for more than the reactor can
/// pay for are rationed in AUTHORED priority order, and the plan still
/// fits: the top-priority bid is paid in full, the next takes what is left,
/// the last lands on its minimum. Nothing is refused by the applier.
#[test]
fn plan_allocation_rations_a_budget_collision_by_authored_priority() {
    let ps = reactor_with_auxiliary_group();
    let plan = plan_allocation(
        &ps,
        &[
            bid(helm(), 4, 5),
            bid(weapons(), 4, 30),
            bid(shields(), 4, 20),
        ],
    );

    let mut after = ps.clone();
    for (group, level) in &plan {
        after.set_group_allocation(group, *level).unwrap();
    }
    // ops holds 1 (nothing bid for it); 7 points are left for three groups
    // that each need at least 1, so 4 are discretionary: weapons (30) takes
    // 3, shields (20) takes the last 1, helm (5) gets none.
    assert_eq!(after.commanded_level_for(&weapons()), 4);
    assert_eq!(after.commanded_level_for(&shields()), 2);
    assert_eq!(after.commanded_level_for(&helm()), 1);
    assert_eq!(
        after.commanded_level_for(&auxiliary()),
        1,
        "un-bid groups are reserved"
    );
    assert_eq!(after.commanded_total(), ps.max_commanded_total());
    // And every level the plan asked for is the level the reactor actually
    // holds — the whole point: no silent refusal anywhere in the plan.
    for (group, level) in &plan {
        assert_eq!(
            after.commanded_level_for(group),
            *level,
            "{} was planned at {level} and the applier refused it",
            group.0
        );
    }
}

/// An equal-priority tie falls back to the caller's deterministic group
/// order (`POWER_GROUP_ORDER`: helm before weapons), not to any preference
/// baked into the planner. With the battery floors reverted there is no
/// secondary authored key left, so this fallback is the whole tie-break.
#[test]
fn plan_allocation_breaks_a_priority_tie_on_the_callers_group_order() {
    let ps = reactor_with_auxiliary_group();
    // Both at priority 10, both asking for 4, only 4 discretionary points.
    let plan = plan_allocation(&ps, &[bid(helm(), 4, 10), bid(weapons(), 4, 10)]);
    let mut after = ps.clone();
    for (group, level) in &plan {
        after.set_group_allocation(group, *level).unwrap();
    }
    assert_eq!(
        after.commanded_level_for(&helm()),
        4,
        "helm precedes weapons in POWER_GROUP_ORDER, so the tie serves it first"
    );
    assert_eq!(after.commanded_level_for(&weapons()), 2);
}

/// **No re-emit stall.** Re-planning against the reactor the previous plan
/// produced returns NOTHING — the decision has settled, so the host emits
/// nothing and admission stays quiet. This is the invariant the old
/// per-group emit could not hold: its refused command was re-issued on
/// every decision arm for ever.
#[test]
fn plan_allocation_settles_and_stops_emitting() {
    let mut ps = reactor_with_auxiliary_group();
    let bids = [
        bid(helm(), 4, 10),
        bid(weapons(), 4, 10),
        bid(shields(), 4, 10),
    ];

    let first = plan_allocation(&ps, &bids);
    assert!(!first.is_empty(), "the first arm has work to do");
    for (group, level) in &first {
        ps.set_group_allocation(group, *level).unwrap();
    }
    assert!(ps.commanded_total() <= ps.max_commanded_total());

    for arm in 0..5 {
        let again = plan_allocation(&ps, &bids);
        assert!(
            again.is_empty(),
            "arm {arm} re-emitted {again:?} after the allocation had settled"
        );
    }
}

/// Decreases are ordered ahead of increases, because the applier tests the
/// budget one command at a time. A plan that ends at the cap is refused
/// halfway through if the increase lands before the decrease that pays for
/// it — which is the silent refusal wearing a different hat.
#[test]
fn plan_allocation_orders_decreases_before_the_increases_they_pay_for() {
    // Commanded at the cap already: helm 4 / weapons 2 / shields 1 / ops 1.
    let mut ps = reactor_with_auxiliary_group();
    ps.set_group_allocation(&helm(), 4).unwrap();
    assert_eq!(ps.commanded_total(), ps.max_commanded_total());

    // The policy now wants the two swapped over.
    let plan = plan_allocation(&ps, &[bid(helm(), 2, 10), bid(weapons(), 4, 20)]);
    assert_eq!(
        plan.first().map(|(g, l)| (g.0.as_str(), *l)),
        Some((HELM_POWER_GROUP, 2)),
        "the decrease must come first: {plan:?}"
    );

    // Apply in the planned order through the real applier semantics.
    for (group, level) in &plan {
        ps.set_group_allocation(group, *level).unwrap();
    }
    assert_eq!(
        ps.commanded_level_for(&weapons()),
        4,
        "the increase was paid for"
    );
    assert_eq!(ps.commanded_level_for(&helm()), 2);
}

/// A group the reactor does not track is dropped rather than charged to the
/// budget — the applier would reject it as `UnknownGroup`, and counting it
/// would starve a real group of a point that was never spent.
#[test]
fn plan_allocation_ignores_a_bid_for_an_untracked_group() {
    let ps = reactor_with_auxiliary_group();
    let plan = plan_allocation(
        &ps,
        &[
            bid(PowerGroupId("life-support".into()), 4, 100),
            bid(weapons(), 4, 10),
        ],
    );
    assert_eq!(plan, vec![(weapons(), 4)]);
}

// ── Issue #1395: the planner and a coldable group ────────────────────────

/// **The AI never raises a cold group.** Every Alliance hull ships an
/// unconditional priority-0 weapons rule bidding level 2; without this the
/// planner would warm a cold weapons group on the very next decision arm
/// and restraint would last one tick.
#[test]
fn plan_allocation_never_raises_a_cold_group() {
    let mut ps = reactor_with_coldable_weapons();
    ps.set_group_allocation(&weapons(), 0).unwrap();

    let plan = plan_allocation(&ps, &[coldable_bid(weapons(), 2, 0), bid(helm(), 2, 10)]);
    assert!(
        !plan.iter().any(|(id, _)| id == &weapons()),
        "the cold group is not in the plan at all: {plan:?}"
    );
    assert!(plan.is_empty(), "and helm was already at 2: {plan:?}");
}

/// The freed point is genuinely freed. A cold group is reserved at 0 rather
/// than at the global minimum, so the rest of the reactor may spend what it
/// is not using — helm to its ceiling on a budget that could not otherwise
/// have paid for it.
#[test]
fn a_cold_group_hands_its_point_back_to_the_budget() {
    let mut ps = reactor_with_coldable_weapons();
    ps.set_group_allocation(&weapons(), 0).unwrap();

    let plan = plan_allocation(&ps, &[bid(helm(), 4, 10), bid(shields(), 4, 10)]);
    let mut after = ps.clone();
    for (group, level) in &plan {
        after.set_group_allocation(group, *level).unwrap();
    }
    assert_eq!(after.level_for(&helm()), 4);
    assert_eq!(after.level_for(&shields()), 4);
    assert_eq!(after.level_for(&weapons()), 0);
    assert_eq!(
        after.commanded_total(),
        8,
        "the whole 8-point budget went to the two warm groups"
    );
}

/// **The other half of the rule: the AI never PARKS a group cold either.**
///
/// Rationing hands a group as much as the budget reaches, and for a group
/// whose floor is 0 that could reach zero. Here helm and shields hold the
/// whole 8-point budget between them — the over-authored shape the test
/// below documents, and the only way a bidder's discretionary share can
/// saturate to nothing — and weapons bids 3 with not a point left to pay
/// for it. It holds the 1 it has rather than being cut to 0. Otherwise a
/// busy reactor would switch the guns off, and the cold rule above would
/// then keep them off for the rest of the encounter.
#[test]
fn plan_allocation_rations_a_coldable_group_down_to_one_not_to_zero() {
    let ps = PowerSystem::from_authored_groups(
        &PowerConfig::default(),
        &[
            seed(helm(), 4),
            coldable_seed(weapons(), 1),
            seed(shields(), 4),
        ],
    );
    assert_eq!(ps.commanded_total(), 9, "deliberately over the 8-point cap");

    let plan = plan_allocation(&ps, &[coldable_bid(weapons(), 3, 0)]);
    assert_eq!(
        plan,
        vec![],
        "no spare to grant, and the guarantee holds weapons at the 1 it \
             already has rather than cutting it to 0: {plan:?}"
    );
}

/// A rule that ASKS for 0 is served, because that is an authored decision
/// rather than a rounding outcome — and it costs the budget nothing, so the
/// point it gives up is available to the group bidding beside it.
#[test]
fn plan_allocation_serves_a_rule_that_bids_a_group_cold() {
    let ps = reactor_with_coldable_weapons();
    let plan = plan_allocation(&ps, &[coldable_bid(weapons(), 0, 10), bid(helm(), 4, 5)]);
    assert_eq!(
        plan,
        vec![(weapons(), 0), (helm(), 4)],
        "the decrease is ordered first, as ever, and helm gets the point"
    );
}

/// A group whose hull authored a floor ABOVE the global minimum is
/// guaranteed its own floor, not 1 — the same rule read from the other end.
/// Helm here holds 4 and shields 3, leaving one discretionary point for an
/// `ops` group authored at `min_level = 2` bidding 4: it lands on 3.
#[test]
fn plan_allocation_guarantees_a_group_its_own_raised_floor() {
    let mut ps = PowerSystem::from_authored_groups(
        &PowerConfig::default(),
        &[
            seed(helm(), 2),
            seed(shields(), 2),
            AuthoredPowerGroup {
                id: auxiliary(),
                level: 2,
                floor: 2,
            },
        ],
    );
    ps.set_group_allocation(&helm(), 4).unwrap();
    assert_eq!(ps.commanded_total(), 8);

    let raised = AllocationBid {
        floor: 2,
        ..bid(auxiliary(), 4, 10)
    };
    let plan = plan_allocation(&ps, &[raised]);
    assert_eq!(
        plan,
        vec![],
        "helm 4 + shields 2 reserved is 6; ops is guaranteed its own floor of \
             2, which is exactly what it already holds: {plan:?}"
    );

    ps.set_group_allocation(&shields(), 1).unwrap();
    let raised = AllocationBid {
        floor: 2,
        ..bid(auxiliary(), 4, 10)
    };
    let plan = plan_allocation(&ps, &[raised]);
    assert_eq!(
        plan,
        vec![(auxiliary(), 3)],
        "one point freed, one point granted on top of the authored floor"
    );
}

/// **The qualifier on "the returned total fits."** A hull whose
/// `[power_groups.*] default_level` values already sum past
/// the authored `max_commanded_total` is not something this function can undo, and
/// nothing rejects that authoring at load — so the doc claim is "never
/// rises above the budget", not "always fits", and this is the case that
/// makes the difference.
///
/// What IS guaranteed on such a reactor is that the plan never makes the
/// overspend worse: `reserved + mins` is already over budget, `spare`
/// saturates to 0, and the plan carries decreases only — so the applier has
/// nothing to refuse and there is nothing to re-emit next arm.
#[test]
fn plan_allocation_cannot_rescue_a_reactor_authored_over_its_own_budget() {
    let life_support = PowerGroupId("life-support".into());
    // Five groups seeded at 2: an authoring nothing rejects, and a commanded
    // total of 10 against a budget of 8.
    let over = PowerSystem::from_authored_groups(
        &PowerConfig::default(),
        &[
            seed(helm(), 2),
            seed(weapons(), 2),
            seed(shields(), 2),
            seed(auxiliary(), 2),
            seed(life_support.clone(), 2),
        ],
    );
    assert!(over.commanded_total() > over.max_commanded_total());

    // Only helm bids. The other four hold 8 between them, so there is no
    // discretionary budget at all and nothing licenses cutting them.
    let plan = plan_allocation(&over, &[bid(helm(), 4, 10)]);
    assert_eq!(plan, vec![(helm(), GROUP_LEVEL_MIN)]);
    assert!(
        plan.iter()
            .all(|(id, level)| *level <= over.commanded_level_for(id)),
        "an over-budget reactor must never be handed an increase"
    );

    let mut after = over.clone();
    for (group, level) in &plan {
        after.set_group_allocation(group, *level).unwrap();
    }
    assert_eq!(
        after.commanded_total(),
        9,
        "the plan lowers the overspend as far as it may and no further"
    );

    // The other arm, and the reason the claim is conditional rather than
    // simply false: when EVERY group bids there is nothing held outside the
    // plan, and the same reactor does land inside the budget.
    let all_bid: Vec<AllocationBid> = over.iter().map(|(id, _)| bid(id.clone(), 4, 10)).collect();
    let mut after_all = over.clone();
    for (group, level) in plan_allocation(&over, &all_bid) {
        after_all.set_group_allocation(&group, level).unwrap();
    }
    assert_eq!(after_all.commanded_total(), after_all.max_commanded_total());
}
