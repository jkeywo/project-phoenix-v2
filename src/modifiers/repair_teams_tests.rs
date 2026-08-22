use super::*;

fn sid(s: &str) -> SystemId {
    SystemId(s.into())
}

fn hull_with_helm(max_hp: f32) -> SystemHull {
    SystemHull::from_config(&[(sid("helm"), max_hp)])
}

fn hull_full() -> SystemHull {
    hull_with_helm(25.0)
}

fn hull_damaged(current: f32) -> SystemHull {
    let mut h = SystemHull::from_config(&[(sid("helm"), 25.0)]);
    // Damage it down to `current` by applying the difference.
    let dmg = 25.0 - current;
    if dmg > 0.0 {
        let mut rng = crate::sim_rng::unseeded_test_rng();
        h.apply_damage(dmg, &mut rng);
    }
    h
}

// ── Default state ─────────────────────────────────────────────────────────

#[test]
fn new_teams_all_idle() {
    let teams = RepairTeams::new(3);
    assert_eq!(teams.slots().len(), 3);
    assert!(teams.slots().iter().all(|s| matches!(s, TeamSlot::Idle)));
}

#[test]
fn default_has_two_teams() {
    let teams = RepairTeams::default();
    assert_eq!(teams.slots().len(), 2);
    assert!(teams.slots().iter().all(|s| matches!(s, TeamSlot::Idle)));
}

// ── lowest_free_team ──────────────────────────────────────────────────────

#[test]
fn lowest_free_team_returns_zero_when_all_idle() {
    let teams = RepairTeams::new(2);
    assert_eq!(teams.lowest_free_team(), Some(0));
}

#[test]
fn lowest_free_team_skips_busy_teams() {
    let mut teams = RepairTeams::new(2);
    teams.dispatch(0, sid("helm"), "Helm".to_string());
    assert_eq!(teams.lowest_free_team(), Some(1));
}

#[test]
fn lowest_free_team_returns_none_when_all_busy() {
    let mut teams = RepairTeams::new(2);
    teams.dispatch(0, sid("helm"), "Helm".to_string());
    teams.dispatch(1, sid("tactical"), "Tactical".to_string());
    assert_eq!(teams.lowest_free_team(), None);
}

// ── dispatch ──────────────────────────────────────────────────────────────

#[test]
fn dispatch_idle_team_enters_travelling() {
    let mut teams = RepairTeams::new(2);
    teams.dispatch(0, sid("helm"), "Helm".to_string());
    let expected = Some(sid("helm"));
    assert!(matches!(
        &teams.slots()[0],
        TeamSlot::Travelling { system_id, elapsed, .. }
            if *system_id == expected && *elapsed == 0.0
    ));
}

#[test]
fn dispatch_non_idle_team_is_noop() {
    // Dispatching to the same system (recall) sets Returning with no queue.
    let mut teams = RepairTeams::new(2);
    teams.dispatch(0, sid("helm"), "Helm".to_string());
    // Recall (same system)
    teams.dispatch(0, sid("helm"), "Helm".to_string());
    assert!(matches!(
        &teams.slots()[0],
        TeamSlot::Returning {
            queued_system_id: None,
            ..
        }
    ));
}

// ── Travelling → Repairing ────────────────────────────────────────────────

#[test]
fn travelling_transitions_to_repairing_after_5s() {
    let mut teams = RepairTeams::new(1);
    let mut hull = hull_damaged(20.0); // not at max
    teams.dispatch(0, sid("helm"), "Helm".to_string());
    teams.tick(5.0, &mut hull, None);
    let expected = Some(sid("helm"));
    assert!(matches!(
        &teams.slots()[0],
        TeamSlot::Repairing { system_id, .. } if *system_id == expected
    ));
}

#[test]
fn travelling_does_not_transition_before_5s() {
    let mut teams = RepairTeams::new(1);
    let mut hull = hull_damaged(20.0);
    teams.dispatch(0, sid("helm"), "Helm".to_string());
    teams.tick(4.9, &mut hull, None);
    assert!(matches!(&teams.slots()[0], TeamSlot::Travelling { .. }));
}

#[test]
fn team_arrives_at_full_hp_console_enters_returning() {
    let mut teams = RepairTeams::new(1);
    let mut hull = hull_full(); // system already at full HP
    teams.dispatch(0, sid("helm"), "Helm".to_string());
    teams.tick(5.0, &mut hull, None);
    assert!(matches!(&teams.slots()[0], TeamSlot::Returning { .. }));
}

#[test]
fn repairing_restores_hp_at_correct_rate() {
    let mut teams = RepairTeams::new(1);
    let mut hull = hull_damaged(1.0); // 1 HP (Disabled, not Destroyed)
    teams.dispatch(0, sid("helm"), "Helm".to_string());
    teams.tick(5.0, &mut hull, None); // travel
                                      // Now repairing; restore for 2s should give 1 more HP (0.5 HP/s)
    teams.tick(2.0, &mut hull, None);
    let hp = hull.current_for(&sid("helm")).unwrap();
    assert!(
        (hp - 2.0).abs() < 1e-4,
        "expected 2 HP after 2s repair starting from 1 HP, got {hp}"
    );
}

/// With no station context (`config: None`) the "system reached max HP"
/// edge still ends the visit, exactly as it did before the #1013 sweep.
#[test]
fn repairing_transitions_to_returning_when_console_full() {
    let mut teams = RepairTeams::new(1);
    let mut hull = hull_damaged(24.9); // almost full
    teams.dispatch(0, sid("helm"), "Helm".to_string());
    teams.tick(5.0, &mut hull, None); // travel
                                      // One tick of 1s restores 0.5 HP — enough to max at 25
    teams.tick(1.0, &mut hull, None);
    assert!(matches!(&teams.slots()[0], TeamSlot::Returning { .. }));
}

// ── Returning → Idle ──────────────────────────────────────────────────────

#[test]
fn returning_transitions_to_idle_after_5s() {
    let mut teams = RepairTeams::new(1);
    let mut hull = hull_full();
    teams.dispatch(0, sid("helm"), "Helm".to_string());
    teams.tick(5.0, &mut hull, None); // travel (arrives full → Returning with remaining=0)
                                      // remaining is already 0 from arriving at full hp; tick 0.1 to trigger idle
    teams.tick(0.1, &mut hull, None);
    assert!(matches!(&teams.slots()[0], TeamSlot::Idle));
}

#[test]
fn returning_does_not_complete_before_remaining_expires() {
    let mut teams = RepairTeams::new(1);
    let mut hull = hull_damaged(24.9); // not full
    teams.dispatch(0, sid("helm"), "Helm".to_string());
    teams.tick(5.0, &mut hull, None); // travel → Repairing
    teams.tick(1.0, &mut hull, None); // repair → full → Returning { remaining: 5.0 }
    teams.tick(4.9, &mut hull, None); // remaining not yet expired
    assert!(matches!(&teams.slots()[0], TeamSlot::Returning { .. }));
}

// ── Full lifecycle ────────────────────────────────────────────────────────

/// The whole Idle → Travelling → Repairing → Returning → Idle walk, with no
/// station context and so no sweep. The `Some(config)` counterpart — a team
/// that keeps working instead of going home — is
/// `sweep_repairs_every_damaged_system_at_the_station_in_one_visit`.
#[test]
fn full_lifecycle_travel_repair_return_idle() {
    let mut teams = RepairTeams::new(1);
    let mut hull = hull_damaged(1.0); // 1 HP (Disabled, not Destroyed — repairable)
    teams.dispatch(0, sid("helm"), "Helm".to_string());

    // Travelling
    teams.tick(5.0, &mut hull, None);
    assert!(matches!(&teams.slots()[0], TeamSlot::Repairing { .. }));

    // Repairing until full (24 HP remaining at 0.5 HP/s = 48s)
    teams.tick(50.0, &mut hull, None);
    assert!(matches!(&teams.slots()[0], TeamSlot::Returning { .. }));

    // Returning (remaining starts at TRAVEL_DURATION = 5s)
    teams.tick(5.1, &mut hull, None);
    assert!(matches!(&teams.slots()[0], TeamSlot::Idle));
}

// ── Multiple teams independence ───────────────────────────────────────────

#[test]
fn two_teams_operate_independently() {
    let mut hull = SystemHull::from_config(&[(sid("helm"), 25.0), (sid("tactical"), 25.0)]);
    // Damage both systems
    let mut rng = crate::sim_rng::unseeded_test_rng();
    hull.apply_damage(10.0, &mut rng);
    hull.apply_damage(10.0, &mut rng);

    let mut teams = RepairTeams::new(2);
    teams.dispatch(0, sid("helm"), "Helm".to_string());
    teams.dispatch(1, sid("tactical"), "Tactical".to_string());

    // Both should be Travelling to correct sids
    let expected_helm = Some(sid("helm"));
    let expected_tac = Some(sid("tactical"));
    assert!(matches!(
        &teams.slots()[0],
        TeamSlot::Travelling { system_id, .. } if *system_id == expected_helm
    ));
    assert!(matches!(
        &teams.slots()[1],
        TeamSlot::Travelling { system_id, .. } if *system_id == expected_tac
    ));

    // After 5s both transition
    teams.tick(5.0, &mut hull, None);
    let s0 = &teams.slots()[0];
    let s1 = &teams.slots()[1];
    assert!(
        matches!(
            s0,
            TeamSlot::Repairing { system_id, .. } if *system_id == expected_helm
        ) || matches!(s0, TeamSlot::Returning { .. })
    );
    assert!(
        matches!(
            s1,
            TeamSlot::Repairing { system_id, .. } if *system_id == expected_tac
        ) || matches!(s1, TeamSlot::Returning { .. })
    );
}

#[test]
fn non_idle_team_cannot_be_redirected_while_travelling() {
    // Redirect while Travelling to a DIFFERENT system → Returning with queued
    let mut teams = RepairTeams::new(2);
    teams.dispatch(0, sid("helm"), "Helm".to_string());
    teams.dispatch(0, sid("tactical"), "Tactical".to_string()); // redirect to different system
    let expected = Some(sid("tactical"));
    assert!(matches!(
        &teams.slots()[0],
        TeamSlot::Returning { queued_system_id, .. } if *queued_system_id == expected
    ));
}

#[test]
fn team_after_returning_can_be_dispatched_again() {
    let mut teams = RepairTeams::new(1);
    let mut hull = hull_full();
    teams.dispatch(0, sid("helm"), "Helm".to_string());
    teams.tick(5.0, &mut hull, None); // travel → Returning (remaining=0, full HP)
    teams.tick(0.1, &mut hull, None); // → Idle
    assert!(matches!(&teams.slots()[0], TeamSlot::Idle));
    teams.dispatch(0, sid("helm"), "Helm".to_string());
    assert!(matches!(&teams.slots()[0], TeamSlot::Travelling { .. }));
}

// ── Redirect / Recall new behaviors ──────────────────────────────────────

#[test]
fn redirect_mid_travel_sets_remaining_equal_to_elapsed() {
    let mut teams = RepairTeams::new(1);
    let mut hull = hull_damaged(10.0);
    teams.dispatch(0, sid("helm"), "Helm".to_string());
    // Advance 2s into travel
    teams.tick(2.0, &mut hull, None);
    assert!(
        matches!(&teams.slots()[0], TeamSlot::Travelling { elapsed, .. } if (*elapsed - 2.0).abs() < 1e-4)
    );
    // Redirect to a different system
    teams.dispatch(0, sid("tactical"), "Tactical".to_string());
    // remaining should equal the elapsed (2.0)
    let expected = Some(sid("tactical"));
    assert!(matches!(
        &teams.slots()[0],
        TeamSlot::Returning {
            remaining,
            queued_system_id,
            ..
        } if (*remaining - 2.0).abs() < 1e-4 && *queued_system_id == expected
    ));
}

#[test]
fn recall_mid_travel_sets_returning_no_queue() {
    let mut teams = RepairTeams::new(1);
    let mut hull = hull_damaged(10.0);
    teams.dispatch(0, sid("helm"), "Helm".to_string());
    teams.tick(3.0, &mut hull, None);
    teams.dispatch(0, sid("helm"), "Helm".to_string()); // same system = recall
    assert!(matches!(
        &teams.slots()[0],
        TeamSlot::Returning {
            queued_system_id: None,
            ..
        }
    ));
}

#[test]
fn redirect_while_repairing_sets_returning_with_travel_duration() {
    let mut teams = RepairTeams::new(1);
    let mut hull = hull_damaged(1.0); // 1 HP (Disabled, not Destroyed — repairable)
    teams.dispatch(0, sid("helm"), "Helm".to_string());
    teams.tick(5.0, &mut hull, None); // travel → Repairing
    assert!(matches!(&teams.slots()[0], TeamSlot::Repairing { .. }));
    teams.dispatch(0, sid("tactical"), "Tactical".to_string());
    let expected = Some(sid("tactical"));
    assert!(matches!(
        &teams.slots()[0],
        TeamSlot::Returning {
            remaining,
            queued_system_id,
            ..
        } if (*remaining - 5.0).abs() < 1e-4 && *queued_system_id == expected
    ));
}

#[test]
fn recall_while_repairing_sets_returning_no_queue() {
    let mut teams = RepairTeams::new(1);
    let mut hull = hull_damaged(1.0); // 1 HP (Disabled, not Destroyed — repairable)
    teams.dispatch(0, sid("helm"), "Helm".to_string());
    teams.tick(5.0, &mut hull, None); // travel → Repairing
    teams.dispatch(0, sid("helm"), "Helm".to_string()); // recall
    assert!(matches!(
        &teams.slots()[0],
        TeamSlot::Returning {
            queued_system_id: None,
            ..
        }
    ));
}

#[test]
fn partial_hp_restored_before_recall_is_preserved() {
    let mut teams = RepairTeams::new(1);
    let mut hull = hull_damaged(1.0); // 1 HP (Disabled, not Destroyed — repairable)
    teams.dispatch(0, sid("helm"), "Helm".to_string());
    teams.tick(5.0, &mut hull, None); // travel → Repairing
    teams.tick(2.0, &mut hull, None); // restore 1 HP (0.5 HP/s * 2s = 1 HP → now 2 HP)
    let hp_before_recall = hull.current_for(&sid("helm")).unwrap();
    assert!(
        (hp_before_recall - 2.0).abs() < 1e-4,
        "expected 2 HP before recall, got {hp_before_recall}"
    );
    teams.dispatch(0, sid("helm"), "Helm".to_string()); // recall
                                                        // HP should not have changed
    let hp_after_recall = hull.current_for(&sid("helm")).unwrap();
    assert!((hp_after_recall - hp_before_recall).abs() < 1e-4);
}

#[test]
fn returning_with_queue_auto_dispatches_on_completion() {
    let mut teams = RepairTeams::new(1);
    let mut hull = hull_damaged(10.0);
    teams.dispatch(0, sid("helm"), "Helm".to_string());
    teams.tick(2.0, &mut hull, None); // elapsed=2
    teams.dispatch(0, sid("tactical"), "Tactical".to_string()); // redirect → Returning { remaining:2, queued:Tactical }
    teams.tick(2.1, &mut hull, None); // remaining expires → auto-dispatch to Tactical
    let expected = Some(sid("tactical"));
    assert!(matches!(
        &teams.slots()[0],
        TeamSlot::Travelling {
            system_id,
            elapsed,
            ..
        } if *system_id == expected && *elapsed < 1e-3
    ));
}

#[test]
fn returning_with_no_queue_becomes_idle_on_completion() {
    let mut teams = RepairTeams::new(1);
    let mut hull = hull_damaged(10.0);
    teams.dispatch(0, sid("helm"), "Helm".to_string());
    teams.tick(2.0, &mut hull, None);
    teams.dispatch(0, sid("helm"), "Helm".to_string()); // recall → Returning { remaining:2, queued:None }
    teams.tick(2.1, &mut hull, None); // expires → Idle
    assert!(matches!(&teams.slots()[0], TeamSlot::Idle));
}

#[test]
fn dispatching_team_0_does_not_affect_team_1() {
    let mut teams = RepairTeams::new(2);
    teams.dispatch(0, sid("helm"), "Helm".to_string());
    // team 1 remains Idle
    assert!(matches!(&teams.slots()[1], TeamSlot::Idle));
    // redirect team 0
    teams.dispatch(0, sid("tactical"), "Tactical".to_string());
    assert!(
        matches!(&teams.slots()[1], TeamSlot::Idle),
        "team 1 should be unaffected"
    );
}

// ── Destroyed latch tests ─────────────────────────────────────────────────

/// The direct inverse of the pre-#1013 rule: a repair team dispatched to a
/// Destroyed system (hp == 0) works on it like any other, and the first
/// restored HP un-latches the tier — `tier_for` tests `current == 0.0`
/// before it looks at any threshold, so there is nothing further to clear.
///
/// This is the "un-stuck" acceptance criterion: before #1013 a destroyed
/// system stayed destroyed forever, because the team bounced off it and
/// nothing else in the game restores HP.
#[test]
fn destroyed_system_is_repaired_and_unlatched_by_repair_tick() {
    let mut teams = RepairTeams::new(1);
    // Build a hull with helm at 0 HP (Destroyed).
    let mut hull = SystemHull::from_config(&[(sid("helm"), 25.0)]);
    let mut rng = crate::sim_rng::unseeded_test_rng();
    hull.apply_damage(1000.0, &mut rng); // wipe to 0
    assert_eq!(
        hull.tier_for(&sid("helm")),
        DamageTier::Destroyed,
        "precondition: helm must be Destroyed"
    );
    assert_eq!(hull.current_for(&sid("helm")), Some(0.0));

    teams.dispatch(0, sid("helm"), "Helm".to_string());
    teams.tick(5.0, &mut hull, None); // travel
    assert!(
        matches!(&teams.slots()[0], TeamSlot::Repairing { .. }),
        "a team must now go to work on a Destroyed system, got {:?}",
        teams.slots()[0]
    );

    teams.tick(2.0, &mut hull, None); // 0.5 HP/s * 2s = 1 HP
    let hp_after = hull.current_for(&sid("helm")).unwrap();
    assert!(
        (hp_after - 1.0).abs() < 1e-4,
        "the team must restore HP to a Destroyed system (got {hp_after})"
    );
    assert_eq!(
        hull.tier_for(&sid("helm")),
        DamageTier::Disabled,
        "any positive HP un-latches Destroyed"
    );
}

// ── The station sweep (issue #1013) ───────────────────────────────────────

/// A ship config built from `(system id, owning station)` pairs. `None`
/// leaves the system ownerless, which is the `core` bucket group.
/// Constructed struct-first rather than through TOML: these tests are about
/// station MEMBERSHIP and nothing else in `ShipConfig` matters to the sweep.
fn config_with(systems: &[(&str, Option<&str>)]) -> ShipConfig {
    use crate::ship::config::SystemInstanceConfig;
    ShipConfig {
        stations: vec![],
        systems: systems
            .iter()
            .map(|(id, station)| SystemInstanceConfig {
                id: sid(id),
                kind: "generic".into(),
                station: station.map(|s| StationId(s.into())),
                ai_only: station.is_none(),
                human_seeking: false,
                seek_order: Vec::new(),
                power_group: None,
                marker: None,
                config: None,
            })
            .collect(),
        power_groups: Default::default(),
        coordination_lag_secs: 2.0,
    }
}

/// A hull built from `(system id, max hp, current hp)` triples.
fn hull_at(entries: &[(&str, f32, f32)]) -> SystemHull {
    let mut hull = SystemHull::from_config(
        &entries
            .iter()
            .map(|(id, max, _)| (sid(id), *max))
            .collect::<Vec<_>>(),
    );
    for (id, _, current) in entries {
        hull.set_hp(&sid(id), *current);
    }
    hull
}

/// The three-system `helm` station every sweep test below works over, plus
/// one `tactical` system and one ownerless `core` entry that must never be
/// swept from `helm`. Tiers (defaults: <0.75 Damaged, <0.25 Disabled,
/// 0 Destroyed) are therefore, worst first:
/// `helm-c` Destroyed, `helm-b` Disabled, `helm-a` Damaged.
fn station_hull() -> SystemHull {
    hull_at(&[
        ("helm-a", 10.0, 7.0),     // Damaged (0.70)
        ("helm-b", 10.0, 2.0),     // Disabled (0.20)
        ("helm-c", 10.0, 0.0),     // Destroyed
        ("tactical-x", 10.0, 1.0), // Disabled, but a different station
        ("core", 10.0, 5.0),       // Damaged, ownerless bucket
    ])
}

fn station_config() -> ShipConfig {
    config_with(&[
        ("helm-a", Some("helm")),
        ("helm-b", Some("helm")),
        ("helm-c", Some("helm")),
        ("tactical-x", Some("tactical")),
        // `core` is deliberately absent: the shipped hulls declare it as a
        // `[[hull.system_hull]]` row with no `[[system]]` behind it.
    ])
}

/// The system team 0 is currently working on, if any.
fn repairing_at(teams: &RepairTeams) -> Option<String> {
    match &teams.slots()[0] {
        TeamSlot::Repairing {
            system_id: Some(s), ..
        } => Some(s.0.clone()),
        _ => None,
    }
}

/// Drive team 0 in small steps, recording each system it works on in order.
/// Stops the moment the team goes `Returning` (or runs out of steps), so the
/// returned flag distinguishes "swept everything then went home" from "was
/// still working when we gave up".
fn walk_sweep(
    teams: &mut RepairTeams,
    hull: &mut SystemHull,
    config: Option<&ShipConfig>,
    steps: usize,
) -> (Vec<String>, bool) {
    let mut visited: Vec<String> = vec![];
    let mut returned = false;
    for _ in 0..steps {
        teams.tick(0.5, hull, config);
        match &teams.slots()[0] {
            TeamSlot::Repairing {
                system_id: Some(s), ..
            } if visited.last() != Some(&s.0) => {
                visited.push(s.0.clone());
            }
            TeamSlot::Returning { .. } => {
                returned = true;
                break;
            }
            _ => {}
        }
    }
    (visited, returned)
}

/// AC1: one team, one station, three damaged systems — all three are
/// repaired in worst-first order, in one visit, with no trip home between
/// them. The team only heads back once the station is clean.
#[test]
fn sweep_repairs_every_damaged_system_at_the_station_in_one_visit() {
    let mut teams = RepairTeams::new(1);
    let mut hull = station_hull();
    let config = station_config();
    teams.dispatch(0, sid("helm-c"), "Helm C".to_string());

    let (visited, returned) = walk_sweep(&mut teams, &mut hull, Some(&config), 400);

    assert_eq!(
        visited,
        vec!["helm-c", "helm-b", "helm-a"],
        "the team must work the station worst-first without going home \
         between systems"
    );
    assert!(returned, "the team goes home once the station is clean");
    for id in ["helm-a", "helm-b", "helm-c"] {
        assert!(
            hull.is_at_max(&sid(id)),
            "{id} must be fully repaired by the sweep"
        );
    }
}

/// The sweep is bounded by the station: a damaged system another station
/// owns is not the sweeping team's business, and neither is the ownerless
/// `core` bucket.
#[test]
fn sweep_does_not_cross_into_another_station_or_the_core_bucket() {
    let mut teams = RepairTeams::new(1);
    let mut hull = station_hull();
    let config = station_config();
    teams.dispatch(0, sid("helm-c"), "Helm C".to_string());

    let (visited, returned) = walk_sweep(&mut teams, &mut hull, Some(&config), 400);

    assert!(returned);
    assert!(
        !visited.iter().any(|v| v == "tactical-x" || v == "core"),
        "a helm team must not sweep other stations' work, got {visited:?}"
    );
    assert_eq!(hull.current_for(&sid("tactical-x")), Some(1.0));
    assert_eq!(hull.current_for(&sid("core")), Some(5.0));
}

/// The ownerless bucket is a sweep group of its own: a team at `core`
/// (a hull row with no `[[system]]` behind it) sweeps on to other
/// station-less systems, and stops at the station boundary the same way.
#[test]
fn sweep_covers_the_ownerless_core_bucket_group() {
    let mut teams = RepairTeams::new(1);
    let mut hull = hull_at(&[
        ("core", 10.0, 5.0),       // Damaged, ownerless
        ("aux-sensor", 10.0, 0.0), // Destroyed, ownerless (`ai_only`)
        ("helm-a", 10.0, 2.0),     // Disabled, but owned by `helm`
    ]);
    let config = config_with(&[("aux-sensor", None), ("helm-a", Some("helm"))]);
    teams.dispatch(0, sid("core"), "Core".to_string());

    let (visited, returned) = walk_sweep(&mut teams, &mut hull, Some(&config), 400);

    assert_eq!(
        visited,
        vec!["core", "aux-sensor"],
        "an ownerless-bucket team sweeps the other ownerless systems only"
    );
    assert!(returned);
    assert_eq!(hull.current_for(&sid("helm-a")), Some(2.0));
}

/// A single-damaged-system station behaves exactly as it did before the
/// sweep existed: repair it, then go home.
#[test]
fn single_damaged_system_station_still_repairs_one_and_returns() {
    let mut teams = RepairTeams::new(1);
    let mut hull = hull_at(&[("helm-a", 10.0, 2.0), ("helm-b", 10.0, 10.0)]);
    let config = config_with(&[("helm-a", Some("helm")), ("helm-b", Some("helm"))]);
    teams.dispatch(0, sid("helm-a"), "Helm A".to_string());

    let (visited, returned) = walk_sweep(&mut teams, &mut hull, Some(&config), 200);

    assert_eq!(visited, vec!["helm-a"]);
    assert!(returned);
}

/// A system that is below max HP but still `Operational` is not swept to:
/// the sweep's damage predicate is the same `tier != Operational` the AI
/// dispatch prune uses, so the team never chases work the dispatcher would
/// not have sent it for.
#[test]
fn sweep_ignores_a_below_max_but_operational_system() {
    let mut teams = RepairTeams::new(1);
    // helm-b at 9/10 → ratio 0.9 → Operational despite the missing HP.
    let mut hull = hull_at(&[("helm-a", 10.0, 2.0), ("helm-b", 10.0, 9.0)]);
    let config = config_with(&[("helm-a", Some("helm")), ("helm-b", Some("helm"))]);
    teams.dispatch(0, sid("helm-a"), "Helm A".to_string());

    let (visited, returned) = walk_sweep(&mut teams, &mut hull, Some(&config), 200);

    assert_eq!(visited, vec!["helm-a"]);
    assert!(returned);
    assert_eq!(hull.current_for(&sid("helm-b")), Some(9.0));
}

/// Without a ship config there is no station membership to sweep over, so a
/// team falls back to the pre-#1013 behaviour: fix the one system it was
/// sent to and walk home, even with more damage sitting next to it.
#[test]
fn without_config_a_team_repairs_one_system_and_returns() {
    let mut teams = RepairTeams::new(1);
    let mut hull = station_hull();
    teams.dispatch(0, sid("helm-c"), "Helm C".to_string());

    let (visited, returned) = walk_sweep(&mut teams, &mut hull, None, 400);

    assert_eq!(visited, vec!["helm-c"]);
    assert!(returned);
    assert_eq!(
        hull.current_for(&sid("helm-b")),
        Some(2.0),
        "with no station context the team cannot know helm-b is its neighbour"
    );
}

/// A team that arrives to find its target already whole sweeps the station
/// rather than turning straight around — it is standing right there.
#[test]
fn arrival_at_a_whole_system_sweeps_the_station_instead_of_returning() {
    let mut teams = RepairTeams::new(1);
    let mut hull = hull_at(&[("helm-a", 10.0, 10.0), ("helm-b", 10.0, 2.0)]);
    let config = config_with(&[("helm-a", Some("helm")), ("helm-b", Some("helm"))]);
    teams.dispatch(0, sid("helm-a"), "Helm A".to_string());

    teams.tick(5.0, &mut hull, Some(&config));

    assert_eq!(
        repairing_at(&teams).as_deref(),
        Some("helm-b"),
        "arriving at a healthy system must hand off to the station's real \
         work, got {:?}",
        teams.slots()[0]
    );
}

/// With nothing else to do at the station, the arrival bounce is unchanged.
#[test]
fn arrival_at_a_whole_system_returns_when_the_station_is_clean() {
    let mut teams = RepairTeams::new(1);
    let mut hull = hull_at(&[("helm-a", 10.0, 10.0), ("helm-b", 10.0, 10.0)]);
    let config = config_with(&[("helm-a", Some("helm")), ("helm-b", Some("helm"))]);
    teams.dispatch(0, sid("helm-a"), "Helm A".to_string());

    teams.tick(5.0, &mut hull, Some(&config));

    assert!(matches!(&teams.slots()[0], TeamSlot::Returning { .. }));
}

/// The swept-to system carries the hull entry's display name, the same
/// source `handle_dispatch_repair_team` uses for a console-ordered dispatch.
///
/// The hull is built through `from_config_with_display_names` — the spawner's
/// own path — so the label and the raw SystemId are DIFFERENT strings.
/// `from_config` sets `display_name = sid.0`, which makes "labelled from the
/// hull entry" and "fell back to the raw id" indistinguishable and lets the
/// fallback pass a test written for the label.
#[test]
fn sweep_labels_the_next_system_from_its_hull_entry() {
    use crate::ship::damage::ConsoleTierConfig;
    let mut teams = RepairTeams::new(1);
    let mut hull = SystemHull::from_config_with_display_names(vec![
        (
            sid("helm-a"),
            "Helm Alpha".to_string(),
            10.0,
            ConsoleTierConfig::default(),
        ),
        (
            sid("helm-b"),
            "Helm Beta".to_string(),
            10.0,
            ConsoleTierConfig::default(),
        ),
    ]);
    hull.set_hp(&sid("helm-b"), 2.0);
    let config = config_with(&[("helm-a", Some("helm")), ("helm-b", Some("helm"))]);
    teams.dispatch(0, sid("helm-a"), "Helm Alpha".to_string());
    teams.tick(5.0, &mut hull, Some(&config));

    assert!(
        matches!(
            &teams.slots()[0],
            TeamSlot::Repairing { display_name: Some(d), .. } if d == "Helm Beta"
        ),
        "the swept-to slot must carry the hull entry's display name, not the \
         raw id `helm-b`; got {:?}",
        teams.slots()[0]
    );
}

/// A hull row that can never be progressed is not swept to, and — the point
/// of the guard — two of them do not trap the team forever.
///
/// A `max_hp = 0` row is `Destroyed` (`tier_for` tests `current == 0.0`
/// first) AND at max HP at the same time, so it satisfies the sweep's damage
/// predicate while `restore` can never change it. Before the `current < max`
/// guard the team handed off from one ghost row to the other and back,
/// forever: never finishing, never returning, and never releasing the slot
/// for the next dispatch.
#[test]
fn sweep_skips_zero_max_hp_rows_and_still_finishes_the_real_work() {
    let mut teams = RepairTeams::new(1);
    let mut hull = hull_at(&[
        ("helm-a", 10.0, 2.0),      // real work: Disabled, repairable
        ("helm-ghost-a", 0.0, 0.0), // permanently Destroyed AND at max
        ("helm-ghost-b", 0.0, 0.0), // ditto — the pair is what livelocked
    ]);
    assert_eq!(
        hull.tier_for(&sid("helm-ghost-a")),
        DamageTier::Destroyed,
        "fixture precondition: a zero-max row reads Destroyed"
    );
    assert!(
        hull.is_at_max(&sid("helm-ghost-a")),
        "fixture precondition: it is simultaneously at max HP"
    );
    let config = config_with(&[
        ("helm-a", Some("helm")),
        ("helm-ghost-a", Some("helm")),
        ("helm-ghost-b", Some("helm")),
    ]);
    teams.dispatch(0, sid("helm-a"), "Helm A".to_string());

    let (visited, returned) = walk_sweep(&mut teams, &mut hull, Some(&config), 400);

    assert_eq!(
        visited,
        vec!["helm-a"],
        "the sweep must only pick work it can progress, got {visited:?}"
    );
    assert!(
        returned,
        "the team must go home once the repairable work is done rather than \
         alternating between the two unfinishable rows forever"
    );
    assert!(hull.is_at_max(&sid("helm-a")));
}

/// A dispatch that resolved to a STATION NAME rather than a hull row bounces
/// exactly as it did before the sweep existed — it does not walk off into
/// the ownerless `core` bucket.
///
/// `resolve_repair_target` falls back to `SystemId(station_id)` when no
/// system of the station is repairable. That id is untracked, so `is_at_max`
/// answers the permissive `true` and the arrival lands on the sweep branch;
/// `sweep_group` then buckets an id the config does not describe as
/// OWNERLESS, which is `core`'s own group. Without the hull-row gate the team
/// sent to `helm` would arrive and start repairing `core`.
///
/// The COLLIDING case — a station name that is also a hull row, which would
/// pass this gate — is now impossible at the source rather than caught here:
/// `resolve_repair_target` produces the fallback only for a name the hull
/// does not track and returns `None` otherwise, so no such dispatch is ever
/// applied. This test pins the other half of that pair, the arrival that IS
/// untracked.
#[test]
fn arrival_at_a_station_name_returns_instead_of_sweeping_the_core_bucket() {
    let mut teams = RepairTeams::new(1);
    let mut hull = hull_at(&[("core", 10.0, 5.0), ("helm-a", 10.0, 2.0)]);
    let config = config_with(&[("helm-a", Some("helm"))]);
    assert!(
        hull.get(&sid("helm")).is_none(),
        "fixture precondition: `helm` is a station name, not a hull row"
    );
    teams.dispatch(0, sid("helm"), "Helm".to_string());

    let (visited, returned) = walk_sweep(&mut teams, &mut hull, Some(&config), 400);

    assert!(
        visited.is_empty(),
        "a team that arrived at a non-hull id must repair nothing, got {visited:?}"
    );
    assert!(returned, "it must bounce straight back to Returning");
    assert_eq!(
        hull.current_for(&sid("core")),
        Some(5.0),
        "the ownerless core bucket must be untouched — it is not this team's \
         station and the arrival id never belonged to a group at all"
    );
}

/// A destroyed system is swept to like any other, and comes out the far
/// side at full HP and Operational.
#[test]
fn sweep_repairs_a_destroyed_neighbour_back_to_operational() {
    let mut teams = RepairTeams::new(1);
    let mut hull = hull_at(&[("helm-a", 10.0, 2.0), ("helm-b", 10.0, 0.0)]);
    let config = config_with(&[("helm-a", Some("helm")), ("helm-b", Some("helm"))]);
    assert_eq!(hull.tier_for(&sid("helm-b")), DamageTier::Destroyed);
    teams.dispatch(0, sid("helm-a"), "Helm A".to_string());

    let (visited, returned) = walk_sweep(&mut teams, &mut hull, Some(&config), 400);

    assert_eq!(
        visited,
        vec!["helm-a", "helm-b"],
        "the team works the system it was actually sent to first — the \
         ranking only chooses among what is LEFT — and then sweeps on to \
         the Destroyed neighbour"
    );
    assert!(returned);
    assert!(hull.is_at_max(&sid("helm-b")));
    assert_eq!(hull.tier_for(&sid("helm-b")), DamageTier::Operational);
}

// ── Priority: the sweep's order selector (issue #1013) ────────────────────

/// Set up a `helm` station with four systems and put team 0 on site at
/// `helm-a`, so the remaining work ranks `helm-d` (Destroyed),
/// `helm-c` (Disabled), `helm-b` (Damaged) worst-first.
fn team_on_site_with_three_jobs_left() -> (RepairTeams, SystemHull, ShipConfig) {
    let mut teams = RepairTeams::new(1);
    let mut hull = hull_at(&[
        ("helm-a", 10.0, 7.0), // Damaged — the system the team is sent to
        ("helm-b", 10.0, 6.0), // Damaged, less hurt than helm-a
        ("helm-c", 10.0, 2.0), // Disabled
        ("helm-d", 10.0, 0.0), // Destroyed
    ]);
    let config = config_with(&[
        ("helm-a", Some("helm")),
        ("helm-b", Some("helm")),
        ("helm-c", Some("helm")),
        ("helm-d", Some("helm")),
    ]);
    teams.dispatch(0, sid("helm-a"), "Helm A".to_string());
    teams.tick(5.0, &mut hull, Some(&config));
    assert_eq!(
        repairing_at(&teams).as_deref(),
        Some("helm-a"),
        "fixture precondition: the team is on site at helm-a"
    );
    (teams, hull, config)
}

/// Finish the system team 0 is on (3 HP at most) and land on whatever the
/// sweep picks next.
fn finish_current_system(teams: &mut RepairTeams, hull: &mut SystemHull, config: &ShipConfig) {
    teams.tick(30.0, hull, Some(config));
}

/// No priority set → the sweep takes the worst remaining job.
#[test]
fn sweep_without_priority_takes_the_worst_remaining_job() {
    let (mut teams, mut hull, config) = team_on_site_with_three_jobs_left();
    finish_current_system(&mut teams, &mut hull, &config);
    assert_eq!(repairing_at(&teams).as_deref(), Some("helm-d"));
}

/// AC2: `priority` is READ by the sweep. The identical fixture, differing
/// only in the priority the console set, sends the team to a different
/// system — 1 to the worst, 2 to the second worst, 3 to the third.
#[test]
fn priority_selects_which_remaining_job_the_sweep_takes() {
    for (priority, expected) in [(1_u8, "helm-d"), (2, "helm-c"), (3, "helm-b")] {
        let (mut teams, mut hull, config) = team_on_site_with_three_jobs_left();
        assert!(teams.set_priority(0, priority));
        finish_current_system(&mut teams, &mut hull, &config);
        assert_eq!(
            repairing_at(&teams).as_deref(),
            Some(expected),
            "priority {priority} must select the {priority}-ranked remaining job"
        );
    }
}

/// `None` and `0` both mean "the worst job" — the ordinal is 1-based and
/// clamped at the bottom, so a console that sends 0 does something sane.
#[test]
fn priority_zero_means_the_worst_job_like_no_priority_at_all() {
    let (mut teams, mut hull, config) = team_on_site_with_three_jobs_left();
    assert!(teams.set_priority(0, 0));
    finish_current_system(&mut teams, &mut hull, &config);
    assert_eq!(repairing_at(&teams).as_deref(), Some("helm-d"));
}

/// A priority past the end of the remaining work clamps to the last job
/// rather than stranding the team.
#[test]
fn priority_beyond_the_remaining_jobs_clamps_to_the_last_one() {
    let (mut teams, mut hull, config) = team_on_site_with_three_jobs_left();
    assert!(teams.set_priority(0, 9));
    finish_current_system(&mut teams, &mut hull, &config);
    assert_eq!(repairing_at(&teams).as_deref(), Some("helm-b"));
}

/// AC2, the live version: changing the priority MID-SWEEP re-orders the work
/// the team has left. The first hand-off takes the worst job (no priority);
/// the console then taps priority 2, and the second hand-off takes the
/// second-worst of what remains instead of the worst.
#[test]
fn priority_change_mid_sweep_reorders_the_remaining_work() {
    let (mut teams, mut hull, config) = team_on_site_with_three_jobs_left();

    finish_current_system(&mut teams, &mut hull, &config);
    assert_eq!(
        repairing_at(&teams).as_deref(),
        Some("helm-d"),
        "first hand-off, no priority: the worst job"
    );

    assert!(teams.set_priority(0, 2));
    finish_current_system(&mut teams, &mut hull, &config);
    assert_eq!(
        repairing_at(&teams).as_deref(),
        Some("helm-b"),
        "with priority 2 the team must take the SECOND worst of the \
         remaining {{helm-c, helm-b}}, not helm-c"
    );
}

/// The priority is a standing instruction about the station, so it survives
/// the sweep's in-place system hand-off instead of resetting to `None`.
#[test]
fn priority_survives_the_sweep_hand_off() {
    let (mut teams, mut hull, config) = team_on_site_with_three_jobs_left();
    assert!(teams.set_priority(0, 2));
    finish_current_system(&mut teams, &mut hull, &config);
    assert!(
        matches!(
            &teams.slots()[0],
            TeamSlot::Repairing {
                priority: Some(2),
                ..
            }
        ),
        "got {:?}",
        teams.slots()[0]
    );
}

// ── Naming a system instead of an ordinal (issue #1015) ───────────────────
//
// The repair console's damaged-systems taps name a SYSTEM; the host pins
// that system on the team's slot and leaves the ordinal untouched. Every
// test here therefore asserts on the same observable the #1013 tests do
// — where the team actually goes at the hand-off — plus the pin the
// console highlights.

/// The headline: tapping the third-ranked job sends the team there next,
/// with no ordinal anywhere near the caller.
#[test]
fn prioritise_system_sends_the_sweep_to_the_named_system() {
    let (mut teams, mut hull, config) = team_on_site_with_three_jobs_left();

    assert_eq!(
        teams.prioritise_system(&sid("helm-b"), &hull, &config),
        Some(0),
        "team 0 is the one sweeping helm, so it takes the order"
    );
    finish_current_system(&mut teams, &mut hull, &config);

    assert_eq!(
        repairing_at(&teams).as_deref(),
        Some("helm-b"),
        "the tapped system must be the next job, not the worst one"
    );
}

/// A tap stores the SYSTEM and nothing else. It deliberately does NOT
/// resolve to an ordinal: `priority` is #1013's standing per-team
/// instruction, and a tap is a one-shot choice about one row, so the two
/// never write to the same place. (`the_pin_beats_a_stale_ordinal_after_a_re_rank`
/// below is why storing a rank here would be wrong and not merely redundant.)
#[test]
fn prioritise_system_stores_the_pin_and_never_an_ordinal() {
    for target in ["helm-d", "helm-c", "helm-b"] {
        let (mut teams, hull, config) = team_on_site_with_three_jobs_left();
        assert_eq!(
            teams.prioritise_system(&sid(target), &hull, &config),
            Some(0)
        );
        assert!(
            matches!(
                &teams.slots()[0],
                TeamSlot::Repairing {
                    priority: None,
                    priority_system_id: Some(pinned),
                    ..
                } if pinned.0 == target
            ),
            "{target} must be pinned with no ordinal written, got {:?}",
            teams.slots()[0]
        );
    }
}

/// A tap leaves an existing standing ordinal alone — the two levers are
/// independent, and the pin simply outranks the ordinal while it lasts.
#[test]
fn prioritise_system_does_not_disturb_a_standing_ordinal() {
    let (mut teams, hull, config) = team_on_site_with_three_jobs_left();
    assert!(teams.set_priority(0, 3));
    teams.prioritise_system(&sid("helm-c"), &hull, &config);
    assert!(
        matches!(
            &teams.slots()[0],
            TeamSlot::Repairing {
                priority: Some(3),
                priority_system_id: Some(pinned),
                ..
            } if pinned.0 == "helm-c"
        ),
        "got {:?}",
        teams.slots()[0]
    );
}

/// The console's highlight: the resolved slot echoes WHICH system the
/// host pinned, because the client cannot re-derive it (issue #737 hides
/// most of the candidates from it).
#[test]
fn prioritise_system_echoes_the_pinned_system_for_the_console() {
    let (mut teams, hull, config) = team_on_site_with_three_jobs_left();
    teams.prioritise_system(&sid("helm-c"), &hull, &config);
    assert!(
        matches!(
            &teams.slots()[0],
            TeamSlot::Repairing { priority_system_id: Some(s), .. } if s.0 == "helm-c"
        ),
        "got {:?}",
        teams.slots()[0]
    );
}

/// The pin describes one hand-off and is spent by it — otherwise the
/// console would keep highlighting a row the team has already arrived at.
/// The ORDINAL survives untouched, because #1013 makes that a standing
/// instruction about the station; here it is deliberately set to something
/// the pin overrules (3 would take `helm-b`), so the assertion that the team
/// landed on `helm-c` also proves which of the two levers won.
#[test]
fn the_priority_pin_clears_at_the_hand_off_but_the_ordinal_does_not() {
    let (mut teams, mut hull, config) = team_on_site_with_three_jobs_left();
    assert!(teams.set_priority(0, 3));
    teams.prioritise_system(&sid("helm-c"), &hull, &config);
    finish_current_system(&mut teams, &mut hull, &config);

    assert_eq!(repairing_at(&teams).as_deref(), Some("helm-c"));
    assert!(
        matches!(
            &teams.slots()[0],
            TeamSlot::Repairing {
                priority: Some(3),
                priority_system_id: None,
                ..
            }
        ),
        "got {:?}",
        teams.slots()[0]
    );
}

/// The finding this design exists for: a tap and the hand-off it steers are
/// separated by however long the current system takes to finish, and combat
/// damage in between RE-RANKS the group.
///
/// The standing ordinal is set to 3 first, so the pin and the ordinal
/// fallback disagree instead of coincidentally landing on the same row:
/// `helm-b` is tapped while it ranks third. It is then blown to Destroyed,
/// which makes it rank FIRST (tied with `helm-d` on tier and fraction,
/// winning on the id tiebreak) and pushes `helm-c` into third — exactly
/// what the stale ordinal 3 would now select. The pin sends the team to
/// `helm-b` instead, which is the row the player actually asked for, so
/// the assertion below discriminates the pin from the ordinal fallback
/// rather than passing either way.
#[test]
fn the_pin_beats_a_stale_ordinal_after_a_re_rank() {
    let (mut teams, mut hull, config) = team_on_site_with_three_jobs_left();
    assert!(teams.set_priority(0, 3));
    assert_eq!(
        teams.prioritise_system(&sid("helm-b"), &hull, &config),
        Some(0)
    );

    // Fresh damage between the tap and the hand-off.
    hull.set_hp(&sid("helm-b"), 0.0);
    assert_eq!(hull.tier_for(&sid("helm-b")), DamageTier::Destroyed);
    // What the stale ordinal 3 would now select, spelled out so the test
    // fails loudly if the fixture's ranking ever shifts under it.
    let ranked: Vec<String> = sweep_candidates(&sid("helm-a"), &hull, &config, &[])
        .into_iter()
        .map(|(s, _)| s.0)
        .collect();
    assert_eq!(ranked, vec!["helm-b", "helm-d", "helm-c"]);

    finish_current_system(&mut teams, &mut hull, &config);
    assert_eq!(
        repairing_at(&teams).as_deref(),
        Some("helm-b"),
        "the pinned row must win, not whatever now sits at its old rank"
    );
}

/// The pin's only failure mode: the row it names stops being candidate work
/// before the hand-off. Then — and only then — the standing ordinal decides,
/// rather than the team stranding itself waiting for a job that is gone.
#[test]
fn a_pin_that_leaves_the_candidate_list_falls_back_to_the_ordinal() {
    let (mut teams, mut hull, config) = team_on_site_with_three_jobs_left();
    assert!(teams.set_priority(0, 2));
    assert_eq!(
        teams.prioritise_system(&sid("helm-b"), &hull, &config),
        Some(0)
    );

    // Somebody else finished helm-b, so it is Operational and at max —
    // failing both halves of the candidate test.
    hull.set_hp(&sid("helm-b"), 10.0);

    finish_current_system(&mut teams, &mut hull, &config);
    assert_eq!(
        repairing_at(&teams).as_deref(),
        Some("helm-c"),
        "with the pin gone, ordinal 2 must pick the second of the remaining \
         {{helm-d, helm-c}}"
    );
}

/// A tap on a system ANOTHER team is standing on is refused outright, and
/// crucially does not fall through: before the on-site exclusion existed,
/// team 0's own system was never team 0's own candidate, so the search moved
/// on and pinned it on team 1 — pointing a second team at a row already
/// being worked while the rest of the station waited.
#[test]
fn prioritise_system_refuses_a_tap_on_a_system_another_team_is_on_site_at() {
    let mut teams = RepairTeams::new(2);
    let mut hull = hull_at(&[
        ("helm-a", 10.0, 7.0),
        ("helm-b", 10.0, 6.0),
        ("helm-c", 10.0, 2.0),
    ]);
    let config = config_with(&[
        ("helm-a", Some("helm")),
        ("helm-b", Some("helm")),
        ("helm-c", Some("helm")),
    ]);
    teams.dispatch(0, sid("helm-a"), "Helm A".to_string());
    teams.dispatch(1, sid("helm-b"), "Helm B".to_string());
    teams.tick(5.0, &mut hull, Some(&config));

    assert_eq!(
        teams.prioritise_system(&sid("helm-a"), &hull, &config),
        None,
        "team 0's own system is nobody's next job"
    );
    assert!(
        teams.slots().iter().all(|s| matches!(
            s,
            TeamSlot::Repairing {
                priority_system_id: None,
                ..
            }
        )),
        "no team may be pinned by a refused tap, got {:?}",
        teams.slots()
    );
}

/// The same exclusion protecting #1013's own hand-off: a team that finishes
/// its system does not walk onto the one its crewmate is standing on. With
/// only those two systems damaged it has nothing left and goes home.
#[test]
fn a_sweep_hand_off_does_not_converge_on_another_teams_system() {
    let mut teams = RepairTeams::new(2);
    let mut hull = hull_at(&[
        ("helm-a", 10.0, 9.5), // team 0: nearly done
        ("helm-b", 10.0, 2.0), // team 1: a long job
    ]);
    let config = config_with(&[("helm-a", Some("helm")), ("helm-b", Some("helm"))]);
    teams.dispatch(0, sid("helm-a"), "Helm A".to_string());
    teams.dispatch(1, sid("helm-b"), "Helm B".to_string());
    teams.tick(5.0, &mut hull, Some(&config));
    assert_eq!(repairing_at(&teams).as_deref(), Some("helm-a"));

    // Long enough for team 0 to finish helm-a, nowhere near long enough for
    // team 1 to finish helm-b.
    teams.tick(2.0, &mut hull, Some(&config));

    assert!(
        matches!(&teams.slots()[0], TeamSlot::Returning { .. }),
        "team 0 must head home rather than pile onto helm-b, got {:?}",
        teams.slots()[0]
    );
    assert!(
        matches!(
            &teams.slots()[1],
            TeamSlot::Repairing { system_id: Some(s), .. } if s.0 == "helm-b"
        ),
        "team 1 keeps its job, got {:?}",
        teams.slots()[1]
    );
}

/// A tap on the system the team is already working on is not a candidate —
/// `next_sweep_target` excludes the current system — so it changes nothing
/// rather than resolving to some neighbouring rank.
#[test]
fn prioritise_system_ignores_a_tap_on_the_system_under_repair() {
    let (mut teams, hull, config) = team_on_site_with_three_jobs_left();
    assert_eq!(
        teams.prioritise_system(&sid("helm-a"), &hull, &config),
        None
    );
    assert!(
        matches!(
            &teams.slots()[0],
            TeamSlot::Repairing {
                priority: None,
                priority_system_id: None,
                ..
            }
        ),
        "got {:?}",
        teams.slots()[0]
    );
}

/// A tap on another station's system finds no team standing in that group
/// and is refused. The sweep is station-bounded, so "prioritise it" has no
/// meaning until somebody is dispatched there.
#[test]
fn prioritise_system_refuses_a_system_outside_every_teams_sweep_group() {
    let mut teams = RepairTeams::new(1);
    let mut hull = station_hull();
    let config = station_config();
    teams.dispatch(0, sid("helm-b"), "Helm B".to_string());
    teams.tick(5.0, &mut hull, Some(&config));
    assert_eq!(repairing_at(&teams).as_deref(), Some("helm-b"));

    assert_eq!(
        teams.prioritise_system(&sid("tactical-x"), &hull, &config),
        None,
        "no team is sweeping `tactical`, so there is nothing to re-order"
    );
    assert_eq!(
        teams.prioritise_system(&sid("core"), &hull, &config),
        None,
        "the ownerless bucket is its own group, and nobody is in it"
    );
}

/// A team that has not arrived yet has no sweep to steer: `Travelling`
/// carries a `priority` field but no candidate list, exactly as
/// `set_priority` already refuses it.
#[test]
fn prioritise_system_refuses_a_team_that_is_still_travelling() {
    let mut teams = RepairTeams::new(1);
    let hull = station_hull();
    let config = station_config();
    teams.dispatch(0, sid("helm-c"), "Helm C".to_string());

    assert_eq!(
        teams.prioritise_system(&sid("helm-a"), &hull, &config),
        None
    );
}

/// Two teams in the same group: the order goes to the lowest slot index, so
/// the outcome is a pure function of state rather than of iteration luck.
#[test]
fn prioritise_system_gives_a_shared_group_to_the_lowest_team_index() {
    let mut teams = RepairTeams::new(2);
    let mut hull = hull_at(&[
        ("helm-a", 10.0, 7.0),
        ("helm-b", 10.0, 6.0),
        ("helm-c", 10.0, 2.0),
    ]);
    let config = config_with(&[
        ("helm-a", Some("helm")),
        ("helm-b", Some("helm")),
        ("helm-c", Some("helm")),
    ]);
    teams.dispatch(0, sid("helm-a"), "Helm A".to_string());
    teams.dispatch(1, sid("helm-b"), "Helm B".to_string());
    teams.tick(5.0, &mut hull, Some(&config));

    assert_eq!(
        teams.prioritise_system(&sid("helm-c"), &hull, &config),
        Some(0)
    );
    assert!(
        matches!(
            &teams.slots()[1],
            TeamSlot::Repairing {
                priority_system_id: None,
                ..
            }
        ),
        "team 1 must be untouched, got {:?}",
        teams.slots()[1]
    );
}

/// The ownerless bucket is steerable too — that is the "fix the hull first"
/// case the playtest could not express, and core rows are the ones
/// Engineering can always see.
#[test]
fn prioritise_system_works_inside_the_ownerless_core_bucket() {
    let mut teams = RepairTeams::new(1);
    let mut hull = hull_at(&[
        ("core", 10.0, 5.0),         // Damaged — where the team is
        ("aux-sensor", 10.0, 0.0),   // Destroyed — ranks first
        ("hull-plating", 10.0, 4.0), // Damaged — ranks second
    ]);
    let config = config_with(&[("aux-sensor", None), ("hull-plating", None)]);
    teams.dispatch(0, sid("core"), "Core".to_string());
    teams.tick(5.0, &mut hull, Some(&config));
    assert_eq!(repairing_at(&teams).as_deref(), Some("core"));

    assert_eq!(
        teams.prioritise_system(&sid("hull-plating"), &hull, &config),
        Some(0)
    );
    finish_current_system(&mut teams, &mut hull, &config);
    assert_eq!(
        repairing_at(&teams).as_deref(),
        Some("hull-plating"),
        "the tapped core row must beat the Destroyed one that outranks it"
    );
}

/// The tier key dominates the damage-fraction key. `helm-disabled` and
/// `helm-damaged` are a rounding error apart in HP — 2.4 and 2.6 of 10 — but
/// they land either side of the 0.25 Disabled threshold, and the worse tier
/// wins even though the Damaged one is barely less hurt.
#[test]
fn sweep_prefers_a_worse_tier_over_a_larger_damage_fraction() {
    let config = config_with(&[
        ("helm-here", Some("helm")),
        ("helm-disabled", Some("helm")),
        ("helm-damaged", Some("helm")),
    ]);
    let hull = hull_at(&[
        ("helm-here", 10.0, 5.0),
        ("helm-disabled", 10.0, 2.4), // 0.24 → Disabled, fraction 0.76
        ("helm-damaged", 10.0, 2.6),  // 0.26 → Damaged, fraction 0.74
    ]);
    assert_eq!(hull.tier_for(&sid("helm-disabled")), DamageTier::Disabled);
    assert_eq!(hull.tier_for(&sid("helm-damaged")), DamageTier::Damaged);

    let (winner, _) =
        next_sweep_target(&sid("helm-here"), None, None, &hull, &config, &[]).unwrap();
    assert_eq!(winner.0, "helm-disabled");
}

/// Within one tier, the larger damage fraction goes first.
#[test]
fn sweep_prefers_the_larger_damage_fraction_within_a_tier() {
    let config = config_with(&[
        ("helm-here", Some("helm")),
        ("helm-worse", Some("helm")),
        ("helm-better", Some("helm")),
    ]);
    let hull = hull_at(&[
        ("helm-here", 10.0, 5.0),
        ("helm-worse", 10.0, 3.0),  // fraction 0.70
        ("helm-better", 10.0, 7.0), // fraction 0.30
    ]);
    assert_eq!(hull.tier_for(&sid("helm-worse")), DamageTier::Damaged);
    assert_eq!(hull.tier_for(&sid("helm-better")), DamageTier::Damaged);

    let (winner, _) =
        next_sweep_target(&sid("helm-here"), None, None, &hull, &config, &[]).unwrap();
    assert_eq!(winner.0, "helm-worse");
}

/// A full tie resolves to the smallest system id, so hull iteration order
/// cannot reach the decision — a repair choice feeds the sim digest like
/// any other. `helm-tie-b` is declared FIRST in both the hull and the
/// config, so an order-sensitive comparator would pick it.
#[test]
fn sweep_breaks_a_full_tie_on_the_smallest_system_id() {
    let config = config_with(&[
        ("helm-here", Some("helm")),
        ("helm-tie-b", Some("helm")),
        ("helm-tie-a", Some("helm")),
    ]);
    let hull = hull_at(&[
        ("helm-here", 10.0, 5.0),
        ("helm-tie-b", 10.0, 5.0),
        ("helm-tie-a", 10.0, 5.0),
    ]);

    let (first, _) = next_sweep_target(&sid("helm-here"), None, None, &hull, &config, &[]).unwrap();
    assert_eq!(first.0, "helm-tie-a");
    let (second, _) =
        next_sweep_target(&sid("helm-here"), Some(2), None, &hull, &config, &[]).unwrap();
    assert_eq!(second.0, "helm-tie-b");
}

/// Nothing left at the station → no sweep target, which is what puts the
/// team on the road home.
#[test]
fn next_sweep_target_is_none_when_the_station_is_clean() {
    let config = config_with(&[("helm-here", Some("helm")), ("helm-other", Some("helm"))]);
    let hull = hull_at(&[("helm-here", 10.0, 5.0), ("helm-other", 10.0, 10.0)]);
    assert!(next_sweep_target(&sid("helm-here"), None, None, &hull, &config, &[]).is_none());
}

// ── Display-name propagation (regression for reviewer's #617 finding) ──

/// Dispatch must record the caller-supplied `display_name` on the
/// resulting `TeamSlot::Travelling`. Regression for the reviewer's
/// finding on issue #617 that dispatch was regressing display_name to
/// the raw SystemId string ("helm-engine-port") instead of the
/// human-readable label ("Engine (Port)") that the pre-#617
/// `derive_system_fields(&Console)` helper produced.
#[test]
fn dispatch_records_supplied_display_name_on_travelling_slot() {
    let mut teams = RepairTeams::new(2);
    teams.dispatch(0, sid("helm"), "Helm".to_string());
    let slot = &teams.slots()[0];
    assert!(
        matches!(
            slot,
            TeamSlot::Travelling { display_name: Some(d), .. }
                if d == "Helm"
        ),
        "team 0 must be Travelling with display_name = Some(\"Helm\"), got {slot:?}"
    );
}

// ── set_priority ─────────────────────────────────────────────────────────

#[test]
fn set_priority_repairing_team_sets_priority() {
    let mut teams = RepairTeams::new(2);
    let mut hull = hull_damaged(10.0);
    teams.dispatch(0, sid("helm"), "Helm".to_string());
    teams.tick(5.0, &mut hull, None); // travel → Repairing
    assert!(teams.set_priority(0, 3));
    let slot = &teams.slots()[0];
    assert!(matches!(
        slot,
        TeamSlot::Repairing {
            priority: Some(3),
            ..
        }
    ));
}

#[test]
fn set_priority_idle_team_returns_false() {
    let mut teams = RepairTeams::new(2);
    assert!(!teams.set_priority(0, 3));
}

#[test]
fn set_priority_out_of_range_index_returns_false() {
    let mut teams = RepairTeams::new(1);
    assert!(!teams.set_priority(5, 3));
}

#[test]
fn set_priority_travelling_team_returns_false() {
    let mut teams = RepairTeams::new(2);
    teams.dispatch(0, sid("helm"), "Helm".to_string());
    assert!(!teams.set_priority(0, 3));
}

#[test]
fn set_priority_returning_team_returns_false() {
    let mut teams = RepairTeams::new(1);
    let mut hull = hull_full();
    teams.dispatch(0, sid("helm"), "Helm".to_string());
    teams.tick(5.0, &mut hull, None);
    // After arriving at full hull the team becomes Returning.
    assert!(!teams.set_priority(0, 3));
}

/// The caller-supplied display name must survive the
/// `Travelling → Repairing` transition (regression guard for the
/// clobber inside `tick()`).
#[test]
fn tick_preserves_display_name_through_travelling_to_repairing() {
    let mut teams = RepairTeams::new(1);
    let mut hull = hull_damaged(10.0);
    teams.dispatch(0, sid("helm"), "Helm".to_string());
    teams.tick(5.0, &mut hull, None); // travel → Repairing
    let slot = &teams.slots()[0];
    assert!(
        matches!(
            slot,
            TeamSlot::Repairing { display_name: Some(d), .. }
                if d == "Helm"
        ),
        "team 0 must be Repairing with display_name preserved as \
         Some(\"Helm\"), got {slot:?}"
    );
}

// ── Issue #1027: teams committed to an external field-repair ──

#[test]
fn a_commitment_holds_back_teams_from_the_top_of_the_idle_list() {
    let teams = RepairTeams::new(4);
    assert_eq!(
        teams.free_team_indices(0),
        vec![0, 1, 2, 3],
        "a ship committing nothing behaves exactly as it did before commitments existed"
    );
    assert_eq!(
        teams.free_team_indices(2),
        vec![0, 1],
        "a commitment eats from the TOP, so what remains is still the lowest-numbered teams              and the AI's deterministic visit order is untouched"
    );
    assert!(
        teams.free_team_indices(4).is_empty(),
        "a ship that has committed every team has none left for its own damage — that is the              capacity-as-cost trade, not a bug"
    );
    assert!(
        teams.free_team_indices(9).is_empty(),
        "…and over-committing saturates rather than underflowing"
    );
}

#[test]
fn a_commitment_only_ever_holds_back_idle_teams() {
    let mut teams = RepairTeams::new(3);
    teams.dispatch(0, sid("helm"), "Helm".into());
    assert_eq!(
        teams.free_team_indices(1),
        vec![1],
        "team 0 is out on an internal job and was never part of the commitment, so the              commitment comes out of the two that are still idle"
    );
    assert!(
        !teams.is_committed_to_operation(0, 1),
        "a team already travelling is not held by the operation — recalling or redirecting it              stays the console's business"
    );
    assert!(!teams.is_committed_to_operation(1, 1));
    assert!(
        teams.is_committed_to_operation(2, 1),
        "the highest-numbered idle team is the one spoken for"
    );
}

#[test]
fn a_committed_team_is_still_idle_in_every_readout() {
    // The teams never leave the hull. Nothing is dispatched, nothing
    // travels, and the console goes on showing three idle teams — they are
    // simply not available to be sent anywhere.
    let teams = RepairTeams::new(3);
    assert!(
        teams
            .slots()
            .iter()
            .all(|slot| matches!(slot, TeamSlot::Idle)),
        "precondition"
    );
    assert_eq!(teams.free_team_indices(3).len(), 0);
    assert!(
        teams.slots().iter().all(|slot| matches!(slot, TeamSlot::Idle)),
        "asking which teams are free must not MOVE any of them: the commitment is a              reservation the readers honour, not a dispatch, which is what lets it be derived              fresh from the live hold every tick and released by the hold simply settling"
    );
}
