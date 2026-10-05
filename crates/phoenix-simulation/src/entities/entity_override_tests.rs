use super::*;

#[test]
fn behaviour_state_is_retired_and_no_longer_reconciles() {
    for policy in [MergePolicy::InstanceOverride, MergePolicy::ComposeFragments] {
        assert_eq!(
            policy.array_rule("behaviour.state"),
            ArrayRule::Replace,
            "the FSM was dissolved in #572; a resolved document carrying \
                 [[behaviour.state]] does not parse, so there is nothing to \
                 reconcile ({policy:?})"
        );
    }
    let parsed = crate::entities::config::EntityConfig::from_toml(
        "[behaviour]\n[[behaviour.state]]\nname = \"patrol\"\n",
    );
    assert!(
        parsed.is_err(),
        "if this ever parses, `behaviour.state` is real again and belongs \
             back in an identity table"
    );
}

// ── merge_entity_config_toml: behaviour.doctrine by-id semantics ──────

#[test]
fn a_tombstone_in_a_world_override_fails_against_the_real_hull() {
    use crate::entities::loader::{apply_overrides, TemplateLoader};

    const HULL: &str = "assets/entities/ship_harrow_patrol.toml";
    let hull = crate::entities::loader::FsTemplateLoader
        .load_template(HULL)
        .unwrap_or_else(|| panic!("{HULL} must load — this test is about the MERGE"));
    let doctrine_ids = |c: &crate::entities::config::EntityConfig| -> Vec<String> {
        c.behaviour
            .as_ref()
            .map(|b| b.doctrine.iter().map(|d| d.id.clone()).collect())
            .unwrap_or_default()
    };
    let before = doctrine_ids(&hull);
    assert!(
        before.len() >= 2,
        "{HULL} must ship more than one doctrine entry or the tombstone has \
             nothing to silently fail to remove, got {before:?}"
    );

    let tombstone: toml::Value = toml::from_str(&format!(
        "[[behaviour.doctrine]]\nid = {:?}\n{REMOVE_KEY} = true\n",
        before[0]
    ))
    .unwrap();
    let err = apply_overrides(&hull, &tombstone)
        .expect_err("a tombstone in a world override must fail LOUDLY");
    assert!(
        err.contains(REMOVE_KEY),
        "the diagnostic must name the marker so the author can find it, got {err:?}"
    );

    // …and the lever that DOES work here still does, on the same hull.
    let cleared = apply_overrides(
        &hull,
        &toml::from_str("behaviour = { doctrine = [] }").unwrap(),
    )
    .expect("the authored empty array is an instance override's subtractive lever");
    assert!(
        doctrine_ids(&cleared).is_empty(),
        "clearing the array is what an author must write instead of a tombstone"
    );
}
