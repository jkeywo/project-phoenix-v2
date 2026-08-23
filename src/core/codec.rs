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
/// #818). The wire shape is `{"from_label":…,"to_label":…,"text":…}` — the
/// `__updateChatter` handler in `server.html` reads exactly these keys.
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
/// `[(DebugFlag, bool)]` — the exact list `ServerMessage::DebugState` carries —
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
pub fn encode_debug_flags(flags: &[(crate::core::messages::DebugFlag, bool)]) -> String {
    let map: std::collections::BTreeMap<String, bool> = flags
        .iter()
        .map(|(flag, on)| (format!("{flag:?}"), *on))
        .collect();
    serde_json::to_string(&map).unwrap_or_default()
}

/// Decode inbound JSON from the HTML/PeerJS bridge.
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
