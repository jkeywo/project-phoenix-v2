//! Adapter tests for the debris hazard (issue #1347).
//!
//! The pure projection is covered in `threat.rs`; what is proved here is the
//! part only the adapter can be wrong about — that the rock moves, that a flag
//! rises exactly once, that an unassessed contact never becomes a threat however
//! obviously it is on course, and that a strike is judged on the simulation's
//! own geometry rather than on whether anybody was watching.

use super::*;
use crate::debris::threat::{DebrisAssessment, DebrisConfig};

const ROCK: &str = "rock-1";
const DEPOT_NAME: &str = "world.probe.entity.depot.name";

/// The authored table every test here starts from: a rock aimed straight at the
/// depot, arriving inside a 20-unit radius, urgent with 5 seconds to run.
fn config() -> DebrisConfig {
    DebrisConfig {
        drift: [-10.0, 0.0, 0.0],
        protected_target: DEPOT_NAME.into(),
        impact_radius: 20.0,
        urgent_secs: 5.0,
        // Read once. A cadence is exercised on its own below rather than left
        // running under every other assertion here.
        reassess_secs: 0.0,
        assessed_flag: "probe_rock_read".into(),
        confirmed_flag: "probe_rock_confirmed".into(),
        urgent_flag: "probe_rock_urgent".into(),
        impact_flag: "probe_rock_struck".into(),
    }
}

/// Advance the clock by exactly one second per `update()`.
///
/// These fixtures run the two systems in `Update` on a bare `App` with no
/// `TimePlugin`, so `Time` would otherwise never move and nothing would drift.
/// One second per step is deliberately coarse: every distance and deadline
/// below is then checkable by eye, which is what makes a failing assertion say
/// something.
fn advance_one_second(mut time: ResMut<Time>) {
    time.advance_by(std::time::Duration::from_secs(1));
}

/// One rock at `x`, one depot at the origin, and the two systems under test.
/// `Time` is advanced by exactly one second per `update()`, so a test can count
/// seconds instead of guessing at frames.
fn app_with(rock_x: f32) -> (App, Entity) {
    let mut app = App::new();
    app.add_systems(
        Update,
        (advance_one_second, tick_debris_drift, tick_debris_state).chain(),
    );
    app.init_resource::<crate::effect_queue::EffectQueue<DebrisAssessed>>();
    app.insert_resource(crate::world::server::WorldContentRuntime::default());
    app.insert_resource(crate::sim_tick::SimTick(0));
    app.insert_resource(Time::<()>::default());

    let rock = app
        .world_mut()
        .spawn((
            EntityUuid(ROCK.to_string()),
            Transform::from_xyz(rock_x, 0.0, 0.0),
            DebrisThreat::new(config()),
        ))
        .id();
    app.world_mut().spawn((
        EntityUuid("depot-1".to_string()),
        EntityName(DEPOT_NAME.to_string()),
        Transform::from_xyz(0.0, 0.0, 0.0),
    ));
    (app, rock)
}

/// Push the assessment a scan of this rock would have produced at `now`.
fn push_assessment(app: &mut App, seconds_to_impact: Option<f32>, on_course: bool) {
    let tick = app.world().resource::<crate::sim_tick::SimTick>().0;
    app.world_mut()
        .resource_mut::<crate::effect_queue::EffectQueue<DebrisAssessed>>()
        .0
        .push(DebrisAssessed {
            subject_uuid: ROCK.to_string(),
            taken_at_tick: tick,
            assessment: DebrisAssessment {
                protected_name: DEPOT_NAME.into(),
                course: [-10.0, 0.0],
                closest_approach: if on_course { 0.0 } else { 90.0 },
                seconds_to_closest_approach: 10.0,
                on_collision_course: on_course,
                seconds_to_impact,
            },
        });
}

fn flag(app: &App, name: &str) -> bool {
    app.world()
        .resource::<crate::world::server::WorldContentRuntime>()
        .flags
        .flag(name)
}

fn threat(app: &mut App, rock: Entity) -> DebrisThreat {
    app.world()
        .entity(rock)
        .get::<DebrisThreat>()
        .unwrap()
        .clone()
}

/// Step one second of simulation, keeping `SimTick` in step with it.
fn step(app: &mut App) {
    app.update();
    let mut tick = app.world_mut().resource_mut::<crate::sim_tick::SimTick>();
    tick.0 += 1;
}

#[test]
fn a_contact_drifts_at_its_authored_velocity() {
    let (mut app, rock) = app_with(200.0);
    step(&mut app);
    let x = app
        .world()
        .entity(rock)
        .get::<Transform>()
        .unwrap()
        .translation
        .x;
    assert!(
        (x - 190.0).abs() < 1e-3,
        "one second of -10 units/sec drift from 200, got {x}"
    );
}

#[test]
fn an_unassessed_contact_is_never_confirmed_however_plainly_it_is_on_course() {
    // The whole beat, in one assertion: this rock is aimed at the middle of the
    // depot and the simulation knows it. Nobody has looked, so it is not a
    // threat and no flag has moved.
    let (mut app, rock) = app_with(200.0);
    for _ in 0..5 {
        step(&mut app);
    }
    let t = threat(&mut app, rock);
    assert!(!t.assessed, "nobody scanned it");
    assert!(!t.confirmed, "and so it is not a confirmed threat");
    assert!(!flag(&app, "probe_rock_confirmed"));
}

#[test]
fn an_assessment_marks_the_contact_read_and_raises_the_authored_flag() {
    let (mut app, rock) = app_with(200.0);
    push_assessment(&mut app, None, false);
    step(&mut app);
    let t = threat(&mut app, rock);
    assert!(t.assessed, "the crew read it");
    assert!(
        !t.confirmed,
        "and what they read said it misses, so it is not a threat"
    );
    assert!(flag(&app, "probe_rock_read"));
    assert!(!flag(&app, "probe_rock_confirmed"));
}

#[test]
fn an_on_course_assessment_confirms_the_threat_once_and_only_once() {
    let (mut app, rock) = app_with(200.0);
    push_assessment(&mut app, Some(18.0), true);
    step(&mut app);
    assert!(flag(&app, "probe_rock_confirmed"));
    assert!(threat(&mut app, rock).confirmed);

    // A re-scan is an ordinary thing for a crew to do. It must not emit a second
    // world event for a bit that is already up — the rule `mirror_scanned` keeps.
    let before = app
        .world()
        .resource::<crate::world::server::WorldContentRuntime>()
        .pending_world_events
        .len();
    push_assessment(&mut app, Some(16.0), true);
    step(&mut app);
    let after = app
        .world()
        .resource::<crate::world::server::WorldContentRuntime>()
        .pending_world_events
        .len();
    assert_eq!(before, after, "a re-read raises nothing new");
}

#[test]
fn urgency_is_read_off_the_crews_own_dead_reckoned_number() {
    // Assessed with 8 seconds to run against a 5-second urgency window: not
    // urgent yet. Three seconds later, with no fresh scan at all, the crew's own
    // number has run down to 5 and the rung fires — which is the point of dead
    // reckoning rather than re-deriving.
    let (mut app, _rock) = app_with(200.0);
    push_assessment(&mut app, Some(8.0), true);
    step(&mut app);
    assert!(!flag(&app, "probe_rock_urgent"), "8 seconds is not yet 5");
    step(&mut app);
    step(&mut app);
    assert!(!flag(&app, "probe_rock_urgent"), "6 seconds is not yet 5");
    step(&mut app);
    assert!(flag(&app, "probe_rock_urgent"), "5 seconds is");
}

#[test]
fn a_strike_is_judged_on_geometry_whether_or_not_anybody_looked() {
    // 25 units out, closing at 10: it crosses the 20-unit radius on the first
    // step. No assessment is ever pushed — a scenario that only punished the
    // crews who looked would be teaching the wrong lesson.
    let (mut app, rock) = app_with(25.0);
    step(&mut app);
    assert!(flag(&app, "probe_rock_struck"));
    assert!(threat(&mut app, rock).struck);
}

#[test]
fn a_struck_contact_stops_drifting_and_does_not_arrive_twice() {
    let (mut app, rock) = app_with(25.0);
    step(&mut app);
    let x = app
        .world()
        .entity(rock)
        .get::<Transform>()
        .unwrap()
        .translation
        .x;
    let events = app
        .world()
        .resource::<crate::world::server::WorldContentRuntime>()
        .pending_world_events
        .len();
    step(&mut app);
    let x_after = app
        .world()
        .entity(rock)
        .get::<Transform>()
        .unwrap()
        .translation
        .x;
    assert_eq!(x, x_after, "a rock that has arrived does not keep going");
    assert_eq!(
        events,
        app.world()
            .resource::<crate::world::server::WorldContentRuntime>()
            .pending_world_events
            .len(),
        "and does not arrive twice"
    );
}

#[test]
fn a_contact_that_has_already_arrived_is_never_confirmed_by_a_later_reading() {
    // The rock lands unread on the first step, and only THEN does somebody scan
    // it. That reading is on a collision course and always will be: the mass is
    // frozen inside the very radius the arrival test uses, so the projection
    // answers `on_collision_course` with nought seconds to run — geometrically
    // true and operationally meaningless.
    //
    // Latching `confirmed` off it hands the scenario a threat to intercept for a
    // contact its own strike ending has already closed, out of order, after the
    // impact. A mass that has arrived is not a threat left to confirm.
    let (mut app, rock) = app_with(25.0);
    step(&mut app);
    assert!(
        threat(&mut app, rock).struck,
        "precondition: the unread mass has arrived"
    );

    push_assessment(&mut app, Some(0.0), true);
    step(&mut app);

    let t = threat(&mut app, rock);
    assert!(
        t.assessed,
        "the reading still lands — they looked, and they know"
    );
    assert!(flag(&app, "probe_rock_read"));
    assert!(
        !t.confirmed,
        "but what they read is a rock that is already down, not a threat"
    );
    assert!(!flag(&app, "probe_rock_confirmed"));
    assert!(
        !t.urgent,
        "and nothing that hangs off the confirmation follows"
    );
    assert!(!flag(&app, "probe_rock_urgent"));
}

#[test]
fn a_contact_whose_protected_asset_left_the_world_can_no_longer_strike() {
    let (mut app, _rock) = app_with(25.0);
    // The depot is gone — destroyed, or never spawned. There is nothing there
    // to hit, and saying so is more honest than striking an absence.
    let depot = app
        .world_mut()
        .query_filtered::<Entity, With<EntityName>>()
        .iter(app.world())
        .next()
        .unwrap();
    app.world_mut().entity_mut(depot).despawn();
    step(&mut app);
    assert!(!flag(&app, "probe_rock_struck"));
}

#[test]
fn a_table_that_names_no_flags_raises_none_and_still_ticks() {
    // The absent-authoring arm: a contact with no flag names is a hazard the
    // scenario watches some other way, and it must not panic or invent a name.
    let mut app = App::new();
    app.add_systems(
        Update,
        (advance_one_second, tick_debris_drift, tick_debris_state).chain(),
    );
    app.init_resource::<crate::effect_queue::EffectQueue<DebrisAssessed>>();
    app.insert_resource(crate::world::server::WorldContentRuntime::default());
    app.insert_resource(Time::<()>::default());
    let rock = app
        .world_mut()
        .spawn((
            EntityUuid(ROCK.to_string()),
            Transform::from_xyz(25.0, 0.0, 0.0),
            DebrisThreat::new(DebrisConfig {
                drift: [-10.0, 0.0, 0.0],
                protected_target: DEPOT_NAME.into(),
                impact_radius: 20.0,
                ..Default::default()
            }),
        ))
        .id();
    app.world_mut().spawn((
        EntityUuid("depot-1".to_string()),
        EntityName(DEPOT_NAME.to_string()),
        Transform::from_xyz(0.0, 0.0, 0.0),
    ));
    app.update();
    assert!(
        app.world()
            .entity(rock)
            .get::<DebrisThreat>()
            .unwrap()
            .struck
    );
    assert!(app
        .world()
        .resource::<crate::world::server::WorldContentRuntime>()
        .pending_world_events
        .is_empty());
}

#[test]
fn dead_reckoning_never_runs_a_deadline_past_zero() {
    let threat = DebrisThreat {
        config: config(),
        assessment: Some(DebrisAssessment {
            seconds_to_impact: Some(2.0),
            on_collision_course: true,
            ..Default::default()
        }),
        assessed_at_tick: 10,
        assessed: true,
        confirmed: true,
        ..Default::default()
    };
    assert_eq!(threat.seconds_to_impact_at(10, 1.0), Some(2.0));
    assert_eq!(threat.seconds_to_impact_at(11, 1.0), Some(1.0));
    assert_eq!(
        threat.seconds_to_impact_at(99, 1.0),
        Some(0.0),
        "a passed deadline reads zero, never a negative"
    );
}

#[test]
fn a_contact_nobody_assessed_has_no_deadline_to_reckon() {
    let threat = DebrisThreat::new(config());
    assert_eq!(threat.seconds_to_impact_at(100, 1.0), None);
}

// ── The re-assessment cadence (issue #1347) ─────────────────────────────────
//
// `needs_assessment` is what the Sensors seat works a debris field by, so the
// four answers it can give each get a test: never read, read and current, read
// and stale, and already landed.

#[test]
fn a_rock_nobody_has_read_always_wants_assessing() {
    let threat = DebrisThreat::new(config());
    assert!(threat.needs_assessment(0, 1.0));
    assert!(
        threat.needs_assessment(10_000, 1.0),
        "and goes on wanting it — an unread rock does not become less unknown"
    );
}

#[test]
fn a_rock_read_once_stops_asking_when_no_cadence_is_authored() {
    // The default `reassess_secs = 0` means "read once", and it is what makes a
    // seat move on down a field instead of admiring the rock it just finished.
    let mut threat = DebrisThreat::new(config());
    threat.assessed = true;
    threat.assessed_at_tick = 10;
    assert!(!threat.needs_assessment(10, 1.0));
    assert!(
        !threat.needs_assessment(10_000, 1.0),
        "no authored cadence means the reading never goes stale"
    );
}

#[test]
fn an_authored_cadence_brings_a_rock_back_round() {
    // 45 seconds at one second a tick: current at 44, stale at 45. The boundary
    // is inclusive because a reading exactly its own age old has expired.
    let mut cfg = config();
    cfg.reassess_secs = 45.0;
    let mut threat = DebrisThreat::new(cfg);
    threat.assessed = true;
    threat.assessed_at_tick = 100;
    assert!(!threat.needs_assessment(144, 1.0), "still current");
    assert!(threat.needs_assessment(145, 1.0), "stale, worth re-reading");
}

#[test]
fn a_struck_rock_never_wants_assessing_however_stale_its_reading() {
    // The struck test lives inside `needs_assessment` rather than at each call
    // site, so every consumer inherits it. There is nothing left to learn.
    let mut cfg = config();
    cfg.reassess_secs = 1.0;
    let mut threat = DebrisThreat::new(cfg);
    threat.assessed = true;
    threat.assessed_at_tick = 0;
    threat.struck = true;
    assert!(!threat.needs_assessment(10_000, 1.0));
}

#[test]
fn a_negative_cadence_fails_closed_to_read_once() {
    // `validate` refuses this at load, so it is unreachable through content.
    // What is pinned is the direction it fails IF it ever arrived: silence,
    // rather than a contact re-scanning itself every tick forever.
    let mut cfg = config();
    cfg.reassess_secs = -5.0;
    let mut threat = DebrisThreat::new(cfg);
    threat.assessed = true;
    assert!(!threat.needs_assessment(10_000, 1.0));
    assert!(cfg_is_refused(-5.0), "and content-load refuses it outright");
}

fn cfg_is_refused(reassess_secs: f32) -> bool {
    let mut cfg = config();
    cfg.reassess_secs = reassess_secs;
    cfg.validate().is_err()
}
