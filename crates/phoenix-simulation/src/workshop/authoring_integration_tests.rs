use super::*;
use document::edit;
const ALLIANCE: &str = "aaaaaaaa-1111-4111-8111-aaaaaaaaaaaa";
const ROGUE: &str = "eeeeeeee-5555-4555-8555-eeeeeeeeeeee";
#[test]
fn workshop_slot_form_requests_preserve_exact_source() {
    for (name, request) in crate::core::codec::workshop_slot_edit_fixtures() {
        let source = &request.expected_source;
        let after = edit(source, &request).unwrap();
        if name == "label" {
            assert_eq!(
                after,
                source.replace(
                    "label='Original' # label tail\r\n",
                    "label=\"Changed\" # label tail\n"
                )
            );
        } else {
            assert!(after.starts_with(source));
            let world = crate::world::config::parse_world(&after).unwrap();
            assert_eq!(world.ship_slots[2].id, "wing");
            assert_eq!(
                world.ship_slots[2].ships[0].template_path,
                "assets/entities/base.toml"
            );
        }
    }
}

#[test]
fn the_native_provider_answers_definitions_edit_and_new_faction() {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or_default();
    let directory = std::env::temp_dir().join(format!(
        "phoenix-workshop-definitions-{}-{nanos}",
        std::process::id()
    ));
    let root = directory.join("project");
    std::fs::create_dir_all(root.join("assets/factions")).unwrap();
    std::fs::write(
        root.join("assets/scenarios.toml"),
        "[content]\nid='phoenix-base'\nepoch=1\n",
    )
    .unwrap();
    let source = format!("uuid = \"{ROGUE}\"\nname = \"Rogue\"\nenemies = []\n");
    std::fs::write(root.join("assets/factions/rogue.toml"), &source).unwrap();
    let mut provider = provider::NativeWorkshopProvider::open(
        provider::WorkspaceKind::Project,
        &root,
        directory.join("private"),
        WorkshopDependencies::default(),
    )
    .unwrap();

    let catalog = provider.handle_json(&format!(
        "{{\"id\":1,\"op\":\"definitions\",\"files\":{{\"assets/factions/rogue.toml\":\"uuid = \\\"{ROGUE}\\\"\\nname = \\\"Rogue\\\"\\nenemies = []\\n\"}}}}"
    ));
    assert!(catalog.contains("\"status\":\"definitions\""), "{catalog}");
    assert!(catalog.contains("\"origin\":\"draft\""), "{catalog}");
    assert!(
        catalog.contains("\"order_responses\":[\"comply\",\"refuse\"]"),
        "{catalog}"
    );
    assert!(
        catalog.contains("\"ai_rules\":[\"torpedo_auto_fire\"]"),
        "{catalog}"
    );

    let skeleton = provider.handle_json(&format!(
        "{{\"id\":2,\"op\":\"new-faction\",\"name\":\"Rogue\",\"uuid\":\"{ROGUE}\"}}"
    ));
    assert!(skeleton.contains("\"status\":\"patched\""), "{skeleton}");
    assert!(skeleton.contains("name = \\\"Rogue\\\""), "{skeleton}");
    let refused = provider
        .handle_json("{\"id\":3,\"op\":\"new-faction\",\"name\":\" \",\"uuid\":\"not-a-uuid\"}");
    assert!(refused.contains("\"status\":\"refused\""), "{refused}");

    let edited = provider.handle_json(&format!(
        "{{\"id\":4,\"op\":\"edit\",\"source\":\"uuid = 1\\n\",\"edit\":{{\"document_path\":\"assets/factions/rogue.toml\",\"expected_source\":\"uuid = 1\\n\",\"edits\":[{{\"op\":\"put\",\"path\":[\"name\"],\"value_source\":\"\\\"Renamed\\\"\"}},{{\"op\":\"insert\",\"path\":[\"enemies\"],\"index\":0,\"value_source\":\"\\\"{ALLIANCE}\\\"\"}}]}}}}"
    ));
    assert!(
        edited.contains("\"status\":\"refused\""),
        "an insert into a missing array refuses the whole group: {edited}"
    );
    let edited = provider.handle_json(
        "{\"id\":5,\"op\":\"edit\",\"source\":\"uuid = 1\\n\",\"edit\":{\"document_path\":\"assets/factions/rogue.toml\",\"expected_source\":\"uuid = 1\\n\",\"edits\":[{\"op\":\"put\",\"path\":[\"name\"],\"value_source\":\"\\\"Renamed\\\"\"}]}}",
    );
    assert!(edited.contains("\"status\":\"patched\""), "{edited}");
    assert!(
        edited.contains("\"source\":\"uuid = 1\\nname = \\\"Renamed\\\"\\n\""),
        "{edited}"
    );
    drop(provider);
    let _ = std::fs::remove_dir_all(&directory);
}
