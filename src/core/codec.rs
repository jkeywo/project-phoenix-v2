use crate::core::messages::{ClientMessage, ServerMessage};

pub trait MessageCodec {
    type Error;
    fn encode_client(&self, msg: &ClientMessage) -> Result<String, Self::Error>;
    fn decode_client(&self, s: &str) -> Result<ClientMessage, Self::Error>;
    fn encode_server(&self, msg: &ServerMessage) -> Result<String, Self::Error>;
    fn decode_server(&self, s: &str) -> Result<ServerMessage, Self::Error>;
}

/// Browser-host local FFI reply. This does not change the game wire protocol.
pub fn encode_connection_binding(
    result: Result<
        Option<crate::session_connections::ConnectionId>,
        crate::session_connections::BindRefusal,
    >,
) -> String {
    use crate::session_connections::BindRefusal;
    match result {
        Ok(previous) => serde_json::json!({
            "ok": true,
            "previous": previous.map(|id| id.incarnation.to_string()),
        }),
        Err(reason) => serde_json::json!({
            "ok": false,
            "code": match reason {
                BindRefusal::ReservedToken => "reserved-token",
                _ => "invalid-token",
            },
        }),
    }
    .to_string()
}

/// Opaque physical handles stay strings across the JavaScript number boundary.
pub fn encode_connection_recipients(ids: &[crate::session_connections::ConnectionId]) -> String {
    let handles: Vec<_> = ids.iter().map(|id| id.incarnation.to_string()).collect();
    serde_json::to_string(&handles).expect("connection handles serialize")
}

/// Shared adapter transcript; decoding remains at the codec boundary in tests too.
#[cfg(test)]
#[derive(serde::Deserialize, Debug)]
pub(crate) struct ConnectionTranscriptStep {
    pub op: String,
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub leg: usize,
    pub message: Option<ClientMessage>,
    pub sender: Option<String>,
    #[serde(default)]
    pub departed: Vec<String>,
    pub refusal: Option<String>,
    #[serde(default)]
    pub target: String,
    #[serde(default)]
    pub recipients: Vec<String>,
}

#[cfg(test)]
pub(crate) fn connection_transcript() -> Vec<ConnectionTranscriptStep> {
    #[derive(serde::Deserialize)]
    struct Fixture {
        steps: Vec<ConnectionTranscriptStep>,
    }
    serde_json::from_str::<Fixture>(include_str!(
        "../../tests/fixtures/session-connections.json"
    ))
    .expect("shared connection transcript")
    .steps
}

pub struct JsonCodec;

impl MessageCodec for JsonCodec {
    type Error = serde_json::Error;

    fn encode_client(&self, msg: &ClientMessage) -> Result<String, Self::Error> {
        serde_json::to_string(msg)
    }

    fn decode_client(&self, s: &str) -> Result<ClientMessage, Self::Error> {
        serde_json::from_str(s)
    }

    fn encode_server(&self, msg: &ServerMessage) -> Result<String, Self::Error> {
        serde_json::to_string(msg)
    }

    fn decode_server(&self, s: &str) -> Result<ServerMessage, Self::Error> {
        serde_json::from_str(s)
    }
}

// ── HTML console bridge (de)serialisation (ADR-0001 / PRD #419) ────────────
//
// These are the sanctioned `serde_json` surface for the HTML bridge: the
// host-channel pushes (HUD, lobby, chatter, audio) and the inbound
// `ClientMessage` decode. Bridge / plugin code must call these, never
// `serde_json` directly.

/// Encode a `ViewscreenHudState` to JSON for the HTML viewscreen overlay.
pub fn encode_hud_state(
    s: &crate::core::messages::ViewscreenHudState,
) -> Result<String, serde_json::Error> {
    serde_json::to_string(s)
}

/// Encode an AI→AI chatter event for the `"chatter"` host channel (issue
/// #818). Issue #1255's wire shape carries `from_label`, `to_label`, the typed
/// semantic `payload`, and the same required producer-owned `presentation`
/// envelope a phone popup receives. `server.html::__updateChatter` renders only
/// that envelope; it does not derive words from the payload.
pub fn encode_chatter(
    ev: &crate::console_bridge::AiChatterEvent,
) -> Result<String, serde_json::Error> {
    serde_json::to_string(ev)
}

/// Encode the merged ship + world audio config for `__audioConfig`. Sent once
/// on game start; JS builds its `<audio>` elements and Web Audio graph from it.
pub fn encode_audio_config(
    p: &crate::audio_config::AudioConfigPayload,
) -> Result<String, serde_json::Error> {
    serde_json::to_string(p)
}

/// Encode a one-shot positional audio cue for `__audioCue`. Coordinates are
/// listener-relative — see `audio_config::listener_relative`.
pub fn encode_audio_cue(c: &crate::audio_config::AudioCue) -> Result<String, serde_json::Error> {
    serde_json::to_string(c)
}

/// Encode the rendererless GM peer's absolute local map Host Channel projection.
pub fn encode_gm_entity_projection(
    payload: &crate::gm_projection::GmEntityProjectionPayload,
) -> Result<String, serde_json::Error> {
    serde_json::to_string(payload)
}

/// Encode the rendererless GM peer's absolute bounded activity Host Channel
/// projection (issues #1297/#1298).
pub fn encode_gm_activity_feed(
    payload: &crate::gm_activity::GmActivityFeedPayload,
) -> Result<String, serde_json::Error> {
    serde_json::to_string(payload)
}

/// Encode the rendererless GM peer's authentic Station-interface projection.
pub fn encode_gm_station_projection(
    payload: &crate::gm_projection::GmStationProjectionPayload,
) -> Result<String, serde_json::Error> {
    serde_json::to_string(payload)
}

/// Encode a station-activity debug payload to JSON (issue #1145, PRD #1144).
///
/// The single seam where `crate::debug::payload::StationActivityPayload` becomes
/// the JSON the dock chart parses — AGENTS.md Key Constraint 1 keeps `serde_json`
/// here, so `debug::station_activity::publish_station_activity` calls this rather
/// than serialising itself. Returns `String` (not `Result`): the payload is
/// String/int/float scalars in `Vec`s, which serde never fails to encode, so an
/// error becomes an empty string the dock treats as "no data yet" rather than a
/// panic on the sim thread. This is the encoder every later PRD #1144 surface
/// copies for its own payload.
pub fn encode_station_activity(p: &crate::debug::payload::StationActivityPayload) -> String {
    serde_json::to_string(p).unwrap_or_default()
}

/// Encode the scenario-authored GM role preset list (issue #1319) as JSON for
/// `wasm_get_gm_role_presets`. Presentation-only data — see
/// `pasm/spec/design/gm-console-t2.yaml`'s `gm-t2-performing-surface` — so
/// this never touches a `GmAction`, the crew-public GM roster, a snapshot, or
/// the sim digest. `serde_json::to_string` never fails on this shape (plain
/// strings and vectors), so this returns `String` rather than `Result`,
/// matching [`encode_station_activity`] above.
pub fn encode_gm_role_presets(presets: &[crate::world::config::GmRolePresetEntry]) -> String {
    serde_json::to_string(presets).unwrap_or_default()
}

/// Encode an AI-state debug payload to JSON (issues #1149 and #1152, PRD #1144).
///
/// The single seam where `crate::debug::payload::AiStatePayload` becomes the JSON
/// the dock panel parses (`gui/ai-doctrine-panel.js`) and the headless report
/// embeds — AGENTS.md Key Constraint 1 keeps `serde_json` here, so
/// `debug::ai_state::publish_ai_doctrine` and `headless::report::build_report`
/// call this rather than serialising themselves. The one encoder carries BOTH
/// AI sub-surfaces — the per-ship doctrine pool (`ships`, #1149) and the per-host
/// policy-machine view (`hosts`, #1152) — because they are one payload; the
/// `hosts` field is additive, so no schema-version bump. Returns `String` (not
/// `Result`)
/// for the same reason [`encode_station_activity`] does: the payload is
/// String/int/float scalars in `Vec`s, which serde never fails to encode, so an
/// error becomes an empty string a consumer treats as "no data yet".
pub fn encode_ai_doctrine(p: &crate::debug::payload::AiStatePayload) -> String {
    serde_json::to_string(p).unwrap_or_default()
}

/// Encode a scenario-state debug payload to JSON (issue #1148, PRD #1144).
///
/// The single seam where `crate::debug::payload::ScenarioStatePayload` becomes
/// the JSON the dock panel and the headless report read — the same
/// `serde_json`-confined encoder every PRD #1144 surface uses (AGENTS.md Key
/// Constraint 1). Returns `String` (not `Result`) for `encode_station_activity`'s
/// reason: the payload is String/int/float scalars in `Vec`s that serde never
/// fails to encode, so an error becomes an empty string the dock treats as "no
/// data yet" rather than a panic on the sim thread.
pub fn encode_scenario_state(p: &crate::debug::payload::ScenarioStatePayload) -> String {
    serde_json::to_string(p).unwrap_or_default()
}

/// Encode a damage-log debug payload to JSON (issue #1150, PRD #1144).
///
/// The seam where `crate::debug::payload::DamageDebugPayload` becomes the JSON
/// the dock's damage renderer parses. Same contract as
/// [`encode_station_activity`]: `serde_json` is confined here, the return is a
/// `String` (a serialise failure becomes the empty string the dock treats as
/// "no data yet"), and it replaces the legacy `DamageLog::format` text stream.
pub fn encode_damage_debug(p: &crate::debug::payload::DamageDebugPayload) -> String {
    serde_json::to_string(p).unwrap_or_default()
}

/// Encode a modifier debug payload to JSON (issue #1150, PRD #1144).
///
/// Replaces the legacy `ShipModifiers::format_debug` text stream. See
/// [`encode_station_activity`] for the shared encoder contract.
pub fn encode_modifier_debug(p: &crate::debug::payload::ModifierDebugPayload) -> String {
    serde_json::to_string(p).unwrap_or_default()
}

/// Encode an entity-behavior debug payload to JSON (issue #1150, PRD #1144).
///
/// Replaces the legacy `write_entity_debug_state` text stream. See
/// [`encode_station_activity`] for the shared encoder contract.
pub fn encode_entity_behavior(p: &crate::debug::payload::EntityBehaviorPayload) -> String {
    serde_json::to_string(p).unwrap_or_default()
}

/// Encode an entity-inspector debug payload to JSON (issue #1150, PRD #1144).
///
/// Replaces the legacy `update_entity_inspector` text stream. See
/// [`encode_station_activity`] for the shared encoder contract.
pub fn encode_entity_inspector(p: &crate::debug::payload::EntityInspectorPayload) -> String {
    serde_json::to_string(p).unwrap_or_default()
}

/// Encode a console input-to-feedback latency payload to JSON (issue #1169,
/// PRD #1144).
///
/// The single seam where `crate::debug::payload::ConsoleLatencyPayload` becomes
/// the JSON the dock panel (`gui/console-latency-panel.js`) parses and the
/// headless run report embeds — the same `serde_json`-confined encoder every
/// PRD #1144 surface uses (AGENTS.md Key Constraint 1). Returns `String` (not
/// `Result`) for [`encode_station_activity`]'s reason: the payload is
/// String/int/float scalars in `Vec`s that serde never fails to encode, so an
/// error becomes an empty string a consumer treats as "no data yet".
pub fn encode_console_latency(p: &crate::debug::payload::ConsoleLatencyPayload) -> String {
    serde_json::to_string(p).unwrap_or_default()
}

/// Encode the debug-flag read-back for the host page's settings cog (issue
/// #1169 review, finding C2).
///
/// `[(DebugSurface, bool)]` — the exact list `ServerMessage::DebugState` carries —
/// as a flat object keyed by each flag's own variant name:
/// `{"ConsoleLatency":true,"Regions":false,…}`. Flat rather than the wire's pair
/// list because the only consumer asks about one named flag at a time, and
/// `gui/server-settings.js` should not have to know the wire's ordering to answer
/// that. A `BTreeMap`, so the JSON is deterministic.
///
/// Confined here with every other `serde_json` call (AGENTS.md Key Constraint 1)
/// and returns `String` for [`encode_station_activity`]'s reason: the value is
/// string keys and booleans, which serde never fails to encode, so an error
/// becomes the empty string the cog already treats as "the simulation has not
/// reported yet".
pub fn encode_debug_surfaces(flags: &[(crate::core::debug_surface::DebugSurface, bool)]) -> String {
    let map: std::collections::BTreeMap<String, bool> = flags
        .iter()
        .map(|(surface, on)| (surface.wire_name().to_string(), *on))
        .collect();
    serde_json::to_string(&map).unwrap_or_default()
}

/// Decode inbound JSON from the HTML transport bridge.
///
/// The wire shape is a full `ClientMessage` — every emitter (phone consoles,
/// host-page consoles via `gui/action-map.js`, smoke fixtures) sends the
/// serde envelope directly. The short-form system-control shim that used to
/// live here was retired by issue #822 once no console emitted short form.
pub fn decode_bridge_client_message(s: &str) -> Result<ClientMessage, serde_json::Error> {
    serde_json::from_str(s)
}

/// Encode a `LobbyStatePayload` to JSON for the HTML lobby overlay.
pub fn encode_lobby_state(
    s: &crate::core::messages::LobbyStatePayload,
) -> Result<String, serde_json::Error> {
    serde_json::to_string(s)
}

// ── The native host's lobby surface (issues #1328/#1330) ────────────────────
//
// That surface takes one more push beside the lobby state above — the bridge's
// monitor roster — and answers with the operator's own presses: a scenario, a
// hull, an AI launch, a monitor for the viewscreen. Both directions cross as
// JSON, so both are encoded HERE and nowhere else (AGENTS.md Key Constraint 1);
// the types stay pure and Bevy-free in `native_host::host_lobby::{layout,
// scenario}`.
//
// One decode for all four verbs, because the record queue they share is a drain
// with exactly one reader — see `HostLobbyRecord`'s note on why a second record
// type would be a queue two systems fight over.
//
// Gated on the same cfg `crate::native_host` itself carries: a browser host has
// no monitors to offer, and on wasm the module these name does not exist.

/// Encode the bridge's monitor row for the native host's lobby surface.
#[cfg(all(feature = "server", not(target_arch = "wasm32")))]
pub fn encode_bridge_layout(
    p: &crate::native_host::host_lobby::layout::BridgeLayoutPayload,
) -> Result<String, serde_json::Error> {
    serde_json::to_string(p)
}

/// Decode one record the native lobby surface queued — a pick, an AI launch or
/// a monitor button press.
#[cfg(all(feature = "server", not(target_arch = "wasm32")))]
pub fn decode_host_lobby_record(
    s: &str,
) -> Result<crate::native_host::host_lobby::HostLobbyRecord, serde_json::Error> {
    serde_json::from_str(s)
}

/// Queue a typed local recovery intent through the existing host-lobby reader.
#[cfg(all(feature = "server", not(target_arch = "wasm32")))]
pub fn encode_host_lobby_record(
    record: &crate::native_host::host_lobby::HostLobbyRecord,
) -> Result<String, serde_json::Error> {
    serde_json::to_string(record)
}

// ── Batch inbound decode (issue #602) ───────────────────────────────────────

/// A single decode failure from the bridge inbound drain, with truncated
/// fields for safe logging.
#[derive(Clone, Debug, PartialEq)]
pub struct DecodeError {
    pub token: String,
    pub payload_snippet: String,
}

/// Batch-decode a list of `(token, json)` pairs into successful
/// `ClientMessage` values and `DecodeError` failures. Truncates
/// token to 12 chars and payload snippet to 80 chars at collection time.
pub fn decode_bridge_client_messages(
    entries: Vec<(String, String)>,
) -> (Vec<(String, ClientMessage)>, Vec<DecodeError>) {
    let mut successes = Vec::new();
    let mut failures = Vec::new();
    for (token, json) in entries {
        match decode_bridge_client_message(&json) {
            Ok(msg) => successes.push((token, msg)),
            Err(_) => {
                let truncated_token: String = token.chars().take(12).collect();
                let payload_snippet: String = json.chars().take(80).collect();
                failures.push(DecodeError {
                    token: truncated_token,
                    payload_snippet,
                });
            }
        }
    }
    (successes, failures)
}

// ── Rendezvous frames (issue #1113) ──────────────────────────────────────────
//
// The native host's crew path speaks the SAME frames the browser does
// (worker-rendezvous/src/registry.js, gui/rendezvous-transport.js), so it needs
// JSON — and this module is the only one allowed to name `serde_json`
// (AGENTS.md rule 1). The types are `crate::core::rendezvous`'s; the two calls
// per direction are here.
//
// Both decoders are TOLERANT by construction rather than by effort: the frame
// types carry `#[serde(default)]` on every optional field and no
// `deny_unknown_fields`, so an additive field from a newer service (#1114's and
// #1115's are coming) decodes as a frame this build does not act on rather than
// as an error that drops the socket.

/// Encode one rendezvous frame for the socket.
pub fn encode_rendezvous_frame(
    frame: &crate::core::rendezvous::RendezvousFrame,
) -> Result<String, serde_json::Error> {
    serde_json::to_string(frame)
}

/// Decode one rendezvous frame off the socket.
pub fn decode_rendezvous_frame(
    s: &str,
) -> Result<crate::core::rendezvous::RendezvousFrame, serde_json::Error> {
    serde_json::from_str(s)
}

/// Encode one in-band compatibility-handshake frame — the payload INSIDE a
/// relayed game frame, never a `ServerMessage`.
pub fn encode_handshake_frame(
    frame: &crate::core::rendezvous::HandshakeFrame,
) -> Result<String, serde_json::Error> {
    serde_json::to_string(frame)
}

/// Decode one in-band compatibility-handshake frame.
pub fn decode_handshake_frame(
    s: &str,
) -> Result<crate::core::rendezvous::HandshakeFrame, serde_json::Error> {
    serde_json::from_str(s)
}

// ── Delivery documents (PRD #855) ─────────────────────────────────────────────
//
// The native host serves these over HTTP and the browser host publishes the
// identical bytes through `bridge::wasm_delivery_manifest`. They live here for
// the same reason everything above does: `serde_json` is confined to this
// module (AGENTS.md constraint 1), so a host that wants JSON asks for it here.
//
// Catalogue field names and defaults are the serde wire types' own.

/// Encode a presentation diagnostic artifact, outside the crew protocol.
pub fn encode_presentation_capture<T: serde::Serialize>(
    capture: &T,
) -> Result<String, serde_json::Error> {
    serde_json::to_string(capture)
}

#[cfg(test)]
pub(crate) fn decode_presentation_capture<T: serde::de::DeserializeOwned>(
    capture: &[u8],
) -> Result<T, serde_json::Error> {
    serde_json::from_slice(capture)
}

/// The native HUD accepts the same JSON string as the browser HUD bridge.
pub fn encode_hud_update_script(json: &str) -> String {
    let argument = serde_json::to_string(json).unwrap_or_else(|_| "\"{}\"".into());
    format!("window.__updateHud({argument})")
}

fn stamp_json(stamp: &crate::delivery::stamp::DeliveryStamp) -> serde_json::Value {
    serde_json::json!({
        "protocol": stamp.protocol,
        "content_id": stamp.content_id,
        "content_epoch": stamp.content_epoch,
    })
}

/// Encode the shared catalogue entries for the browser host's picker.
pub fn encode_scenario_catalog(
    scenarios: &[crate::core::messages::ScenarioCatalogWire],
) -> Result<String, serde_json::Error> {
    serde_json::to_string(scenarios)
}

/// Decode the browser's current enriched picker snapshot before publication.
pub fn decode_scenario_catalog(
    json: &str,
) -> Result<Vec<crate::core::messages::ScenarioCatalogWire>, serde_json::Error> {
    serde_json::from_str(json)
}

/// Encode a hull with the same optional enrichment on every surface.
pub fn encode_catalog_ship(
    ship: &crate::core::messages::CatalogShipWire,
) -> Result<String, serde_json::Error> {
    serde_json::to_string(ship)
}

/// Encode a host's own version stamp — the body of `/host/stamp.json`.
pub fn encode_delivery_stamp(stamp: &crate::delivery::stamp::DeliveryStamp) -> String {
    stamp_json(stamp).to_string()
}

/// Encode the browser host's join-handshake verdict (issue #1111).
///
/// The same `StampMismatch::code()` the native host answers
/// `/host/manifest.json` with, shaped for the host page's JS rather than for
/// HTTP: `{"ok":true,…}` or `{"ok":false,"code":…,"detail":…}`, always carrying
/// the host's own stamp so a refused client can see what it should have been.
/// `detail` is operator prose, not player-visible text — see
/// [`crate::delivery::stamp::StampMismatch::detail`].
pub fn encode_join_verdict(
    verdict: &Result<(), crate::delivery::stamp::StampMismatch>,
    host: &crate::delivery::stamp::DeliveryStamp,
) -> String {
    match verdict {
        Ok(()) => serde_json::json!({ "ok": true, "host": stamp_json(host) }).to_string(),
        Err(mismatch) => serde_json::json!({
            "ok": false,
            "code": mismatch.code(),
            "detail": mismatch.detail(),
            "host": stamp_json(host),
        })
        .to_string(),
    }
}

/// Encode the content manifest + catalogue a host publishes.
pub fn encode_delivery_manifest(manifest: &crate::delivery::DeliveryManifest) -> String {
    serde_json::json!({
        "stamp": stamp_json(&manifest.stamp),
        "manifest_path": manifest.manifest_path,
        "scenarios": manifest.scenarios,
    })
    .to_string()
}

/// Encode a version-pin refusal — the body of a `409` from either host.
pub fn encode_delivery_refusal(refusal: &crate::delivery::DeliveryRefusal) -> String {
    serde_json::json!({
        "error": refusal.mismatch.code(),
        "detail": refusal.mismatch.detail(),
        "host": stamp_json(&refusal.host),
    })
    .to_string()
}

#[cfg(test)]
#[path = "codec_tests.rs"]
mod tests;

// ── The host mesh's running-mission frames (issue #1116) ──────────────────────
//
// The lockstep vocabulary crosses the same wire `gui/host-mesh.js` owns, in the
// same versioned envelope `{ m, t, tick, d }`. It is encoded HERE rather than
// beside the types for the reason every other JSON encoding in this project is:
// AGENTS.md constraint 1 keeps `serde_json` in this module. The types themselves
// (`lockstep::frame`) stay pure and Bevy-free, and are also serialised as RON in
// the replay/diagnostic path — one shape, two encodings, neither of which knows
// about the other.
//
// The JS half never builds or reads these bodies; it refuses an unknown `t`,
// recognises these two as the simulation's, and ferries them. Rust minting them
// is what keeps "a command on this wire is one an authority gate accepted" true:
// JavaScript that could construct a `tick` frame could construct a command no
// host admitted.

/// The envelope key for the vocabulary revision, matching `gui/host-mesh.js`.
const MESH_ENVELOPE_PROTOCOL: &str = "m";
/// The envelope key for the frame type.
const MESH_ENVELOPE_TYPE: &str = "t";
/// The envelope key for the tick a frame applies at.
const MESH_ENVELOPE_TICK: &str = "tick";
/// The envelope key for the body.
const MESH_ENVELOPE_BODY: &str = "d";

/// Encode one host-mesh simulation frame for the wire.
///
/// The `digest` field crosses as a **hex string**, not a number, and that is
/// load-bearing rather than stylistic: a digest is a `u64` and JavaScript's
/// number type loses integers above 2^53, so a JSON number would silently round
/// the very value two hosts are comparing — the fleet would then report a
/// divergence it does not have, or miss one it does. Ticks and sequences stay
/// numbers; neither can reach 2^53 in any run a human will sit through.
pub fn encode_mesh_frame(frame: &crate::lockstep::MeshFrame) -> Result<String, serde_json::Error> {
    use crate::lockstep::MeshFrame;
    let (tick, body) = match frame {
        MeshFrame::Tick(f) => (
            f.tick,
            serde_json::json!({
                "from": f.from.0,
                "tick": f.tick,
                "ready_through": f.ready_through,
                "start_grant": f.start_grant,
                "commands": f
                    .commands
                    .iter()
                    .map(|c| {
                        Ok(serde_json::json!({
                            "tick": c.tick,
                            "origin": c.order.origin.0,
                            "seq": c.order.seq,
                            "ship": c.ship.0,
                            "target": c.target.0,
                            "payload": serde_json::to_value(&c.payload)?,
                        }))
                    })
                    .collect::<Result<Vec<_>, serde_json::Error>>()?,
            }),
        ),
        MeshFrame::Digest(f) => (
            f.tick,
            serde_json::json!({
                "from": f.from.0,
                "tick": f.tick,
                "digest": format!("{:016x}", f.digest),
            }),
        ),
        // A snapshot chunk (issue #1117). `transfer_id` and `whole_hash` are u64s,
        // so they cross as hex STRINGS for the same reason `digest` does — a JSON
        // number would silently round them above 2^53, and the whole-hash is the
        // value a receiver checks the reassembled record against. `seq`, `total`
        // and `crc` are u32s and stay numbers; none can reach 2^53. `text` is a
        // slice of the RON export, JSON-escaped like any string.
        MeshFrame::Snapshot(c) => (
            c.tick,
            serde_json::json!({
                "from": c.from.0,
                "transfer_id": format!("{:016x}", c.transfer_id),
                "tick": c.tick,
                "seq": c.seq,
                "total": c.total,
                "whole_hash": format!("{:016x}", c.whole_hash),
                "crc": c.crc,
                "text": c.text,
            }),
        ),
        // A host-loss report (issue #1119). `lost` and both slot ordinals are
        // small, and the tick — like `tick` above and unlike `digest` — cannot
        // reach 2^53 in any run a human sits through, so all three cross as
        // numbers rather than as the hex a `u64` digest needs.
        MeshFrame::HostLoss(f) => (
            f.tick,
            serde_json::json!({
                "from": f.from.0,
                "lost": f.lost.0,
                "tick": f.tick,
            }),
        ),
        // A slot-claim announcement (issue #1120). `slot`, both ordinals, the
        // owner-minted `claim_seq` and the tick are all small counters that cannot
        // reach 2^53 in any run a human sits through, so — like `HostLoss` and
        // unlike a `u64` digest — they cross as numbers rather than as hex.
        MeshFrame::SlotClaim(f) => (
            f.tick,
            serde_json::json!({
                "from": f.from.0,
                "slot": f.slot.0,
                "claim_seq": f.claim_seq,
                "tick": f.tick,
            }),
        ),
        MeshFrame::GmAction(f) => match f {
            crate::gm_action::GmActionFrame::Proposal(proposal) => (
                0,
                serde_json::json!({
                    "from": proposal.from.0,
                    "tick": 0,
                    "kind": "proposal",
                    "operator_id": proposal.operator_id,
                    "correlation": proposal.correlation,
                    "action": serde_json::to_value(&proposal.action)?,
                }),
            ),
            crate::gm_action::GmActionFrame::Granted(grant) => (
                grant.apply_tick,
                serde_json::json!({
                    "from": grant.sequenced_by.0,
                    "tick": grant.apply_tick,
                    "kind": "granted",
                    "requester": grant.from.0,
                    "operator_id": grant.operator_id,
                    "correlation": grant.correlation,
                    "recovery_generation": grant.recovery_generation,
                    "apply_tick": grant.apply_tick,
                    "sequence": grant.order.sequence,
                    "action": serde_json::to_value(&grant.action)?,
                }),
            ),
            crate::gm_action::GmActionFrame::Refused(refusal) => {
                let mut body = serde_json::json!({
                    "from": refusal.sequenced_by.0,
                    "tick": refusal.tick,
                    "kind": "refused",
                    "requester": refusal.requester.0,
                    "operator_id": refusal.operator_id,
                    "correlation": refusal.correlation,
                    "action_kind": refusal.action_kind,
                    "requested_active": refusal.requested_active,
                    "reason": refusal.reason,
                    // The stable target the refused action named (issue #1301),
                    // `null` for every family that has none. The decoder reads
                    // an absent key the same way, so a refusal minted by a peer
                    // that predates the event-control family still decodes.
                    "target": refusal.target,
                    // And WHICH lever it refused (issue #1303), on the same
                    // terms: `null` for every family without one, and an
                    // absent key reads the same way.
                    "verb": refusal.verb,
                    // Which lever the refused action pulled (issue #1304),
                    // `null` for a Fire, a Pause and for every family that
                    // pulls none. Same reading rule as `target` above.
                    "lever": refusal.lever,
                    "effect_scope": refusal.effect_scope,
                    "objective_verb": refusal.objective_verb,
                    "objective_recipients": refusal.objective_recipients,
                    "observer": refusal.observer,
                });
                // Keep every unaffected action's wire bytes unchanged.
                if let Some(recipients) = &refusal.comms_recipients {
                    body["comms_recipients"] = serde_json::to_value(recipients)?;
                }
                if let Some(doctrine) = &refusal.npc_doctrine {
                    body["npc_doctrine"] = serde_json::to_value(doctrine)?;
                }
                (refusal.tick, body)
            }
        },
        MeshFrame::GmJoin(frame) => match frame {
            crate::gm_join::GmJoinFrame::Pause(approval) => (
                approval.apply_tick,
                serde_json::json!({
                    "from": approval.owner.0,
                    "tick": approval.apply_tick,
                    "kind": "pause",
                    "join_kind": approval.kind,
                    "join_id": approval.id.0,
                    "approved_by": approval.approved_by.0,
                    "candidate": approval.candidate.host.0,
                    "operator_id": approval.candidate.operator_id,
                    "apply_tick": approval.apply_tick,
                    "transfer_id": format!("{:016x}", approval.transfer_id),
                }),
            ),
            crate::gm_join::GmJoinFrame::Restored { from, id, digest } => (
                0,
                serde_json::json!({
                    "from": from.0,
                    "tick": 0,
                    "kind": "restored",
                    "join_id": id.0,
                    "digest": format!("{:016x}", digest),
                }),
            ),
            crate::gm_join::GmJoinFrame::RestoreBoundary { from, id, boundary } => (
                0,
                serde_json::json!({
                    "from": from.0,
                    "tick": 0,
                    "kind": "restore-boundary",
                    "join_id": id.0,
                    "boundary": boundary,
                }),
            ),
            crate::gm_join::GmJoinFrame::Committed(commit) => (
                commit.tick,
                serde_json::json!({
                    "from": commit.owner.0,
                    "tick": commit.tick,
                    "kind": "committed",
                    "join_kind": commit.kind,
                    "join_id": commit.id.0,
                    "candidate": commit.candidate.host.0,
                    "operator_id": commit.candidate.operator_id,
                    "digest": format!("{:016x}", commit.digest),
                }),
            ),
            crate::gm_join::GmJoinFrame::Refused { from, id, reason } => (
                0,
                serde_json::json!({
                    "from": from.0,
                    "tick": 0,
                    "kind": "refused",
                    "join_id": id.0,
                    "reason": reason,
                }),
            ),
        },
    };
    Ok(serde_json::json!({
        MESH_ENVELOPE_PROTOCOL: crate::lockstep::HOST_MESH_PROTOCOL,
        MESH_ENVELOPE_TYPE: frame.type_name(),
        MESH_ENVELOPE_TICK: tick,
        MESH_ENVELOPE_BODY: body,
    })
    .to_string())
}

/// Decode one host-mesh simulation frame, or `None`.
///
/// `None` covers every "this is not a simulation frame of a revision I speak"
/// case together — unparseable text, a lobby frame, a crew message, a future
/// revision, a body missing a field. The caller's answer to all of them is to
/// drop it, exactly as `gui/host-mesh.js`'s `decodeHostFrame` answers `null`
/// for the same reasons: distinguishing them would invite a receiver to act on
/// a frame it does not understand.
pub fn decode_mesh_frame(raw: &str) -> Option<crate::lockstep::MeshFrame> {
    use crate::command_admission::{CommandOrder, HostSlot, ShipKey};
    use crate::core::messages::SystemId;
    use crate::lockstep::{DigestFrame, HostLossFrame, MeshCommand, MeshFrame, TickFrame};

    let value: serde_json::Value = serde_json::from_str(raw).ok()?;
    if value.get(MESH_ENVELOPE_PROTOCOL)?.as_u64()?
        != u64::from(crate::lockstep::HOST_MESH_PROTOCOL)
    {
        return None;
    }
    let body = value.get(MESH_ENVELOPE_BODY)?;
    let from = HostSlot(u32::try_from(body.get("from")?.as_u64()?).ok()?);
    let tick = body.get("tick")?.as_u64()?;
    match value.get(MESH_ENVELOPE_TYPE)?.as_str()? {
        crate::lockstep::frame::TYPE_TICK => {
            let start_grant = match body.get("start_grant")? {
                serde_json::Value::Null => None,
                value => {
                    let grant: crate::lobby::start_policy::StartGrant =
                        serde_json::from_value(value.clone()).ok()?;
                    grant.validate().ok()?;
                    Some(grant)
                }
            };
            let mut commands = Vec::new();
            for entry in body.get("commands")?.as_array()? {
                commands.push(MeshCommand {
                    tick: entry.get("tick")?.as_u64()?,
                    order: CommandOrder::new(
                        HostSlot(u32::try_from(entry.get("origin")?.as_u64()?).ok()?),
                        entry.get("seq")?.as_u64()?,
                    ),
                    ship: ShipKey(entry.get("ship")?.as_str()?.to_string()),
                    target: SystemId(entry.get("target")?.as_str()?.to_string()),
                    payload: serde_json::from_value(entry.get("payload")?.clone()).ok()?,
                });
            }
            Some(MeshFrame::Tick(TickFrame {
                from,
                tick,
                ready_through: body.get("ready_through")?.as_u64()?,
                commands,
                start_grant,
            }))
        }
        crate::lockstep::frame::TYPE_DIGEST => Some(MeshFrame::Digest(DigestFrame {
            from,
            tick,
            digest: u64::from_str_radix(body.get("digest")?.as_str()?, 16).ok()?,
        })),
        crate::lockstep::frame::TYPE_SNAPSHOT => {
            Some(MeshFrame::Snapshot(crate::lockstep::SnapshotChunk {
                from,
                transfer_id: u64::from_str_radix(body.get("transfer_id")?.as_str()?, 16).ok()?,
                tick,
                seq: u32::try_from(body.get("seq")?.as_u64()?).ok()?,
                total: u32::try_from(body.get("total")?.as_u64()?).ok()?,
                whole_hash: u64::from_str_radix(body.get("whole_hash")?.as_str()?, 16).ok()?,
                crc: u32::try_from(body.get("crc")?.as_u64()?).ok()?,
                text: body.get("text")?.as_str()?.to_string(),
            }))
        }
        crate::lockstep::frame::TYPE_HOST_LOSS => Some(MeshFrame::HostLoss(HostLossFrame {
            from,
            lost: HostSlot(u32::try_from(body.get("lost")?.as_u64()?).ok()?),
            tick,
        })),
        crate::lockstep::frame::TYPE_SLOT_CLAIM => {
            Some(MeshFrame::SlotClaim(crate::lockstep::SlotClaimFrame {
                from,
                slot: HostSlot(u32::try_from(body.get("slot")?.as_u64()?).ok()?),
                claim_seq: body.get("claim_seq")?.as_u64()?,
                tick,
            }))
        }
        crate::lockstep::frame::TYPE_GM_ACTION => {
            let frame = match body.get("kind")?.as_str()? {
                "proposal" => {
                    if tick != 0 {
                        return None;
                    }
                    let proposal = crate::gm_action::GmActionProposal {
                        from,
                        operator_id: body.get("operator_id")?.as_str()?.to_string(),
                        correlation: serde_json::from_value(body.get("correlation")?.clone())
                            .ok()?,
                        action: serde_json::from_value(body.get("action")?.clone()).ok()?,
                    };
                    proposal
                        .validate()
                        .is_ok()
                        .then_some(crate::gm_action::GmActionFrame::Proposal(proposal))?
                }
                "granted" => {
                    let requester = HostSlot(u32::try_from(body.get("requester")?.as_u64()?).ok()?);
                    let grant = crate::gm_action::GmActionGrant {
                        from: requester,
                        sequenced_by: from,
                        operator_id: body.get("operator_id")?.as_str()?.to_string(),
                        correlation: serde_json::from_value(body.get("correlation")?.clone())
                            .ok()?,
                        recovery_generation: body.get("recovery_generation")?.as_u64()?,
                        apply_tick: body.get("apply_tick")?.as_u64()?,
                        order: crate::gm_action::GmActionOrder::new(
                            requester,
                            body.get("sequence")?.as_u64()?,
                        ),
                        action: serde_json::from_value(body.get("action")?.clone()).ok()?,
                    };
                    if grant.apply_tick != tick || grant.validate().is_err() {
                        return None;
                    }
                    crate::gm_action::GmActionFrame::Granted(grant)
                }
                "refused" => {
                    let action_kind =
                        serde_json::from_value(body.get("action_kind")?.clone()).ok()?;
                    let effect_scope = match body.get("effect_scope") {
                        None | Some(serde_json::Value::Null) => None,
                        Some(value) => Some(serde_json::from_value(value.clone()).ok()?),
                    };
                    let observer = match body.get("observer") {
                        None | Some(serde_json::Value::Null) => None,
                        Some(value) => Some(bounded_gm_target_id(value.as_str()?)?),
                    };
                    let comms_recipients: Option<Vec<String>> = body
                        .get("comms_recipients")
                        .filter(|value| !value.is_null())
                        .map(|value| serde_json::from_value(value.clone()))
                        .transpose()
                        .ok()?;
                    if !crate::gm_action::valid_comms_result_scope(
                        action_kind,
                        comms_recipients.as_deref(),
                    ) {
                        return None;
                    }
                    if matches!(
                        action_kind,
                        crate::gm_action::GmActionKind::ContactReveal
                            | crate::gm_action::GmActionKind::ContactConceal
                            | crate::gm_action::GmActionKind::ContactNormal
                    ) != observer.is_some()
                    {
                        return None;
                    }
                    if effect_scope.is_some()
                        && action_kind != crate::gm_action::GmActionKind::DirectEffect
                        && !matches!(
                            action_kind,
                            crate::gm_action::GmActionKind::SystemDisable
                                | crate::gm_action::GmActionKind::SystemRestore
                        )
                    {
                        return None;
                    }
                    crate::gm_action::GmActionFrame::Refused(crate::gm_action::GmActionRefusal {
                        sequenced_by: from,
                        requester: HostSlot(u32::try_from(body.get("requester")?.as_u64()?).ok()?),
                        operator_id: body.get("operator_id")?.as_str()?.to_string(),
                        correlation: serde_json::from_value(body.get("correlation")?.clone())
                            .ok()?,
                        action_kind,
                        requested_active: body.get("requested_active")?.as_bool()?,
                        effect_scope,
                        npc_doctrine: body
                            .get("npc_doctrine")
                            .filter(|v| !v.is_null())
                            .map(|v| serde_json::from_value(v.clone()))
                            .transpose()
                            .ok()?,
                        objective_verb: body
                            .get("objective_verb")
                            .filter(|v| !v.is_null())
                            .map(|v| serde_json::from_value(v.clone()))
                            .transpose()
                            .ok()?,
                        objective_recipients: body
                            .get("objective_recipients")
                            .filter(|v| !v.is_null())
                            .map(|v| serde_json::from_value(v.clone()))
                            .transpose()
                            .ok()?,
                        comms_recipients,
                        observer,
                        tick,
                        reason: serde_json::from_value(body.get("reason")?.clone()).ok()?,
                        // Absent, `null` or unbounded all read as "this family
                        // named no stable target": a peer that predates issue
                        // #1301 omits the key, and the bound is the same one
                        // every other GM id crosses this ingress under.
                        target: body
                            .get("target")
                            .and_then(serde_json::Value::as_str)
                            .and_then(bounded_gm_target_id),
                        // The lever (issue #1303). Absent, `null` or an
                        // unrecognised spelling all read as "this family names
                        // no lever" — `validate_fleet_frame` then refuses the
                        // frame if its family required one, rather than this
                        // ingress guessing Fire.
                        verb: body
                            .get("verb")
                            .cloned()
                            .and_then(|value| serde_json::from_value(value).ok()),
                        // The event-control lever (issue #1304), on the same
                        // terms: absent or `null` leaves Fire/Pause identity
                        // to `verb`, as pre-Skip peers do. An unrecognised
                        // spelling fails the whole frame rather than degrading
                        // to Fire — reporting a refused Skip as a refused Fire
                        // would be the opposite sentence about the same button.
                        lever: match body.get("lever") {
                            None | Some(serde_json::Value::Null) => None,
                            Some(value) => Some(serde_json::from_value(value.clone()).ok()?),
                        },
                    })
                }
                _ => return None,
            };
            Some(MeshFrame::GmAction(frame))
        }
        crate::lockstep::frame::TYPE_GM_JOIN => {
            use crate::gm_join::{
                GmJoinApproval, GmJoinCandidate, GmJoinCommit, GmJoinFrame, GmJoinId,
            };
            let id = GmJoinId(body.get("join_id")?.as_u64()?);
            let frame = match body.get("kind")?.as_str()? {
                "pause" => {
                    let approval = GmJoinApproval {
                        id,
                        kind: serde_json::from_value(body.get("join_kind")?.clone()).ok()?,
                        owner: from,
                        approved_by: HostSlot(
                            u32::try_from(body.get("approved_by")?.as_u64()?).ok()?,
                        ),
                        candidate: GmJoinCandidate {
                            host: HostSlot(u32::try_from(body.get("candidate")?.as_u64()?).ok()?),
                            operator_id: body.get("operator_id")?.as_str()?.to_string(),
                        },
                        apply_tick: body.get("apply_tick")?.as_u64()?,
                        transfer_id: u64::from_str_radix(body.get("transfer_id")?.as_str()?, 16)
                            .ok()?,
                    };
                    if approval.apply_tick != tick {
                        return None;
                    }
                    GmJoinFrame::Pause(approval)
                }
                "restored" => {
                    if tick != 0 {
                        return None;
                    }
                    GmJoinFrame::Restored {
                        from,
                        id,
                        digest: u64::from_str_radix(body.get("digest")?.as_str()?, 16).ok()?,
                    }
                }
                "restore-boundary" => {
                    if tick != 0 {
                        return None;
                    }
                    GmJoinFrame::RestoreBoundary {
                        from,
                        id,
                        boundary: u16::try_from(body.get("boundary")?.as_u64()?).ok()?,
                    }
                }
                "committed" => GmJoinFrame::Committed(GmJoinCommit {
                    id,
                    kind: serde_json::from_value(body.get("join_kind")?.clone()).ok()?,
                    owner: from,
                    candidate: GmJoinCandidate {
                        host: HostSlot(u32::try_from(body.get("candidate")?.as_u64()?).ok()?),
                        operator_id: body.get("operator_id")?.as_str()?.to_string(),
                    },
                    tick,
                    digest: u64::from_str_radix(body.get("digest")?.as_str()?, 16).ok()?,
                }),
                "refused" => {
                    if tick != 0 {
                        return None;
                    }
                    GmJoinFrame::Refused {
                        from,
                        id,
                        reason: serde_json::from_value(body.get("reason")?.clone()).ok()?,
                    }
                }
                _ => return None,
            };
            Some(MeshFrame::GmJoin(frame))
        }
        _ => None,
    }
}

/// Decode the frozen fleet roster a host page hands the simulation at mission
/// start (issue #1116).
///
/// ```json
/// { "local": 2, "owner": 1, "participants": [1, 2, 3], "delay": 6,
///   "gms": [ { "host": 2, "operator_id": "gm-1" } ],
///   "ships": [ { "host": 1, "ship_path": "assets/entities/alliance_cruiser.toml",
///                "crew": [["helm", "Std"], ["tactical", "Std"]] } ] }
/// ```
///
/// `delay` is optional and normally absent: the fleet agreed a MISSION, and the
/// mission's own `[global] command_delay_ticks` is the number. It is accepted
/// so a harness or a future operator control can override it without a second
/// entry point.
///
/// `None` for anything unreadable, and the caller refuses to start rather than
/// guessing — a fleet that disagreed about its own roster would spawn different
/// ships with different identities and never agree on a single tick.
pub fn decode_fleet_roster(raw: &str) -> Option<(crate::lockstep::FleetRoster, Option<u64>)> {
    use crate::command_admission::HostSlot;
    use crate::core::messages::StationId;
    use crate::lockstep::{FleetGm, FleetRoster, FleetShip};

    let value: serde_json::Value = serde_json::from_str(raw).ok()?;
    let local = HostSlot(u32::try_from(value.get("local")?.as_u64()?).ok()?);
    let delay = value.get("delay").and_then(|d| d.as_u64());
    let mut ships = Vec::new();
    for entry in value.get("ships")?.as_array()? {
        let host = HostSlot(u32::try_from(entry.get("host")?.as_u64()?).ok()?);
        let ship_path = entry
            .get("ship_path")
            .and_then(|p| p.as_str())
            .map(str::to_string);
        let mut crew = Vec::new();
        let seats = match entry.get("crew") {
            None => &[][..],
            Some(value) => value.as_array()?.as_slice(),
        };
        if seats.len() > crate::lockstep::crew::MAX_FLEET_CREW_SEATS {
            return None;
        }
        for seat in seats {
            let pair = seat.as_array()?;
            if pair.len() != 2 {
                return None;
            }
            crew.push((
                StationId(pair.first()?.as_str()?.to_string()),
                pair.get(1)?.as_str()?.to_string(),
            ));
        }
        ships.push(FleetShip {
            host,
            ship_path,
            crew: crate::lockstep::crew::canonical_station_ratings(crew)?,
        });
    }
    if let Some(entries) = value.get("participants") {
        let participants = entries
            .as_array()?
            .iter()
            .map(|entry| {
                let slot = u32::try_from(entry.as_u64()?).ok()?;
                (slot > 0).then_some(HostSlot(slot))
            })
            .collect::<Option<Vec<_>>>()?;
        let owner = HostSlot(u32::try_from(value.get("owner")?.as_u64()?).ok()?);
        let gms = value
            .get("gms")
            .and_then(serde_json::Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or_default()
            .iter()
            .map(|entry| {
                Some(FleetGm {
                    host: HostSlot(u32::try_from(entry.get("host")?.as_u64()?).ok()?),
                    operator_id: entry.get("operator_id")?.as_str()?.to_string(),
                })
            })
            .collect::<Option<Vec<_>>>()?;
        return Some((
            FleetRoster::with_participants_and_gms(ships, participants, gms, local, owner)?,
            delay,
        ));
    }
    // Compatibility for pre-v7 native fixtures and stored boot identities. A
    // new browser fleet always supplies the explicit private participant set;
    // only that exact path may represent a zero-ship GM-only simulation.
    (!ships.is_empty()).then(|| (FleetRoster::new(ships, local), delay))
}

/// Decode one complete crew-public GM roster from the host page (issue #1289).
///
/// The accepted wire shape is exactly an array of
/// `{ id, name, connected, ready }`
/// rows. [`crate::gm_roster::GmOperator`]'s `deny_unknown_fields` prevents a
/// private peer id, reconnect credential or authority flag from crossing this
/// boundary unnoticed; [`crate::gm_roster::GmRoster::try_new`] applies the
/// bounded, unique and deterministic roster contract.
pub fn decode_gm_roster(raw: &str) -> Option<crate::gm_roster::GmRoster> {
    let operators: Vec<crate::gm_roster::GmOperator> = serde_json::from_str(raw).ok()?;
    crate::gm_roster::GmRoster::try_new(operators).ok()
}

/// Decode the one privileged browser-GM action ingress (issue #1292).
/// Unknown fields are refused so this narrow route cannot accidentally become
/// a generic host mutation surface.
pub fn decode_gm_action_request(raw: &str) -> Option<crate::gm_action::GmActionRequest> {
    let value: serde_json::Value = serde_json::from_str(raw).ok()?;
    let object = value.as_object()?;
    let action = match object.get("action")?.as_str()? {
        "set_session_paused" if object.len() == 4 && object.contains_key("active") => {
            crate::gm_action::GmAction::SetSessionPaused {
                active: object.get("active")?.as_bool()?,
            }
        }
        "set_station_puppet"
            if object.len() == 6
                && object.contains_key("ship")
                && object.contains_key("station")
                && object.contains_key("active") =>
        {
            crate::gm_action::GmAction::SetStationPuppet {
                ship: crate::command_admission::log::ShipKey(bounded_gm_target_id(
                    object.get("ship")?.as_str()?,
                )?),
                station: crate::core::messages::StationId(bounded_gm_target_id(
                    object.get("station")?.as_str()?,
                )?),
                active: object.get("active")?.as_bool()?,
            }
        }
        // Exactly `{operator_id, correlation, action, event}` — the same
        // per-shape field-count guard every arm here carries, so a Fire cannot
        // smuggle a second target past the narrow ingress.
        "fire_gm_event" if object.len() == 4 && object.contains_key("event") => {
            crate::gm_action::GmAction::FireGmEvent {
                event: bounded_gm_event_id(object.get("event")?.as_str()?)?,
            }
        }
        // The Skip lever (issue #1304) on the same exact narrow shape: one
        // stable layer-qualified event id and nothing else. A separate wire
        // verb rather than a `lever` field on `fire_gm_event`, so a page that
        // means to fire can never arm a skip by mistyping one value — the two
        // levers do opposite things to the same event.
        "arm_gm_event_skip" if object.len() == 4 && object.contains_key("event") => {
            crate::gm_action::GmAction::ArmGmEventSkip {
                event: bounded_gm_event_id(object.get("event")?.as_str()?)?,
            }
        }
        // Exactly `{operator_id, correlation, action, entity, effect, amount,
        // scope, scope_id}` (issues #1310, #1311). The amount arrives as
        // milli-HP so the browser cannot hand the journal a float it would then
        // have to round.
        //
        // `scope` and `scope_id` are ALWAYS present — `scope_id` is `null` for
        // the whole-entity scope — so the field-count guard stays exact rather
        // than accepting two shapes, exactly as `spawn_palette_entity`'s
        // always-present `variant` does. The pair is a discriminant plus one id
        // rather than the enum's own serde shape because THIS boundary is a
        // narrow deny-by-default ingress: a flat pair has one spelling a browser
        // can send, and a nested object would give a client a second place to
        // put a field nobody validated.
        "apply_direct_effect"
            if object.len() == 8
                && object.contains_key("entity")
                && object.contains_key("effect")
                && object.contains_key("amount_milli_hp")
                && object.contains_key("scope")
                && object.contains_key("scope_id") =>
        {
            let effect = match object.get("effect")?.as_str()? {
                "damage" => crate::gm_effect::GmDirectEffectKind::Damage,
                "heal" => crate::gm_effect::GmDirectEffectKind::Heal,
                _ => return None,
            };
            let scope_id = object.get("scope_id")?;
            let scope = match object.get("scope")?.as_str()? {
                // A whole-entity effect carries no id, and one that arrives
                // anyway is refused rather than ignored: a request the ingress
                // silently reinterprets is a request whose result cannot be
                // predicted from what was sent.
                "entity" if scope_id.is_null() => crate::gm_effect::GmDirectEffectScope::Entity,
                "station" => crate::gm_effect::GmDirectEffectScope::Station(
                    crate::core::messages::StationId(bounded_gm_target_id(scope_id.as_str()?)?),
                ),
                "system" => crate::gm_effect::GmDirectEffectScope::System(
                    crate::core::messages::SystemId(bounded_gm_target_id(scope_id.as_str()?)?),
                ),
                _ => return None,
            };
            let amount = object.get("amount_milli_hp")?.as_u64()?;
            crate::gm_action::GmAction::ApplyDirectEffect {
                target: bounded_gm_target_id(object.get("entity")?.as_str()?)?,
                scope,
                effect,
                amount_milli_hp: u32::try_from(amount).ok().filter(|amount| *amount > 0)?,
            }
        }
        // Exactly `{operator_id, correlation, action, palette, variant,
        // position_mm, heading_mdeg}` (issue #1305). `variant` is always
        // present — `null` for the bare template — so the field-count guard
        // stays exact rather than accepting two shapes. There is deliberately
        // no template/asset field to smuggle anything through: the palette id
        // IS the vocabulary.
        "objective_action" if object.len() == 6 => {
            let action = crate::gm_action::GmAction::ObjectiveAction {
                objective: bounded_gm_target_id(object.get("objective")?.as_str()?)?,
                verb: serde_json::from_value(object.get("verb")?.clone()).ok()?,
                recipients: serde_json::from_value(object.get("recipients")?.clone()).ok()?,
            };
            // Reuse canonical bounds and ordering: duplicate or malformed
            // recipients are invalid vocabulary before they reach admission.
            action.validate().ok()?;
            action
        }
        "set_contact_override" if object.len() == 6 => {
            crate::gm_action::GmAction::SetContactOverride {
                ship: crate::command_admission::log::ShipKey(
                    object.get("ship")?.as_str()?.to_owned(),
                ),
                target: object.get("target")?.as_str()?.to_owned(),
                mode: serde_json::from_value(object.get("mode")?.clone()).ok()?,
            }
        }
        "set_system_disabled" if object.len() == 6 => {
            crate::gm_action::GmAction::SetSystemDisabled {
                target: bounded_gm_target_id(object.get("target")?.as_str()?)?,
                system: crate::core::messages::SystemId(bounded_gm_target_id(
                    object.get("system")?.as_str()?,
                )?),
                disabled: object.get("disabled")?.as_bool()?,
            }
        }
        "transmit_comms" if object.len() == 4 => {
            let transmission: crate::gm_comms::GmCommsTransmission =
                serde_json::from_value(object.get("transmission")?.clone()).ok()?;
            if !transmission.valid_shape() {
                return None;
            }
            crate::gm_action::GmAction::TransmitComms { transmission }
        }
        "set_npc_doctrine" if object.len() == 5 => crate::gm_action::GmAction::SetNpcDoctrine {
            target: bounded_gm_target_id(object.get("target")?.as_str()?)?,
            doctrine: bounded_gm_target_id(object.get("doctrine")?.as_str()?)?,
        },
        "despawn_entity" if object.len() == 4 => crate::gm_action::GmAction::DespawnEntity {
            target: bounded_gm_target_id(object.get("target")?.as_str()?)?,
        },
        "spawn_palette_entity"
            if object.len() == 7
                && object.contains_key("palette")
                && object.contains_key("variant")
                && object.contains_key("position_mm")
                && object.contains_key("heading_mdeg") =>
        {
            let position = object.get("position_mm")?.as_array()?;
            if position.len() != 3 {
                return None;
            }
            let mut position_mm = [0i64; 3];
            for (slot, value) in position_mm.iter_mut().zip(position) {
                *slot = value.as_i64()?;
            }
            let variant = match object.get("variant")? {
                serde_json::Value::Null => None,
                value => Some(bounded_gm_target_id(value.as_str()?)?),
            };
            let heading_mdeg = i32::try_from(object.get("heading_mdeg")?.as_i64()?).ok()?;
            // The placement bound is the action's own answer, applied HERE too
            // so an out-of-range request never becomes a grant, exactly as
            // `bounded_gm_event_id` refuses an unqualified event id at ingress.
            if !crate::gm_spawn::placement_is_valid(position_mm, heading_mdeg) {
                return None;
            }
            crate::gm_action::GmAction::SpawnPaletteEntity {
                palette: bounded_gm_target_id(object.get("palette")?.as_str()?)?,
                variant,
                position_mm,
                heading_mdeg,
            }
        }
        // Exactly `{operator_id, correlation, action, event, active}`. The
        // absolute `active` is the same shape `set_session_paused` carries and
        // for the same reason: two GMs pressing at once must not depend on
        // arrival order for the state the event ends up in.
        "set_event_paused"
            if object.len() == 5
                && object.contains_key("event")
                && object.contains_key("active") =>
        {
            crate::gm_action::GmAction::SetEventPaused {
                event: bounded_gm_event_id(object.get("event")?.as_str()?)?,
                active: object.get("active")?.as_bool()?,
            }
        }
        "issue_station_command"
            if object.len() == 7
                && object.contains_key("ship")
                && object.contains_key("station")
                && object.contains_key("target")
                && object.contains_key("payload") =>
        {
            let payload: crate::core::messages::SystemControlPayload =
                serde_json::from_value(object.get("payload")?.clone()).ok()?;
            crate::gm_action::GmAction::IssueStationCommand {
                ship: crate::command_admission::log::ShipKey(bounded_gm_target_id(
                    object.get("ship")?.as_str()?,
                )?),
                station: crate::core::messages::StationId(bounded_gm_target_id(
                    object.get("station")?.as_str()?,
                )?),
                target: crate::core::messages::SystemId(bounded_gm_target_id(
                    object.get("target")?.as_str()?,
                )?),
                payload: canonical_system_command(&payload)?,
            }
        }
        _ => return None,
    };
    let request = crate::gm_action::GmActionRequest {
        operator_id: object.get("operator_id")?.as_str()?.to_string(),
        correlation: serde_json::from_value(object.get("correlation")?.clone()).ok()?,
        action,
    };
    (!request.operator_id.is_empty()
        && request.operator_id.chars().count() <= crate::gm_roster::MAX_GM_OPERATOR_ID_CHARS)
        .then_some(request)
}

/// One layer-qualified GM event id: an ordinary bounded target id that also
/// carries its `<layer>::<authored id>` qualifier. An unqualified id is refused
/// at the ingress rather than resolved, because it cannot name one event once
/// two layers each author the same authored id.
fn bounded_gm_event_id(value: &str) -> Option<String> {
    bounded_gm_target_id(value)
        .filter(|value| value.contains("::") && !value.ends_with("::") && !value.starts_with("::"))
}

fn bounded_gm_target_id(value: &str) -> Option<String> {
    (!value.is_empty() && value.len() <= 128 && !value.chars().any(char::is_control))
        .then(|| value.to_string())
}

pub fn canonical_system_command(
    payload: &crate::core::messages::SystemControlPayload,
) -> Option<crate::gm_puppet::CanonicalSystemCommandPayload> {
    crate::gm_puppet::CanonicalSystemCommandPayload::new(serde_json::to_string(payload).ok()?).ok()
}

pub fn decode_canonical_system_command(
    raw: &str,
) -> Option<crate::core::messages::SystemControlPayload> {
    let payload = serde_json::from_str(raw).ok()?;
    (canonical_system_command(&payload)?.as_str() == raw).then_some(payload)
}

pub fn encode_gm_session_projection(
    projection: &crate::gm_action::GmSessionProjection,
) -> Result<String, serde_json::Error> {
    serde_json::to_string(projection)
}

/// Encode the absolute GM spawn-panel projection (issue #1305) for the
/// `gm_spawn` Host Channel. The payload carries palette ids and String Table
/// labels only — never a `template_path`, which the browser must never be in a
/// position to name.
pub fn encode_gm_spawn_projection(
    payload: &crate::gm_spawn::GmSpawnProjection,
) -> Result<String, serde_json::Error> {
    serde_json::to_string(payload)
}

/// Encode the absolute GM mission-panel projection (issue #1301).
pub fn encode_gm_mission_projection(
    projection: &crate::gm_event::GmMissionProjection,
) -> Result<String, serde_json::Error> {
    serde_json::to_string(projection)
}

/// Encode the read-only GM admission/reconnect progress mirrored to the page.
pub fn encode_gm_join_progress(
    progress: &crate::gm_join::GmJoinProgress,
) -> Result<String, serde_json::Error> {
    serde_json::to_string(progress)
}

/// Decode and validate one host-mesh lobby start grant (issue #1290).
///
/// JSON stays confined to this codec seam. The grant itself is exact and
/// bounded; `StartGrant::validate` additionally checks the `start-N`
/// idempotency key and the mode/attribution pairing.
pub fn decode_start_grant(raw: &str) -> Option<crate::lobby::start_policy::StartGrant> {
    let grant: crate::lobby::start_policy::StartGrant = serde_json::from_str(raw).ok()?;
    grant.validate().ok()?;
    Some(grant)
}

pub fn encode_start_grant_result(
    result: &crate::lobby::start_policy::StartGrantResult,
) -> Result<String, serde_json::Error> {
    serde_json::to_string(result)
}

/// Encode the definitive asynchronous result of `wasm_join_fleet` adoption.
pub fn encode_fleet_join_status(
    status: &crate::lockstep::FleetJoinStatus,
) -> Result<String, serde_json::Error> {
    serde_json::to_string(status)
}

/// Encode the fleet-link status the host page's operator surface reads.
///
/// Derived and read-only. `waiting_on` names the peers a stall is blocked on,
/// which is what turns "the mission froze" into "slot 2 is eleven ticks behind"
/// — the difference between a bug report and a fix.
#[allow(clippy::too_many_arguments)]
pub fn encode_mesh_status(
    in_fleet: bool,
    slot: Option<u32>,
    tick: u64,
    delay: u64,
    diagnostics: &crate::lockstep::MeshDiagnostics,
    agreement: &crate::lockstep::MeshAgreement,
    peers: &[u32],
) -> String {
    let disagreement = agreement.first_disagreement().map(|found| {
        serde_json::json!({
            "tick": found.tick,
            "peer": found.peer.0,
            "local": format!("{:016x}", found.local_digest),
            "peer_digest": format!("{:016x}", found.peer_digest),
        })
    });
    serde_json::json!({
        "in_fleet": in_fleet,
        "slot": slot,
        "tick": tick,
        "delay": delay,
        "stalled": diagnostics.is_stalled(),
        "stalled_frames": diagnostics.stalled_frames,
        "longest_stall": diagnostics.longest_stall,
        // Only while it is actually waiting. `MeshDiagnostics` remembers the
        // last stall for the operator log, but "who am I waiting for" has no
        // answer when the answer is nobody — reporting the remembered one would
        // have a running fleet permanently accusing a peer that is keeping up.
        "waiting_on": if diagnostics.is_stalled() {
            diagnostics
                .last_stall
                .as_ref()
                .map(|stall| stall.waiting_on.iter().map(|(slot, _)| slot.0).collect::<Vec<_>>())
                .unwrap_or_default()
        } else {
            Vec::new()
        },
        "peers": peers,
        "peers_heard": agreement.peers.keys().map(|slot| slot.0).collect::<Vec<_>>(),
        "samples": agreement.local.checkpoints.len(),
        "agreed": agreement.agreed(),
        "disagreement": disagreement,
    })
    .to_string()
}

/// Absolute local GM Comms Studio projection.
pub fn encode_gm_comms_projection(
    payload: &crate::gm_comms::GmCommsProjection,
) -> Result<String, serde_json::Error> {
    serde_json::to_string(payload)
}

#[cfg(all(feature = "server", not(target_arch = "wasm32")))]
pub fn decode_native_gm_record(
    json: &str,
) -> Option<crate::native_host::native_gm::NativeGmRecord> {
    if json.len() > 128 * 1024 {
        return None;
    }
    serde_json::from_str(json).ok()
}
#[cfg(all(feature = "server", not(target_arch = "wasm32")))]
pub fn encode_native_gm_metadata(
    value: &crate::native_host::native_gm::NativeGmMetadata,
) -> Result<String, serde_json::Error> {
    serde_json::to_string(value)
}

#[cfg(test)]
mod mesh_frame_tests {
    use crate::command_admission::{CommandOrder, HostSlot, ShipKey};
    use crate::core::messages::{SystemControlPayload, SystemId};
    use crate::lobby::start_policy::{StartGrant, StartGrantMode};
    use crate::lockstep::{
        DigestFrame, HostLossFrame, MeshCommand, MeshFrame, TickFrame, HOST_MESH_PROTOCOL,
    };

    fn tick_frame() -> MeshFrame {
        MeshFrame::Tick(TickFrame {
            from: HostSlot(2),
            tick: 412,
            ready_through: 418,
            commands: vec![
                MeshCommand {
                    tick: 418,
                    order: CommandOrder::new(HostSlot(2), 7),
                    ship: ShipKey("00000000-0000-8000-8000-000000000001".into()),
                    target: SystemId("helm-steering".into()),
                    payload: SystemControlPayload::SetSteering { value: -0.4 },
                },
                MeshCommand {
                    tick: 418,
                    order: CommandOrder::new(HostSlot(2), 8),
                    ship: ShipKey("00000000-0000-8000-8000-000000000001".into()),
                    target: SystemId("red-alert".into()),
                    payload: SystemControlPayload::SetRedAlert { active: true },
                },
            ],
            start_grant: Some(StartGrant {
                id: "start-7".into(),
                mode: StartGrantMode::Forced,
                operator_id: Some("gm-1".into()),
                apply_tick: 419,
            }),
        })
    }

    /// The wire shape is the envelope `gui/host-mesh.js` owns, and a frame
    /// survives it unchanged.
    #[test]
    fn a_tick_frame_round_trips_through_the_shared_envelope() {
        let frame = tick_frame();
        let text = super::encode_mesh_frame(&frame).expect("encodes");
        assert!(
            text.contains(&format!("\"m\":{HOST_MESH_PROTOCOL}")),
            "the revision travels: {text}"
        );
        assert!(text.contains("\"t\":\"tick\""), "{text}");
        assert!(
            text.contains("\"tick\":412"),
            "the envelope's own tick stamp — the field #1114 added for exactly \
             this — must carry the tick the frame applies at: {text}"
        );
        assert!(
            text.contains("\"apply_tick\":419"),
            "the owner-scheduled start boundary travels inside the tick frame: {text}"
        );
        assert_eq!(super::decode_mesh_frame(&text), Some(frame));
    }

    /// A digest crosses as a hex STRING.
    ///
    /// The load-bearing half of this vocabulary's encoding: a digest is a `u64`
    /// and JavaScript's number type loses integers above 2^53, so a JSON number
    /// would silently round the very value two hosts compare — reporting a
    /// divergence the fleet does not have, or missing one it does.
    #[test]
    fn a_digest_crosses_as_a_string_because_json_numbers_lose_it() {
        let digest = 0xdead_beef_dead_beef_u64;
        assert!(
            digest > (1_u64 << 53),
            "precondition: the sample must be big enough for a JSON number to \
             round it, or this test proves nothing"
        );
        let frame = MeshFrame::Digest(DigestFrame {
            from: HostSlot(1),
            tick: 300,
            digest,
        });
        let text = super::encode_mesh_frame(&frame).expect("encodes");
        assert!(
            text.contains("\"digest\":\"deadbeefdeadbeef\""),
            "the digest must be a hex string: {text}"
        );
        assert_eq!(super::decode_mesh_frame(&text), Some(frame));
    }

    /// A snapshot chunk round-trips through the shared envelope (issue #1117),
    /// and its two u64 fields — `transfer_id` and `whole_hash` — cross as hex
    /// strings for the same reason a digest does: a JSON number rounds above 2^53,
    /// and the whole-hash is the value a receiver verifies the record against.
    #[test]
    fn a_snapshot_chunk_round_trips_and_keeps_its_u64_fields_exact() {
        use crate::lockstep::SnapshotChunk;
        let whole_hash = 0xfeed_face_dead_beef_u64;
        let transfer_id = 0x0123_4567_89ab_cdef_u64;
        assert!(whole_hash > (1_u64 << 53) && transfer_id > (1_u64 << 53));
        let frame = MeshFrame::Snapshot(SnapshotChunk {
            from: HostSlot(2),
            transfer_id,
            tick: 400,
            seq: 3,
            total: 9,
            whole_hash,
            crc: 0xdead_beef,
            text: "a RON slice with \"quotes\" and \n a newline".to_string(),
        });
        let text = super::encode_mesh_frame(&frame).expect("encodes");
        assert!(text.contains("\"t\":\"snapshot\""), "{text}");
        assert!(
            text.contains("\"whole_hash\":\"feedfacedeadbeef\""),
            "{text}"
        );
        assert!(
            text.contains("\"transfer_id\":\"0123456789abcdef\""),
            "{text}"
        );
        assert_eq!(super::decode_mesh_frame(&text), Some(frame));
    }

    /// Everything that is not a simulation frame of a revision this build
    /// speaks answers `None`, together — the same discipline
    /// `decodeHostFrame` keeps on the other side of the wire.
    #[test]
    fn anything_that_is_not_a_frame_of_this_revision_is_refused() {
        for raw in [
            "not json at all",
            r#"{"type":"Identify","token":"abc"}"#,
            // A superseded revision — refused whole rather than half-read.
            r#"{"m":2,"t":"tick","tick":1,"d":{"from":1,"tick":1,"ready_through":1,"commands":[]}}"#,
            // A lobby frame on the simulation decoder.
            r#"{"m":4,"t":"hello","tick":null,"d":{}}"#,
            // A tick frame missing its watermark and commands.
            r#"{"m":4,"t":"tick","tick":1,"d":{"from":1,"tick":1}}"#,
            // A digest as a JSON number, which loses the top bits — refused.
            r#"{"m":4,"t":"digest","tick":1,"d":{"from":1,"tick":1,"digest":12345}}"#,
            // A host-loss frame missing the slot it names.
            r#"{"m":4,"t":"host-loss","tick":1,"d":{"from":1,"tick":1}}"#,
            // A slot-claim missing the slot it reclaims (issue #1120).
            r#"{"m":4,"t":"slot-claim","tick":1,"d":{"from":1,"claim_seq":1}}"#,
        ] {
            assert_eq!(
                super::decode_mesh_frame(raw),
                None,
                "must be refused rather than half understood: {raw}"
            );
        }
    }

    /// A host-loss report (issue #1119) survives the shared envelope: the slot
    /// it names, the reporter, and the agreed tick all come back unchanged.
    #[test]
    fn a_host_loss_frame_round_trips_through_the_shared_envelope() {
        let frame = MeshFrame::HostLoss(HostLossFrame {
            from: HostSlot(1),
            lost: HostSlot(3),
            tick: 418,
        });
        let text = super::encode_mesh_frame(&frame).expect("encodes");
        assert!(
            text.contains(&format!("\"m\":{HOST_MESH_PROTOCOL}")),
            "the revision travels: {text}"
        );
        assert!(text.contains("\"t\":\"host-loss\""), "{text}");
        assert!(
            text.contains("\"lost\":3"),
            "the slot whose host left must survive the wire: {text}"
        );
        assert!(
            text.contains("\"tick\":418"),
            "the agreed disconnect tick must survive the wire: {text}"
        );
        assert_eq!(super::decode_mesh_frame(&text), Some(frame));
    }

    /// A slot-claim announcement (issue #1120) survives the shared envelope: the
    /// slot it reclaims, the owner that stamped it, the deterministic claim
    /// sequence and the tick all come back unchanged.
    #[test]
    fn a_slot_claim_frame_round_trips_through_the_shared_envelope() {
        let frame = MeshFrame::SlotClaim(crate::lockstep::SlotClaimFrame {
            from: crate::command_admission::HostSlot(1),
            slot: crate::command_admission::HostSlot(3),
            claim_seq: 7,
            tick: 512,
        });
        let text = super::encode_mesh_frame(&frame).expect("encodes");
        assert!(text.contains("\"t\":\"slot-claim\""), "{text}");
        assert!(
            text.contains("\"slot\":3"),
            "the slot being reclaimed must survive the wire: {text}"
        );
        assert!(
            text.contains("\"claim_seq\":7"),
            "the deterministic tiebreak must survive the wire: {text}"
        );
        assert_eq!(super::decode_mesh_frame(&text), Some(frame));
    }

    #[test]
    fn an_attributed_gm_action_round_trips_through_the_shared_envelope() {
        let frame = MeshFrame::GmAction(crate::gm_action::GmActionFrame::Granted(
            crate::gm_action::GmActionGrant {
                from: HostSlot(2),
                sequenced_by: HostSlot(1),
                operator_id: "gm-1".into(),
                correlation: crate::gm_action::GmActionId::new("pause-17").unwrap(),
                recovery_generation: 0,
                apply_tick: 419,
                order: crate::gm_action::GmActionOrder::new(HostSlot(2), 17),
                action: crate::gm_action::GmAction::SetSessionPaused { active: true },
            },
        ));
        let text = super::encode_mesh_frame(&frame).expect("encodes");
        assert!(
            text.contains(&format!("\"m\":{HOST_MESH_PROTOCOL}")),
            "the revision travels: {text}"
        );
        assert!(text.contains("\"t\":\"gm-action\""), "{text}");
        assert!(text.contains("\"operator_id\":\"gm-1\""), "{text}");
        assert!(text.contains("\"tick\":419"), "{text}");
        assert_eq!(super::decode_mesh_frame(&text), Some(frame));
    }

    #[test]
    fn gm_proposals_and_canonical_refusals_round_trip_on_the_same_lane() {
        let proposal = MeshFrame::GmAction(crate::gm_action::GmActionFrame::Proposal(
            crate::gm_action::GmActionProposal {
                from: HostSlot(2),
                operator_id: "gm-1".into(),
                correlation: crate::gm_action::GmActionId::new("proposal-1").unwrap(),
                action: crate::gm_action::GmAction::SetSessionPaused { active: true },
            },
        ));
        let refusal = MeshFrame::GmAction(crate::gm_action::GmActionFrame::Refused(
            crate::gm_action::GmActionRefusal {
                sequenced_by: HostSlot(1),
                requester: HostSlot(2),
                operator_id: "gm-1".into(),
                correlation: crate::gm_action::GmActionId::new("proposal-1").unwrap(),
                action_kind: crate::gm_action::GmActionKind::SessionPause,
                requested_active: true,
                tick: 419,
                reason: crate::gm_action::GmActionRefusalReason::WrongPhase,
                target: None,
                verb: None,
                lever: None,
                effect_scope: None,
                objective_verb: None,
                objective_recipients: None,
                comms_recipients: None,
                observer: None,
                npc_doctrine: None,
            },
        ));
        // A refused Fire crosses the same lane still naming the event it tried
        // to fire (issue #1301); every GM must read the same attributed answer.
        let refused_fire = MeshFrame::GmAction(crate::gm_action::GmActionFrame::Refused(
            crate::gm_action::GmActionRefusal {
                sequenced_by: HostSlot(1),
                requester: HostSlot(2),
                operator_id: "gm-1".into(),
                correlation: crate::gm_action::GmActionId::new("fire-1").unwrap(),
                action_kind: crate::gm_action::GmActionKind::EventControl,
                requested_active: true,
                tick: 419,
                reason: crate::gm_action::GmActionRefusalReason::UnknownGmEvent,
                target: Some("base-world::breach_alarm".into()),
                verb: Some(crate::gm_action::GmEventVerb::Fire),
                lever: None,
                effect_scope: None,
                objective_verb: None,
                objective_recipients: None,
                comms_recipients: None,
                observer: None,
                npc_doctrine: None,
            },
        ));
        // And a refused SKIP crosses it naming both the event and the lever
        // (issue #1304): without the lever every GM's feed would report it as a
        // refused Fire, the opposite sentence about the same button.
        let refused_skip = MeshFrame::GmAction(crate::gm_action::GmActionFrame::Refused(
            crate::gm_action::GmActionRefusal {
                sequenced_by: HostSlot(1),
                requester: HostSlot(2),
                operator_id: "gm-1".into(),
                correlation: crate::gm_action::GmActionId::new("skip-1").unwrap(),
                action_kind: crate::gm_action::GmActionKind::EventControl,
                requested_active: true,
                tick: 419,
                reason: crate::gm_action::GmActionRefusalReason::UnknownGmEvent,
                target: Some("base-world::breach_alarm".into()),
                verb: None,
                lever: Some(crate::gm_event::GmEventLever::SkipNext),
                effect_scope: None,
                objective_verb: None,
                objective_recipients: None,
                comms_recipients: None,
                observer: None,
                npc_doctrine: None,
            },
        ));
        for frame in [proposal, refusal, refused_fire, refused_skip] {
            let text = super::encode_mesh_frame(&frame).unwrap();
            assert_eq!(super::decode_mesh_frame(&text), Some(frame));
        }
    }

    /// The Skip ingress (issue #1304): its own verb on the same exact narrow
    /// shape, so a page that means to fire cannot arm a skip by getting one
    /// value wrong -- the two levers do opposite things to the same event.
    #[test]
    fn arming_a_skip_uses_its_own_verb_on_the_same_exact_typed_ingress() {
        let request = super::decode_gm_action_request(
            r#"{"operator_id":"gm-1","correlation":"skip-3","action":"arm_gm_event_skip","event":"base-world::courier_lost"}"#,
        )
        .expect("valid skip request");
        assert_eq!(
            request.action,
            crate::gm_action::GmAction::ArmGmEventSkip {
                event: "base-world::courier_lost".into(),
            }
        );
        assert_eq!(
            request.action.kind(),
            crate::gm_action::GmActionKind::EventControl,
            "one family, three levers"
        );
        assert_eq!(
            request.action.event_lever(),
            Some(crate::gm_event::GmEventLever::SkipNext),
            "and the durable result must be able to say WHICH lever"
        );
        assert_eq!(request.action.target_id(), Some("base-world::courier_lost"));
        assert_eq!(request.action.requested_pause(), None);

        for refused in [
            // An unqualified id cannot name one event across layers.
            r#"{"operator_id":"gm-1","correlation":"skip-3","action":"arm_gm_event_skip","event":"courier_lost"}"#,
            r#"{"operator_id":"gm-1","correlation":"skip-3","action":"arm_gm_event_skip","event":""}"#,
            r#"{"operator_id":"gm-1","correlation":"skip-3","action":"arm_gm_event_skip","event":"base-world::"}"#,
            // A second field is a wider mutation surface, not a Skip.
            r#"{"operator_id":"gm-1","correlation":"skip-3","action":"arm_gm_event_skip","event":"base-world::a","count":2}"#,
            r#"{"operator_id":"gm-1","correlation":"skip-3","action":"arm_gm_event_skip"}"#,
        ] {
            assert!(
                super::decode_gm_action_request(refused).is_none(),
                "must fail closed: {refused}"
            );
        }
    }

    #[test]
    fn first_time_gm_join_frames_round_trip_on_the_shared_envelope() {
        use crate::gm_join::{
            GmJoinApproval, GmJoinCandidate, GmJoinCommit, GmJoinFrame, GmJoinId, GmJoinKind,
            GmJoinRefusal,
        };

        let candidate = GmJoinCandidate {
            host: HostSlot(3),
            operator_id: "gm-2".into(),
        };
        let frames = [
            MeshFrame::GmJoin(GmJoinFrame::Pause(GmJoinApproval {
                id: GmJoinId(7),
                kind: GmJoinKind::FirstTime,
                owner: HostSlot(1),
                approved_by: HostSlot(2),
                candidate: candidate.clone(),
                apply_tick: 419,
                transfer_id: 0x1293_0000_0000_0007,
            })),
            MeshFrame::GmJoin(GmJoinFrame::Restored {
                from: HostSlot(3),
                id: GmJoinId(7),
                digest: 0xfeed_beef,
            }),
            MeshFrame::GmJoin(GmJoinFrame::RestoreBoundary {
                from: HostSlot(3),
                id: GmJoinId(7),
                boundary: 4,
            }),
            MeshFrame::GmJoin(GmJoinFrame::Committed(GmJoinCommit {
                id: GmJoinId(7),
                kind: GmJoinKind::Reconnect,
                owner: HostSlot(1),
                candidate: candidate.clone(),
                tick: 419,
                digest: 0xfeed_beef,
            })),
            MeshFrame::GmJoin(GmJoinFrame::Refused {
                from: HostSlot(1),
                id: GmJoinId(7),
                reason: GmJoinRefusal::DigestMismatch {
                    expected: 1,
                    restored: 2,
                },
            }),
        ];
        for frame in frames {
            let text = super::encode_mesh_frame(&frame).expect("encodes");
            assert!(
                text.contains(&format!("\"m\":{HOST_MESH_PROTOCOL}")),
                "the revision travels: {text}"
            );
            assert!(text.contains("\"t\":\"gm-join\""), "{text}");
            assert_eq!(super::decode_mesh_frame(&text), Some(frame));
        }
    }

    #[test]
    fn the_gm_action_ingress_is_one_exact_bounded_typed_shape() {
        let request = super::decode_gm_action_request(
            r#"{"operator_id":"gm-1","correlation":"pause-17","action":"set_session_paused","active":true}"#,
        )
        .expect("valid request");
        assert_eq!(request.operator_id, "gm-1");
        assert_eq!(request.correlation.as_str(), "pause-17");
        assert_eq!(request.action.requested_pause(), Some(true));

        for refused in [
            r#"{"operator_id":"","correlation":"pause-17","action":"set_session_paused","active":true}"#,
            r#"{"operator_id":"gm-1","correlation":"bad id","action":"set_session_paused","active":true}"#,
            r#"{"operator_id":"gm-1","correlation":"pause-17","action":"toggle_pause","active":true}"#,
            r#"{"operator_id":"gm-1","correlation":"pause-17","action":"set_session_paused","active":true,"component":"Transform"}"#,
        ] {
            assert!(
                super::decode_gm_action_request(refused).is_none(),
                "must fail closed: {refused}"
            );
        }
    }

    #[test]
    fn station_takeover_and_existing_system_commands_share_the_exact_typed_ingress() {
        let takeover = super::decode_gm_action_request(
            r#"{"operator_id":"gm-1","correlation":"take-17","action":"set_station_puppet","ship":"ship-1","station":"captain","active":true}"#,
        )
        .expect("valid takeover request");
        assert_eq!(
            takeover.action,
            crate::gm_action::GmAction::SetStationPuppet {
                ship: crate::command_admission::log::ShipKey("ship-1".into()),
                station: crate::core::messages::StationId("captain".into()),
                active: true,
            }
        );

        let command = super::decode_gm_action_request(
            r#"{"operator_id":"gm-1","correlation":"command-17","action":"issue_station_command","ship":"ship-1","station":"captain","target":"red-alert","payload":{"type":"SetRedAlert","data":{"active":true}}}"#,
        )
        .expect("valid existing System command");
        let crate::gm_action::GmAction::IssueStationCommand { payload, .. } = command.action else {
            panic!("decoded the wrong typed action")
        };
        assert_eq!(
            super::decode_canonical_system_command(payload.as_str()),
            Some(crate::core::messages::SystemControlPayload::SetRedAlert { active: true })
        );
        assert!(
            super::decode_canonical_system_command(
                r#"{ "type":"SetRedAlert", "data":{"active":true} }"#
            )
            .is_none(),
            "only the canonical command bytes replay"
        );

        for refused in [
            r#"{"operator_id":"gm-1","correlation":"take-17","action":"set_station_puppet","ship":"ship-1","station":"captain","active":true,"authority":"human"}"#,
            r#"{"operator_id":"gm-1","correlation":"command-17","action":"issue_station_command","ship":"ship-1","station":"captain","target":"red-alert","payload":{"type":"NotACommand"}}"#,
        ] {
            assert!(
                super::decode_gm_action_request(refused).is_none(),
                "must fail closed: {refused}"
            );
        }
    }

    /// The Fire ingress (issue #1301) shares the exact same narrow shape: one
    /// stable layer-qualified event id, nothing else, and a per-shape field
    /// count so a second target cannot ride along.
    #[test]
    fn firing_an_authored_gm_event_uses_the_same_exact_typed_ingress() {
        let request = super::decode_gm_action_request(
            r#"{"operator_id":"gm-1","correlation":"fire-3","action":"fire_gm_event","event":"base-world::breach_alarm"}"#,
        )
        .expect("valid fire request");
        assert_eq!(
            request.action,
            crate::gm_action::GmAction::FireGmEvent {
                event: "base-world::breach_alarm".into(),
            }
        );
        assert_eq!(
            request.action.kind(),
            crate::gm_action::GmActionKind::EventControl
        );
        assert_eq!(
            request.action.target_id(),
            Some("base-world::breach_alarm"),
            "the durable result must be able to say WHICH event was fired"
        );
        assert_eq!(request.action.requested_pause(), None);

        for refused in [
            // An unqualified id cannot name one event across layers.
            r#"{"operator_id":"gm-1","correlation":"fire-3","action":"fire_gm_event","event":"breach_alarm"}"#,
            r#"{"operator_id":"gm-1","correlation":"fire-3","action":"fire_gm_event","event":""}"#,
            r#"{"operator_id":"gm-1","correlation":"fire-3","action":"fire_gm_event","event":"base-world::"}"#,
            // A second field is a wider mutation surface, not a Fire.
            r#"{"operator_id":"gm-1","correlation":"fire-3","action":"fire_gm_event","event":"base-world::a","handler":"anything"}"#,
            r#"{"operator_id":"gm-1","correlation":"fire-3","action":"fire_gm_event"}"#,
        ] {
            assert!(
                super::decode_gm_action_request(refused).is_none(),
                "must fail closed: {refused}"
            );
        }
    }

    /// Issue #1303: pausing an authored event crosses the same narrow typed
    /// ingress, carrying the ABSOLUTE state rather than a toggle.
    #[test]
    fn pausing_an_authored_gm_event_uses_the_same_exact_typed_ingress() {
        for (raw, active) in [
            (
                r#"{"operator_id":"gm-1","correlation":"pause-3","action":"set_event_paused","event":"base-world::breach_alarm","active":true}"#,
                true,
            ),
            (
                r#"{"operator_id":"gm-1","correlation":"resume-3","action":"set_event_paused","event":"base-world::breach_alarm","active":false}"#,
                false,
            ),
        ] {
            let request = super::decode_gm_action_request(raw).expect("valid pause request");
            assert_eq!(
                request.action,
                crate::gm_action::GmAction::SetEventPaused {
                    event: "base-world::breach_alarm".into(),
                    active,
                }
            );
            assert_eq!(
                request.action.kind(),
                crate::gm_action::GmActionKind::EventControl,
                "Pause routes to the mission surface beside Fire"
            );
            assert_eq!(
                request.action.verb(),
                Some(crate::gm_action::GmEventVerb::Pause),
                "and the durable result must be able to say WHICH lever it was"
            );
            assert_eq!(request.action.target_id(), Some("base-world::breach_alarm"));
            assert_eq!(
                request.action.requested_pause(),
                None,
                "a paused EVENT is not a paused session"
            );
            assert_eq!(request.action.requested_active(), active);
        }

        for refused in [
            // An unqualified id cannot name one event across layers.
            r#"{"operator_id":"gm-1","correlation":"pause-3","action":"set_event_paused","event":"breach_alarm","active":true}"#,
            r#"{"operator_id":"gm-1","correlation":"pause-3","action":"set_event_paused","event":"","active":true}"#,
            r#"{"operator_id":"gm-1","correlation":"pause-3","action":"set_event_paused","event":"base-world::","active":true}"#,
            // Absolute state only: a toggle would depend on arrival order.
            r#"{"operator_id":"gm-1","correlation":"pause-3","action":"set_event_paused","event":"base-world::a"}"#,
            r#"{"operator_id":"gm-1","correlation":"pause-3","action":"set_event_paused","event":"base-world::a","active":"yes"}"#,
            // A second target is a wider mutation surface, not a Pause.
            r#"{"operator_id":"gm-1","correlation":"pause-3","action":"set_event_paused","event":"base-world::a","active":true,"ship":"player-1"}"#,
            r#"{"operator_id":"gm-1","correlation":"pause-3","action":"set_event_paused"}"#,
        ] {
            assert!(
                super::decode_gm_action_request(refused).is_none(),
                "must fail closed: {refused}"
            );
        }
    }

    /// A refused Pause replicates as a refused PAUSE (issue #1303): the lever
    /// crosses the owner-decision lane beside the event id, because a frame
    /// carrying only the id could be republished on every other GM's feed as a
    /// refused Fire of that same event.
    #[test]
    fn a_replicated_event_control_refusal_carries_its_lever() {
        let refusal = MeshFrame::GmAction(crate::gm_action::GmActionFrame::Refused(
            crate::gm_action::GmActionRefusal {
                sequenced_by: HostSlot(1),
                requester: HostSlot(2),
                operator_id: "gm-1".into(),
                correlation: crate::gm_action::GmActionId::new("pause-1").unwrap(),
                action_kind: crate::gm_action::GmActionKind::EventControl,
                requested_active: false,
                tick: 419,
                reason: crate::gm_action::GmActionRefusalReason::UnknownGmEvent,
                target: Some("base-world::breach_alarm".into()),
                verb: Some(crate::gm_action::GmEventVerb::Pause),
                lever: None,
                effect_scope: None,
                objective_verb: None,
                objective_recipients: None,
                comms_recipients: None,
                observer: None,
                npc_doctrine: None,
            },
        ));
        let text = super::encode_mesh_frame(&refusal).expect("encodes");
        assert!(text.contains(r#""verb":"pause""#), "{text}");
        assert_eq!(super::decode_mesh_frame(&text), Some(refusal));

        // The same frame from a peer that predates the lever: the key is simply
        // absent, and it still decodes rather than being dropped on the floor.
        // `validate_fleet_frame` is what then refuses it, rather than this
        // ingress guessing Fire.
        let legacy = text.replace(r#","verb":"pause""#, "");
        let legacy_value: serde_json::Value = serde_json::from_str(&legacy).unwrap();
        assert!(legacy_value["d"].get("verb").is_none(), "{legacy}");
        let Some(MeshFrame::GmAction(crate::gm_action::GmActionFrame::Refused(decoded))) =
            super::decode_mesh_frame(&legacy)
        else {
            panic!("a legacy refusal still decodes")
        };
        assert_eq!(decoded.verb, None);
        assert_eq!(decoded.target.as_deref(), Some("base-world::breach_alarm"));
    }

    /// No session token can reach this wire, because the type it projects from
    /// carries none. Asserted at the encoding site because this is the moment
    /// the frame becomes bytes on a socket.
    #[test]
    fn the_wire_carries_no_session_token() {
        let text = super::encode_mesh_frame(&tick_frame()).expect("encodes");
        assert!(!text.contains("response_token"), "{text}");
        assert!(!text.contains("token"), "{text}");
    }

    /// The direct-effect ingress (issue #1310): one entity identity, one kind,
    /// one positive integer amount in milli-HP, and a per-shape field count so
    /// nothing wider can ride in behind it.
    #[test]
    fn a_direct_effect_uses_the_same_exact_typed_ingress() {
        let request = super::decode_gm_action_request(
            r#"{"operator_id":"gm-1","correlation":"hit-4","action":"apply_direct_effect","entity":"npc-1","effect":"damage","amount_milli_hp":25000,"scope":"entity","scope_id":null}"#,
        )
        .expect("valid direct-effect request");
        assert_eq!(
            request.action,
            crate::gm_action::GmAction::ApplyDirectEffect {
                target: "npc-1".into(),
                scope: crate::gm_effect::GmDirectEffectScope::Entity,
                effect: crate::gm_effect::GmDirectEffectKind::Damage,
                amount_milli_hp: 25_000,
            }
        );
        assert_eq!(
            request.action.kind(),
            crate::gm_action::GmActionKind::DirectEffect
        );
        assert_eq!(
            request.action.target_id(),
            Some("npc-1"),
            "the durable result must be able to say WHAT was hit"
        );

        let heal = super::decode_gm_action_request(
            r#"{"operator_id":"gm-1","correlation":"heal-4","action":"apply_direct_effect","entity":"npc-1","effect":"heal","amount_milli_hp":1,"scope":"entity","scope_id":null}"#,
        )
        .expect("valid heal request");
        assert!(matches!(
            heal.action,
            crate::gm_action::GmAction::ApplyDirectEffect {
                effect: crate::gm_effect::GmDirectEffectKind::Heal,
                ..
            }
        ));

        // The narrowed scopes (issue #1311) arrive through the SAME arm, as a
        // discriminant plus one ship-local authoring key. Nothing else about
        // the request changes shape, which is the point of scope being a field
        // of one mechanic rather than a second action.
        let station = super::decode_gm_action_request(
            r#"{"operator_id":"gm-1","correlation":"hit-5","action":"apply_direct_effect","entity":"npc-1","effect":"damage","amount_milli_hp":25000,"scope":"station","scope_id":"helm"}"#,
        )
        .expect("valid Station-scoped request");
        assert_eq!(
            station.action,
            crate::gm_action::GmAction::ApplyDirectEffect {
                target: "npc-1".into(),
                scope: crate::gm_effect::GmDirectEffectScope::Station(
                    crate::core::messages::StationId("helm".into())
                ),
                effect: crate::gm_effect::GmDirectEffectKind::Damage,
                amount_milli_hp: 25_000,
            }
        );
        let system = super::decode_gm_action_request(
            r#"{"operator_id":"gm-1","correlation":"heal-5","action":"apply_direct_effect","entity":"npc-1","effect":"heal","amount_milli_hp":4000,"scope":"system","scope_id":"impulse-drive"}"#,
        )
        .expect("valid System-scoped request");
        assert_eq!(
            system.action,
            crate::gm_action::GmAction::ApplyDirectEffect {
                target: "npc-1".into(),
                scope: crate::gm_effect::GmDirectEffectScope::System(
                    crate::core::messages::SystemId("impulse-drive".into())
                ),
                effect: crate::gm_effect::GmDirectEffectKind::Heal,
                amount_milli_hp: 4_000,
            }
        );

        for refused in [
            // A signed amount is not the vocabulary: the sign is the kind.
            r#"{"operator_id":"gm-1","correlation":"x","action":"apply_direct_effect","entity":"npc-1","effect":"damage","amount_milli_hp":-5,"scope":"entity","scope_id":null}"#,
            r#"{"operator_id":"gm-1","correlation":"x","action":"apply_direct_effect","entity":"npc-1","effect":"damage","amount_milli_hp":0,"scope":"entity","scope_id":null}"#,
            r#"{"operator_id":"gm-1","correlation":"x","action":"apply_direct_effect","entity":"npc-1","effect":"damage","amount_milli_hp":2.5,"scope":"entity","scope_id":null}"#,
            r#"{"operator_id":"gm-1","correlation":"x","action":"apply_direct_effect","entity":"","effect":"damage","amount_milli_hp":5,"scope":"entity","scope_id":null}"#,
            r#"{"operator_id":"gm-1","correlation":"x","action":"apply_direct_effect","entity":"npc-1","effect":"vaporise","amount_milli_hp":5,"scope":"entity","scope_id":null}"#,
            // A scope this build does not implement cannot be named from here.
            r#"{"operator_id":"gm-1","correlation":"x","action":"apply_direct_effect","entity":"npc-1","effect":"damage","amount_milli_hp":5,"scope":"deck","scope_id":"a"}"#,
            // A narrowed scope with no id names nothing, and a whole-entity one
            // carrying an id is a request the ingress would have to reinterpret.
            r#"{"operator_id":"gm-1","correlation":"x","action":"apply_direct_effect","entity":"npc-1","effect":"damage","amount_milli_hp":5,"scope":"station","scope_id":null}"#,
            r#"{"operator_id":"gm-1","correlation":"x","action":"apply_direct_effect","entity":"npc-1","effect":"damage","amount_milli_hp":5,"scope":"system","scope_id":""}"#,
            r#"{"operator_id":"gm-1","correlation":"x","action":"apply_direct_effect","entity":"npc-1","effect":"damage","amount_milli_hp":5,"scope":"entity","scope_id":"helm"}"#,
            // The field-count guard stays exact in both directions.
            r#"{"operator_id":"gm-1","correlation":"x","action":"apply_direct_effect","entity":"npc-1","effect":"damage","amount_milli_hp":5}"#,
            r#"{"operator_id":"gm-1","correlation":"x","action":"apply_direct_effect","entity":"npc-1","effect":"damage","amount_milli_hp":5,"scope":"entity","scope_id":null,"station":"helm"}"#,
            r#"{"operator_id":"gm-1","correlation":"x","action":"apply_direct_effect","entity":"npc-1","effect":"damage"}"#,
        ] {
            assert!(
                super::decode_gm_action_request(refused).is_none(),
                "must fail closed: {refused}"
            );
        }
    }
}
