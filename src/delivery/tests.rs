use super::*;
use crate::core::messages::PROTOCOL_VERSION;

fn request(head: &str) -> http::Request {
    http::parse_request(head).expect("well-formed head")
}

#[test]
fn a_stamp_header_is_read_as_protocol_content_epoch() {
    let req =
        request("GET /host/manifest.json HTTP/1.1\r\nx-phoenix-client-stamp: 1/phoenix-base/2\r\n");
    assert_eq!(
        client_stamp_from_request(&req),
        Some(DeliveryStamp {
            protocol: 1,
            content_id: "phoenix-base".into(),
            content_epoch: 2,
        })
    );
}

#[test]
fn query_parameters_are_the_other_accepted_form() {
    let req = request(
        "GET /host/manifest.json?protocol=1&content_id=phoenix-base&content_epoch=2 HTTP/1.1\r\n",
    );
    assert_eq!(
        client_stamp_from_request(&req),
        Some(DeliveryStamp {
            protocol: 1,
            content_id: "phoenix-base".into(),
            content_epoch: 2,
        })
    );
}

#[test]
fn a_header_wins_over_a_query_string_that_disagrees_with_it() {
    let req = request(
        "GET /host/manifest.json?protocol=9&content_id=other&content_epoch=9 HTTP/1.1\r\n\
             x-phoenix-client-stamp: 1/phoenix-base/2\r\n",
    );
    let stamp = client_stamp_from_request(&req).unwrap();
    assert_eq!(stamp.content_id, "phoenix-base");
}

#[test]
fn a_malformed_stamp_header_reads_as_unstamped_rather_than_falling_back() {
    let req = request(
        "GET /host/manifest.json?protocol=1&content_id=phoenix-base&content_epoch=2 HTTP/1.1\r\n\
             x-phoenix-client-stamp: 1/phoenix-base\r\n",
    );
    assert_eq!(client_stamp_from_request(&req), None);
}

#[test]
fn a_request_carrying_neither_form_is_unstamped() {
    let req = request("GET /host/manifest.json HTTP/1.1\r\n");
    assert_eq!(client_stamp_from_request(&req), None);
    assert_eq!(
        stamp::check_client_stamp(
            &DeliveryStamp {
                protocol: PROTOCOL_VERSION,
                content_id: "phoenix-base".into(),
                content_epoch: 1,
            },
            None,
        )
        .unwrap_err()
        .code(),
        "client-stamp-missing"
    );
}

// ── The browser host's in-band join handshake (issue #1111) ─────────────

fn browser_host() -> DeliveryStamp {
    DeliveryStamp {
        protocol: PROTOCOL_VERSION,
        content_id: "phoenix-base".into(),
        content_epoch: 1,
    }
}

#[test]
fn a_join_stamp_matching_the_host_is_admitted() {
    let field = format!("{PROTOCOL_VERSION}/phoenix-base/1");
    assert!(check_join_stamp(&browser_host(), Some(&field)).is_ok());
}

#[test]
fn a_join_from_another_protocol_is_refused_by_the_host_not_the_rendezvous() {
    let field = format!("{}/phoenix-base/1", PROTOCOL_VERSION + 1);
    let err = check_join_stamp(&browser_host(), Some(&field)).unwrap_err();
    assert_eq!(err.code(), "protocol-mismatch");
}

#[test]
fn a_join_from_another_content_set_is_refused() {
    let field = format!("{PROTOCOL_VERSION}/other-game/1");
    assert_eq!(
        check_join_stamp(&browser_host(), Some(&field))
            .unwrap_err()
            .code(),
        "content-id-mismatch"
    );
}

#[test]
fn an_unstamped_join_is_refused_now_that_every_client_is_a_phoenix_one() {
    // The #1112 flip. Until PeerJS was retired an unstamped joiner was the
    // shipped client, so admitting it was the only option; now the only way
    // to arrive unstamped is to not be a built Phoenix bundle.
    for absent in [None, Some(""), Some("  ")] {
        assert_eq!(
            check_join_stamp(&browser_host(), absent)
                .unwrap_err()
                .code(),
            "client-stamp-missing",
            "{absent:?}"
        );
    }
}

#[test]
fn a_garbled_join_stamp_is_refused_under_the_same_code_as_an_absent_one() {
    for garbled in ["1/phoenix-base", "1/phoenix-base/1/extra", "nonsense"] {
        assert_eq!(
            check_join_stamp(&browser_host(), Some(garbled))
                .unwrap_err()
                .code(),
            "client-stamp-missing",
            "{garbled}"
        );
    }
}

#[test]
fn a_host_that_has_not_loaded_a_manifest_still_binds_the_protocol_half() {
    let lobby = DeliveryStamp::for_manifest("");
    assert!(lobby.content_id.is_empty());
    // Content cannot be compared yet, so a real client is admitted…
    let ok = format!("{PROTOCOL_VERSION}/phoenix-base/1");
    assert!(check_join_stamp(&lobby, Some(&ok)).is_ok());
    // …but a protocol difference is still fatal.
    let bad = format!("{}/phoenix-base/1", PROTOCOL_VERSION + 1);
    assert_eq!(
        check_join_stamp(&lobby, Some(&bad)).unwrap_err().code(),
        "protocol-mismatch"
    );
}

// ── The fleet's host-to-host handshake (issue #1114) ────────────────────

#[test]
fn a_matching_ship_host_is_admitted_to_the_fleet() {
    let field = format!("{PROTOCOL_VERSION}/phoenix-base/1");
    assert!(check_host_stamp(&browser_host(), Some(&field)).is_ok());
}

#[test]
fn a_ship_host_on_another_protocol_or_content_set_is_refused() {
    for (field, code) in [
        (
            format!("{}/phoenix-base/1", PROTOCOL_VERSION + 1),
            "protocol-mismatch",
        ),
        (
            format!("{PROTOCOL_VERSION}/other-game/1"),
            "content-id-mismatch",
        ),
        (
            format!("{PROTOCOL_VERSION}/phoenix-base/2"),
            "content-epoch-mismatch",
        ),
    ] {
        assert_eq!(
            check_host_stamp(&browser_host(), Some(&field))
                .unwrap_err()
                .code(),
            code,
            "{field}"
        );
    }
}

#[test]
fn an_unstamped_or_garbled_ship_host_is_refused_like_an_unstamped_phone() {
    for absent in [
        None,
        Some(""),
        Some("  "),
        Some("1/phoenix-base"),
        Some("junk"),
    ] {
        assert_eq!(
            check_host_stamp(&browser_host(), absent)
                .unwrap_err()
                .code(),
            "client-stamp-missing",
            "{absent:?}"
        );
    }
}

#[test]
fn a_fleet_admits_nobody_until_it_knows_what_content_it_is_running() {
    // The one behavioural difference from the crew handshake, and the whole
    // reason this function exists rather than a call to check_join_stamp: a
    // host with no manifest loaded waives the CONTENT half for a phone, and
    // must not waive it for another authoritative simulation.
    let undecided = DeliveryStamp::for_manifest("");
    let ok = format!("{PROTOCOL_VERSION}/phoenix-base/1");
    assert!(check_join_stamp(&undecided, Some(&ok)).is_ok());
    assert_eq!(
        check_host_stamp(&undecided, Some(&ok)).unwrap_err().code(),
        "content-id-mismatch"
    );
    // The other direction: a host that knows what it is running refuses a
    // ship host that does not.
    let unstated = format!("{PROTOCOL_VERSION}//0");
    assert_eq!(
        check_host_stamp(&browser_host(), Some(&unstated))
            .unwrap_err()
            .code(),
        "content-id-mismatch"
    );
    // And the case an equality test admits: NEITHER of them has decided.
    // "" is an absent identity, not a value two hosts can agree on — this
    // is the pair that would otherwise fly one mission over two different
    // content sets and desynchronise silently, later.
    assert_eq!(
        check_host_stamp(&undecided, Some(&unstated))
            .unwrap_err()
            .code(),
        "content-id-mismatch"
    );
    // A protocol difference still outranks it, so the operator is sent
    // after the thing that makes every other field's meaning uncertain.
    let other_protocol = format!("{}//0", PROTOCOL_VERSION + 1);
    assert_eq!(
        check_host_stamp(&undecided, Some(&other_protocol))
            .unwrap_err()
            .code(),
        "protocol-mismatch"
    );
}

#[test]
fn the_join_handshake_parses_the_same_field_the_http_header_carries() {
    let field = "1/phoenix-base/2";
    assert_eq!(
        parse_stamp_field(field),
        client_stamp_from_request(&request(&format!(
            "GET /host/manifest.json HTTP/1.1\r\nx-phoenix-client-stamp: {field}\r\n"
        )))
    );
}
