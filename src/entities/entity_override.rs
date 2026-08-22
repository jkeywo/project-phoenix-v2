/// The plain structural deep-merge: tables recurse, **everything else — arrays
/// included — is replaced by the override**.
///
/// # This is NOT the entity merge — do not reach for it
///
/// No entity path calls this any more. Since issue #911 both layers go through
/// [`merge_entity_config_toml_with`], which knows that `behaviour.doctrine`
/// reconciles by `id`, that a fragment can extend `[[system]]`, and that `tags`
/// unions at one layer and replaces at the other. Merging an entity document
/// with this function instead would silently discard a template's whole system
/// suite, its doctrine, and its shield arcs.
///
/// It remains public as the primitive the entity merge is defined in terms of,
/// and as the reference point the pre-#911 differential test in
/// `entity_loader` reconstructs the old algorithm from.
pub fn merge_toml(template: &toml::Value, override_: &toml::Value) -> toml::Value {
    match (template, override_) {
        (toml::Value::Table(t_table), toml::Value::Table(o_table)) => {
            let mut result = t_table.clone();
            for (key, o_val) in o_table {
                match result.get(key) {
                    Some(t_val) => {
                        result.insert(key.clone(), merge_toml(t_val, o_val));
                    }
                    None => {
                        result.insert(key.clone(), o_val.clone());
                    }
                }
            }
            toml::Value::Table(result)
        }
        _ => override_.clone(),
    }
}

// ── Which layer is merging (issue #911) ──────────────────────────────────────

/// The authored per-entry tombstone: `{ id = "x", _remove = true }`.
///
/// One marker, one meaning, one strip site. See [`MergePolicy`] for which
/// layers accept it.
pub const REMOVE_KEY: &str = "_remove";

/// What the merge does with an array at a given dotted path.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArrayRule {
    /// Reconcile element-by-element against this identity key: same key ⇒ deep
    /// merge in place, new key ⇒ append, tombstone ⇒ remove.
    Keyed(&'static str),
    /// Set-union of bare values, template order first. Only `tags`, and only
    /// under [`MergePolicy::ComposeFragments`].
    Union,
    /// The override's array wins whole.
    Replace,
}

/// Which layer is merging, and therefore which arrays reconcile.
///
/// # Why this seam exists at all
///
/// Before issue #911 there was exactly one merge, shared by include resolution
/// and per-instance `[[entity]]` overrides. That sharing is what made #869 put
/// array extension out of scope: widening the rule so a hull could extend a
/// fragment's `[[system]]` suite would have silently widened it for every world
/// override too.
///
/// The two layers genuinely want different answers, and `tags` is the proof.
/// A fragment library wants `tags` to UNION — the library's tags plus mine. A
/// world override needs it to REPLACE, because replacing is the only way to
/// take a tag away, and three shipped worlds depend on doing exactly that
/// (`assets/worlds/default.toml:148`, `patrol.toml:65`,
/// `reinforcements.toml:56` all drop `ship_harrow_patrol`'s `comms_contact`).
/// One rule cannot serve both. So the rule became a parameter.
///
/// [`merge_entity_config_toml`] keeps the two-argument shape and the
/// instance-override policy, so every caller that was right before stays right
/// and unedited; only [`crate::entities::include_resolve`] opts into the other policy.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MergePolicy {
    /// A world's per-instance `[[entity]].overrides` merging onto a resolved
    /// template — **exactly the pre-#911 behaviour**.
    ///
    /// Only `behaviour.doctrine` reconciles (by `id`). Every other array,
    /// `tags` included, replaces wholesale. Tombstones are NOT accepted: the
    /// merge itself **rejects** an override carrying `_remove` anywhere
    /// ([`reject_unhonoured_removals`]), so a world override that writes one
    /// fails loudly instead of quietly doing nothing. A world's subtractive
    /// levers stay what they have always been — the authored empty array, and
    /// restating the array.
    ///
    /// # Why the merge rejects rather than leaving it to the parser
    ///
    /// It used to be assumed that a surviving `_remove` key would reach
    /// `EntityConfig` and be caught by `deny_unknown_fields`. It would not.
    /// `behaviour.doctrine` is the one array that reconciles at THIS layer, so
    /// a tombstone written there deep-merges into the matching template entry —
    /// and `DoctrineObjective` is not `deny_unknown_fields`, so serde ignores
    /// the key, `apply_overrides` returns `Ok`, and the doctrine is unchanged.
    /// Measured against the real `ship_harrow_patrol` hull that was
    /// `ACCEPTED SILENTLY`. Nothing in `ship::config` is
    /// `deny_unknown_fields` either. That is precisely the silent-no-op failure
    /// mode issue #838 existed to end, so the guarantee is now enforced where
    /// it is stated rather than delegated to a parser that does not make it.
    #[default]
    InstanceOverride,
    /// One entity template merging onto its include closure.
    ///
    /// Every array in the identity table reconciles by key, `tags` unions, and
    /// a `{ id = "…", _remove = true }` entry removes an inherited one.
    ComposeFragments,
}

/// Arrays that reconcile by an identity key when composing fragments.
///
/// Paths are dotted and **index-free**: an element of `[[station]]` is reached
/// at path `station`, so its own `[[station.rating]]` array is reached at
/// `station.rating`.
///
/// # Why these, and why by these keys
///
/// Every key here is already an identity the loader enforces as unique **within
/// its parent entry** — which is all the merge needs, because a path is only
/// reachable inside a matched parent. `system.id` is unique document-wide
/// (`DuplicateSystemId`); `station.rating.name` is unique only *within its
/// station* — `"Std"` and `"Simplified"` repeat in every station of
/// `alliance_cruiser.toml` — and that is enough, because `station.rating` is
/// only ever reached inside a `[[station]]` already matched by `id`. Either way
/// reconciling by the key cannot merge two things an author meant to keep
/// apart. This is not a new idea: nine shipped
/// worlds have relied on `behaviour.doctrine` merging by `id` daily since
/// `68bda1be`. #911 applies it consistently instead of inventing a second
/// mechanism.
///
/// **`kind` is deliberately NOT an identity.** It repeats in 8 of the 11 files
/// that declare systems — a hull has many `phaser_bank` systems — so keying on
/// it would collapse a weapons suite into one entry.
///
/// # Arrays deliberately left replacing
///
/// * `*.ai.rule` and `*_ai.state[].transition` — their only candidate key is
///   the composite `(channel, priority)`, so an author bumping a priority would
///   silently "rename" the entry and get an append instead of an edit. Equal
///   priorities are already rejected at load, so there is no stable key to be
///   had. **A fragment contributing an AI policy contributes it WHOLE**; that
///   is the intended granularity, not a gap.
/// * `*.selector.score` — the entries carry no identity at all.
/// * `hull.system_hull` — a positional/derived list with no key.
///
/// # Nested arrays inside a reconciled entry
///
/// A matched entry deep-merges through the same path-aware walk, so an array
/// nested inside it is judged by ITS path. `station.rating` therefore
/// reconciles by `name`, while `behaviour.doctrine.directive_anchors` — the
/// `directive_anchors = []` idiom `world/dispatch.rs` documents — is absent
/// from this table and keeps replacing wholesale, at both layers.
const COMPOSE_KEYED_ARRAYS: &[(&str, &str)] = &[
    ("behaviour.doctrine", "id"),
    ("shield_arc", "id"),
    ("station", "id"),
    ("station.rating", "name"),
    ("system", "id"),
    ("torpedoes.tubes", "id"),
    ("weapons_console.blaster_banks", "id"),
    ("weapons_console.phaser_banks", "id"),
];

/// The pre-#911 table, unchanged: instance overrides reconcile doctrine only.
///
/// `behaviour.state` is NOT here and is not in the compose table either — see
/// [`merge_keyed_array`]'s note on the retired FSM.
const INSTANCE_KEYED_ARRAYS: &[(&str, &str)] = &[("behaviour.doctrine", "id")];

impl MergePolicy {
    /// The identity table this layer merges by. **Provenance reads the same
    /// table** (`include_resolve::record_leaves`) — if the two ever disagree, a
    /// merged-in `[[system]]` is recorded as a wholesale leaf and every field
    /// an earlier fragment contributed to it is pruned from the record.
    pub fn keyed_arrays(self) -> &'static [(&'static str, &'static str)] {
        match self {
            MergePolicy::InstanceOverride => INSTANCE_KEYED_ARRAYS,
            MergePolicy::ComposeFragments => COMPOSE_KEYED_ARRAYS,
        }
    }

    /// What to do with the array at `path`, which is dotted and index-free.
    pub fn array_rule(self, path: &str) -> ArrayRule {
        if let Some((_, key)) = self.keyed_arrays().iter().find(|(p, _)| *p == path) {
            return ArrayRule::Keyed(key);
        }
        if path == "tags" && self == MergePolicy::ComposeFragments {
            return ArrayRule::Union;
        }
        ArrayRule::Replace
    }

    /// Whether this layer honours the `_remove` tombstone.
    pub fn accepts_removals(self) -> bool {
        self == MergePolicy::ComposeFragments
    }
}

/// True for `{ … , _remove = true }` — the per-entry tombstone.
pub fn is_removal(entry: &toml::Value) -> bool {
    entry
        .get(REMOVE_KEY)
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
}

fn join_path(prefix: &str, key: &str) -> String {
    if prefix.is_empty() {
        key.to_string()
    } else {
        format!("{prefix}.{key}")
    }
}

/// Merge two TOML values with entity-config–aware special-casing.
///
/// * All table keys deep-merge as in `merge_toml`.
/// * An array whose dotted path is in the layer's identity table
///   ([`MergePolicy::keyed_arrays`]) reconciles **element-wise**: an override
///   entry whose key matches a template entry deep-merges into it *in place*,
///   an entry with a new key is **appended**, and a
///   `{ id = "…", _remove = true }` entry **removes** the match.
/// * `tags` unions when composing fragments, and replaces at the instance
///   layer.
/// * Every other array is full-replacement when the override supplies one.
///
/// # An authored empty array clears the list
///
/// The reconciling rules above only apply when the override actually supplies
/// entries. An **explicitly authored empty array** (`doctrine = []`, `tags =
/// []`) means "clear this list", not "merge nothing in": it is the only way a
/// scenario can take a behaviour *away* from a template, and it is what every
/// replacing array in an override already does. Before this,
/// `assets/worlds/probe_aggressor.toml`'s `behaviour = { doctrine = [] }` was a
/// silent no-op: the "passive" hostile it describes kept the template's
/// `destroy-hostiles` Destroy doctrine and opened fire first.
///
/// Omitting the key entirely is still the way to say "leave the template's list
/// alone" — an absent key never reaches the merge.
///
/// # A tombstone here is an ERROR
///
/// `_remove` is a fragment-composition marker. Written in a world override it
/// returns `Err` — see [`reject_unhonoured_removals`] and [`MergePolicy`].
///
/// Call this instead of `merge_toml` when resolving `WorldEntity` overrides.
pub fn merge_entity_config_toml(
    template: &toml::Value,
    override_: &toml::Value,
) -> Result<toml::Value, String> {
    merge_entity_config_toml_with(template, override_, MergePolicy::InstanceOverride)
}

/// [`merge_entity_config_toml`] with the layer stated explicitly (issue #911).
///
/// Fallible so that the tombstone rule is enforced by the merge rather than by
/// each caller remembering to check. Under [`MergePolicy::ComposeFragments`] —
/// the only policy that honours `_remove` — this never returns `Err`.
pub fn merge_entity_config_toml_with(
    template: &toml::Value,
    override_: &toml::Value,
    policy: MergePolicy,
) -> Result<toml::Value, String> {
    reject_unhonoured_removals(override_, policy)?;
    let merged = merge_at("", template, override_, policy);
    Ok(if policy.accepts_removals() {
        strip_removals(&merged)
    } else {
        merged
    })
}

/// The dotted, index-free path of the first `_remove` key anywhere in `value`.
///
/// A tombstone in the third `[[system]]` entry reports `system._remove`, the
/// same index-free shape [`MergePolicy::array_rule`] speaks.
pub fn find_removal_marker(value: &toml::Value) -> Option<String> {
    fn walk(path: &str, value: &toml::Value) -> Option<String> {
        match value {
            toml::Value::Table(table) => {
                if table.contains_key(REMOVE_KEY) {
                    return Some(join_path(path, REMOVE_KEY));
                }
                table.iter().find_map(|(k, v)| walk(&join_path(path, k), v))
            }
            toml::Value::Array(items) => items.iter().find_map(|v| walk(path, v)),
            _ => None,
        }
    }
    walk("", value)
}

/// Reject a `_remove` tombstone written at a layer that does not honour it.
///
/// # Why this is a hard error and not a warning
///
/// `_remove` is subtractive: the author is asking for something to be GONE.
/// Every other outcome — ignoring it, ignoring it with a log line — leaves a
/// document that looks like it did what was asked and did not. A tombstone that
/// reaches [`MergePolicy::InstanceOverride`] does not merely fail to remove: on
/// `behaviour.doctrine`, the one array that reconciles at that layer, it
/// deep-merges into the matching template entry and disappears into serde's
/// unknown-field ignore, because `DoctrineObjective` is not
/// `deny_unknown_fields`. So the load SUCCEEDS with the doctrine intact.
///
/// The key's mere presence is the mistake, so `_remove = false` is rejected
/// too: at this layer there is no reading of the key that does anything.
pub fn reject_unhonoured_removals(
    override_: &toml::Value,
    policy: MergePolicy,
) -> Result<(), String> {
    if policy.accepts_removals() {
        return Ok(());
    }
    match find_removal_marker(override_) {
        None => Ok(()),
        Some(path) => Err(format!(
            "`{REMOVE_KEY}` at `{path}` is a fragment-composition marker and is not \
             honoured by a per-instance override ({policy:?}). To take an entry away \
             here, restate the array without it, or clear the whole array with `[]`."
        )),
    }
}

fn merge_at(
    path: &str,
    template: &toml::Value,
    override_: &toml::Value,
    policy: MergePolicy,
) -> toml::Value {
    match (template, override_) {
        (toml::Value::Table(t_table), toml::Value::Table(o_table)) => {
            let mut result = t_table.clone();
            for (key, o_val) in o_table {
                let child = join_path(path, key);
                match result.get(key) {
                    Some(t_val) => {
                        result.insert(key.clone(), merge_at(&child, t_val, o_val, policy));
                    }
                    None => {
                        result.insert(key.clone(), o_val.clone());
                    }
                }
            }
            toml::Value::Table(result)
        }
        // An EMPTY override array never reconciles — it clears. See the doc
        // above; this is a scenario's and a fragment's only subtractive lever
        // for a whole list.
        (toml::Value::Array(t_items), toml::Value::Array(o_items)) if !o_items.is_empty() => {
            match policy.array_rule(path) {
                ArrayRule::Keyed(key) => {
                    toml::Value::Array(merge_keyed_array_at(path, t_items, o_items, key, policy))
                }
                ArrayRule::Union => toml::Value::Array(union_array(t_items, o_items)),
                ArrayRule::Replace => override_.clone(),
            }
        }
        _ => override_.clone(),
    }
}

/// Set-union preserving template order, appending only what is new.
///
/// Used for `tags` alone: an array of bare strings has no key to reconcile by,
/// so union and replace are the only two options there are.
fn union_array(template: &[toml::Value], overrides: &[toml::Value]) -> Vec<toml::Value> {
    let mut result = template.to_vec();
    for entry in overrides {
        if !result.contains(entry) {
            result.push(entry.clone());
        }
    }
    result
}

/// Merge two arrays whose elements are TOML tables carrying `key`.
///
/// * An override entry whose `key` matches a template entry **deep-merges into
///   it at the template entry's original position**.
/// * An override entry with an unmatched (or missing) `key` is **appended**.
/// * An override entry with `_remove = true` **removes** the matching template
///   entry rather than merging into it, and is never itself appended.
///
/// # Position is a guarantee, not an accident
///
/// `[[shield_arc]]` order is load-bearing: `ShieldSystem::from_arcs` maps arcs
/// positionally, `focused_facing` is a positional index, and the FIRST arc's
/// `frequency` seeds the ship-wide shield frequency. Keeping matched entries
/// where the template put them and appending only what is new is what makes
/// keyed reconciliation safe for that array — see
/// `keyed_merge_keeps_template_order_and_appends_new_entries`.
///
/// # `behaviour.state` (the retired FSM)
///
/// This function still merges by an arbitrary key, including `name`, but
/// `behaviour.state` is no longer in either identity table. `BehaviourConfig`
/// is `deny_unknown_fields` and has had no `state` field since #572 dissolved
/// the FSM, so a resolved document carrying `[[behaviour.state]]` does not
/// parse and no shipped hull or fragment has one. Generalising a special case
/// for a field that cannot exist would have been carrying a corpse; #911
/// retired it instead. The `name`-keyed path itself is still exercised, by
/// `station.rating` and by the tests below.
///
/// # Test-only, because it hardcodes a policy a caller cannot see
///
/// This wrapper fixes two things its signature does not mention: the policy
/// ([`MergePolicy::ComposeFragments`], so tombstones ARE honoured) and the
/// starting path (`""`, so an array nested inside an entry is judged as if it
/// sat at the document root — an entry carrying `tags` would UNION rather than
/// replace). Both are right for the resolver and wrong for an instance
/// override, and it was `pub` and policy-neutral before #911, so leaving it
/// public would have left the compose policy one call away from the override
/// path. Production merges go through [`merge_entity_config_toml_with`], which
/// takes the policy explicitly; this stays for the unit tests that exercise the
/// element-wise rules directly.
#[cfg(test)]
fn merge_keyed_array(
    template: &[toml::Value],
    overrides: &[toml::Value],
    key: &str,
) -> Vec<toml::Value> {
    merge_keyed_array_at("", template, overrides, key, MergePolicy::ComposeFragments)
}

fn merge_keyed_array_at(
    path: &str,
    template: &[toml::Value],
    overrides: &[toml::Value],
    key: &str,
    policy: MergePolicy,
) -> Vec<toml::Value> {
    let mut result = template.to_vec();
    for o_entry in overrides {
        let Some(id) = o_entry.get(key).and_then(|v| v.as_str()) else {
            // Keyless entries have no identity to reconcile by, so they can
            // only be appended — the pre-#911 rule, unchanged.
            result.push(o_entry.clone());
            continue;
        };
        let pos = result
            .iter()
            .position(|e| e.get(key).and_then(|v| v.as_str()) == Some(id));
        match (pos, policy.accepts_removals() && is_removal(o_entry)) {
            // A tombstone drops the inherited entry and contributes nothing.
            (Some(i), true) => {
                result.remove(i);
            }
            // A tombstone for something nothing contributed is a no-op, not an
            // error: fragments compose in any order, and an author removing an
            // entry a sibling *might* provide should not have to know whether
            // it did.
            (None, true) => {}
            // Re-adding after a removal wins whole: the tombstone is not a
            // table to deep-merge into.
            (Some(i), false) if is_removal(&result[i]) => result[i] = o_entry.clone(),
            (Some(i), false) => result[i] = merge_at(path, &result[i], o_entry, policy),
            (None, false) => result.push(o_entry.clone()),
        }
    }
    result
}

/// Merge two arrays whose elements are TOML tables with a `name` field.
///
/// Thin wrapper over [`merge_keyed_array`]; see it for the full contract,
/// including why it is test-only.
#[cfg(test)]
fn merge_named_array(template: &[toml::Value], overrides: &[toml::Value]) -> Vec<toml::Value> {
    merge_keyed_array(template, overrides, "name")
}

/// Drop every surviving `_remove` entry from a composed document.
///
/// [`merge_keyed_array_at`] already consumes a tombstone that matched
/// something. This is the mop-up for the ones that never met a merge at all —
/// the first fragment in a closure is inserted whole, with no accumulator to
/// merge against — so that no `_remove` key can reach `EntityConfig`.
///
/// A marker left at the document's TOP level would be rejected there
/// (`EntityConfig` is `deny_unknown_fields`); one left inside a
/// `[[behaviour.doctrine]]` or a `[[system]]` entry would NOT be, because
/// `DoctrineObjective` and the `ship::config` structs are not. That asymmetry
/// is exactly why the marker is stripped structurally here rather than left for
/// a parser to catch. Same shape as `include_resolve::take_includes` stripping
/// `includes`, and for the same reason: an authoring marker must not exist at
/// runtime.
pub fn strip_removals(value: &toml::Value) -> toml::Value {
    match value {
        toml::Value::Table(table) => toml::Value::Table(
            table
                .iter()
                .map(|(k, v)| (k.clone(), strip_removals(v)))
                .collect(),
        ),
        toml::Value::Array(items) => toml::Value::Array(
            items
                .iter()
                .filter(|e| !is_removal(e))
                .map(strip_removals)
                .collect(),
        ),
        other => other.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// [`merge_entity_config_toml`] for the tests whose override is expected to
    /// be ACCEPTED. The merge is fallible only for a `_remove` tombstone
    /// (issue #911); `an_instance_override_is_rejected_for_a_tombstone` covers
    /// the other side.
    fn instance(template: &toml::Value, over: &toml::Value) -> toml::Value {
        merge_entity_config_toml(template, over)
            .expect("this override carries no `_remove` tombstone")
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
            &toml::from_str("[[system]]\nid = \"power-reactor\"\nkind = \"power_reactor\"\n")
                .unwrap(),
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
    /// `DoctrineObjective` is not `deny_unknown_fields`, so serde ignored it:
    /// before this test, `apply_overrides` returned `Ok`, the doctrine came back
    /// as `["patrol-ironveil", "destroy-hostiles"]`, and the author who asked
    /// for `destroy-hostiles` to be GONE got a hull that still had it and no
    /// warning. `src/ship/config.rs` has no `deny_unknown_fields` either, so
    /// `[[system]]`, `[[station]]` and `[[station.rating]]` are no safer.
    ///
    /// That is exactly the silent-no-op failure mode #838 existed to end, so
    /// the guarantee is enforced by the merge and pinned here END TO END.
    #[cfg(not(target_arch = "wasm32"))]
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

    /// CLEAR — the whole-array lever still works, and still beats the
    /// element-wise rules.
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
}
