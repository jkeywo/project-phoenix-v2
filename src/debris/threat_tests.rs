use super::*;

/// A rock 100 units up-track from a depot, closing at 10 units/sec straight
/// down the line: it arrives, and the arithmetic is checkable by eye.
fn head_on() -> DebrisSubject {
    DebrisSubject {
        relative_position: [100.0, 0.0],
        relative_velocity: [-10.0, 0.0],
        protected_name: "world.probe.entity.depot.name".into(),
        impact_radius: 20.0,
    }
}

#[test]
fn a_head_on_contact_is_confirmed_and_reports_its_arrival() {
    let a = assess(&head_on());
    assert!(a.on_collision_course, "it is aimed at the middle of it");
    assert_eq!(a.closest_approach, 0.0);
    assert_eq!(a.seconds_to_closest_approach, 10.0);
    // The IMPACT is the radius crossing at 80 units, not the centre at 100.
    assert_eq!(a.seconds_to_impact, Some(8.0));
    assert_eq!(a.course, [-10.0, 0.0]);
    assert_eq!(a.protected_name, "world.probe.entity.depot.name");
}

#[test]
fn a_contact_that_passes_wide_is_not_a_threat_however_close_it_comes() {
    // Same closing speed, offset 30 units across the depot's beam — outside
    // the authored 20-unit radius, so it misses. The projection still says
    // exactly how near and exactly when, which is the finding.
    let mut subject = head_on();
    subject.relative_position = [100.0, 30.0];
    let a = assess(&subject);
    assert!(!a.on_collision_course);
    assert_eq!(a.closest_approach, 30.0);
    assert_eq!(a.seconds_to_closest_approach, 10.0);
    assert_eq!(
        a.seconds_to_impact, None,
        "no arrival exists, and 0.0 must never stand in for that"
    );
}

#[test]
fn a_contact_already_drawing_away_reports_its_present_separation() {
    let mut subject = head_on();
    subject.relative_velocity = [10.0, 0.0];
    let a = assess(&subject);
    assert_eq!(
        a.seconds_to_closest_approach, 0.0,
        "the vertex is behind it"
    );
    assert_eq!(a.closest_approach, 100.0);
    assert!(!a.on_collision_course);
}

#[test]
fn a_stationary_contact_outside_the_radius_never_arrives() {
    let mut subject = head_on();
    subject.relative_velocity = [0.0, 0.0];
    let a = assess(&subject);
    assert_eq!(a.closest_approach, 100.0);
    assert_eq!(a.seconds_to_closest_approach, 0.0);
    assert!(!a.on_collision_course);
    assert_eq!(a.seconds_to_impact, None);
}

#[test]
fn a_contact_already_inside_the_radius_is_arriving_now() {
    let mut subject = head_on();
    subject.relative_position = [10.0, 0.0];
    let a = assess(&subject);
    assert!(a.on_collision_course);
    assert_eq!(a.seconds_to_impact, Some(0.0));
}

#[test]
fn an_unprotected_contact_is_never_on_a_collision_course() {
    // A field of harmless wreckage: no protected asset, so no radius, so
    // nothing to confirm — but the projection still reads.
    let subject = DebrisSubject {
        relative_position: [100.0, 0.0],
        relative_velocity: [-10.0, 0.0],
        protected_name: String::new(),
        impact_radius: 0.0,
    };
    let a = assess(&subject);
    assert!(!a.on_collision_course);
    assert_eq!(a.seconds_to_impact, None);
    assert_eq!(a.closest_approach, 0.0);
}

#[test]
fn the_assessment_is_reproducible_from_the_same_inputs() {
    // The lockstep claim, at the pure boundary: the same subject twice is
    // the same bytes twice.
    let subject = head_on();
    assert_eq!(assess(&subject), assess(&subject));
}

#[test]
fn a_protected_contact_with_no_impact_radius_is_refused_at_load() {
    let cfg = DebrisConfig {
        protected_target: "world.probe.entity.depot.name".into(),
        impact_radius: 0.0,
        ..Default::default()
    };
    let err = cfg.validate().expect_err("a hazard that cannot arrive");
    assert!(err.contains("can never arrive"), "got: {err}");
}

#[test]
fn an_unprotected_table_validates_without_a_radius() {
    let cfg = DebrisConfig {
        drift: [1.0, 0.0, 2.0],
        ..Default::default()
    };
    assert!(cfg.validate().is_ok());
}

#[test]
fn a_non_finite_drift_is_refused_at_load() {
    let cfg = DebrisConfig {
        drift: [f32::NAN, 0.0, 0.0],
        ..Default::default()
    };
    assert!(cfg.validate().is_err());
}
