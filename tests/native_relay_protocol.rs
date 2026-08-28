//! The Rust and JavaScript ends of the rendezvous vocabulary, pinned together
//! (issue #1113).
//!
//! Three implementations now speak this protocol: the service
//! (`worker-rendezvous/src/`), the browser (`gui/rendezvous-transport.js`), and
//! a native host (`src/native_host/relay_transport.rs`). The first two share
//! modules and cannot drift. The third is a different language with no build
//! step between them, so the only thing that can catch a divergence is a test
//! that reads both — which is this file.
//!
//! It is deliberately a TEXT check rather than a runtime one. There is no way
//! to run the JS modules from `cargo test` without adding a JS runtime to the
//! Rust test harness, and the failures worth catching are all textual anyway: a
//! bumped protocol revision, a renamed verb, a renamed field. Each assertion
//! below therefore reads the shipped `gui/` or `worker-rendezvous/` source and
//! asserts the Rust constant appears in it.
//!
//! What it cannot catch: a semantic change that keeps every name. That is what
//! `tests/native_relay_live.rs` is for, and it needs a real service.

use std::path::{Path, PathBuf};

use project_phoenix::core::rendezvous::{
    CLASS_RELIABLE, CLASS_SNAPSHOT, JOIN_ACCEPTED, JOIN_HANDSHAKE, JOIN_REFUSED,
    RENDEZVOUS_PROTOCOL, TRANSPORT_WEBRTC, TRANSPORT_WS_RELAY,
};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf()
}

fn read(rel: &str) -> String {
    let path = repo_root().join(rel);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
}

#[test]
fn the_protocol_revision_matches_the_one_module_that_declares_it() {
    // Both ends hard-refuse a frame carrying another `v`, so these two numbers
    // moving apart is a total join outage rather than a degradation — and the
    // JS side has a single-source module precisely because two copies of the
    // literal held together by a comment were one careless edit away from that.
    // Rust is the third copy, and this is its comment made executable.
    let js = read("gui/rendezvous-protocol.js");
    let expected = format!("export const RENDEZVOUS_PROTOCOL = {RENDEZVOUS_PROTOCOL};");
    assert!(
        js.contains(&expected),
        "gui/rendezvous-protocol.js does not declare revision {RENDEZVOUS_PROTOCOL}; \
         src/core/rendezvous.rs must be bumped with it"
    );
}

#[test]
fn every_verb_a_native_host_sends_is_one_the_service_handles() {
    // The registry's `switch` is the service's whole surface: a verb it has no
    // `case` for is answered `malformed`, which from a native host reads as an
    // unexplained refusal to register.
    let registry = read("worker-rendezvous/src/registry.js");
    for verb in [
        "host-open",
        "relay",
        "relay-open",
        "relay-close",
        "host-close",
    ] {
        assert!(
            registry.contains(&format!("case '{verb}':")),
            "worker-rendezvous/src/registry.js has no case for {verb:?}"
        );
    }
}

#[test]
fn every_frame_a_native_host_acts_on_is_one_the_service_sends() {
    let registry = read("worker-rendezvous/src/registry.js");
    for kind in [
        "ready",
        "hosted",
        "relay-peer",
        "relay-peer-left",
        "relay-closed",
        "error",
    ] {
        assert!(
            registry.contains(&format!("type: '{kind}'")),
            "worker-rendezvous/src/registry.js never sends a {kind:?} frame, but \
             src/native_host/relay_transport.rs handles one"
        );
    }
}

#[test]
fn the_transport_names_are_the_ones_the_service_will_relay() {
    // A host's claim is sanitised against this list; a name the service does
    // not know is DROPPED rather than relayed, so a native host claiming an
    // unrecognised one would silently be advertised as capable of everything.
    let registry = read("worker-rendezvous/src/registry.js");
    assert!(registry.contains(&format!(
        "const TRANSPORTS = ['{TRANSPORT_WEBRTC}', '{TRANSPORT_WS_RELAY}'];"
    )));
}

#[test]
fn the_delivery_classes_are_spelled_the_same_in_both_languages() {
    // The service refuses a class it cannot read rather than guessing one, so a
    // spelling difference here is every snapshot frame refused.
    let relay = read("worker-rendezvous/src/relay.js");
    assert!(relay.contains(&format!(
        "export const RELAY_RELIABLE = '{CLASS_RELIABLE}';"
    )));
    assert!(relay.contains(&format!(
        "export const RELAY_SNAPSHOT = '{CLASS_SNAPSHOT}';"
    )));
}

#[test]
fn the_in_band_handshake_frames_are_spelled_the_same_in_both_languages() {
    // These three are transport-plane and never reach the crew protocol, which
    // is exactly why nothing else pins them: they are not `ClientMessage`
    // variants and no codec round-trip covers them.
    let joiner = read("gui/rendezvous-transport.js");
    assert!(joiner.contains(&format!("type: '{JOIN_HANDSHAKE}'")));
    assert!(joiner.contains(&format!("msg.type === '{JOIN_ACCEPTED}'")));
    assert!(joiner.contains(&format!("msg.type === '{JOIN_REFUSED}'")));
}

#[test]
fn the_relay_limit_fields_are_named_the_same_in_both_languages() {
    // A native host takes the service's authored numbers rather than carrying
    // its own copy, so a renamed field means it silently falls back to defaults
    // that may be stricter or laxer than the deployment's.
    let registry = read("worker-rendezvous/src/registry.js");
    for field in ["max_frame_bytes", "max_send_buffer_bytes"] {
        assert!(
            registry.contains(&format!("{field}:")),
            "the service no longer advertises {field:?}"
        );
    }
}

/// The one case here that needs the socket, and so the `host` feature.
///
/// Everything else in this file reads text and runs in the ordinary `cargo
/// test` CI job, which is where a drifting vocabulary has to be caught. This
/// one calls into `relay_socket`, which is the only part of the native crew
/// path behind the feature — so it is gated rather than dragging the whole file
/// out of the default test run with it.
#[cfg(feature = "host")]
#[test]
fn the_service_endpoint_a_native_host_dials_is_the_one_the_worker_serves() {
    let index = read("worker-rendezvous/src/index.js");
    assert!(index.contains("'/v1/host'"));
    let url = project_phoenix::native_host::relay_socket::host_socket_url("https://example.test")
        .expect("a usable base");
    assert!(
        url.ends_with("/v1/host"),
        "a native host dials {url}, which is not the worker's host endpoint"
    );
}
