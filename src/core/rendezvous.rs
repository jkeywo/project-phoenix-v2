//! The rendezvous service's frame vocabulary, in Rust (issue #1113).
//!
//! `worker-rendezvous/src/registry.js` is the service and
//! `gui/rendezvous-transport.js` is the browser's end of it. This module is the
//! third speaker: a NATIVE host, which reaches the same service over the same
//! secure WebSocket and speaks the same frames — because the alternative was a
//! second protocol for native processes, and two join protocols is exactly what
//! PRD #1093 spent #1111 and #1112 removing.
//!
//! # What is deliberately NOT here
//!
//! No join-code minting, no typed lookup, no normalisation. A host is ISSUED a
//! code; it never parses or composes one, so `gui/join-code.js`'s table has no
//! Rust twin to drift from. Everything this module knows about a code is that
//! it is a string the service handed over and the viewscreen prints.
//!
//! No `serde_json` either: the types carry `serde` derives, and the two
//! `to_string`/`from_str` calls live in [`crate::core::codec`], which is the
//! one module in this crate allowed to name that crate (AGENTS.md rule 1).
//!
//! # Shape: one flat struct, not an enum
//!
//! The wire frames are `{"v":1,"type":"…", …}` with a different field set per
//! type, and every field optional from any one frame's point of view. A Rust
//! enum would model that more tightly and would also make every future
//! additive field — the kind #1114 and #1115 are going to add — a breaking
//! decode for a host built before it. A flat struct with optional fields
//! decodes an unknown frame as "a frame I do not act on", which is the same
//! forward-compatibility posture `registry.js`'s own `default:` arm takes.

use serde::{Deserialize, Serialize};

/// Frame-vocabulary revision. The Rust mirror of `gui/rendezvous-protocol.js`'s
/// `RENDEZVOUS_PROTOCOL`; both ends hard-refuse a frame carrying another value,
/// so these two numbers moving apart is a total join outage rather than a
/// degradation. There is no build step that could single-source them across the
/// two languages, so the pairing is asserted in
/// `tests/native_relay_protocol.rs` instead, which reads the JS module's text.
pub const RENDEZVOUS_PROTOCOL: u32 = 1;

/// Transport names a host may claim it can answer on, in the `host-open` frame.
///
/// A native host is the reason this field exists: it is a Rust process holding
/// a WebSocket and has no WebRTC of any kind, so a joiner that did not know
/// would spend the whole 8/16/30 s ladder four times over before falling back
/// to the only path there ever was.
pub const TRANSPORT_WEBRTC: &str = "webrtc";
pub const TRANSPORT_WS_RELAY: &str = "ws-relay";

/// Delivery class of one relayed game frame. The wire spelling matches
/// `DeliveryClass`'s two arms and the DataChannel labels, because it is the
/// same distinction carried by a different transport.
pub const CLASS_RELIABLE: &str = "reliable";
pub const CLASS_SNAPSHOT: &str = "snapshot";

/// A join code as the service issues it. Opaque to a host: `full` is what a QR
/// encodes and `suffix` is what a guest types, and neither is ever built here.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct JoinCode {
    #[serde(default)]
    pub full: String,
    #[serde(default)]
    pub suffix: String,
    #[serde(default)]
    pub project: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub namespace: String,
}

/// The relay bounds the service advertises on `relay-peer` / `relay-ready`.
///
/// Authored in `assets/join/join-codes.toml`; a host takes the service's
/// numbers rather than carrying its own, so a designer retuning them does not
/// need a native release. The defaults here are only for a service too old to
/// advertise any, and are deliberately conservative — a host that guessed
/// generously and then met a stricter service would be cut off mid-mission
/// rather than shedding a snapshot.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RelayLimits {
    #[serde(default = "default_max_frame_bytes")]
    pub max_frame_bytes: usize,
    #[serde(default = "default_max_send_buffer_bytes")]
    pub max_send_buffer_bytes: usize,
}

fn default_max_frame_bytes() -> usize {
    65536
}

fn default_max_send_buffer_bytes() -> usize {
    262144
}

impl Default for RelayLimits {
    fn default() -> Self {
        Self {
            max_frame_bytes: default_max_frame_bytes(),
            max_send_buffer_bytes: default_max_send_buffer_bytes(),
        }
    }
}

/// The `code` field, which is genuinely two shapes on one name.
///
/// A CLIENT sends `join { code: "PROJECT_VERSION_SUFFIX" }` — a typed string,
/// because that is what a guest read off a viewscreen. The SERVICE answers
/// `hosted { code: { full, suffix, … } }` — the structured identifier it just
/// minted. Modelling that as one optional string plus one optional object under
/// two names would be tidier Rust and a lie about the wire; an untagged enum is
/// exactly the thing the wire does.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum CodeField {
    /// What a guest typed or a QR carried, whole.
    Typed(String),
    /// What the service issued.
    Issued(Box<JoinCode>),
}

impl CodeField {
    /// The structured code, when this is one the service issued.
    pub fn issued(&self) -> Option<&JoinCode> {
        match self {
            CodeField::Issued(code) => Some(code),
            CodeField::Typed(_) => None,
        }
    }
}

/// One frame on the rendezvous socket, in either direction.
///
/// `payload` is the load-bearing field and the load-bearing *absence* of a
/// type: it holds the game's own bytes, opaque, exactly as the DataChannel
/// would have carried them. Nothing in this module may ever grow an opinion
/// about what is inside it.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct RendezvousFrame {
    pub v: u32,
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub namespace: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub transports: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<CodeField>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub peer: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    #[serde(default, rename = "class", skip_serializing_if = "Option::is_none")]
    pub class: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// Which request an `error` frame is refusing — `registry.js`'s `fail()`
    /// stamps it. Named in the operator-facing fault message, but it is
    /// `reason` that is load-bearing for telling a per-REQUEST refusal (one
    /// frame the service would not carry) from a link event — see
    /// `RelayTransport::is_terminal_error`, which switches on `reason` alone
    /// and never looks at this field. The difference between one lost frame
    /// and a whole crew reported gone is decided without it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limits: Option<RelayLimits>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dropped: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub admission: Option<String>,
}

impl RendezvousFrame {
    /// A frame of `kind` carrying this build's protocol revision.
    pub fn new(kind: &str) -> Self {
        Self {
            v: RENDEZVOUS_PROTOCOL,
            kind: kind.to_string(),
            ..Default::default()
        }
    }

    /// The `host-open` a native host sends: this namespace, and the honest
    /// claim that the only transport it can answer on is the relay.
    pub fn host_open(namespace: &str, version: Option<&str>) -> Self {
        Self {
            namespace: Some(namespace.to_string()),
            version: version.map(str::to_string),
            transports: vec![TRANSPORT_WS_RELAY.to_string()],
            ..Self::new("host-open")
        }
    }

    /// One game frame for `peer`, in `class`.
    pub fn relay(peer: &str, class: &str, payload: String) -> Self {
        Self {
            to: Some(peer.to_string()),
            class: Some(class.to_string()),
            payload: Some(payload),
            ..Self::new("relay")
        }
    }

    /// Ask the service to detach `peer` from this record — the same in-band
    /// frame the browser host's `onSever`/`onFailure` send
    /// (`gui/rendezvous-transport.js`). Without it a link this host gives up
    /// on locally is one-sided: the host walks away but the service keeps the
    /// peer's mailbox open and the phone sits on a status line that still
    /// reads "connected".
    pub fn relay_close(peer: &str) -> Self {
        Self {
            to: Some(peer.to_string()),
            ..Self::new("relay-close")
        }
    }
}

/// The in-band compatibility handshake — `JoinHandshake`, `JoinAccepted`,
/// `JoinRefused` — as it travels INSIDE a relayed payload.
///
/// Deliberately not a `ClientMessage`/`ServerMessage` variant, on both sides of
/// the wire and now in both languages: `pasm/spec/design/p2p-design-deltas.yaml`
/// forbids layering transport concerns onto the crew protocol, and these three
/// never reach the simulation at all. The host answers them from
/// `delivery::check_join_stamp`, which is the same authority the browser host
/// asks through `wasm_check_client_stamp`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct HandshakeFrame {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub data: HandshakeData,
}

/// The payload of a [`HandshakeFrame`]. Every field optional, because the three
/// frame kinds use disjoint subsets of them.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct HandshakeData {
    /// The joiner's delivery stamp, on `JoinHandshake`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stamp: Option<String>,
    /// The release the joiner's code named, advisory only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code_version: Option<String>,
    /// `StampMismatch::code()`, on `JoinRefused`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    /// `StampMismatch::detail()` — operator prose, never player-visible text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// Frame kinds of the in-band handshake, spelled once.
pub const JOIN_HANDSHAKE: &str = "JoinHandshake";
pub const JOIN_ACCEPTED: &str = "JoinAccepted";
pub const JOIN_REFUSED: &str = "JoinRefused";

impl HandshakeFrame {
    /// The host's acceptance. Carries nothing: the joiner answers it with
    /// `Identify`, and everything the host knows travels in the crew protocol
    /// after that.
    pub fn accepted() -> Self {
        Self {
            kind: JOIN_ACCEPTED.to_string(),
            data: HandshakeData::default(),
        }
    }

    /// The host's refusal, naming the machine code the phone maps to a
    /// strings.csv id and the operator prose for a log.
    pub fn refused(code: &str, detail: String) -> Self {
        Self {
            kind: JOIN_REFUSED.to_string(),
            data: HandshakeData {
                code: Some(code.to_string()),
                detail: Some(detail),
                ..Default::default()
            },
        }
    }
}
