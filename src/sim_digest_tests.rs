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

fn gm_grant(operator: &str, correlation: &str) -> crate::gm_action::GmActionGrant {
    let origin = crate::command_admission::HostSlot(1);
    crate::gm_action::GmActionGrant {
        from: origin,
        sequenced_by: origin,
        operator_id: operator.to_string(),
        correlation: crate::gm_action::GmActionId::new(correlation).unwrap(),
        recovery_generation: 0,
        apply_tick: 20,
        order: crate::gm_action::GmActionOrder::new(origin, 1),
        action: crate::gm_action::GmAction::SetSessionPaused { active: true },
    }
}

#[test]
fn only_the_applied_gm_action_frontier_moves_the_digest() {
    let mut baseline = fold_world();
    baseline.insert_resource(SimTick(10));
    baseline.insert_resource(crate::gm_action::SimulationPaused(false));
    baseline.insert_resource(crate::gm_action::GmActionJournal::default());
    let baseline_digest = world_digest(&baseline);

    let mut paused = fold_world();
    paused.insert_resource(SimTick(10));
    paused.insert_resource(crate::gm_action::SimulationPaused(true));
    paused.insert_resource(crate::gm_action::GmActionJournal::default());
    assert_ne!(world_digest(&paused), baseline_digest);

    let mut pending = fold_world();
    pending.insert_resource(SimTick(10));
    pending.insert_resource(crate::gm_action::SimulationPaused(false));
    let mut journal = crate::gm_action::GmActionJournal::default();
    journal.insert(gm_grant("gm-one", "future-pause")).unwrap();
    pending.insert_resource(journal);
    let pending_digest = world_digest(&pending);
    assert_eq!(
        pending_digest, baseline_digest,
        "receipt of a future owner commit is not current simulation state"
    );

    let mut different_attribution = fold_world();
    different_attribution.insert_resource(SimTick(10));
    different_attribution.insert_resource(crate::gm_action::SimulationPaused(false));
    let mut journal = crate::gm_action::GmActionJournal::default();
    journal.insert(gm_grant("gm-two", "future-pause")).unwrap();
    different_attribution.insert_resource(journal);
    assert_eq!(
        world_digest(&different_attribution),
        pending_digest,
        "unapplied attribution is retained for replay but not folded yet"
    );

    let mut exact_boundary = fold_world();
    exact_boundary.insert_resource(SimTick(20));
    exact_boundary.insert_resource(crate::gm_action::SimulationPaused(false));
    let mut journal = crate::gm_action::GmActionJournal::default();
    journal.insert(gm_grant("gm-one", "future-pause")).unwrap();
    exact_boundary.insert_resource(journal);
    let exact_pending_digest = world_digest(&exact_boundary);

    let mut boundary_control = fold_world();
    boundary_control.insert_resource(SimTick(20));
    boundary_control.insert_resource(crate::gm_action::SimulationPaused(false));
    boundary_control.insert_resource(crate::gm_action::GmActionJournal::default());
    assert_eq!(
        exact_pending_digest,
        world_digest(&boundary_control),
        "after FixedLast advances to tick 20, its grant still awaits PreUpdate"
    );

    let mut applied = fold_world();
    applied.insert_resource(SimTick(20));
    applied.insert_resource(crate::gm_action::SimulationPaused(true));
    let mut applied_journal = crate::gm_action::GmActionJournal::default();
    applied_journal
        .insert(gm_grant("gm-one", "future-pause"))
        .unwrap();
    applied_journal.restore_applied_frontier(1).unwrap();
    applied.insert_resource(applied_journal);
    let applied_digest = world_digest(&applied);
    assert_ne!(applied_digest, exact_pending_digest);

    let mut applied_by_other = fold_world();
    applied_by_other.insert_resource(SimTick(20));
    applied_by_other.insert_resource(crate::gm_action::SimulationPaused(true));
    let mut other_journal = crate::gm_action::GmActionJournal::default();
    other_journal
        .insert(gm_grant("gm-two", "future-pause"))
        .unwrap();
    other_journal.restore_applied_frontier(1).unwrap();
    applied_by_other.insert_resource(other_journal);
    assert_ne!(
        world_digest(&applied_by_other),
        applied_digest,
        "operator attribution becomes part of the fold once applied"
    );
}

#[test]
fn canonical_station_puppet_membership_is_folded_independent_of_arrival_order() {
    fn world_with(operators: &[&str]) -> World {
        let mut world = fold_world();
        world.insert_resource(SimTick(20));
        world.insert_resource(crate::gm_action::SimulationPaused(false));
        world.insert_resource(crate::gm_action::GmActionJournal::default());
        let target = crate::gm_puppet::StationPuppetTarget::new(
            crate::command_admission::log::ShipKey("ship-player-1".into()),
            crate::core::messages::StationId("captain".into()),
        );
        let mut puppets = crate::gm_puppet::StationPuppets::default();
        for operator in operators {
            puppets.set_operator(target.clone(), (*operator).to_string(), true);
        }
        world.insert_resource(puppets);
        world
    }

    let idle = world_with(&[]);
    let forward = world_with(&["gm-b", "gm-a"]);
    let reverse = world_with(&["gm-a", "gm-b"]);
    let one = world_with(&["gm-a"]);
    assert_ne!(world_digest(&idle), world_digest(&forward));
    assert_eq!(
        world_digest(&forward),
        world_digest(&reverse),
        "canonical operator order makes receipt order irrelevant"
    );
    assert_ne!(
        world_digest(&one),
        world_digest(&forward),
        "equal-GM attribution membership is authoritative"
    );
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

/// A tender, optionally holding one named team abroad on a named target.
fn spawn_tender(world: &mut World, uuid: &str, abroad: Option<(u8, &str)>) {
    let mut dispatch = crate::console::repair::ExternalRepairDispatch::new(
        crate::console::repair::ExternalRepairConfig {
            range: 600.0,
            repair_rate: 8.0,
        },
    );
    if let Some((team_idx, target)) = abroad {
        dispatch.claim(team_idx, Some(target.to_string()));
    }
    world.spawn((EntityUuid(uuid.to_string()), dispatch));
}

/// **Issues #1161, #1386.** A hull holding nobody abroad folds nothing at all —
/// the empty-walk affordance that lets a shipped hull gain
/// `[repair.external_dispatch]` without moving any committed world's digest —
/// and a live claim folds BOTH what it is working and WHICH team is doing it.
///
/// The team has to fold for the reason the target does. Two hosts that disagree
/// about which slot is abroad disagree about which team this hull has free for
/// its own damage-control sweep, so the next internal dispatch lands on a
/// different system, and that reaches `EntitySystemHull` and therefore the
/// digest one tick later — as a divergence nothing upstream could explain.
#[test]
fn a_field_repair_folds_its_target_and_the_team_working_it() {
    let id = "00000000-0000-8000-8000-000000000001";

    let mut idle = fold_world();
    spawn_tender(&mut idle, id, None);
    assert_eq!(
        fold_external_repair_namespace(&idle, FOLD_SEED),
        FOLD_SEED,
        "a hull dispatching nobody folds nothing — a folded row here would have moved every \
         committed world's digest for state none of them carry"
    );

    let mut team_zero = fold_world();
    spawn_tender(&mut team_zero, id, Some((0, "ally-A")));
    let mut team_zero_again = fold_world();
    spawn_tender(&mut team_zero_again, id, Some((0, "ally-A")));
    let mut team_one = fold_world();
    spawn_tender(&mut team_one, id, Some((1, "ally-A")));
    let mut other_ally = fold_world();
    spawn_tender(&mut other_ally, id, Some((0, "ally-B")));

    assert_ne!(
        world_digest(&idle),
        world_digest(&team_zero),
        "sending a team abroad must move the digest — nothing else records that the ally's \
         condition is climbing and this hull is a team short"
    );
    assert_eq!(
        world_digest(&team_zero),
        world_digest(&team_zero_again),
        "two hosts with the same team on the same target must agree"
    );
    assert_ne!(
        world_digest(&team_zero),
        world_digest(&team_one),
        "…and a host that thinks a DIFFERENT team went must fold to a different number \
         (issue #1386: the claim names its team, so the digest has to see it)"
    );
    assert_ne!(
        world_digest(&team_zero),
        world_digest(&other_ally),
        "…as must a host that thinks the team is working someone else"
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

// ── Scenario scope (issue #1086) ─────────────────────────────────────────────

use crate::comms::content::{
    ActiveDialogue, CommsDialogueNode, CommsResponse, OpenCommsRequest, ScriptedDialogue,
};
use crate::core::messages::{CommsMessage, CommsResponseView};
use crate::dossier::evidence::EvidenceProvenance;
use crate::world::commitments::{Commitment, CommitmentState};
use crate::world::config::{Trigger, TriggerCondition};
use crate::world::content::TriggerState;
use crate::world::deadlines::{DeadlineRecord, DeadlineState};
use crate::world::script::schedule::{PendingCallbacks, ScheduledCall, TickBudget};
use crate::world::server::WorldRuntime;
use crate::world::workforce::WorkforceRecord;

/// A script runtime carrying no compiled units — enough to hold the two queues
/// the fold walks.
///
/// Written out rather than built through `WorldScriptRuntime::from_compiled`,
/// which returns `None` for an empty set on purpose (a script-free world
/// inserts no resource at all, and that case is covered by
/// [`an_empty_scenario_folds_as_no_scenario_at_all`]).
fn empty_script_runtime() -> WorldScriptRuntime {
    WorldScriptRuntime {
        host: crate::world::script::engine::RuntimeHost::new(),
        asts: std::collections::BTreeMap::new(),
        ast_owners: std::collections::BTreeMap::new(),
        triggers: Vec::new(),
        budget: TickBudget::new(),
        budget_tick: 0,
        content_hash: 0,
        pending_callbacks: PendingCallbacks::new(),
        pending_comms_opens: Vec::new(),
        deadline_handlers: Vec::new(),
    }
}

/// [`fold_world`] plus every scenario resource the scope walks, all empty — the
/// shape a booted world has before its first flag is set.
fn scenario_world() -> World {
    let mut world = fold_world();
    world.insert_resource(WorldContentRuntime::default());
    world.insert_resource(WorldLayerMap::default());
    world.insert_resource(CommsRuntime::default());
    world.insert_resource(CommsInboxRes::default());
    world.insert_resource(empty_script_runtime());
    world
}

/// A single-shot `on_destroyed` trigger, in the shape `world::content`'s own
/// fixtures build.
fn trigger_state(name: &str) -> TriggerState {
    TriggerState {
        trigger: Trigger {
            condition: TriggerCondition::OnDestroyed {
                entity_name: name.into(),
            },
            when: None,
            id: None,
            repeat: false,
            cooldown_secs: None,
            gm_controls: None,
        },
        fired: false,
        origin_layer: None,
        seen_destroyed: std::collections::HashSet::new(),
        last_fired_elapsed: None,
    }
}

fn scheduled_call(fire_tick: u64, fn_name: &str) -> ScheduledCall {
    ScheduledCall {
        fire_tick,
        script_path: "probe#script.main".into(),
        fn_name: fn_name.into(),
        origin_layer: None,
    }
}

fn message(id: &str, body: &str) -> CommsMessage {
    CommsMessage::injected(
        id.into(),
        "sender-uuid".into(),
        "Skyway Control".into(),
        body.into(),
        std::collections::BTreeMap::new(),
        vec![CommsResponseView {
            text: "comms.response.acknowledge".into(),
            important: false,
            available: true,
        }],
        "thread-1".into(),
        true,
        false,
    )
}

fn dialogue(node_fn: &str) -> ActiveDialogue {
    ActiveDialogue {
        current_node: CommsDialogueNode {
            body: "comms.body.opening".into(),
            body_params: std::collections::BTreeMap::new(),
            responses: vec![CommsResponse {
                text: "comms.response.acknowledge".into(),
                important: false,
                ai: Default::default(),
            }],
        },
        thread_id: "thread-1".into(),
        script: ScriptedDialogue {
            script_path: "probe#script.comms".into(),
            origin_layer: None,
            node_fn: node_fn.into(),
            on_pick: vec!["on_acknowledge".into()],
        },
    }
}

fn open_request(root_fn: &str) -> OpenCommsRequest {
    OpenCommsRequest {
        from: "control".into(),
        root_fn: root_fn.into(),
        display_name: None,
        thread_id: None,
        priority: CommsPriority::Routine,
        urgent: false,
        script_path: "probe#script.comms".into(),
        origin_layer: None,
    }
}

/// The compatibility claim the whole widening rests on: a world that carries
/// every scenario resource but has *said nothing yet* folds to exactly the
/// number a world with no scenario resource at all does.
///
/// This is [`fold_infrastructure_namespace`]'s empty-walk affordance taken five
/// times over, and it is what leaves the committed cross-target ledger
/// (`tests/fixtures/cross-target-ledger.json`) untouched by this issue: the
/// probe builds its world from Rust literals and registers no scenario resource,
/// so a widening that folded a zero for each empty walk would have moved a
/// number that is verified in a browser.
#[test]
fn an_empty_scenario_folds_as_no_scenario_at_all() {
    assert_eq!(world_digest(&fold_world()), world_digest(&scenario_world()));
}

/// A flag is the scenario's memory, and the counter is folded rather than its
/// truth: `2` and `1` are different states to an `increment_flag` chain even
/// though both read as "set".
#[test]
fn a_world_flag_moves_the_digest_and_its_counter_is_what_moves_it() {
    let quiet = world_digest(&scenario_world());

    let mut world = scenario_world();
    world
        .resource_mut::<WorldContentRuntime>()
        .flags
        .set_flag("lyra_clear");
    let one = world_digest(&world);
    assert_ne!(quiet, one, "setting a flag must move the digest");

    world
        .resource_mut::<WorldContentRuntime>()
        .flags
        .set_flag_value("lyra_clear", 2);
    assert_ne!(
        one,
        world_digest(&world),
        "the counter is folded, not a bit"
    );

    // Clearing removes the entry, which is the store's whole vocabulary — so a
    // cleared flag and one never set are the same authoritative state.
    world
        .resource_mut::<WorldContentRuntime>()
        .flags
        .clear_flag("lyra_clear");
    assert_eq!(
        quiet,
        world_digest(&world),
        "a cleared flag folds as an unset one — the store removes the entry"
    );
}

/// A layer's flags fold under the layer's identity, and only while the layer is
/// ACTIVE: a failed-load sentinel occupies its path to suppress retries and is
/// not part of the composition a peer has to agree with.
#[test]
fn only_an_active_layer_folds_and_its_path_and_loader_fold_with_it() {
    let quiet = world_digest(&scenario_world());

    let mut world = scenario_world();
    let mut sentinel = WorldRuntime::default();
    sentinel.flags.set_flag("storm_warning");
    world
        .resource_mut::<WorldLayerMap>()
        .0
        .insert("layers/storm.toml".into(), sentinel);
    assert_eq!(
        quiet,
        world_digest(&world),
        "a layer that never activated folds nothing at all"
    );

    world
        .resource_mut::<WorldLayerMap>()
        .0
        .get_mut("layers/storm.toml")
        .expect("just inserted")
        .is_active = true;
    let active = world_digest(&world);
    assert_ne!(quiet, active, "an active layer's flags are folded");

    world
        .resource_mut::<WorldLayerMap>()
        .0
        .get_mut("layers/storm.toml")
        .expect("just inserted")
        .loader_path = Some("layers/act_two.toml".into());
    assert_ne!(
        active,
        world_digest(&world),
        "the loader is half the reconcile key and the whole of the `parent:` \
         chain, so it folds beside the path"
    );
}

/// The layer walk folds each layer's PLACE, never its runtime ordinal — and the
/// two claims are separable, so both are asserted.
///
/// `WorldRuntime::activation_order` is `max(active) + 1` at each load, so
/// unloading from the middle leaves the survivors GAPPED and nothing renumbers
/// them. `PhoenixSnapshot::layer_flags` stores only the vector position, and
/// `reconcile_world_layers` reloads a resumed composition from one — so a live
/// run with a gap and its own restore hold the same composition under different
/// ordinals. Folding the ordinal made that read as save corruption; folding the
/// enumerate index keeps the composition claim and drops the unreproducible one.
#[test]
fn a_layers_place_is_folded_but_its_absolute_ordinal_is_not() {
    let layer = |order: u64, flag: &str| {
        let mut runtime = WorldRuntime {
            is_active: true,
            activation_order: order,
            ..WorldRuntime::default()
        };
        runtime.flags.set_flag(flag);
        runtime
    };

    let sole = |order: u64| {
        let mut world = scenario_world();
        world
            .resource_mut::<WorldLayerMap>()
            .0
            .insert("layers/storm.toml".into(), layer(order, "storm_warning"));
        world_digest(&world)
    };
    assert_eq!(
        sole(1),
        sole(7),
        "the sole active layer's absolute ordinal is NOT folded: a restore \
         renumbers it from one, and folding it would make a gapped live run fail \
         its own restore's digest equality"
    );

    let pair = |storm_order: u64, relief_order: u64| {
        let mut world = scenario_world();
        {
            let mut layers = world.resource_mut::<WorldLayerMap>();
            layers
                .0
                .insert("layers/storm.toml".into(), layer(storm_order, "storm"));
            layers
                .0
                .insert("layers/relief.toml".into(), layer(relief_order, "relief"));
        }
        world_digest(&world)
    };
    assert_eq!(
        pair(1, 2),
        pair(4, 9),
        "and neither ordinal is folded when the ORDER they imply is unchanged"
    );
    assert_ne!(
        pair(1, 2),
        pair(2, 1),
        "but the order itself is the composition — a host that loaded the relief \
         layer before the storm resolves `parent:` differently and appends \
         scripted triggers in the other order"
    );
}

/// The latch, the accumulation, and the shape of the table itself.
#[test]
fn a_trigger_latch_moves_the_digest_and_so_does_the_tables_shape() {
    let mut world = scenario_world();
    world
        .resource_mut::<WorldContentRuntime>()
        .triggers
        .push(trigger_state("raider"));
    let armed = world_digest(&world);
    assert_ne!(
        world_digest(&scenario_world()),
        armed,
        "a live trigger table is folded"
    );

    let mut continuation = world.resource::<WorldContentRuntime>().triggers.capture();
    continuation[0].fired = true;
    world
        .resource_mut::<WorldContentRuntime>()
        .triggers
        .restore(&continuation)
        .unwrap();
    let fired = world_digest(&world);
    assert_ne!(
        armed, fired,
        "the single-shot latch is the point of the walk"
    );

    continuation[0].seen_destroyed.push("escort".into());
    world
        .resource_mut::<WorldContentRuntime>()
        .triggers
        .restore(&continuation)
        .unwrap();
    let seen = world_digest(&world);
    assert_ne!(fired, seen, "the OnAllDestroyed accumulation is folded");

    let mut owned_state = world.resource::<WorldContentRuntime>().triggers[0].clone();
    owned_state.origin_layer = Some("layers/storm.toml".into());
    world
        .resource_mut::<WorldContentRuntime>()
        .triggers
        .replace_declarative(vec![owned_state]);
    let owned = world_digest(&world);
    assert_ne!(seen, owned, "which layer owns the row is folded with it");

    world
        .resource_mut::<WorldContentRuntime>()
        .triggers
        .push(trigger_state("courier"));
    assert_ne!(
        owned,
        world_digest(&world),
        "a table that grew a row — a layer loaded on one host and not the other \
         — moves the digest before either new trigger fires"
    );
}

/// A table that was RESHAPED without changing size still moves the digest.
///
/// Positional keying with the row count as its only shape guard is exactly the
/// hazard `WorldContentRuntime::triggers.generation()` exists to name: a
/// layer unloaded from the middle and another loaded in its place leaves the
/// count and the `origin_layer` tags alone while every row past the removal now
/// names a different trigger. Each row's authored identity — its `id`, and its
/// condition's KIND where there is no id — is what closes it.
#[test]
fn a_reshaped_trigger_table_of_the_same_size_moves_the_digest() {
    let named = |id: &str| {
        let mut state = trigger_state("raider");
        state.trigger.id = Some(id.into());
        state
    };
    let table = |rows: Vec<TriggerState>| {
        let mut world = scenario_world();
        world
            .resource_mut::<WorldContentRuntime>()
            .triggers
            .replace_declarative(rows);
        world_digest(&world)
    };

    assert_ne!(
        table(vec![named("storm_opens"), named("storm_closes")]),
        table(vec![named("storm_opens"), named("storm_breaks")]),
        "same length, same latches, same origin layers — and still a different \
         set of triggers, which the authored id is what says"
    );

    let mut timer = trigger_state("raider");
    timer.trigger.condition = TriggerCondition::OnTimer { after_secs: 30.0 };
    assert_ne!(
        table(vec![trigger_state("raider")]),
        table(vec![timer]),
        "an anonymous row is keyed by the KIND of trigger occupying it, so a \
         swap is caught without an authored id to lean on"
    );
}

/// A GM-operable event's ARMED Fire is folded; its authored control set is not
/// (issue #1301).
///
/// The control set is authored config that `snapshot::content_digest` answers
/// for, exactly as a condition's fields are. What a RUN moves is which Fires
/// have crossed their apply boundary and are waiting for the pipeline — so two
/// peers that disagree about that disagree about what is about to happen, and
/// this is the fold that says so. It is deliberately absent while the set is
/// empty, which is why no existing world's digest moved when it landed.
#[test]
fn an_armed_gm_fire_moves_the_digest_and_an_empty_set_leaves_it_alone() {
    let gm_event = |id: &str| {
        let mut state = trigger_state("raider");
        state.trigger.condition = TriggerCondition::Manual;
        state.trigger.id = Some(id.into());
        state.trigger.gm_controls = Some(crate::world::config::GmEventControls::fire_only(
            id.into(),
            format!("world.gm.event.{id}"),
        ));
        state
    };

    let mut world = scenario_world();
    world.resource_mut::<WorldContentRuntime>().trigger_states =
        vec![gm_event("breach"), gm_event("sweep")];
    let idle = world_digest(&world);

    // An empty pending set folds nothing at all: a world that authors GM events
    // but has none armed digests exactly as it would have before this issue.
    let mut plain = scenario_world();
    plain.resource_mut::<WorldContentRuntime>().trigger_states =
        vec![gm_event("breach"), gm_event("sweep")];
    plain
        .resource_mut::<WorldContentRuntime>()
        .pending_gm_event_fires
        .clear();
    assert_eq!(world_digest(&plain), idle);

    world
        .resource_mut::<WorldContentRuntime>()
        .pending_gm_event_fires
        .insert("base-world::breach".into());
    let armed = world_digest(&world);
    assert_ne!(idle, armed, "an armed Fire is authoritative pending work");

    // WHICH event is armed is part of it: two peers holding one arm each on
    // different events are about to run different missions.
    let mut other = scenario_world();
    other.resource_mut::<WorldContentRuntime>().trigger_states =
        vec![gm_event("breach"), gm_event("sweep")];
    other
        .resource_mut::<WorldContentRuntime>()
        .pending_gm_event_fires
        .insert("base-world::sweep".into());
    assert_ne!(armed, world_digest(&other));

    // And the Manual condition is its own kind, so a manual row cannot be
    // mistaken for the automatic one it replaced in a reshaped table.
    let mut swapped = scenario_world();
    let mut automatic = gm_event("breach");
    automatic.trigger.condition = TriggerCondition::OnWorldLoaded;
    swapped.resource_mut::<WorldContentRuntime>().trigger_states =
        vec![automatic, gm_event("sweep")];
    assert_ne!(idle, world_digest(&swapped));

    // Issue #1302: an ORDINARY condition-bearing event declaring the same
    // control set folds on identical terms. The armed set is keyed by qualified
    // id and the fold never reads `Trigger::condition`, so two peers that
    // disagree about an automatic event's pending Fire disagree in the digest
    // exactly as they do about a manual one's — the guard against a future
    // change that folds only `TriggerCondition::Manual` arms.
    let mut ordinary = scenario_world();
    let mut evac = gm_event("evac");
    evac.trigger.condition = TriggerCondition::OnDestroyed {
        entity_name: "courier".into(),
    };
    ordinary
        .resource_mut::<WorldContentRuntime>()
        .trigger_states = vec![evac];
    let ordinary_idle = world_digest(&ordinary);
    ordinary
        .resource_mut::<WorldContentRuntime>()
        .pending_gm_event_fires
        .insert("base-world::evac".into());
    assert_ne!(
        ordinary_idle,
        world_digest(&ordinary),
        "an armed Fire on an AUTOMATIC event is authoritative pending work too"
    );
}

/// An armed GM PLACEMENT is folded; the authored palette it names is not
/// (issue #1305).
///
/// Same argument as the armed Fire above: the palette is authored content that
/// `snapshot::content_digest` answers for, and what a RUN moves is which
/// placements are owed. The queue's ORDER folds too, because it decides which
/// placement draws which `WorldIdMint` id — two peers holding the same two
/// placements the other way round are about to give each hull the other's
/// identity.
#[test]
fn an_armed_gm_placement_moves_the_digest_and_its_order_is_part_of_it() {
    let armed = |name: &str, x_mm: i64| crate::gm_spawn::PendingGmSpawn {
        palette: "raider".into(),
        variant: None,
        name: name.into(),
        position_mm: [x_mm, 0, 0],
        heading_mdeg: 0,
    };
    let palette = || {
        vec![crate::world::config::GmPaletteEntry {
            id: "raider".into(),
            label: "world.gm.palette.raider.label".into(),
            template_path: "assets/entities/ship_harrow_destroyer.toml".into(),
            ..Default::default()
        }]
    };

    // A world that authors a palette but has nothing armed — and, crucially,
    // authors no trigger at all — folds exactly what it folded before this
    // issue, which is nothing.
    let mut idle_world = scenario_world();
    idle_world.resource_mut::<WorldContentRuntime>().gm_palette = palette();
    let idle = world_digest(&idle_world);
    assert_eq!(idle, world_digest(&scenario_world()));

    let mut world = scenario_world();
    world.resource_mut::<WorldContentRuntime>().gm_palette = palette();
    world
        .resource_mut::<WorldContentRuntime>()
        .pending_gm_spawns = vec![armed("raider_1", 100_000)];
    let one = world_digest(&world);
    assert_ne!(
        idle, one,
        "an armed placement is authoritative pending work"
    );

    // WHERE it is placed is part of it.
    let mut moved = scenario_world();
    moved.resource_mut::<WorldContentRuntime>().gm_palette = palette();
    moved
        .resource_mut::<WorldContentRuntime>()
        .pending_gm_spawns = vec![armed("raider_1", 100_001)];
    assert_ne!(one, world_digest(&moved));

    // And so is the ORDER two placements sit in.
    let mut forward = scenario_world();
    forward.resource_mut::<WorldContentRuntime>().gm_palette = palette();
    forward
        .resource_mut::<WorldContentRuntime>()
        .pending_gm_spawns = vec![armed("raider_1", 100_000), armed("raider_2", 200_000)];
    let mut backward = scenario_world();
    backward.resource_mut::<WorldContentRuntime>().gm_palette = palette();
    backward
        .resource_mut::<WorldContentRuntime>()
        .pending_gm_spawns = vec![armed("raider_2", 200_000), armed("raider_1", 100_000)];
    assert_ne!(world_digest(&forward), world_digest(&backward));
}

/// The cooldown stamp folds as present-or-absent and never by value: a restore
/// reconstructs the mission-clock anchor by `f32` subtraction, so its readings
/// are not bit-exact across a resume. See `fold_scenario_triggers`.
#[test]
fn the_cooldown_stamp_folds_as_set_or_unset_but_not_as_a_number() {
    let mut world = scenario_world();
    world
        .resource_mut::<WorldContentRuntime>()
        .triggers
        .push(trigger_state("raider"));
    let unset = world_digest(&world);

    let mut continuation = world.resource::<WorldContentRuntime>().triggers.capture();
    continuation[0].last_fired_elapsed = Some(5.0);
    world
        .resource_mut::<WorldContentRuntime>()
        .triggers
        .restore(&continuation)
        .unwrap();
    let stamped = world_digest(&world);
    assert_ne!(
        unset, stamped,
        "whether a repeat trigger has ever fired is folded — a ResetTrigger \
         moves it"
    );

    continuation[0].last_fired_elapsed = Some(5.000_000_5);
    world
        .resource_mut::<WorldContentRuntime>()
        .triggers
        .restore(&continuation)
        .unwrap();
    assert_eq!(
        stamped,
        world_digest(&world),
        "the reading itself is NOT folded: a resumed world's anchor is rebuilt \
         by subtraction and lands an ULP away, which is not a divergence"
    );
}

/// The scripted callback queue, and its ORDER — which is what fires.
#[test]
fn a_scheduled_callback_moves_the_digest_and_so_does_its_place_in_the_queue() {
    let quiet = world_digest(&scenario_world());

    let mut world = scenario_world();
    world
        .resource_mut::<WorldScriptRuntime>()
        .pending_callbacks
        .push(scheduled_call(120, "on_survey"));
    let one = world_digest(&world);
    assert_ne!(quiet, one, "queued future work is authoritative and folded");

    world
        .resource_mut::<WorldScriptRuntime>()
        .pending_callbacks
        .push(scheduled_call(120, "on_admission"));
    let both = world_digest(&world);

    let mut swapped = scenario_world();
    {
        let mut script = swapped.resource_mut::<WorldScriptRuntime>();
        script
            .pending_callbacks
            .push(scheduled_call(120, "on_admission"));
        script
            .pending_callbacks
            .push(scheduled_call(120, "on_survey"));
    }
    assert_ne!(
        both,
        world_digest(&swapped),
        "two callbacks due on the same tick fire in queue order, so the queue's \
         order is folded rather than sorted away"
    );
}

/// A chained world event queued for the next tick's trigger pass.
#[test]
fn a_queued_world_event_moves_the_digest() {
    let quiet = world_digest(&scenario_world());

    let mut world = scenario_world();
    world
        .resource_mut::<WorldContentRuntime>()
        .pending_world_events
        .push(WorldEvent::FlagSet {
            name: "lyra_clear".into(),
            origin_layer: None,
        });
    let set = world_digest(&world);
    assert_ne!(quiet, set, "a queued event is folded");

    world
        .resource_mut::<WorldContentRuntime>()
        .pending_world_events[0] = WorldEvent::FlagCleared {
        name: "lyra_clear".into(),
        origin_layer: None,
    };
    assert_ne!(
        set,
        world_digest(&world),
        "and the variant is folded, not just the name it carries"
    );
}

/// Every named record: groups, deadlines, promises, findings, the dispute.
#[test]
fn each_named_scenario_record_moves_the_digest() {
    let mut world = scenario_world();
    let mut previous = world_digest(&world);

    let step = |world: &World, what: &str, previous: &mut u64| {
        let now = world_digest(world);
        assert_ne!(*previous, now, "{what} must move the digest");
        *previous = now;
    };

    world
        .resource_mut::<WorldContentRuntime>()
        .entity_groups
        .entry("escorts".into())
        .or_default()
        .insert("courier".into());
    step(&world, "an entity group", &mut previous);

    world
        .resource_mut::<WorldContentRuntime>()
        .deadlines
        .records
        .push(DeadlineRecord {
            id: "stabiliser_failure".into(),
            origin_layer: None,
            label: "world.deadline.stabiliser".into(),
            visible: true,
            due_tick: 900,
            state: DeadlineState::default(),
            armed: Some(scheduled_call(900, "on_stabiliser_failure")),
        });
    step(&world, "an armed deadline", &mut previous);

    world
        .resource_mut::<WorldContentRuntime>()
        .deadlines
        .records[0]
        .due_tick = 1_500;
    step(&world, "a slipped deadline", &mut previous);

    world
        .resource_mut::<WorldContentRuntime>()
        .commitments
        .records
        .push(Commitment {
            id: "evacuate_lyra".into(),
            made_to: "Skyway strike committee".into(),
            terms: "world.commitment.evacuate".into(),
            resolves_when: "world.commitment.evacuate.resolves".into(),
            state: CommitmentState::Open,
            made_at_tick: 300,
            resolved_at_tick: None,
        });
    step(&world, "a promise", &mut previous);

    world
        .resource_mut::<WorldContentRuntime>()
        .commitments
        .records[0]
        .state = CommitmentState::Kept;
    step(&world, "keeping a promise", &mut previous);

    world.resource_mut::<WorldContentRuntime>().evidence.append(
        "subject-uuid",
        "world.evidence.stress_fracture",
        EvidenceProvenance::Scan,
        420,
    );
    step(&world, "a gathered finding", &mut previous);

    world
        .resource_mut::<WorldContentRuntime>()
        .workforce
        .records
        .push(WorkforceRecord {
            id: "dockers".into(),
            label: "world.workforce.dockers".into(),
            on_strike: false,
            disposition: 0,
        });
    step(&world, "a declared workforce side", &mut previous);

    world
        .resource_mut::<WorldContentRuntime>()
        .workforce
        .records[0]
        .on_strike = true;
    step(&world, "a side walking out", &mut previous);

    world
        .resource_mut::<WorldContentRuntime>()
        .workforce
        .records[0]
        .disposition = -3;
    step(&world, "a side's disposition", &mut previous);
}

/// The `armed` latches are folded beside their rows: an empty-but-armed table
/// and an empty-and-unarmed one behave differently on the very next tick,
/// because the arming systems read exactly that bit.
#[test]
fn an_armed_but_empty_table_is_not_an_unarmed_one() {
    let unarmed = world_digest(&scenario_world());

    let mut world = scenario_world();
    world.resource_mut::<WorldContentRuntime>().deadlines.armed = true;
    let deadlines_armed = world_digest(&world);
    assert_ne!(unarmed, deadlines_armed, "the deadline table's armed latch");

    world.resource_mut::<WorldContentRuntime>().workforce.armed = true;
    assert_ne!(
        deadlines_armed,
        world_digest(&world),
        "the workforce register's armed latch"
    );
}

/// The inbox, and the two states an officer moves on it.
#[test]
fn an_inbox_message_moves_the_digest_and_so_does_answering_it() {
    let quiet = world_digest(&scenario_world());

    let mut world = scenario_world();
    world
        .resource_mut::<CommsInboxRes>()
        .0
        .inject(message("msg-1", "comms.body.opening"));
    let injected = world_digest(&world);
    assert_ne!(quiet, injected, "an unread message is folded");

    world
        .resource_mut::<CommsInboxRes>()
        .0
        .record_response("msg-1", 0);
    assert_ne!(
        injected,
        world_digest(&world),
        "the response the officer picked is what the scenario reads back"
    );
}

/// The inbox folds WHOLE: the stored `sender_in_range` and each response's
/// stored `available` move the digest like every other field of a message.
///
/// They were excluded at first on the premise that `update_comms_range_flags`
/// rewrote them every tick. It does not touch a `CommsMessage` at all — the
/// per-tick stamping happens on the CLONES `broadcast_comms_state` and
/// `publish_comms_blackboard` take — so the stored reading is written once, at
/// injection, and then carried for the life of the message, `CommsState::inbox`
/// included. It is also authoritative in its own right: the response router
/// refuses a reply whose message reads out of range.
#[test]
fn the_stored_range_reading_on_a_message_moves_the_digest() {
    let mut world = scenario_world();
    world
        .resource_mut::<CommsInboxRes>()
        .0
        .inject(message("msg-1", "comms.body.opening"));
    let in_range = world_digest(&world);

    let mut out_of_range = scenario_world();
    let mut drifted = message("msg-1", "comms.body.opening");
    drifted.sender_in_range = false;
    out_of_range
        .resource_mut::<CommsInboxRes>()
        .0
        .inject(drifted);
    let sender_moved = world_digest(&out_of_range);
    assert_ne!(
        in_range, sender_moved,
        "a message stored as out of range is a different authoritative state \
         from one stored as reachable — and the payload carries it verbatim, so \
         a restore reproduces it"
    );

    let mut unavailable = scenario_world();
    let mut greyed = message("msg-1", "comms.body.opening");
    greyed.sender_in_range = false;
    greyed.responses[0].available = false;
    unavailable.resource_mut::<CommsInboxRes>().0.inject(greyed);
    assert_ne!(
        sender_moved,
        world_digest(&unavailable),
        "and each response's own stored `available` folds beside its text"
    );
}

/// The `CommsRuntime` fields that genuinely ARE re-derived stay out of the fold,
/// so a host whose contact drifted out of range does not read as a divergence.
///
/// `contacts`, `range_flags` and `range_active` are rebuilt from scratch every
/// tick by `update_comms_range_flags`, from the live hailable entities and the
/// transforms the entity namespace already folds — which is the same call
/// `snapshot::CommsState` makes when it declines to carry them. `needs_broadcast`
/// and `last_broadcast_host` are broadcast bookkeeping: which peer was last sent
/// a `CommsState`, and whether one is owed. A restore sets the first
/// unconditionally and re-establishes the second on its next broadcast.
#[test]
fn the_re_derived_comms_fields_do_not_move_the_digest() {
    let mut world = scenario_world();
    world
        .resource_mut::<CommsRuntime>()
        .active_dialogues
        .insert("msg-1".into(), dialogue("node_opening"));
    let settled = world_digest(&world);

    {
        let mut comms = world.resource_mut::<CommsRuntime>();
        comms.contacts.push(crate::core::messages::CommsContact {
            uuid: "contact-uuid".into(),
            name: "Skyway Control".into(),
            in_range: false,
            is_urgent: true,
        });
        comms.range_flags.insert("contact-uuid".into(), false);
        comms.range_active = true;
        comms.needs_broadcast = true;
        comms.last_broadcast_host = Some("session-token".into());
    }

    assert_eq!(
        settled,
        world_digest(&world),
        "the roster, the range map, the range-active latch and the broadcast \
         bookkeeping are all re-derived or re-established, so none of them folds"
    );
}

/// A live dialogue, an open hail and a queued scripted open — the rest of
/// `CommsState`.
#[test]
fn every_other_comms_surface_moves_the_digest() {
    let mut world = scenario_world();
    let mut previous = world_digest(&world);

    world
        .resource_mut::<CommsRuntime>()
        .active_dialogues
        .insert("msg-1".into(), dialogue("node_opening"));
    let opened = world_digest(&world);
    assert_ne!(previous, opened, "a live dialogue is folded");
    previous = opened;

    world
        .resource_mut::<CommsRuntime>()
        .active_dialogues
        .get_mut("msg-1")
        .expect("just inserted")
        .script
        .node_fn = "node_second".into();
    let advanced = world_digest(&world);
    assert_ne!(
        previous, advanced,
        "which node a thread is showing is what it can be answered from"
    );
    previous = advanced;

    world
        .resource_mut::<CommsRuntime>()
        .open_hails
        .insert("contact-uuid".into());
    let hailed = world_digest(&world);
    assert_ne!(
        previous, hailed,
        "a hail this ship made and has not cleared"
    );
    previous = hailed;

    world
        .resource_mut::<WorldScriptRuntime>()
        .pending_comms_opens
        .push(open_request("node_foreman"));
    assert_ne!(
        previous,
        world_digest(&world),
        "a scripted open queued but not yet materialised mints an id when it \
         drains, so two hosts must agree the queue holds it"
    );
}

/// Issue #1343 — the two surfaces the delayed weighted picker adds.
///
/// The METADATA decides what an unmanned console may reach for; the WAIT decides
/// when it reaches. Two peers holding a live conversation who disagree about
/// either are already divergent, and this is the tick that says so rather than
/// the tick the answer lands on one of them and not the other.
#[test]
fn the_backfill_choice_metadata_and_its_running_wait_move_the_digest() {
    let mut world = scenario_world();
    world
        .resource_mut::<CommsRuntime>()
        .active_dialogues
        .insert("msg-1".into(), dialogue("node_opening"));
    let mut previous = world_digest(&world);

    let set_ai = |world: &mut World, ai: crate::comms::content::CommsResponseAi| {
        world
            .resource_mut::<CommsRuntime>()
            .active_dialogues
            .get_mut("msg-1")
            .expect("live")
            .current_node
            .responses[0]
            .ai = ai;
    };

    // A weight authored on a response the peers already agree exists.
    set_ai(
        &mut world,
        crate::comms::content::CommsResponseAi {
            weight: Some(1),
            delay_seconds: None,
        },
    );
    let weighted = world_digest(&world);
    assert_ne!(
        previous, weighted,
        "an authored weight changes which responses an unmanned console may pick"
    );
    previous = weighted;

    // ZERO is not ABSENT: a forbidden option and an unoffered one are different
    // instructions, so the present/absent marker has to fold.
    set_ai(
        &mut world,
        crate::comms::content::CommsResponseAi {
            weight: Some(0),
            delay_seconds: None,
        },
    );
    let forbidden = world_digest(&world);
    assert_ne!(previous, forbidden, "weight 0 must not fold as no weight");
    previous = forbidden;

    set_ai(
        &mut world,
        crate::comms::content::CommsResponseAi {
            weight: Some(0),
            delay_seconds: Some(5),
        },
    );
    let delayed = world_digest(&world);
    assert_ne!(previous, delayed, "the authored pause is folded too");
    previous = delayed;

    // And the running wait itself.
    world
        .resource_mut::<CommsRuntime>()
        .pending_ai_responses
        .insert(
            crate::comms::server::PendingAiResponseKey::new(
                crate::command_admission::HostSlot::SOLO,
                "msg-1",
            ),
            crate::comms::server::PendingAiResponse {
                due_tick: 300,
                response_fingerprint: 0xabcd,
            },
        );
    let armed = world_digest(&world);
    assert_ne!(previous, armed, "an armed wait is folded");

    world
        .resource_mut::<CommsRuntime>()
        .pending_ai_responses
        .get_mut(&crate::comms::server::PendingAiResponseKey::new(
            crate::command_admission::HostSlot::SOLO,
            "msg-1",
        ))
        .expect("armed")
        .due_tick = 301;
    let retimed = world_digest(&world);
    assert_ne!(
        armed, retimed,
        "two hosts that agree a wait is running but not WHEN it expires have \
         already diverged"
    );

    // …and WHICH hull is waiting. A fleet has one inbox but a Comms console per
    // hull, so the same message waited on by a different fleet slot is a
    // different mission state (issues #1343 + #1116).
    {
        let mut comms = world.resource_mut::<CommsRuntime>();
        let record = comms
            .pending_ai_responses
            .remove(&crate::comms::server::PendingAiResponseKey::new(
                crate::command_admission::HostSlot::SOLO,
                "msg-1",
            ))
            .expect("armed");
        comms.pending_ai_responses.insert(
            crate::comms::server::PendingAiResponseKey::new(
                crate::command_admission::HostSlot(2),
                "msg-1",
            ),
            record,
        );
    }
    assert_ne!(
        retimed,
        world_digest(&world),
        "the fleet slot holding the wait is folded too — two peers that swapped \
         which hull is waiting have diverged even though the message ids agree"
    );
}

/// The whole reason every `HashMap` walk in this scope sorts: two worlds that
/// reached the same scenario state by inserting it in a different order are the
/// same state, and must fold to the same number.
///
/// Without the sorts this passes or fails on the hasher's mood, which is the
/// failure mode a per-tick cross-peer comparison would surface as a phantom
/// divergence.
#[test]
fn insertion_order_does_not_reach_the_fold() {
    let names = ["alpha", "beta", "gamma", "delta", "epsilon", "zeta"];

    let build = |order: &[&str]| {
        let mut world = scenario_world();
        {
            let mut runtime = world.resource_mut::<WorldContentRuntime>();
            for name in order {
                // Keyed off the NAME, never the insertion index — the two runs
                // must reach the same state, not merely the same set of names.
                runtime
                    .flags
                    .set_flag_value(name, i64::from(name.as_bytes()[0]));
                runtime
                    .entity_groups
                    .entry((*name).into())
                    .or_default()
                    .insert(format!("{name}-member"));
            }
            let mut state = trigger_state("raider");
            for name in order {
                state.seen_destroyed.insert((*name).into());
            }
            runtime.triggers.push(state);
        }
        {
            let mut comms = world.resource_mut::<CommsRuntime>();
            for name in order {
                comms
                    .active_dialogues
                    .insert((*name).into(), dialogue(name));
                comms.open_hails.insert((*name).into());
            }
        }
        world_digest(&world)
    };

    let forwards: Vec<&str> = names.to_vec();
    let backwards: Vec<&str> = names.iter().rev().copied().collect();
    assert_eq!(
        build(&forwards),
        build(&backwards),
        "the same scenario state reached in a different insertion order must \
         fold to the same number"
    );
}

/// An armed GM direct effect is authoritative pending work, and an empty queue
/// folds nothing at all (issue #1310) — so no world that never uses the surface
/// moved its digest when this landed, and no cross-target ledger needed
/// re-blessing.
#[test]
fn an_armed_direct_effect_moves_the_digest_and_an_empty_queue_leaves_it_alone() {
    let effect =
        |sequence: u64, target: &str, amount: u32| crate::gm_effect::PendingGmDirectEffect {
            tick: 12,
            order: crate::gm_action::GmActionOrder::new(
                crate::command_admission::HostSlot(1),
                sequence,
            ),
            target: target.into(),
            scope: crate::gm_effect::GmDirectEffectScope::Entity,
            kind: crate::gm_effect::GmDirectEffectKind::Damage,
            amount_milli_hp: amount,
        };

    let mut world = fold_world();
    let bare = world_digest(&world);
    world.insert_resource(crate::gm_effect::PendingGmDirectEffects::default());
    assert_eq!(
        world_digest(&world),
        bare,
        "an empty queue and no queue are the same fact"
    );

    world
        .resource_mut::<crate::gm_effect::PendingGmDirectEffects>()
        .push(effect(1, "npc-1", 25_000));
    let armed = world_digest(&world);
    assert_ne!(
        bare, armed,
        "an armed effect is unapplied authoritative work"
    );

    // WHAT it will do is part of it: two peers holding different amounts, or
    // aiming at different hulls, are about to fold different worlds.
    let mut other = fold_world();
    other.insert_resource(crate::gm_effect::PendingGmDirectEffects::default());
    other
        .resource_mut::<crate::gm_effect::PendingGmDirectEffects>()
        .push(effect(1, "npc-1", 25_001));
    assert_ne!(armed, world_digest(&other));

    let mut elsewhere = fold_world();
    elsewhere.insert_resource(crate::gm_effect::PendingGmDirectEffects::default());
    elsewhere
        .resource_mut::<crate::gm_effect::PendingGmDirectEffects>()
        .push(effect(1, "npc-2", 25_000));
    assert_ne!(armed, world_digest(&elsewhere));
}
