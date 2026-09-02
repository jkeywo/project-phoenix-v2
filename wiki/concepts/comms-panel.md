---
title: Comms Panel
type: concept
tags: [comms, client, inbox, hails, priority, localisation, input, feedback]
sources: [gui/stations/comms-console.js, gui/stations/comms-actions.js, gui/semantic-action-registry.js, gui/action-feedback.js, gui/comms-state.js, gui/components/ph-comms-contact-list.js, gui/components/ph-comms-hail-list.js, gui/components/ph-comms-current-message.js, gui/console-state.js, gui/action-map.js, src/command_admission/mod.rs, src/console/comms/server.rs, src/console/comms/inbox.rs, src/comms/content.rs, src/comms/scripted.rs, src/core/messages.rs, assets/strings/strings.csv]
updated: 2026-08-31
---

# Comms Panel

The Comms client is a mounted station controller plus three reusable components: contacts, hail threads, and the current message/reply surface. It renders authoritative `CommsState`; it does not advance dialogue locally.

`gui/comms-state.js` folds contacts, messages, range flags, thread state, and priority. `gui/stations/comms-console.js` owns the one local selected-message id and projects it into both the hail list and current-message panel. Hull-specific Comms HTML composes the same controller/components.

## Actions and feedback

`gui/stations/comms-actions.js` registers hail, message selection, response, clear, and show-on-screen under the shared `comms` action context. Every action has two remappable keyboard/gamepad slots and visible controls call the same semantic activation seam as keys and gamepads. The contact adapter preserves the authoritative wire `uuid`; the response adapter reads the renderer's selected-or-automatic current thread and preserves exact message id/index, availability, and important-response confirmation.

Hail, response, clear, and show-on-screen use correlated commands. Their existing server consumers return `Applied` only after consuming the command and `Refused` at their existing rejection points. Message selection is local because the server has no selection-state consumer: the shared renderer validates the requested live message, updates the list, current thread and battleship footer synchronously, and only then lets the lifecycle report `Applied`. It sends no `SelectCommsMessage`; a stale request returns unhandled instead of falsely completing. The established `CommsResponseRejected` message remains alongside shared `ActionFeedback`, so a refused response still identifies and flashes its exact button.

## Priority

`CommsPriority` is authoritative. `critical` is a generic continuing interruption and wins the panel's automatic current-thread selection, but it remains normal non-modal content, so an explicit local message selection can inspect another thread. Reply/clear effects change the authoritative state; optimistic client dismissal is not the source of truth.

## Server path

`CommsConsolePlugin` owns admitted hail/reply application and the two Backfill hosts. Both human and AI paths converge on `handle_hail` and `handle_respond_to_message`. Scripted `on_pick` effects enter the normal world command/dispatch pipeline, so a reply cannot bypass scenario authority.

Player-visible titles, bodies, speaker names, and responses are string ids resolved through `assets/strings/strings.csv`.

## Related

- [Comms Range](./comms-range.md)
- [Localisation](./localisation.md)
- [Message Flow](./message-flow.md)
