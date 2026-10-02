const REQUEST: &str = include_str!("../../tests/fixtures/npc-live-request.json");

#[test]
fn live_inspector_codec_accepts_only_the_named_bounded_checked_action() {
    let request = crate::core::codec::decode_gm_action_request(REQUEST).unwrap();
    assert!(
        matches!(request.action, crate::gm_action::GmAction::SetNpcDoctrineChecked { target, doctrine, expected_revision }
            if target == "courier" && doctrine == "north" && expected_revision == "0123456789abcdef")
    );
    for invalid in [
        REQUEST.replace("0123456789abcdef", "NaN"),
        REQUEST.replace("0123456789abcdef", "0123456789ABCDEF"),
        REQUEST.replace("\"north\"", "\"\""),
        REQUEST.replace("\"target\":", "\"raw_ecs\":{},\"target\":"),
        REQUEST.replace("\"expected_revision\":\"0123456789abcdef\",", ""),
    ] {
        assert!(
            crate::core::codec::decode_gm_action_request(&invalid).is_none(),
            "{invalid}"
        );
    }
}

#[cfg(all(feature = "host", not(target_arch = "wasm32")))]
#[test]
fn live_inspector_native_bridge_delivers_the_same_checked_request_only_from_its_active_pane() {
    use crate::native_host::{
        native_gm::{bridge::NativeGmBridge, NativeGmRecord},
        panes::{registry::PaneId, surface::RecordingSurface},
    };
    let bridge = NativeGmBridge::default();
    bridge.activate(PaneId(41));
    let mut page = RecordingSurface::ready();
    let record = serde_json::json!({"kind":"action", "request":REQUEST}).to_string();
    page.queue_record(record.clone());
    bridge.pump(PaneId(41), &mut page);
    let records = bridge.take_records();
    assert_eq!(records.len(), 1);
    let NativeGmRecord::Action { request } =
        crate::core::codec::decode_native_gm_record(&records[0]).unwrap()
    else {
        panic!("checked action record")
    };
    assert!(crate::core::codec::decode_gm_action_request(&request).is_some());
    bridge.activate(PaneId(42));
    page.queue_record(record);
    bridge.pump(PaneId(41), &mut page);
    assert!(bridge.take_records().is_empty());
}
