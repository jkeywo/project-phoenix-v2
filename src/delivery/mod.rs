//! Delivery — how a Phoenix host hands a browser its client, its catalogue and
//! its version pin (PRD #855).
//!
//! Phoenix has always had exactly one host role and two places to run it: the
//! browser tab that owns `server.html`, and — since this module — a native PC
//! binary (`phoenix-host`). PRD #855's first implementation decision is that
//! the two "consume the same content manifest, snapshots, and protocol
//! contracts", so this module is built to make forking them awkward:
//!
//! * [`payload`] projects shared serde wire types for delivery, both
//!   pickers and every crew catalogue message.
//! * [`stamp`] holds the version pin, built from numbers that already existed
//!   (`messages::PROTOCOL_VERSION`, the manifest's `[content]` identity).
//! * The catalogue itself is `world::manifest`'s, unchanged — the native host
//!   restricts its public catalogue by choosing which manifest FILE to serve,
//!   exactly as `?manifest=assets/scenarios.demo.toml` does in the browser
//!   (issue #917).
//!
//! [`http`] is the pure transport contract (paths, MIME, caching); [`serve`] is
//! the only part that touches a socket and is native-only.

#[cfg(not(target_arch = "wasm32"))]
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

/// The fleet's authoritative host-to-host check (issue #1114).
///
/// Same three numbers, same [`stamp::check_client_stamp`] comparison, one
/// deliberate difference: **there is no grace here at all.** Protocol AND
/// content must match, including the content-identity half that
/// [`check_join_stamp`] waives for a host that has not loaded a manifest yet.
///
/// The waiver is right for a phone and wrong for a ship. A phone that connects
/// to a host still sitting in its lobby is joining something that has not
/// chosen its content yet, and refusing it would be a race rather than a safety
/// property — the phone will be sent whatever the host later loads. Two HOSTS
/// are not in that relationship. Each runs its own authoritative simulation
/// over its own content, and #1116 will make them advance one shared tick
/// stream; two hosts that agree on the protocol and differ on the content set
/// do not desynchronise loudly, they desynchronise silently and later. "I have
/// not decided what I am running yet" is therefore not a reason to admit a
/// fleet member — it is the strongest possible reason not to.
///
/// "Undecided" is refused on BOTH sides, and that is not the same as comparing
/// them. Two hosts that have not loaded a manifest both carry `content_id ==
/// ""`, and an equality test admits that pair happily — `"" == ""`, epochs
/// `(0, 0)` — which is the exact case the paragraph above says is the strongest
/// possible reason to refuse. An empty identity is an ABSENT one, not a value
/// two parties can agree on, and it is treated here the way
/// [`stamp::check_bundle_content`] already treats a bundle whose manifest
/// declares no `[content]`: as missing, never as matching.
///
/// The reason codes are `StampMismatch`'s own, unchanged, and deliberately so:
/// a fleet refusal reads back through the same `reasonStringId` map on the same
/// wire (`gui/join-code.js`), so "the other ship is on a different build" gets
/// one sentence in this project rather than two. Read `client` in
/// `client-stamp-missing` as "the joining side", which is what it has always
/// meant.
pub fn check_host_stamp(
    host: &DeliveryStamp,
    peer_field: Option<&str>,
) -> Result<(), StampMismatch> {
    let Some(raw) = peer_field.map(str::trim).filter(|s| !s.is_empty()) else {
        return Err(StampMismatch::ClientStampMissing);
    };
    let Some(peer) = parse_stamp_field(raw) else {
        return Err(StampMismatch::ClientStampMissing);
    };
    // Protocol first, then the ordinary comparison — the shared check owns the
    // order, and reporting a content difference over a protocol one would send
    // the operator after the wrong thing.
    stamp::check_client_stamp(host, Some(&peer))?;
    // They agreed. On nothing, if neither has decided what it is running.
    if host.content_id.trim().is_empty() || peer.content_id.trim().is_empty() {
        return Err(StampMismatch::ContentId {
            host: host.content_id.clone(),
            client: peer.content_id.clone(),
        });
    }
    Ok(())
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
