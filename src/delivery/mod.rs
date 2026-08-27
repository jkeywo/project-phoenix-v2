//! Delivery — how a Phoenix host hands a browser its client, its catalogue and
//! its version pin (PRD #855).
//!
//! Phoenix has always had exactly one host role and two places to run it: the
//! browser tab that owns `server.html`, and — since this module — a native PC
//! binary (`phoenix-host`). PRD #855's first implementation decision is that
//! the two "consume the same content manifest, snapshots, and protocol
//! contracts", so this module is built to make forking them awkward:
//!
//! * [`payload`] holds the ONE list of catalogue field names, walked by the
//!   wasm bridge's `Reflect::set` loop and by the JSON encoder alike.
//! * [`stamp`] holds the version pin, built from numbers that already existed
//!   (`messages::PROTOCOL_VERSION`, the manifest's `[content]` identity).
//! * The catalogue itself is `world::manifest`'s, unchanged — the native host
//!   restricts its public catalogue by choosing which manifest FILE to serve,
//!   exactly as `?manifest=assets/scenarios.demo.toml` does in the browser
//!   (issue #917).
//!
//! [`http`] is the pure transport contract (paths, MIME, caching); [`serve`] is
//! the only part that touches a socket and is native-only.

pub mod args;
pub mod http;
pub mod payload;
#[cfg(not(target_arch = "wasm32"))]
pub mod serve;
pub mod stamp;

use payload::ScenarioPayload;
use stamp::{DeliveryStamp, StampMismatch};

/// The document a host publishes at [`http::MANIFEST_PATH`]: who the host is,
/// which manifest file it is serving, and the catalogue that manifest produced.
///
/// This is the whole "content manifest" contract of PRD #855 — a native host
/// and the browser host build it from the same `world::manifest` catalogue and
/// encode it with the same `core::codec::encode_delivery_manifest`.
#[derive(Clone, Debug, PartialEq)]
pub struct DeliveryManifest {
    pub stamp: DeliveryStamp,
    /// The manifest file this host was started with, e.g.
    /// `assets/scenarios.toml` or `assets/scenarios.demo.toml`. Published so an
    /// operator can see which catalogue is live without diffing its contents.
    pub manifest_path: String,
    pub scenarios: Vec<ScenarioPayload>,
}

/// The document a host publishes instead, when the caller's stamp does not
/// match: the machine-readable reason, the prose, and the host's own stamp so
/// the caller can see what it should have been.
#[derive(Clone, Debug, PartialEq)]
pub struct DeliveryRefusal {
    pub mismatch: StampMismatch,
    pub host: DeliveryStamp,
}

/// Read a client's stamp off a parsed request, from either accepted form.
///
/// The header wins when both are present: a query string can be rewritten by a
/// cache or a redirect, and the header is what a real client sends.
pub fn client_stamp_from_request(req: &http::Request) -> Option<DeliveryStamp> {
    if let Some(raw) = req.header(http::CLIENT_STAMP_HEADER) {
        return parse_stamp_field(raw);
    }
    DeliveryStamp::from_params(
        req.query_param("protocol"),
        req.query_param("content_id"),
        req.query_param("content_epoch"),
    )
}

/// Parse the `<protocol>/<content_id>/<content_epoch>` field.
///
/// One spelling, two carriers: the `x-phoenix-client-stamp` header a native
/// host reads off a request, and the browser host's in-band join handshake
/// (issue #1111). A field with a fourth part is malformed, not merely long:
/// content ids never contain `/`, so an extra part means the sender is speaking
/// some other format and its first three parts cannot be trusted.
pub fn parse_stamp_field(raw: &str) -> Option<DeliveryStamp> {
    let mut parts = raw.split('/');
    let (protocol, content_id, epoch) = (parts.next(), parts.next(), parts.next());
    if parts.next().is_some() {
        return None;
    }
    DeliveryStamp::from_params(protocol, content_id, epoch)
}

/// The browser host's authoritative join check (issue #1111).
///
/// The rendezvous service can only give *advice* about version compatibility —
/// its version GUID is a coarse release marker carried in a printed join code.
/// This is the check that binds, and it is deliberately the same
/// [`stamp::check_client_stamp`] the native host runs at `/host/manifest.json`,
/// so a Phoenix host has one version pin rather than two.
///
/// **Every joiner must present a stamp** (issue #1112). #1111 admitted an absent
/// one, because PeerJS was still the default route and had never stamped
/// anything, so refusing the unstamped would have locked out the shipped join
/// path. #1112 retired PeerJS: every client that can reach this host is a
/// Phoenix client built by `scripts/build-client.mjs`, which writes the field
/// into `<meta name="phoenix-client-stamp">` on every build. Nothing legitimate
/// arrives unstamped any more, so an absent stamp is refused with the same
/// `ClientStampMissing` a garbled one gets — neither is evidence of
/// compatibility, and admitting the silent case would leave the only
/// version-skew hole exactly where a stale cached bundle sits.
///
/// One departure from the native host's rules remains, because a browser host
/// is a live process rather than a served bundle: **a host with no content
/// identity checks the protocol half only.** The native rule ("an unidentified
/// content set matches nothing") protects a host serving a bundle it cannot
/// name. A browser host sitting in the lobby has simply not loaded a manifest
/// yet, and refusing every phone until it does would be a race, not a safety
/// property.
pub fn check_join_stamp(
    host: &DeliveryStamp,
    client_field: Option<&str>,
) -> Result<(), StampMismatch> {
    let Some(raw) = client_field.map(str::trim).filter(|s| !s.is_empty()) else {
        return Err(StampMismatch::ClientStampMissing);
    };
    let Some(client) = parse_stamp_field(raw) else {
        return Err(StampMismatch::ClientStampMissing);
    };
    if host.content_id.trim().is_empty() {
        if host.protocol != client.protocol {
            return Err(StampMismatch::Protocol {
                host: host.protocol,
                client: client.protocol,
            });
        }
        return Ok(());
    }
    stamp::check_client_stamp(host, Some(&client))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::messages::PROTOCOL_VERSION;

    fn request(head: &str) -> http::Request {
        http::parse_request(head).expect("well-formed head")
    }

    #[test]
    fn a_stamp_header_is_read_as_protocol_content_epoch() {
        let req = request(
            "GET /host/manifest.json HTTP/1.1\r\nx-phoenix-client-stamp: 1/phoenix-base/2\r\n",
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
}
