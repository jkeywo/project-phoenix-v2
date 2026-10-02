use super::*;
use crate::core::report::{MissionReport, ReportRow, ReportRowState};

fn hit(attacker: Option<&str>, victim: &str, shield: f32, hull: f32) -> BalanceEvent {
    BalanceEvent::DamageApplied {
        attacker: attacker.map(|s| s.to_string()),
        victim: victim.to_string(),
        victim_kind: VictimKind::Ship,
        weapon: "fore_phaser".into(),
        amount: shield + hull,
        shield_absorbed: shield,
        hull_damage: hull,
        system_hit: None,
    }
}

fn stamp(tick: u64, sim_t: f64, event: BalanceEvent) -> StampedBalanceEvent {
    StampedBalanceEvent { tick, sim_t, event }
}

#[test]
fn aggregate_splits_dealt_and_taken_per_ship() {
    let events = vec![
        hit(Some("player"), "raider", 6.0, 4.0),
        hit(Some("player"), "raider", 0.0, 5.0),
        hit(Some("raider"), "player", 3.0, 0.0),
    ];
    let ledgers = aggregate_damage(&events, &BTreeMap::new());

    assert_eq!(ledgers["player"].damage_dealt, 15.0);
    assert_eq!(ledgers["player"].damage_taken, 3.0);
    assert_eq!(ledgers["raider"].damage_dealt, 3.0);
    assert_eq!(ledgers["raider"].damage_taken, 15.0);
}

#[test]
fn environmental_damage_has_no_attacker_but_still_charges_the_victim() {
    let events = vec![BalanceEvent::DamageApplied {
        attacker: None,
        victim: "player".into(),
        victim_kind: VictimKind::Ship,
        weapon: WEAPON_KIND_REGION.into(),
        amount: 8.0,
        shield_absorbed: 2.0,
        hull_damage: 6.0,
        system_hit: None,
    }];
    let ledgers = aggregate_damage(&events, &BTreeMap::new());

    assert_eq!(ledgers.len(), 1);
    assert_eq!(ledgers["player"].damage_taken, 8.0);
    assert_eq!(ledgers["player"].damage_dealt, 0.0);
}

/// `damage_by_ship` is a combat-effectiveness table. A shooter chewing
/// through an asteroid field must not read as a shooter winning a fight,
/// and the rock must not get a ledger row of its own.
#[test]
fn asteroid_victims_are_excluded_from_both_sides_of_the_ledger() {
    let mining = BalanceEvent::DamageApplied {
        attacker: Some("player".into()),
        victim: "asteroid-7".into(),
        victim_kind: VictimKind::Asteroid,
        weapon: "fore_phaser".into(),
        amount: 30.0,
        shield_absorbed: 0.0,
        hull_damage: 30.0,
        system_hit: None,
    };
    let events = vec![mining, hit(Some("player"), "raider", 1.0, 2.0)];
    let ledgers = aggregate_damage(&events, &BTreeMap::new());

    assert!(
        !ledgers.contains_key("asteroid-7"),
        "an asteroid must not get a ledger row"
    );
    assert_eq!(
        ledgers["player"].damage_dealt, 3.0,
        "only the hit on the raider counts as damage dealt"
    );
    assert_eq!(ledgers["raider"].damage_taken, 3.0);
}

/// A run that only ever shot rocks produced no combat at all.
#[test]
fn asteroid_only_log_yields_an_empty_ledger_map() {
    let events = vec![BalanceEvent::DamageApplied {
        attacker: Some("player".into()),
        victim: "asteroid-1".into(),
        victim_kind: VictimKind::Asteroid,
        weapon: "fore_tube".into(),
        amount: 12.0,
        shield_absorbed: 0.0,
        hull_damage: 12.0,
        system_hit: None,
    }];

    assert!(aggregate_damage(&events, &BTreeMap::new()).is_empty());
}

#[test]
fn names_are_attached_when_known_and_null_otherwise() {
    let events = vec![hit(Some("player"), "raider", 0.0, 1.0)];
    // A TOML-defined ship carries a strings.csv key here, not display
    // text — the ledger stores whatever `EntityName` held, verbatim.
    let names: BTreeMap<String, String> = [(
        "player".to_string(),
        "entity.alliance_cruiser.name".to_string(),
    )]
    .into_iter()
    .collect();
    let ledgers = aggregate_damage(&events, &names);

    assert_eq!(
        ledgers["player"].name_id.as_deref(),
        Some("entity.alliance_cruiser.name")
    );
    assert_eq!(ledgers["raider"].name_id, None);
}

// ── New split fields ──────────────────────────────────────────────────────

/// `by_weapon` credits each weapon with the damage it landed; `by_pair`
/// credits each victim.
#[test]
fn by_weapon_and_by_pair_split_the_dealt_total() {
    let events = vec![
        BalanceEvent::DamageApplied {
            attacker: Some("player".into()),
            victim: "raider".into(),
            victim_kind: VictimKind::Ship,
            weapon: "fore_phaser".into(),
            amount: 10.0,
            shield_absorbed: 4.0,
            hull_damage: 6.0,
            system_hit: None,
        },
        BalanceEvent::DamageApplied {
            attacker: Some("player".into()),
            victim: "scout".into(),
            victim_kind: VictimKind::Ship,
            weapon: "fore_tube".into(),
            amount: 5.0,
            shield_absorbed: 0.0,
            hull_damage: 5.0,
            system_hit: None,
        },
    ];
    let l = &aggregate_damage(&events, &BTreeMap::new())["player"];
    assert_eq!(l.damage_dealt, 15.0);
    assert_eq!(l.by_weapon["fore_phaser"], 10.0);
    assert_eq!(l.by_weapon["fore_tube"], 5.0);
    assert_eq!(l.by_pair["raider"], 10.0);
    assert_eq!(l.by_pair["scout"], 5.0);
}

/// The shield/hull split of `damage_taken` reconciles to the total.
#[test]
fn shield_and_hull_taken_split_the_taken_total() {
    let events = vec![
        hit(Some("raider"), "player", 6.0, 4.0),
        hit(Some("raider"), "player", 0.0, 5.0),
    ];
    let l = &aggregate_damage(&events, &BTreeMap::new())["player"];
    assert_eq!(l.damage_taken, 15.0);
    assert_eq!(l.shield_absorbed, 6.0);
    assert_eq!(l.hull_taken, 9.0);
    assert_eq!(l.shield_absorbed + l.hull_taken, l.damage_taken);
}

#[test]
fn shots_fired_counts_weapon_fired_per_weapon_even_without_a_hit() {
    let events = vec![
        BalanceEvent::WeaponFired {
            shooter: Some("player".into()),
            weapon: "port".into(),
            kind: FIRED_KIND_BEAM.into(),
        },
        BalanceEvent::WeaponFired {
            shooter: Some("player".into()),
            weapon: "port".into(),
            kind: FIRED_KIND_BEAM.into(),
        },
        BalanceEvent::WeaponFired {
            shooter: Some("player".into()),
            weapon: "fore_tube".into(),
            kind: FIRED_KIND_TORPEDO.into(),
        },
    ];
    let l = &aggregate_damage(&events, &BTreeMap::new())["player"];
    assert_eq!(l.shots_fired["port"], 2);
    assert_eq!(l.shots_fired["fore_tube"], 1);
    // A ship that only fired (never landed) still gets a row.
    assert_eq!(l.damage_dealt, 0.0);
}

#[test]
fn kills_credit_the_killer_and_death_stamps_the_victim() {
    let events = vec![
        stamp(5, 0.5, hit(Some("player"), "raider", 0.0, 3.0)),
        stamp(
            9,
            0.9,
            BalanceEvent::EntityDestroyed {
                victim: "raider".into(),
                killer: Some("player".into()),
            },
        ),
        // A later hit on the corpse must not move the death stamp.
        stamp(
            12,
            1.2,
            BalanceEvent::EntityDestroyed {
                victim: "raider".into(),
                killer: Some("player".into()),
            },
        ),
    ];
    let ledgers = aggregate_ledgers(&events, &BTreeMap::new(), 2.0);
    assert_eq!(ledgers["player"].kills, 2);
    assert_eq!(ledgers["raider"].death, Some((9, 0.9)));
}

#[test]
fn knockouts_record_disabling_crossings_with_timestamps() {
    let events = vec![
        // A crossing to Damaged is not a knockout.
        stamp(
            3,
            0.3,
            BalanceEvent::SystemTierCrossed {
                ship: "raider".into(),
                system_id: "phaser-fore".into(),
                from_tier: "Operational".into(),
                to_tier: "Damaged".into(),
            },
        ),
        stamp(
            7,
            0.7,
            BalanceEvent::SystemTierCrossed {
                ship: "raider".into(),
                system_id: "phaser-fore".into(),
                from_tier: "Damaged".into(),
                to_tier: "Disabled".into(),
            },
        ),
        stamp(
            8,
            0.8,
            BalanceEvent::SystemTierCrossed {
                ship: "raider".into(),
                system_id: "helm".into(),
                from_tier: "Operational".into(),
                to_tier: "Destroyed".into(),
            },
        ),
    ];
    let l = &aggregate_ledgers(&events, &BTreeMap::new(), 1.0)["raider"];
    assert_eq!(
        l.system_knockouts.len(),
        2,
        "only disabling crossings count"
    );
    assert_eq!(l.system_knockouts[0].system_id, "phaser-fore");
    assert_eq!(l.system_knockouts[0].tier, "Disabled");
    assert_eq!(l.system_knockouts[0].tick, 7);
    assert_eq!(l.system_knockouts[1].system_id, "helm");
    assert_eq!(l.system_knockouts[1].sim_t, 0.8);
}

fn phase(ship: &str, phase: &str) -> BalanceEvent {
    BalanceEvent::DoctrinePhaseChanged {
        ship: ship.to_string(),
        phase: phase.to_string(),
    }
}

/// Occupancy is the time between consecutive phase changes, per ship, with
/// the open interval at the end closed at the run's final sim time.
#[test]
fn phase_occupancy_attributes_time_between_changes_and_closes_at_run_end() {
    let events = vec![
        stamp(1, 0.0, phase("player", "acquire")),
        stamp(50, 5.0, phase("player", "attack_run")),
        // Re-entering a phase accumulates onto the same key.
        stamp(120, 12.0, phase("player", "acquire")),
        // A second ship's machine is folded independently.
        stamp(2, 1.0, phase("raider", "acquire")),
    ];
    let ledgers = aggregate_ledgers(&events, &BTreeMap::new(), 20.0);
    let p = &ledgers["player"].phase_seconds;
    assert!((p["acquire"] - (5.0 + 8.0)).abs() < 1e-9, "got {p:?}");
    assert!((p["attack_run"] - 7.0).abs() < 1e-9, "got {p:?}");
    assert!((ledgers["raider"].phase_seconds["acquire"] - 19.0).abs() < 1e-9);
}

/// A dead ship's machine stops with the ship: its open phase closes at the
/// death stamp, never at the end of the run.
#[test]
fn phase_occupancy_closes_at_the_ships_death_not_run_end() {
    let events = vec![
        stamp(1, 0.0, phase("raider", "acquire")),
        stamp(30, 3.0, phase("raider", "escape")),
        stamp(
            90,
            9.0,
            BalanceEvent::EntityDestroyed {
                victim: "raider".into(),
                killer: Some("player".into()),
            },
        ),
    ];
    let l = &aggregate_ledgers(&events, &BTreeMap::new(), 60.0)["raider"];
    assert!((l.phase_seconds["acquire"] - 3.0).abs() < 1e-9);
    assert!(
        (l.phase_seconds["escape"] - 6.0).abs() < 1e-9,
        "the corpse must not accrue occupancy: got {:?}",
        l.phase_seconds
    );
}

/// The ledger JSON carries `phase_seconds` as a parseable object.
#[test]
fn ledger_json_carries_phase_seconds() {
    let events = vec![
        stamp(1, 0.0, phase("player", "acquire")),
        stamp(60, 6.0, phase("player", "attack_run")),
    ];
    let json = format!(
        "{{{}}}",
        ledgers_to_json(&aggregate_ledgers(&events, &BTreeMap::new(), 10.0))
    );
    let parsed: serde_json::Value = serde_json::from_str(&json)
        .unwrap_or_else(|e| panic!("ledgers are not valid JSON: {e}\n{json}"));
    assert_eq!(parsed["player"]["phase_seconds"]["acquire"], 6.0);
    assert_eq!(parsed["player"]["phase_seconds"]["attack_run"], 4.0);
}

#[test]
fn repair_applied_accumulates_hull_restored() {
    let events = vec![
        BalanceEvent::RepairApplied {
            ship: "player".into(),
            hp: 2.5,
        },
        BalanceEvent::RepairApplied {
            ship: "player".into(),
            hp: 1.5,
        },
    ];
    let l = &aggregate_damage(&events, &BTreeMap::new())["player"];
    assert_eq!(l.repair_hp, 4.0);
}

/// The ndjson timeline drops per-tick repair deltas (they were ~80% of a
/// combat run's lines) — but the ledger must still total every one of
/// them, so filtering the *stream* may never touch the fold.
#[test]
fn repair_is_kept_out_of_the_timeline_but_still_totalled() {
    let events = vec![
        BalanceEvent::RepairApplied {
            ship: "player".into(),
            hp: 2.0,
        },
        hit(Some("raider"), "player", 1.0, 3.0),
        BalanceEvent::RepairApplied {
            ship: "player".into(),
            hp: 1.0,
        },
    ];
    let streamed: Vec<&BalanceEvent> = events.iter().filter(|e| e.in_timeline_stream()).collect();
    assert_eq!(streamed.len(), 1, "only the hit is a timeline beat");
    assert!(matches!(streamed[0], BalanceEvent::DamageApplied { .. },));
    let l = &aggregate_damage(&events, &BTreeMap::new())["player"];
    assert_eq!(l.repair_hp, 3.0, "the fold still sees every repair tick");
}

/// Everything other than the per-tick repair delta is a story beat and
/// belongs in the timeline. Written as an exhaustive list so a new variant
/// forces a deliberate decision rather than silently defaulting in.
#[test]
fn every_other_variant_stays_in_the_timeline() {
    let cases = vec![
        hit(Some("a"), "b", 1.0, 1.0),
        BalanceEvent::WeaponFired {
            shooter: Some("a".into()),
            weapon: "port".into(),
            kind: FIRED_KIND_BEAM.into(),
        },
        BalanceEvent::ShieldArcCollapsed {
            ship: "b".into(),
            arc_id: "fore".into(),
        },
        BalanceEvent::SystemTierCrossed {
            ship: "b".into(),
            system_id: "helm".into(),
            from_tier: "Operational".into(),
            to_tier: "Disabled".into(),
        },
        BalanceEvent::Disarmed { ship: "b".into() },
        BalanceEvent::EntityDestroyed {
            victim: "b".into(),
            killer: Some("a".into()),
        },
        BalanceEvent::RedAlertChanged {
            ship: "a".into(),
            on: true,
        },
        BalanceEvent::ObjectiveCompleted {
            objective_id: "reach_beacon".into(),
        },
        BalanceEvent::ObjectiveChanged {
            objective_id: "reach_beacon".into(),
            status: crate::core::messages::ObjectiveStatus::Completed,
            targets: vec!["beacon".into()],
        },
        BalanceEvent::TriggerFired {
            trigger_id: "reach_beacon".into(),
            origin: "world.rhai".into(),
            entity: Some("a".into()),
        },
        BalanceEvent::PhaseChanged {
            from: "Lobby".into(),
            to: "InProgress".into(),
        },
        BalanceEvent::DoctrinePhaseChanged {
            ship: "a".into(),
            phase: "attack_run".into(),
        },
    ];
    // Anti-vacuity: the list above is one per variant *except*
    // `RepairApplied`, which is the only intentional exclusion. Pinning the
    // count is what makes "a new variant forces a deliberate decision" true
    // rather than aspirational — add a variant and this fails until you have
    // said, here, which side of the timeline it belongs on.
    assert_eq!(
        cases.len(),
        BalanceEvent::VARIANT_COUNT - 1,
        "every BalanceEvent variant but RepairApplied must be covered here"
    );
    for event in cases {
        assert!(event.in_timeline_stream(), "{event:?} left the timeline");
    }
}

#[test]
fn ledger_json_is_parseable_and_ordered_by_uuid() {
    let events = vec![
        stamp(1, 0.1, hit(Some("zulu"), "alpha", 1.0, 2.0)),
        stamp(2, 0.2, hit(Some("alpha"), "zulu", 0.5, 0.0)),
        stamp(
            3,
            0.3,
            BalanceEvent::WeaponFired {
                shooter: Some("alpha".into()),
                weapon: "port".into(),
                kind: FIRED_KIND_BEAM.into(),
            },
        ),
        stamp(
            4,
            0.4,
            BalanceEvent::EntityDestroyed {
                victim: "zulu".into(),
                killer: Some("alpha".into()),
            },
        ),
    ];
    let names: BTreeMap<String, String> = [("alpha".to_string(), "Ironveil".to_string())]
        .into_iter()
        .collect();
    let json = format!(
        "{{{}}}",
        ledgers_to_json(&aggregate_ledgers(&events, &names, 0.4))
    );
    let parsed: serde_json::Value = serde_json::from_str(&json)
        .unwrap_or_else(|e| panic!("ledgers are not valid JSON: {e}\n{json}"));

    assert_eq!(parsed["alpha"]["name_id"], "Ironveil");
    assert_eq!(parsed["alpha"]["damage_taken"], 3.0);
    assert_eq!(parsed["alpha"]["damage_dealt"], 0.5);
    assert_eq!(parsed["alpha"]["by_weapon"]["fore_phaser"], 0.5);
    assert_eq!(parsed["alpha"]["by_pair"]["zulu"], 0.5);
    assert_eq!(parsed["alpha"]["shots_fired"]["port"], 1);
    assert_eq!(parsed["alpha"]["kills"], 1);
    assert!(parsed["zulu"]["name_id"].is_null());
    assert_eq!(parsed["zulu"]["death"][0], 4);
    assert!(json.find("\"alpha\"").unwrap() < json.find("\"zulu\"").unwrap());
}

#[test]
fn damage_event_json_carries_the_split_and_nulls_the_unknowns() {
    let event = BalanceEvent::DamageApplied {
        attacker: None,
        victim: "player".into(),
        victim_kind: VictimKind::Ship,
        weapon: WEAPON_KIND_COLLISION.into(),
        amount: 12.0,
        shield_absorbed: 4.0,
        hull_damage: 8.0,
        system_hit: None,
    };
    let parsed: serde_json::Value = serde_json::from_str(&event.to_json()).unwrap();

    assert_eq!(parsed["event"], "DamageApplied");
    assert_eq!(parsed["victim_kind"], "ship");
    assert!(parsed["attacker"].is_null());
    assert!(parsed["system_hit"].is_null());
    assert_eq!(parsed["victim"], "player");
    assert_eq!(parsed["weapon"], "collision");
    assert_eq!(parsed["shield_absorbed"], 4.0);
    assert_eq!(parsed["hull_damage"], 8.0);
}

/// Every new variant round-trips through `to_json` as parseable JSON that
/// names the variant and carries its fields.
#[test]
fn new_variants_encode_parseable_json() {
    let cases = vec![
        BalanceEvent::WeaponFired {
            shooter: Some("player".into()),
            weapon: "port".into(),
            kind: FIRED_KIND_BEAM.into(),
        },
        BalanceEvent::ShieldArcCollapsed {
            ship: "player".into(),
            arc_id: "fore".into(),
        },
        BalanceEvent::SystemTierCrossed {
            ship: "raider".into(),
            system_id: "helm".into(),
            from_tier: "Operational".into(),
            to_tier: "Disabled".into(),
        },
        BalanceEvent::Disarmed {
            ship: "raider".into(),
        },
        BalanceEvent::EntityDestroyed {
            victim: "raider".into(),
            killer: Some("player".into()),
        },
        BalanceEvent::RedAlertChanged {
            ship: "player".into(),
            on: true,
        },
        BalanceEvent::ObjectiveCompleted {
            objective_id: "reach_beacon".into(),
        },
        BalanceEvent::ObjectiveChanged {
            objective_id: "reach_beacon".into(),
            status: crate::core::messages::ObjectiveStatus::Completed,
            targets: vec!["beacon".into()],
        },
        BalanceEvent::TriggerFired {
            trigger_id: "reach_beacon".into(),
            origin: "world.rhai".into(),
            entity: Some("player".into()),
        },
        BalanceEvent::PhaseChanged {
            from: "Lobby".into(),
            to: "InProgress".into(),
        },
        BalanceEvent::RepairApplied {
            ship: "player".into(),
            hp: 3.5,
        },
        BalanceEvent::DoctrinePhaseChanged {
            ship: "player".into(),
            phase: "acquire".into(),
        },
    ];
    for event in cases {
        let json = event.to_json();
        let parsed: serde_json::Value = serde_json::from_str(&json)
            .unwrap_or_else(|e| panic!("variant JSON is not parseable: {e}\n{json}"));
        assert!(parsed["event"].is_string(), "event tag missing in {json}");
    }
}

#[test]
fn weapon_fired_and_destroyed_json_shape() {
    let fired = BalanceEvent::WeaponFired {
        shooter: None,
        weapon: "fore_tube".into(),
        kind: FIRED_KIND_TORPEDO.into(),
    };
    let parsed: serde_json::Value = serde_json::from_str(&fired.to_json()).unwrap();
    assert_eq!(parsed["event"], "WeaponFired");
    assert!(parsed["shooter"].is_null());
    assert_eq!(parsed["weapon"], "fore_tube");
    assert_eq!(parsed["kind"], "torpedo");

    let killed = BalanceEvent::EntityDestroyed {
        victim: "raider".into(),
        killer: Some("player".into()),
    };
    let parsed: serde_json::Value = serde_json::from_str(&killed.to_json()).unwrap();
    assert_eq!(parsed["killer"], "player");
    assert_eq!(parsed["victim"], "raider");
}

// ── Run outcome classification (issue #843) ────────────────────────────

#[test]
fn outcome_parse_is_case_insensitive_and_rejects_junk() {
    assert_eq!(Outcome::parse("victory"), Ok(Outcome::Victory));
    assert_eq!(Outcome::parse("  Victory "), Ok(Outcome::Victory));
    assert_eq!(Outcome::parse("DEFEAT"), Ok(Outcome::Defeat));
    assert!(Outcome::parse("draw").is_err());
    assert!(Outcome::parse("").is_err());
}

fn margins(hull: f32, hull_max: f32, dealt: f32, taken: f32, closing: f32) -> SideMargins {
    SideMargins::new(hull, hull_max, dealt, taken, closing)
}

/// A run that reached GameOver with a declared victory flag is a victory.
#[test]
fn classify_victory_from_game_over_flag() {
    let report = classify(
        true,
        Some(Outcome::Victory),
        margins(80.0, 100.0, 200.0, 40.0, 0.0),
        margins(0.0, 0.0, 40.0, 200.0, 0.0),
        MissionReport::default(),
    );
    assert_eq!(report.outcome, RunOutcome::Victory);
    // A scripted end with no declared outcome also reads as victory.
    assert_eq!(
        classify(
            true,
            None,
            SideMargins::default(),
            SideMargins::default(),
            MissionReport::default()
        )
        .outcome,
        RunOutcome::Victory
    );
}

/// The built-in player-death latch (Defeat) wins over the default.
#[test]
fn classify_defeat_from_game_over_flag() {
    let report = classify(
        true,
        Some(Outcome::Defeat),
        margins(0.0, 100.0, 30.0, 220.0, 0.0),
        margins(60.0, 100.0, 220.0, 30.0, 0.0),
        MissionReport::default(),
    );
    assert_eq!(report.outcome, RunOutcome::Defeat);
}

/// Budget exhausted with damage still landing in the closing window is a
/// timeout, not a draw.
#[test]
fn classify_timeout_when_closing_window_is_live() {
    let report = classify(
        false,
        None,
        margins(55.0, 100.0, 120.0, 90.0, 3.5),
        margins(40.0, 100.0, 90.0, 120.0, 2.0),
        MissionReport::default(),
    );
    assert_eq!(report.outcome, RunOutcome::Timeout);
    // Margins survive onto the report for AC2.
    assert_eq!(report.player.remaining_hull_fraction, 0.55);
    assert_eq!(report.enemy.closing_damage_rate, 2.0);
}

/// Budget exhausted with a dead closing window (mutual ineffectiveness) is
/// a draw.
#[test]
fn classify_draw_when_closing_window_is_silent() {
    let report = classify(
        false,
        None,
        margins(70.0, 100.0, 10.0, 8.0, 0.0),
        margins(65.0, 100.0, 8.0, 10.0, 0.0),
        MissionReport::default(),
    );
    assert_eq!(report.outcome, RunOutcome::Draw);
    assert_eq!(report.player.damage_dealt, 10.0);
    assert_eq!(report.enemy.damage_taken, 10.0);
}

/// The outcome flag only matters once GameOver is reached: an unresolved
/// run ignores a stray flag and still classifies on the closing window.
#[test]
fn classify_ignores_outcome_flag_before_game_over() {
    let live = classify(
        false,
        Some(Outcome::Victory),
        margins(50.0, 100.0, 0.0, 0.0, 5.0),
        margins(50.0, 100.0, 0.0, 0.0, 0.0),
        MissionReport::default(),
    );
    assert_eq!(live.outcome, RunOutcome::Timeout);
}

#[test]
fn closing_rates_only_count_landed_damage_inside_the_window() {
    let events = vec![
        // Well before the window — ignored.
        stamp(10, 5.0, hit(Some("player"), "raider", 4.0, 6.0)),
        // Inside the 15 s window ending at t=60 (cutoff 45).
        stamp(100, 50.0, hit(Some("player"), "raider", 0.0, 10.0)),
        stamp(110, 55.0, hit(Some("raider"), "player", 5.0, 5.0)),
        // Environmental (no attacker) — belongs to no side, dropped.
        stamp(
            115,
            58.0,
            BalanceEvent::DamageApplied {
                attacker: None,
                victim: "player".into(),
                victim_kind: VictimKind::Ship,
                weapon: WEAPON_KIND_REGION.into(),
                amount: 30.0,
                shield_absorbed: 0.0,
                hull_damage: 30.0,
                system_hit: None,
            },
        ),
    ];
    let rates = closing_damage_rates(&events, 60.0, CLOSING_WINDOW_SECS);
    // player: 10 landed / 15 s window.
    assert!((rates["player"] - 10.0 / 15.0).abs() < 1e-4);
    // raider: 10 landed / 15 s window.
    assert!((rates["raider"] - 10.0 / 15.0).abs() < 1e-4);
    assert_eq!(rates.len(), 2, "environmental damage gets no side row");
}

#[test]
fn closing_rates_are_empty_for_a_nonpositive_window() {
    let events = vec![stamp(1, 59.0, hit(Some("player"), "raider", 0.0, 10.0))];
    assert!(closing_damage_rates(&events, 60.0, 0.0).is_empty());
}

#[test]
fn outcome_report_json_carries_outcome_and_both_sides() {
    let report = classify(
        false,
        None,
        margins(55.0, 100.0, 120.0, 90.0, 3.5),
        margins(40.0, 80.0, 90.0, 120.0, 2.0),
        MissionReport::default(),
    );
    let json = format!("{{{}}}", report.to_json());
    let parsed: serde_json::Value = serde_json::from_str(&json)
        .unwrap_or_else(|e| panic!("outcome report is not valid JSON: {e}\n{json}"));
    assert_eq!(parsed["outcome"], "timeout");
    assert_eq!(parsed["sides"]["player"]["remaining_hull"], 55.0);
    assert_eq!(parsed["sides"]["player"]["closing_damage_rate"], 3.5);
    assert_eq!(parsed["sides"]["enemy"]["remaining_hull_max"], 80.0);
    assert_eq!(parsed["sides"]["enemy"]["damage_dealt"], 90.0);
    // Always written, even with nothing authored — see `OutcomeReport::to_json`.
    assert_eq!(parsed["report"]["total"], 0);
    assert!(parsed["report"]["rows"].as_array().unwrap().is_empty());
}

// ── Report-bearing endings (issue #1344) ───────────────────────────────

/// A Lyra-shaped row, the tracer this classification was built for.
fn lyra_row(state: ReportRowState, score: i32) -> ReportRow {
    ReportRow {
        id: "lyra".into(),
        heading_id: "world.falling_skyway.report.lyra.heading".into(),
        outcome_id: format!("world.falling_skyway.report.lyra.{}", state.as_str()),
        state,
        score,
    }
}

fn report_with(rows: Vec<ReportRow>) -> MissionReport {
    let mut report = MissionReport::default();
    for row in rows {
        report.set_row(row);
    }
    report
}

/// AC1: a report-bearing ending is classified `reported`.
#[test]
fn classify_reported_when_the_ending_carries_a_report() {
    let report = classify(
        true,
        Some(Outcome::Victory),
        margins(80.0, 100.0, 200.0, 40.0, 0.0),
        margins(0.0, 0.0, 40.0, 200.0, 0.0),
        report_with(vec![lyra_row(ReportRowState::Saved, 6)]),
    );
    assert_eq!(report.outcome, RunOutcome::Reported);
    assert_eq!(report.report.rows().len(), 1);
    assert_eq!(report.report.total(), 6);
}

/// The catastrophic half of AC2: a declared DEFEAT that still carries a row
/// is reported, not framed as a defeat. Losing the skyway does not erase
/// what the crew did for Lyra on the way there.
#[test]
fn classify_reported_outranks_a_declared_defeat() {
    let report = classify(
        true,
        Some(Outcome::Defeat),
        margins(0.0, 100.0, 30.0, 220.0, 0.0),
        margins(60.0, 100.0, 220.0, 30.0, 0.0),
        report_with(vec![lyra_row(ReportRowState::Saved, 6)]),
    );
    assert_eq!(report.outcome, RunOutcome::Reported);
}

/// AC5: an EMPTY report is not a report. Every scenario that authors none
/// keeps the ending it always had.
#[test]
fn classify_leaves_an_unreported_ending_exactly_as_it_was() {
    for (flag, expected) in [
        (Some(Outcome::Victory), RunOutcome::Victory),
        (Some(Outcome::Defeat), RunOutcome::Defeat),
        (None, RunOutcome::Victory),
    ] {
        let report = classify(
            true,
            flag,
            SideMargins::default(),
            SideMargins::default(),
            MissionReport::default(),
        );
        assert_eq!(report.outcome, expected, "flag {flag:?}");
    }
}

/// A run still `InProgress` has not ended, so rows written so far cannot
/// make it report-bearing: the budget branch still decides.
#[test]
fn classify_ignores_a_report_before_game_over() {
    let live = classify(
        false,
        None,
        margins(50.0, 100.0, 0.0, 0.0, 5.0),
        margins(50.0, 100.0, 0.0, 0.0, 0.0),
        report_with(vec![lyra_row(ReportRowState::Saved, 6)]),
    );
    assert_eq!(live.outcome, RunOutcome::Timeout);
    // The rows still travel — a reader diagnosing an unfinished run wants
    // to see what had been recorded when the budget ran out.
    assert_eq!(live.report.rows().len(), 1);
}

/// AC4: the headless JSON carries the signed per-row score and a total
/// equal to the visible rows' scores.
#[test]
fn outcome_report_json_carries_the_rows_scores_and_total() {
    let report = classify(
        true,
        Some(Outcome::Defeat),
        SideMargins::default(),
        SideMargins::default(),
        report_with(vec![lyra_row(ReportRowState::Lost, -6)]),
    );
    let json = format!("{{{}}}", report.to_json());
    let parsed: serde_json::Value = serde_json::from_str(&json)
        .unwrap_or_else(|e| panic!("outcome report is not valid JSON: {e}\n{json}"));
    assert_eq!(parsed["outcome"], "reported");
    assert_eq!(parsed["report"]["rows"][0]["id"], "lyra");
    assert_eq!(parsed["report"]["rows"][0]["state"], "lost");
    assert_eq!(parsed["report"]["rows"][0]["score"], -6);
    assert_eq!(parsed["report"]["total"], -6);
}

/// The wire vocabulary guard: every `RunOutcome` has a stable label, and
/// no two share one.
#[test]
fn every_run_outcome_has_a_distinct_label() {
    let labels: std::collections::BTreeSet<&str> = [
        RunOutcome::Victory,
        RunOutcome::Defeat,
        RunOutcome::Draw,
        RunOutcome::Timeout,
        RunOutcome::Reported,
    ]
    .iter()
    .map(|o| o.as_str())
    .collect();
    assert_eq!(labels.len(), 5);
    assert!(labels.contains("reported"));
}
