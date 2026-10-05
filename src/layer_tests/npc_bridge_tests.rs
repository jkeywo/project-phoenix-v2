const REQUEST: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/npc-live-request.json"
));
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
