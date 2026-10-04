use crate::core::messages::{ClientMessage, ServerMessage};

/// Serialize ordinary JSON at the repository codec seam.
pub fn to_json<T: serde::Serialize + ?Sized>(value: &T) -> Result<String, serde_json::Error> {
    serde_json::to_string(value)
}

/// Decode ordinary JSON text at the repository codec seam.
pub fn from_json<T: serde::de::DeserializeOwned>(text: &str) -> Result<T, serde_json::Error> {
    serde_json::from_str(text)
}

/// Decode ordinary JSON bytes at the repository codec seam.
pub fn from_json_bytes<T: serde::de::DeserializeOwned>(
    bytes: &[u8],
) -> Result<T, serde_json::Error> {
    serde_json::from_slice(bytes)
}

#[cfg(all(feature = "server", not(target_arch = "wasm32")))]
pub fn encode_workshop_key(key: &crate::native_host::workshop::keyboard::WorkshopKey) -> String {
    serde_json::to_string(key).expect("local Authoring keys serialize")
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

impl JsonCodec {
    pub fn encode_client(&self, msg: &ClientMessage) -> Result<String, serde_json::Error> {
        serde_json::to_string(msg)
    }

    pub fn decode_client(&self, s: &str) -> Result<ClientMessage, serde_json::Error> {
        serde_json::from_str(s)
    }

    pub fn encode_server(&self, msg: &ServerMessage) -> Result<String, serde_json::Error> {
        serde_json::to_string(msg)
    }

    pub fn decode_server(&self, s: &str) -> Result<ServerMessage, serde_json::Error> {
        serde_json::from_str(s)
    }
}

// ── HTML console bridge (de)serialisation (ADR-0001 / PRD #419) ────────────
//
// Ordinary host-channel pushes (HUD, lobby, chatter, audio) use `to_json`;
// inbound `ClientMessage` decoding retains its named protocol entry point.
// Bridge / plugin code calls this module, never `serde_json` directly.

/// The room's immutable authored cue definitions travel with its normal config,
/// so providers can prepare assets before a live occurrence without queuing it.
pub fn encode_room_audio_config(
    p: &crate::audio_config::AudioConfigPayload,
    catalog: Option<&crate::gm_presentation::sound::LiveSoundCatalog>,
) -> Result<String, serde_json::Error> {
    #[derive(serde::Serialize)]
    struct Payload<'a> {
        #[serde(flatten)]
        audio: &'a crate::audio_config::AudioConfigPayload,
        #[serde(skip_serializing_if = "Vec::is_empty")]
        authored_sounds: Vec<crate::sound_cues::SoundDefinition>,
    }
    serde_json::to_string(&Payload {
        audio: p,
        authored_sounds: catalog.map_or_else(Vec::new, |catalog| catalog.room()),
    })
}

#[cfg(test)]
pub fn workshop_slot_edit_fixtures() -> Vec<(String, crate::workshop::document::EditRequest)> {
    #[derive(serde::Deserialize)]
    struct Fixture {
        name: String,
        source: String,
        edits: Vec<crate::workshop::document::Edit>,
    }
    let fixtures: Vec<Fixture> = serde_json::from_str(include_str!(
        "../../tests/fixtures/workshop-slot-edits.json"
    ))
    .expect("valid shared Workshop slot fixtures");
    fixtures
        .into_iter()
        .map(|fixture| {
            (
                fixture.name,
                crate::workshop::document::EditRequest {
                    document_path: "assets/worlds/mission.toml".into(),
                    expected_source: fixture.source,
                    edits: fixture.edits,
                },
            )
        })
        .collect()
}

#[cfg(all(feature = "server", not(target_arch = "wasm32")))]
pub fn encode_native_audio_visual(
    cue: &crate::native_host::audio::visual::VisualCue,
) -> Result<String, serde_json::Error> {
    serde_json::to_string(cue)
        .map(|json| vellum_ultralight::bridge::push_call("window.__phoenixHudAudioCue", &json))
}

#[cfg(all(feature = "server", not(target_arch = "wasm32")))]
pub(crate) fn is_native_gm_profile_record(json: &str) -> Option<bool> {
    let value: serde_json::Value = serde_json::from_str(json).ok()?;
    (value["type"] == "NativeOperator")
        .then(|| matches!(value["operation"].as_str(), Some("load" | "save")))
}

/// Private host-page presentation request, not a participant message.
pub fn decode_gm_inspector_interest(
    json: &str,
) -> Option<Vec<crate::gm_projection::GmInspectorKind>> {
    if json.len() > 256 {
        return None;
    }
    serde_json::from_str(json).ok()
}

pub fn decode_gm_console_interest(json: &str) -> Option<crate::gm_projection::GmConsoleInterest> {
    if json.len() > 1024 {
        return None;
    }
    let request: crate::gm_projection::GmConsoleInterest = serde_json::from_str(json).ok()?;
    request.valid().then_some(request)
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

// ── The native host's lobby surface (issues #1328/#1330) ────────────────────
//
// That surface takes one more push beside the lobby state — the bridge's
// monitor roster — and answers with the operator's own presses: a scenario, a
// hull, an AI launch, a monitor for the viewscreen. Both directions cross as
// JSON through this codec seam (AGENTS.md Key Constraint 1);
// the types stay pure and Bevy-free in `native_host::host_lobby::{layout,
// scenario}`.
//
// One decode for all four verbs, because the record queue they share is a drain
// with exactly one reader — see `HostLobbyRecord`'s note on why a second record
// type would be a queue two systems fight over.
//
// Gated on the same cfg `crate::native_host` itself carries: a browser host has
// no monitors to offer, and on wasm the module these name does not exist.

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

/// Encode named Objective-instance lifecycle/report state at the one JSON
/// boundary used by headless and native diagnostics.
pub fn encode_objective_instances(
    manager: &crate::objective_instances::ObjectiveInstanceManager,
) -> String {
    serde_json::to_string(manager).unwrap_or_else(|_| r#"{"instances":[],"history":{}}"#.into())
}

/// Encode a host's own version stamp — the body of `/host/stamp.json`.
pub fn encode_delivery_stamp(stamp: &crate::delivery::stamp::DeliveryStamp) -> String {
    stamp_json(stamp).to_string()
}

/// Advertise the native delivery owner's asset revision without losing u64
/// precision in a pure-JS consumer. This is local delivery metadata, not a
/// simulation message or a cue record.
pub fn encode_native_asset_revision(revision: u64) -> String {
    serde_json::json!({
        "capability": "phoenix-native-asset-revision", "version": 1,
        "revision": revision.to_string(),
    })
    .to_string()
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

mod wire {
    //! Private wire projection: keep JSON shape separate from durable typed frames.

    use serde::de::DeserializeOwned;
    use serde_json::{Map, Value};

    use crate::lockstep::MeshFrame;

    type Object = Map<String, Value>;

    fn decode<T: DeserializeOwned>(value: Value) -> Option<T> {
        serde_json::from_value(value).ok()
    }

    fn project(body: &Value, required: &str, optional: &str) -> Option<Object> {
        let mut object = Object::new();
        for key in required.split_whitespace() {
            object.insert(key.into(), body.get(key)?.clone());
        }
        for key in optional.split_whitespace() {
            if let Some(value) = body.get(key) {
                object.insert(key.into(), value.clone());
            }
        }
        Some(object)
    }

    fn rename(object: &mut Object, from: &str, to: &str) -> Option<()> {
        let value = object.remove(from)?;
        object.insert(to.into(), value);
        Some(())
    }

    fn unhex(object: &mut Object, keys: &str) -> Option<()> {
        for key in keys.split_whitespace() {
            let value = u64::from_str_radix(object.get(key)?.as_str()?, 16).ok()?;
            object.insert(key.into(), value.into());
        }
        Some(())
    }

    fn hex(object: &mut Object, keys: &str) {
        for key in keys.split_whitespace() {
            let value = object[key].as_u64().expect("typed hexadecimal field");
            object.insert(key.into(), format!("{value:016x}").into());
        }
    }

    fn tagged(tag: &str, object: Object) -> Value {
        let mut value = Object::new();
        value.insert(tag.into(), Value::Object(object));
        Value::Object(value)
    }

    pub(super) fn decode_gm_request(raw: &str) -> Option<crate::gm_action::GmActionRequest> {
        let value: Value = serde_json::from_str(raw).ok()?;
        let mut object = value.as_object()?.clone();
        let operator_id = object.remove("operator_id")?.as_str()?.to_owned();
        let correlation = decode(object.remove("correlation")?)?;
        if operator_id.is_empty()
            || operator_id.chars().count() > crate::gm_roster::MAX_GM_OPERATOR_ID_CHARS
        {
            return None;
        }
        let kind = object.remove("action")?.as_str()?.to_owned();
        // Exact flat ingress shapes, including required nullable keys. Nested domain
        // values keep their existing Serde rules; there is no flatten/deny-unknown mix.
        let (keys, bounded) = match kind.as_str() {
            "set_session_paused" => ("active", ""),
            "set_station_puppet" => ("ship station active", "ship station"),
            "fire_gm_event" | "arm_gm_event_skip" => ("event", "event"),
            "apply_direct_effect" => ("entity effect amount_milli_hp scope scope_id", "entity"),
            "objective_instance_action" => ("objective scope verb", "objective"),
            "objective_action" => ("objective verb recipients", "objective"),
            "presentation" => ("ship cue", "ship"),
            "set_contact_information" => ("ship change", "ship"),
            "set_contact_classification" => ("ship target palette", "ship target palette"),
            // This older ingress intentionally accepts raw identities.
            "set_contact_override" => ("ship target mode", ""),
            "set_system_disabled" => ("target system disabled", "target system"),
            "transmit_comms" => ("transmission", ""),
            "set_npc_doctrine" => ("target doctrine", "target doctrine"),
            "set_npc_doctrine_checked" => ("target doctrine expected_revision", "target doctrine"),
            "set_faction_hostility" => ("faction enemy hostile", "faction enemy"),
            "undo_gm_action" => ("original original_operator original_sequence expected", ""),
            "despawn_entity" => ("target", "target"),
            "request_live_restore" => ("candidate", "candidate"),
            "backfill_ship_slot" => ("slot", "slot"),
            "spawn_palette_entity" => (
                "palette variant position_mm heading_mdeg",
                "palette variant",
            ),
            "set_event_paused" => ("event active", "event"),
            "issue_station_command" => ("ship station target payload", "ship station target"),
            _ => return None,
        };
        if object.len() != keys.split_whitespace().count()
            || keys.split_whitespace().any(|key| !object.contains_key(key))
        {
            return None;
        }
        for key in bounded.split_whitespace() {
            let value = object.get(key)?;
            if value.is_null()
                && matches!(
                    (kind.as_str(), key),
                    ("set_contact_classification", "palette") | ("spawn_palette_entity", "variant")
                )
            {
                continue;
            }
            if key == "event" {
                super::bounded_gm_event_id(value.as_str()?)?;
            } else {
                super::bounded_gm_target_id(value.as_str()?)?;
            }
        }
        match kind.as_str() {
            "apply_direct_effect" => {
                rename(&mut object, "entity", "target")?;
                let scope_id = object.remove("scope_id")?;
                let scope = match object.get("scope")?.as_str()? {
                    "entity" if scope_id.is_null() => crate::gm_effect::GmDirectEffectScope::Entity,
                    "station" => crate::gm_effect::GmDirectEffectScope::Station(
                        crate::core::messages::StationId(super::bounded_gm_target_id(
                            scope_id.as_str()?,
                        )?),
                    ),
                    "system" => crate::gm_effect::GmDirectEffectScope::System(
                        crate::core::messages::SystemId(super::bounded_gm_target_id(
                            scope_id.as_str()?,
                        )?),
                    ),
                    _ => return None,
                };
                let amount: u32 = decode(object.get("amount_milli_hp")?.clone())?;
                if amount == 0 {
                    return None;
                }
                object.insert("scope".into(), serde_json::to_value(scope).ok()?);
            }
            "issue_station_command" => {
                let payload = decode(object.remove("payload")?)?;
                object.insert(
                    "payload".into(),
                    serde_json::to_value(super::canonical_system_command(&payload)?).ok()?,
                );
            }
            _ => {}
        }
        let action: crate::gm_action::GmAction = decode(tagged(&kind, object))?;
        // Only these ingress families performed domain validation before sequencing.
        match &action {
            crate::gm_action::GmAction::ObjectiveInstanceAction { .. }
            | crate::gm_action::GmAction::ObjectiveAction { .. }
            | crate::gm_action::GmAction::Presentation { .. }
            | crate::gm_action::GmAction::SetContactInformation { .. }
            | crate::gm_action::GmAction::SetNpcDoctrineChecked { .. }
            | crate::gm_action::GmAction::SetFactionHostility { .. }
            | crate::gm_action::GmAction::UndoGmAction { .. } => action.validate().ok()?,
            crate::gm_action::GmAction::TransmitComms { transmission }
                if !transmission.valid_shape() =>
            {
                return None;
            }
            crate::gm_action::GmAction::SpawnPaletteEntity {
                position_mm,
                heading_mdeg,
                ..
            } if !crate::gm_spawn::placement_is_valid(*position_mm, *heading_mdeg) => {
                return None;
            }
            _ => {}
        }
        Some(crate::gm_action::GmActionRequest {
            operator_id,
            correlation,
            action,
        })
    }

    fn nest(object: &mut Object, key: &str, fields: &[(&str, &str)]) -> Option<()> {
        let mut nested = Object::new();
        for (flat, inner) in fields {
            nested.insert((*inner).into(), object.remove(*flat)?);
        }
        object.insert(key.into(), Value::Object(nested));
        Some(())
    }

    fn flatten(object: &mut Object, key: &str, fields: &[(&str, &str)]) {
        let nested = object.remove(key).expect("typed nested field");
        for (flat, inner) in fields {
            object.insert((*flat).into(), nested[*inner].clone());
        }
    }

    pub(super) fn encode_mesh(frame: &MeshFrame) -> Result<String, serde_json::Error> {
        let encoded = serde_json::to_value(frame)?;
        let (_, body) = encoded
            .as_object()
            .expect("typed mesh enum")
            .iter()
            .next()
            .unwrap();
        let mut body = body.clone();
        let mut kind = None;
        if matches!(
            frame,
            MeshFrame::GmAction(_) | MeshFrame::GmJoin(_) | MeshFrame::GmRestore(_)
        ) {
            let (variant, nested) = body
                .as_object()
                .expect("typed frame enum")
                .iter()
                .next()
                .unwrap();
            kind = Some(variant.clone());
            body = nested.clone();
        }
        let body = body.as_object_mut().expect("typed frame body");
        let tick = match frame {
            MeshFrame::Tick(frame) => {
                for command in body.get_mut("commands").unwrap().as_array_mut().unwrap() {
                    flatten(
                        command.as_object_mut().unwrap(),
                        "order",
                        &[("origin", "origin"), ("seq", "seq")],
                    );
                }
                frame.tick
            }
            MeshFrame::Digest(frame) => {
                hex(body, "digest");
                frame.tick
            }
            MeshFrame::Snapshot(frame) => {
                hex(body, "transfer_id whole_hash");
                frame.tick
            }
            MeshFrame::HostLoss(frame) => frame.tick,
            MeshFrame::SlotClaim(frame) => frame.tick,
            MeshFrame::GmAction(frame) => {
                match frame {
                    crate::gm_action::GmActionFrame::Proposal(_) => {}
                    crate::gm_action::GmActionFrame::Granted(_) => {
                        rename(body, "from", "requester").unwrap();
                        rename(body, "sequenced_by", "from").unwrap();
                        flatten(body, "order", &[("sequence", "sequence")]);
                    }
                    crate::gm_action::GmActionFrame::Refused(_) => {
                        rename(body, "sequenced_by", "from").unwrap();
                        body.remove("objective_instance_scope");
                        for key in "target verb lever effect_scope objective_verb objective_recipients observer".split_whitespace() {
                            body.entry(key).or_insert(Value::Null);
                        }
                    }
                }
                frame.tick()
            }
            MeshFrame::GmJoin(frame) => {
                rename(body, "id", "join_id").unwrap();
                if matches!(
                    frame,
                    crate::gm_join::GmJoinFrame::Pause(_)
                        | crate::gm_join::GmJoinFrame::Committed(_)
                ) {
                    rename(body, "owner", "from").unwrap();
                    rename(body, "kind", "join_kind").unwrap();
                    flatten(
                        body,
                        "candidate",
                        &[("candidate", "host"), ("operator_id", "operator_id")],
                    );
                }
                if matches!(frame, crate::gm_join::GmJoinFrame::Pause(_)) {
                    hex(body, "transfer_id");
                }
                if matches!(
                    frame,
                    crate::gm_join::GmJoinFrame::Restored { .. }
                        | crate::gm_join::GmJoinFrame::Committed(_)
                ) {
                    hex(body, "digest");
                }
                frame.tick()
            }
            MeshFrame::GmRestore(frame) => {
                flatten(
                    body,
                    "restore",
                    &[
                        ("restore_origin", "origin"),
                        ("restore_sequence", "sequence"),
                    ],
                );
                if matches!(frame, crate::gm_restore::GmRestoreFrame::Loaded { .. }) {
                    hex(body, "digest");
                }
                if body.get("failure").is_some_and(Value::is_null) {
                    body.remove("failure");
                }
                match frame {
                    crate::gm_restore::GmRestoreFrame::Loaded { tick, .. } => *tick,
                    _ => 0,
                }
            }
        };
        body.insert("tick".into(), tick.into());
        if let Some(kind) = kind {
            let kind = match kind.as_str() {
                "RestoreBoundary" => "restore-boundary".into(),
                _ => kind.to_lowercase(),
            };
            body.insert("kind".into(), kind.into());
        }
        // Value retains the existing map ordering; direct struct serialization would not.
        Ok(serde_json::json!({"m": crate::lockstep::HOST_MESH_PROTOCOL, "t": frame.type_name(), "tick": tick, "d": body}).to_string())
    }

    pub(super) fn decode_mesh(raw: &str) -> Option<MeshFrame> {
        // Value parsing deliberately retains last-key-wins duplicate handling.
        let value: Value = serde_json::from_str(raw).ok()?;
        if value.get("m")?.as_u64()? != u64::from(crate::lockstep::HOST_MESH_PROTOCOL) {
            return None;
        }
        let body = value.get("d")?;
        let _: u32 = decode(body.get("from")?.clone())?;
        let tick = body.get("tick")?.as_u64()?;
        let (tag, body) = match value.get("t")?.as_str()? {
            "tick" => {
                let mut object = project(body, "from tick ready_through commands start_grant", "")?;
                for command in object.get_mut("commands")?.as_array_mut()? {
                    let mut fields = project(command, "tick origin seq ship target payload", "")?;
                    nest(
                        &mut fields,
                        "order",
                        &[("origin", "origin"), ("seq", "seq")],
                    )?;
                    *command = Value::Object(fields);
                }
                ("Tick", object)
            }
            "digest" => {
                let mut object = project(body, "from tick digest", "")?;
                unhex(&mut object, "digest")?;
                ("Digest", object)
            }
            "snapshot" => {
                let mut object = project(
                    body,
                    "from tick transfer_id seq total whole_hash crc text",
                    "",
                )?;
                unhex(&mut object, "transfer_id whole_hash")?;
                ("Snapshot", object)
            }
            "host-loss" => ("HostLoss", project(body, "from tick lost", "")?),
            "slot-claim" => ("SlotClaim", project(body, "from tick slot claim_seq", "")?),
            "gm-action" => {
                let kind = body.get("kind")?.as_str()?;
                let mut object = match kind {
                    "proposal" if tick == 0 => {
                        project(body, "from operator_id correlation action", "")?
                    }
                    "granted" => {
                        let mut object = project(body, "from requester operator_id correlation recovery_generation apply_tick sequence action", "")?;
                        if object.get("apply_tick")?.as_u64()? != tick {
                            return None;
                        }
                        rename(&mut object, "from", "sequenced_by")?;
                        rename(&mut object, "requester", "from")?;
                        object.insert("origin".into(), object["from"].clone());
                        nest(
                            &mut object,
                            "order",
                            &[("origin", "origin"), ("sequence", "sequence")],
                        )?;
                        object
                    }
                    "refused" => {
                        let mut object = project(body, "from requester operator_id correlation action_kind requested_active tick reason", "effect_scope observer comms_recipients npc_doctrine objective_verb objective_instance_scope objective_recipients lever")?;
                        rename(&mut object, "from", "sequenced_by")?;
                        object.insert(
                            "target".into(),
                            body.get("target")
                                .and_then(Value::as_str)
                                .and_then(super::bounded_gm_target_id)
                                .map(Value::String)
                                .unwrap_or(Value::Null),
                        );
                        let verb: Option<crate::gm_action::GmEventVerb> =
                            body.get("verb").cloned().and_then(decode);
                        object.insert("verb".into(), serde_json::to_value(verb).ok()?);
                        if let Some(observer) = object.get("observer").filter(|v| !v.is_null()) {
                            super::bounded_gm_target_id(observer.as_str()?)?;
                        }
                        object
                    }
                    _ => return None,
                };
                let frame: crate::gm_action::GmActionFrame =
                    decode(tagged(kind, std::mem::take(&mut object)))?;
                match &frame {
                    crate::gm_action::GmActionFrame::Proposal(proposal) => {
                        proposal.validate().ok()?
                    }
                    crate::gm_action::GmActionFrame::Granted(grant) => grant.validate().ok()?,
                    crate::gm_action::GmActionFrame::Refused(refusal) => {
                        use crate::gm_action::GmActionKind::*;
                        if !crate::gm_action::valid_comms_result_scope(
                            refusal.action_kind,
                            refusal.comms_recipients.as_deref(),
                        ) || matches!(
                            refusal.action_kind,
                            ContactReveal
                                | ContactConceal
                                | ContactNormal
                                | ContactMisclassify
                                | ContactClassificationNormal
                                | ContactInformation
                        ) != refusal.observer.is_some()
                            || (refusal.effect_scope.is_some()
                                && !matches!(
                                    refusal.action_kind,
                                    DirectEffect | SystemDisable | SystemRestore
                                ))
                        {
                            return None;
                        }
                    }
                }
                return Some(MeshFrame::GmAction(frame));
            }
            "gm-join" => {
                let (kind, mut object) = match body.get("kind")?.as_str()? {
                    "pause" | "committed" => {
                        let pause = body.get("kind")?.as_str()? == "pause";
                        let required = if pause {
                            "join_id join_kind from approved_by candidate operator_id apply_tick transfer_id"
                        } else {
                            "join_id join_kind from candidate operator_id tick digest"
                        };
                        let mut object = project(body, required, "")?;
                        if pause && object.get("apply_tick")?.as_u64()? != tick {
                            return None;
                        }
                        rename(&mut object, "from", "owner")?;
                        rename(&mut object, "join_kind", "kind")?;
                        nest(
                            &mut object,
                            "candidate",
                            &[("candidate", "host"), ("operator_id", "operator_id")],
                        )?;
                        unhex(&mut object, if pause { "transfer_id" } else { "digest" })?;
                        (if pause { "Pause" } else { "Committed" }, object)
                    }
                    "restored" if tick == 0 => {
                        let mut object = project(body, "from join_id digest", "")?;
                        unhex(&mut object, "digest")?;
                        ("Restored", object)
                    }
                    "restore-boundary" if tick == 0 => (
                        "RestoreBoundary",
                        project(body, "from join_id boundary", "")?,
                    ),
                    "refused" if tick == 0 => {
                        ("Refused", project(body, "from join_id reason", "")?)
                    }
                    _ => return None,
                };
                rename(&mut object, "join_id", "id")?;
                return decode(tagged(kind, object)).map(MeshFrame::GmJoin);
            }
            "gm-restore" => {
                let (kind, mut object) = match body.get("kind")?.as_str()? {
                    "ready" => (
                        "Ready",
                        project(body, "from restore_origin restore_sequence", "")?,
                    ),
                    "loaded" => {
                        let mut object =
                            project(body, "from restore_origin restore_sequence tick digest", "")?;
                        unhex(&mut object, "digest")?;
                        ("Loaded", object)
                    }
                    "unable" => (
                        "Unable",
                        project(body, "from restore_origin restore_sequence failure", "")?,
                    ),
                    "settle" => {
                        let mut object =
                            project(body, "from restore_origin restore_sequence commit", "")?;
                        let failure: Option<crate::gm_restore::GmRestoreFailure> =
                            body.get("failure").cloned().and_then(decode);
                        object.insert("failure".into(), serde_json::to_value(failure).ok()?);
                        ("Settle", object)
                    }
                    _ => return None,
                };
                nest(
                    &mut object,
                    "restore",
                    &[
                        ("restore_origin", "origin"),
                        ("restore_sequence", "sequence"),
                    ],
                )?;
                return decode(tagged(kind, object)).map(MeshFrame::GmRestore);
            }
            _ => return None,
        };
        let frame: MeshFrame = decode(tagged(tag, body))?;
        if let MeshFrame::Tick(frame) = &frame {
            if let Some(grant) = &frame.start_grant {
                grant.validate().ok()?;
            }
        }
        Some(frame)
    }
}

/// Encode a mesh frame with the existing envelope and exact hexadecimal fields.
pub fn encode_mesh_frame(frame: &crate::lockstep::MeshFrame) -> Result<String, serde_json::Error> {
    wire::encode_mesh(frame)
}

/// Drop malformed or unsupported frames; envelope tick remains advisory.
pub fn decode_mesh_frame(raw: &str) -> Option<crate::lockstep::MeshFrame> {
    wire::decode_mesh(raw)
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
        let authored_slot_id = entry
            .get("authored_slot_id")
            .and_then(|value| value.as_str())
            .filter(|value| !value.is_empty())
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
            authored_slot_id,
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

/// Encode the local peer's own bound GM operator row for the host page.
///
/// The read-back half of the standalone binding
/// ([`crate::gm_solo::local_gm_operator`]): the page must not fabricate the id
/// its actions are attributed to, so it reads the exact row the simulation
/// bound. Same `{ id, name, connected, ready }` shape the page sends the other
/// way through [`decode_gm_roster`]; `""` means this peer may not act.
pub fn encode_local_gm_operator(operator: Option<&crate::gm_roster::GmOperator>) -> String {
    operator
        .and_then(|operator| serde_json::to_string(operator).ok())
        .unwrap_or_default()
}

/// Decode the one privileged browser-GM action ingress (issue #1292).
/// Unknown fields are refused so this narrow route cannot accidentally become
/// a generic host mutation surface.
pub fn decode_gm_action_request(raw: &str) -> Option<crate::gm_action::GmActionRequest> {
    wire::decode_gm_request(raw)
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
    recovery: &serde_json::Value,
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
        "recovery": recovery,
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

#[cfg(all(feature = "server", not(target_arch = "wasm32")))]
pub fn decode_native_gm_record(
    json: &str,
) -> Option<crate::native_host::native_gm::NativeGmRecord> {
    if json.len() > 128 * 1024 {
        return None;
    }
    serde_json::from_str(json).ok()
}

#[cfg(not(target_arch = "wasm32"))]
pub fn decode_workshop_request(
    text: &str,
) -> Result<crate::workshop::provider::WorkshopRequest, String> {
    use crate::workshop::provider::{Operation, WorkshopRequest};
    let mut fields = serde_json::from_str::<serde_json::Map<String, serde_json::Value>>(text)
        .map_err(|error| error.to_string())?;
    let id = fields
        .remove("id")
        .and_then(|value| value.as_u64())
        .ok_or("Invalid Workshop request id")?;
    let field_count = fields.len();
    // A flattened internally tagged enum cannot use deny_unknown_fields on
    // its outer envelope. Remove only that envelope field, then deserialize
    // the typed operation. Serde's unit variants ignore additional fields,
    // so those empty operations also need an exact envelope-size check.
    let operation: Operation = serde_json::from_value(serde_json::Value::Object(fields))
        .map_err(|error| error.to_string())?;
    if matches!(
        operation,
        Operation::Load
            | Operation::LoadSources
            | Operation::LoadDependencies
            | Operation::RecoveryLoad
            | Operation::RecoveryClear
            | Operation::TestStatus
            | Operation::TestStop
    ) && field_count != 1
    {
        return Err("Unexpected Workshop operation field".into());
    }
    Ok(WorkshopRequest { id, operation })
}

pub fn decode_fleet_continuation(
    value: &str,
) -> Result<crate::lockstep::continuation::ContinuationRequest, String> {
    if value.len() > 4096 {
        return Err("continuation-request-too-large".into());
    }
    serde_json::from_str(value).map_err(|error| error.to_string())
}

#[cfg(test)]
#[path = "codec_mesh_frame_tests.rs"]
mod mesh_frame_tests;

#[cfg(test)]
#[path = "codec_wire_tests.rs"]
mod wire_tests;
