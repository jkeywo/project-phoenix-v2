use super::*;
use crate::core::messages::{ScanBlackboard, ScanReadingSnapshot};
#[test]
fn only_a_qualified_explicitly_permitted_acquired_subject_can_supply_an_advisory() {
    let config = crate::entities::include_resolve::load_entity_config(
        "assets/entities/alliance_cruiser.toml",
    )
    .unwrap();
    let mut ship = config.ship_config.unwrap();
    let owner = SystemId("sensors".into());
    let source = super::super::scan_blackboard_key_for(&owner);
    let finding = ShieldFinding::acquire(0.7, Default::default(), 0.01).unwrap();
    let reading = ScanReadingSnapshot {
        aim: FocusedAim::Shields,
        shield_finding: Some(finding.clone()),
        subject_uuid: "public-return".into(),
        taken_at_tick: 10,
        ..Default::default()
    };
    let mut boards = ShipSystemBlackboards::default();
    boards.0.insert(
        source.clone(),
        SystemBlackboard::Scan(ScanBlackboard {
            capable: true,
            owner: Some(owner.clone()),
            history: vec![reading],
            ..Default::default()
        }),
    );
    assert_eq!(
        latest_permitted_finding(&ship, &boards, &owner, "public-return"),
        Some(&finding)
    );
    if let Some(SystemBlackboard::Scan(scan)) = boards.0.get_mut(&source) {
        let mut newer = scan.history[0].clone();
        newer.taken_at_tick = 11;
        newer.shield_finding = ShieldFinding::acquire(0.3, Default::default(), 0.01);
        scan.history.push(newer);
    }
    assert_eq!(
        latest_permitted_finding(&ship, &boards, &owner, "public-return")
            .unwrap()
            .frequency,
        0.3
    );
    assert!(latest_permitted_finding(&ship, &boards, &owner, "private-actor").is_none());
    assert!(
        latest_permitted_finding(&ship, &boards, &SystemId("repair".into()), "public-return")
            .is_none()
    );
    let repair = SystemId("repair".into());
    let console = ship.system(&repair).unwrap().console.clone().unwrap();
    ship.consoles
        .iter_mut()
        .find(|row| row.id == console)
        .unwrap()
        .feeds
        .push(phoenix_model::console::ConsoleFeed {
            source: source.clone(),
            kind: phoenix_model::console::ConsoleObservationKind::Scan,
        });
    assert!(latest_permitted_finding(&ship, &boards, &repair, "public-return").is_some());
    ship.consoles
        .iter_mut()
        .find(|row| row.id == console)
        .unwrap()
        .feeds
        .retain(|feed| feed.source != source);
    assert!(latest_permitted_finding(&ship, &boards, &repair, "public-return").is_none());
}

#[test]
fn equally_recent_contradictory_findings_across_proven_aliases_withhold_advice() {
    let config = crate::entities::include_resolve::load_entity_config(
        "assets/entities/alliance_cruiser.toml",
    )
    .unwrap()
    .ship_config
    .unwrap();
    let owner = SystemId("sensors".into());
    let source = super::super::scan_blackboard_key_for(&owner);
    let mut boards = ShipSystemBlackboards::default();
    super::fixture::acquired(&mut boards, "science-handle", 0.2);
    super::fixture::acquired(&mut boards, "weapon-handle", 0.8);
    let Some(SystemBlackboard::Scan(scan)) = boards.0.get_mut(&source) else {
        panic!("Scan");
    };
    scan.history[0].taken_at_tick = 10;
    scan.history[1].taken_at_tick = 10;
    let original = scan.history.clone();
    let subjects =
        std::collections::BTreeSet::from(["science-handle".into(), "weapon-handle".into()]);
    assert!(latest_permitted_finding_for_subjects(&config, &boards, &owner, &subjects).is_none());
    let Some(SystemBlackboard::Scan(scan)) = boards.0.get(&source) else {
        unreachable!();
    };
    assert_eq!(scan.history, original);
}

#[test]
fn shield_finding_preflight_is_only_a_permitted_qualified_necessary_condition() {
    let config = crate::entities::include_resolve::load_entity_config(
        "assets/entities/alliance_cruiser.toml",
    )
    .unwrap()
    .ship_config
    .unwrap();
    let owner = SystemId("sensors".into());
    let source = super::super::scan_blackboard_key_for(&owner);
    let subjects = std::collections::BTreeSet::from(["public".to_string()]);
    for variant in 0..10 {
        let mut boards = ShipSystemBlackboards::default();
        super::fixture::acquired(&mut boards, "public", 0.73);
        let Some(SystemBlackboard::Scan(scan)) = boards.0.get_mut(&source) else {
            panic!("Scan");
        };
        match variant {
            0 => scan.history.clear(),
            1 => scan.history[0].aim = FocusedAim::General,
            2 => scan.history[0].shield_finding = None,
            3 => scan.capable = false,
            4 => scan.owner = Some(SystemId("other".into())),
            5 => scan.history[0].shield_finding.as_mut().unwrap().frequency = -1.0,
            6 => {
                let mut newer = scan.history[0].clone();
                newer.taken_at_tick += 1;
                newer.shield_finding = None;
                scan.history.push(newer);
            }
            7 => scan.history[0].subject_uuid = "other-public".into(),
            8 => {}
            _ => {
                let row = boards.0.remove(&source).unwrap();
                boards.0.insert(SystemId("scan/noncanonical".into()), row);
            }
        }
        let before = boards.0.clone();
        let possible = has_permitted_shield_finding(&config, &boards, &owner);
        let actual = latest_permitted_finding_for_subjects(&config, &boards, &owner, &subjects);
        if actual.is_some() {
            assert!(possible);
        }
        assert_eq!(possible, matches!(variant, 5..=8), "variant {variant}");
        if matches!(variant, 5..=7) {
            assert!(
                actual.is_none(),
                "slow path still decides variant {variant}"
            );
        }
        assert!(!has_permitted_shield_finding(
            &config,
            &boards,
            &SystemId("repair".into())
        ));
        assert_eq!(boards.0, before);
    }
}
