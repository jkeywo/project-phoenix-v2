# Increment 16 — Captured text and paid decoding

Adopted bounded consumer, 15 October 2026. Root approved the scope before runtime.
This extends the existing Comms System, ordinary controller and Console knowledge;
it does not complete increment 16 or claim native/browser acceptance.

## Acquisition is an actual measurement

An optional authored capture/decoder capability monitors the radio's explicitly
configured channel IDs and finite ranges. A receiver must be actually installed,
operational and continuously paid. At the real radio emission/service phase,
sample the original emitter directly against that listener using the same range,
terrain, field, jamming and receiver-protection path function as normal radio.
Ignore final audience and key compatibility for this measurement only. Do not
search historical messages that were never physically emitted during reception.

The first consumer measures original actual emission events only, on their paid
emission tick. Existing source records stamp emitted_tick only after successful
actual payment and canonical source-copy creation. A retained archive or paid
carrier does not rebroadcast historical traffic. Retried/hop interception requires
an explicit real retransmission event in its existing owner; the private selected
route or final-recipient deduplication does not create a remotely observable
emitter. Synthetic plot/internal messages cannot be captured. Retain at most one
capture per actual original emission and listener.

Capture supported text and immutable typed text/request/outcome content only.
Human audio/video bytes, device recordings and media history are excluded. A
MediaCall request is ordinary typed metadata, never permission to record media.

## Evidence does not become addressed participation

The existing Comms owner retains private captured payload plus original source,
channel, security and emission/reception clocks. Use a separate evidence record,
never an addressed CommsMessage or CommsRequestProgress. Its opaque local handle
and public header expose channel, protected/unprotected state, encryption level,
capture clock and work state. No sender UUID, audience, sender name, text, typed
body, key ID, source origin ID, route or hidden actor count leaks before decode.
The public header is a genuine radio metadata observation, not a contact grant.

After successful decode publish the attributed literal sender name and original
text/typed content as dated intercepted evidence. Keep private routing UUIDs,
key IDs and original addressed-recipient machinery lineage private. Even decoded
foreign requests/outcomes cannot be answered, accepted, used as a planned request
receipt, selected as Formation intent or treated as independently observed truth.
Any downstream correlation uses explicit evidence, never private source mapping.

## One ordinary paid work owner

Protected channels have a closed authored encryption level 1–3; existing keyed
channels default to level 1 and unkeyed channels are level 0. Optional equipment
defines its maximum breakable level and positive bounded full-payment decode
ticks per level and optional positive held-key decode ticks. A genuinely held
matching key permits that authored cheaper/higher-level route only when installed
equipment supports it; otherwise levels above the ceiling remain unavailable.
Supported lower levels need paid work rather than possession of a secret key.
This first slice uses the owning RadioProfile's immutable accepted equipment
keys only, never an enemy profile. It has no runtime key acquisition/revocation.
Future changing-key content must adopt its actual provenance/history consumer
and preserve completed decoding when a later key expires; replacing accepted
equipment/content is not a silent live key change or a save-conversion bypass.
Changing a key cannot recover a transmission that was never captured.

Human Comms selects one captured handle or pauses through ordinary System
admission. The same existing per-System AI may choose the oldest feasible capture
after current conversation/reading and planned initiation; there is no second
controller, job manager, retry queue, cipher minigame or background free work.
AI policy may inspect permitted headers and own decoder feasibility, never private
payloads. Ordinary Human/Offline handoff preserves the same selected work.

Decode consumes additional actual continuous Comms work. Advance a bounded
integer work frontier from the actual paid fraction, once per fixed interval;
unavailable/damaged/unfunded operation pauses it. Round partial grant downward,
with positive full-payment progress, and retain original start/completion clocks.
Unprotected intercepted text is immediately readable after actual capture;
protected traffic never becomes plaintext from elapsed wall time alone.

The capture history has a technical 256-row bound and deterministic oldest-
emission retirement with a private monotonic retirement watermark. It has no
message-capacity stock, quotas, traffic budget or replenishment.
Retiring the selected row clears only that work selection. This is an operational
memory bound, never a scenario economy or congestion game.

## Ownership, authoring and continuation

Keep authored definitions/closed DTOs in Model, pure work/validation in Gameplay,
actual path/payment/admission/controller integration in Simulation and accepted
radio source editing in the existing Workshop transaction/history. Console views
come only from their explicit permitted Comms feed; co-hosted tabs gain nothing.
Human controls cannot select another hull's handle or issue remote machinery.

Canonical evidence, selected handle and paid frontier join existing snapshots,
digests and whole-candidate preflight. Validate exact owning System and immutable
accepted capability, unique actual origin/listener identity, ordered finite clocks,
bounded progress and completion, technical retirement order and all surviving
source/copy content consistency before any write. Derived listening/routes are
rebuilt after restore. Decode cannot consume a stale pre-restore paid grant.

Root owns central Model/payload/publication/snapshot/digest/epoch joins. The radio
measurement hook preserves media's final paid-carrier derivation. Leaves do not
introduce a transport, new knowledge pool or competing simulation store.

Required exit evidence includes actual paid capture versus out-of-range/blocked/
unfunded/never-received traffic; audience/key independence without early plaintext;
Human and ordinary AI paid decode; impossible security and held-key cases; explicit
Console isolation; decoded-request non-participation; Workshop/history/Preview/Test;
and partially decoded fresh JSON continuation for at least 360 full state/digest
steps. Malformed owner/capability/clock/frontier/content must refuse without writes.
Source-only checks are not native, device, workload or balance acceptance.

## Bounded leaf source handoff — 16 October 2026

New Model DTOs, pure Gameplay state/work/validation, actual radio acquisition and
decoder payment, the existing policy's closed decoder output, optional reference
hull, independent controls and accepted Workshop edits are prepared together.
The radio hook runs after actual send/outcome creation and before media's final
paid-carrier derivation. Every acquisition requires emitted_tick == current tick;
the pure owner also enforces this before inserting evidence. It cannot consume
retained history, private recipient deduplication or a prior paid media carrier.

Focused JavaScript: 17 tests across three files passed (latest 4.17s); strict strings
6,307 with zero errors/warnings; Workshop bundle and layer import checks passed.
Rust leaves are formatted. Pure Model/Gameplay source review passed under the
explicit immutable accepted-key boundary. Independent actual capture/decoder/AI
leaf and radio-hook source review passed after correcting the actual work API
Result and the owning System's implicit own-publication test; native execution and
integrated continuation remain pending. The Rust owner tests use real spawned
radios and reserves for paid foreign capture, no addressed inbox/early body,
paid decode, impossible protection and never-captured archive refusal. Their
execution is root-owned; this handoff claims no native or milestone exit.

### Root-owned central joins

- Export Model `comms_capture` and Gameplay `electronic::capture`; export Simulation
  `comms::capture` and `capture_ai`. Preserve existing media registrations.
- Add optional serde-default Radio `capture: Option<CommsCaptureConfig>` and keyed
  channel `protection_level: u8` default 1, validated 1–3. Unkeyed effective level
  is always 0. Call capture config validation from Radio validation. Preserve
  existing no-decoder behavior and media capability; update explicit constructors.
- Add `SelectCommsCapture { capture: Option<u64> }` under existing Comms admission,
  including an own safe-positive-integer handle bound (2^53−1). Add optional
  `CommsBlackboard.captured: Option<CommsCaptureControlsView>`. Publish only
  `capture::views` for that exact actor/System and explicit Console feed; the
  private captured payload type must never enter a client/AI publication.
- Add one `ShipCommsCapture(CaptureState)` canonical component declaration in the
  existing radio owner. Installation and the coordinated radio measurement/work
  hook are in the donor; preserve media's tail hook after capture work.
- Existing `operate_comms_response_ai` calls `capture_ai::decision` at the same
  normal cadence with actual config/boards/sources/radio/policy/flag chain and own
  availability. Collect its `(entity, owner, capture)` result and emit normal
  `SelectCommsCapture` via the existing `emit_ai_command` pass. The helper holds
  behind visible open conversation/released request work and already selected
  decoding, so it does not replace reading or create a second controller/state.
- Add optional canonical EntityState capture payload, complete capture/digest and
  restore. Validate required/forbidden presence from the exact accepted Radio
  recipe for all canonical actors including Disabled/carried ones; call
  `CaptureState::validate` with its exact owner/capability/immutable keys and tick.
  Its `payloads()` accessor is trusted preflight only: verify each source differs
  from receiver; cross-check source equipment/channel/key/effective level where a
  genuinely retained candidate source/accepted origin is available. A historical
  emitter may be absent after ordinary removal: never preserve/protect its body
  or add an actor archive to validate dated intercepted evidence. Validate the
  immutable captured provenance/header/payload internally; every surviving
  matching original/heard inbox copy must agree with source name,
  body/literal flag and immutable wire (ignoring the copy's received_tick).
  This proves structural and retained-evidence consistency, not authentication of
  erased historical truth. Do not require cleared text to reappear or publish a
  source UUID.
- Advance epochs coherently and refuse malformed owners, missing/extra state,
  capture clock != actual emission clock, impossible paid interval/work/frontier,
  private source/copy mismatch and incompatible accepted capability before writes.
  There is no derived prepaid decoding resource to restore; listening is rebuilt
  through the existing actual radio pass before work can advance.
- Client central plumbing calls `capturedPayload(bb.captured, owner, authorised)`
  from the explicit Comms read view into `comms_capture`. Register independent
  `registerCaptureActions`, map `comms_capture_select` to that exact owner and
  closed payload, and mount/update/dispose `mountCommsCaptureControls` on the
  existing Comms console. No new screen/seat or layout redesign.

Required next evidence is joined native owner execution, actual Human/ordinary AI
full-host partially paid JSON360/digest/no-write proof, explicit-feed isolation and
actual Workshop Preview/Test. It is not replaced by the pure state JSON test or
the focused GUI/history fixtures. Dynamic keys and retransmission/hop capture require separately selected later
consumers; they are not required by this bounded accepted decoder contract.
Other selected 16/17 scope and acceptance remain open.

### Central integration checkpoint — 16 October 2026

The prepared media branch now joins the capture modules, accepted Radio fields,
ordinary Comms admission/controller/publication, existing-screen controls, canonical
snapshot state and complete digest namespace. Capture work remains before media's
final paid-carrier derivation on the same existing radio pass. There is no new
controller, schedule step, clock or prepaid state.

Whole-candidate validation uses the saved graph and exact receiving recipe rather
than the bootstrap inbox. Available candidate source equipment and surviving
original/heard copies constrain captured metadata and content. Absent historical
emitters remain valid. The single-row inverse uses retained live evidence, while
the shared structural guard does not read bootstrap source state. A saved Comms
capture display may withhold information; any supplied view must equal the exact
canonical paid decoder projection and cannot reveal premature plaintext.

Central owner tests cover actual paid acquisition, missing/foreign state, forged
content/channel/self-source/clock/display, single-row and whole-candidate no-write,
Disabled receiver lookup and absent materializable receiver recipes. Admission
tests cover Human/AI handle bounds and wrong System kinds. Digest tests preserve
the exact empty-stage contribution and stable actual-receiver order, including
Disabled canonical actors. These Rust tests are authored, not executed here.

Focused client checks passed: four capture/Comms/core files, 59 tests (2.28s), then
three Console projection/Workshop files, 432 tests (4.73s). Layer rules passed;
strict strings found 6,419 IDs with zero errors/warnings after removing four
duplicate media IDs introduced by the capture donor while keeping the prepared
media definitions. Owned Rust formatting and diff checks passed. No Cargo, full
integration gates, device/native execution or milestone exit is claimed. Root
owns compatibility epochs, final integration and native continuation evidence.


### Remaining scope reconciliation — 16 October 2026

The adopted captured-text decision uses immutable accepted receiver keys. Dynamic
key acquisition/revocation, opaque keyless forwarding and relay-hop/retransmission
capture are future consumers requiring their own selected contracts. They are not
new exit requirements created by this first slice's roadmap wording. Audio/video
recording/interception and a crypto economy are likewise not implied.

The selected original typed-content disclosure remains incomplete: current decoded
evidence presents body and kind while omitting safe typed request parameters and
outcome facts. Complete that read-only evidence without exposing source routing,
creating a contact alias or admitting participation in intercepted conversations.
The broader selected Comms scope still needs finite listening/search priorities,
routine public chatter, applicable pending-request lifecycle rules and concrete
contextual request/relationship consumers. Several of those joins belong with
movement, craft, personnel and continuing missions. Source audit does not establish
native execution or human/device/Workshop acceptance.
