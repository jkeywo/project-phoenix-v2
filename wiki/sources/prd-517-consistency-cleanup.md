---
title: PRD #517 - Consistency cleanup for the 9 coarse systems
type: source
tags: [prd, stations, systems, consoles, coordination, repair, navigation, serde]
source_url: https://github.com/jkeywo/project-phoenix-v2/issues/517
status: open
updated: 2026-06-23
---

# PRD #517 - Consistency cleanup for the 9 coarse systems

## Status

Open. Parent is PRD #487. Eight independently-grabbable slices (A1–A8 + D).

## Problem

The seven coarse-system conversion PRs (#491, #497, #498, #499, #500, #502,
#503) landed with several inconsistencies between systems and two consoles
(Repair, Navigation) were not converted at all. A hardcoded console list in
channel-3 routing, `serde_json` used outside the codec module, and a missing
`RatingChanged` broadcast were also identified.

## Solution

Eight independently-grabbable slices:

- **A1** - Shields → CoordinationEnqueue (replace direct SimOutbox push). Shipped:
  see [Issue #528](./issue-528-shields-coordination.md).
- **A2** - `RatingChanged` broadcast from `handle_station_rating_change`.
- **A3** - Captain station owns viewscreen system kind. Shipped:
  see [Issue #529](./issue-529-captain-viewscreen.md).
- **A4** - `Console::from_console_id` helper; delete hardcoded console list in
  `process_coordination_lag`. Covered by issue #523.
- **A5** - `serde_json` outside codec cleanup in `src/ship/coordination.rs`,
  `src/core/flag_kind.rs`, and `src/regions/effects.rs`. Shipped: see
  [Issue #524](./issue-524-serde-json-cleanup.md).
- **A6** - `SystemId` naming convention pinned in doc-block + wiki page. Shipped:
  see [Issue #525](./issue-525-systemid-naming.md).
- **A7** - Repair coarse-system conversion. Shipped: see
  [Issue #526](./issue-526-repair-control-system.md).
- **A8** - Navigation coarse-system conversion. Shipped: see
  [Issue #527](./issue-527-navigation-control-system.md).
- **D** - Docs: `wiki/concepts/coarse-system-migration.md`; one source page per
  slice; update status pages and index.

## Key decisions

- All channel-3 traffic routes via `CoordinationEnqueue`; no direct
  `SimOutbox` pushes of `CoordinationPopup` outside `process_coordination_lag`.
- `serde_json` references in `src/` are eliminated outside `src/core/codec.rs`.
- `SystemId` naming convention: lowercase kebab; coarse system = kind id
  (`tactical`, `helm`); fine system = kind+instance suffix
  (`phaser-fore`, `torpedo-tube-fore-port`); ownerless = capability id
  (`red-alert`, `viewscreen`).

## Open user stories

Blocked by #504 (power pull / channel 1), #505 (viewscreen / channel 2), and
#506 (comms hails / channel 2) for the full consistency story. A4 (issue #523)
is unblocked and can start immediately.

## Cross-references

- [PRD #487 - Station / Console / System architecture redesign](./prd-487-station-console-system-redesign.md)
- [Coarse-system migration concept](../concepts/coarse-system-migration.md)
- [Issue #523 - Console ID lookup](./issue-523-console-id-lookup.md)
- [Issue #524 - serde_json outside codec cleanup (A5)](./issue-524-serde-json-cleanup.md) — shipped
- [Issue #525 - SystemId naming convention (A6)](./issue-525-systemid-naming.md) — shipped
- [Issue #526 - Repair coarse-system conversion (A7)](./issue-526-repair-control-system.md) — shipped
- [Issue #527 - Navigation coarse-system conversion (A8)](./issue-527-navigation-control-system.md) — shipped
- [Issue #528 - Shields advisories through CoordinationEnqueue (A1)](./issue-528-shields-coordination.md) — shipped
- [Issue #529 - Captain exposes viewscreen-owned system (A3)](./issue-529-captain-viewscreen.md) — shipped
- [Issue #493 - Coordination-lag scope](./issue-493-coordination-lag-scope.md)
