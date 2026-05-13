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
        assert_eq!(
            result.get("name").and_then(|v| v.as_str()),
            Some("base")
        );
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
        assert_eq!(
            result.get("online").and_then(|v| v.as_bool()),
            Some(false)
        );
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
}
