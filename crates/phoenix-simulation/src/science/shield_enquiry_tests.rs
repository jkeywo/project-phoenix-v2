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
