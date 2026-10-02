use super::*;
use crate::world::config::AvailableShipEntry;

fn slots() -> Vec<ShipSlotConfig> {
    vec![ShipSlotConfig {
        id: "lead".into(),
        label: None,
        ships: vec![AvailableShipEntry {
            template_path: "cruiser".into(),
            label: None,
        }],
        default_ship: "cruiser".into(),
        unclaimed: UnclaimedSlotPolicy::Backfill,
    }]
}

#[test]
fn multi_ship_racing_claims_are_exclusive_and_release_is_immediate() {
    let mut held = ShipSlotReservations::default();
    assert_eq!(
        held.claim(&slots(), "lead", "host-a"),
        ClaimOutcome::Claimed
    );
    assert_eq!(
        held.claim(&slots(), "lead", "host-b"),
        ClaimOutcome::Occupied
    );
    assert_eq!(held.release_claimant("host-a"), ["lead"]);
    assert_eq!(
        held.claim(&slots(), "lead", "host-b"),
        ClaimOutcome::Claimed
    );
}

#[test]
fn multi_ship_only_holder_can_confirm_an_allowed_hull() {
    let mut held = ShipSlotReservations::default();
    held.claim(&slots(), "lead", "host-a");
    assert_eq!(
        held.confirm_hull(&slots(), "lead", "host-b", "cruiser"),
        HullOutcome::NotClaimant
    );
    assert_eq!(
        held.confirm_hull(&slots(), "lead", "host-a", "destroyer"),
        HullOutcome::HullNotAllowed
    );
    assert_eq!(
        held.confirm_hull(&slots(), "lead", "host-a", "cruiser"),
        HullOutcome::Confirmed
    );
    assert!(held.claimed_are_confirmed());
}

#[test]
fn multi_ship_unclaimed_policy_is_frozen_once_and_survives_round_trip() {
    let mut authored = slots();
    authored.push(ShipSlotConfig {
        id: "wing".into(),
        label: None,
        ships: authored[0].ships.clone(),
        default_ship: "cruiser".into(),
        unclaimed: UnclaimedSlotPolicy::Absent,
    });
    let frozen = ShipSlotReservations::default().freeze(&authored).unwrap();
    assert_eq!(frozen.0.len(), 1);
    assert_eq!(frozen.0[0].source, LaunchSource::Backfill);
    assert_eq!(frozen.0[0].slot_id, "lead");
    let json = serde_json::to_string(&frozen).unwrap();
    assert_eq!(
        serde_json::from_str::<FrozenShipSlots>(&json).unwrap(),
        frozen
    );
}

#[test]
fn workshop_test_controls_one_slot_and_backfills_or_omits_the_others() {
    let mut authored = slots();
    authored.push(ShipSlotConfig {
        id: "wing".into(),
        label: None,
        ships: vec![AvailableShipEntry {
            template_path: "destroyer".into(),
            label: None,
        }],
        default_ship: "destroyer".into(),
        unclaimed: UnclaimedSlotPolicy::Absent,
    });
    authored.push(ShipSlotConfig {
        id: "reserve".into(),
        label: None,
        ships: vec![AvailableShipEntry {
            template_path: "frigate".into(),
            label: None,
        }],
        default_ship: "frigate".into(),
        unclaimed: UnclaimedSlotPolicy::Backfill,
    });
    let frozen = FrozenShipSlots::for_workshop_test(&authored, "wing", "destroyer").unwrap();
    assert_eq!(
        frozen
            .0
            .iter()
            .map(|row| (row.slot_id.as_str(), row.source))
            .collect::<Vec<_>>(),
        [
            ("lead", LaunchSource::Backfill),
            ("wing", LaunchSource::Claimed),
            ("reserve", LaunchSource::Backfill)
        ]
    );
    assert_eq!(frozen.0[1].hull, "destroyer");
    assert!(FrozenShipSlots::for_workshop_test(&authored, "wing", "cruiser").is_err());
}

#[test]
fn multi_ship_fleet_roster_freezes_claimed_backfill_and_absent_slots() {
    use crate::command_admission::HostSlot;
    use crate::lockstep::{FleetRoster, FleetShip};

    let mut authored = slots();
    authored.push(ShipSlotConfig {
        id: "wing".into(),
        label: None,
        ships: vec![AvailableShipEntry {
            template_path: "destroyer".into(),
            label: None,
        }],
        default_ship: "destroyer".into(),
        unclaimed: UnclaimedSlotPolicy::Backfill,
    });
    authored.push(ShipSlotConfig {
        id: "reserve".into(),
        label: None,
        ships: vec![AvailableShipEntry {
            template_path: "scout".into(),
            label: None,
        }],
        default_ship: "scout".into(),
        unclaimed: UnclaimedSlotPolicy::Absent,
    });
    let roster = FleetRoster::new(
        vec![FleetShip {
            host: HostSlot(1),
            ship_path: Some("cruiser".into()),
            authored_slot_id: Some("lead".into()),
            crew: Vec::new(),
        }],
        HostSlot(1),
    );
    let frozen = FrozenShipSlots::from_fleet_roster(&authored, &roster).unwrap();
    assert_eq!(frozen.0.len(), 2);
    assert_eq!(frozen.0[0].slot_id, "lead");
    assert_eq!(frozen.0[0].source, LaunchSource::Claimed);
    assert_eq!(frozen.0[1].slot_id, "wing");
    assert_eq!(frozen.0[1].source, LaunchSource::Backfill);
}

#[test]
fn a_gm_backfill_adds_an_empty_slot_in_authored_order_once() {
    let mut authored = slots();
    authored.push(ShipSlotConfig {
        id: "wing".into(),
        label: None,
        ships: vec![AvailableShipEntry {
            template_path: "wing.toml".into(),
            label: None,
        }],
        default_ship: "wing.toml".into(),
        unclaimed: UnclaimedSlotPolicy::Absent,
    });
    let mut frozen = FrozenShipSlots(vec![LaunchedSlot {
        slot_id: "lead".into(),
        hull: "ship.toml".into(),
        claimant: Some("host-1".into()),
        source: LaunchSource::Claimed,
    }]);
    assert_eq!(
        frozen.backfill_slot(&authored, "wing"),
        BackfillSlotOutcome::Applied
    );
    assert_eq!(
        frozen.0[1],
        LaunchedSlot {
            slot_id: "wing".into(),
            hull: "wing.toml".into(),
            claimant: None,
            source: LaunchSource::Backfill
        }
    );
    assert_eq!(
        frozen.backfill_slot(&authored, "wing"),
        BackfillSlotOutcome::AlreadyPresent
    );
    assert_eq!(
        frozen.backfill_slot(&authored, "missing"),
        BackfillSlotOutcome::UnknownSlot
    );
}

#[test]
fn multi_ship_curation_refuses_an_excluded_default_and_an_empty_slot() {
    let authored = slots();
    assert!(curate_ship_slots(&authored, &["destroyer".into()])
        .unwrap_err()
        .contains("offers no hull"));

    let mut two = authored;
    two[0].ships.push(AvailableShipEntry {
        template_path: "destroyer".into(),
        label: None,
    });
    assert!(curate_ship_slots(&two, &["destroyer".into()])
        .unwrap_err()
        .contains("default"));
}
