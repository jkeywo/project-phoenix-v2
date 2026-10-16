use super::*;
use phoenix_model::console::{ConsoleFeed, ConsoleObservationKind};
use phoenix_model::messages::{ScanBlackboard, SystemBlackboard};
fn original(key: &SystemId) -> Option<SystemId> {
    let (size, owner) = key.0.strip_prefix("scan/")?.split_once(':')?;
    let size = size.parse::<usize>().ok()?;
    if owner.len() != size || size == 0 || size > 128 || owner.chars().any(char::is_control) {
        return None;
    }
    let owner = SystemId(owner.into());
    (phoenix_model::sensing::scan_blackboard_key_for(&owner) == *key).then_some(owner)
}

fn original_feed(feed: &ConsoleFeed, source: &SystemId, board: &SystemBlackboard) -> bool {
    &feed.source == source
        && feed.kind == ConsoleObservationKind::of(board)
        && match board {
            SystemBlackboard::Scan(scan) => {
                original(source).as_ref() == scan.owner.as_ref() && scan.owner.is_some()
            }
            _ => true,
        }
}
fn original_get<'a>(
    view: &ConsoleKnowledgeView<'a>,
    source: &SystemId,
) -> Option<&'a SystemBlackboard> {
    let console = view.console?;
    let value = view.publications.get(source)?;
    let derived_scan = if let SystemBlackboard::Scan(scan) = value {
        let owner = original(source)?;
        if scan.owner.as_ref() != Some(&owner) {
            return None;
        }
        Some(owner)
    } else {
        None
    };
    let owned = view
        .config
        .system(derived_scan.as_ref().unwrap_or(source))
        .is_some_and(|system| system.console.as_ref() == Some(&console.id));
    (owned
        || console
            .feeds
            .iter()
            .any(|feed| original_feed(feed, source, value)))
    .then_some(value)
}
#[test]
fn borrowed_scan_knowledge_matches_original_full_permission_predicate() {
    let original_config: ShipConfig = toml::from_str(
        r#"
[[console]]
id = "owner"
[[console]]
id = "visitor"
[[system]]
id = "é"
kind = "phaser_bank"
console = "owner"
"#,
    )
    .unwrap();
    for duplicate_first in [false, true] {
        for key in [
            "scan/2:é",
            "scan/02:é",
            "scan/+2:é",
            "scan/1:é",
            "scan/0:",
            "é",
            "scan/1:x",
        ] {
            for reported in [None, Some(SystemId("é".into())), Some(SystemId("x".into()))] {
                for feed_kind in [
                    None,
                    Some(ConsoleObservationKind::Scan),
                    Some(ConsoleObservationKind::Helm),
                ] {
                    let mut config = original_config.clone();
                    if duplicate_first {
                        let mut duplicate = config.systems[0].clone();
                        duplicate.console = Some(ConsoleId("visitor".into()));
                        config.systems.insert(0, duplicate);
                    }
                    let source = SystemId(key.into());
                    if let Some(kind) = feed_kind {
                        config.consoles[1].feeds.push(ConsoleFeed {
                            source: source.clone(),
                            kind,
                        });
                    }
                    for board in [
                        SystemBlackboard::Scan(ScanBlackboard {
                            owner: reported.clone(),
                            ..Default::default()
                        }),
                        SystemBlackboard::Helm(Default::default()),
                    ] {
                        let publications =
                            std::collections::HashMap::from([(source.clone(), board)]);
                        for console in [None, config.consoles.first(), config.consoles.get(1)] {
                            let view = ConsoleKnowledgeView {
                                config: &config,
                                console,
                                publications: &publications,
                            };
                            assert_eq!(view.get(&source), original_get(&view, &source));
                            assert_eq!(view.get(&SystemId("missing".into())), None);
                        }
                    }
                    // Information grants cannot change the first authored control owner.
                    assert_eq!(
                        config
                            .console_for_system(&SystemId("é".into()))
                            .unwrap()
                            .id
                            .0,
                        if duplicate_first { "visitor" } else { "owner" }
                    );
                }
            }
        }
    }
}
