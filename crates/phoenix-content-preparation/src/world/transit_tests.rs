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

#[test]
fn destination_support_walk_promotes_already_read_content_without_duplicates() {
    use crate::world::config::parse_world;
    let root = parse_world("extra_worlds=['assets/worlds/shared.toml','assets/worlds/legacy.toml']\n[[transit_destination]]\nid='outer'\nlabel='world.outer'\ntarget={kind='layer',path='assets/worlds/outer.toml',anchor='arrival'}").unwrap();
    let mut walk = SupportingWorldWalk::new("assets/worlds/root.toml", &root).unwrap();
    let mut seen = Vec::new();
    while let Some(path) = walk.next_path() {
        let text = match path.as_str() {
            "assets/worlds/shared.toml" => "extra_worlds=['assets/worlds/deep.toml']",
            "assets/worlds/legacy.toml" => "extra_worlds=['assets/worlds/legacy-deep.toml']",
            "assets/worlds/outer.toml" => "extra_worlds=['assets/worlds/shared.toml']",
            "assets/worlds/deep.toml" => "extra_worlds=['assets/worlds/outer.toml']",
            other => panic!("unrequested content {other}"),
        };
        walk.accept(&path, &parse_world(text).unwrap()).unwrap();
        if path == "assets/worlds/deep.toml" {
            assert_eq!(walk.parent_path(&path), Some("assets/worlds/shared.toml"));
        } else {
            assert_eq!(walk.parent_path(&path), None);
        }
        seen.push(path);
    }
    assert_eq!(
        seen,
        [
            "assets/worlds/shared.toml",
            "assets/worlds/legacy.toml",
            "assets/worlds/outer.toml",
            "assets/worlds/deep.toml"
        ]
    );
}

#[test]
fn destination_walk_refuses_root_reload_and_noncanonical_support_paths() {
    use crate::world::config::parse_world;
    let root = parse_world("[[transit_destination]]\nid='outer'\nlabel='world.outer'\ntarget={kind='layer',path='assets/worlds/outer.toml',anchor='arrival'}").unwrap();
    for invalid in [
        "assets/worlds/root.toml",
        "assets/worlds/../outer.toml",
        "arbitrary.toml",
    ] {
        let mut walk = SupportingWorldWalk::new("assets/worlds/root.toml", &root).unwrap();
        let outer = walk.next_path().unwrap();
        let child = crate::world::config::WorldConfig {
            extra_worlds: vec![invalid.into()],
            ..Default::default()
        };
        assert!(walk.accept(&outer, &child).is_err(), "{invalid}");
    }
}

#[test]
fn accepted_destination_requires_its_own_finite_anchor_and_unique_resident_names() {
    use crate::world::config::parse_world;
    use crate::world::validate::WorldSource;
    let text = "[anchors]\nhome=[0,0]\n[[transit_destination]]\nid='home'\nlabel='world.home'\ntarget={kind='base',anchor='home'}\n[[transit_destination]]\nid='outer'\nlabel='world.outer'\ntarget={kind='layer',path='assets/worlds/outer.toml',anchor='arrival'}\n[[entity]]\nname='same'\ntemplate_path='body.toml'";
    let root_config = parse_world(text).unwrap();
    let root = WorldSource::new("assets/worlds/root.toml", text, &root_config);
    assert_eq!(destination_findings(&root, &[]).len(), 1);
    let child_text =
        "[anchors]\narrival=[1000,0]\n[[entity]]\nname='same'\ntemplate_path='body.toml'";
    let mut child_config = parse_world(child_text).unwrap();
    let child = WorldSource::new("assets/worlds/outer.toml", child_text, &child_config);
    assert_eq!(
        destination_findings(&root, &[child])[0].category,
        "ambiguous-transit-actor"
    );
    child_config.entities[0].name = Some("distinct".into());
    let child = WorldSource::new("assets/worlds/outer.toml", child_text, &child_config);
    assert!(destination_findings(&root, &[child]).is_empty());
    child_config.anchors.get_mut("arrival").unwrap()[0] = f32::INFINITY;
    let child = WorldSource::new("assets/worlds/outer.toml", child_text, &child_config);
    assert_eq!(
        destination_findings(&root, &[child])[0].category,
        "invalid-transit-destination"
    );
}

#[test]
fn activate_load_retains_dormant_destination_content_without_startup_requests() {
    use crate::world::load::{load, LoadPolicy, LoadRequest, MemoryReader, MemoryTemplateLoader};
    use crate::world::script::load::NoSiblingScripts;
    let root = "[[transit_destination]]\nid='outer'\nlabel='world.outer'\ntarget={kind='layer',path='assets/worlds/outer.toml',anchor='arrival'}";
    let reader = MemoryReader::new([
        ("assets/worlds/root.toml", root),
        (
            "assets/worlds/outer.toml",
            "extra_worlds=['assets/worlds/support.toml']\n[anchors]\narrival=[1000,0]",
        ),
        ("assets/worlds/support.toml", "[global]\ntitle='Support'"),
    ]);
    let loaded = load(
        LoadRequest::new(
            "assets/worlds/root.toml",
            &reader,
            &NoSiblingScripts,
            LoadPolicy::Activate,
        ),
        &MemoryTemplateLoader::authoritative_empty(),
        &std::collections::BTreeMap::<String, String>::new(),
    )
    .unwrap();
    assert!(
        loaded
            .findings
            .iter()
            .all(|row| row.severity != crate::world::validate::Severity::Error),
        "{:?}",
        loaded.findings
    );
    assert!(
        loaded.config.extra_worlds.is_empty(),
        "prefetch must not author startup activation"
    );
    assert_eq!(loaded.children.len(), 2);
    assert_eq!(
        loaded
            .ledger
            .records
            .iter()
            .map(|row| row.path.as_str())
            .collect::<Vec<_>>(),
        [
            "assets/worlds/root.toml",
            "assets/worlds/outer.toml",
            "assets/worlds/support.toml"
        ]
    );
}

#[test]
fn support_walk_keeps_actual_declaring_world_parent_and_refuses_excess_scope() {
    use crate::world::config::{parse_world, WorldConfig};
    let root = parse_world("[[transit_destination]]\nid='outer'\nlabel='world.outer'\ntarget={kind='layer',path='assets/worlds/outer.toml',anchor='arrival'}").unwrap();
    let mut walk = SupportingWorldWalk::new("assets/worlds/root.toml", &root).unwrap();
    let mut parent = None;
    for index in 0..128 {
        let path = walk.next_path().unwrap();
        assert_eq!(walk.parent_path(&path), parent.as_deref());
        let next = format!("assets/worlds/support-{index}.toml");
        let result = walk.accept(
            &path,
            &WorldConfig {
                extra_worlds: vec![next],
                ..Default::default()
            },
        );
        if index == 127 {
            assert!(result.unwrap_err().contains("128-World"));
        } else {
            result.unwrap();
        }
        parent = Some(path);
    }
}
