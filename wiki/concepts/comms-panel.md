---
title: Comms Panel
type: concept
tags: [comms, client, inbox, hails, priority, localisation, input, feedback]
sources: [gui/cruiser/comms.console.js, gui/stations/comms-console.js, gui/stations/comms-actions.js, gui/semantic-action-registry.js, gui/action-feedback.js, gui/comms-state.js, gui/components/ph-comms-contact-list.js, gui/components/ph-comms-hail-list.js, gui/components/ph-comms-current-message.js, gui/console-state.js, gui/action-map.js, src/command_admission/mod.rs, src/console/comms/server.rs, src/console/comms/inbox.rs, src/comms/content.rs, src/comms/server.rs, src/world/server.rs, tests/publisher_ordering.rs, src/comms/scripted.rs, src/core/messages.rs, src/gm_comms.rs, gui/gm-comms-panel.js, docs/gm-comms-authoring.md, assets/strings/strings.csv]
updated: 2026-09-08
---

# Comms Panel

The Comms client is a mounted station controller plus three reusable components: contacts, hail threads, and the current message/reply surface. It renders authoritative `CommsState`; it does not advance dialogue locally.

`gui/comms-state.js` folds contacts, messages, range flags, thread state, and priority. `gui/stations/comms-console.js` owns the one local selected-message id and projects it into both the hail list and current-message panel. Hull-specific Comms HTML composes the same controller/components.

The Cruiser shares one document between Comms and auxiliary Navigation. `gui/cruiser/comms.console.js` selects the visible view from authored `system_ids` and their projected families. A human Comms holder also receives visiting Navigation data through `withVisitingSystems`; those extra keyed views do not replace the Comms view or its separate Navigation tab.

## Actions and feedback

`gui/stations/comms-actions.js` registers hail, message selection, response, clear, and show-on-screen under the shared `comms` action context. Every action has two remappable keyboard/gamepad slots and visible controls call the same semantic activation seam as keys and gamepads. The contact adapter preserves the authoritative wire `uuid`; the response adapter reads the renderer's selected-or-automatic current thread and preserves exact message id/index, availability, and important-response confirmation.

Hail, response, clear, and show-on-screen use correlated commands. Their existing server consumers return `Applied` only after consuming the command and `Refused` at their existing rejection points. Message selection is local because the server has no selection-state consumer: the shared renderer validates the requested live message, updates the list, current thread and battleship footer synchronously, and only then lets the lifecycle report `Applied`. It sends no `SelectCommsMessage`; a stale request returns unhandled instead of falsely completing. The established `CommsResponseRejected` message remains alongside shared `ActionFeedback`, so a refused response still identifies and flashes its exact button.

## Priority

`CommsPriority` is authoritative. `critical` is a generic continuing interruption and wins the panel's automatic current-thread selection, but it remains normal non-modal content, so an explicit local message selection can inspect another thread. Reply/clear effects change the authoritative state; optimistic client dismissal is not the source of truth.

## Server path

The Comms state publisher runs before ObjectiveSummary and both precede the
simulation broadcaster's outbox drain. A newly resolved Comms host receives
its reliable state on that same tick. `tests/publisher_ordering.rs` checks the
actual host transition and wire delivery across ordinary and opposed schedules.

`CommsConsolePlugin` owns admitted hail/reply application and the two Backfill hosts. Both human and AI paths converge on `handle_hail` and `handle_respond_to_message`. Scripted `on_pick` effects enter the normal world command/dispatch pipeline, so a reply cannot bypass scenario authority.

Authored titles, bodies, speaker names, and responses are string ids resolved through `assets/strings/strings.csv`. The bounded literal transmission added in #1317 carries `literal_body: true`; `gui/strings.js` preserves its exact body and subject even when they match a String Table id. The renderer still uses text content.

`src/gm_comms.rs` resolves scenario-authored routes to existing hailable identities and live Fleet ships. `gui/gm-comms-panel.js` captures sender, recipients, route, and content before its optional shared confirmation seam; canonical GM results retain the exact intent and operator attribution. The `gm_comms` Host Channel stays raw until that panel renders display fields. Authoring examples live in `docs/gm-comms-authoring.md`.

A routed inbox message belongs to one immutable `ShipKey`; one multi-recipient send creates one ordinary thread per selected ship. Blackboard publication, client Comms state, Backfill decisions, response admission, clear, and viewscreen selection all respect that audience. Queued scripted hails and subsequent `ScriptedDialogue` nodes carry it through snapshot restoration and continuation. Legacy messages with no audience remain fleet-visible.

## Related

- [Comms Range](./comms-range.md)
- [Localisation](./localisation.md)
- [Message Flow](./message-flow.md)
