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
/// * `behaviour.state` arrays merge **by name**: override entries replace
///   template entries with the same `name`; unmentioned entries are kept.
/// * `behaviour.transition` arrays are **full-replacement** when the override
///   supplies one (this is the default for arrays in `merge_toml`).
///
/// Call this instead of `merge_toml` when resolving `WorldEntity` overrides.
pub fn merge_entity_config_toml(template: &toml::Value, override_: &toml::Value) -> toml::Value {
    let mut result = merge_toml(template, override_);

    // Post-process: re-apply by-name merge for behaviour.state if both sides
    // supply a `state` array (merge_toml will have replaced it wholesale).
    let t_beh = template.get("behaviour").and_then(|v| v.as_table());
    let o_beh = override_.get("behaviour").and_then(|v| v.as_table());
    if let (Some(tb), Some(ob)) = (t_beh, o_beh) {
        if let (Some(t_states), Some(o_states)) = (
            tb.get("state").and_then(|v| v.as_array()),
            ob.get("state").and_then(|v| v.as_array()),
        ) {
            let merged_states = merge_named_array(t_states, o_states);
            // Write merged_states back into result.behaviour.state
            if let Some(result_beh) = result
                .as_table_mut()
                .and_then(|t| t.get_mut("behaviour"))
                .and_then(|v| v.as_table_mut())
            {
                result_beh.insert("state".to_string(), toml::Value::Array(merged_states));
            }
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
