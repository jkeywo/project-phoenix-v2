---
title: Console UI Authoring Library
kind: concept
status: decided
sources:
  - gui/components/
  - gui/console-ui.js
related:
  - wiki/entities/console.md
  - wiki/concepts/client-architecture.md
---

## Decision

Console controls are reusable custom elements in `gui/components/`. They own their shadow-DOM rendering and interaction details. Ship/station HTML files compose those elements into a console layout and retain only the wiring unique to that console.

`gui/console-ui.js` contains small shared DOM helpers for the remaining ordinary markup, including consistent status bars and AI-control presentation.

## Rationale

- Complex controls such as radar, helm joysticks, repair teams, weapons, shields, and navigation maps are tested independently and can be reused by more than one ship layout.
- A station still owns its overall layout and any genuinely station-specific aggregation; componentisation does not imply a schema-driven renderer.
- The server remains authoritative. Components render snapshots and send the normal client actions; they do not create client-side simulation state.

## Authoring Rule

Add a new component when a control has its own behaviour or is shared across layouts. Keep simple composition and station-specific presentation in the relevant ship/station HTML file. Register custom elements defensively so component unit tests and a loaded console can coexist.

## What This Rules Out

- Copying a complex control's DOM and behaviour into every console layout.
- A config-file renderer that attempts to generate every station interface.
