---
title: Comms Panel
type: concept
tags: [comms, client, inbox, hails, priority, localisation, input, feedback]
sources: [crates/phoenix-simulation/src/comms/capture.rs, crates/phoenix-simulation/src/snapshot/comms_capture.rs, gui/comms-capture.js, gui/components/comms-capture-controls.js, crates/phoenix-simulation/src/comms/media_control.rs, crates/phoenix-simulation/src/comms/media_session.rs, crates/phoenix-simulation/src/comms/media_edge.rs, crates/phoenix-simulation/src/comms/media_mesh.rs, gui/comms-media-client.js, gui/fleet-media.js, crates/phoenix-simulation/src/comms/formation.rs, crates/phoenix-simulation/src/comms/planned.rs, crates/phoenix-simulation/src/comms/request_delivery.rs, crates/phoenix-simulation/src/comms/radio_route.rs, assets/entities/relay_cruiser.toml, crates/phoenix-model/src/comms_request.rs, crates/phoenix-simulation/src/comms/requests.rs, crates/phoenix-simulation/src/comms/request_returns.rs, crates/phoenix-simulation/src/snapshot/comms_requests.rs, gui/comms-requests.js, gui/components/ph-comms-text-composer.js, editor/workshop-comms-radio.js, gui/workshop-comms-radio-panel.js, gui/cruiser/comms.console.js, gui/stations/comms-console.js, gui/stations/comms-actions.js, gui/semantic-action-registry.js, gui/action-feedback.js, gui/comms-state.js, gui/components/ph-comms-contact-list.js, gui/components/ph-comms-hail-list.js, gui/components/ph-comms-current-message.js, gui/console-state.js, gui/action-map.js, crates/phoenix-simulation/src/command_admission/mod.rs, crates/phoenix-simulation/src/console/comms/server.rs, crates/phoenix-simulation/src/console/comms/inbox.rs, crates/phoenix-sim-world/src/comms/content.rs, crates/phoenix-simulation/src/comms/server.rs, crates/phoenix-simulation/src/world/server/mod.rs, tests/publisher_ordering.rs, crates/phoenix-simulation/src/comms/scripted.rs, crates/phoenix-model/src/messages.rs, crates/phoenix-simulation/src/gm/operations/comms.rs, gui/gm-comms-panel.js, docs/gm-comms-authoring.md, assets/strings/strings.csv]
updated: 2026-10-16
---

# Comms Panel

## Captured evidence

The existing radio pass measures new paid foreign emissions into one optional
ShipCommsCapture owner. Human selection and ordinary Comms AI use the same
permitted headers and admitted local decoder handle. The decoder pays additional
continuous work; only completed evidence reveals dated attributed text. Captured
requests never become addressed messages or command receipts.

The canonical frontier and bounded history join snapshot and digest. Whole-candidate
preflight checks exact receiving equipment, retained source equipment and surviving
emission copies. A historical emitter may be gone; no body or actor archive is
required. Saved capture publications must agree with canonical paid progress.
gui/comms-capture.js and the shared Comms renderer expose independent controls
on the existing screen and preserve explicit Console feed/ownership boundaries.

## Selected human media

`comms/media_control.rs` owns the admitted selected call and the current paid
carrier derived by fixed radio work. `media_session.rs` binds explicit device
starts to the actual current human operator and physical connection. `media_edge.rs`
reads those owners in PostUpdate; raw packets cannot issue machinery commands or
spend power. `media_mesh.rs` carries bounded ephemeral packets over existing
authenticated fleet links, outside continuation history. Restore clears starts,
queued bytes and routes while retaining generation high-water; an authenticated
current peer heartbeat can require a fresh start after host replacement.

The Comms thread's media controls select an actually accepted/heard conversation
through normal System admission, then require separate local capture consent.
`gui/comms-media-client.js` handles independent PCM/JPEG packets and bounded
playback. Native panes and insecure origins report capture unavailable. Ordinary
AI declines media requests after its normal reading work. `gui/fleet-media.js`
and the host adapters keep media outside ordinary retained queues and use only
the negotiated drop-only Snapshot path. Device and two-host acceptance are
separate from source review and mocked-device tests.

## Radio authoring

`editor/workshop-comms-radio.js` and `gui/workshop-comms-radio-panel.js`
provide structured channel and held-key rows on the existing Workshop surface.
They bind the actual canonical Comms System, owning Console, finite endpoint
reach and positive continuous/discrete energy costs. Each channel has a unique
id, endpoint-bounded range, positive whole work units and optional held-key
binding, plus optional relay forwarding. Removing a key leaves a protected channel invalid until its author
explicitly changes that channel.

Edits use the normal accepted draft/history transaction, explicit inherited
materialisation and existing Preview/Test entry points. The bounded writer
preserves other Comms settings and comments and checks parsed equivalence
before accepting a candidate. These authoring checks remain separate from
native radio runtime and human acceptance evidence.

`comms/radio_route.rs` derives direct-first shortest-hop delivery through actual
currently paid radios whose matching channel opts into forwarding. Stable UUID
order breaks equal-hop ties and visited identities prevent cycles. Every hop
uses the same actual endpoint range, fields, terrain, jammer and exact receiver
protection. This first protected relay requires its held matching key.
`relay_cruiser.toml` is an optional materialised radio reference with unchanged
base energy. Forwarding neither changes the original sender/audience nor gives a
relay an unaddressed inbox copy. No route membership is published or persisted.
One-shot text/manual-request emission, retained accepted planned requests and returned outcomes share the route;
returned retries retain the existing single discrete emission/payment frontier.

Runtime messages keep canonical `priority`. Legacy `is_urgent` is derived at the wire boundary and at its historical digest fold position; deserialization promotes legacy urgency only when no priority was supplied.

The Comms client is a mounted station controller plus three reusable components: contacts, hail threads, and the current message/reply surface. It renders authoritative `CommsState`; it does not advance dialogue locally.

`gui/comms-state.js` folds contacts, messages, range flags, thread state, and priority. `gui/stations/comms-console.js` owns the one local selected-message id and projects it into both the hail list and current-message panel. Hull-specific Comms HTML composes the same controller/components.

The Cruiser shares one document between Comms and auxiliary Navigation. `gui/cruiser/comms.console.js` selects the visible view from authored `system_ids` and their projected families. A human Comms holder also receives visiting Navigation data through `withVisitingSystems`; those extra keyed views do not replace the Comms view or its separate Navigation tab.

## Actions and feedback

`gui/stations/comms-actions.js` registers hail, message selection, response, clear, show-on-screen and radio text under the shared `comms` action context. Every action has two remappable keyboard/gamepad slots and visible controls call the same semantic activation seam as keys and gamepads. The contact adapter preserves the authoritative wire `uuid`; the response adapter reads the renderer's selected-or-automatic current thread and preserves exact message id/index, availability, and important-response confirmation.

`gui/components/ph-comms-text-composer.js` keeps only the local text draft. The
shared renderer supplies the owning Console's published channels and contacts.
`SendCommsText` uses the normal Comms System command boundary. A sent inbox fact
clears a matching unchanged draft; invoking Send or receiving another ship's text
does not. Sent and received labels report only the recorded local fact.

Hail, response, clear, and show-on-screen use correlated commands. Their existing server consumers return `Applied` only after consuming the command and `Refused` at their existing rejection points. Message selection is local because the server has no selection-state consumer: the shared renderer validates the requested live message, updates the list, current thread and battleship footer synchronously, and only then lets the lifecycle report `Applied`. It sends no `SelectCommsMessage`; a stale request returns unhandled instead of falsely completing. The established `CommsResponseRejected` message remains alongside shared `ActionFeedback`, so a refused response still identifies and flashes its exact button.

## Structured status requests

The composer sends the closed `ReportStatus` request through the same authored
channel/audience and normal correlated Comms command as text. It preserves the
literal text draft. Received request controls explicitly mark a copy read or
submit Report/Decline; delegated request controls are read-only. The ordinary
receiving Comms controller reads for the authored positive
`request_reading_ticks`, retaining partial progress in its existing pending
response entry and pausing under Human or Offline control. Reading and a local
decision are separate from receipt.

The request, its original clocks and receiver-local facts live on the existing
recipient-owned inbox copy. A Report describes only its actual controlling
Comms System's coarse availability at the recorded decision tick. Returned
messages display recorded heard/read/decision/report ticks; they do not claim
remote actuator acceptance. Independently paid return transmission can wait for
a working return link without changing the reporter's local decision or exposing
a hidden listener's viability.

Clear dismisses eligible structured rows without deleting their lineage or
pending returns. Show cleared requests restores those same permitted records
for the Console operator; it grants no other Console's history. New returned
outcomes have their own presentation state. Ordinary plot/text Clear retains
its prior behaviour. `last_returned` is an internal idempotence frontier and is
removed from Blackboard, direct CommsState and ShowOnScreen projections.

## Planned status requests

`comms/planned.rs` reads released requests from the Comms System's existing typed
Console publication. A row requires the recipient's actual UUID already in that
Console's contacts and a published equipment channel. Neither a shared Station
nor a task identifier expands this knowledge. The composer passes the chosen
row through the same correlated request action; sending revalidates the exact
row, audience, channel and request before charging real radio work.

The existing Comms response AI has an authored `comms_request` output alongside
its response output. Planned reading uses the same `pending_ai_responses` owner
with a distinct key kind; Human control, missing current knowledge and Offline
operation pause reading. Only successful real emission retires the pending read.
The World binding uses a prepared exclusive permit: a refused send leaves the
Situation unaccepted, while its successful same-tick commit binds the actual
minted transmission identity.

`comms/request_delivery.rs` derives retained delivery from that original sent
inbox fact and current paid endpoints. Only accepted planned Contact requests
have this flag. Retries reuse the original origin, sender, request and emission
tick, and a stable recipient-copy id prevents duplicate reception. There is no
separate retry queue or repeated emission charge. Ordinary text and manually
sent status requests retain one-shot delivery. Actual returned reports remain
distinct from permission to act.

## Formation coordination

`GenericCommsRequest::FormUp` carries only a bounded integer World-x/z slot.
Its Contact recipient receives the same actual radio request and ordinary
reading/response controls. The positive reply records `Accept` coordination;
it creates no status report and makes no Navigation/Helm readiness claim.
The composer exposes its slot in a separate Contact-only detail section and
keeps the text draft. Request rendering distinguishes acceptance from reporting.

The actual emission derives an increasing Formation sequence from retained own
sent request history, which is the only sequence owner. Copies and returned
references preserve it. Clear and the plot-placeholder removal seam retain
structured history. Old delayed requests cannot be accepted after a newer
same-source request has been accepted locally. Complete candidate validation
checks source sequence continuity, immutable copies and nondecreasing accepted
sequence clocks per recipient. Contact FormUp delivery uses the same paid
retained inbox service; planned ReportStatus separately retains its World binding.

`comms/formation.rs::accepted` gives a receiving System only the latest genuinely
heard, locally accepted suggestion per source through its explicitly bound Comms
publication. It retains original sender/callsign/sequence/clocks as coordination
provenance. That source identity is not evidence for private Broad association.
Navigation and Helm require their separate current knowledge and authority;
this helper executes no machinery. The World planning Communication primitive
remains ReportStatus-only.

## Priority

`CommsPriority` is authoritative. `critical` is a generic continuing interruption and wins the panel's automatic current-thread selection, but it remains normal non-modal content, so an explicit local message selection can inspect another thread. Reply/clear effects change the authoritative state; optimistic client dismissal is not the source of truth.

## Server path

`crates/phoenix-simulation/src/recipients.rs` resolves script Comms and addressed actions from authored
ship slots, live factions, all player ships and current Objective-instance
members. `crates/phoenix-sim-world/src/world/script/recipient_refs.rs` checks literal names at Workshop
Save; computed selectors are checked at execution. Empty or refused selections
appear in Workshop Test trace and `gui/gm-recipient-diagnostics.js`. The Comms
adapter evaluates one root and binds a separate ordinary private thread to each
receiver. See [recipient authoring](../../docs/recipient-actions-authoring.md).

The Comms state publisher runs before ObjectiveSummary and both precede the
simulation broadcaster's outbox drain. A newly resolved Comms host receives
its reliable state on that same tick. `tests/publisher_ordering.rs` checks the
actual host transition and wire delivery across ordinary and opposed schedules.

`CommsConsolePlugin` owns admitted hail/reply application and the two Backfill hosts. Both human and AI paths converge on `handle_hail` and `handle_respond_to_message`. Scripted `on_pick` effects enter the normal world command/dispatch pipeline, so a reply cannot bypass scenario authority.

Authored titles, bodies, speaker names, and responses are string ids resolved through `assets/strings/strings.csv`. The bounded literal transmission added in #1317 carries `literal_body: true`; `gui/strings.js` preserves its exact body and subject even when they match a String Table id. The renderer still uses text content.

`crates/phoenix-simulation/src/gm/operations/comms.rs` resolves scenario-authored routes to existing hailable identities and live Fleet ships. `gui/gm-comms-panel.js` captures sender, recipients, route, and content before its optional shared confirmation seam; canonical GM results retain the exact intent and operator attribution. The `gm_comms` Host Channel stays raw until that panel renders display fields. Authoring examples live in `docs/gm-comms-authoring.md`.

A routed inbox message belongs to one immutable `ShipKey`; one multi-recipient send creates one ordinary thread per selected ship. Blackboard publication, client Comms state, Backfill decisions, response admission, clear, and viewscreen selection all respect that audience. Queued scripted hails and subsequent `ScriptedDialogue` nodes carry it through snapshot restoration and continuation. Legacy messages with no audience remain fleet-visible.

## Related

- [Comms Range](./comms-range.md)
- [Localisation](./localisation.md)
- [Message Flow](./message-flow.md)
