#![allow(clippy::field_reassign_with_default)]

use super::*;
use crate::entities::config::TorpedoTubeConfig;
use std::collections::HashMap;

fn cfg(id: &str, facing_deg: f32, fire_arc_deg: f32) -> TorpedoTubeConfig {
    TorpedoTubeConfig {
        id: id.into(),
        facing_deg,
        fire_arc_deg,
        load_time: None,
        marker: None,
        barrels: Vec::new(),
        pattern: Vec::new(),
        volley_max: 1,
        ai_target_count: None,
        ai: None,
    }
}

fn no_uuid() -> String {
    "test-uuid".to_string()
}

fn default_system() -> TorpedoSystem {
    let tubes = vec![
        cfg("fore_port", -30.0, 90.0),
        cfg("fore_starboard", 30.0, 90.0),
        cfg("aft", 180.0, 90.0),
    ];
    TorpedoSystem::from_configs(&tubes, TorpedoConfig::default())
}

fn load_tube(sys: &mut TorpedoSystem, id: &str) {
    let load_time = sys.tube(id).unwrap().load_time;
    // Set target_count = 1 before loading so the auto-unload logic
    // inside tick() does not immediately drain the torpedo we are about
    // to load (auto-unload fires when loaded_count > target_count).
    sys.tube_mut(id).unwrap().target_count = 1;
    assert!(sys.start_load(id));
    let targets: HashMap<String, (f32, f32, f32)> = HashMap::new();
    sys.tick(load_time, &targets, &mut no_uuid);
}

fn loaded_system() -> TorpedoSystem {
    let mut sys = default_system();
    load_tube(&mut sys, "fore_port");
    load_tube(&mut sys, "fore_starboard");
    load_tube(&mut sys, "aft");
    sys
}

#[test]
fn tubes_start_unloaded() {
    let sys = default_system();
    assert!(sys.tubes.iter().all(|tube| !tube.is_loaded()));
    assert!(sys
        .tubes
        .iter()
        .all(|tube| tube.load_state == TubeLoadState::Unloaded));
}

#[test]
fn launch_returns_launched_with_uuid() {
    let mut sys = loaded_system();
    let r = sys.launch("fore_port", "t1".into(), 0.0, 0.0, 0.0, 0.0, None, None);
    assert_eq!(
        r,
        LaunchResult::Launched {
            uuid: "t1".into(),
            count_remaining: 0
        }
    );
}

#[test]
fn launch_adds_torpedo_to_in_flight() {
    let mut sys = loaded_system();
    sys.launch("fore_port", "t1".into(), 0.0, 0.0, 0.0, 0.0, None, None);
    assert_eq!(sys.in_flight.len(), 1);
    assert_eq!(sys.in_flight[0].uuid, "t1");
}

#[test]
fn salvo_shortfall_counts_only_rounds_not_yet_committed() {
    // Three tubes at volley_max 1, all empty: three rounds short.
    let mut sys = default_system();
    assert_eq!(sys.salvo_shortfall(), 3);

    // A round already claimed for an in-progress load is NOT counted again —
    // `start_load` has already taken it out of the magazine, so counting the
    // gap it has not yet filled would charge for it twice and make a ship
    // with exactly enough rounds look one short for every load cycle.
    sys.tubes[0].target_count = 1;
    assert!(sys.start_load("fore_port"));
    assert_eq!(sys.salvo_shortfall(), 2);

    // And once it lands, the shortfall stays where it was: the round moved
    // from `Loading` into `loaded_count`, it did not appear from nowhere.
    let load_time = sys.tube("fore_port").unwrap().load_time;
    sys.tick(load_time, &HashMap::new(), &mut no_uuid);
    assert_eq!(sys.tube("fore_port").unwrap().loaded_count, 1);
    assert_eq!(sys.salvo_shortfall(), 2);
}

#[test]
fn salvo_shortfall_is_zero_for_a_full_battery_and_for_no_tubes() {
    assert_eq!(loaded_system().salvo_shortfall(), 0);

    let mut config = TorpedoConfig::default();
    config.count = 0;
    let tubeless = TorpedoSystem::from_configs(&[], config);
    assert_eq!(
        tubeless.salvo_shortfall(),
        0,
        "a hull with no tubes is short of nothing — callers asking `can I fill \
         my tubes` must rule the tubeless case out themselves"
    );
}

/// Issue #943: the count a conservation decision is made against is
/// CONSERVED — only a launch may lower it. Loading, waiting, unloading and
/// reloading all move a round between the magazine and a tube, and none of
/// them is a round spent.
#[test]
fn rounds_aboard_only_falls_when_a_round_is_launched() {
    let mut sys = default_system();
    let aboard = sys.rounds_aboard();
    assert_eq!(
        aboard, sys.torpedoes_remaining,
        "with every tube empty the two measures agree — they only diverge \
         once rounds are parked in tubes, which is the whole defect"
    );

    // Mid-load: the magazine is already debited and `loaded_count` has not
    // risen yet, the one moment the round is in neither field.
    sys.tubes[0].target_count = 1;
    assert!(sys.start_load("fore_port"));
    assert_eq!(sys.torpedoes_remaining, aboard - 1);
    assert_eq!(
        sys.rounds_aboard(),
        aboard,
        "a round in transit from the magazine to a tube is still aboard"
    );

    // Landed in the tube.
    let load_time = sys.tube("fore_port").unwrap().load_time;
    sys.tick(load_time, &HashMap::new(), &mut no_uuid);
    assert_eq!(sys.tube("fore_port").unwrap().loaded_count, 1);
    assert_eq!(sys.rounds_aboard(), aboard);

    // Unloading it holds the count too: it is in `loaded_count` while the
    // timer runs and back in the magazine after it. (`target_count` back to
    // 0 first, or `tick`'s auto-reload claims it again the same tick.)
    sys.tubes[0].target_count = 0;
    assert!(sys.start_unload("fore_port"));
    assert_eq!(sys.rounds_aboard(), aboard);
    sys.tick(load_time, &HashMap::new(), &mut no_uuid);
    assert_eq!(sys.torpedoes_remaining, aboard);
    assert_eq!(sys.rounds_aboard(), aboard);

    // Only a launch spends one.
    load_tube(&mut sys, "fore_port");
    sys.launch("fore_port", "t1".into(), 0.0, 0.0, 0.0, 0.0, None, None);
    assert_eq!(
        sys.rounds_aboard(),
        aboard - 1,
        "a launched round is spent — an in-flight torpedo is not aboard"
    );
}

#[test]
fn start_load_decrements_torpedo_count() {
    let mut sys = default_system();
    assert_eq!(sys.torpedoes_remaining, 10);
    assert!(sys.start_load("fore_port"));
    assert_eq!(sys.torpedoes_remaining, 9);
}

#[test]
fn start_load_fails_when_no_torpedoes() {
    let mut config = TorpedoConfig::default();
    config.count = 0;
    let tubes = vec![cfg("fore_port", -30.0, 90.0)];
    let mut sys = TorpedoSystem::from_configs(&tubes, config);
    assert!(!sys.start_load("fore_port"));
    assert_eq!(sys.torpedoes_remaining, 0);
}

// ── channel-2 magazine claim helpers (issue #512) ─────────────────────

#[test]
fn claim_magazine_round_decrements_when_available() {
    let mut sys = default_system();
    assert_eq!(sys.torpedoes_remaining, 10);
    assert!(sys.claim_magazine_round());
    assert_eq!(sys.torpedoes_remaining, 9);
}

#[test]
fn claim_magazine_round_returns_false_when_empty() {
    let mut config = TorpedoConfig::default();
    config.count = 0;
    let tubes = vec![cfg("fore_port", -30.0, 90.0)];
    let mut sys = TorpedoSystem::from_configs(&tubes, config);
    assert!(!sys.claim_magazine_round());
    assert_eq!(sys.torpedoes_remaining, 0);
}

#[test]
fn start_load_reserved_begins_loading_without_touching_magazine() {
    let mut sys = default_system();
    assert_eq!(sys.torpedoes_remaining, 10);
    assert!(sys.start_load_reserved("fore_port"));
    assert_eq!(
        sys.torpedoes_remaining, 10,
        "reserved load must not touch the magazine counter (caller decremented already)"
    );
    assert!(matches!(
        sys.tube("fore_port").unwrap().load_state,
        TubeLoadState::Loading { .. }
    ));
}

#[test]
fn start_load_reserved_fails_for_unknown_tube() {
    let mut sys = default_system();
    assert!(!sys.start_load_reserved("dorsal"));
}

#[test]
fn start_load_reserved_fails_when_tube_not_unloaded() {
    let mut sys = default_system();
    assert!(sys.start_load_reserved("fore_port"));
    // Second call must fail because tube is now Loading.
    assert!(!sys.start_load_reserved("fore_port"));
}

#[test]
fn launch_does_not_change_torpedo_count() {
    let mut sys = loaded_system();
    sys.launch("fore_port", "t1".into(), 0.0, 0.0, 0.0, 0.0, None, None);
    // Count was decremented at load time, not at launch
    assert_eq!(sys.torpedoes_remaining, 7);
}

#[test]
fn start_load_fails_for_unknown_tube() {
    let mut sys = default_system();
    assert!(!sys.start_load("dorsal"));
    assert_eq!(sys.torpedoes_remaining, 10);
}

#[test]
fn launch_leaves_tube_unloaded_until_manual_load() {
    let mut sys = loaded_system();
    sys.launch("fore_port", "t1".into(), 0.0, 0.0, 0.0, 0.0, None, None);
    assert!(!sys.tube("fore_port").unwrap().is_loaded());
    assert_eq!(
        sys.tube("fore_port").unwrap().load_state,
        TubeLoadState::Unloaded
    );
    // Disable auto-management on this tube: the test verifies that a
    // manual launch does NOT trigger an automatic reload on its own.
    // (Auto-management only reloads when target_count > loaded_count.)
    sys.tube_mut("fore_port").unwrap().target_count = 0;
    let targets: HashMap<String, (f32, f32, f32)> = HashMap::new();
    sys.tick(sys.config.load_time, &targets, &mut no_uuid);
    assert_eq!(
        sys.tube("fore_port").unwrap().load_state,
        TubeLoadState::Unloaded
    );
}

#[test]
fn launch_from_unloaded_tube_returns_not_loaded() {
    let mut sys = default_system();
    let r = sys.launch("fore_port", "t1".into(), 0.0, 0.0, 0.0, 0.0, None, None);
    assert_eq!(r, LaunchResult::TubeNotLoaded);
}

#[test]
fn launch_from_unknown_tube_returns_unknown() {
    let mut sys = default_system();
    let r = sys.launch("dorsal", "t1".into(), 0.0, 0.0, 0.0, 0.0, None, None);
    assert_eq!(r, LaunchResult::UnknownTube);
}

#[test]
fn unload_cancelling_load_returns_torpedo_to_pool() {
    let mut sys = default_system();
    assert!(sys.start_load("fore_port"));
    assert_eq!(sys.torpedoes_remaining, 9);
    assert!(sys.start_unload("fore_port"));
    assert_eq!(sys.torpedoes_remaining, 10);
}

#[test]
fn start_unload_loaded_tube_starts_timer_torpedo_returned_on_completion() {
    let mut sys = default_system();
    load_tube(&mut sys, "fore_port");
    assert_eq!(sys.torpedoes_remaining, 9);
    assert!(sys.start_unload("fore_port"));
    assert_eq!(sys.torpedoes_remaining, 9); // not returned yet
                                            // Disable auto-management so the manual-unload test is isolated: after
                                            // the timer fires we expect exactly one torpedo to return to the pool
                                            // and no auto-reload to start.
    sys.tube_mut("fore_port").unwrap().target_count = 0;
    let targets: HashMap<String, (f32, f32, f32)> = HashMap::new();
    sys.tick(
        sys.tube("fore_port").unwrap().load_time,
        &targets,
        &mut no_uuid,
    );
    assert_eq!(sys.torpedoes_remaining, 10); // returned after timer
}

#[test]
fn start_unload_on_unloaded_tube_does_nothing() {
    let mut sys = default_system();
    assert!(!sys.start_unload("fore_port"));
    assert_eq!(sys.torpedoes_remaining, 10);
}

#[test]
fn start_unload_on_unloading_tube_does_nothing() {
    let mut sys = default_system();
    load_tube(&mut sys, "fore_port");
    assert!(sys.start_unload("fore_port"));
    // Already unloading — second call does nothing
    assert!(!sys.start_unload("fore_port"));
}

#[test]
fn start_unload_unknown_tube_returns_false() {
    let mut sys = default_system();
    assert!(!sys.start_unload("dorsal"));
}

#[test]
fn can_launch_from_all_three_tubes_independently() {
    let mut sys = loaded_system();
    let r1 = sys.launch("fore_port", "t1".into(), 0.0, 0.0, 0.0, 0.0, None, None);
    let r2 = sys.launch(
        "fore_starboard",
        "t2".into(),
        0.0,
        0.0,
        0.0,
        0.0,
        None,
        None,
    );
    let r3 = sys.launch("aft", "t3".into(), 0.0, 0.0, 0.0, 0.0, None, None);
    assert!(matches!(r1, LaunchResult::Launched { .. }));
    assert!(matches!(r2, LaunchResult::Launched { .. }));
    assert!(matches!(r3, LaunchResult::Launched { .. }));
    assert_eq!(sys.in_flight.len(), 3);
}

#[test]
fn torpedo_with_no_target_flies_straight() {
    let mut sys = loaded_system();
    sys.launch("fore_port", "t1".into(), 0.0, 0.0, 0.0, 0.0, None, None);
    let initial = sys.in_flight[0].heading;
    let targets: HashMap<String, (f32, f32, f32)> = HashMap::new();
    sys.tick(0.1, &targets, &mut no_uuid);
    assert_eq!(sys.in_flight[0].heading, initial);
}

#[test]
fn torpedo_moves_forward_in_straight_flight() {
    let mut sys = loaded_system();
    sys.launch("fore_port", "t1".into(), 0.0, 0.0, 0.0, 0.0, None, None);
    let targets: HashMap<String, (f32, f32, f32)> = HashMap::new();
    sys.tick(1.0, &targets, &mut no_uuid);
    let t = &sys.in_flight[0];
    assert!(t.x.abs() < 0.01);
    assert!(t.z < 0.0);
}

#[test]
fn torpedo_homes_toward_target() {
    let mut sys = loaded_system();
    sys.launch(
        "fore_port",
        "t1".into(),
        0.0,
        0.0,
        0.0,
        0.0,
        Some("enemy".into()),
        None,
    );
    let mut targets = HashMap::new();
    targets.insert("enemy".into(), (20.0_f32, 0.0_f32, 0.0_f32));
    let h0 = sys.in_flight[0].heading;
    sys.tick(0.1, &targets, &mut no_uuid);
    assert!(sys.in_flight[0].heading > h0);
}

#[test]
fn torpedo_turn_rate_is_limited() {
    let mut config = TorpedoConfig::default();
    config.turn_rate = PI / 4.0;
    let tubes = vec![cfg("fore_port", -30.0, 90.0)];
    let mut sys = TorpedoSystem::from_configs(&tubes, config);
    load_tube(&mut sys, "fore_port");
    sys.launch(
        "fore_port",
        "t1".into(),
        0.0,
        0.0,
        0.0,
        0.0,
        Some("enemy".into()),
        None,
    );
    let mut targets = HashMap::new();
    targets.insert("enemy".into(), (20.0_f32, 0.0_f32, 0.0_f32));
    sys.tick(1.0, &targets, &mut no_uuid);
    assert!(sys.in_flight[0].heading <= PI / 4.0 + 0.001);
}

#[test]
fn torpedo_flies_straight_when_target_destroyed() {
    let mut sys = loaded_system();
    sys.launch(
        "fore_port",
        "t1".into(),
        0.0,
        0.0,
        0.0,
        0.0,
        Some("enemy".into()),
        None,
    );
    let targets: HashMap<String, (f32, f32, f32)> = HashMap::new();
    let h0 = sys.in_flight[0].heading;
    sys.tick(0.5, &targets, &mut no_uuid);
    assert_eq!(sys.in_flight[0].heading, h0);
}

#[test]
fn torpedo_target_uuid_locked_at_launch_and_never_updated() {
    // Fire at "target-a". Then tick with positions for both "target-a"
    // (far right) and a new "target-b" (straight ahead). The torpedo must
    // keep homing toward "target-a", never re-routing to "target-b", and
    // its stored target_uuid must remain "target-a" throughout.
    let mut sys = loaded_system();
    sys.launch(
        "fore_port",
        "t1".into(),
        0.0,
        0.0,
        0.0,
        0.0,
        Some("target-a".into()),
        None,
    );
    let mut targets = HashMap::new();
    targets.insert("target-a".into(), (100.0_f32, 0.0_f32, 0.0_f32)); // hard right
    targets.insert("target-b".into(), (0.0_f32, 0.0_f32, -100.0_f32)); // straight ahead

    let h0 = sys.in_flight[0].heading;
    sys.tick(0.1, &targets, &mut no_uuid);

    // The torpedo must have turned right (toward target-a), not stayed straight.
    assert!(
        sys.in_flight[0].heading > h0,
        "should home toward target-a (rightward turn)"
    );
    // The stored target_uuid is still "target-a".
    assert_eq!(
        sys.in_flight[0].target_uuid.as_deref(),
        Some("target-a"),
        "target_uuid must not change after launch"
    );
}

#[test]
fn torpedo_expires_after_lifespan() {
    let mut config = TorpedoConfig::default();
    config.lifespan = 5.0;
    let tubes = vec![cfg("fore_port", -30.0, 90.0)];
    let mut sys = TorpedoSystem::from_configs(&tubes, config);
    load_tube(&mut sys, "fore_port");
    sys.launch("fore_port", "t1".into(), 0.0, 0.0, 0.0, 0.0, None, None);
    let targets: HashMap<String, (f32, f32, f32)> = HashMap::new();
    let r = sys.tick(5.1, &targets, &mut no_uuid);
    assert!(r.expired.contains(&"t1".to_string()));
    assert_eq!(sys.in_flight.len(), 0);
}

#[test]
fn torpedo_not_expired_before_lifespan() {
    let mut config = TorpedoConfig::default();
    config.lifespan = 5.0;
    let tubes = vec![cfg("fore_port", -30.0, 90.0)];
    let mut sys = TorpedoSystem::from_configs(&tubes, config);
    load_tube(&mut sys, "fore_port");
    sys.launch("fore_port", "t1".into(), 0.0, 0.0, 0.0, 0.0, None, None);
    let targets: HashMap<String, (f32, f32, f32)> = HashMap::new();
    let r = sys.tick(4.9, &targets, &mut no_uuid);
    assert!(!r.expired.contains(&"t1".to_string()));
    assert_eq!(sys.in_flight.len(), 1);
}

#[test]
fn collision_removes_torpedo_and_returns_damage() {
    let mut sys = loaded_system();
    sys.launch("fore_port", "t1".into(), 0.0, 0.0, 0.0, 0.0, None, None);
    let d = sys.handle_collision("t1");
    assert_eq!(d, Some(50));
    assert_eq!(sys.in_flight.len(), 0);
}

#[test]
fn collision_with_unknown_uuid_returns_none() {
    let mut sys = default_system();
    let d = sys.handle_collision("nonexistent");
    assert_eq!(d, None);
}

#[test]
fn tube_loads_after_manual_load_time() {
    let mut config = TorpedoConfig::default();
    config.load_time = 10.0;
    let tubes = vec![cfg("fore_port", -30.0, 90.0)];
    let mut sys = TorpedoSystem::from_configs(&tubes, config);
    assert!(sys.start_load("fore_port"));
    assert!(!sys.tube("fore_port").unwrap().is_loaded());
    let targets: HashMap<String, (f32, f32, f32)> = HashMap::new();
    sys.tick(10.0, &targets, &mut no_uuid);
    assert!(sys.tube("fore_port").unwrap().is_loaded());
}

#[test]
fn tube_not_loaded_before_manual_load_time_expires() {
    let mut config = TorpedoConfig::default();
    config.load_time = 10.0;
    let tubes = vec![cfg("fore_port", -30.0, 90.0)];
    let mut sys = TorpedoSystem::from_configs(&tubes, config);
    assert!(sys.start_load("fore_port"));
    let targets: HashMap<String, (f32, f32, f32)> = HashMap::new();
    sys.tick(9.9, &targets, &mut no_uuid);
    assert!(!sys.tube("fore_port").unwrap().is_loaded());
}

// ── proximity detonation ──────────────────────────────────────────────

fn detonation_system(detonation_radius: f32) -> TorpedoSystem {
    let mut config = TorpedoConfig::default();
    config.detonation_radius = detonation_radius;
    let tubes = vec![cfg("fore_port", -30.0, 90.0)];
    let mut sys = TorpedoSystem::from_configs(&tubes, config);
    load_tube(&mut sys, "fore_port");
    sys
}

#[test]
fn find_detonation_hits_returns_empty_when_no_targets_in_range() {
    let mut sys = detonation_system(5.0);
    sys.launch("fore_port", "t1".into(), 0.0, 0.0, 0.0, 0.0, None, None);
    // Target far away with small radius.
    let targets = vec![("enemy".to_string(), 100.0, 0.0, 100.0, 1.0)];
    let hits = sys.find_detonation_hits(&targets);
    assert!(hits.is_empty());
}

#[test]
fn find_detonation_hits_reports_target_within_detonation_radius() {
    let mut sys = detonation_system(5.0);
    sys.launch("fore_port", "t1".into(), 0.0, 0.0, 0.0, 0.0, None, None);
    // Target at (0, -4): distance 4, threshold 5+0 = 5.
    let targets = vec![("enemy".to_string(), 0.0, 0.0, -4.0, 0.0)];
    let hits = sys.find_detonation_hits(&targets);
    assert_eq!(hits, vec![("t1".to_string(), "enemy".to_string())]);
}

#[test]
fn find_detonation_hits_includes_target_radius_in_threshold() {
    // Detonation radius 1, target radius 10, distance 9 → should hit.
    let mut sys = detonation_system(1.0);
    sys.launch("fore_port", "t1".into(), 0.0, 0.0, 0.0, 0.0, None, None);
    let targets = vec![("rock".to_string(), 0.0, 0.0, -9.0, 10.0)];
    let hits = sys.find_detonation_hits(&targets);
    assert_eq!(hits, vec![("t1".to_string(), "rock".to_string())]);
}

#[test]
fn find_detonation_hits_picks_nearest_when_multiple_in_range() {
    let mut sys = detonation_system(50.0);
    sys.launch("fore_port", "t1".into(), 0.0, 0.0, 0.0, 0.0, None, None);
    let targets = vec![
        ("far".to_string(), 0.0, 0.0, -40.0, 0.0),
        ("near".to_string(), 0.0, 0.0, -5.0, 0.0),
    ];
    let hits = sys.find_detonation_hits(&targets);
    assert_eq!(hits, vec![("t1".to_string(), "near".to_string())]);
}

#[test]
fn find_detonation_hits_detonates_unlocked_torpedo_on_contact() {
    // Bug repro: shot without a target lock should still explode.
    let mut sys = detonation_system(5.0);
    sys.launch(
        "fore_port",
        "t1".into(),
        0.0,
        0.0,
        0.0,
        0.0,
        /*target_uuid*/ None,
        /*source_uuid*/ None,
    );
    let targets = vec![("raider".to_string(), 0.0, 0.0, -3.0, 1.0)];
    let hits = sys.find_detonation_hits(&targets);
    assert_eq!(hits, vec![("t1".to_string(), "raider".to_string())]);
}

#[test]
fn find_detonation_hits_handles_multiple_torpedoes_independently() {
    let mut sys = detonation_system(2.0);
    sys.launch("fore_port", "t1".into(), 0.0, 0.0, 0.0, 0.0, None, None);
    // Manually push a second torpedo so the test can focus on detonation
    // matching rather than tube load state.
    sys.in_flight.push(Torpedo {
        uuid: "t2".into(),
        x: 100.0,
        y: 0.0,
        z: 100.0,
        heading: 0.0,
        pitch: 0.0,
        lifespan_remaining: 10.0,
        target_uuid: None,
        source_uuid: None,
        tube_id: "fore_port".into(),
        shield_pierce: 0.0,
    });
    let targets = vec![
        ("a".to_string(), 1.0, 0.0, 0.0, 0.0),     // close to t1
        ("b".to_string(), 101.0, 0.0, 100.0, 0.0), // close to t2
    ];
    let hits = sys.find_detonation_hits(&targets);
    assert_eq!(hits.len(), 2);
    assert!(hits.contains(&("t1".to_string(), "a".to_string())));
    assert!(hits.contains(&("t2".to_string(), "b".to_string())));
}

#[test]
fn find_detonation_hits_never_detonates_on_source_uuid() {
    // Regression: torpedoes spawn at the firing ship's centre. Without
    // source_uuid filtering, every torpedo would instantly detonate on
    // its launcher and never reach an actual target.
    let mut sys = detonation_system(5.0);
    sys.launch(
        "fore_port",
        "t1".into(),
        0.0,
        0.0,
        0.0,
        0.0,
        None,
        Some("player-ship".into()),
    );
    // Player ship sitting right on top of the torpedo, plus a raider
    // also in range further out.
    let targets = vec![
        ("player-ship".to_string(), 0.0, 0.0, 0.0, 5.0),
        ("raider".to_string(), 0.0, 0.0, -3.0, 1.0),
    ];
    let hits = sys.find_detonation_hits(&targets);
    // Should hit the raider, not the launcher.
    assert_eq!(hits, vec![("t1".to_string(), "raider".to_string())]);
}

#[test]
fn find_detonation_hits_with_no_targets_in_range_returns_empty_even_if_source_present() {
    let mut sys = detonation_system(5.0);
    sys.launch(
        "fore_port",
        "t1".into(),
        0.0,
        0.0,
        0.0,
        0.0,
        None,
        Some("player-ship".into()),
    );
    let targets = vec![("player-ship".to_string(), 0.0, 0.0, 0.0, 5.0)];
    let hits = sys.find_detonation_hits(&targets);
    assert!(hits.is_empty());
}

// ── Volley mechanics (issue #632) ─────────────────────────────────────

fn volley_cfg(id: &str, volley_max: u32) -> TorpedoTubeConfig {
    TorpedoTubeConfig {
        id: id.into(),
        facing_deg: 0.0,
        fire_arc_deg: 180.0,
        load_time: None,
        marker: None,
        barrels: Vec::new(),
        pattern: Vec::new(),
        volley_max,
        ai_target_count: None,
        ai: None,
    }
}

#[test]
fn volley_max_defaults_to_1_on_standard_tube() {
    let sys = default_system();
    assert_eq!(sys.tube("fore_port").unwrap().volley_max, 1);
}

/// The AI's standing volley target is TOML, not a constant, and a hull
/// that says nothing gets "keep the tube as full as it can".
#[test]
fn ai_target_count_defaults_to_volley_max() {
    let sys = TorpedoSystem::from_configs(&[volley_cfg("t1", 3)], TorpedoConfig::default());
    assert_eq!(sys.tube("t1").unwrap().ai_target_count, 3);
    // The tube still starts empty — the AI has to *ask* for the load.
    assert_eq!(sys.tube("t1").unwrap().target_count, 0);
}

#[test]
fn ai_target_count_reads_the_ship_wide_default() {
    let config = TorpedoConfig {
        ai_volley_target: Some(2),
        ..Default::default()
    };
    let sys = TorpedoSystem::from_configs(&[volley_cfg("t1", 3)], config);
    assert_eq!(sys.tube("t1").unwrap().ai_target_count, 2);
}

#[test]
fn per_tube_ai_target_count_overrides_the_ship_wide_default_and_clamps() {
    let config = TorpedoConfig {
        ai_volley_target: Some(2),
        ..Default::default()
    };
    let mut low = volley_cfg("low", 3);
    low.ai_target_count = Some(1);
    let mut greedy = volley_cfg("greedy", 3);
    greedy.ai_target_count = Some(9);
    let sys = TorpedoSystem::from_configs(&[low, greedy], config);
    assert_eq!(sys.tube("low").unwrap().ai_target_count, 1);
    assert_eq!(
        sys.tube("greedy").unwrap().ai_target_count,
        3,
        "a per-tube ai_target_count above volley_max clamps to what fits"
    );
}

#[test]
fn set_volley_target_clamps_to_volley_max() {
    let mut sys = TorpedoSystem::from_configs(&[volley_cfg("t1", 3)], TorpedoConfig::default());
    assert!(sys.set_volley_target("t1", 5)); // 5 > 3, should clamp
    assert_eq!(sys.tube("t1").unwrap().target_count, 3);
}

#[test]
fn set_volley_target_returns_false_for_unknown_tube() {
    let mut sys = default_system();
    assert!(!sys.set_volley_target("dorsal", 1));
}

#[test]
fn auto_load_toward_target_count() {
    // Set target_count=2, volley_max=2. Tick twice through load_time.
    // Should end up with loaded_count==2 and torpedoes_remaining decremented by 2.
    let mut config = TorpedoConfig::default();
    config.count = 10;
    config.load_time = 5.0;
    let mut sys = TorpedoSystem::from_configs(&[volley_cfg("t1", 2)], config);
    sys.set_volley_target("t1", 2);
    let targets: HashMap<String, (f32, f32, f32)> = HashMap::new();
    // First tick starts loading torpedo #1.
    sys.tick(0.0, &targets, &mut no_uuid);
    assert_eq!(sys.torpedoes_remaining, 9);
    assert!(matches!(
        sys.tube("t1").unwrap().load_state,
        TubeLoadState::Loading { .. }
    ));
    // Tick past load_time: torpedo #1 finishes, torpedo #2 starts immediately.
    sys.tick(5.0, &targets, &mut no_uuid);
    assert_eq!(sys.tube("t1").unwrap().loaded_count, 1);
    assert_eq!(sys.torpedoes_remaining, 8); // second load started
                                            // Tick past load_time again: torpedo #2 finishes.
    sys.tick(5.0, &targets, &mut no_uuid);
    assert_eq!(sys.tube("t1").unwrap().loaded_count, 2);
    // No more auto-loads: loaded_count == target_count.
    assert_eq!(sys.torpedoes_remaining, 8);
}

#[test]
fn fire_volley_fires_first_torpedo_immediately_rest_in_burst() {
    let mut config = TorpedoConfig::default();
    config.count = 10;
    config.load_time = 1.0;
    config.burst_interval_secs = 0.3;
    let tubes = vec![volley_cfg("t1", 3)];
    let mut sys = TorpedoSystem::from_configs(&tubes, config);
    // Load 3 torpedoes manually.
    sys.torpedoes_remaining -= 3;
    let tube = sys.tube_mut("t1").unwrap();
    tube.loaded_count = 3;
    let result = sys.launch("t1", "uuid-0".into(), 0.0, 0.0, 0.0, 0.0, None, None);
    assert!(matches!(
        result,
        LaunchResult::Launched {
            count_remaining: 2,
            ..
        }
    ));
    assert_eq!(sys.in_flight.len(), 1);
    assert_eq!(sys.burst_states.len(), 1);
    assert_eq!(sys.burst_states[0].pending, 2);
}

#[test]
fn burst_launches_remaining_torpedoes_at_interval() {
    let mut config = TorpedoConfig::default();
    config.count = 10;
    config.burst_interval_secs = 0.3;
    let tubes = vec![volley_cfg("t1", 3)];
    let mut sys = TorpedoSystem::from_configs(&tubes, config);
    sys.torpedoes_remaining -= 3;
    let tube = sys.tube_mut("t1").unwrap();
    tube.loaded_count = 3;
    sys.launch("t1", "uuid-0".into(), 0.0, 0.0, 0.0, 0.0, None, None);
    assert_eq!(sys.in_flight.len(), 1);

    let targets: HashMap<String, (f32, f32, f32)> = HashMap::new();
    let mut uuid_counter = 1u32;
    let mut next = || {
        let s = format!("uuid-{uuid_counter}");
        uuid_counter += 1;
        s
    };
    // Tick past first burst interval: should fire torpedo #2.
    sys.tick(0.3, &targets, &mut next);
    assert_eq!(
        sys.in_flight.len(),
        2,
        "torpedo #2 should fire after interval"
    );
    // Tick past second interval: torpedo #3.
    sys.tick(0.3, &targets, &mut next);
    assert_eq!(
        sys.in_flight.len(),
        3,
        "torpedo #3 should fire after second interval"
    );
    assert!(sys.burst_states.is_empty(), "burst state should be cleared");
}

#[test]
fn fire_with_partial_load_fires_what_is_loaded() {
    let mut config = TorpedoConfig::default();
    config.count = 10;
    let tubes = vec![volley_cfg("t1", 4)];
    let mut sys = TorpedoSystem::from_configs(&tubes, config);
    // Only 2 of 4 are loaded.
    sys.torpedoes_remaining -= 2;
    let tube = sys.tube_mut("t1").unwrap();
    tube.loaded_count = 2;
    let result = sys.launch("t1", "uuid-0".into(), 0.0, 0.0, 0.0, 0.0, None, None);
    assert!(matches!(
        result,
        LaunchResult::Launched {
            count_remaining: 1,
            ..
        }
    ));
    assert_eq!(sys.tube("t1").unwrap().loaded_count, 0);
}

#[test]
fn auto_unload_when_target_count_decremented() {
    let mut config = TorpedoConfig::default();
    config.count = 10;
    config.load_time = 1.0;
    let tubes = vec![volley_cfg("t1", 3)];
    let mut sys = TorpedoSystem::from_configs(&tubes, config);
    // Start with 2 loaded, target_count = 2 (auto-managed mode).
    sys.torpedoes_remaining -= 2;
    {
        let tube = sys.tube_mut("t1").unwrap();
        tube.loaded_count = 2;
        tube.target_count = 2;
    }
    // Drop target to 1 → should auto-unload one torpedo.
    sys.set_volley_target("t1", 1);
    let targets: HashMap<String, (f32, f32, f32)> = HashMap::new();
    // First tick starts unloading one.
    sys.tick(0.0, &targets, &mut no_uuid);
    assert!(matches!(
        sys.tube("t1").unwrap().load_state,
        TubeLoadState::Unloading { .. }
    ));
    // Complete the unload: loaded_count goes from 2 to 1.
    sys.tick(1.0, &targets, &mut no_uuid);
    assert_eq!(sys.tube("t1").unwrap().loaded_count, 1);
    // target_count == loaded_count now → no more auto-unload.
    sys.tick(0.0, &targets, &mut no_uuid);
    assert_eq!(
        sys.tube("t1").unwrap().loaded_count,
        1,
        "should stop at target_count=1"
    );
    assert_eq!(
        sys.torpedoes_remaining, 9,
        "one torpedo returned to magazine"
    );
}

#[test]
fn auto_unload_when_target_set_to_zero() {
    // Regression for issue #632: setting target_count=0 must drain all
    // loaded torpedoes back to the magazine automatically.
    let mut config = TorpedoConfig::default();
    config.count = 10;
    config.load_time = 1.0;
    let tubes = vec![volley_cfg("t1", 3)];
    let mut sys = TorpedoSystem::from_configs(&tubes, config);
    // Pre-load 2 torpedoes directly (bypass timer for test clarity).
    sys.torpedoes_remaining -= 2;
    {
        let tube = sys.tube_mut("t1").unwrap();
        tube.loaded_count = 2;
        tube.target_count = 0; // target is already 0
    }
    let targets: HashMap<String, (f32, f32, f32)> = HashMap::new();
    // First tick should start unloading the first torpedo.
    sys.tick(0.0, &targets, &mut no_uuid);
    assert!(
        matches!(
            sys.tube("t1").unwrap().load_state,
            TubeLoadState::Unloading { .. }
        ),
        "tube should be unloading after target_count set to 0"
    );
    // Complete the first unload.
    sys.tick(1.0, &targets, &mut no_uuid);
    assert_eq!(sys.tube("t1").unwrap().loaded_count, 1);
    // Next tick starts unloading the second torpedo.
    sys.tick(0.0, &targets, &mut no_uuid);
    assert!(matches!(
        sys.tube("t1").unwrap().load_state,
        TubeLoadState::Unloading { .. }
    ));
    // Complete the second unload.
    sys.tick(1.0, &targets, &mut no_uuid);
    assert_eq!(sys.tube("t1").unwrap().loaded_count, 0);
    assert_eq!(
        sys.tube("t1").unwrap().load_state,
        TubeLoadState::Unloaded,
        "tube should be fully unloaded"
    );
    assert_eq!(
        sys.torpedoes_remaining, 10,
        "both torpedoes returned to magazine"
    );
}

// ── Patterned multi-barrel attacks (issue #766) ──────────────────────────

use crate::weapons::pattern::BarrelPatternStep;

fn barrel_step(barrels: &[u32], offset: f32) -> BarrelPatternStep {
    BarrelPatternStep {
        barrels: barrels.to_vec(),
        offset_secs: offset,
    }
}

fn patterned_cfg(
    id: &str,
    barrels: Vec<String>,
    pattern: Vec<BarrelPatternStep>,
    volley_max: u32,
) -> TorpedoTubeConfig {
    TorpedoTubeConfig {
        id: id.into(),
        facing_deg: 0.0,
        fire_arc_deg: 180.0,
        load_time: None,
        marker: None,
        barrels,
        pattern,
        volley_max,
        ai_target_count: None,
        ai: None,
    }
}

/// Pre-load `n` rounds into `tube` directly, decrementing the magazine to
/// mirror the load-time spend (so a later `launch` proves it does NOT spend
/// again).
fn preload(sys: &mut TorpedoSystem, tube: &str, n: u32) {
    sys.torpedoes_remaining -= n;
    sys.tube_mut(tube).unwrap().loaded_count = n;
}

fn burst_next() -> impl FnMut() -> String {
    let mut i = 100u32;
    move || {
        i += 1;
        format!("burst-{i}")
    }
}

#[test]
fn patterned_alternating_launches_from_barrels_in_sequence() {
    // Two barrels, two steps at increasing offsets → the volley's rounds
    // leave from barrel 0 then barrel 1.
    let mut config = TorpedoConfig::default();
    config.count = 10;
    config.burst_interval_secs = 0.3;
    let cfg = patterned_cfg(
        "t1",
        vec!["b0".into(), "b1".into()],
        vec![barrel_step(&[0], 0.0), barrel_step(&[1], 0.5)],
        2,
    );
    let mut sys = TorpedoSystem::from_configs(&[cfg], config);
    preload(&mut sys, "t1", 2);

    // Distinct origins per barrel so a torpedo's X identifies its barrel.
    let origins = [(10.0, 0.0, 0.0), (20.0, 0.0, 0.0)];
    let r = sys.launch_with_barrels("t1", "u0".into(), &origins, 0.0, 0.0, 0.0, 0.0, None, None);
    assert!(matches!(
        r,
        LaunchResult::Launched {
            count_remaining: 1,
            ..
        }
    ));
    // Immediate round from barrel 0.
    assert_eq!(sys.in_flight.len(), 1);
    assert!((sys.in_flight[0].x - 10.0).abs() < 1e-4, "barrel 0 origin");
    assert_eq!(sys.tube("t1").unwrap().active_barrels, vec![0]);
    assert_eq!(sys.tube("t1").unwrap().pattern_step, 1);

    // Burst shot fires from barrel 1 after the burst interval.
    let targets: HashMap<String, (f32, f32, f32)> = HashMap::new();
    let mut next = burst_next();
    sys.tick(0.3, &targets, &mut next);
    assert_eq!(sys.in_flight.len(), 2);
    let burst = sys.in_flight.iter().find(|t| t.uuid != "u0").unwrap();
    assert!((burst.x - 20.0).abs() < 1e-4, "barrel 1 origin");
    assert_eq!(sys.tube("t1").unwrap().active_barrels, vec![1]);
    assert_eq!(sys.tube("t1").unwrap().pattern_step, 2);
}

#[test]
fn patterned_simultaneous_launches_from_multiple_barrels() {
    // One step listing several barrels → consecutive rounds leave from each
    // listed barrel. With two loaded, both barrels are used.
    let mut config = TorpedoConfig::default();
    config.count = 10;
    config.burst_interval_secs = 0.3;
    let cfg = patterned_cfg(
        "t1",
        vec!["b0".into(), "b1".into()],
        vec![barrel_step(&[0, 1], 0.0)],
        2,
    );
    let mut sys = TorpedoSystem::from_configs(&[cfg], config);
    preload(&mut sys, "t1", 2);

    let origins = [(10.0, 0.0, 0.0), (20.0, 0.0, 0.0)];
    sys.launch_with_barrels("t1", "u0".into(), &origins, 0.0, 0.0, 0.0, 0.0, None, None);
    assert!((sys.in_flight[0].x - 10.0).abs() < 1e-4, "barrel 0 origin");

    let targets: HashMap<String, (f32, f32, f32)> = HashMap::new();
    let mut next = burst_next();
    sys.tick(0.3, &targets, &mut next);
    assert_eq!(sys.in_flight.len(), 2, "both barrels fire");
    let burst = sys.in_flight.iter().find(|t| t.uuid != "u0").unwrap();
    assert!((burst.x - 20.0).abs() < 1e-4, "barrel 1 origin");
}

/// AC3: a two-barrel simultaneous step with only ONE round loaded fires
/// exactly one torpedo and leaves the magazine untouched. The pattern never
/// invents rounds — `loaded_count` is the count authority.
#[test]
fn patterned_simultaneous_step_with_one_loaded_fires_exactly_one() {
    let mut config = TorpedoConfig::default();
    config.count = 10;
    let cfg = patterned_cfg(
        "t1",
        vec!["b0".into(), "b1".into()],
        vec![barrel_step(&[0, 1], 0.0)],
        2,
    );
    let mut sys = TorpedoSystem::from_configs(&[cfg], config);
    preload(&mut sys, "t1", 1);
    let mag_before = sys.torpedoes_remaining;

    let origins = [(10.0, 0.0, 0.0), (20.0, 0.0, 0.0)];
    let r = sys.launch_with_barrels("t1", "u0".into(), &origins, 0.0, 0.0, 0.0, 0.0, None, None);
    assert!(
        matches!(
            r,
            LaunchResult::Launched {
                count_remaining: 0,
                ..
            }
        ),
        "a step listing 2 barrels must NOT fire 2 when only 1 is loaded"
    );
    assert_eq!(sys.in_flight.len(), 1, "exactly one torpedo fired");
    assert!(
        sys.burst_states.is_empty(),
        "no burst scheduled for 1 round"
    );
    assert!(
        (sys.in_flight[0].x - 10.0).abs() < 1e-4,
        "first barrel only"
    );
    assert_eq!(
        sys.torpedoes_remaining, mag_before,
        "launch must not spend from the magazine (spend happens at load)"
    );
}

/// AC3: the burst count stays bounded by `loaded_count` even when the
/// pattern is short — the barrel sequence cycles for origins but never
/// extends the volley.
#[test]
fn patterned_burst_count_bounded_by_loaded_count() {
    let mut config = TorpedoConfig::default();
    config.count = 10;
    config.burst_interval_secs = 0.3;
    // Single-step, single-barrel pattern; two rounds loaded.
    let cfg = patterned_cfg("t1", vec!["b0".into()], vec![barrel_step(&[0], 0.0)], 3);
    let mut sys = TorpedoSystem::from_configs(&[cfg], config);
    preload(&mut sys, "t1", 2);
    let mag_before = sys.torpedoes_remaining;

    let origins = [(10.0, 0.0, 0.0)];
    let r = sys.launch_with_barrels("t1", "u0".into(), &origins, 0.0, 0.0, 0.0, 0.0, None, None);
    assert!(matches!(
        r,
        LaunchResult::Launched {
            count_remaining: 1,
            ..
        }
    ));
    let targets: HashMap<String, (f32, f32, f32)> = HashMap::new();
    let mut next = burst_next();
    // Drive well past several burst intervals.
    sys.tick(0.3, &targets, &mut next);
    sys.tick(0.3, &targets, &mut next);
    sys.tick(0.3, &targets, &mut next);
    assert_eq!(
        sys.in_flight.len(),
        2,
        "exactly loaded_count torpedoes fire — no more"
    );
    assert!(sys.burst_states.is_empty());
    assert_eq!(
        sys.torpedoes_remaining, mag_before,
        "magazine untouched by patterned firing"
    );
}

#[test]
fn legacy_launch_without_barrels_uses_ship_centre_origin() {
    // Back-compat: no barrels/pattern authored → every round leaves from the
    // passed launch origin exactly as before issue #766.
    let mut config = TorpedoConfig::default();
    config.count = 10;
    config.burst_interval_secs = 0.3;
    let mut sys = TorpedoSystem::from_configs(&[volley_cfg("t1", 2)], config);
    preload(&mut sys, "t1", 2);
    sys.launch("t1", "u0".into(), 7.0, 0.0, -3.0, 0.0, None, None);
    assert!((sys.in_flight[0].x - 7.0).abs() < 1e-4);
    assert!((sys.in_flight[0].z - (-3.0)).abs() < 1e-4);
    // Legacy tube reports no pattern → no step indicator.
    assert_eq!(sys.tube("t1").unwrap().pattern_len(), 0);
    let targets: HashMap<String, (f32, f32, f32)> = HashMap::new();
    let mut next = burst_next();
    sys.tick(0.3, &targets, &mut next);
    let burst = sys.in_flight.iter().find(|t| t.uuid != "u0").unwrap();
    assert!((burst.x - 7.0).abs() < 1e-4, "burst from ship centre too");
}

// ── Full-3D torpedo flight (issue #768) ──────────────────────────────────

/// A torpedo fired level at a target ABOVE it climbs: its `y` and `pitch`
/// both increase as it homes toward the target's altitude. Vertical
/// separation therefore changes guidance (AC2).
#[test]
fn torpedo_climbs_toward_target_above() {
    let mut sys = loaded_system();
    sys.launch(
        "fore_port",
        "t1".into(),
        0.0,
        0.0,
        0.0,
        0.0,
        Some("enemy".into()),
        None,
    );
    // Target dead ahead (−Z) but 40 m up.
    let mut targets = HashMap::new();
    targets.insert("enemy".into(), (0.0_f32, 40.0_f32, -40.0_f32));
    assert_eq!(sys.in_flight[0].y, 0.0);
    assert_eq!(sys.in_flight[0].pitch, 0.0);
    sys.tick(0.5, &targets, &mut no_uuid);
    assert!(
        sys.in_flight[0].pitch > 0.0,
        "pitch should tilt up toward the higher target"
    );
    assert!(
        sys.in_flight[0].y > 0.0,
        "torpedo should gain altitude climbing toward the target"
    );
}

/// Mirror of the climb case: a target BELOW drives a descent (negative
/// pitch, decreasing `y`).
#[test]
fn torpedo_descends_toward_target_below() {
    let mut sys = loaded_system();
    sys.launch(
        "fore_port",
        "t1".into(),
        0.0,
        0.0,
        0.0,
        0.0,
        Some("enemy".into()),
        None,
    );
    let mut targets = HashMap::new();
    targets.insert("enemy".into(), (0.0_f32, -40.0_f32, -40.0_f32));
    sys.tick(0.5, &targets, &mut no_uuid);
    assert!(sys.in_flight[0].pitch < 0.0, "pitch should tilt down");
    assert!(sys.in_flight[0].y < 0.0, "torpedo should lose altitude");
}

/// The vertical steering is rate-limited by the SAME `turn_rate` clamp as
/// the yaw: over one second the pitch cannot exceed `turn_rate` radians even
/// when the target sits straight overhead (desired pitch = +π/2).
#[test]
fn vertical_steering_is_rate_limited_by_turn_rate() {
    let mut config = TorpedoConfig::default();
    config.turn_rate = PI / 4.0;
    let tubes = vec![cfg("fore_port", -30.0, 90.0)];
    let mut sys = TorpedoSystem::from_configs(&tubes, config);
    load_tube(&mut sys, "fore_port");
    sys.launch(
        "fore_port",
        "t1".into(),
        0.0,
        0.0,
        0.0,
        0.0,
        Some("enemy".into()),
        None,
    );
    // Target directly overhead → desired pitch is +π/2, far beyond one
    // second of turn budget.
    let mut targets = HashMap::new();
    targets.insert("enemy".into(), (0.0_f32, 1000.0_f32, 0.0_f32));
    sys.tick(1.0, &targets, &mut no_uuid);
    assert!(
        sys.in_flight[0].pitch <= PI / 4.0 + 1e-4,
        "pitch climb per second must be clamped to turn_rate"
    );
}

/// Vertical separation changes collision: same XZ, a large ΔY leaves the
/// torpedo OUTSIDE the 3D detonation sphere (no hit), while a small ΔY is
/// inside it (hit). AC2 / AC1.
#[test]
fn vertical_separation_governs_3d_collision() {
    let mut sys = detonation_system(5.0);
    sys.launch("fore_port", "t1".into(), 0.0, 0.0, 0.0, 0.0, None, None);
    // Same XZ as the torpedo (0,0), radius 0. ΔY = 20 ≫ det radius 5 → miss.
    let far_up = vec![("blimp".to_string(), 0.0, 20.0, 0.0, 0.0)];
    assert!(
        sys.find_detonation_hits(&far_up).is_empty(),
        "a torpedo 20 m below a target at the same XZ must NOT detonate"
    );
    // ΔY = 3 < det radius 5 → hit.
    let near_up = vec![("blimp".to_string(), 0.0, 3.0, 0.0, 0.0)];
    assert_eq!(
        sys.find_detonation_hits(&near_up),
        vec![("t1".to_string(), "blimp".to_string())],
        "within the 3D radius the torpedo detonates"
    );
}

/// The detonation payload carries the torpedo's vertical impact position, so
/// a torpedo that climbed reports a non-zero `impact_y` (AC1: 3D detonation).
#[test]
fn detonation_carries_vertical_impact_point() {
    let mut sys = detonation_system(5.0);
    sys.launch(
        "fore_port",
        "t1".into(),
        0.0,
        0.0,
        0.0,
        0.0,
        Some("enemy".into()),
        None,
    );
    // Fly a while homing at a high target so the torpedo gains altitude.
    let mut targets = HashMap::new();
    targets.insert("enemy".into(), (0.0_f32, 100.0_f32, -60.0_f32));
    sys.tick(1.0, &targets, &mut no_uuid);
    sys.tick(1.0, &targets, &mut no_uuid);
    let expected_y = sys.in_flight[0].y;
    assert!(expected_y > 0.0, "torpedo should have climbed");
    let det = sys.handle_collision_full("t1").unwrap();
    assert!(
        (det.impact_y - expected_y).abs() < 1e-4,
        "impact_y must equal the torpedo's altitude at detonation"
    );
}

/// Patterned launch preserves the barrel marker's Y: each authored barrel
/// origin is a full 3D point, so a round leaving a raised barrel spawns at
/// that altitude (AC4: patterned origins carry Y).
#[test]
fn patterned_origins_carry_barrel_y() {
    let mut config = TorpedoConfig::default();
    config.count = 10;
    config.burst_interval_secs = 0.3;
    let cfg = patterned_cfg(
        "t1",
        vec!["b0".into(), "b1".into()],
        vec![barrel_step(&[0], 0.0), barrel_step(&[1], 0.5)],
        2,
    );
    let mut sys = TorpedoSystem::from_configs(&[cfg], config);
    preload(&mut sys, "t1", 2);
    // Distinct Y per barrel so a torpedo's altitude identifies its barrel.
    let origins = [(10.0, 2.0, 0.0), (20.0, -3.0, 0.0)];
    sys.launch_with_barrels("t1", "u0".into(), &origins, 0.0, 0.0, 0.0, 0.0, None, None);
    assert!(
        (sys.in_flight[0].y - 2.0).abs() < 1e-4,
        "immediate round keeps barrel 0's Y"
    );
    let targets: HashMap<String, (f32, f32, f32)> = HashMap::new();
    let mut next = burst_next();
    sys.tick(0.3, &targets, &mut next);
    let burst = sys.in_flight.iter().find(|t| t.uuid != "u0").unwrap();
    assert!(
        (burst.y - (-3.0)).abs() < 1e-4,
        "burst round keeps barrel 1's Y"
    );
}

/// AC3 planar collapse: with every Y at 0, the 3D distance check reduces
/// EXACTLY to the 2D one. This target sits 6 m astern (ΔZ only) with the
/// detonation radius at 5 — a boundary the 2D check also missed — proving no
/// spurious `dy` term crept in.
#[test]
fn planar_collision_matches_2d_when_all_y_zero() {
    let mut sys = detonation_system(5.0);
    sys.launch("fore_port", "t1".into(), 0.0, 0.0, 0.0, 0.0, None, None);
    // distance 6 > 5 → miss, exactly as the pure XZ check would decide.
    let miss = vec![("e".to_string(), 0.0, 0.0, -6.0, 0.0)];
    assert!(sys.find_detonation_hits(&miss).is_empty());
    // distance 4 < 5 → hit.
    let hit = vec![("e".to_string(), 0.0, 0.0, -4.0, 0.0)];
    assert_eq!(
        sys.find_detonation_hits(&hit),
        vec![("t1".to_string(), "e".to_string())]
    );
}
