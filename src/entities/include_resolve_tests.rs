use super::*;
use std::collections::HashMap;

fn src(pairs: &[(&str, &str)]) -> HashMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

fn resolve(root: &str, pairs: &[(&str, &str)]) -> ResolvedTemplate {
    resolve_template(root, &src(pairs)).expect("fixture must resolve")
}

// ── Ordered precedence ───────────────────────────────────────────────────

#[test]
fn includer_wins_over_its_fragment() {
    let r = resolve(
        "e/hull.toml",
        &[
            (
                "e/base.toml",
                "class = \"escort\"\n[hull]\nhull_integrity = 100.0\n",
            ),
            (
                "e/hull.toml",
                "includes = [\"base.toml\"]\n[hull]\nhull_integrity = 500.0\n",
            ),
        ],
    );
    assert_eq!(
        r.value
            .get("hull")
            .unwrap()
            .get("hull_integrity")
            .unwrap()
            .as_float(),
        Some(500.0),
        "the declaring template merges last, so it wins"
    );
    assert_eq!(
        r.value.get("class").unwrap().as_str(),
        Some("escort"),
        "a field only the fragment sets survives"
    );
}

#[test]
fn later_include_wins_over_earlier_include() {
    let r = resolve(
        "e/hull.toml",
        &[
            ("e/a.toml", "class = \"a\"\nhull_id = \"from-a\"\n"),
            ("e/b.toml", "class = \"b\"\n"),
            ("e/hull.toml", "includes = [\"a.toml\", \"b.toml\"]\n"),
        ],
    );
    assert_eq!(
        r.value.get("class").unwrap().as_str(),
        Some("b"),
        "includes merge in declared order, so the later one wins"
    );
    assert_eq!(
        r.value.get("hull_id").unwrap().as_str(),
        Some("from-a"),
        "a field the later include does not mention keeps the earlier value"
    );
}

/// The mutation guard for precedence: if the includer were merged FIRST
/// (or the include list were walked in reverse), one of these two
/// assertions has to break. They pin opposite ends of the order.
#[test]
fn precedence_order_is_fragments_then_declarer() {
    let r = resolve(
        "e/hull.toml",
        &[
            (
                "e/a.toml",
                "class = \"a\"\nhull_id = \"a\"\npower_rating = 1\n",
            ),
            ("e/b.toml", "class = \"b\"\nhull_id = \"b\"\n"),
            (
                "e/hull.toml",
                "includes = [\"a.toml\", \"b.toml\"]\nclass = \"self\"\n",
            ),
        ],
    );
    assert_eq!(r.value.get("class").unwrap().as_str(), Some("self"));
    assert_eq!(r.value.get("hull_id").unwrap().as_str(), Some("b"));
    assert_eq!(r.value.get("power_rating").unwrap().as_integer(), Some(1));
    assert_eq!(
        r.provenance.sources(),
        vec!["e/a.toml", "e/b.toml", "e/hull.toml"],
        "merge order must be depth-first in declared order, declarer last"
    );
}

// ── Nested includes ──────────────────────────────────────────────────────

#[test]
fn nested_includes_resolve_depth_first() {
    let r = resolve(
        "e/hull.toml",
        &[
            ("e/deep.toml", "class = \"deep\"\nhull_id = \"deep\"\n"),
            (
                "e/mid.toml",
                "includes = [\"deep.toml\"]\nclass = \"mid\"\n",
            ),
            ("e/hull.toml", "includes = [\"mid.toml\"]\n"),
        ],
    );
    assert_eq!(
        r.provenance.sources(),
        vec!["e/deep.toml", "e/mid.toml", "e/hull.toml"],
        "a fragment's own includes are merged before the fragment itself"
    );
    assert_eq!(r.value.get("class").unwrap().as_str(), Some("mid"));
    assert_eq!(r.value.get("hull_id").unwrap().as_str(), Some("deep"));
}

#[test]
fn a_fragment_included_twice_is_merged_twice_not_rejected() {
    // A diamond is legal: two fragments may both build on a common base.
    let r = resolve(
        "e/hull.toml",
        &[
            ("e/base.toml", "class = \"base\"\n"),
            ("e/a.toml", "includes = [\"base.toml\"]\nhull_id = \"a\"\n"),
            ("e/b.toml", "includes = [\"base.toml\"]\npower_rating = 2\n"),
            ("e/hull.toml", "includes = [\"a.toml\", \"b.toml\"]\n"),
        ],
    );
    assert_eq!(r.value.get("class").unwrap().as_str(), Some("base"));
    assert_eq!(r.value.get("hull_id").unwrap().as_str(), Some("a"));
    assert_eq!(r.value.get("power_rating").unwrap().as_integer(), Some(2));
}

// ── Relative paths ───────────────────────────────────────────────────────

#[test]
fn include_paths_resolve_relative_to_the_declaring_template() {
    let r = resolve(
        "assets/entities/hull.toml",
        &[
            ("assets/entities/frag/a.toml", "class = \"a\"\n"),
            (
                "assets/entities/shared/b.toml",
                // relative to `assets/entities/frag/`, NOT to the root hull
                "class = \"b\"\nhull_id = \"b\"\n",
            ),
            (
                "assets/entities/hull.toml",
                "includes = [\"frag/a.toml\", \"./shared/b.toml\"]\n",
            ),
        ],
    );
    assert_eq!(r.value.get("hull_id").unwrap().as_str(), Some("b"));
    assert_eq!(
        r.provenance.sources(),
        vec![
            "assets/entities/frag/a.toml",
            "assets/entities/shared/b.toml",
            "assets/entities/hull.toml"
        ]
    );
}

#[test]
fn a_nested_fragment_resolves_its_own_includes_relative_to_itself() {
    let r = resolve(
        "assets/entities/hull.toml",
        &[
            ("assets/shared/core.toml", "class = \"core\"\n"),
            (
                "assets/entities/frag/mid.toml",
                "includes = [\"../../shared/core.toml\"]\nhull_id = \"mid\"\n",
            ),
            (
                "assets/entities/hull.toml",
                "includes = [\"frag/mid.toml\"]\n",
            ),
        ],
    );
    assert_eq!(r.value.get("class").unwrap().as_str(), Some("core"));
    assert_eq!(
        r.provenance.sources()[0],
        "assets/shared/core.toml",
        "`..` must be collapsed against the DECLARING fragment's directory"
    );
}

#[test]
fn canonical_include_path_collapses_dot_segments() {
    assert_eq!(
        canonical_include_path("a/b/hull.toml", "./frag/../frag/x.toml").as_deref(),
        Some("a/b/frag/x.toml")
    );
    assert_eq!(
        canonical_include_path("a/b/hull.toml", "..\\shared\\x.toml").as_deref(),
        Some("a/shared/x.toml"),
        "backslashes are normalised so a Windows-authored path resolves identically"
    );
}

#[test]
fn canonical_include_path_rejects_absolute_references() {
    assert!(canonical_include_path("a/hull.toml", "/etc/passwd.toml").is_none());
    assert!(canonical_include_path("a/hull.toml", "C:\\x\\y.toml").is_none());
    assert!(canonical_include_path("a/hull.toml", "   ").is_none());
}

// ── Named / id array behaviour ───────────────────────────────────────────

#[test]
fn doctrine_merges_by_id_across_fragments() {
    let r = resolve(
        "e/hull.toml",
        &[
            (
                "e/base.toml",
                r#"
[behaviour]
[[behaviour.doctrine]]
id = "destroy-hostiles"
directive_kind = "Destroy"
base_priority = 40.0
[[behaviour.doctrine]]
id = "hold-station"
base_priority = 10.0
"#,
            ),
            (
                "e/hull.toml",
                r#"
includes = ["base.toml"]
[[behaviour.doctrine]]
id = "destroy-hostiles"
base_priority = 90.0
"#,
            ),
        ],
    );
    let doctrine = r
        .value
        .get("behaviour")
        .unwrap()
        .get("doctrine")
        .unwrap()
        .as_array()
        .unwrap();
    assert_eq!(doctrine.len(), 2, "unmentioned entries survive the merge");
    assert_eq!(
        doctrine[0].get("base_priority").unwrap().as_float(),
        Some(90.0),
        "the includer's entry replaces the fragment's by id"
    );
    assert_eq!(
        doctrine[0].get("directive_kind").unwrap().as_str(),
        Some("Destroy"),
        "keys the includer did not mention survive"
    );
}

/// `behaviour.state` was reconciled by `name` before #911; it is not any
/// more, and it must not be.
///
/// The FSM was dissolved in #572: `BehaviourConfig` is
/// `deny_unknown_fields` with no `state` field, so a resolved document
/// carrying `[[behaviour.state]]` cannot parse and no shipped hull or
/// fragment has one. #911 retired the special case rather than generalising
/// a corpse. The `name`-keyed MECHANISM is alive and tested through
/// `[[station.rating]]` — see `nested_arrays_reconcile_under_a_composed_chain`.
///
/// Re-pointed rather than deleted so the retirement is a checked claim: if
/// `state` ever comes back, this fails and sends the author to the identity
/// table.
#[test]
fn state_is_retired_and_no_longer_merges_by_name_across_fragments() {
    let r = resolve(
        "e/hull.toml",
        &[
            (
                "e/base.toml",
                r#"
[behaviour]
[[behaviour.state]]
name = "patrol"
target_speed = 0.5
[[behaviour.state]]
name = "idle"
target_speed = 0.0
"#,
            ),
            (
                "e/hull.toml",
                r#"
includes = ["base.toml"]
[[behaviour.state]]
name = "patrol"
target_speed = 0.9
"#,
            ),
        ],
    );
    let states = r
        .value
        .get("behaviour")
        .unwrap()
        .get("state")
        .unwrap()
        .as_array()
        .unwrap();
    assert_eq!(
        states.len(),
        1,
        "an array with no identity entry replaces wholesale"
    );
    assert_eq!(states[0].get("name").unwrap().as_str(), Some("patrol"));
    assert!(
        EntityConfig::from_toml(&r.toml).is_err(),
        "and the resolved document does not parse either way — which is why \
             there was nothing to generalise"
    );
}

/// `68bda1be`'s empty-array rule has to mean something coherent between
/// fragments too: an authored empty array CLEARS, an omitted key does not.
#[test]
fn a_fragment_authoring_an_empty_doctrine_clears_what_came_before() {
    let r = resolve(
        "e/hull.toml",
        &[
            (
                "e/armed.toml",
                r#"
[behaviour]
waypoint_arrival_radius = 20.0
[[behaviour.doctrine]]
id = "destroy-hostiles"
base_priority = 40.0
"#,
            ),
            (
                "e/hull.toml",
                "includes = [\"armed.toml\"]\nbehaviour = { doctrine = [] }\n",
            ),
        ],
    );
    let doctrine = r
        .value
        .get("behaviour")
        .unwrap()
        .get("doctrine")
        .unwrap()
        .as_array()
        .unwrap();
    assert!(
        doctrine.is_empty(),
        "an explicitly authored empty array is a fragment's only subtractive lever"
    );
    assert_eq!(
        r.value
            .get("behaviour")
            .unwrap()
            .get("waypoint_arrival_radius")
            .unwrap()
            .as_float(),
        Some(20.0),
        "clearing one list must not disturb the rest of the block"
    );
}

#[test]
fn omitting_doctrine_leaves_the_fragments_list_alone() {
    let r = resolve(
        "e/hull.toml",
        &[
            (
                "e/armed.toml",
                "[behaviour]\n[[behaviour.doctrine]]\nid = \"kill\"\nbase_priority = 40.0\n",
            ),
            (
                "e/hull.toml",
                "includes = [\"armed.toml\"]\n[behaviour]\nwaypoint_arrival_radius = 5.0\n",
            ),
        ],
    );
    let doctrine = r
        .value
        .get("behaviour")
        .unwrap()
        .get("doctrine")
        .unwrap()
        .as_array()
        .unwrap();
    assert_eq!(doctrine.len(), 1, "an absent key never reaches the merge");
}

fn strings_at(r: &ResolvedTemplate, key: &str) -> Vec<String> {
    r.value
        .get(key)
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str())
                .map(String::from)
                .collect()
        })
        .unwrap_or_default()
}

fn ids_at(r: &ResolvedTemplate, key: &str) -> Vec<String> {
    r.value
        .get(key)
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|e| e.get("id").and_then(|v| v.as_str()).map(String::from))
                .collect()
        })
        .unwrap_or_default()
}

/// #869 asserted that `tags` REPLACED between fragments. #911 changes that
/// deliberately: `tags` has no key, so at the compose layer it can only
/// union, and union is what a fragment library needs.
///
/// This is a behaviour change confined to the compose layer.
/// `entity_override::instance_override_tags_replace_they_do_not_union` pins
/// the other half, which is the one three shipped worlds depend on.
#[test]
fn tags_union_between_fragments() {
    let r = resolve(
        "e/hull.toml",
        &[
            ("e/base.toml", "tags = [\"ship\", \"npc\"]\n"),
            (
                "e/hull.toml",
                "includes = [\"base.toml\"]\ntags = [\"npc\", \"scenery\"]\n",
            ),
        ],
    );
    assert_eq!(
        strings_at(&r, "tags"),
        vec!["ship", "npc", "scenery"],
        "the hull ADDS to the fragment's tags; a tag both declare is not \
             duplicated"
    );
}

/// …and an authored empty array is still a fragment's lever to clear them.
#[test]
fn a_fragment_authoring_empty_tags_clears_them() {
    let r = resolve(
        "e/hull.toml",
        &[
            ("e/base.toml", "tags = [\"ship\", \"npc\"]\n"),
            ("e/hull.toml", "includes = [\"base.toml\"]\ntags = []\n"),
        ],
    );
    assert!(strings_at(&r, "tags").is_empty());
}

/// Arrays with no stable identity keep replacing wholesale between
/// fragments. A fragment contributing an AI policy contributes it WHOLE.
#[test]
fn keyless_arrays_still_replace_wholesale_between_fragments() {
    let r = resolve(
            "e/hull.toml",
            &[
                (
                    "e/base.toml",
                    "[[captain_console.ai.rule]]\nchannel = \"a\"\npriority = 1\n",
                ),
                (
                    "e/hull.toml",
                    "includes = [\"base.toml\"]\n[[captain_console.ai.rule]]\nchannel = \"b\"\npriority = 2\n",
                ),
            ],
        );
    let rules = r
        .value
        .get("captain_console")
        .unwrap()
        .get("ai")
        .unwrap()
        .get("rule")
        .unwrap()
        .as_array()
        .unwrap();
    assert_eq!(rules.len(), 1);
    assert_eq!(rules[0].get("channel").unwrap().as_str(), Some("b"));
}

// ── Array extension across a composed chain (issue #911) ─────────────────

/// The issue in one test: "the library's systems, plus two of my own."
#[test]
fn a_hull_extends_a_fragments_system_suite_instead_of_replacing_it() {
    let r = resolve(
        "e/hull.toml",
        &[
            (
                "e/systems.toml",
                "[[system]]\nid = \"helm-thrust\"\nkind = \"helm_thrust\"\n\
                     [[system]]\nid = \"power-reactor\"\nkind = \"power_reactor\"\n",
            ),
            (
                "e/hull.toml",
                "includes = [\"systems.toml\"]\n\
                     [[system]]\nid = \"phaser-dorsal\"\nkind = \"phaser_bank\"\n",
            ),
        ],
    );
    assert_eq!(
        ids_at(&r, "system"),
        vec!["helm-thrust", "power-reactor", "phaser-dorsal"],
        "a hull needing one extra system no longer has to restate the suite \
             — and so no longer silently opts out of future library changes"
    );
}

/// Replace-in-place and remove, through a THREE-deep chain, so the rules
/// are shown to survive an intermediate fragment rather than only working
/// between a hull and its direct include.
#[test]
fn a_composed_chain_specialises_and_removes_inherited_entries() {
    let r = resolve(
        "e/hull.toml",
        &[
            (
                "e/library.toml",
                "[[system]]\nid = \"helm-thrust\"\nkind = \"helm_thrust\"\nai_only = true\n\
                     [[system]]\nid = \"power-reactor\"\nkind = \"power_reactor\"\n\
                     [[system]]\nid = \"legacy-probe\"\nkind = \"sensor_probe\"\n",
            ),
            (
                "e/class.toml",
                "includes = [\"library.toml\"]\n\
                     [[system]]\nid = \"legacy-probe\"\n_remove = true\n\
                     [[system]]\nid = \"phaser-dorsal\"\nkind = \"phaser_bank\"\n",
            ),
            (
                "e/hull.toml",
                "includes = [\"class.toml\"]\n\
                     [[system]]\nid = \"helm-thrust\"\nai_only = false\n",
            ),
        ],
    );
    assert_eq!(
        ids_at(&r, "system"),
        vec!["helm-thrust", "power-reactor", "phaser-dorsal"],
        "the mid fragment's removal and append both survive to the hull"
    );
    let thrust = &r.value.get("system").unwrap().as_array().unwrap()[0];
    assert_eq!(thrust.get("ai_only").unwrap().as_bool(), Some(false));
    assert_eq!(
        thrust.get("kind").unwrap().as_str(),
        Some("helm_thrust"),
        "a key only the library declared survives two levels of merge"
    );
    assert!(
        !r.toml
            .contains(crate::entities::entity_override::REMOVE_KEY),
        "the tombstone marker must never reach the resolved document"
    );
}

/// A tombstone in the FIRST fragment of a closure never meets a merge — it
/// is the value the accumulator is seeded with. It must still be stripped,
/// which is what the resolver's own strip site is for.
#[test]
fn an_unmatched_tombstone_never_reaches_the_resolved_document() {
    let r = resolve(
        "e/hull.toml",
        &[
            (
                "e/frag.toml",
                "[[system]]\nid = \"never-declared\"\n_remove = true\n\
                     [[system]]\nid = \"real\"\nkind = \"power_reactor\"\n",
            ),
            ("e/hull.toml", "includes = [\"frag.toml\"]\nclass = \"x\"\n"),
        ],
    );
    assert_eq!(ids_at(&r, "system"), vec!["real"]);
    assert!(!r
        .toml
        .contains(crate::entities::entity_override::REMOVE_KEY));
    assert!(!r
        .value
        .to_string()
        .contains(crate::entities::entity_override::REMOVE_KEY));
}

/// The one document the merge cannot clean: an UNCOMPOSED root with a
/// tombstone. It never meets an accumulator, so the resolver's own strip
/// site is the only thing standing between it and `value`.
///
/// Authoring a tombstone here is a mistake (there is nothing inherited to
/// remove), and the resolved `toml` is served verbatim from `root_text`, so
/// `parse()` rejects it loudly. What must NOT happen is `value` and `toml`
/// disagreeing about whether the marker is there.
#[test]
fn an_uncomposed_template_never_leaks_a_tombstone_into_its_value() {
    let body = "class = \"solo\"\n[[system]]\nid = \"ghost\"\n_remove = true\n";
    let r = resolve("e/hull.toml", &[("e/hull.toml", body)]);
    assert!(!r.is_composed());
    assert!(
        !r.value
            .to_string()
            .contains(crate::entities::entity_override::REMOVE_KEY),
        "no `_remove` may survive into the resolved value, composed or not"
    );
    assert_eq!(
        r.toml, body,
        "an uncomposed template is still served verbatim — byte-identity is \
             not traded away for the strip"
    );
    assert!(
        r.parse().is_err(),
        "and the verbatim bytes still carry the marker, so the mistake is \
             rejected rather than silently absorbed"
    );
}

/// Nested arrays under a composed chain: `[[station.rating]]` reconciles by
/// `name` INSIDE a `[[station]]` reconciled by `id`.
#[test]
fn nested_arrays_reconcile_under_a_composed_chain() {
    let r = resolve(
        "e/hull.toml",
        &[
            (
                "e/frag.toml",
                "[[station]]\nid = \"bridge\"\n\
                     [[station.rating]]\nname = \"helm\"\nlevel = 1\n\
                     [[station.rating]]\nname = \"tactical\"\nlevel = 1\n\
                     [[station]]\nid = \"engineering\"\n",
            ),
            (
                "e/hull.toml",
                "includes = [\"frag.toml\"]\n\
                     [[station]]\nid = \"bridge\"\n\
                     [[station.rating]]\nname = \"tactical\"\nlevel = 3\n",
            ),
        ],
    );
    assert_eq!(ids_at(&r, "station"), vec!["bridge", "engineering"]);
    let ratings = r.value.get("station").unwrap().as_array().unwrap()[0]
        .get("rating")
        .unwrap()
        .as_array()
        .unwrap();
    assert_eq!(ratings.len(), 2, "the unmentioned rating survives");
    assert_eq!(ratings[0].get("level").unwrap().as_integer(), Some(1));
    assert_eq!(ratings[1].get("level").unwrap().as_integer(), Some(3));
}

/// `[[shield_arc]]` order is load-bearing (`ShieldSystem::from_arcs` maps
/// arcs positionally; the FIRST arc's frequency seeds the ship-wide shield
/// frequency). Stated as a guarantee of composition, not left to chance.
#[test]
fn shield_arc_order_survives_composition() {
    let r = resolve(
        "e/hull.toml",
        &[
            (
                "e/frag.toml",
                "[[shield_arc]]\nid = \"fore\"\nfrequency = 1.0\n\
                     [[shield_arc]]\nid = \"aft\"\nfrequency = 2.0\n",
            ),
            (
                // Specialises the FIRST arc: an override that only touched
                // the last one would pass even if matched entries moved.
                "e/hull.toml",
                "includes = [\"frag.toml\"]\n\
                     [[shield_arc]]\nid = \"fore\"\nfrequency = 9.0\n\
                     [[shield_arc]]\nid = \"dorsal\"\nfrequency = 5.0\n",
            ),
        ],
    );
    assert_eq!(
        ids_at(&r, "shield_arc"),
        vec!["fore", "aft", "dorsal"],
        "specialised arcs hold their template position; new arcs append AFTER"
    );
    let arcs = r.value.get("shield_arc").unwrap().as_array().unwrap();
    assert_eq!(
        arcs[0].get("frequency").unwrap().as_float(),
        Some(9.0),
        "the ship-wide shield frequency is seeded from whichever arc is \
             FIRST, so composition must not reorder them"
    );
    assert_eq!(arcs[1].get("id").unwrap().as_str(), Some("aft"));
}

/// Provenance is driven from the SAME identity table as the merge. If it
/// were not, `system` would be recorded as a wholesale leaf and
/// `insert_leaf`'s prune would erase every field the library fragment
/// contributed to the systems it did not touch.
#[test]
fn provenance_addresses_every_reconciled_array_by_key_not_just_doctrine() {
    let r = resolve(
        "e/hull.toml",
        &[
            (
                "e/frag.toml",
                "[[system]]\nid = \"helm-thrust\"\nkind = \"helm_thrust\"\nai_only = true\n\
                     [[system]]\nid = \"power-reactor\"\nkind = \"power_reactor\"\n",
            ),
            (
                "e/hull.toml",
                "includes = [\"frag.toml\"]\n[[system]]\nid = \"helm-thrust\"\nai_only = false\n",
            ),
        ],
    );
    assert_eq!(
        r.provenance
            .origin("system[id=helm-thrust].ai_only")
            .expect("the hull's specialisation is recorded")
            .source,
        "e/hull.toml"
    );
    assert_eq!(
        r.provenance
            .origin("system[id=helm-thrust].kind")
            .expect("a key the hull never mentioned is still recorded")
            .source,
        "e/frag.toml",
        "if provenance recorded `system` as a wholesale leaf, this field \
             would have been pruned"
    );
    assert_eq!(
        r.provenance
            .origin("system[id=power-reactor].kind")
            .expect("an untouched sibling is still recorded")
            .source,
        "e/frag.toml"
    );
}

/// A removal is the opposite of authoring: provenance must stop reporting
/// fields of an entry that no longer exists.
#[test]
fn provenance_prunes_an_entry_a_later_fragment_removed() {
    let r = resolve(
        "e/hull.toml",
        &[
            (
                "e/frag.toml",
                "[[system]]\nid = \"legacy\"\nkind = \"sensor_probe\"\n",
            ),
            (
                "e/hull.toml",
                "includes = [\"frag.toml\"]\n[[system]]\nid = \"legacy\"\n_remove = true\n",
            ),
        ],
    );
    assert!(ids_at(&r, "system").is_empty());
    assert!(
        r.provenance.origin("system[id=legacy].kind").is_none(),
        "a removed entry must not still report a field that no longer exists"
    );
    assert!(
        r.provenance.origin("system[id=legacy]._remove").is_none(),
        "the marker is authoring metadata, not an authored field"
    );
}

// ── Cycles and missing paths ─────────────────────────────────────────────

#[test]
fn a_direct_cycle_is_a_load_error() {
    let err = resolve_template(
        "e/a.toml",
        &src(&[
            ("e/a.toml", "includes = [\"b.toml\"]\n"),
            ("e/b.toml", "includes = [\"a.toml\"]\n"),
        ]),
    )
    .expect_err("a cycle must not resolve");
    assert_eq!(err.category(), "include-cycle");
    assert_eq!(
        err.chain,
        vec!["e/a.toml", "e/b.toml", "e/a.toml"],
        "the error must name the chain that closed the loop"
    );
    assert!(err.finding.is_error(), "a cycle is never a warning");
}

#[test]
fn a_self_include_is_a_load_error() {
    let err = resolve_template(
        "e/a.toml",
        &src(&[("e/a.toml", "includes = [\"a.toml\"]\n")]),
    )
    .expect_err("a self-include must not resolve");
    assert_eq!(err.category(), "include-cycle");
    assert_eq!(err.chain, vec!["e/a.toml", "e/a.toml"]);
}

#[test]
fn a_cycle_through_relative_paths_is_still_detected() {
    // The two references spell the same file differently; canonicalisation
    // is what makes them one identity for cycle detection.
    let err = resolve_template(
        "e/a.toml",
        &src(&[
            ("e/a.toml", "includes = [\"./frag/b.toml\"]\n"),
            ("e/frag/b.toml", "includes = [\"../a.toml\"]\n"),
        ]),
    )
    .expect_err("a cycle spelled through `.`/`..` must still be caught");
    assert_eq!(err.category(), "include-cycle");
}

#[test]
fn a_missing_include_is_a_load_error_naming_the_chain() {
    let err = resolve_template(
        "e/a.toml",
        &src(&[
            ("e/a.toml", "includes = [\"mid.toml\"]\n"),
            ("e/mid.toml", "includes = [\"gone.toml\"]\n"),
        ]),
    )
    .expect_err("a missing fragment must not resolve");
    assert_eq!(err.category(), "include-missing");
    assert_eq!(err.chain, vec!["e/a.toml", "e/mid.toml", "e/gone.toml"]);
    assert_eq!(
        err.finding.source.file, "e/mid.toml",
        "the diagnostic points at the file that DECLARED the bad include"
    );
    assert!(err.to_string().contains("include chain"));
}

#[test]
fn a_fragment_that_is_not_valid_toml_is_a_load_error() {
    let err = resolve_template(
        "e/a.toml",
        &src(&[
            ("e/a.toml", "includes = [\"bad.toml\"]\n"),
            ("e/bad.toml", "this is not = = toml\n"),
        ]),
    )
    .expect_err("an unparseable fragment must not resolve");
    assert_eq!(err.category(), "include-parse");
    assert_eq!(err.chain, vec!["e/a.toml", "e/bad.toml"]);
}

#[test]
fn a_malformed_includes_declaration_is_a_load_error() {
    for body in ["includes = \"base.toml\"\n", "includes = [3]\n"] {
        let err = resolve_template("e/a.toml", &src(&[("e/a.toml", body)]))
            .expect_err("`includes` must be an array of path strings");
        assert_eq!(err.category(), "include-malformed", "for body {body:?}");
    }
}

#[test]
fn an_absolute_include_is_a_load_error() {
    let err = resolve_template(
        "e/a.toml",
        &src(&[("e/a.toml", "includes = [\"/etc/hull.toml\"]\n")]),
    )
    .expect_err("absolute include paths are not resolvable relative to the declarer");
    assert_eq!(err.category(), "include-malformed");
}

#[test]
fn a_resolved_template_that_is_not_a_valid_entity_is_a_load_error() {
    // A `Patrol` doctrine carrying `directive_anchors` is valid; so is a
    // bare `Destroy`. Reconciling them by id produces a `Destroy` directive
    // that still carries the Patrol-only field — which nothing rejects
    // until the RESOLVED document is validated, because the offending
    // combination exists in neither authored file.
    const FRAGMENT: &str = r#"
[behaviour]
[[behaviour.doctrine]]
id = "patrol-lane"
directive_kind = "Patrol"
directive_anchors = ["alpha"]
base_priority = 10.0
"#;
    const HULL: &str = r#"
[behaviour]
[[behaviour.doctrine]]
id = "patrol-lane"
directive_kind = "Destroy"
"#;
    // Lenient: these two snippets are doctrine fixtures, not hulls, and
    // the point is the RESOLVED document's doctrine reconciliation. Strict
    // AI-declaration mode would reject both for the fifteen declarations
    // neither was ever meant to carry — see `EntityConfig::from_toml_in_mode`.
    let lenient = crate::entities::ai_declaration_manifest::AiDeclarationMode::Lenient;
    assert!(
        EntityConfig::from_toml_in_mode(FRAGMENT, lenient).is_ok(),
        "fragment is valid alone"
    );
    assert!(
        EntityConfig::from_toml_in_mode(HULL, lenient).is_ok(),
        "hull is valid alone"
    );

    let hull_with_include = format!("includes = [\"base.toml\"]\n{HULL}");
    let resolved = resolve(
        "e/hull.toml",
        &[
            ("e/base.toml", FRAGMENT),
            ("e/hull.toml", hull_with_include.as_str()),
        ],
    );
    let err = resolved
        .parse()
        .expect_err("an invalid RESOLVED template must be rejected");
    assert_eq!(err.category(), "include-invalid-template");
    assert_eq!(
        err.chain,
        vec!["e/base.toml", "e/hull.toml"],
        "the error names every template that contributed"
    );
}

/// The `[[mesh.lod]]` relocation guard (issue #914) runs on the RESOLVED
/// document, so a fragment library entry that still authors the banned
/// location is caught exactly like a shipped hull would be — it cannot
/// hide behind composition and slip an old-style ladder into every hull
/// that includes it.
#[test]
fn a_fragment_carrying_relocated_mesh_lod_is_rejected_with_the_targeted_message() {
    const FRAGMENT: &str = r#"
[mesh]
model = "assets/models/rock.glb"
variant = "small"
shape = "sphere"
colour = [0.5, 0.5, 0.5]
radius = 2.0

[[mesh.lod]]
max_distance = 50.0
model = "assets/models/rock.glb"
"#;
    let resolved = resolve(
        "e/hull.toml",
        &[
            ("e/frag.toml", FRAGMENT),
            ("e/hull.toml", "includes = [\"frag.toml\"]\n"),
        ],
    );
    let err = resolved
        .parse()
        .expect_err("[[mesh.lod]] authored by a fragment must still be rejected");
    assert_eq!(err.category(), "include-invalid-template");
    assert!(
        err.message().contains("assets/models/rock.small.toml"),
        "the error must name the sidecar the chain moved to; got: {}",
        err.message()
    );
    assert!(
        err.message().contains("[[lod]]"),
        "the error must name the new block; got: {}",
        err.message()
    );
}

// ── Provenance ───────────────────────────────────────────────────────────

#[test]
fn provenance_names_the_fragment_that_authored_each_field() {
    let r = resolve(
        "e/hull.toml",
        &[
            ("e/base.toml", "class = \"escort\"\nhull_id = \"BASE\"\n"),
            (
                "e/hull.toml",
                "includes = [\"base.toml\"]\nhull_id = \"NCC-1\"\n",
            ),
        ],
    );
    assert_eq!(r.provenance.origin("class").unwrap().source, "e/base.toml");
    assert_eq!(
        r.provenance.origin("hull_id").unwrap().source,
        "e/hull.toml",
        "the LAST author of a field is the one that won the merge"
    );
    assert!(r.provenance.origin("power_rating").is_none());
}

#[test]
fn provenance_records_the_chain_that_reached_each_source() {
    let r = resolve(
        "e/hull.toml",
        &[
            ("e/deep.toml", "class = \"deep\"\n"),
            (
                "e/mid.toml",
                "includes = [\"deep.toml\"]\nhull_id = \"mid\"\n",
            ),
            ("e/hull.toml", "includes = [\"mid.toml\"]\n"),
        ],
    );
    assert_eq!(
        r.provenance.origin("class").unwrap().chain,
        vec!["e/hull.toml", "e/mid.toml", "e/deep.toml"],
        "the chain runs root-first down to the authoring fragment"
    );
    assert_eq!(
        r.provenance.origin("hull_id").unwrap().chain,
        vec!["e/hull.toml", "e/mid.toml"]
    );
}

#[test]
fn provenance_addresses_reconciled_array_elements_by_key() {
    let r = resolve(
            "e/hull.toml",
            &[
                (
                    "e/base.toml",
                    r#"
[behaviour]
[[behaviour.doctrine]]
id = "kill"
base_priority = 40.0
directive_kind = "Destroy"
"#,
                ),
                (
                    "e/hull.toml",
                    "includes = [\"base.toml\"]\n[[behaviour.doctrine]]\nid = \"kill\"\nbase_priority = 90.0\n",
                ),
            ],
        );
    assert_eq!(
        r.provenance
            .origin("behaviour.doctrine[id=kill].base_priority")
            .unwrap()
            .source,
        "e/hull.toml",
        "the overriding priority is attributed to the hull"
    );
    assert_eq!(
        r.provenance
            .origin("behaviour.doctrine[id=kill].directive_kind")
            .unwrap()
            .source,
        "e/base.toml",
        "a key the hull never mentioned stays attributed to the fragment"
    );
}

#[test]
fn provenance_drops_fields_a_later_fragment_replaced_wholesale() {
    let r = resolve(
        "e/hull.toml",
        &[
            (
                "e/base.toml",
                "[behaviour]\n[[behaviour.doctrine]]\nid = \"kill\"\nbase_priority = 40.0\n",
            ),
            (
                "e/hull.toml",
                "includes = [\"base.toml\"]\nbehaviour = { doctrine = [] }\n",
            ),
        ],
    );
    assert!(
        r.provenance
            .origin("behaviour.doctrine[id=kill].base_priority")
            .is_none(),
        "a cleared list must not still report a field that no longer exists"
    );
    assert_eq!(
        r.provenance.origin("behaviour.doctrine").unwrap().source,
        "e/hull.toml",
        "the clear itself is attributed to the fragment that authored it"
    );
}

#[test]
fn provenance_of_an_uncomposed_template_is_the_template_itself() {
    let r = resolve("e/hull.toml", &[("e/hull.toml", "class = \"solo\"\n")]);
    assert!(!r.is_composed());
    assert_eq!(r.provenance.sources(), vec!["e/hull.toml"]);
    assert_eq!(
        r.provenance.origin("class").unwrap().chain,
        vec!["e/hull.toml"]
    );
}

// ── Byte stability ───────────────────────────────────────────────────────

#[test]
fn an_uncomposed_template_resolves_to_its_own_bytes_verbatim() {
    let body = "# a comment\nclass  =  \"solo\"\n\n[hull]\nhull_integrity = 10.0\n";
    let r = resolve("e/hull.toml", &[("e/hull.toml", body)]);
    assert_eq!(
        r.toml, body,
        "a template with no includes must not be reformatted — its raw text is \
             what marker validation and line lookups read"
    );
}

#[test]
fn resolution_is_byte_stable_across_runs_and_delivery_order() {
    let pairs = [
        ("e/a.toml", "class = \"a\"\nhull_id = \"a\"\n"),
        ("e/b.toml", "power_rating = 3\nclass = \"b\"\n"),
        (
            "e/hull.toml",
            "includes = [\"a.toml\", \"b.toml\"]\nname = \"H\"\n",
        ),
    ];
    let first = resolve("e/hull.toml", &pairs);
    // A different insertion order into the source map — the hash map's
    // iteration order changes, the resolved bytes must not.
    let mut reordered: Vec<(&str, &str)> = pairs.to_vec();
    reordered.reverse();
    let second = resolve("e/hull.toml", &reordered);
    assert_eq!(first.toml, second.toml);
    assert_eq!(
        first.toml,
        resolve("e/hull.toml", &pairs).toml,
        "resolving the same inputs twice must produce identical bytes"
    );
    assert!(first.is_composed());
    assert!(
        !first.toml.contains(INCLUDES_KEY),
        "the resolved document must not carry the authoring key into the runtime"
    );
}

#[test]
fn the_resolved_document_never_carries_an_includes_key() {
    let r = resolve(
        "e/hull.toml",
        &[
            ("e/deep.toml", "class = \"deep\"\n"),
            ("e/mid.toml", "includes = [\"deep.toml\"]\n"),
            ("e/hull.toml", "includes = [\"mid.toml\"]\n"),
        ],
    );
    assert!(r.value.get(INCLUDES_KEY).is_none());
    assert!(!r.toml.contains(INCLUDES_KEY));
}

// ── Preload contract (the shape both hosts share) ────────────────────────

#[test]
fn preload_step_reports_the_paths_still_to_fetch() {
    let delivered = src(&[(
        "e/hull.toml",
        "includes = [\"frag/a.toml\", \"frag/b.toml\"]\n",
    )]);
    let step = preload_step("e/hull.toml", &delivered).expect("not an error, just pending");
    assert_eq!(
        step,
        PreloadStep::AwaitingIncludes(vec![
            "e/frag/a.toml".to_string(),
            "e/frag/b.toml".to_string()
        ]),
        "the host is told the CANONICAL paths to fetch, in declared order"
    );
}

#[test]
fn preload_step_walks_the_closure_one_layer_at_a_time() {
    let mut delivered = src(&[("e/hull.toml", "includes = [\"mid.toml\"]\n")]);
    let PreloadStep::AwaitingIncludes(fetch) = preload_step("e/hull.toml", &delivered).unwrap()
    else {
        panic!("expected a pending step");
    };
    assert_eq!(fetch, vec!["e/mid.toml"]);

    delivered.insert(
        "e/mid.toml".into(),
        "includes = [\"deep.toml\"]\nclass = \"mid\"\n".into(),
    );
    let PreloadStep::AwaitingIncludes(fetch) = preload_step("e/hull.toml", &delivered).unwrap()
    else {
        panic!("the transitive include must be discovered once its parent lands");
    };
    assert_eq!(fetch, vec!["e/deep.toml"]);

    delivered.insert("e/deep.toml".into(), "hull_id = \"deep\"\n".into());
    let PreloadStep::Ready(resolved) = preload_step("e/hull.toml", &delivered).unwrap() else {
        panic!("with every fragment delivered the template must resolve");
    };
    assert_eq!(
        resolved.value.get("hull_id").unwrap().as_str(),
        Some("deep")
    );
    assert_eq!(resolved.value.get("class").unwrap().as_str(), Some("mid"));
}

#[test]
fn preload_step_still_rejects_a_cycle() {
    let delivered = src(&[
        ("e/a.toml", "includes = [\"b.toml\"]\n"),
        ("e/b.toml", "includes = [\"a.toml\"]\n"),
    ]);
    let err = preload_step("e/a.toml", &delivered)
        .expect_err("absence is 'not yet'; a cycle is never fetchable");
    assert_eq!(err.category(), "include-cycle");
}

#[test]
fn preload_step_reports_a_missing_root_as_an_error_not_a_fetch() {
    let err = preload_step("e/gone.toml", &src(&[]))
        .expect_err("the root was just delivered by the host; its absence is a bug");
    assert_eq!(err.category(), "include-missing");
}

#[test]
fn preload_step_is_ready_immediately_for_an_uncomposed_template() {
    let delivered = src(&[("e/hull.toml", "class = \"solo\"\n")]);
    let PreloadStep::Ready(resolved) = preload_step("e/hull.toml", &delivered).unwrap() else {
        panic!("a template with no includes needs no extra fetches");
    };
    assert!(!resolved.is_composed());
}

// ── Filesystem adapter + the shipped fixtures ────────────────────────────

#[cfg(not(target_arch = "wasm32"))]
mod on_disk {
    use super::*;

    const COMPOSED: &str = "assets/entities/fragments/composed_escort.toml";
    const CORE: &str = "assets/entities/fragments/npc_escort_core.toml";
    const CAPTAIN: &str = "assets/entities/fragments/ai/captain_red_alert_aggressive.toml";
    /// The fourteen other ship-level AI declarations, which every AI-bearing
    /// hull has owed since #885b stage 5d made strict mode the default.
    const BASELINE: &str = "assets/entities/fragments/ai/fleet_baseline.toml";

    #[test]
    fn the_composed_fixture_hull_resolves_off_disk() {
        let resolved = resolve_from_disk(COMPOSED).expect("fixture hull must resolve");
        assert_eq!(
            resolved.provenance.sources(),
            vec![CAPTAIN, BASELINE, CORE, COMPOSED],
            "depth-first in declared order, declaring hull last"
        );
    }

    #[test]
    fn the_composed_fixture_hull_parses_as_an_entity() {
        let config = load_entity_config(COMPOSED).expect("resolved fixture must be valid");
        let ship = config
            .ship_config
            .as_ref()
            .expect("the systems fragment must supply a ship_config");
        assert!(
            ship.systems.iter().any(|s| s.kind == "helm_thrust"),
            "the shared fragment's system suite must reach the resolved hull"
        );
        assert!(
            config
                .captain_console
                .as_ref()
                .and_then(|c| c.ai.as_ref())
                .is_some(),
            "the nested AI fragment's captain policy must reach the resolved hull"
        );
    }

    #[test]
    fn the_fixture_hull_specialises_the_fragments_doctrine_by_id() {
        let config = load_entity_config(COMPOSED).expect("resolved fixture must be valid");
        let behaviour = config.behaviour.expect("fragment supplies [behaviour]");
        let doctrine = behaviour
            .doctrine
            .iter()
            .find(|d| d.id == "destroy-hostiles")
            .expect("the shared fragment's doctrine must survive");
        assert!(
            (doctrine.base_priority - 90.0).abs() < 1e-6,
            "the hull's by-id specialisation must beat the fragment's base_priority, \
                 got {}",
            doctrine.base_priority
        );
        assert_eq!(
            doctrine.directive_kind.as_deref(),
            Some("Destroy"),
            "a key the hull never mentioned comes from the fragment"
        );
    }

    #[test]
    fn provenance_attributes_the_fixture_fields_to_the_right_files() {
        let resolved = resolve_from_disk(COMPOSED).expect("fixture hull must resolve");
        let p = &resolved.provenance;
        assert_eq!(
            p.origin("behaviour.doctrine[id=destroy-hostiles].base_priority")
                .expect("doctrine priority is recorded")
                .source,
            COMPOSED
        );
        assert_eq!(
            p.origin("behaviour.doctrine[id=destroy-hostiles].directive_kind")
                .expect("directive kind is recorded")
                .source,
            CORE
        );
        assert_eq!(
            p.origin("captain_console.ai.rule")
                .expect("the captain rule list is recorded")
                .chain,
            vec![COMPOSED, CORE, CAPTAIN],
            "the chain must show the AI fragment was reached THROUGH the core fragment"
        );
    }

    /// The fragments are partial by design — none of them is a valid entity
    /// on its own, which is exactly why they live outside `assets/entities/`
    /// where every "shipped template still loads" test would try to parse
    /// them.
    #[test]
    fn the_fragments_live_outside_the_shipped_template_directory() {
        for path in [CORE, CAPTAIN] {
            assert!(
                std::path::Path::new(path).exists(),
                "{path} must exist on disk"
            );
            let dir = std::path::Path::new(path).parent().unwrap();
            assert_ne!(
                dir,
                std::path::Path::new("assets/entities"),
                "a fragment in assets/entities/ would be scanned as a shipped hull"
            );
        }
    }

    /// "Resolution must be identical on native and WASM", made checkable:
    /// the browser's incremental delivery of the SAME files must produce
    /// the same bytes as reading them straight off disk.
    #[test]
    fn the_incremental_walk_and_the_filesystem_walk_agree_byte_for_byte() {
        let native = resolve_from_disk(COMPOSED).expect("fixture resolves off disk");

        let mut delivered: HashMap<String, String> = HashMap::new();
        delivered.insert(
            COMPOSED.to_string(),
            std::fs::read_to_string(COMPOSED).unwrap(),
        );
        let mut rounds = 0;
        let browser = loop {
            rounds += 1;
            assert!(rounds < 16, "the closure walk must terminate");
            match preload_step(COMPOSED, &delivered).expect("no composition error") {
                PreloadStep::Ready(resolved) => break *resolved,
                PreloadStep::AwaitingIncludes(paths) => {
                    for path in paths {
                        let body = std::fs::read_to_string(&path)
                            .unwrap_or_else(|e| panic!("the resolver asked for {path}: {e}"));
                        delivered.insert(path, body);
                    }
                }
            }
        };
        assert_eq!(browser.toml, native.toml);
        assert_eq!(browser.provenance, native.provenance);
    }

    #[test]
    fn fs_fragment_source_misses_return_none_rather_than_panicking() {
        assert!(FsFragmentSource
            .read("assets/entities/fragments/definitely_absent.toml")
            .is_none());
    }
}

// ── The shipped tree (issue #906) ────────────────────────────────────────
//
// The byte-stability tests above prove the MECHANISM: `resolve_with` hands
// back the root text untouched when nothing was composed, so an uncomposed
// template is never round-tripped through `toml::to_string`. These prove
// the same thing over the content that actually ships, and pin the one
// condition under which the `include_str!` sites that bake hull bytes into
// the binary are allowed to stay as they are.
#[cfg(not(target_arch = "wasm32"))]
mod shipped_tree {
    use super::*;

    /// Dotted provenance prefixes a HULL must author for itself, whatever
    /// it composes. Matched on segment boundaries, so `power.capacity` does
    /// not also claim `power.ai_policy`.
    ///
    /// This is the hazard list: every authoring surface the spawner gates a
    /// real capability on, where composition bringing a parent table into
    /// existence merely by authoring a child of it would hand an includer
    /// equipment it never authored. See
    /// [`the_composed_destroyer_takes_only_ai_policy_from_its_fragments`]
    /// for what each entry does when a fragment supplies it.
    ///
    /// Module-level rather than inline in that test (issue #875 review) so
    /// the shipped-tree walk can apply it to EVERY composed hull. The
    /// destroyer's own test keeps the parts that only make sense for one
    /// hull — the reactor values, the coverage floor, player-flyability.
    const HULL_OWNED: [&str; 17] = [
        "tags",
        "faction",
        "system",
        "station",
        "shield_arc",
        "hull",
        "mesh",
        "collider",
        "comms",
        "torpedoes",
        "weapons_console.phaser_banks",
        "weapons_console.blaster_banks",
        "shields_console.base",
        "repair.repair_team_count",
        // The reactor scalars, per prefix rather than as a bare `power`:
        // `fleet_baseline.toml` really does carry `capacity = 90`,
        // `rates` and `emergency_threshold = 22` so that its
        // `[power.ai_policy]` has a table to sit in, and none of the three
        // has a parse-time default. A future hull that includes that
        // fragment and authors no `[power]` of its own would silently
        // inherit a 90-capacity reactor — the exact silent-equipment class
        // this list exists to catch. Bare `power` cannot be used:
        // `power.ai_policy` is precisely what a hull is MEANT to take from
        // the fragment library.
        "power.capacity",
        "power.rates",
        "power.emergency_threshold",
    ];

    /// Whether a provenance path falls under `prefix`, on segment
    /// boundaries: `system[id=helm-thrust].ai_only` starts with `system`
    /// and then a delimiter; `systems_foo` must not match `system`.
    fn owned(path: &str, prefix: &str) -> bool {
        path == prefix
            || path
                .strip_prefix(prefix)
                .is_some_and(|rest| rest.starts_with('.') || rest.starts_with('['))
    }

    /// Assert that no fragment authored any hazard-list surface of
    /// `resolved`, whose root template is `path`.
    ///
    /// Returns how many fields it checked, so a caller can pin that the
    /// provenance addressing has not changed shape underneath it.
    fn assert_no_fragment_supplied_equipment(path: &str, resolved: &ResolvedTemplate) -> usize {
        let mut checked = 0usize;
        for (field, origin) in resolved.provenance.fields() {
            if !HULL_OWNED.iter().any(|p| owned(field, p)) {
                continue;
            }
            checked += 1;
            assert_eq!(
                origin.source, path,
                "`{field}` in the resolved {path} was authored by {}, not by \
                     the hull. A shared AI-policy fragment must never contribute \
                     an authoring surface the spawner gates a capability on — \
                     see `the_composed_destroyer_takes_only_ai_policy_from_its_fragments` \
                     for what each one does. Include chain: {:?}",
                origin.source, origin.chain
            );
        }
        checked
    }

    /// **Issue #875 AC4, as provenance: fragments apply only to the systems
    /// a hull actually owns.**
    ///
    /// The player destroyer takes AI POLICY from the fragment library and
    /// nothing else. Every authoring surface the spawner gates a real
    /// capability on must still be authored by the hull itself.
    ///
    /// # Why provenance rather than a shape assertion
    ///
    /// The failure this guards is not "the hull came out wrong" — it is "a
    /// fragment quietly supplied something", which a shape assertion cannot
    /// tell from the hull supplying it. Composition brings a parent table
    /// into existence merely by authoring a child of it, and Rust gates on
    /// the parent, so a shared AI fragment is one careless line away from
    /// handing every hull that includes it equipment it never authored:
    ///
    /// * `tags` UNIONS at compose (unlike at the instance-override layer),
    ///   so a fragment carrying `"npc"` would flip a player hull out of
    ///   `marker_validate::is_player_flyable` — silently, since nothing
    ///   about the resolved document would look wrong;
    /// * a bare `[torpedoes.ai]` creates `[torpedoes]`, i.e. a torpedo
    ///   system with zero tubes;
    /// * `[shields_console.base]` or a `[[shield_arc]]` gives a hull
    ///   shields;
    /// * `repair_team_count` hands teams to a hull that must not have them;
    /// * `[[hull.system_hull]]` is NOT a keyed array, so a fragment touching
    ///   it REPLACES all fourteen of this hull's entries.
    ///
    /// [`HULL_OWNED`] is that hazard list, turned into an assertion. A
    /// future fragment that starts contributing one of these fails with the
    /// field named — here for this hull, and in
    /// [`every_shipped_template_resolves_to_its_own_bytes`] for every hull
    /// that composes later. What stays here is what only makes sense for
    /// one hull: the reactor VALUES, the coverage floor, player-flyability.
    #[test]
    fn the_composed_destroyer_takes_only_ai_policy_from_its_fragments() {
        const HULL: &str = "assets/entities/alliance_destroyer.toml";
        // The reactor scalars have no parse-time defaults, and
        // `fleet_baseline.toml` also carries them (at other values) so that
        // its `[power.ai_policy]` has a table to sit in. The hull's own must
        // win, or this ship silently gains the fragment's reactor.
        const HULL_REACTOR: [(&str, f64); 2] = [
            ("power.capacity", 70.0),
            ("power.emergency_threshold", 20.0),
        ];

        let resolved = resolve_from_disk(HULL).expect("the player destroyer must resolve");
        assert!(
            resolved.is_composed(),
            "this test is about what COMPOSITION contributed; if the hull \
                 stopped declaring `includes` it proves nothing"
        );

        let checked = assert_no_fragment_supplied_equipment(HULL, &resolved);
        assert!(
            checked > 100,
            "only {checked} hull-owned fields were checked, so the provenance \
                 addressing has changed shape and this guard is matching nothing"
        );

        for (path, want) in HULL_REACTOR {
            let origin = resolved
                .provenance
                .origin(path)
                .unwrap_or_else(|| panic!("`{path}` must be authored somewhere"));
            assert_eq!(
                origin.source, HULL,
                "`{path}` came from {} — the hull's own reactor must win over \
                     any a fragment carries",
                origin.source
            );
            let config = resolved.parse().expect("the resolved hull must parse");
            let power = config.power.as_ref().expect("the hull authors [power]");
            let got = match path {
                "power.capacity" => power.capacity as f64,
                _ => power.emergency_threshold as f64,
            };
            assert_eq!(got, want, "`{path}` resolved to the wrong value");
        }

        // The shape the provenance check exists to protect, stated once so a
        // reader can see what "the systems a hull actually owns" means here.
        let config = resolved.parse().expect("the resolved hull must parse");
        assert!(
            crate::entities::marker_validate::is_player_flyable(&config),
            "the player destroyer must still be player-flyable — this is what \
                 a fragment carrying `tags = [\"npc\"]` would silently break"
        );
        assert_eq!(config.tags, vec!["ship".to_string()]);
        let ship = config
            .ship_config
            .as_ref()
            .expect("the hull declares [[system]] blocks");
        let stations: Vec<&str> = ship.stations.iter().map(|s| s.id.0.as_str()).collect();
        assert_eq!(
            stations,
            vec![
                "captain",
                "helm",
                "tactical",
                "navigation",
                "comms",
                "engineering",
                "command"
            ],
            "seven Stations, exactly as authored — no fragment adds or removes one"
        );
        let arcs: Vec<&str> = config
            .shield_arcs
            .iter()
            .map(|a| a.id.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            arcs,
            vec!["fore", "aft"],
            "TWO arcs, not four. A shared shields fragment that assumed the \
                 four-arc layout would add two this hull never authored."
        );
    }

    /// Where a baked asset path sits in the tree.
    ///
    /// `assets/entities/*.toml` is a spawnable HULL; anything under
    /// `assets/entities/fragments/` is a FRAGMENT, which is not spawnable
    /// and is the thing hulls compose FROM. The scan below reports the two
    /// separately, because "this file bakes a composed hull" and "this file
    /// bakes a fragment that has itself grown includes" send an author to
    /// completely different fixes.
    fn is_fragment(asset_path: &str) -> bool {
        asset_path.starts_with("assets/entities/fragments/")
    }

    /// The literal inside `include_str!( … "…" )`, and how far past it to
    /// resume scanning, given the text immediately after `include_str!(`.
    ///
    /// `None` when the macro's argument is not a plain string literal — a
    /// `concat!`, a `const`, a nested macro — none of which bake a path
    /// this scan can name.
    fn baked_literal(after_open: &str) -> Option<(&str, usize)> {
        let quote = after_open.find(|c: char| !c.is_whitespace())?;
        if after_open.as_bytes()[quote] != b'"' {
            return None;
        }
        let body = &after_open[quote + 1..];
        let end = body.find('"')?;
        Some((&body[..end], quote + 1 + end))
    }

    /// Shipped entity-asset paths baked into the binary by `include_str!`,
    /// paired with the source file that bakes them.
    ///
    /// Tolerates rustfmt's wrapping: a long path is routinely pushed onto
    /// the line after `include_str!(`, so the scan skips whitespace before
    /// expecting the opening quote. Searching for the contiguous bytes
    /// `include_str!("` instead would silently miss every wrapped site —
    /// and it is the wrapped ones, being the long paths, that are most
    /// likely to be assets.
    ///
    /// Walks BOTH crate source roots. `tests/` is not decoration: the
    /// headless runner's integration tests bake a hull with
    /// `include_str!("../assets/entities/…")` exactly as `src/` does, and a
    /// scan that only saw `src/` would leave those sites unenumerated — the
    /// AC asks for every site to be named or excused, and a site the scan
    /// cannot see is neither. Any future source root (`benches/`,
    /// `examples/`) belongs in this list for the same reason.
    const SOURCE_ROOTS: [&str; 2] = ["src", "tests"];

    /// Alongside the sites, returns how many `.rs` files the WALK ITSELF
    /// read under each of `SOURCE_ROOTS` — the "did the scan reach this
    /// root at all" reading, which has to come from this walk rather than
    /// a second, separately-written one: a standalone directory walk would
    /// prove only that the root exists and holds `.rs` files, not that the
    /// enumeration above ever looked at them. It would pass unchanged if
    /// `SOURCE_ROOTS` were trimmed to `["src"]`, or if this walk grew a bug
    /// that returned early. Threading the count through the same recursion
    /// the sites come from ties the "reached" evidence to the thing it is
    /// evidence for.
    fn include_str_baked_hulls() -> (Vec<(String, String)>, HashMap<&'static str, usize>) {
        fn walk(dir: &std::path::Path, out: &mut Vec<(String, String)>, read: &mut usize) {
            let entries = std::fs::read_dir(dir)
                .unwrap_or_else(|e| panic!("{} must be readable: {e}", dir.display()));
            for entry in entries {
                let path = entry.expect("readable dir entry").path();
                if path.is_dir() {
                    walk(&path, out, read);
                    continue;
                }
                if path.extension().is_none_or(|e| e != "rs") {
                    continue;
                }
                *read += 1;
                let file = path.to_string_lossy().replace('\\', "/");
                let src = std::fs::read_to_string(&path)
                    .unwrap_or_else(|e| panic!("{file} must be readable: {e}"));
                let mut rest = src.as_str();
                while let Some(i) = rest.find("include_str!(") {
                    rest = &rest[i + "include_str!(".len()..];
                    // Not a plain string literal (a `concat!`, a macro
                    // argument): nothing to bake, and `rest` has already
                    // advanced, so the walk cannot stall here.
                    let Some((literal, consumed)) = baked_literal(rest) else {
                        continue;
                    };
                    // The literal is relative to the declaring FILE; the
                    // repo-relative form is whatever follows the assets
                    // prefix, which is unambiguous.
                    if let Some(j) = literal.find("assets/entities/") {
                        out.push((file.clone(), literal[j..].to_string()));
                    }
                    rest = &rest[consumed..];
                }
            }
        }
        let mut out = Vec::new();
        let mut read_per_root: HashMap<&'static str, usize> = HashMap::new();
        for root in SOURCE_ROOTS {
            let mut read = 0usize;
            walk(std::path::Path::new(root), &mut out, &mut read);
            read_per_root.insert(root, read);
        }
        out.sort();
        out.dedup();
        (out, read_per_root)
    }

    /// THE EXCUSE for the `include_str!` sites, recorded where a future
    /// author trips over it (issue #906).
    ///
    /// `include_str!` bakes a hull's bytes into the binary at COMPILE time.
    /// There is no seam at which resolution could run: the resolver needs a
    /// fragment source at runtime, so a baked template can never see a
    /// resolved document. Migrating every site would mean turning each into
    /// a disk load inside the test body — a large mechanical change to
    /// tests that mostly assert on ONE authored field of ONE hull and are
    /// all correct today.
    ///
    /// So they are excused, on a condition this test enforces: **every hull
    /// reached by an `include_str!` must be uncomposed.** While that holds,
    /// the baked bytes and the resolved document are the same text (proved
    /// by `every_shipped_template_resolves_to_its_own_bytes` above) and the
    /// excuse costs nothing. The moment #875/#878 compose one of these
    /// hulls, this test names the exact sites that must move — strictly
    /// better than a frozen list, because it covers `include_str!` sites
    /// added after this was written too.
    #[test]
    fn include_str_baked_hulls_are_all_uncomposed() {
        let (baked, read_per_root) = include_str_baked_hulls();
        // The floor is a "did the scan actually run" check, not a budget. It
        // stood at 20 until issue #878 composed the five Harrow hulls and
        // moved every site that baked one onto the resolving load path — a
        // little over half the sites in the tree, and exactly the migration
        // this test's own doc comment predicted. Lower it again only
        // alongside another such migration, never to make a red run green.
        assert!(
            baked.len() >= 8,
            "the source scan found only {} baked hull sites — it has stopped \
                 finding them, so it is guarding nothing",
            baked.len()
        );
        // Every source root must actually be REACHED by the SCAN ITSELF, or
        // the enumeration this AC rests on is silently partial. `tests/` is
        // the one that was missed first time round: `tests/headless_runner.rs`
        // baked a hull through `../assets/entities/…`, and a src-only scan
        // would have excused a site it had never looked at.
        //
        // Asserted on the walk's own per-root read count (from
        // `include_str_baked_hulls`) rather than on a baked site being found
        // there, because issue #878 composed the five Harrow hulls and
        // `tests/headless_runner.rs`'s two sites — both Harrow — moved onto
        // the resolving load path. A root with no baked site left is not a
        // root the scan cannot see, and conflating the two would have this
        // guard fail for the very migration it exists to demand. It is also
        // asserted on the SAME walk rather than a second, independently
        // written directory count: a re-implemented walk would prove the
        // root has `.rs` files, not that this scan reaches them — trimming
        // `SOURCE_ROOTS` to `["src"]` would still pass that.
        //
        // Spelled out as literals rather than read from `SOURCE_ROOTS`:
        // deriving them would let the guard shrink in step with the thing it
        // is guarding, which is exactly the regression to catch.
        for root in ["src", "tests"] {
            assert!(
                read_per_root.get(root).is_some_and(|&n| n > 0),
                "the scan itself read zero .rs files under {root}/ — either the \
                     root has been renamed and SOURCE_ROOTS is stale, or the walk \
                     never reached it, so the scan is looking at nothing there"
            );
        }
        assert!(
            baked.iter().any(|(site, _)| site.starts_with("src/")),
            "no baked `include_str!` site was found under src/ — the scan has \
                 stopped parsing. Sites found: {:?}",
            baked.iter().map(|(s, _)| s).collect::<Vec<_>>()
        );
        let mut composed: Vec<String> = Vec::new();
        for (site, asset) in &baked {
            let resolved = resolve_from_disk(asset)
                .unwrap_or_else(|e| panic!("{site} bakes {asset}, which must resolve: {e}"));
            if resolved.is_composed() {
                // Naming the KIND matters: a composed hull sends the author
                // to the resolved document, a composed fragment sends them
                // to the fragment tree. Reporting a fragment as a hull is
                // the wrong diagnosis.
                let kind = if is_fragment(asset) {
                    "composed fragment"
                } else {
                    "composed hull"
                };
                composed.push(format!("{site} bakes {kind} {asset}"));
            }
        }
        assert!(
            composed.is_empty(),
            "these `include_str!` sites bake an entity asset that is now COMPOSED, \
                 so they assert on unresolved text. Replace each with a disk load \
                 through `entity_includes::load_entity_config` (or `resolve_from_disk` \
                 where the raw text is needed for line lookups). A `composed fragment` \
                 is a different diagnosis from a `composed hull`: the fragment tree \
                 has grown a level, so check what ELSE includes it before changing \
                 the site:\n{}",
            composed.join("\n")
        );
    }

    /// A fragment is not a hull, and the scan must not call one the other.
    ///
    /// `src/world/validate.rs` bakes `fragments/ai/fleet_baseline.toml`,
    /// which is a FRAGMENT. If a fragment ever grows its own `includes`,
    /// reporting it as a composed *hull* would send the author looking for
    /// a spawnable template that does not exist — the wrong diagnosis, and
    /// the wrong fix.
    #[test]
    fn the_scan_tells_a_fragment_apart_from_a_hull() {
        assert!(is_fragment(
            "assets/entities/fragments/ai/fleet_baseline.toml"
        ));
        assert!(!is_fragment("assets/entities/alliance_cruiser.toml"));
        let (baked, _read_per_root) = include_str_baked_hulls();
        let fragments: Vec<&(String, String)> =
            baked.iter().filter(|(_, a)| is_fragment(a)).collect();
        assert!(
            !fragments.is_empty(),
            "no baked site reaches the fragment tree any more, so the \
                 hull/fragment distinction in the failure message is guarding \
                 nothing — check the scan is still parsing before removing it"
        );
        assert!(
            fragments.len() < baked.len(),
            "the scan must still reach hulls too, or `is_fragment` has \
                 stopped discriminating"
        );
    }

    /// rustfmt wraps a long `include_str!` path onto the next line, and the
    /// scan above must still see it.
    ///
    /// This is the mechanism in isolation, pinned synthetically rather than
    /// against the real tree. The wrapped form is not hypothetical — issue
    /// #878 composed the five Harrow hulls and moved every site that baked
    /// one onto the resolving load path, and those long
    /// `"../../assets/entities/ship_harrow_*.toml"` literals were exactly the
    /// ones rustfmt had wrapped — but a synthetic fixture needs no wrapped
    /// site to exist in the tree at all, so this stays load-bearing even if
    /// the real tree later converges back to all-contiguous sites.
    #[test]
    fn the_scan_reads_a_wrapped_include_str_literal() {
        let one_line = "include_str!(\"../../assets/entities/x.toml\")";
        let wrapped = "include_str!(\n            \"../../assets/entities/x.toml\"\n        )";
        for src in [one_line, wrapped] {
            let after = &src[src.find("include_str!(").expect("the macro") + 13..];
            let (literal, _) = baked_literal(after)
                .unwrap_or_else(|| panic!("the scan must read the literal out of: {src:?}"));
            assert_eq!(literal, "../../assets/entities/x.toml");
        }
        assert!(
            baked_literal("concat!(\"a\", \"b\"))").is_none(),
            "a non-literal argument bakes no path this scan can name"
        );
    }
}

// ── Composition as a world finding (issue #906) ──────────────────────────

mod composition_findings {
    use super::*;
    use crate::world::validate::{has_error, Severity};

    fn source(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(p, t)| (p.to_string(), t.to_string()))
            .collect()
    }

    #[test]
    fn a_missing_fragment_becomes_an_error_finding_naming_the_declaring_file() {
        let src = source(&[(
            "e/hull.toml",
            "includes = [\"absent.toml\"]\nname = \"H\"\n",
        )]);
        let f = composition_finding("e/hull.toml", &src).expect("a finding");
        assert_eq!(f.severity, Severity::Error);
        assert_eq!(f.category, "include-missing");
        assert_eq!(
            f.source.file, "e/hull.toml",
            "the finding names the file that DECLARED the bad include"
        );
        assert_eq!(f.source.line, Some(1));
        assert!(f.message.contains("include chain"), "{}", f.message);
        assert!(has_error(&[f]), "an error finding must gate activation");
    }

    #[test]
    fn a_cycle_becomes_an_error_finding() {
        let src = source(&[
            ("e/hull.toml", "includes = [\"frag.toml\"]\n"),
            ("e/frag.toml", "includes = [\"hull.toml\"]\n"),
        ]);
        let f = composition_finding("e/hull.toml", &src).expect("a finding");
        assert_eq!(f.category, "include-cycle");
    }

    #[test]
    fn a_malformed_includes_declaration_becomes_an_error_finding() {
        let src = source(&[("e/hull.toml", "includes = 7\n")]);
        let f = composition_finding("e/hull.toml", &src).expect("a finding");
        assert_eq!(f.category, "include-malformed");
        assert_eq!(f.source.file, "e/hull.toml");
    }

    #[test]
    fn a_composed_document_that_is_not_a_valid_entity_becomes_an_error_finding() {
        let src = source(&[
            ("e/frag.toml", "not_a_real_key = 1\n"),
            ("e/hull.toml", "includes = [\"frag.toml\"]\n"),
        ]);
        let f = composition_finding("e/hull.toml", &src).expect("a finding");
        assert_eq!(
            f.category, "include-invalid-template",
            "the offending combination exists in no single authored file"
        );
    }

    #[test]
    fn a_template_the_source_cannot_serve_is_not_a_composition_finding() {
        let src = source(&[]);
        assert!(
            composition_finding("e/hull.toml", &src).is_none(),
            "a validator must not manufacture an error out of its own blindness — \
                 a missing template has its own diagnostics"
        );
    }

    #[test]
    fn an_uncomposed_template_that_is_not_valid_toml_is_not_a_composition_finding() {
        let src = source(&[("e/hull.toml", "this is not toml\n")]);
        assert!(
            composition_finding("e/hull.toml", &src).is_none(),
            "a plain parse error keeps its historical skip-with-warning; it is not \
                 a composition failure"
        );
    }

    #[test]
    fn an_uncomposed_template_that_is_not_a_valid_entity_is_not_a_composition_finding() {
        let src = source(&[("e/hull.toml", "not_a_real_key = 1\n")]);
        assert!(composition_finding("e/hull.toml", &src).is_none());
    }

    #[test]
    fn an_unparseable_fragment_is_a_composition_finding() {
        let src = source(&[
            ("e/frag.toml", "this is not toml\n"),
            ("e/hull.toml", "includes = [\"frag.toml\"]\n"),
        ]);
        let f = composition_finding("e/hull.toml", &src).expect("a finding");
        assert_eq!(f.category, "include-parse");
    }

    /// The host source's answer to `absence_is_final` is target-dependent,
    /// and the browser answer is the one that matters — so it is also the
    /// one `cargo test` can never observe.
    ///
    /// The structural guard against losing that answer is that
    /// [`FragmentSource::absence_is_final`] has no default: delete
    /// `HostFragmentSource`'s override and the crate stops compiling, on
    /// both targets. This test pins the *values* on top of that. The native
    /// arm runs in CI; the wasm arm is compiled only under `wasm32` and so
    /// runs only under a wasm test runner — it is written as an assertion
    /// rather than a comment so that if this crate ever grows one, the
    /// claim is already being checked rather than merely described.
    #[test]
    fn the_host_source_answers_absence_by_target() {
        #[cfg(not(target_arch = "wasm32"))]
        assert!(
            HostFragmentSource.absence_is_final(),
            "on native the filesystem is authoritative, so a fragment that \
                 cannot be read genuinely does not exist and validation may say so"
        );
        #[cfg(target_arch = "wasm32")]
        assert!(
            !HostFragmentSource.absence_is_final(),
            "in the browser the raw-template channel fills one delivery at a \
                 time, so an unread fragment may still be in flight; calling that \
                 final blanks the world permanently"
        );
    }

    /// A source that fills INCREMENTALLY — the browser's raw-template
    /// channel, where a root can be in hand a whole layer-load before the
    /// fragment it includes.
    struct StillFilling(HashMap<String, String>);

    impl FragmentSource for StillFilling {
        fn read(&self, path: &str) -> Option<String> {
            self.0.get(path).cloned()
        }
        fn absence_is_final(&self) -> bool {
            false
        }
    }

    fn still_filling(pairs: &[(&str, &str)]) -> StillFilling {
        StillFilling(source(pairs))
    }

    /// The wasm hazard, stated directly: a fragment that has not been
    /// delivered YET is not a fault.
    ///
    /// If this reported, `has_error` would gate the world, and
    /// `spawn_immediate_entities_internal` would return zero entities — for
    /// a world whose only sin is that its fragments are still arriving. The
    /// runtime layer load never retries, so the loss would be permanent
    /// rather than a frame of lag.
    #[test]
    fn a_fragment_that_has_not_arrived_yet_is_not_a_composition_finding() {
        let src = still_filling(&[("e/hull.toml", "includes = [\"frag.toml\"]\nname = \"H\"\n")]);
        assert!(
            composition_finding("e/hull.toml", &src).is_none(),
            "a source that is still filling must not have its own race read \
                 back to it as a broken include"
        );
    }

    /// …and the blindness is not permanent: once the fragment lands, the
    /// same pair is composed and validated like any other.
    #[test]
    fn the_same_pair_validates_once_the_fragment_arrives() {
        let good = still_filling(&[
            ("e/frag.toml", "class = \"cruiser\"\n"),
            ("e/hull.toml", "includes = [\"frag.toml\"]\nname = \"H\"\n"),
        ]);
        assert!(composition_finding("e/hull.toml", &good).is_none());

        let bad = still_filling(&[
            ("e/frag.toml", "not_a_real_key = 1\n"),
            ("e/hull.toml", "includes = [\"frag.toml\"]\nname = \"H\"\n"),
        ]);
        let f = composition_finding("e/hull.toml", &bad)
            .expect("a delivered fragment is judged, not excused");
        assert_eq!(f.category, "include-invalid-template");
    }

    /// Deferring on ABSENCE must not defer on the faults. A cycle is a
    /// fault no delivery can fix, and it is still reported from a source
    /// that is still filling.
    #[test]
    fn the_real_faults_are_still_reported_while_a_source_is_still_filling() {
        let cyclic = still_filling(&[
            ("e/hull.toml", "includes = [\"frag.toml\"]\n"),
            ("e/frag.toml", "includes = [\"hull.toml\"]\n"),
        ]);
        assert_eq!(
            composition_finding("e/hull.toml", &cyclic)
                .expect("a cycle is not a delivery race")
                .category,
            "include-cycle"
        );

        let malformed = still_filling(&[("e/hull.toml", "includes = 7\n")]);
        assert_eq!(
            composition_finding("e/hull.toml", &malformed)
                .expect("a malformed declaration is not a delivery race")
                .category,
            "include-malformed"
        );

        let unparseable = still_filling(&[
            ("e/frag.toml", "this is not toml\n"),
            ("e/hull.toml", "includes = [\"frag.toml\"]\n"),
        ]);
        assert_eq!(
            composition_finding("e/hull.toml", &unparseable)
                .expect("a delivered but unparseable fragment is not a delivery race")
                .category,
            "include-parse"
        );
    }

    /// The default the other way round: a source that already holds
    /// everything it will ever hold — the filesystem, every fixture map —
    /// still reports a genuinely missing fragment, with the declaring file
    /// and line intact.
    #[test]
    fn a_source_whose_absence_is_final_still_reports_the_missing_fragment() {
        let src = source(&[(
            "e/hull.toml",
            "includes = [\"absent.toml\"]\nname = \"H\"\n",
        )]);
        assert!(
            src.absence_is_final(),
            "a fixture map holds everything it will ever hold"
        );
        let f = composition_finding("e/hull.toml", &src).expect("a finding");
        assert_eq!(f.category, "include-missing");
        assert_eq!(f.source.file, "e/hull.toml");
        assert_eq!(f.source.line, Some(1));
    }

    #[test]
    fn a_template_that_composes_cleanly_produces_no_finding() {
        let src = source(&[
            ("e/frag.toml", "class = \"cruiser\"\n"),
            ("e/hull.toml", "includes = [\"frag.toml\"]\nname = \"H\"\n"),
        ]);
        assert!(composition_finding("e/hull.toml", &src).is_none());
    }
}
