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

/// Merge two TOML values with entity-config–aware special-casing:
///
/// * All table keys deep-merge as in `merge_toml`.
/// * `behaviour.state` arrays merge **by name** (`name` key): override entries
///   replace template entries with the same `name`; unmentioned entries are kept.
/// * `behaviour.doctrine` arrays merge **by id** (`id` key): override entries
///   replace template entries with the same `id`; unmentioned entries are kept.
/// * `behaviour.transition` arrays are **full-replacement** when the override
///   supplies one (this is the default for arrays in `merge_toml`).
///
/// # An authored empty array clears the list
///
/// The by-id / by-name merges above only apply when the override actually
/// supplies entries. An **explicitly authored empty array** (`doctrine = []`)
/// means "clear this list", not "merge nothing in": it is the only way a
/// scenario can take a behaviour *away* from a template, and it is what every
/// other array in an override already does (`merge_toml` replaces arrays
/// wholesale — see `override_replaces_array_wholesale`). Before this,
/// `assets/worlds/probe_aggressor.toml`'s `behaviour = { doctrine = [] }` was a
/// silent no-op: the "passive" hostile it describes kept the template's
/// `destroy-hostiles` Destroy doctrine and opened fire first.
///
/// Omitting the key entirely is still the way to say "leave the template's list
/// alone" — an absent key never reaches the merge.
///
/// Call this instead of `merge_toml` when resolving `WorldEntity` overrides.
pub fn merge_entity_config_toml(template: &toml::Value, override_: &toml::Value) -> toml::Value {
    let mut result = merge_toml(template, override_);

    let t_beh = template.get("behaviour").and_then(|v| v.as_table());
    let o_beh = override_.get("behaviour").and_then(|v| v.as_table());
    if let (Some(tb), Some(ob)) = (t_beh, o_beh) {
        // Post-process: re-apply by-name merge for behaviour.state.
        // Skipped for an empty override array, which `merge_toml` has already
        // resolved to the cleared list — see "An authored empty array clears
        // the list" above.
        if let (Some(t_states), Some(o_states)) = (
            tb.get("state").and_then(|v| v.as_array()),
            ob.get("state")
                .and_then(|v| v.as_array())
                .filter(|a| !a.is_empty()),
        ) {
            let merged_states = merge_named_array(t_states, o_states);
            if let Some(result_beh) = result
                .as_table_mut()
                .and_then(|t| t.get_mut("behaviour"))
                .and_then(|v| v.as_table_mut())
            {
                result_beh.insert("state".to_string(), toml::Value::Array(merged_states));
            }
        }

        // Post-process: re-apply by-id merge for behaviour.doctrine.
        // Skipped for an empty override array, which `merge_toml` has already
        // resolved to the cleared list — see "An authored empty array clears
        // the list" above.
        if let (Some(t_doctrines), Some(o_doctrines)) = (
            tb.get("doctrine").and_then(|v| v.as_array()),
            ob.get("doctrine")
                .and_then(|v| v.as_array())
                .filter(|a| !a.is_empty()),
        ) {
            let merged_doctrines = merge_id_array(t_doctrines, o_doctrines);
            if let Some(result_beh) = result
                .as_table_mut()
                .and_then(|t| t.get_mut("behaviour"))
                .and_then(|v| v.as_table_mut())
            {
                result_beh.insert("doctrine".to_string(), toml::Value::Array(merged_doctrines));
            }
        }
    }

    result
}

/// Merge two arrays whose elements are TOML tables with an `id` field.
///
/// Override entries replace template entries with the same id.
/// Template entries with no matching override are kept at their original
/// position. Override entries with no matching template entry are appended.
pub fn merge_id_array(template: &[toml::Value], overrides: &[toml::Value]) -> Vec<toml::Value> {
    let mut result = template.to_vec();
    for o_entry in overrides {
        let o_id = o_entry.get("id").and_then(|v| v.as_str());
        match o_id {
            Some(id) => {
                let pos = result
                    .iter()
                    .position(|e| e.get("id").and_then(|v| v.as_str()) == Some(id));
                match pos {
                    Some(i) => result[i] = merge_toml(&result[i], o_entry),
                    None => result.push(o_entry.clone()),
                }
            }
            None => result.push(o_entry.clone()),
        }
    }
    result
}

/// Merge two arrays whose elements are TOML tables with a `name` field.
///
/// Override entries replace template entries with the same name.
/// Template entries with no matching override are kept at their original
/// position. Override entries with no matching template entry are appended.
pub fn merge_named_array(template: &[toml::Value], overrides: &[toml::Value]) -> Vec<toml::Value> {
    let mut result = template.to_vec();
    for o_entry in overrides {
        let o_name = o_entry.get("name").and_then(|v| v.as_str());
        match o_name {
            Some(name) => {
                let pos = result
                    .iter()
                    .position(|e| e.get("name").and_then(|v| v.as_str()) == Some(name));
                match pos {
                    Some(i) => result[i] = merge_toml(&result[i], o_entry),
                    None => result.push(o_entry.clone()),
                }
            }
            None => result.push(o_entry.clone()),
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

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

    // ── merge_entity_config_toml: behaviour.state by-name semantics ───────

    #[test]
    fn entity_merge_behaviour_state_by_name_replaces_matching_entry() {
        let template: toml::Value = toml::from_str(
            r#"
[behaviour]
initial_state = "patrol"

[[behaviour.state]]
name = "patrol"
kind = "patrolling"
target_speed = 0.5

[[behaviour.state]]
name = "idle"
kind = "idle"
target_speed = 0.0
"#,
        )
        .unwrap();

        let override_: toml::Value = toml::from_str(
            r#"
[[behaviour.state]]
name = "patrol"
target_speed = 0.9
"#,
        )
        .unwrap();

        let result = merge_entity_config_toml(&template, &override_);
        let states = result
            .get("behaviour")
            .unwrap()
            .get("state")
            .unwrap()
            .as_array()
            .unwrap();
        assert_eq!(states.len(), 2, "idle must be kept");
        let patrol = states
            .iter()
            .find(|s| s.get("name").and_then(|v| v.as_str()) == Some("patrol"))
            .unwrap();
        assert_eq!(
            patrol.get("target_speed").and_then(|v| v.as_float()),
            Some(0.9)
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

        let result = merge_entity_config_toml(&doctrine_template(), &override_);
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
        let result = merge_entity_config_toml(&doctrine_template(), &override_);
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
        let result = merge_entity_config_toml(&doctrine_template(), &override_);
        assert_eq!(
            doctrine_ids(&result),
            vec!["destroy-hostiles", "hold-station"]
        );
    }

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
        let result = merge_entity_config_toml(&template, &override_);
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

        let result = merge_entity_config_toml(&template, &override_);
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
