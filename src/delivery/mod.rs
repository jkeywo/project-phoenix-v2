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
        let mut parts = raw.split('/');
        let (protocol, content_id, epoch) = (parts.next(), parts.next(), parts.next());
        // A header with a fourth field is malformed, not merely long: content
        // ids never contain `/`, so an extra field means the sender is speaking
        // some other format and its first three fields cannot be trusted.
        if parts.next().is_none() {
            return DeliveryStamp::from_params(protocol, content_id, epoch);
        }
        return None;
    }
    DeliveryStamp::from_params(
        req.query_param("protocol"),
        req.query_param("content_id"),
        req.query_param("content_epoch"),
    )
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
}
