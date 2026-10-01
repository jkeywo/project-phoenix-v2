---
title: Shared Behaviour Modules
type: concept
tags: [architecture, client, testing]
sources: [gui/gm-action-feedback.js, gui/gm-npc-panel.js, gui/gm-contact-panel.js, gui/gm-despawn-panel.js, gui/gm-journal-panel.js, gui/gm-comms-panel.js]
updated: 2026-10-01
---

# Shared Behaviour Modules

GM panels use GmActionFeedback for request ownership, timers, exact operator/correlation settlement and reset. Request-shaped access keeps timer metadata out of panel state. NPC, contact, despawn and journal retain one pending request; Comms retains its 32-request limit and local result history. Each panel still validates its own typed results and owns confirmation and presentation.
