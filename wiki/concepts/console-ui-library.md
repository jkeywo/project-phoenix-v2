---
title: Console UI Authoring Library
type: concept
tags: [client, console, components, html, css, javascript]
sources:
  - gui/components/
  - gui/console-ui.js
  - gui/mount-plan.js
  - gui/hero-bar.js
  - gui/cruiser/engineering.html
  - gui/components/ph-repair-teams.js
  - gui/console.css
  - gui/cruiser/tactical.html
updated: 2026-09-27
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

Cruiser Tactical opts into `tactical-scope-row` for its portrait layout: a
full-width scope, paired bounded weapon rails, and a full-width target strip.
The torpedo and target components opt into compact rendering through `compact-rail` (all orientations) and
`portrait-strip` (portrait only). The target card sits below the phasers in landscape and desktop.

The shell's `gui/hero-bar.js` owns Station identity, settings/help chrome, and
the roving Station/overlay tablist. Console documents contain only their
working controls; they reserve no legacy settings gutter or repeated title
and damage footer. `gui/console.css` supplies the shared
Captain and Engineering segmented selectors.

Cruiser Engineering composes Power and Repair with the Engineering-owned
tractor and umbilical controls. Named repair-team cards render the external
target row and dispatch through the same semantic action adapter as internal
assignments. Helm owns the contextual Dock control and the passive tow readout.


The explicit field-repair shortcut also emits the named-team command: it keeps
an explicit team context or selects the first available team, and recalls the
team named by the authoritative external claim. The generic internal dispatch
shortcut never silently selects the field.

The six documents in `gui/dynasty-cruiser/` retain shared controls and semantic
actions while `gui/themes/dynasty-cruiser.css` supplies their common identity.
Each carries a reopenable role/attack-cycle guide. The hull's authored tutorials
use the shared `gui/tutorial-state.js` evaluator: Power observes charging and
depletion, Gunnery observes available charge, enabled boost and depletion.
Those lessons consume console projections and private dismissal progress;
they neither allocate power nor toggle boost. All guide text uses catalogue IDs.
