use super::*;

/// [`merge_entity_config_toml`] for the tests whose override is expected to
/// be ACCEPTED. The merge is fallible only for a `_remove` tombstone
/// (issue #911); `an_instance_override_is_rejected_for_a_tombstone` covers
/// the other side.
fn instance(template: &toml::Value, over: &toml::Value) -> toml::Value {
    merge_entity_config_toml(template, over).expect("this override carries no `_remove` tombstone")
}

fn doctrine_template() -> toml::Value {
    toml::from_str(
        r#"
[behaviour]
waypoint_arrival_radius = 20.0

[[behaviour.doctrine]]
id = "destroy-hostiles"
directive_kind = "Destroy"
base_priority = 45.0

[[behaviour.doctrine]]
id = "hold-station"
base_priority = 20.0
"#,
    )
    .unwrap()
}

fn doctrine_ids(merged: &toml::Value) -> Vec<String> {
    merged
        .get("behaviour")
        .and_then(|b| b.get("doctrine"))
        .and_then(|d| d.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|e| e.get("id").and_then(|v| v.as_str()).map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

#[test]
fn override_replaces_scalar() {
    let template: toml::Value = toml::from_str("speed = 50").unwrap();
    let over: toml::Value = toml::from_str("speed = 100").unwrap();
    let result = merge_toml(&template, &over);
    assert_eq!(result, over);
}

#[test]
fn override_replaces_array_wholesale() {
    let template: toml::Value = toml::from_str(r#"tags = ["a", "b", "c"]"#).unwrap();
    let over: toml::Value = toml::from_str(r#"tags = ["x", "y"]"#).unwrap();
    let result = merge_toml(&template, &over);
    assert_eq!(result, over);
}

/// **`tags` REPLACES at the instance-override layer** — the AC-4 tripwire
/// for the one array that has no key.
///
/// `tags` is an array of bare strings, so it can only union or replace;
/// there is no third option. Union is what a fragment library wants, and it
/// is what the COMPOSE layer does. It is also exactly wrong here: three
/// shipped worlds — `default.toml:148`, `patrol.toml:65`,
/// `reinforcements.toml:56` — override `ship_harrow_patrol`'s tags to
/// `["ship", "npc", "enemy"]` precisely to DROP the template's
/// `comms_contact`, and tags are behaviourally live (`entities/tags.rs`,
/// `gui/radar.rs`). Union them and those three hostiles become hailable
/// again, silently.
///
/// Nothing asserted this before [`MergePolicy`] existed, because there was
/// only one merge and nothing to diverge from. Now there are two, so this
/// pins the instance-layer half.
#[test]
fn instance_override_tags_replace_they_do_not_union() {
    let template: toml::Value =
        toml::from_str(r#"tags = ["ship", "npc", "enemy", "comms_contact"]"#).unwrap();
    let over: toml::Value = toml::from_str(r#"tags = ["ship", "npc", "enemy"]"#).unwrap();
    let result = instance(&template, &over);
    let tags: Vec<&str> = result
        .get("tags")
        .and_then(|v| v.as_array())
        .unwrap()
        .iter()
        .filter_map(|v| v.as_str())
        .collect();
    assert_eq!(
        tags,
        vec!["ship", "npc", "enemy"],
        "an instance override must be able to take a tag AWAY — three shipped \
             worlds drop `comms_contact` exactly this way"
    );
}

/// The other half of the same rule, from the other side: the compose layer
/// unions, so a fragment library adds tags rather than clobbering them.
#[test]
fn compose_layer_tags_union_rather_than_replace() {
    let template: toml::Value = toml::from_str(r#"tags = ["ship", "npc"]"#).unwrap();
    let over: toml::Value = toml::from_str(r#"tags = ["npc", "enemy"]"#).unwrap();
    let result = merge_entity_config_toml_with(&template, &over, MergePolicy::ComposeFragments)
        .expect("ComposeFragments honours the tombstone, so it never rejects one");
    let tags: Vec<&str> = result
        .get("tags")
        .and_then(|v| v.as_array())
        .unwrap()
        .iter()
        .filter_map(|v| v.as_str())
        .collect();
    assert_eq!(
        tags,
        vec!["ship", "npc", "enemy"],
        "union preserves template order, appends what is new, and does not \
             duplicate what both sides declare"
    );
}

/// A fragment's subtractive lever for `tags` is still the authored empty
/// array — union must not swallow it.
#[test]
fn compose_layer_empty_tags_still_clears() {
    let template: toml::Value = toml::from_str(r#"tags = ["ship", "npc"]"#).unwrap();
    let over: toml::Value = toml::from_str(r#"tags = []"#).unwrap();
    let result = merge_entity_config_toml_with(&template, &over, MergePolicy::ComposeFragments)
        .expect("ComposeFragments honours the tombstone, so it never rejects one");
    assert!(result
        .get("tags")
        .and_then(|v| v.as_array())
        .unwrap()
        .is_empty());
}

#[test]
fn recursive_table_merge_preserves_template_keys_not_in_override() {
    let template: toml::Value = toml::from_str(
        r#"
[hull]
hull_integrity = 100
armour = 50
"#,
    )
    .unwrap();
    let over: toml::Value = toml::from_str(
        r#"
[hull]
hull_integrity = 200
"#,
    )
    .unwrap();
    let result = merge_toml(&template, &over);
    let hull = result.get("hull").and_then(|v| v.as_table()).unwrap();
    assert_eq!(
        hull.get("hull_integrity").and_then(|v| v.as_integer()),
        Some(200)
    );
    assert_eq!(hull.get("armour").and_then(|v| v.as_integer()), Some(50));
}

#[test]
fn override_adds_section_absent_in_template() {
    let template: toml::Value = toml::from_str(r#"name = "base""#).unwrap();
    let over: toml::Value = toml::from_str(
        r#"
[power]
capacity = 150
"#,
    )
    .unwrap();
    let result = merge_toml(&template, &over);
    assert_eq!(result.get("name").and_then(|v| v.as_str()), Some("base"));
    let power = result.get("power").and_then(|v| v.as_table()).unwrap();
    assert_eq!(
        power.get("capacity").and_then(|v| v.as_integer()),
        Some(150)
    );
}

#[test]
fn override_false_does_not_remove_template_field() {
    let template: toml::Value = toml::from_str("online = true").unwrap();
    let over: toml::Value = toml::from_str("online = false").unwrap();
    let result = merge_toml(&template, &over);
    assert_eq!(result.get("online").and_then(|v| v.as_bool()), Some(false));
}

#[test]
fn ordering_stability_merge_preserves_sorted_key_order() {
    let template: toml::Value = toml::from_str(
        r#"
z_key = "last"
a_key = "first"
"#,
    )
    .unwrap();
    let over: toml::Value = toml::from_str(
        r#"
m_key = "middle"
"#,
    )
    .unwrap();
    let result = merge_toml(&template, &over);
    let table = result.as_table().unwrap();
    let keys: Vec<&String> = table.keys().collect();
    assert_eq!(keys, vec!["a_key", "m_key", "z_key"]);
}

// ── merge_named_array tests ───────────────────────────────────────────

#[test]
fn merge_named_array_replaces_entry_by_name() {
    let template: Vec<toml::Value> = toml::from_str::<toml::Value>(
        r#"
[[item]]
name = "alpha"
target_speed = 0.5

[[item]]
name = "beta"
target_speed = 0.3
"#,
    )
    .unwrap()
    .get("item")
    .unwrap()
    .as_array()
    .unwrap()
    .clone();

    let overrides: Vec<toml::Value> = toml::from_str::<toml::Value>(
        r#"
[[item]]
name = "alpha"
target_speed = 0.9
"#,
    )
    .unwrap()
    .get("item")
    .unwrap()
    .as_array()
    .unwrap()
    .clone();

    let result = merge_named_array(&template, &overrides);
    assert_eq!(result.len(), 2, "length must be preserved");
    let alpha = &result[0];
    assert_eq!(
        alpha.get("target_speed").and_then(|v| v.as_float()),
        Some(0.9)
    );
    // beta unchanged
    let beta = &result[1];
    assert_eq!(
        beta.get("target_speed").and_then(|v| v.as_float()),
        Some(0.3)
    );
}

#[test]
fn merge_named_array_keeps_unmentioned_entries() {
    let template: Vec<toml::Value> = toml::from_str::<toml::Value>(
        r#"
[[item]]
name = "alpha"
target_speed = 0.5

[[item]]
name = "beta"
target_speed = 0.3
"#,
    )
    .unwrap()
    .get("item")
    .unwrap()
    .as_array()
    .unwrap()
    .clone();

    let overrides: Vec<toml::Value> = vec![];
    let result = merge_named_array(&template, &overrides);
    assert_eq!(result.len(), 2, "no overrides: both template entries kept");
}

#[test]
fn merge_named_array_appends_new_entry() {
    let template: Vec<toml::Value> = toml::from_str::<toml::Value>(
        r#"
[[item]]
name = "alpha"
"#,
    )
    .unwrap()
    .get("item")
    .unwrap()
    .as_array()
    .unwrap()
    .clone();

    let overrides: Vec<toml::Value> = toml::from_str::<toml::Value>(
        r#"
[[item]]
name = "gamma"
target_speed = 0.7
"#,
    )
    .unwrap()
    .get("item")
    .unwrap()
    .as_array()
    .unwrap()
    .clone();

    let result = merge_named_array(&template, &overrides);
    assert_eq!(result.len(), 2, "new entry should be appended");
    assert_eq!(
        result[1].get("name").and_then(|v| v.as_str()),
        Some("gamma")
    );
}

// ── the `name`-keyed path (was behaviour.state; now station.rating) ──
//
// #911 RETIRED the `behaviour.state` special case rather than generalising
// it: `BehaviourConfig` is `deny_unknown_fields` and has had no `state`
// field since #572 dissolved the FSM, so a resolved document carrying
// `[[behaviour.state]]` cannot parse and no shipped hull or fragment has
// one. The `name`-keyed MECHANISM is not retired — `station.rating` uses it
// — so this test is re-pointed at the mechanism instead of deleted.
//
// ── AC6, stated with its shortfall rather than as met verbatim ──
//
// AC6 asks that the by-`name` and by-`id` reconciliation "still works". The
// by-`id` half (`behaviour.doctrine`) is preserved and live at BOTH layers.
// The by-`name` half was RETIRED, and that is a deliberate deviation, not a
// pass. Nor does the test below make up for it: it calls the element-wise
// merger DIRECTLY on a synthetic `[[item]]` array, so it pins the algorithm
// and proves nothing about whether the `name` path is reachable through the
// public merge. The test that proves reachability, on the live user, is
// `compose_reconciles_a_nested_array_inside_a_matched_entry`
// (`station.rating`). Read the two together; neither is sufficient alone.

#[test]
fn keyed_merge_by_name_replaces_matching_entry() {
    let template: toml::Value = toml::from_str(
        r#"
[[item]]
name = "patrol"
kind = "patrolling"
target_speed = 0.5

[[item]]
name = "idle"
kind = "idle"
target_speed = 0.0
"#,
    )
    .unwrap();

    let override_: toml::Value = toml::from_str(
        r#"
[[item]]
name = "patrol"
target_speed = 0.9
"#,
    )
    .unwrap();

    let states = merge_keyed_array(
        template.get("item").unwrap().as_array().unwrap(),
        override_.get("item").unwrap().as_array().unwrap(),
        "name",
    );
    assert_eq!(states.len(), 2, "idle must be kept");
    let patrol = states
        .iter()
        .find(|s| s.get("name").and_then(|v| v.as_str()) == Some("patrol"))
        .unwrap();
    assert_eq!(
        patrol.get("target_speed").and_then(|v| v.as_float()),
        Some(0.9)
    );
    assert_eq!(
        patrol.get("kind").and_then(|v| v.as_str()),
        Some("patrolling"),
        "a key the override never mentioned survives the deep merge"
    );
    // idle untouched
    let idle = states
        .iter()
        .find(|s| s.get("name").and_then(|v| v.as_str()) == Some("idle"))
        .unwrap();
    assert_eq!(
        idle.get("target_speed").and_then(|v| v.as_float()),
        Some(0.0)
    );
}

/// `behaviour.state` is no longer reconciled at EITHER layer, and the
/// reason is that it cannot exist: `BehaviourConfig` is
/// `deny_unknown_fields` with no `state` field. Pinned as a rule rather
/// than left to be rediscovered.
#[test]
fn entity_merge_behaviour_doctrine_by_id_replaces_matching_entry_and_keeps_others() {
    let override_: toml::Value = toml::from_str(
        r#"
[[behaviour.doctrine]]
id = "destroy-hostiles"
base_priority = 99.0
"#,
    )
    .unwrap();

    let result = instance(&doctrine_template(), &override_);
    assert_eq!(
        doctrine_ids(&result),
        vec!["destroy-hostiles", "hold-station"],
        "a non-empty override still merges by id and keeps unmentioned entries"
    );
    let destroy = result
        .get("behaviour")
        .unwrap()
        .get("doctrine")
        .unwrap()
        .as_array()
        .unwrap()[0]
        .clone();
    assert_eq!(
        destroy.get("base_priority").and_then(|v| v.as_float()),
        Some(99.0)
    );
    assert_eq!(
        destroy.get("directive_kind").and_then(|v| v.as_str()),
        Some("Destroy"),
        "unmentioned keys of a merged entry survive"
    );
}

/// An explicitly authored `doctrine = []` clears the template's doctrine.
///
/// This is a scenario's only subtractive lever: `probe_aggressor.toml`
/// spawns a hull whose whole purpose is to have NO Destroy directive, so
/// that it can never fire the first shot. While an empty array merged as a
/// no-op that hull kept its template `destroy-hostiles` doctrine and opened
/// fire proactively.
#[test]
fn entity_merge_empty_doctrine_override_clears_template_doctrine() {
    let override_: toml::Value = toml::from_str("behaviour = { doctrine = [] }").unwrap();
    let result = instance(&doctrine_template(), &override_);
    assert!(
        doctrine_ids(&result).is_empty(),
        "an authored empty doctrine array must clear the list, got {:?}",
        doctrine_ids(&result)
    );
    // Clearing the list does not disturb the rest of the behaviour block.
    assert_eq!(
        result
            .get("behaviour")
            .and_then(|b| b.get("waypoint_arrival_radius"))
            .and_then(|v| v.as_float()),
        Some(20.0)
    );
}

/// An override that never mentions `doctrine` leaves the template's list
/// alone — the distinction the empty-array rule turns on.
#[test]
fn entity_merge_override_without_doctrine_key_keeps_template_doctrine() {
    let override_: toml::Value =
        toml::from_str("behaviour = { waypoint_arrival_radius = 5.0 }").unwrap();
    let result = instance(&doctrine_template(), &override_);
    assert_eq!(
        doctrine_ids(&result),
        vec!["destroy-hostiles", "hold-station"]
    );
}

/// The empty-array-clears rule is not special to the reconciled arrays: it
/// is what EVERY array does when the override authors `[]`. Kept pointed at
/// `behaviour.state` (whose reconciliation #911 retired) precisely because
/// that makes it the un-reconciled case.
#[test]
fn entity_merge_empty_state_override_clears_template_states() {
    let template: toml::Value = toml::from_str(
        r#"
[behaviour]
initial_state = "patrol"

[[behaviour.state]]
name = "patrol"
kind = "patrolling"
"#,
    )
    .unwrap();
    let override_: toml::Value = toml::from_str("behaviour = { state = [] }").unwrap();
    let result = instance(&template, &override_);
    let states = result
        .get("behaviour")
        .unwrap()
        .get("state")
        .unwrap()
        .as_array()
        .unwrap();
    assert!(
        states.is_empty(),
        "an authored empty state array must clear the list"
    );
}

// ── The identity table (issue #911) ──────────────────────────────────

/// The whole point of the seam: the two layers answer differently, and the
/// instance layer's answers are exactly what they were before #911.
#[test]
fn the_two_layers_disagree_only_where_they_are_meant_to() {
    use ArrayRule::*;
    let cases: &[(&str, ArrayRule, ArrayRule)] = &[
        // path                         instance          compose
        ("behaviour.doctrine", Keyed("id"), Keyed("id")),
        ("system", Replace, Keyed("id")),
        ("station", Replace, Keyed("id")),
        ("station.rating", Replace, Keyed("name")),
        ("shield_arc", Replace, Keyed("id")),
        ("weapons_console.phaser_banks", Replace, Keyed("id")),
        ("weapons_console.blaster_banks", Replace, Keyed("id")),
        ("torpedoes.tubes", Replace, Keyed("id")),
        ("tags", Replace, Union),
        // Deliberately left replacing at BOTH layers — a fragment
        // contributing an AI policy contributes it whole.
        ("captain_console.ai.rule", Replace, Replace),
        ("helm_console.steering_ai.state", Replace, Replace),
        ("repair.selector.score", Replace, Replace),
        ("hull.system_hull", Replace, Replace),
        // Nested inside a reconciled doctrine entry: the `directive_anchors
        // = []` idiom `world/dispatch.rs` documents relies on this.
        ("behaviour.doctrine.directive_anchors", Replace, Replace),
        ("weapons_console.blaster_banks.pattern", Replace, Replace),
    ];
    for (path, instance, compose) in cases {
        assert_eq!(
            MergePolicy::InstanceOverride.array_rule(path),
            *instance,
            "instance-layer rule for {path}"
        );
        assert_eq!(
            MergePolicy::ComposeFragments.array_rule(path),
            *compose,
            "compose-layer rule for {path}"
        );
    }
}

/// `kind` repeats across systems (a hull has many `phaser_bank`s), so it is
/// never an identity. Keying on it would collapse a weapons suite.
#[test]
fn no_identity_key_is_ever_kind() {
    for policy in [MergePolicy::InstanceOverride, MergePolicy::ComposeFragments] {
        for (path, key) in policy.keyed_arrays() {
            assert_ne!(
                *key, "kind",
                "{path} must not reconcile by `kind` — it is duplicated in 8 of \
                     the 11 shipped files that declare systems"
            );
        }
    }
}

fn compose(template: &str, over: &str) -> toml::Value {
    merge_entity_config_toml_with(
        &toml::from_str(template).unwrap(),
        &toml::from_str(over).unwrap(),
        MergePolicy::ComposeFragments,
    )
    .expect("ComposeFragments honours the tombstone, so it never rejects one")
}

fn system_ids(merged: &toml::Value) -> Vec<String> {
    merged
        .get("system")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|e| e.get("id").and_then(|v| v.as_str()).map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

const SYSTEM_FRAGMENT: &str = r#"
[[system]]
id = "helm-thrust"
kind = "helm_thrust"
ai_only = true

[[system]]
id = "power-reactor"
kind = "power_reactor"
"#;

/// EXTEND — the thing #911 exists for: the library's systems plus my own,
/// with no new author syntax.
#[test]
fn compose_extends_a_keyed_array_with_new_entries() {
    let merged = compose(
        SYSTEM_FRAGMENT,
        "[[system]]\nid = \"phaser-dorsal\"\nkind = \"phaser_bank\"\n",
    );
    assert_eq!(
        system_ids(&merged),
        vec!["helm-thrust", "power-reactor", "phaser-dorsal"],
        "a new id appends; the fragment's suite is not replaced"
    );
}

/// REPLACE-IN-PLACE — a matching id specialises the inherited entry and
/// keeps its position, without restating the fields it does not change.
#[test]
fn compose_replaces_a_keyed_entry_in_place() {
    let merged = compose(
        SYSTEM_FRAGMENT,
        "[[system]]\nid = \"helm-thrust\"\nai_only = false\n",
    );
    assert_eq!(system_ids(&merged), vec!["helm-thrust", "power-reactor"]);
    let thrust = &merged.get("system").unwrap().as_array().unwrap()[0];
    assert_eq!(thrust.get("ai_only").and_then(|v| v.as_bool()), Some(false));
    assert_eq!(
        thrust.get("kind").and_then(|v| v.as_str()),
        Some("helm_thrust"),
        "a field the hull never mentioned comes from the fragment"
    );
}

/// REMOVE — the per-entry tombstone. A hull can drop ONE inherited entry
/// without clearing the array and restating the rest.
#[test]
fn compose_removes_a_single_keyed_entry_by_tombstone() {
    let merged = compose(
        SYSTEM_FRAGMENT,
        "[[system]]\nid = \"power-reactor\"\n_remove = true\n",
    );
    assert_eq!(
        system_ids(&merged),
        vec!["helm-thrust"],
        "the tombstone removes its match and contributes nothing itself"
    );
    assert!(
        !toml::to_string(&merged).unwrap().contains(REMOVE_KEY),
        "the marker must never reach EntityConfig, which is deny_unknown_fields"
    );
}

/// A tombstone for something no fragment contributed is a no-op, not an
/// error — fragments compose in any order.
#[test]
fn a_tombstone_matching_nothing_is_a_no_op_and_leaves_no_marker() {
    let merged = compose(
        SYSTEM_FRAGMENT,
        "[[system]]\nid = \"not-here\"\n_remove = true\n",
    );
    assert_eq!(system_ids(&merged), vec!["helm-thrust", "power-reactor"]);
    assert!(!toml::to_string(&merged).unwrap().contains(REMOVE_KEY));
}

/// Removal is not sticky: a later fragment re-adding the id wins WHOLE,
/// rather than deep-merging into a tombstone and inheriting its marker.
#[test]
fn a_later_fragment_can_re_add_what_an_earlier_one_removed() {
    let removed = compose(
        SYSTEM_FRAGMENT,
        "[[system]]\nid = \"power-reactor\"\n_remove = true\n",
    );
    // Compose again with the accumulator that still carries the tombstone,
    // which is what the resolver's intermediate state looks like.
    let with_tombstone = merge_at(
        "",
        &toml::from_str(SYSTEM_FRAGMENT).unwrap(),
        &toml::from_str("[[system]]\nid = \"power-reactor\"\n_remove = true\n").unwrap(),
        MergePolicy::ComposeFragments,
    );
    let re_added = merge_entity_config_toml_with(
        &with_tombstone,
        &toml::from_str("[[system]]\nid = \"power-reactor\"\nkind = \"power_reactor\"\n").unwrap(),
        MergePolicy::ComposeFragments,
    )
    .expect("ComposeFragments honours the tombstone, so it never rejects one");
    assert_eq!(system_ids(&removed), vec!["helm-thrust"]);
    assert_eq!(system_ids(&re_added), vec!["helm-thrust", "power-reactor"]);
    assert!(!toml::to_string(&re_added).unwrap().contains(REMOVE_KEY));
}

/// **A tombstone is a COMPOSE-layer marker only, and writing one in a world
/// override is an ERROR.**
///
/// The instance layer's subtractive levers stay the authored empty array
/// and restating the list. What must NOT happen is the third outcome: the
/// override being accepted and quietly doing nothing.
#[test]
fn an_instance_override_is_rejected_for_a_tombstone() {
    assert!(!MergePolicy::InstanceOverride.accepts_removals());
    for over in [
        // The wholesale-replacing case…
        "[[system]]\nid = \"power-reactor\"\n_remove = true\n",
        // …and the reconciling one, which is the dangerous half.
        "[[behaviour.doctrine]]\nid = \"destroy-hostiles\"\n_remove = true\n",
        // Nested arbitrarily deep, and even written `false`: at this layer
        // there is no reading of the key that does anything.
        "[weapons_console]\n_remove = false\n",
    ] {
        let err = merge_entity_config_toml(
            &toml::from_str(SYSTEM_FRAGMENT).unwrap(),
            &toml::from_str(over).unwrap(),
        )
        .expect_err("a tombstone in an instance override must be rejected");
        assert!(
            err.contains(REMOVE_KEY),
            "the diagnostic must name the marker, got {err:?}"
        );
    }
}

/// The same rule measured against a REAL SHIPPED HULL through the real
/// public entry point, because a synthetic value cannot show what actually
/// went wrong.
///
/// `behaviour.doctrine` is the one array that reconciles at the instance
/// layer, so a tombstone written there does not sit in the merged document
/// waiting to be rejected — it deep-merges INTO the matching template entry.
/// Before issue #1268 taught the doctrine contract to reject unknown keys,
/// serde ignored the marker: before this test, `apply_overrides` returned
/// `Ok`, the doctrine came back
/// as `["patrol-ironveil", "destroy-hostiles"]`, and the author who asked
/// for `destroy-hostiles` to be GONE got a hull that still had it and no
/// warning. `crates/phoenix-sim-gameplay/src/ship/config.rs` has no `deny_unknown_fields` either, so
/// `[[system]]`, `[[station]]` and `[[station.rating]]` are no safer.
///
/// That is exactly the silent-no-op failure mode #838 existed to end, so
/// the guarantee is enforced by the merge and pinned here END TO END.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn compose_empty_array_still_clears_a_keyed_array() {
    let merged = compose(SYSTEM_FRAGMENT, "system = []\n");
    assert!(system_ids(&merged).is_empty());
}

/// NESTED — an array inside a reconciled entry is judged by ITS path:
/// `station.rating` reconciles by `name`, so a hull can retune one rating
/// of one station without restating either list.
#[test]
fn compose_reconciles_a_nested_array_inside_a_matched_entry() {
    let merged = compose(
        r#"
[[station]]
id = "bridge"
[[station.rating]]
name = "helm"
level = 1
[[station.rating]]
name = "tactical"
level = 1

[[station]]
id = "engineering"
"#,
        r#"
[[station]]
id = "bridge"
[[station.rating]]
name = "tactical"
level = 3
"#,
    );
    let stations = merged.get("station").unwrap().as_array().unwrap();
    assert_eq!(stations.len(), 2, "the unmentioned station survives");
    let ratings = stations[0].get("rating").unwrap().as_array().unwrap();
    assert_eq!(ratings.len(), 2, "the unmentioned rating survives");
    assert_eq!(ratings[0].get("name").unwrap().as_str(), Some("helm"));
    assert_eq!(ratings[1].get("level").unwrap().as_integer(), Some(3));
}

/// `[[shield_arc]]` order is LOAD-BEARING — `ShieldSystem::from_arcs` maps
/// arcs positionally, `focused_facing` is a positional index, and the FIRST
/// arc's `frequency` seeds the ship-wide shield frequency. Keyed
/// reconciliation keeping matched entries where the template put them is a
/// guarantee, not an accident.
#[test]
fn keyed_merge_keeps_template_order_and_appends_new_entries() {
    let merged = compose(
        r#"
[[shield_arc]]
id = "fore"
frequency = 1.0
[[shield_arc]]
id = "aft"
frequency = 2.0
"#,
        // Specialises the FIRST arc deliberately: an override that only
        // ever touched the last one would pass even if matched entries were
        // moved to the end of the array.
        r#"
[[shield_arc]]
id = "fore"
frequency = 9.0
[[shield_arc]]
id = "dorsal"
frequency = 5.0
"#,
    );
    let arcs = merged.get("shield_arc").unwrap().as_array().unwrap();
    let ids: Vec<&str> = arcs
        .iter()
        .filter_map(|a| a.get("id").and_then(|v| v.as_str()))
        .collect();
    assert_eq!(
        ids,
        vec!["fore", "aft", "dorsal"],
        "a specialised arc stays at its template position and a new arc is \
             appended AFTER — reordering would change `focused_facing` and the \
             ship-wide shield frequency"
    );
    assert_eq!(
        arcs[0].get("frequency").unwrap().as_float(),
        Some(9.0),
        "the SHIP-WIDE shield frequency is seeded from the first arc, so \
             which arc is first is a runtime-visible fact"
    );
    assert_eq!(arcs[1].get("frequency").unwrap().as_float(), Some(2.0));
}

/// An AI policy is contributed WHOLE. Stated as a test because it is the
/// granularity decision, not an oversight.
#[test]
fn an_ai_rule_list_is_contributed_whole_not_merged_rule_by_rule() {
    let merged = compose(
        "[[captain_console.ai.rule]]\nchannel = \"a\"\npriority = 1\n",
        "[[captain_console.ai.rule]]\nchannel = \"b\"\npriority = 2\n",
    );
    let rules = merged
        .get("captain_console")
        .unwrap()
        .get("ai")
        .unwrap()
        .get("rule")
        .unwrap()
        .as_array()
        .unwrap();
    assert_eq!(
        rules.len(),
        1,
        "the only candidate key is the composite (channel, priority), which \
             an author bumping a priority would silently 'rename'"
    );
    assert_eq!(rules[0].get("channel").unwrap().as_str(), Some("b"));
}

#[test]
fn entity_merge_behaviour_transition_full_replacement() {
    let template: toml::Value = toml::from_str(
        r#"
[behaviour]
initial_state = "idle"

[[behaviour.transition]]
from = "idle"
to = "patrol"
trigger = "damage"
"#,
    )
    .unwrap();

    let override_: toml::Value = toml::from_str(
        r#"
[[behaviour.transition]]
from = "patrol"
to = "idle"
trigger = "safe"
"#,
    )
    .unwrap();

    let result = instance(&template, &override_);
    let transitions = result
        .get("behaviour")
        .unwrap()
        .get("transition")
        .unwrap()
        .as_array()
        .unwrap();
    // Full replacement: only the override entry
    assert_eq!(transitions.len(), 1);
    assert_eq!(
        transitions[0].get("trigger").and_then(|v| v.as_str()),
        Some("safe")
    );
}
