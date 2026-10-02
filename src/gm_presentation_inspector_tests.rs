use super::*;
use std::collections::HashMap;

#[test]
fn inventory_covers_every_known_leaf_and_excludes_private_runtime_state() {
    let inventory = fields();
    let ids = inventory
        .iter()
        .map(|field| field.id.as_str())
        .collect::<Vec<_>>();
    for required in [
        "views.camera[].direction.z",
        "cue.title_card.subtitle",
        "cue.incoming_comms.message",
        "runtime.card.remaining_ticks",
        "asset.informative",
        "sound.equivalent.elevation",
    ] {
        assert!(ids.contains(&required), "missing descriptor {required}");
    }
    assert!(!ids.iter().any(|id| {
        id.contains("profile")
            || id.contains("mixer")
            || id.contains("autoplay")
            || id.contains("output")
    }));
    assert!(inventory
        .iter()
        .filter(|field| field.descriptor.live_mutability == LiveMutability::RecreateRequired)
        .all(
            |field| field.descriptor.validation == ["inspector.presentation.recreate_explanation"]
        ));
    assert!(inventory
        .iter()
        .filter(|field| field.descriptor.live_mutability == LiveMutability::NamedAction)
        .all(|field| field.action_panel.as_deref() == Some("presentation")));
}

#[test]
fn sound_reading_is_complete_and_only_viewscreen_definitions_link_actions() {
    let catalog = crate::sound_cues::bundled();
    let definition = catalog.cues.iter().find(|cue| cue.id == "weapons").unwrap();
    let asset = catalog
        .assets
        .iter()
        .find(|asset| asset.file == definition.file);
    let reading = sound_reading(catalog.version, definition, asset);
    assert_eq!(reading.values["sound.equivalent.bearing"], "45");
    assert_eq!(reading.values["asset.informative"], "true");
    assert!(reading.action_available);
    let private = catalog
        .cues
        .iter()
        .find(|cue| cue.id == "private-alert")
        .unwrap();
    assert!(!sound_reading(catalog.version, private, None).action_available);
    let asset = asset_reading(catalog.version, &catalog.assets[0]);
    assert_eq!(asset.values["asset.file"], catalog.assets[0].file);
    let empty_catalog = catalog_reading(1);
    assert_eq!(empty_catalog.values["catalog.version"], "1");
    assert!(empty_catalog
        .values
        .keys()
        .all(|key| !key.starts_with("sound.")));
    let zero_cues = crate::sound_cues::Catalog {
        version: 1,
        assets: vec![crate::sound_cues::Asset {
            file: "assets/sounds/unused.ogg".into(),
            category: "interface".into(),
            informative: false,
        }],
        cues: Vec::new(),
    };
    let readings = catalog_readings(&zero_cues);
    assert!(readings.contains_key("catalog:sound-cues"));
    assert!(readings.contains_key("asset:assets/sounds/unused.ogg"));
    assert!(!readings.keys().any(|key| key.starts_with("sound:")));
}

#[test]
fn ship_reading_uses_loaded_rig_and_canonical_active_state() {
    let markers = crate::entities::model_rig::ModelMarkers::from_markers(HashMap::from([(
        "camera_fore".into(),
        crate::entities::model_rig::Marker {
            position: [1.0, 2.0, 3.0],
            direction: [0.0, 0.0, -1.0],
        },
    )]));
    let state = crate::gm_presentation::ShipPresentation {
        forced_view: Some(crate::gm_presentation::TimedView {
            view: crate::gm_presentation::PresentationView::Camera("camera_fore".into()),
            until_tick: 25,
        }),
        card: Some(crate::gm_presentation::TimedCard {
            card: crate::gm_presentation::PresentationCard::Incoming {
                message: "hail".into(),
            },
            until_tick: 30,
        }),
    };
    let messages = [crate::gm_presentation::PresentationMessageChoice {
        message: "hail".into(),
        sender: "Lyra".into(),
        ship: Some("ship-a".into()),
    }];
    let reading = ship_reading(ShipPresentationInputs {
        ship_id: "ship-a",
        label: "Phoenix",
        template: Some("assets/entities/ship.toml"),
        layer: Some("assets/worlds/root.toml"),
        model: Some("assets/models/ship.glb"),
        variant: None,
        markers: Some(&markers),
        state: Some(&state),
        tick: 20,
        messages: &messages,
        card: Some(crate::gm_presentation::PresentationCardWire {
            kind: "incoming".into(),
            title: "Lyra".into(),
            body: "Dock now".into(),
            body_params: BTreeMap::new(),
            literal_body: true,
            literal_title: false,
        }),
    });
    assert_eq!(reading.values["views.camera[0].position.x"], "1");
    assert_eq!(
        reading.values["views.camera[0].rig_document"],
        "assets/models/ship.model.toml"
    );
    assert_eq!(reading.values["runtime.forced_view.remaining_ticks"], "5");
    assert_eq!(reading.values["runtime.card.message"], "hail");
    assert_eq!(reading.values["runtime.card.body"], "Dock now");
    assert_eq!(reading.values["comms.available[0].sender"], "Lyra");
    assert!(!reading.values.keys().any(|key| key.contains("profile")));
}
