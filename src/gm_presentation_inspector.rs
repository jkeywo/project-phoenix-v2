//! Read-only Live Inspector for authored Viewscreen presentation and sound.
//!
//! This projects retained public content only. Operator profiles, browser audio
//! policy, mixer state and physical outputs are deliberately absent, and sound
//! occurrences remain occurrences rather than becoming a second history.
use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::inspector::{FieldDescriptor, FieldOrigin, LiveMutability};

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct PresentationInspectorField {
    pub id: String,
    pub label: String,
    pub group: String,
    #[serde(flatten)]
    pub descriptor: FieldDescriptor,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action_panel: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct PresentationInspection {
    pub label: String,
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ship_id: Option<String>,
    #[serde(default)]
    pub action_available: bool,
    pub values: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct PresentationInspectorProjection {
    pub fields: Vec<PresentationInspectorField>,
    pub readings: BTreeMap<String, PresentationInspection>,
}

impl PresentationInspectorProjection {
    pub fn is_empty(&self) -> bool {
        self.fields.is_empty() && self.readings.is_empty()
    }
}

fn group(path: &str) -> String {
    path.split('.').next().unwrap_or("runtime").into()
}

fn field(
    path: &str,
    kind: &str,
    mutability: LiveMutability,
    action_panel: Option<&str>,
) -> PresentationInspectorField {
    PresentationInspectorField {
        id: path.into(),
        label: "inspector.presentation.field".into(),
        group: group(path),
        descriptor: FieldDescriptor {
            kind: kind.into(),
            default_source: None,
            live_mutability: mutability,
            origin: FieldOrigin {
                schema_path: path.into(),
                document: None,
                line: None,
                layer: None,
            },
            validation: if mutability == LiveMutability::RecreateRequired {
                vec!["inspector.presentation.recreate_explanation".into()]
            } else {
                Vec::new()
            },
        },
        action_panel: action_panel.map(str::to_owned),
    }
}

/// Explicit ratchet for every known loaded Viewscreen/cue/catalog leaf.
pub fn fields() -> Vec<PresentationInspectorField> {
    use LiveMutability::{Derived, NamedAction, RecreateRequired};
    [
        ("identity.kind", "enum", Derived, None),
        ("identity.ship", "optional", Derived, None),
        ("identity.definition", "string", Derived, None),
        ("provenance.document", "string", Derived, None),
        ("provenance.layer", "optional", Derived, None),
        ("views.mode[].id", "enum", NamedAction, Some("presentation")),
        (
            "views.camera[].name",
            "string",
            NamedAction,
            Some("presentation"),
        ),
        (
            "views.camera[].model_asset",
            "string",
            RecreateRequired,
            None,
        ),
        (
            "views.camera[].rig_document",
            "string",
            RecreateRequired,
            None,
        ),
        ("views.camera[].position.x", "float", RecreateRequired, None),
        ("views.camera[].position.y", "float", RecreateRequired, None),
        ("views.camera[].position.z", "float", RecreateRequired, None),
        (
            "views.camera[].direction.x",
            "float",
            RecreateRequired,
            None,
        ),
        (
            "views.camera[].direction.y",
            "float",
            RecreateRequired,
            None,
        ),
        (
            "views.camera[].direction.z",
            "float",
            RecreateRequired,
            None,
        ),
        (
            "cue.force_view.view",
            "enum",
            NamedAction,
            Some("presentation"),
        ),
        (
            "cue.force_view.duration_ticks",
            "integer",
            NamedAction,
            Some("presentation"),
        ),
        (
            "cue.release_view",
            "action",
            NamedAction,
            Some("presentation"),
        ),
        (
            "cue.title_card.title",
            "string",
            NamedAction,
            Some("presentation"),
        ),
        (
            "cue.title_card.subtitle",
            "string",
            NamedAction,
            Some("presentation"),
        ),
        (
            "cue.title_card.duration_ticks",
            "integer",
            NamedAction,
            Some("presentation"),
        ),
        (
            "cue.incoming_comms.message",
            "reference",
            NamedAction,
            Some("presentation"),
        ),
        (
            "cue.incoming_comms.duration_ticks",
            "integer",
            NamedAction,
            Some("presentation"),
        ),
        (
            "cue.clear_card",
            "action",
            NamedAction,
            Some("presentation"),
        ),
        (
            "cue.sound.id",
            "reference",
            NamedAction,
            Some("presentation"),
        ),
        (
            "cue.sound.source",
            "optional",
            NamedAction,
            Some("presentation"),
        ),
        (
            "comms.available[].message",
            "reference",
            NamedAction,
            Some("presentation"),
        ),
        ("comms.available[].sender", "string", Derived, None),
        (
            "comms.available[].recipient_ship",
            "optional",
            Derived,
            None,
        ),
        ("runtime.forced_view.kind", "optional", Derived, None),
        ("runtime.forced_view.id", "optional", Derived, None),
        ("runtime.forced_view.until_tick", "optional", Derived, None),
        (
            "runtime.forced_view.remaining_ticks",
            "optional",
            Derived,
            None,
        ),
        ("runtime.card.kind", "optional", Derived, None),
        ("runtime.card.title", "optional", Derived, None),
        ("runtime.card.body", "optional", Derived, None),
        ("runtime.card.message", "optional", Derived, None),
        ("runtime.card.until_tick", "optional", Derived, None),
        ("runtime.card.remaining_ticks", "optional", Derived, None),
        ("runtime.sound", "enum", Derived, None),
        ("catalog.version", "integer", RecreateRequired, None),
        ("asset.file", "asset", RecreateRequired, None),
        ("asset.category", "enum", RecreateRequired, None),
        ("asset.informative", "bool", RecreateRequired, None),
        ("sound.id", "string", RecreateRequired, None),
        ("sound.label", "string", RecreateRequired, None),
        ("sound.file", "asset", RecreateRequired, None),
        ("sound.category", "enum", RecreateRequired, None),
        ("sound.audience", "enum", RecreateRequired, None),
        ("sound.volume", "float", RecreateRequired, None),
        ("sound.equivalent.present", "bool", RecreateRequired, None),
        (
            "sound.equivalent.meaning",
            "optional",
            RecreateRequired,
            None,
        ),
        (
            "sound.equivalent.source",
            "optional",
            RecreateRequired,
            None,
        ),
        (
            "sound.equivalent.urgency",
            "optional",
            RecreateRequired,
            None,
        ),
        (
            "sound.equivalent.bearing",
            "optional",
            RecreateRequired,
            None,
        ),
        (
            "sound.equivalent.elevation",
            "optional",
            RecreateRequired,
            None,
        ),
    ]
    .into_iter()
    .map(|(path, kind, mutability, owner)| field(path, kind, mutability, owner))
    .collect()
}

fn put(values: &mut BTreeMap<String, String>, path: &str, value: impl ToString) {
    values.insert(path.into(), value.to_string());
}

fn optional(values: &mut BTreeMap<String, String>, path: &str, value: Option<impl ToString>) {
    put(
        values,
        path,
        value
            .map(|value| value.to_string())
            .unwrap_or_else(|| "not-active".into()),
    );
}

pub struct ShipPresentationInputs<'a> {
    pub ship_id: &'a str,
    pub label: &'a str,
    pub template: Option<&'a str>,
    pub layer: Option<&'a str>,
    pub model: Option<&'a str>,
    pub variant: Option<&'a str>,
    pub markers: Option<&'a crate::entities::model_rig::ModelMarkers>,
    pub state: Option<&'a crate::gm_presentation::ShipPresentation>,
    pub tick: u64,
    pub messages: &'a [crate::gm_presentation::PresentationMessageChoice],
    pub card: Option<crate::gm_presentation::PresentationCardWire>,
}

pub fn ship_reading(input: ShipPresentationInputs<'_>) -> PresentationInspection {
    use crate::gm_presentation::{PresentationCard, PresentationView};
    let mut values = BTreeMap::new();
    put(&mut values, "identity.kind", "viewscreen-presentation");
    put(&mut values, "identity.ship", input.ship_id);
    put(&mut values, "identity.definition", input.ship_id);
    optional(&mut values, "provenance.document", input.template);
    optional(&mut values, "provenance.layer", input.layer);
    for (index, mode) in ["radar", "sensors_radar", "navigation_chart", "cinematic"]
        .into_iter()
        .enumerate()
    {
        put(&mut values, &format!("views.mode[{index}].id"), mode);
    }
    let mut cameras = input
        .markers
        .into_iter()
        .flat_map(crate::entities::model_rig::ModelMarkers::marker_names)
        .filter(|name| name.starts_with("camera_"))
        .collect::<Vec<_>>();
    cameras.sort_unstable();
    for (index, name) in cameras.into_iter().enumerate() {
        let marker = input
            .markers
            .and_then(|markers| markers.get(name))
            .expect("marker name came from marker map");
        put(&mut values, &format!("views.camera[{index}].name"), name);
        optional(
            &mut values,
            &format!("views.camera[{index}].model_asset"),
            input.model,
        );
        optional(
            &mut values,
            &format!("views.camera[{index}].rig_document"),
            input
                .model
                .map(|model| crate::entities::model_rig::sidecar_path(model, input.variant)),
        );
        for (axis, value) in ["x", "y", "z"].into_iter().zip(marker.position) {
            put(
                &mut values,
                &format!("views.camera[{index}].position.{axis}"),
                value,
            );
        }
        for (axis, value) in ["x", "y", "z"].into_iter().zip(marker.direction) {
            put(
                &mut values,
                &format!("views.camera[{index}].direction.{axis}"),
                value,
            );
        }
    }
    for (path, value) in [
        ("cue.force_view.view", "select-view"),
        ("cue.force_view.duration_ticks", "required-positive-integer"),
        ("cue.release_view", "available"),
        ("cue.title_card.title", "required-text"),
        ("cue.title_card.subtitle", "text"),
        ("cue.title_card.duration_ticks", "required-positive-integer"),
        ("cue.incoming_comms.message", "select-message"),
        (
            "cue.incoming_comms.duration_ticks",
            "required-positive-integer",
        ),
        ("cue.clear_card", "available"),
        ("cue.sound.id", "select-sound"),
        ("cue.sound.source", "optional-public-entity"),
    ] {
        put(&mut values, path, value);
    }
    for (index, message) in input
        .messages
        .iter()
        .filter(|message| {
            message
                .ship
                .as_deref()
                .is_none_or(|ship| ship == input.ship_id)
        })
        .enumerate()
    {
        put(
            &mut values,
            &format!("comms.available[{index}].message"),
            &message.message,
        );
        put(
            &mut values,
            &format!("comms.available[{index}].sender"),
            &message.sender,
        );
        optional(
            &mut values,
            &format!("comms.available[{index}].recipient_ship"),
            message.ship.as_deref(),
        );
    }
    let forced = input.state.and_then(|state| state.forced_view.as_ref());
    let (view_kind, view_id) = forced.map_or((None, None), |forced| match &forced.view {
        PresentationView::Camera(name) => (Some("camera"), Some(name.as_str())),
        PresentationView::Radar => (Some("view"), Some("radar")),
        PresentationView::SensorsRadar => (Some("view"), Some("sensors_radar")),
        PresentationView::NavigationChart => (Some("view"), Some("navigation_chart")),
        PresentationView::Cinematic => (Some("view"), Some("cinematic")),
    });
    optional(&mut values, "runtime.forced_view.kind", view_kind);
    optional(&mut values, "runtime.forced_view.id", view_id);
    optional(
        &mut values,
        "runtime.forced_view.until_tick",
        forced.map(|row| row.until_tick),
    );
    optional(
        &mut values,
        "runtime.forced_view.remaining_ticks",
        forced.map(|row| row.until_tick.saturating_sub(input.tick)),
    );
    let active_card = input.state.and_then(|state| state.card.as_ref());
    let (kind, message) = active_card.map_or((None, None), |card| match &card.card {
        PresentationCard::Title { .. } => (Some("title"), None),
        PresentationCard::Incoming { message } => (Some("incoming-comms"), Some(message.as_str())),
    });
    optional(&mut values, "runtime.card.kind", kind);
    optional(
        &mut values,
        "runtime.card.title",
        input.card.as_ref().map(|card| card.title.as_str()),
    );
    optional(
        &mut values,
        "runtime.card.body",
        input.card.as_ref().map(|card| card.body.as_str()),
    );
    optional(&mut values, "runtime.card.message", message);
    optional(
        &mut values,
        "runtime.card.until_tick",
        active_card.map(|row| row.until_tick),
    );
    optional(
        &mut values,
        "runtime.card.remaining_ticks",
        active_card.map(|row| row.until_tick.saturating_sub(input.tick)),
    );
    put(
        &mut values,
        "runtime.sound",
        "occurrence-only-no-retained-state",
    );
    PresentationInspection {
        label: input.label.into(),
        kind: "ship".into(),
        ship_id: Some(input.ship_id.into()),
        action_available: true,
        values,
    }
}

pub fn sound_reading(
    version: u32,
    definition: &crate::sound_cues::SoundDefinition,
    asset: Option<&crate::sound_cues::Asset>,
) -> PresentationInspection {
    let mut values = BTreeMap::new();
    put(&mut values, "identity.kind", "sound-cue-definition");
    put(&mut values, "identity.definition", &definition.id);
    put(&mut values, "provenance.document", crate::sound_cues::PATH);
    optional(&mut values, "provenance.layer", None::<&str>);
    put(&mut values, "catalog.version", version);
    optional(
        &mut values,
        "asset.file",
        asset.map(|row| row.file.as_str()),
    );
    optional(
        &mut values,
        "asset.category",
        asset.map(|row| row.category.as_str()),
    );
    optional(
        &mut values,
        "asset.informative",
        asset.map(|row| row.informative),
    );
    put(&mut values, "sound.id", &definition.id);
    put(&mut values, "sound.label", &definition.label);
    put(&mut values, "sound.file", &definition.file);
    put(&mut values, "sound.category", &definition.category);
    put(&mut values, "sound.audience", &definition.audience);
    put(&mut values, "sound.volume", definition.volume);
    // A synthetic link value reaches the existing typed Presentation panel;
    // `sound.id` above remains the authored recreate-required definition.
    put(&mut values, "cue.sound.id", &definition.id);
    put(
        &mut values,
        "sound.equivalent.present",
        definition.equivalent.is_some(),
    );
    let equivalent = definition.equivalent.as_ref();
    optional(
        &mut values,
        "sound.equivalent.meaning",
        equivalent.map(|row| row.meaning.as_str()),
    );
    optional(
        &mut values,
        "sound.equivalent.source",
        equivalent.map(|row| row.source.as_str()),
    );
    optional(
        &mut values,
        "sound.equivalent.urgency",
        equivalent.map(|row| row.urgency.as_str()),
    );
    optional(
        &mut values,
        "sound.equivalent.bearing",
        equivalent.and_then(|row| row.bearing),
    );
    optional(
        &mut values,
        "sound.equivalent.elevation",
        equivalent.and_then(|row| row.elevation),
    );
    PresentationInspection {
        label: definition.label.clone(),
        kind: "sound".into(),
        ship_id: None,
        action_available: definition.audience == "viewscreen",
        values,
    }
}

pub fn asset_reading(version: u32, asset: &crate::sound_cues::Asset) -> PresentationInspection {
    let mut values = BTreeMap::new();
    put(&mut values, "identity.kind", "sound-asset-definition");
    put(&mut values, "identity.definition", &asset.file);
    put(&mut values, "provenance.document", crate::sound_cues::PATH);
    optional(&mut values, "provenance.layer", None::<&str>);
    put(&mut values, "catalog.version", version);
    put(&mut values, "asset.file", &asset.file);
    put(&mut values, "asset.category", &asset.category);
    put(&mut values, "asset.informative", asset.informative);
    PresentationInspection {
        label: asset.file.clone(),
        kind: "asset".into(),
        ship_id: None,
        action_available: false,
        values,
    }
}

pub fn catalog_reading(version: u32) -> PresentationInspection {
    let mut values = BTreeMap::new();
    put(&mut values, "identity.kind", "sound-cue-catalog");
    put(&mut values, "identity.definition", crate::sound_cues::PATH);
    put(&mut values, "provenance.document", crate::sound_cues::PATH);
    optional(&mut values, "provenance.layer", None::<&str>);
    put(&mut values, "catalog.version", version);
    PresentationInspection {
        label: crate::sound_cues::PATH.into(),
        kind: "catalog".into(),
        ship_id: None,
        action_available: false,
        values,
    }
}

/// Pure catalogue projection. Assets are enumerated independently from cues so
/// a valid unused asset and a valid zero-cue catalogue remain inspectable.
pub fn catalog_readings(
    catalog: &crate::sound_cues::Catalog,
) -> BTreeMap<String, PresentationInspection> {
    let mut readings = BTreeMap::from([(
        "catalog:sound-cues".into(),
        catalog_reading(catalog.version),
    )]);
    for asset in &catalog.assets {
        readings.insert(
            format!("asset:{}", asset.file),
            asset_reading(catalog.version, asset),
        );
    }
    for definition in &catalog.cues {
        let asset = catalog
            .assets
            .iter()
            .find(|asset| asset.file == definition.file);
        readings.insert(
            format!("sound:{}", definition.id),
            sound_reading(catalog.version, definition, asset),
        );
    }
    readings
}

#[cfg(test)]
mod tests {
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
            .all(|field| field.descriptor.validation
                == ["inspector.presentation.recreate_explanation"]));
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
}
