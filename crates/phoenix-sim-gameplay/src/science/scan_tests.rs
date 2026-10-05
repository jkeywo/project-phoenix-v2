use super::*;

/// The mirror flag's spelling is a **contract with scenario authors** —
/// `falling_skyway.toml` and `probe_scandiff.toml` write
/// `on_flag_set("scan.depot_ladder_b.taken", …)` against it — so it is
/// pinned here rather than left to be re-read out of a `format!`.
#[test]
fn the_scanned_flag_is_named_after_the_subject_it_was_read_from() {
    assert_eq!(scanned_flag("depot_ladder_b"), "scan.depot_ladder_b.taken");
    assert_ne!(
        scanned_flag("depot_ladder_a"),
        scanned_flag("depot_ladder_b"),
        "two structures do not share one 'somebody scanned something' bit"
    );
}

fn band(id: &str, max_range: f32, step: f32) -> ScanBandConfig {
    ScanBandConfig {
        id: id.to_string(),
        label: format!("world.probe.band.{id}.label"),
        max_range,
        condition_step: step,
        report_thresholds: true,
        report_capacities: true,
    }
}

/// A two-rung ladder: whole percent inside 500 units, quarters out to 3000.
fn suite() -> ScanConfig {
    ScanConfig {
        power_group: "shields".into(),
        min_power_level: 2,
        bands: vec![
            band("detailed", 500.0, 0.01),
            ScanBandConfig {
                report_capacities: false,
                ..band("coarse", 3000.0, 0.25)
            },
        ],
        degraded_by: vec![RegionEffectName::NebulaFog],
        interference_bands: 1,
        mass_classes: Vec::new(),
    }
}

/// The same suite with a three-rung bulk ladder (issue #1347), so a reading
/// reports a class as well as a number.
fn suite_with_mass_classes() -> ScanConfig {
    ScanConfig {
        mass_classes: vec![
            ScanMassClassConfig {
                id: "light".into(),
                label: "world.probe.mass_class.light.label".into(),
                max_mass: 1_000.0,
            },
            ScanMassClassConfig {
                id: "medium".into(),
                label: "world.probe.mass_class.medium.label".into(),
                max_mass: 50_000.0,
            },
            ScanMassClassConfig {
                id: "heavy".into(),
                label: "world.probe.mass_class.heavy.label".into(),
                max_mass: 200_000.0,
            },
        ],
        ..suite()
    }
}

fn depot(fraction: f32) -> ScanSubject {
    ScanSubject {
        uuid: "depot-1".into(),
        name: "world.probe.entity.depot.name".into(),
        condition: Some(SubjectCondition {
            condition_fraction: fraction,
            flags: vec![("world.probe.threshold.transfer.label".into(), false)],
            capacities: vec![("world.probe.capacity.berths.label".into(), 4)],
        }),
        mass: 180_000.0,
        debris: None,
    }
}

fn at(distance: f32) -> ScanConditions {
    ScanConditions {
        distance,
        power_level: 2,
        power_locked: false,
        region_effects: Vec::new(),
    }
}

/// **AC2, the derivation-purity claim, as a property of one function.**
///
/// The same subject at the same range, scanned twice with a different
/// condition in between, reads out differently — and nothing about the
/// config, the labels or this test changed to make it happen.
#[test]
fn moving_the_subjects_condition_moves_the_reading_with_no_other_input_changed() {
    let config = suite();
    let before = derive(&config, &depot(0.82), &at(100.0), 10).expect("a reading");
    let after = derive(&config, &depot(0.31), &at(100.0), 20).expect("a reading");

    assert_eq!(before.condition_fraction, 0.82);
    assert_eq!(after.condition_fraction, 0.31);
    assert_ne!(
        before.condition_fraction, after.condition_fraction,
        "the readout is a derivation of the track, not a string chosen for it"
    );
    assert_eq!(
        (before.band.as_str(), after.band.as_str()),
        ("detailed", "detailed"),
        "…and everything else about the two scans is identical"
    );
}

/// **AC5.** The same structure, the same power, the same weather — twice as
/// far out and the reading is measurably coarser: a rounder number, and the
/// capacity list withdrawn because the authored band says so.
#[test]
fn a_distant_scan_reads_coarser_than_a_close_one_off_the_authored_bands() {
    let config = suite();
    let close = derive(&config, &depot(0.37), &at(400.0), 1).expect("a reading");
    let far = derive(&config, &depot(0.37), &at(2_400.0), 1).expect("a reading");

    assert_eq!(close.band, "detailed");
    assert_eq!(close.condition_fraction, 0.37);
    assert_eq!(close.capacities.len(), 1);

    assert_eq!(far.band, "coarse");
    assert_eq!(
        far.condition_fraction, 0.25,
        "0.37 rounded to the coarse band's authored quarter-steps"
    );
    assert!(
        far.capacities.is_empty(),
        "the coarse band authored report_capacities = false, so the berth count is not \
             something this reading claims to know"
    );
    assert_eq!(
        far.flags.len(),
        1,
        "…while the flags it DOES author still come through"
    );
    assert_eq!(far.condition_step, 0.25, "and it says how precise it is");
}

/// Issue #1154: mass is content identity, not a live measurement, so
/// unlike `condition_fraction` the fidelity ladder never touches it — a
/// coarse reading from 2,400 units out reports the exact same mass as a
/// detailed one from 400.
#[test]
fn mass_rides_through_the_reading_unrounded_at_every_fidelity() {
    let config = suite();
    let close = derive(&config, &depot(0.37), &at(400.0), 1).expect("a reading");
    let far = derive(&config, &depot(0.37), &at(2_400.0), 1).expect("a reading");

    assert_eq!(close.mass, 180_000.0);
    assert_eq!(
        far.mass, 180_000.0,
        "mass does not get coarser with range the way condition_fraction does"
    );
}

// ── The bulk ladder and the debris projection (issue #1347) ─────────────

/// A suite that authors no `[[scan.mass_class]]` ladder reports no class —
/// which is every hull shipped before #1347, so their readings are byte-for-
/// byte what they were.
#[test]
fn a_suite_with_no_bulk_ladder_reports_no_class_at_all() {
    let reading = derive(&suite(), &depot(0.5), &at(400.0), 1).expect("a reading");
    assert!(reading.mass_class.is_empty());
    assert!(reading.mass_class_label.is_empty());
}

/// The class is a projection of the subject's own mass through the SUITE's
/// ladder — lightest first, first covering class wins.
#[test]
fn the_bulk_class_is_the_lightest_authored_class_that_still_covers_the_mass() {
    let config = suite_with_mass_classes();
    let reading = derive(&config, &depot(0.5), &at(400.0), 1).expect("a reading");
    assert_eq!(
        reading.mass_class, "heavy",
        "180,000 is past medium's 50,000 ceiling and inside heavy's 200,000"
    );
    assert_eq!(
        reading.mass_class_label,
        "world.probe.mass_class.heavy.label"
    );

    let mut pebble = depot(0.5);
    pebble.mass = 40.0;
    assert_eq!(
        derive(&config, &pebble, &at(400.0), 1)
            .expect("a reading")
            .mass_class,
        "light"
    );
}

/// The top rung is OPEN: something heavier than every authored ceiling is
/// still reported in the heaviest class the suite has a word for, rather
/// than falling out of the ladder into silence.
#[test]
fn a_subject_heavier_than_every_class_reports_the_heaviest_one() {
    let config = suite_with_mass_classes();
    let mut behemoth = depot(0.5);
    behemoth.mass = 9_000_000.0;
    assert_eq!(
        derive(&config, &behemoth, &at(400.0), 1)
            .expect("a reading")
            .mass_class,
        "heavy"
    );
}

/// Like `mass`, the class does not coarsen with range: it is content
/// identity read through a fixed ladder, not a live measurement.
#[test]
fn the_bulk_class_is_the_same_answer_from_any_band() {
    let config = suite_with_mass_classes();
    let close = derive(&config, &depot(0.37), &at(400.0), 1).expect("a reading");
    let far = derive(&config, &depot(0.37), &at(2_400.0), 1).expect("a reading");
    assert_eq!(close.mass_class, far.mass_class);
    assert_ne!(
        close.condition_step, far.condition_step,
        "…while the fidelity that DOES coarsen still did"
    );
}

/// A bulk ladder whose ceilings do not strictly increase is refused at load,
/// on the band ladder's argument: the class behind the flat one could never
/// be reached.
#[test]
fn a_bulk_ladder_that_does_not_ascend_is_refused_at_load() {
    let mut config = suite_with_mass_classes();
    config.mass_classes[2].max_mass = 50_000.0;
    let err = config.validate().expect_err("an unreachable class");
    assert!(err.contains("LIGHTEST FIRST"), "got: {err}");
}

/// The debris projection rides the ordinary reading, and rides it only when
/// the subject IS debris (issue #1347).
#[test]
fn an_ordinary_subject_carries_no_debris_projection() {
    let reading = derive(&suite(), &depot(0.5), &at(400.0), 1).expect("a reading");
    assert!(
        reading.debris.is_none(),
        "a depot is not going to hit anything"
    );
}

#[test]
fn a_debris_subject_folds_its_projection_into_the_ordinary_reading() {
    let mut rock = depot(0.5);
    rock.mass = 4_200.0;
    rock.debris = Some(crate::debris::DebrisSubject {
        relative_position: [100.0, 0.0],
        relative_velocity: [-10.0, 0.0],
        protected_name: "world.probe.entity.depot.name".into(),
        impact_radius: 20.0,
    });
    let reading = derive(&suite(), &rock, &at(400.0), 1).expect("a reading");
    let assessment = reading.debris.expect("a debris reading projects");
    assert!(assessment.on_collision_course);
    assert_eq!(assessment.seconds_to_impact, Some(8.0));
    assert_eq!(
        assessment.protected_name, "world.probe.entity.depot.name",
        "the reading names what is UNDER the rock, and names it by the id the \
             author wrote against the depot rather than against the threat"
    );
}

/// A mass aimed at nothing still comes back a READING, and the reading is
/// what says so (issue #1347).
///
/// This is the finding the whole beat turns on: "we looked and there is
/// nothing under it" has to be a different state from "nobody has been", and
/// the only thing that can carry the difference is a projection that answers.
/// A `None` here would leave a crew who ruled a contact out indistinguishable
/// from a crew who never went, and would leave the Sensors seat asking for
/// the same rock forever.
#[test]
fn a_mass_aimed_at_nothing_still_reads_as_a_finding() {
    let mut rock = depot(0.5);
    rock.debris = Some(crate::debris::DebrisSubject {
        relative_position: [0.0, 0.0],
        relative_velocity: [-2.2, 0.9],
        protected_name: String::new(),
        impact_radius: 0.0,
    });
    let reading = derive(&suite(), &rock, &at(400.0), 1).expect("a reading");
    let assessment = reading
        .debris
        .expect("a hazard the crew read must come back with an answer");
    assert!(
        !assessment.on_collision_course,
        "there is no radius to cross, so nothing to confirm"
    );
    assert_eq!(assessment.seconds_to_impact, None);
    assert_eq!(
        assessment.protected_name, "",
        "and the reading names no asset, because there is none"
    );
    assert_eq!(
        assessment.course,
        [-2.2, 0.9],
        "what it is DOING is a fact about the contact and survives having \
             nothing to do it to"
    );
}

/// A refused scan reveals nothing about a hazard. The gate that matters for
/// debris is RANGE, and it is the ladder's existing one: a crew told "out of
/// range" have not learned what the rock is aimed at.
#[test]
fn a_refused_scan_of_debris_reveals_no_projection() {
    let mut rock = depot(0.5);
    rock.debris = Some(crate::debris::DebrisSubject {
        relative_position: [100.0, 0.0],
        relative_velocity: [-10.0, 0.0],
        protected_name: "world.probe.entity.depot.name".into(),
        impact_radius: 20.0,
    });
    assert_eq!(
        derive(&suite(), &rock, &at(9_000.0), 1),
        Err(ScanRefusal::OutOfRange),
        "and a refusal carries no reading to hide a projection inside"
    );
}

/// Past the coarsest band there is no reading at all — the ladder has an
/// end, and it is the authored one.
#[test]
fn past_the_last_bands_reach_the_scan_is_refused_as_out_of_range() {
    assert_eq!(
        derive(&suite(), &depot(0.5), &at(3_000.1), 1),
        Err(ScanRefusal::OutOfRange)
    );
    assert!(
        derive(&suite(), &depot(0.5), &at(3_000.0), 1).is_ok(),
        "and the boundary itself still answers — max_range is inclusive"
    );
}

/// **AC1's refusal half.** A target with no condition track is refused with
/// a reason rather than answered with an empty readout.
#[test]
fn a_target_with_no_condition_track_is_refused_with_a_reason() {
    let rock = ScanSubject {
        uuid: "rock-7".into(),
        name: String::new(),
        condition: None,
        mass: 500.0,
        debris: None,
    };
    assert_eq!(
        derive(&suite(), &rock, &at(100.0), 1),
        Err(ScanRefusal::NoReadableCondition)
    );
}

/// **The leak rule, restated for this module.** A structure the scenario
/// keeps off the wire cannot be built into a subject that has a condition,
/// because the only constructor for one is #1025's publish gate — so the
/// scan refuses it, and refuses it with the SAME reason a bare rock gets.
///
/// The identical reason is the load-bearing half: a refusal that said
/// "withheld" would leak the existence of the secret to anyone who scanned.
#[test]
fn a_withheld_condition_track_cannot_be_scanned_and_does_not_announce_itself() {
    use crate::infrastructure::{InfrastructureConfig, InfrastructureState};

    let hidden = InfrastructureState::from_config(&InfrastructureConfig {
        condition_max: 100.0,
        condition: Some(31.0),
        publish: false,
        ..InfrastructureConfig::default()
    });
    assert!(
        crate::core::messages::infrastructure_snapshot_from_state(&hidden).is_none(),
        "the publish gate is #1025's, and this derivation is downstream of it"
    );

    let sealed = ScanSubject {
        uuid: "sealed-1".into(),
        name: "world.probe.entity.sealed.name".into(),
        condition: crate::core::messages::infrastructure_snapshot_from_state(&hidden)
            .as_ref()
            .map(|published| SubjectCondition::from_published(published, |_| None, |_| None)),
        mass: 90_000.0,
        debris: None,
    };
    let refusal = derive(&suite(), &sealed, &at(100.0), 1).expect_err("no reading");
    assert_eq!(
        refusal,
        ScanRefusal::NoReadableCondition,
        "the same answer an unreadable rock gets — a scan that distinguished the two \
             would betray the secret by the shape of its error"
    );
}

/// Power is a live gate, and it refuses rather than degrading: a suite
/// under its authored minimum is not returning a worse answer, it is
/// returning none.
#[test]
fn a_suite_below_its_authored_power_level_or_behind_a_locked_grid_returns_nothing() {
    let config = suite();
    let mut brownout = at(100.0);
    brownout.power_level = 1;
    assert_eq!(
        derive(&config, &depot(0.5), &brownout, 1),
        Err(ScanRefusal::Underpowered)
    );

    let mut locked = at(100.0);
    locked.power_locked = true;
    assert_eq!(
        derive(&config, &depot(0.5), &locked, 1),
        Err(ScanRefusal::Underpowered),
        "the exhaustion lock is the same answer as being under the floor"
    );
}

/// Interference drops the reading a band, out of authored data on both
/// ends: the effect names the hull declared, and the number of steps it
/// declared. An effect the hull did NOT declare changes nothing.
#[test]
fn standing_in_an_authored_interference_effect_drops_the_reading_a_band() {
    let config = suite();
    let mut fogged = at(100.0);
    fogged.region_effects = vec![RegionEffectName::NebulaFog];
    let reading = derive(&config, &depot(0.37), &fogged, 1).expect("still a reading");
    assert_eq!(
        reading.band, "coarse",
        "inside the fog the close-range scan reads out at the next band down"
    );
    assert_eq!(reading.condition_fraction, 0.25);

    let mut irrelevant = at(100.0);
    irrelevant.region_effects = vec![RegionEffectName::SlowZone];
    assert_eq!(
        derive(&config, &depot(0.37), &irrelevant, 1)
            .expect("a reading")
            .band,
        "detailed",
        "a hazard this hull never declared as interference is not interference"
    );
}

/// Interference that pushes past the coarsest band blinds the suite — a
/// refusal, not a silently empty reading.
#[test]
fn interference_past_the_last_band_blinds_the_suite() {
    let config = suite();
    let mut fogged = at(2_400.0);
    fogged.region_effects = vec![RegionEffectName::NebulaFog];
    assert_eq!(
        derive(&config, &depot(0.5), &fogged, 1),
        Err(ScanRefusal::Blinded)
    );
}

/// The reading is stamped with the tick it was taken on, which is what
/// makes it a reading rather than a live gauge.
#[test]
fn a_reading_carries_the_tick_it_was_taken_on() {
    let reading = derive(&suite(), &depot(0.5), &at(100.0), 4_242).expect("a reading");
    assert_eq!(reading.taken_at_tick, 4_242);
    assert_eq!(reading.subject_uuid, "depot-1");
    assert_eq!(reading.subject_name, "world.probe.entity.depot.name");
    assert_eq!(reading.band_label, "world.probe.band.detailed.label");
}

/// Quantisation is arithmetic, and it is stable at both ends of the range.
#[test]
fn quantisation_rounds_to_the_step_and_clamps_to_the_track() {
    assert_eq!(quantise(0.37, 0.25), 0.25);
    assert_eq!(quantise(0.38, 0.25), 0.5);
    assert_eq!(quantise(1.0, 0.25), 1.0);
    assert_eq!(quantise(0.0, 0.25), 0.0);
    assert_eq!(quantise(-0.4, 0.25), 0.0, "a track cannot read below empty");
    assert_eq!(quantise(1.4, 0.25), 1.0, "…or above full");
    assert_eq!(
        quantise(0.37, 0.0),
        0.37,
        "a step of zero reports the value unchanged rather than dividing by it"
    );
}

/// The authored vocabulary round-trips through TOML, and an unknown key is
/// a parse error rather than a silently ignored typo.
#[test]
fn the_authored_table_round_trips_and_refuses_unknown_keys() {
    let toml = r#"
power_group = "shields"
min_power_level = 2
degraded_by = ["nebula_fog"]
interference_bands = 1

[[band]]
id = "detailed"
label = "world.probe.band.detailed.label"
max_range = 500.0
condition_step = 0.01

[[band]]
id = "coarse"
label = "world.probe.band.coarse.label"
max_range = 3000.0
condition_step = 0.25
report_capacities = false
"#;
    let parsed: ScanConfig = toml::from_str(toml).expect("the authored shape parses");
    parsed.validate().expect("and validates");
    assert_eq!(parsed, suite());

    let typo: Result<ScanConfig, _> = toml::from_str("min_power_lvl = 2\n");
    assert!(typo.is_err(), "a mistyped key is refused, not ignored");
}

/// A table that omits everything optional takes the documented defaults.
#[test]
fn an_authored_table_takes_the_documented_defaults_for_everything_it_omits() {
    let parsed: ScanConfig = toml::from_str(
        r#"
[[band]]
id = "only"
label = "world.probe.band.only.label"
max_range = 900.0
condition_step = 0.05
"#,
    )
    .expect("parses");
    assert_eq!(parsed.power_group, "shields");
    assert_eq!(parsed.min_power_level, 1);
    assert_eq!(parsed.interference_bands, 1);
    assert!(parsed.degraded_by.is_empty());
    assert!(parsed.bands[0].report_thresholds);
    assert!(parsed.bands[0].report_capacities);
    parsed.validate().expect("and validates");
}

/// Every author mistake that would otherwise show up as a console which
/// quietly never answers is a load failure naming the band.
#[test]
fn validation_refuses_a_ladder_that_could_never_answer() {
    let empty = ScanConfig::default();
    assert!(empty
        .validate()
        .expect_err("no bands")
        .contains("no [[scan.band]]"));

    let mut duplicate = suite();
    duplicate.bands[1].id = "detailed".into();
    assert!(duplicate
        .validate()
        .expect_err("duplicate id")
        .contains("declared twice"));

    let mut unreachable = suite();
    unreachable.bands[1].max_range = 200.0;
    assert!(unreachable
        .validate()
        .expect_err("descending ranges")
        .contains("FINEST FIRST"));

    let mut nameless = suite();
    nameless.bands[0].label = "  ".into();
    assert!(nameless
        .validate()
        .expect_err("no label")
        .contains("empty label"));

    let mut silly_step = suite();
    silly_step.bands[0].condition_step = 0.0;
    assert!(silly_step
        .validate()
        .expect_err("zero step")
        .contains("condition_step"));

    let mut no_reach = suite();
    no_reach.bands[0].max_range = 0.0;
    assert!(no_reach
        .validate()
        .expect_err("zero range")
        .contains("no reach"));
}

/// The refusal vocabulary is closed, distinct, and every member has a
/// literal `strings.csv` id the checker can find.
#[test]
fn every_refusal_has_its_own_string_id() {
    let mut ids: Vec<&str> = ScanRefusal::ALL.iter().map(|r| r.string_id()).collect();
    assert_eq!(ids.len(), 6);
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), 6, "the six ids are distinct");
    for refusal in ScanRefusal::ALL {
        assert!(refusal.string_id().starts_with("scan.refusal."));
    }
}

/// A refusal serialises under its script name, so a save and a payload
/// spell it identically.
#[test]
fn a_refusal_serialises_under_its_snake_case_name() {
    assert_eq!(
        serde_json::to_string(&ScanRefusal::NoReadableCondition).unwrap(),
        "\"no_readable_condition\""
    );
}
