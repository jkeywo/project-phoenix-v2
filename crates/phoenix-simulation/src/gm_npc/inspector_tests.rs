const REQUEST: &str = include_str!("../../../../tests/fixtures/npc-live-request.json");

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
