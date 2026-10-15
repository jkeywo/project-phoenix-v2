use super::*;

struct BodyTemplate;
impl TemplateLoader for BodyTemplate {
    fn absence_is_final(&self) -> bool {
        true
    }
    fn load_template(&self, _path: &str) -> Option<crate::entities::config::EntityConfig> {
        Some(crate::entities::config::EntityConfig {
            mass: crate::entities::config::DEFAULT_ENTITY_MASS,
            ..Default::default()
        })
    }
}
fn source(text: &str) -> WorldConfig {
    crate::world::config::parse_world(text).unwrap()
}

#[test]
fn ordinary_resolved_world_override_is_valid_but_layered_static_gravity_refuses_actionably() {
    let text = r#"
[[entity]]
template_path="body.toml"
name="body"
transform={position=[100,0,10]}
[entity.overrides.arrival_gravity]
exclusion_radius=10
"#;
    let config = source(text);
    let row = WorldSource::new("base.toml", text, &config);
    assert!(validate_arrival_gravity(&row, &BodyTemplate, true).is_empty());
    let unsupported = validate_arrival_gravity(&row, &BodyTemplate, false);
    assert_eq!(unsupported.len(), 1);
    assert_eq!(unsupported[0].severity, Severity::Error);
    assert_eq!(unsupported[0].category, "unsupported-arrival-gravity");
    assert!(unsupported[0].message.contains("base World"));
    let text = text.replace(
        "position=[100,0,10]",
        "position=[100,0,10],relative_to='moving'",
    );
    let config = source(&text);
    assert_eq!(
        validate_arrival_gravity(
            &WorldSource::new("base.toml", &text, &config),
            &BodyTemplate,
            true
        )
        .len(),
        1
    );
}

#[test]
fn accepted_static_source_bound_refuses_instead_of_silently_omitting_a_body() {
    let mut config = WorldConfig::default();
    for index in 0..=phoenix_model::transit::MAX_TRANSIT_GRAVITY_BODIES {
        config.entities.push(
            toml::from_str(&format!(
                r#"
template_path="body.toml"
name="body-{index}"
transform={{position=[100,0,10]}}
[overrides.arrival_gravity]
exclusion_radius=10
"#
            ))
            .unwrap(),
        );
    }
    let findings = validate_arrival_gravity(
        &WorldSource::new("base.toml", "", &config),
        &BodyTemplate,
        true,
    );
    assert_eq!(findings.len(), 1);
    assert!(findings[0]
        .message
        .contains("finite static arrival body bound"));
}

#[test]
fn exact_existing_resolved_script_spawn_inventory_rejects_gravity_template_at_its_source() {
    struct GravityTemplate;
    impl TemplateLoader for GravityTemplate {
        fn absence_is_final(&self) -> bool {
            true
        }
        fn load_template(&self, _: &str) -> Option<crate::entities::config::EntityConfig> {
            Some(crate::entities::config::EntityConfig {
                arrival_gravity: Some(phoenix_model::transit::ArrivalGravityConfig {
                    exclusion_radius: 8.0,
                }),
                mass: crate::entities::config::DEFAULT_ENTITY_MASS,
                ..Default::default()
            })
        }
    }
    let config = WorldConfig::default();
    let refs = [crate::world::config::ResolvedScriptSpawnRef {
        source_path: "activity.rhai".into(),
        line: 17,
        template_path: "body.toml".into(),
        overrides: None,
    }];
    let source = WorldSource::new("base.toml", "", &config).with_resolved_script_spawns(&refs);
    let findings = validate_arrival_gravity(&source, &GravityTemplate, true);
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].category, "unsupported-arrival-gravity");
    assert_eq!(findings[0].source.file, "activity.rhai");
    assert_eq!(findings[0].source.line, Some(17));
    assert!(validate_arrival_gravity(&source, &BodyTemplate, true).is_empty());
}
