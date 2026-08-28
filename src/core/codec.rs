use crate::core::messages::{ClientMessage, ServerMessage};

pub trait MessageCodec {
    type Error;
    fn encode_client(&self, msg: &ClientMessage) -> Result<String, Self::Error>;
    fn decode_client(&self, s: &str) -> Result<ClientMessage, Self::Error>;
    fn encode_server(&self, msg: &ServerMessage) -> Result<String, Self::Error>;
    fn decode_server(&self, s: &str) -> Result<ServerMessage, Self::Error>;
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
// Field NAMES for the catalogue entries come from `delivery::payload`, never
// from this file — that is the whole point of that module's ordered entry
// lists, and it is why a new catalogue field cannot reach the browser surface
// while skipping the native one.

fn stamp_json(stamp: &crate::delivery::stamp::DeliveryStamp) -> serde_json::Value {
    serde_json::json!({
        "protocol": stamp.protocol,
        "content_id": stamp.content_id,
        "content_epoch": stamp.content_epoch,
    })
}

fn payload_value_json(value: &crate::delivery::payload::PayloadValue) -> serde_json::Value {
    use crate::delivery::payload::PayloadValue;
    match value {
        PayloadValue::Text(s) => serde_json::Value::String(s.clone()),
        PayloadValue::Number(n) => serde_json::Number::from_f64(*n)
            .map(serde_json::Value::Number)
            .unwrap_or(serde_json::Value::Null),
    }
}

fn ship_json(ship: &crate::delivery::payload::ShipPayload) -> serde_json::Value {
    let mut obj = serde_json::Map::new();
    for (key, value) in ship.entries() {
        obj.insert((*key).to_string(), payload_value_json(value));
    }
    serde_json::Value::Object(obj)
}

fn scenario_json(scenario: &crate::delivery::payload::ScenarioPayload) -> serde_json::Value {
    let mut obj = serde_json::Map::new();
    for (key, value) in scenario.entries() {
        obj.insert((*key).to_string(), payload_value_json(value));
    }
    obj.insert(
        crate::delivery::payload::SHIPS_KEY.to_string(),
        serde_json::Value::Array(scenario.ships().iter().map(ship_json).collect()),
    );
    serde_json::Value::Object(obj)
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
        "scenarios": manifest
            .scenarios
            .iter()
            .map(scenario_json)
            .collect::<Vec<_>>(),
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
    use crate::lockstep::{DigestFrame, MeshCommand, MeshFrame, TickFrame};

    let value: serde_json::Value = serde_json::from_str(raw).ok()?;
    if value.get(MESH_ENVELOPE_PROTOCOL)?.as_u64()? != u64::from(crate::lockstep::HOST_MESH_PROTOCOL)
    {
        return None;
    }
    let body = value.get(MESH_ENVELOPE_BODY)?;
    let from = HostSlot(u32::try_from(body.get("from")?.as_u64()?).ok()?);
    let tick = body.get("tick")?.as_u64()?;
    match value.get(MESH_ENVELOPE_TYPE)?.as_str()? {
        crate::lockstep::frame::TYPE_TICK => {
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
            }))
        }
        crate::lockstep::frame::TYPE_DIGEST => Some(MeshFrame::Digest(DigestFrame {
            from,
            tick,
            digest: u64::from_str_radix(body.get("digest")?.as_str()?, 16).ok()?,
        })),
        _ => None,
    }
}

#[cfg(test)]
mod mesh_frame_tests {
    use crate::command_admission::{CommandOrder, HostSlot, ShipKey};
    use crate::core::messages::{SystemControlPayload, SystemId};
    use crate::lockstep::{DigestFrame, MeshCommand, MeshFrame, TickFrame};

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
        })
    }

    /// The wire shape is the envelope `gui/host-mesh.js` owns, and a frame
    /// survives it unchanged.
    #[test]
    fn a_tick_frame_round_trips_through_the_shared_envelope() {
        let frame = tick_frame();
        let text = super::encode_mesh_frame(&frame).expect("encodes");
        assert!(text.contains("\"m\":2"), "the revision travels: {text}");
        assert!(text.contains("\"t\":\"tick\""), "{text}");
        assert!(
            text.contains("\"tick\":412"),
            "the envelope's own tick stamp — the field #1114 added for exactly \
             this — must carry the tick the frame applies at: {text}"
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

    /// Everything that is not a simulation frame of a revision this build
    /// speaks answers `None`, together — the same discipline
    /// `decodeHostFrame` keeps on the other side of the wire.
    #[test]
    fn anything_that_is_not_a_frame_of_this_revision_is_refused() {
        for raw in [
            "not json at all",
            r#"{"type":"Identify","token":"abc"}"#,
            r#"{"m":1,"t":"tick","tick":1,"d":{"from":1,"tick":1,"ready_through":1,"commands":[]}}"#,
            r#"{"m":2,"t":"hello","tick":null,"d":{}}"#,
            r#"{"m":2,"t":"tick","tick":1,"d":{"from":1,"tick":1}}"#,
            r#"{"m":2,"t":"digest","tick":1,"d":{"from":1,"tick":1,"digest":12345}}"#,
        ] {
            assert_eq!(
                super::decode_mesh_frame(raw),
                None,
                "must be refused rather than half understood: {raw}"
            );
        }
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
}
