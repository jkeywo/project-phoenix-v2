use super::*;
use phoenix_model::messages::PROTOCOL_VERSION;

// ── The browser host's join-handshake verdict (issue #1111) ─────────────────

#[test]
fn a_join_verdict_says_ok_and_names_the_host_stamp() {
    let host = crate::stamp::DeliveryStamp {
        protocol: PROTOCOL_VERSION,
        content_id: "phoenix-base".into(),
        content_epoch: 3,
    };
    let json = encode_join_verdict(&Ok(()), &host);
    let value: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(value["ok"], serde_json::Value::Bool(true));
    assert_eq!(value["host"]["content_id"], "phoenix-base");
    assert_eq!(value["host"]["content_epoch"], 3);
}

#[test]
fn a_refused_join_carries_the_same_machine_code_the_native_host_answers_with() {
    let host = crate::stamp::DeliveryStamp {
        protocol: PROTOCOL_VERSION,
        content_id: "phoenix-base".into(),
        content_epoch: 1,
    };
    let bad = format!("{}/phoenix-base/1", PROTOCOL_VERSION + 1);
    let verdict = crate::check_join_stamp(&host, Some(&bad));
    let json = encode_join_verdict(&verdict, &host);
    let value: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(value["ok"], serde_json::Value::Bool(false));
    assert_eq!(value["code"], "protocol-mismatch");
    assert!(value["detail"].as_str().unwrap().contains("protocol"));
}
