use super::*;

#[test]
fn the_cutoff_is_two_simulation_seconds_at_whatever_rate_the_world_runs() {
    assert_eq!(exposure_limit_ticks(60.0), 120);
    assert_eq!(exposure_limit_ticks(30.0), 60);
    assert_eq!(exposure_limit_ticks(240.0), 480);
    // Never zero: a degenerate rate must not refuse every undo outright.
    assert_eq!(exposure_limit_ticks(0.0), 120);
    assert_eq!(exposure_limit_ticks(f32::NAN), 120);
}

#[test]
fn overlapping_ships_count_one_step_not_several() {
    let point = Vec3::new(10.0, 0.0, 0.0);
    let two = [
        (Vec3::ZERO, 100.0),
        (Vec3::new(20.0, 0.0, 0.0), 100.0),
        (Vec3::new(900.0, 0.0, 0.0), 5.0),
    ];
    let mut both = GmSpawnExposure::default();
    both.watch("gm_raider_1");
    both.observe("gm_raider_1", exposed(point, &two), 120);
    let mut one = GmSpawnExposure::default();
    one.watch("gm_raider_1");
    one.observe("gm_raider_1", exposed(point, &two[..1]), 120);
    assert_eq!(both.get("gm_raider_1"), one.get("gm_raider_1"));
    assert_eq!(both.get("gm_raider_1").unwrap().ticks, 1);
}

#[test]
fn leaving_the_range_holds_the_count_instead_of_resetting_it() {
    let mut exposure = GmSpawnExposure::default();
    exposure.watch("gm_raider_1");
    for _ in 0..40 {
        exposure.observe("gm_raider_1", true, 120);
    }
    for _ in 0..500 {
        exposure.observe("gm_raider_1", false, 120);
    }
    assert_eq!(exposure.get("gm_raider_1").unwrap().ticks, 40);
    assert_eq!(
        exposure.eligibility("gm_raider_1"),
        GmSpawnUndoEligibility::Eligible
    );
    for _ in 0..80 {
        exposure.observe("gm_raider_1", true, 120);
    }
    assert_eq!(
        exposure.eligibility("gm_raider_1"),
        GmSpawnUndoEligibility::Exposed,
        "cumulative, not continuous: two separate spells reach the cutoff"
    );
}

#[test]
fn the_latch_never_re_opens() {
    let mut exposure = GmSpawnExposure::default();
    exposure.watch("gm_raider_1");
    for _ in 0..120 {
        exposure.observe("gm_raider_1", true, 120);
    }
    assert!(exposure.get("gm_raider_1").unwrap().latched);
    for _ in 0..10_000 {
        exposure.observe("gm_raider_1", false, 120);
    }
    assert_eq!(exposure.get("gm_raider_1").unwrap().ticks, 120);
    assert!(exposure.get("gm_raider_1").unwrap().latched);
}

#[test]
fn re_arming_a_watched_name_never_restarts_its_clock() {
    let mut exposure = GmSpawnExposure::default();
    exposure.watch("gm_raider_1");
    for _ in 0..119 {
        exposure.observe("gm_raider_1", true, 120);
    }
    exposure.watch("gm_raider_1");
    assert_eq!(exposure.get("gm_raider_1").unwrap().ticks, 119);
    assert!(!exposure.get("gm_raider_1").unwrap().latched);
}

#[test]
fn an_unwatched_placement_is_not_reported_as_exposed() {
    let exposure = GmSpawnExposure::default();
    assert_eq!(
        exposure.eligibility("gm_raider_9"),
        GmSpawnUndoEligibility::Unwatched
    );
}

#[test]
fn the_published_status_reads_in_milliseconds_of_the_stated_cutoff() {
    let status = GmSpawnExposureStatus::new(
        SpawnExposure {
            ticks: 60,
            latched: false,
        },
        60.0,
    );
    assert_eq!(status.exposed_ms, 1000);
    assert_eq!(status.limit_ms, 2000);
    assert!(!status.latched);
    let closed = GmSpawnExposureStatus::new(
        SpawnExposure {
            ticks: 120,
            latched: true,
        },
        60.0,
    );
    assert_eq!(closed.exposed_ms, 2000);
    assert!(closed.latched);
}
