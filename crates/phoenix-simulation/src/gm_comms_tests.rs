use super::*;

fn transmission(text: &str) -> GmCommsTransmission {
    GmCommsTransmission {
        sender: "existing-speaker".into(),
        route: "private".into(),
        recipients: vec![ShipKey("ship-a".into()), ShipKey("ship-b".into())],
        content: GmCommsContent::Literal { text: text.into() },
    }
}

#[test]
fn gm_comms_codec_preserves_literal_text_and_bounds_the_complete_request() {
    let intent = transmission("  server.gm.comms.heading\n🌒 {crew}<b>\t");
    let wire = serde_json::json!({ "operator_id": "gm-a", "correlation": "literal-1", "action": "transmit_comms", "transmission": intent });
    let request = crate::core::codec::decode_gm_action_request(&wire.to_string()).unwrap();
    assert_eq!(
        request.action,
        GmAction::TransmitComms {
            transmission: intent
        }
    );
    // The existing codec exposes the same postcard serialization used by
    // authoritative digests without adding a second direct dependency.
    let codec = vellum_digest::ShareCodec::new("GM-COMMS-TEST-");
    let binary = codec.encode(&request.action).unwrap();
    assert_eq!(codec.decode::<GmAction>(&binary).unwrap(), request.action);
    for text in [String::new(), "🌒".repeat(MAX_TEXT_BYTES / 4 + 1)] {
        let mut invalid = wire.clone();
        invalid["transmission"]["content"]["literal"]["text"] = serde_json::Value::String(text);
        assert!(crate::core::codec::decode_gm_action_request(&invalid.to_string()).is_none());
    }
    let mut limit = wire.clone();
    limit["transmission"]["content"]["literal"]["text"] =
        serde_json::Value::String("🌒".repeat(MAX_TEXT_BYTES / 4));
    assert!(crate::core::codec::decode_gm_action_request(&limit.to_string()).is_some());
    let mut extra = wire.clone();
    extra["transmission"]["invented_identity"] = true.into();
    assert!(crate::core::codec::decode_gm_action_request(&extra.to_string()).is_none());
    let mut duplicate = wire.clone();
    duplicate["transmission"]["recipients"] = serde_json::json!(["ship-a", "ship-a"]);
    assert!(crate::core::codec::decode_gm_action_request(&duplicate.to_string()).is_none());
    let mut unsorted = wire;
    unsorted["transmission"]["recipients"] = serde_json::json!(["ship-b", "ship-a"]);
    assert!(crate::core::codec::decode_gm_action_request(&unsorted.to_string()).is_none());
}

#[test]
fn old_comms_catalogue_fields_decode_before_snapshot_format_refusal() {
    let message = crate::core::messages::CommsMessage::injected(
        "m".into(),
        "s".into(),
        "n".into(),
        "body".into(),
        Default::default(),
        Vec::new(),
        "thread".into(),
        true,
        crate::core::messages::CommsPriority::Routine,
    );
    let mut value = serde_json::to_value(&message).unwrap();
    value.as_object_mut().unwrap().remove("recipient_ship");
    value.as_object_mut().unwrap().remove("literal_body");
    let decoded: crate::core::messages::CommsMessage = serde_json::from_value(value).unwrap();
    assert_eq!(decoded, message);
    let open = crate::comms::content::OpenCommsRequest::default();
    let mut value = serde_json::to_value(&open).unwrap();
    value.as_object_mut().unwrap().remove("recipient_ship");
    value.as_object_mut().unwrap().remove("sender_uuid");
    assert_eq!(
        serde_json::from_value::<crate::comms::content::OpenCommsRequest>(value).unwrap(),
        open
    );
    let dialogue: crate::comms::content::ScriptedDialogue = serde_json::from_value(serde_json::json!({ "script_path":"p", "origin_layer":null, "node_fn":"root", "on_pick":[] })).unwrap();
    assert_eq!(dialogue.recipient_ship, None);
}

#[test]
fn recipient_and_literal_mode_are_authoritative_digest_inputs() {
    let mut message = crate::core::messages::CommsMessage::injected(
        "m".into(),
        "s".into(),
        "n".into(),
        "known.id".into(),
        Default::default(),
        Vec::new(),
        "thread".into(),
        true,
        crate::core::messages::CommsPriority::Routine,
    );
    let digest = |message| {
        let mut world = World::new();
        let mut inbox = CommsInboxRes::default();
        inbox.0.inject(message);
        world.insert_resource(inbox);
        crate::sim_digest::world_digest(&world)
    };
    let global = digest(message.clone());
    message.recipient_ship = Some(ShipKey("ship-a".into()));
    let private = digest(message.clone());
    assert_ne!(global, private);
    message.literal_body = true;
    assert_ne!(private, digest(message));
}

#[test]
fn authored_routes_refuse_duplicates_and_unknown_visibility() {
    let route = GmCommsRoute {
        id: "private".into(),
        label: "label".into(),
        visibility: GmCommsVisibility::SelectedShips,
        senders: vec!["existing".into()],
        hails: Vec::new(),
        attention_band: None,
    };
    assert!(validate_routes(std::slice::from_ref(&route)).is_ok());
    assert!(validate_routes(&[route.clone(), route.clone()]).is_err());
    for band in ["urgent", "attention", "background"] {
        let mut banded = route.clone();
        banded.attention_band = Some(band.into());
        assert!(validate_routes(std::slice::from_ref(&banded)).is_ok());
    }
    for band in ["Urgent", "critical", ""] {
        let mut banded = route.clone();
        banded.attention_band = Some(band.into());
        let error = validate_routes(std::slice::from_ref(&banded)).unwrap_err();
        assert!(
            error.contains("[[gm_comms_route]] 'private'") && error.contains("attention_band"),
            "{error}"
        );
    }
    assert!(serde_json::from_value::<GmCommsRoute>(
        serde_json::json!({"id":"a", "label":"a", "visibility":"invented", "senders":["a"]})
    )
    .is_err());
}
