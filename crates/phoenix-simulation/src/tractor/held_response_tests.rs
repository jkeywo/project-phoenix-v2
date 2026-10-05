use super::*;
use crate::infrastructure::condition::{
    InfrastructureConfig, InfrastructureState, ThresholdConfig,
};

fn cfg(kind: HeldResponseKind) -> HeldResponseConfig {
    HeldResponseConfig {
        kind,
        recover_per_sec: None,
        offset: None,
        distance: None,
    }
}

/// A failing structure that loses 6 condition points a second and whose
/// `holding` flag falls below 40 % and returns at 60 %.
fn failing_structure(start: f32) -> InfrastructureState {
    InfrastructureState::from_config(&InfrastructureConfig {
        condition_max: 100.0,
        condition: Some(start),
        decay_per_sec: 6.0,
        thresholds: vec![ThresholdConfig {
            label: None,
            flag: "holding".to_string(),
            capacity: None,
            fails_below: 0.4,
            restores_above: Some(0.6),
        }],
        ..Default::default()
    })
}

/// One ordinary (unheld) tick: the infra tick's automatic decline.
fn decline_one_tick(state: &mut InfrastructureState, dt: f32) {
    let decay = state.decay_per_sec() * dt;
    if decay > 0.0 {
        state.degrade(decay);
    }
}

/// One HELD tick: the infra tick's automatic decline THEN the held-response
/// adjustment, exactly as `tick_infrastructure_condition` applies them (decay
/// first, queued adjustments second) in one tick.
fn held_one_tick(state: &mut InfrastructureState, response: &HeldResponse, dt: f32) {
    let decay_per_sec = state.decay_per_sec();
    decline_one_tick(state, dt);
    let delta = condition_delta(response, decay_per_sec, dt);
    state.apply_delta(delta);
}

// ── arrest-decline against the condition track ───────────────────────────

#[test]
fn arrest_decline_holds_a_failing_structure_steady_at_a_zero_rate() {
    // recover_per_sec = 0: the decline is cancelled and nothing added, so a
    // structure that would lose six points a second holds exactly still.
    let response = HeldResponse::ArrestDecline {
        recover_per_sec: 0.0,
    };
    let mut state = failing_structure(50.0);
    for _ in 0..120 {
        held_one_tick(&mut state, &response, 1.0 / 60.0);
    }
    assert!(
        (state.condition() - 50.0).abs() < 0.5,
        "two held seconds must arrest the decline, leaving the structure where it started \
             (would be 50 - 12 = 38 unheld), got {}",
        state.condition()
    );
}

#[test]
fn arrest_decline_recovers_at_the_authored_rate_and_crosses_the_targets_own_threshold() {
    // Start below the failure point so the flag is down, then hold and
    // recover at a rate that carries it up across the authored restore point.
    let response = HeldResponse::ArrestDecline {
        recover_per_sec: 20.0,
    };
    let mut state = failing_structure(37.0);
    assert_eq!(
        state.flag("holding"),
        Some(false),
        "precondition: 37 % starts below the 40 % failure point, flag down"
    );
    let mut crossings = 0;
    for _ in 0..120 {
        let before = state.decay_per_sec() * (1.0 / 60.0);
        decline_one_tick(&mut state, 1.0 / 60.0);
        let _ = before;
        let delta = condition_delta(&response, 6.0, 1.0 / 60.0);
        crossings += state.apply_delta(delta).len();
    }
    // Net +20/s for two seconds off 37 → ~77, clamped under the ceiling.
    assert!(
        (state.condition() - 77.0).abs() < 1.0,
        "arrest-decline nets the authored +20/s over the arrested decline, got {}",
        state.condition()
    );
    assert_eq!(
        state.flag("holding"),
        Some(true),
        "…and the recovered condition crossed the target's own 60 % restore point, setting \
             the operational flag a scenario reads"
    );
    assert_eq!(
        crossings, 1,
        "the crossing is reported exactly once, on the tick it carries over the restore point"
    );
}

#[test]
fn releasing_arrest_decline_resumes_the_ordinary_decline_on_the_next_tick() {
    let response = HeldResponse::ArrestDecline {
        recover_per_sec: 0.0,
    };
    let mut state = failing_structure(50.0);
    for _ in 0..60 {
        held_one_tick(&mut state, &response, 1.0 / 60.0);
    }
    let held_value = state.condition();
    assert!(
        (held_value - 50.0).abs() < 0.5,
        "held for a second it stayed put, got {held_value}"
    );
    // Release: no more held ticks, only the ordinary decline.
    for _ in 0..60 {
        decline_one_tick(&mut state, 1.0 / 60.0);
    }
    assert!(
        state.condition() < held_value - 5.0,
        "a released structure resumes its ordinary decline — six points off in the second \
             after release, got a drop from {held_value} to {}",
        state.condition()
    );
}

// ── follow / station-keep leave the condition track alone ────────────────

#[test]
fn follow_and_station_keep_do_not_touch_the_condition_track() {
    for response in [HeldResponse::Follow, HeldResponse::StationKeep] {
        assert_eq!(
            condition_delta(&response, 6.0, 1.0 / 60.0),
            0.0,
            "{response:?} banks no condition — holding a derelict or station-keeping a craft \
                 is a geometry response, not a condition one"
        );
        // A structure held under one of these goes on declining ordinarily:
        // the response arrests nothing.
        let mut state = failing_structure(50.0);
        for _ in 0..120 {
            held_one_tick(&mut state, &response, 1.0 / 60.0);
        }
        assert!(
            (state.condition() - 38.0).abs() < 0.5,
            "{response:?} does not arrest the decline: 50 - 12 = 38, got {}",
            state.condition()
        );
    }
}

// ── formation-keep is a geometry response, distinct from station-keep ─────

#[test]
fn formation_keep_banks_no_condition_but_rides_its_own_authored_slot() {
    let response = HeldResponse::FormationKeep {
        offset: Vec3::new(0.0, 0.0, 1.0),
        distance: 200.0,
    };
    assert_eq!(
        condition_delta(&response, 6.0, 1.0 / 60.0),
        0.0,
        "formation-keep is a geometry response and leaves the condition track alone"
    );
    // Same declining structure, held in formation: it keeps declining.
    let mut state = failing_structure(50.0);
    for _ in 0..120 {
        held_one_tick(&mut state, &response, 1.0 / 60.0);
    }
    assert!(
        (state.condition() - 38.0).abs() < 0.5,
        "formation-keep arrests nothing, got {}",
        state.condition()
    );
}

#[test]
fn formation_keep_rides_its_own_slot_where_the_others_ride_the_operator_rig() {
    let operator_rig = Vec3::new(0.0, 0.0, -120.0);
    // Station-keep / follow / arrest-decline all ride the operator's rig.
    for response in [
        HeldResponse::StationKeep,
        HeldResponse::Follow,
        HeldResponse::ArrestDecline {
            recover_per_sec: 3.0,
        },
    ] {
        assert_eq!(
            held_offset(&response, operator_rig),
            operator_rig,
            "{response:?} rides the operator's authored coupling rig"
        );
    }
    // Formation-keep rides its OWN slot: 200 units along +Z, distinct from
    // the operator's 120-astern rig.
    let formation = HeldResponse::FormationKeep {
        offset: Vec3::new(0.0, 0.0, 5.0),
        distance: 200.0,
    };
    let slot = held_offset(&formation, operator_rig);
    assert!(
        (slot - Vec3::new(0.0, 0.0, 200.0)).length() < 1e-3,
        "formation-keep rides 200 units along its (un-normalised) +Z bearing, got {slot:?}"
    );
    assert_ne!(
        slot, operator_rig,
        "…which is distinct from station-keeping it in place on the operator's rig"
    );
}

// ── config validation ────────────────────────────────────────────────────

#[test]
fn arrest_decline_requires_a_recover_rate_and_forbids_formation_fields() {
    let mut c = cfg(HeldResponseKind::ArrestDecline);
    assert!(
        c.validate().is_err(),
        "arrest-decline with no recover_per_sec is a load error"
    );
    c.recover_per_sec = Some(5.0);
    c.validate().expect("arrest-decline with a rate is valid");
    c.offset = Some([0.0, 0.0, 1.0]);
    assert!(
        c.validate().is_err(),
        "a formation offset on an arrest-decline is a load error"
    );
}

#[test]
fn formation_keep_requires_a_non_zero_offset_and_a_positive_distance() {
    let mut c = cfg(HeldResponseKind::FormationKeep);
    assert!(
        c.validate().is_err(),
        "no offset, no distance: a load error"
    );
    c.offset = Some([0.0, 0.0, 0.0]);
    c.distance = Some(200.0);
    assert!(
        c.validate().is_err(),
        "a zero-length bearing would hold the target on its operator"
    );
    c.offset = Some([0.0, 0.0, 1.0]);
    c.distance = Some(0.0);
    assert!(c.validate().is_err(), "a non-positive distance is rejected");
    c.distance = Some(200.0);
    c.validate().expect("a real bearing and distance validate");
    c.recover_per_sec = Some(1.0);
    assert!(
        c.validate().is_err(),
        "a recover rate on a formation-keep is a load error"
    );
}

#[test]
fn follow_and_station_keep_forbid_every_per_kind_field() {
    for kind in [HeldResponseKind::Follow, HeldResponseKind::StationKeep] {
        let mut c = cfg(kind);
        c.validate().expect("a bare follow/station-keep validates");
        c.recover_per_sec = Some(1.0);
        assert!(
            c.validate().is_err(),
            "{kind:?} authors no recover_per_sec — that belongs to arrest-decline"
        );
        let mut c = cfg(kind);
        c.offset = Some([0.0, 0.0, 1.0]);
        assert!(
            c.validate().is_err(),
            "{kind:?} authors no formation offset"
        );
    }
}

#[test]
fn the_vocabulary_round_trips_through_toml() {
    let authored = r#"
kind = "arrest-decline"
recover_per_sec = 8.0
"#;
    let parsed: HeldResponseConfig = toml::from_str(authored).expect("arrest-decline parses");
    assert_eq!(parsed.kind, HeldResponseKind::ArrestDecline);
    assert_eq!(parsed.recover_per_sec, Some(8.0));
    parsed.validate().expect("valid");

    let formation = r#"
kind = "formation-keep"
offset = [0.0, 0.0, 60.0]
distance = 60.0
"#;
    let parsed: HeldResponseConfig = toml::from_str(formation).expect("formation-keep parses");
    assert_eq!(parsed.kind, HeldResponseKind::FormationKeep);
    assert_eq!(parsed.offset, Some([0.0, 0.0, 60.0]));
    assert_eq!(parsed.distance, Some(60.0));
    parsed.validate().expect("valid");

    // The bare vocabulary — a target that says only which response it wants.
    let station = toml::from_str::<HeldResponseConfig>("kind = \"station-keep\"")
        .expect("station-keep parses");
    assert_eq!(station.kind, HeldResponseKind::StationKeep);
    station.validate().expect("valid");
}

#[test]
fn an_unknown_field_is_a_parse_error_rather_than_a_silently_ignored_typo() {
    let err = toml::from_str::<HeldResponseConfig>("kind = \"follow\"\nrecovr_per_sec = 5.0")
        .expect_err("a misspelt field must not be swallowed");
    assert!(
        err.to_string().contains("recovr_per_sec"),
        "the error must name the offending field, got {err}"
    );
}
