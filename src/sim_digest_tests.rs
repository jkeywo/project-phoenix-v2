//! Tests for [`crate::sim_digest`] — relocated out of the production file
//! under AGENTS.md's 1,000-line rule (issue #1180's convention), unchanged
//! otherwise: same `use super::*`, same fixtures, same assertions.

use super::*;

#[test]
fn nan_flavours_fold_to_one_payload() {
    let quiet = f32::NAN;
    let other = f32::from_bits(0x7fc0_1234);
    assert!(other.is_nan(), "fixture must actually be a NaN");
    assert_eq!(canon_f32(quiet), canon_f32(other));
}

#[test]
fn signed_zeroes_fold_alike_but_ordinary_values_do_not() {
    assert_eq!(canon_f32(0.0), canon_f32(-0.0));
    assert_ne!(canon_f32(1.0), canon_f32(-1.0));
}

/// No quantisation: the smallest representable difference must move the
/// fold, or a digest reports the wrong tick a split happened on.
#[test]
fn one_ulp_moves_the_fold() {
    let a = 1.0f32;
    let b = f32::from_bits(a.to_bits() + 1);
    assert_ne!(fold_f32(FOLD_SEED, a), fold_f32(FOLD_SEED, b));
}

/// Namespaces group before ids, and the numeric pair orders within one —
/// the whole of the fold-order policy, asserted rather than assumed.
#[test]
fn fold_keys_group_by_namespace_then_order_numerically() {
    let id = |tick, seq| crate::world_id::WorldId::new(Namespace::Entity, tick, seq).render();
    let (t2, t10, t10s2) = (id(2, 1), id(10, 1), id(10, 2));
    let mut keys = [
        FoldKey::from_world_id(Namespace::Asteroid, &t2),
        FoldKey::from_world_id(Namespace::Entity, &t10),
        FoldKey::from_world_id(Namespace::Entity, &t10s2),
        FoldKey::from_world_id(Namespace::Entity, &t2),
        FoldKey::from_world_id(Namespace::Asteroid, &t10),
    ];
    keys.sort();
    let rendered: Vec<_> = keys.iter().map(|k| (k.namespace, k.tick, k.seq)).collect();
    assert_eq!(
        rendered,
        vec![
            (Namespace::Entity, 2, 1),
            (Namespace::Entity, 10, 1),
            (Namespace::Entity, 10, 2),
            (Namespace::Asteroid, 2, 1),
            (Namespace::Asteroid, 10, 1),
        ],
        "namespaces group first, then the numeric pair orders within one"
    );
}

/// The failure the structured key exists to prevent, stated as its own
/// assertion: an *unpadded* `tick-seq` rendering sorts 10 before 2, so a
/// fold that sorted strings would reorder itself as the run got longer.
/// `WorldId::render` is fixed-width hex, and the key compares numbers
/// regardless of what the rendering does.
#[test]
fn the_naive_string_rendering_would_have_sorted_wrongly() {
    assert!("10-1" < "2-1");
    let a = crate::world_id::WorldId::new(Namespace::Entity, 2, 1);
    let b = crate::world_id::WorldId::new(Namespace::Entity, 10, 1);
    assert!(a < b, "the structured tuple orders numerically");
    assert!(a.render() < b.render(), "and so does the padded rendering");
}

/// A **v4** uuid must NOT be read as a mint. This is asteroids' live case,
/// not a hypothetical: `deterministic_cell_uuid` ids stay v4-shaped by
/// design (constraint 8), and since a mint is now uuid-shaped too, the
/// version nibble is the entire difference between "fold this at tick 0 on
/// its string" and "invent a tick and a sequence for a rock".
#[test]
fn a_v4_uuid_keys_as_zero_and_sorts_on_its_string() {
    let key = FoldKey::from_world_id(Namespace::Entity, "a1b2c3d4-0000-4000-8000-000000000001");
    assert_eq!((key.tick, key.seq), (0, 0));
    assert_eq!(key.id, "a1b2c3d4-0000-4000-8000-000000000001");
}

/// Register every component the fold walks, so `World::try_query` can see
/// them even in a world where no entity happens to carry one.
fn fold_world() -> World {
    let mut world = World::new();
    world.register_component::<EntityUuid>();
    world.register_component::<ShipPhysics>();
    world.register_component::<EntitySystemHull>();
    world.register_component::<ShipRedAlert>();
    world.register_component::<AsteroidUuid>();
    world.register_component::<Transform>();
    world.register_component::<InfrastructureCondition>();
    world.register_component::<CivilianTraffic>();
    world.register_component::<TractorBeam>();
    world.register_component::<TransferUmbilical>();
    world
}

fn spawn_ship(world: &mut World, uuid: &str, x: f32) {
    world.spawn((
        EntityUuid(uuid.to_string()),
        ShipPhysics {
            x,
            ..Default::default()
        },
        ShipRedAlert(false),
    ));
}

fn spawn_rock(world: &mut World, uuid: &str, x: f32) {
    world.spawn((
        AsteroidUuid(uuid.to_string()),
        Transform::from_xyz(x, 0.0, 0.0),
    ));
}

/// A structure with one threshold, at `condition` of 100 points.
fn spawn_structure(world: &mut World, uuid: &str, condition: f32) {
    let config = crate::infrastructure::InfrastructureConfig {
        condition_max: 100.0,
        condition: Some(condition),
        thresholds: vec![crate::infrastructure::ThresholdConfig {
            label: None,
            flag: "transfer_capable".to_string(),
            capacity: None,
            fails_below: 0.4,
            restores_above: None,
        }],
        ..Default::default()
    };
    world.spawn((
        EntityUuid(uuid.to_string()),
        InfrastructureCondition(InfrastructureState::from_config(&config)),
    ));
}

/// A tug, optionally holding a named derelict on its tractor.
fn spawn_tug(world: &mut World, uuid: &str, holding: Option<&str>) {
    let mut beam = TractorBeam::new(
        crate::tractor::TractorConfig {
            range: 600.0,
            coupling_offset: [0.0, 0.0, -120.0],
            min_power_level: 2,
            tow_load: crate::tractor::TowLoadCurve {
                half_penalty_mass: 10_000.0,
                max_penalty: 0.8,
            },
        },
        crate::core::messages::PowerGroupId("tractor".into()),
    );
    if let Some(target) = holding {
        beam.engaged = true;
        beam.coupled_target = Some(target.to_string());
    }
    world.spawn((EntityUuid(uuid.to_string()), beam));
}

/// **Issue #1156.** A world with no tractor holding folds to exactly the
/// number it did before the namespace existed — and so does a hull that
/// authored a tractor and is holding nothing. Only an ACTIVE grip moves it,
/// and the coupled target is what moves it, because a held derelict carries
/// no `ShipPhysics` and nothing else folds its position.
#[test]
fn only_a_held_tractor_moves_the_digest_and_the_target_is_what_moves_it() {
    let id = "00000000-0000-8000-8000-000000000001";

    let mut idle = fold_world();
    spawn_tug(&mut idle, id, None);
    assert_eq!(
        fold_tractor_namespace(&idle, FOLD_SEED),
        FOLD_SEED,
        "a tractor holding nothing folds nothing — a folded row here would have moved every \
         committed world's digest for state none of them carry"
    );

    let mut holding_a = fold_world();
    spawn_tug(&mut holding_a, id, Some("derelict-A"));
    let mut holding_a_again = fold_world();
    spawn_tug(&mut holding_a_again, id, Some("derelict-A"));
    let mut holding_b = fold_world();
    spawn_tug(&mut holding_b, id, Some("derelict-B"));

    assert_ne!(
        world_digest(&idle),
        world_digest(&holding_a),
        "engaging the tractor and taking a derelict under tow must move the digest — nothing \
         else records that the hulk is being dragged"
    );
    assert_eq!(
        world_digest(&holding_a),
        world_digest(&holding_a_again),
        "two hosts holding the same derelict must agree"
    );
    assert_ne!(
        world_digest(&holding_a),
        world_digest(&holding_b),
        "…and a host that thinks the beam has a different hulk must fold to a different number"
    );
}

/// A docker, optionally mated to a named berth.
fn spawn_docker(world: &mut World, uuid: &str, docked_to: Option<&str>) {
    let mut control = DockControl::new(
        crate::ship::system_registry::dock_system_id(),
        crate::dock::DockConfig {
            range: 200.0,
            engage_distance: 400.0,
            approach_speed: 60.0,
            mate_tolerance: 4.0,
            undock_clear_distance: 120.0,
            min_power_level: 2,
        },
        crate::core::messages::PowerGroupId("dock".into()),
    );
    if let Some(target) = docked_to {
        control.engaged = true;
        control.docked = true;
        control.docking_target = Some(target.to_string());
    }
    world.spawn((EntityUuid(uuid.to_string()), control));
}

/// **Issue #1159.** A world with nobody docked folds to exactly the number it
/// did before the namespace existed — and so does a hull that authored a dock
/// and is not docked. Only a real MATE moves it, and which two hulls are
/// joined is what moves it, because that relationship is folded nowhere else.
#[test]
fn only_a_docked_ship_moves_the_digest_and_the_partner_is_what_moves_it() {
    let id = "00000000-0000-8000-8000-000000000001";

    let mut idle = fold_world();
    spawn_docker(&mut idle, id, None);
    assert_eq!(
        fold_dock_namespace(&idle, FOLD_SEED),
        FOLD_SEED,
        "a dock mated to nothing folds nothing — a folded row here would have moved every \
         committed world's digest for state none of them carry"
    );

    let mut docked_a = fold_world();
    spawn_docker(&mut docked_a, id, Some("berth-A"));
    let mut docked_a_again = fold_world();
    spawn_docker(&mut docked_a_again, id, Some("berth-A"));
    let mut docked_b = fold_world();
    spawn_docker(&mut docked_b, id, Some("berth-B"));

    assert_ne!(
        world_digest(&idle),
        world_digest(&docked_a),
        "mating a dock must move the digest — nothing else records that the two hulls are joined"
    );
    assert_eq!(
        world_digest(&docked_a),
        world_digest(&docked_a_again),
        "two hosts docked to the same berth must agree"
    );
    assert_ne!(
        world_digest(&docked_a),
        world_digest(&docked_b),
        "…and a host that thinks the ship is docked to a different hull must fold to a \
         different number"
    );
}

/// An umbilical-carrying hull, optionally running.
fn spawn_umbilical(world: &mut World, uuid: &str, running: bool) {
    let mut umbilical = TransferUmbilical::new(
        crate::umbilical::UmbilicalConfig {
            capacity: "reserve_fuel".into(),
            rate: 5.0,
            direction: crate::umbilical::UmbilicalDirection::Deliver,
            min_power_level: 2,
        },
        crate::core::messages::PowerGroupId("umbilical".into()),
    );
    umbilical.running = running;
    // The carry and the level projections are live state the fold must
    // ignore: two otherwise-identical running umbilicals mid-flow fold the
    // same, whatever their sub-tick carry.
    umbilical.carry = if running { 0.37 } else { 0.0 };
    umbilical.operator_level = Some(42);
    world.spawn((EntityUuid(uuid.to_string()), umbilical));
}

/// **Issue #1160.** A world with no umbilical running folds to exactly the
/// number it did before the namespace existed — and so does a hull that
/// authored an umbilical and is not running. Only a RUNNING flow moves it, and
/// the carry/level projections never do, because they are re-derived every
/// tick and a resume forgives the sub-tick debt.
#[test]
fn only_a_running_umbilical_moves_the_digest_and_the_carry_does_not() {
    let id = "00000000-0000-8000-8000-000000000001";

    let mut idle = fold_world();
    spawn_umbilical(&mut idle, id, false);
    assert_eq!(
        fold_umbilical_namespace(&idle, FOLD_SEED),
        FOLD_SEED,
        "an umbilical running nothing folds nothing — a folded row here would have moved \
         every committed world's digest for state none of them carry"
    );

    let mut running = fold_world();
    spawn_umbilical(&mut running, id, true);
    assert_ne!(
        world_digest(&idle),
        world_digest(&running),
        "starting the flow must move the digest — whether it runs is what decides the next \
         tick's capacity move, folded nowhere else"
    );

    // Two hosts mid-flow with different sub-tick carries still agree: the
    // carry is a projection the fold ignores.
    let mut running_again = fold_world();
    spawn_umbilical(&mut running_again, id, true);
    if let Some(mut u) = running_again
        .query::<&mut TransferUmbilical>()
        .iter_mut(&mut running_again)
        .next()
    {
        u.carry = 0.91;
        u.operator_level = Some(7);
    }
    assert_eq!(
        world_digest(&running),
        world_digest(&running_again),
        "two hosts running the same flow with different carries must fold the same"
    );
}

/// **Issue #1025.** A world with no infrastructure must fold to exactly the
/// number it folded before the namespace existed.
///
/// This is what keeps every committed world digest where it is. The
/// namespace is a pure addition for worlds that have structures and a
/// literal no-op for worlds that do not, and the two claims are the same
/// claim: an empty walk and an absent feature are the same world state.
#[test]
fn the_infrastructure_namespace_is_a_no_op_for_a_world_that_has_none() {
    let mut world = fold_world();
    spawn_ship(&mut world, "00000000-0000-8000-8000-000000000001", 1.0);
    spawn_rock(&mut world, "a1b2c3d4-0000-4000-8000-000000000001", 2.0);
    assert_eq!(
        fold_infrastructure_namespace(&world, FOLD_SEED),
        FOLD_SEED,
        "an empty infrastructure walk must leave the accumulator untouched — a folded row \
         count here would have moved every world digest in the repository for state none \
         of those worlds carry"
    );
}

/// **Issue #1025.** Two structures that differ only in condition must fold
/// to different numbers, and two that agree must not.
#[test]
fn a_structures_condition_and_flags_move_the_digest() {
    let mut intact = fold_world();
    spawn_structure(&mut intact, "00000000-0000-8000-8000-000000000001", 100.0);
    let mut same = fold_world();
    spawn_structure(&mut same, "00000000-0000-8000-8000-000000000001", 100.0);
    let mut degraded = fold_world();
    spawn_structure(&mut degraded, "00000000-0000-8000-8000-000000000001", 10.0);

    assert_eq!(
        world_digest(&intact),
        world_digest(&same),
        "two hosts holding the same structure in the same condition must agree"
    );
    assert_ne!(
        world_digest(&intact),
        world_digest(&degraded),
        "…and a structure degraded past its threshold — a different condition AND a \
         different operational flag — must not fold to the number an intact one does"
    );
}

/// **Issue #1027.** A depot's capacity LEVEL is folded, so two hosts that
/// disagree about how much a transfer moved disagree about the digest.
#[test]
fn a_moved_capacity_moves_the_digest() {
    fn world_with(level: i64) -> World {
        let mut world = fold_world();
        let config = crate::infrastructure::InfrastructureConfig {
            capacities: vec![crate::infrastructure::CapacityConfig {
                label: None,
                id: "berths".to_string(),
                amount: level,
                ceiling: Some(40),
            }],
            ..Default::default()
        };
        world.spawn((
            EntityUuid("00000000-0000-8000-8000-000000000001".to_string()),
            InfrastructureCondition(InfrastructureState::from_config(&config)),
        ));
        world
    }
    assert_eq!(
        world_digest(&world_with(20)),
        world_digest(&world_with(20)),
        "two hosts holding the same depot at the same level must agree"
    );
    assert_ne!(
        world_digest(&world_with(20)),
        world_digest(&world_with(32)),
        "…and a host that thinks twelve more berths are free disagrees about whether the              transfer window can be met, which is the mission. Before #1027 a capacity could not              move and folding it would have been noise; now it can, and not folding it would be              a hole."
    );
}

/// **Issue #1025 / AC4.** Structure order must not reach the digest.
#[test]
fn structures_fold_in_uuid_order_whatever_order_they_spawned_in() {
    let mut forward = fold_world();
    spawn_structure(&mut forward, "00000000-0000-8000-8000-000000000001", 90.0);
    spawn_structure(&mut forward, "00000000-0000-8000-8000-000000000002", 20.0);
    let mut reverse = fold_world();
    spawn_structure(&mut reverse, "00000000-0000-8000-8000-000000000002", 20.0);
    spawn_structure(&mut reverse, "00000000-0000-8000-8000-000000000001", 90.0);
    assert_eq!(
        world_digest(&forward),
        world_digest(&reverse),
        "the fold is keyed on the minted id, not on archetype order"
    );
}

/// A civilian on `lane` at `leg`, optionally under an order.
fn spawn_civilian(
    world: &mut World,
    uuid: &str,
    leg: usize,
    order: Option<crate::civilian::CivilianOrder>,
) {
    let mut state = CivilianState::from_config(&crate::civilian::CivilianConfig {
        route: Some("depot_run".into()),
        ..Default::default()
    });
    state.observe_leg(leg, None, 0, 60.0);
    if let Some(order) = order {
        state.receive_order(
            order,
            &crate::civilian::ComplianceDisposition::default(),
            0,
            60.0,
        );
    }
    world.spawn((EntityUuid(uuid.to_string()), CivilianTraffic(state)));
}

/// **Issue #1028.** A world with no civilian traffic must fold to exactly
/// the number it folded before the namespace existed.
///
/// The same claim the infrastructure namespace makes, for the same reason:
/// an empty walk and an absent feature are the same world state, and a
/// folded row count here would have moved every committed world digest over
/// state none of those worlds carry.
#[test]
fn the_civilian_namespace_is_a_no_op_for_a_world_that_has_none() {
    let mut world = fold_world();
    spawn_ship(&mut world, "00000000-0000-8000-8000-000000000001", 1.0);
    spawn_rock(&mut world, "a1b2c3d4-0000-4000-8000-000000000001", 2.0);
    assert_eq!(
        fold_civilian_namespace(&world, FOLD_SEED),
        FOLD_SEED,
        "an empty civilian walk must leave the accumulator untouched"
    );
}

/// **Issue #1028.** A craft's lane position and its answer to an order both
/// move the digest; two hosts that agree about both must agree.
#[test]
fn a_civilians_leg_and_its_compliance_move_the_digest() {
    const ID: &str = "00000000-0000-8000-8000-000000000001";
    let mut on_leg_one = fold_world();
    spawn_civilian(&mut on_leg_one, ID, 1, None);
    let mut same = fold_world();
    spawn_civilian(&mut same, ID, 1, None);
    let mut on_leg_two = fold_world();
    spawn_civilian(&mut on_leg_two, ID, 2, None);
    let mut ordered = fold_world();
    spawn_civilian(
        &mut ordered,
        ID,
        1,
        Some(crate::civilian::CivilianOrder::Hold),
    );
    let mut ordered_elsewhere = fold_world();
    spawn_civilian(
        &mut ordered_elsewhere,
        ID,
        1,
        Some(crate::civilian::CivilianOrder::divert_to_anchor(
            "holding_point",
        )),
    );

    assert_eq!(
        world_digest(&on_leg_one),
        world_digest(&same),
        "two hosts holding the same craft on the same leg must agree"
    );
    assert_ne!(
        world_digest(&on_leg_one),
        world_digest(&on_leg_two),
        "…and a craft one leg further round its circuit must not fold to the number the \
         one behind it does"
    );
    assert_ne!(
        world_digest(&on_leg_one),
        world_digest(&ordered),
        "an order taken is authoritative state: a craft that has been told to hold is not \
         the same craft as one that has not"
    );
    assert_ne!(
        world_digest(&ordered),
        world_digest(&ordered_elsewhere),
        "…and neither is one sent somewhere else — the verb AND its destination are what \
         distinguish two orders"
    );
}

/// **Issue #1028.** Craft order must not reach the digest.
#[test]
fn civilians_fold_in_uuid_order_whatever_order_they_spawned_in() {
    let mut forward = fold_world();
    spawn_civilian(
        &mut forward,
        "00000000-0000-8000-8000-000000000001",
        0,
        None,
    );
    spawn_civilian(
        &mut forward,
        "00000000-0000-8000-8000-000000000002",
        2,
        None,
    );
    let mut reverse = fold_world();
    spawn_civilian(
        &mut reverse,
        "00000000-0000-8000-8000-000000000002",
        2,
        None,
    );
    spawn_civilian(
        &mut reverse,
        "00000000-0000-8000-8000-000000000001",
        0,
        None,
    );
    assert_eq!(
        world_digest(&forward),
        world_digest(&reverse),
        "the fold is keyed on the minted id, not on archetype order"
    );
}

/// **AC4.** The same entities spawned in two different orders must produce
/// the same digest.
///
/// Bevy query iteration is archetype order — stable within one process, not
/// across two instances that spawned entities in a different sequence. This
/// is the test that would fail if the fold ever walked a query straight
/// into the accumulator, and it is deliberately a *spawn-order* difference
/// rather than a component-value one: everything about the two worlds is
/// identical except the order the ECS happens to hold them in.
#[test]
fn the_fold_iterates_in_stable_world_id_order() {
    let mut forward = fold_world();
    spawn_ship(&mut forward, "ship-b", 2.0);
    spawn_ship(&mut forward, "ship-a", 1.0);
    spawn_rock(&mut forward, "rock-b", 20.0);
    spawn_rock(&mut forward, "rock-a", 10.0);

    let mut backward = fold_world();
    spawn_rock(&mut backward, "rock-a", 10.0);
    spawn_ship(&mut backward, "ship-a", 1.0);
    spawn_rock(&mut backward, "rock-b", 20.0);
    spawn_ship(&mut backward, "ship-b", 2.0);

    assert_eq!(
        world_digest(&forward),
        world_digest(&backward),
        "two worlds holding the same entities in a different spawn order \
         produced different digests — the fold is following ECS order \
         somewhere instead of world-id order"
    );
}

/// The other half of the same claim: the digest must still be *sensitive*.
/// A fold that returned a constant would pass the test above trivially.
#[test]
fn a_moved_ship_moves_the_digest() {
    let mut before = fold_world();
    spawn_ship(&mut before, "ship-a", 1.0);
    let mut after = fold_world();
    spawn_ship(&mut after, "ship-a", 1.000_000_1);
    assert_ne!(world_digest(&before), world_digest(&after));
}

/// Namespaces are folded in a declared sequence and never merged, so an id
/// that exists in both namespaces must not collapse into one fold position.
#[test]
fn the_same_id_in_two_namespaces_folds_twice() {
    let mut both = fold_world();
    spawn_ship(&mut both, "shared-id", 1.0);
    spawn_rock(&mut both, "shared-id", 1.0);

    let mut one = fold_world();
    spawn_ship(&mut one, "shared-id", 1.0);

    assert_ne!(world_digest(&both), world_digest(&one));
}

#[test]
fn a_zero_interval_never_samples() {
    let ledger = DigestLedger::new(0);
    assert!(!ledger.samples(0));
    assert!(!ledger.samples(120));
}

#[test]
fn the_same_tick_is_never_sampled_twice() {
    let mut ledger = DigestLedger::new(10);
    ledger.record(10, 1);
    ledger.record(10, 1);
    assert_eq!(ledger.checkpoints.len(), 1);
}

#[test]
fn a_divergence_names_the_window_it_happened_in() {
    let mut recorded = DigestLedger::new(10);
    let mut replayed = DigestLedger::new(10);
    for (tick, digest) in [(10, 1), (20, 2), (30, 3)] {
        recorded.record(tick, digest);
        replayed.record(tick, if tick == 30 { 99 } else { digest });
    }
    let found = recorded.first_divergence(&replayed).expect("diverged");
    assert_eq!(found.tick, 30);
    assert_eq!(found.after, Some(20));
    assert!(
        !found.at_end,
        "a sampled-tick mismatch is not the end-of-run shape"
    );
}

/// A bare end-state mismatch with every sampled tick agreeing is a
/// DIFFERENT claim from a sampled tick disagreeing, and must be reported
/// as one: `at_end` is true, and the rendered message says "agreed through
/// N; the final states differ" rather than "first disagree at tick N" —
/// the latter would name a tick that never actually disagreed, since every
/// checkpoint this ledger sampled matched.
#[test]
fn agreeing_ledgers_with_different_endings_still_locate_the_split() {
    let mut recorded = DigestLedger::new(10);
    let mut replayed = DigestLedger::new(10);
    recorded.record(10, 1);
    replayed.record(10, 1);
    recorded.final_digest = 7;
    replayed.final_digest = 8;
    let found = recorded.first_divergence(&replayed).expect("diverged");
    assert_eq!(found.after, Some(10));
    assert!(
        found.at_end,
        "every sampled checkpoint agreed; only the final digest differs"
    );
    let rendered = found.to_string();
    assert!(
        rendered.contains("every sampled tick agreed through 10"),
        "got {rendered:?}"
    );
    assert!(
        rendered.contains("the final states differ"),
        "got {rendered:?}"
    );
    assert!(
        !rendered.contains("first disagree at tick"),
        "the end-of-run shape must not claim a specific tick disagreed \
         when every sample it took actually agreed; got {rendered:?}"
    );
}

#[test]
fn identical_ledgers_do_not_diverge() {
    let mut ledger = DigestLedger::new(10);
    ledger.record(10, 1);
    ledger.final_digest = 5;
    assert_eq!(ledger.first_divergence(&ledger.clone()), None);
}
