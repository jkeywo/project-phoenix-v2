# Project Phoenix — GM and Workshop

| Field | Value |
|---|---|
| Document | GDD-GM-WORKSHOP |
| Last updated | 2026-10-02 |
| Status | Current operator/authoring capabilities; Plan and open acceptance identified below |
| Authority | [GM milestones](../../../pasm/spec/roadmap/gm-console-milestones.yaml), their implementation mappings and the runtime |

GMs are optional equal host-class operators. Ordinary scenarios still run without them. Workshop is the separate authoring tool for content; it is available without a live session or GM credentials.

## Current GM work

| Capability | Operator task | Delivery boundary |
|---|---|---|
| M1 peer and observation | Join/readiness, full map and bounded activity feed, attributed pause/resume, force-start and authentic Station puppeting. | Implemented; M1 exit #1300 closed. Puppeting respects fine-System Admission and Backfill eligibility. |
| M2 directing | Fire/pause/skip authored events, spawn/despawn, select doctrine/directives/Objectives, reveal/conceal, apply damage/healing/availability and puppet supported Stations. | Implemented; M2 replay exit #1316 closed. Typed actions share the scenario effect vocabulary. |
| M3 performing | Send bounded logged Comms as an existing actor, use templates/hails, compare truth with each crew's knowledge, switch private role presets. | Implemented feature code; **Plan:** human two-equal-GM live exit #1320. Presets filter UI, not permissions. |
| M4 facilitation | Prioritise pending Comms/beats/idle actors/disconnected Stations, inspect quiet-time and broad workload states, use mission widgets. | Implemented feature code; **Plan:** busy-session human exit #1440. Assistance is advisory. |
| M5 recovery | Inspect attributed history, undo supported invertible actions, create checkpoints/bookmarks and restore, inspect technical peer health. | Implemented feature code; **Plan:** live recovery human exit #1448. Technical reversibility differs from undoing fictional consequences. |
| M6 Live Inspector | Inspect loaded content through schema-backed domain panels, invoke named actions and checked NPC-doctrine transactions. | Implemented, including combined native ship/GM capability; **Plan:** golden/dock human acceptance #1487/#1514. Source/model/geometry/tuning fields remain read-only or recreate-required where declared. |
| M7 information control | Present observer-specific classification, ghost contacts and degraded/delayed reports. | Implemented; extends crew-reported knowledge without changing material world truth. |
| M8 in-sim presentation | Force/release Viewscreen modes, eligible incoming-Comms takeover, title cards and authored sound cues. | Implemented; scenarios and GMs use the same cue owner. External hardware is **Plan — unscheduled**. |
| M9 continuity/pacing | Carry campaign state across sessions and offer deeper pressure/beat-stacking/lull advice. | **Plan — T6**. Snapshots, campaign projection and attention tooling are foundations, not the completed feature. |

GM omniscient reads are local projections; authoritative actions cross the agreed deterministic command path with attribution. Several GMs are mechanically equal. A native process may combine one fixed ship slot and local GM identities while counting once in peer ordering and deterministic NPC ownership; browser GM remains GM-only.

## Current Workshop work

Browser and native Workshop share exact-source documents, one chronological history, runtime-equivalent candidate validation, diagnostics, recovery and captured previews. Browser authors one editable mod archive over immutable dependencies and exports a ZIP; native authoring saves to an explicitly selected project/mod root. Accepted asset packs can contain validated model, texture and sound members.

Authors can:

- edit worlds, composed entities, playable-ship Stations/Systems/ratings/doctrine, ship slots, role presets/widgets and definitions;
- edit inline/sibling Rhai with registry-derived completion and source-linked diagnostics;
- edit world composition and supported literal script references, and place anchors/Regions/spawns through a canvas or structured alternative;
- import and preview models/assets, edit rig/marker/variant/LOD data, and review native-only LOD/remesh or billboard-capture results before grouped adoption;
- validate, diff, undo/redo, recover a crash draft, save or export the exact candidate;
- run a fresh disposable Backfill Test from the unsaved draft, choose world/hull/seed, restart, pause, step or accelerate, inspect GM/player-ship views, preview roles, read host-call/Flag/callback traces and set scenario-state breakpoints.

Editing marks an existing Test stale; a restart creates a new simulation rather than hot-patching it. Authoring, Test and Live have separate authority and appropriate dock contexts. Panel placement is private presentation state, not content or simulation state. Old editor/viewer entry points redirect into Workshop.

**Plan — acceptance:** golden browser/native workflow and dock tests still require human completion. **Plan — unscheduled:** Rhai source stepping, expression breakpoints, local variables and stack frames. Current deterministic tick stepping and Flag/counter breakpoints are different capabilities.

## Player-experience implications

GM tools make it possible to facilitate a larger session, perform actors, introduce pressure and recover mistakes. Workshop makes new task structures and content cheaper to author and test. Neither proves the current scenarios keep every Station engaged. Compare authored no-GM play and facilitated play separately; record how much GM intervention is needed to maintain useful crew work.

## Canonical sources

- [GM roadmap](../../../pasm/spec/roadmap/gm-console-milestones.yaml), [T2 GM design](../../../pasm/spec/design/gm-console-t2.yaml) and [T3 GM design](../../../pasm/spec/design/gm-console-t3.yaml).
- [Information control](../../../pasm/spec/design/t4-information-control.yaml), [presentation](../../../pasm/spec/design/t4-presentation.yaml) and [audio/equivalents](../../../pasm/spec/design/t4-audio-feedback.yaml).
- [Workshop implementation index](../../../wiki/entities/editor.md), [Save Catalogues](../../../wiki/concepts/save-catalogues.md) and [Editor redirects](../../../editor.html).
- [Full current baseline](../foundation/current-baseline.md) and [Plan baseline](../future/roadmap-baseline.md).
