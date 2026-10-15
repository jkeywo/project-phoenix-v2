use super::*;

#[test]
fn focused_channels_are_canonical_utf8_owner_scoped_and_feed_typed() {
    use crate::{
        console::{ConsoleFeed, ConsoleObservationKind},
        messages::{ScanBlackboard, SystemBlackboard},
    };
    for text in ["sensors", "array:/one", "é-array"] {
        let owner = SystemId(text.into());
        let channel = scan_blackboard_key_for(&owner);
        assert_eq!(
            scan_owner_from_blackboard_key(&channel),
            Some(owner.clone())
        );
        let feed = ConsoleFeed {
            source: channel.clone(),
            kind: ConsoleObservationKind::Scan,
        };
        let board = SystemBlackboard::Scan(ScanBlackboard {
            owner: Some(owner),
            ..Default::default()
        });
        assert!(feed.accepts(&channel, &board));
        let wrong = SystemBlackboard::Scan(ScanBlackboard {
            owner: Some(SystemId("foreign".into())),
            ..Default::default()
        });
        assert!(!feed.accepts(&channel, &wrong));
    }
    for malformed in [
        "scan",
        "scan/0:",
        "scan/07:sensors",
        "scan/6:sensors",
        "scan/7:sensors\n",
        "scan/1:é",
    ] {
        assert!(
            scan_owner_from_blackboard_key(&SystemId(malformed.into())).is_none(),
            "{malformed}"
        );
    }
}
