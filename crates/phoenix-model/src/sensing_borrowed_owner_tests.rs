use super::*;
use crate::console::{ConsoleFeed, ConsoleObservationKind};
use crate::messages::{ScanBlackboard, SystemBlackboard};
fn original(key: &SystemId) -> Option<SystemId> {
    let (size, owner) = key.0.strip_prefix("scan/")?.split_once(':')?;
    let size = size.parse::<usize>().ok()?;
    if owner.len() != size || size == 0 || size > 128 || owner.chars().any(char::is_control) {
        return None;
    }
    let owner = SystemId(owner.into());
    (scan_blackboard_key_for(&owner) == *key).then_some(owner)
}

#[test]
fn borrowed_owner_matches_original_parser_and_feed_for_malformed_numeric_unicode_keys() {
    let mut owners = vec![
        String::new(),
        "a".into(),
        "a:b/c".into(),
        "é".into(),
        "🚀".into(),
        "a".repeat(127),
        "a".repeat(128),
        "a".repeat(129),
        "é".repeat(64),
        "é".repeat(65),
    ];
    owners.extend(
        (0..=0x9f)
            .filter_map(char::from_u32)
            .map(|c| format!("a{c}")),
    );
    let mut sizes = vec![
        String::new(),
        usize::MAX.to_string(),
        format!("{}0", usize::MAX),
    ];
    for length in 0..=140 {
        sizes.extend([
            length.to_string(),
            format!("0{length}"),
            format!("+{length}"),
            format!("-{length}"),
            format!(" {length}"),
            format!("{length} "),
        ]);
    }
    // Exhaust all short tokens from a numeric/malformed alphabet.
    let alphabet = ['0', '1', '2', '9', '+', '-', ' ', '１'];
    for a in alphabet {
        for b in alphabet {
            for c in alphabet {
                sizes.push(format!("{a}{b}{c}"));
            }
        }
    }
    for owner in owners {
        for size in &sizes {
            let key = SystemId(format!("scan/{size}:{owner}"));
            let expected = original(&key);
            assert_eq!(scan_owner_from_blackboard_key(&key), expected, "{key:?}");
            assert_eq!(
                scan_owner_from_blackboard_key_borrowed(&key),
                expected.as_ref().map(|id| id.0.as_str())
            );
            for reported in [
                None,
                Some(SystemId(owner.clone())),
                Some(SystemId("wrong".into())),
            ] {
                let board = SystemBlackboard::Scan(ScanBlackboard {
                    owner: reported.clone(),
                    ..Default::default()
                });
                for kind in [ConsoleObservationKind::Scan, ConsoleObservationKind::Helm] {
                    let feed = ConsoleFeed {
                        source: key.clone(),
                        kind,
                    };
                    let accepted = kind == ConsoleObservationKind::of(&board)
                        && original(&key).as_ref() == reported.as_ref()
                        && reported.is_some();
                    assert_eq!(feed.accepts(&key, &board), accepted);
                    assert!(!feed.accepts(&SystemId(format!("other/{}", key.0)), &board));
                }
            }
        }
    }
    for key in ["", "scan", "SCAN/1:a", "scan/1", "/scan/1:a", "scan//1:a"] {
        let key = SystemId(key.into());
        assert_eq!(scan_owner_from_blackboard_key(&key), original(&key));
    }
}
