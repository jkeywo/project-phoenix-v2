---
title: Comms Panel
type: concept
tags: [comms, client, inbox, hails, priority, localisation]
sources: [gui/stations/comms-console.js, gui/comms-state.js, gui/components/ph-comms-contact-list.js, gui/components/ph-comms-hail-list.js, gui/components/ph-comms-current-message.js, gui/console-state.js, gui/action-map.js, src/console/comms/server.rs, src/console/comms/inbox.rs, src/comms/content.rs, src/comms/scripted.rs, src/core/messages.rs, assets/strings/strings.csv]
updated: 2026-08-27
---

# Comms Panel

The Comms client is a mounted station controller plus three reusable components: contacts, hail threads, and the current message/reply surface. It renders authoritative `CommsState`; it does not advance dialogue locally.

`gui/comms-state.js` folds contacts, messages, range flags, thread state, and priority. `gui/stations/comms-console.js` selects the active thread and wires actions through `gui/action-map.js`. Hull-specific Comms HTML composes the same controller/components.

## Priority

`CommsPriority` is authoritative. `critical` is a generic continuing interruption and wins the panel's current-thread selection, but it remains normal non-modal content. Reply/clear effects change the authoritative state; optimistic client dismissal is not the source of truth.

## Server path

`CommsConsolePlugin` owns admitted hail/reply application and the two Backfill hosts. Both human and AI paths converge on `handle_hail` and `handle_respond_to_message`. Scripted `on_pick` effects enter the normal world command/dispatch pipeline, so a reply cannot bypass scenario authority.

Player-visible titles, bodies, speaker names, and responses are string ids resolved through `assets/strings/strings.csv`.

## Related

- [Comms Range](./comms-range.md)
- [Localisation](./localisation.md)
- [Message Flow](./message-flow.md)
