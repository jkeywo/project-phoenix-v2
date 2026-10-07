use super::*;

#[test]
fn identical_policy_preserves_value_and_reset_removes_bucket() {
    let mut rows = BTreeMap::new();
    assert!(set(
        &mut rows,
        "observer",
        "target",
        Some(7),
        |policy| (policy, "sample"),
        |value| &value.0
    ));
    assert!(!set(
        &mut rows,
        "observer",
        "target",
        Some(7),
        |_| panic!("unchanged policy must not construct"),
        |value| &value.0
    ));
    assert_eq!(rows["observer"]["target"].1, "sample");
    assert!(set(
        &mut rows,
        "observer",
        "target",
        None,
        |policy| (policy, "new"),
        |value| &value.0
    ));
    assert!(rows.is_empty());
    assert!(!set(
        &mut rows,
        "observer",
        "target",
        None,
        |policy| (policy, "new"),
        |value| &value.0
    ));
}

#[test]
fn pruning_requires_player_observer_but_accepts_any_live_target() {
    let mut rows = BTreeMap::from([
        (
            "player".into(),
            BTreeMap::from([("npc".into(), 1), ("gone".into(), 2)]),
        ),
        ("npc".into(), BTreeMap::from([("player".into(), 3)])),
        ("gone".into(), BTreeMap::from([("player".into(), 4)])),
    ]);
    let live = BTreeMap::from([("player", true), ("npc", false)]);
    prune(&mut rows, &live);
    assert_eq!(
        rows,
        BTreeMap::from([("player".into(), BTreeMap::from([("npc".into(), 1)]))])
    );
}
