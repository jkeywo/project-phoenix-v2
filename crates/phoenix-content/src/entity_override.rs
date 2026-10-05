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
    /// and, historically, `DoctrineObjective` let serde ignore the key;
    /// `apply_overrides` returned `Ok` and the doctrine stayed unchanged.
    /// Measured against the real `ship_harrow_patrol` hull, that was `ACCEPTED
    /// SILENTLY`. Issue #1268 now captures and rejects unknown doctrine keys,
    /// but the merge policy still owns whether this marker is honoured, and
    /// the sibling `ship::config` structs remain permissive. Enforce the
    /// guarantee where it is stated rather than delegating it downstream.
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
/// deep-merges into the matching template entry. Before issue #1268 it then
/// disappeared into serde's unknown-field ignore and the load SUCCEEDED with
/// the doctrine intact. The doctrine parser now rejects unknown keys too, but
/// this layer still owns the stronger rule: instance overrides do not honour
/// tombstones anywhere, including in permissive sibling config structs.
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
/// `[[system]]` entry would NOT be because the `ship::config` structs are
/// permissive. `DoctrineObjective` joined the rejecting side in issue #1268,
/// but that partial asymmetry is still why the marker is stripped structurally
/// rather than left for a parser to catch. Same shape as
/// `include_resolve::take_includes` stripping `includes`, and for the same
/// reason: an authoring marker must not exist at runtime.
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
#[path = "entity_override_tests.rs"]
mod tests;
