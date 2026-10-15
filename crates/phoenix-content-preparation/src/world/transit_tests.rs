use super::*;

fn body() -> (WorldEntity, EntityConfig) {
    let row = toml::from_str(
        "template_path='body.toml'\nname='static-body'\ntransform={position=[10,2,3]}\n",
    )
    .unwrap();
    let config = EntityConfig {
        arrival_gravity: Some(phoenix_model::transit::ArrivalGravityConfig {
            exclusion_radius: 5.0,
        }),
        ..Default::default()
    };
    (row, config)
}

#[test]
fn ordinary_static_body_uses_explicit_physical_policy_and_absolute_world_pose() {
    let (row, config) = body();
    let accepted = body_for_row(&row, &config).unwrap().unwrap();
    assert_eq!(accepted.position, [10.0, 2.0, 3.0]);
    assert_eq!(accepted.body_id, "static-body");
    let mut visual_only = config;
    visual_only.arrival_gravity = None;
    assert_eq!(body_for_row(&row, &visual_only).unwrap(), None);
}

#[test]
fn ambiguous_dynamic_or_nonfinite_sources_are_actionable_refusals() {
    for fault in 0..8 {
        let (mut row, mut config) = body();
        match fault {
            0 => row.transform.as_mut().unwrap().anchor = Some("hidden-anchor".into()),
            1 => row.transform.as_mut().unwrap().relative_to = Some("moving-parent".into()),
            2 => row.transform.as_mut().unwrap().position = None,
            3 => row.when = Some("flag(later)".into()),
            4 => row.name = None,
            5 => row.transform.as_mut().unwrap().position = Some([f32::INFINITY, 0.0, 0.0]),
            6 => config.tags.push("ship".into()),
            _ => config.arrival_gravity.as_mut().unwrap().exclusion_radius = f32::NAN,
        }
        assert!(body_for_row(&row, &config).is_err(), "fault{fault}");
    }
}
