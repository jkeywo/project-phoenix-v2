---
title: Coarse-system migration
type: concept
tags: [stations, systems, migration, system-registry, prd-487, prd-517]
updated: 2026-06-23
---

# Coarse-system migration

Status of the migration from per-console message dispatch to the unified coarse-system control path. All 9 consoles must register a system kind, accept `ControlSystem` dispatch, gate on `ControlSourceResolver::policy_for`, and emit channel-3 traffic via `CoordinationEnqueue`.

## SystemId naming convention

Pinned by issue #525. All `SystemId` wire strings follow one of three patterns:

| Pattern | Rule | Examples |
|---------|------|---------|
| **Coarse system** | Lowercase kebab matching the system kind id | `"helm"`, `"tactical"`, `"red-alert"` |
| **Fine system** | Kind id + `-` + instance suffix | `"phaser-fore"`, `"torpedo-tube-fore-port"` |
| **Ownerless capability** | Bare capability id (lowercase kebab) | `"red-alert"`, `"viewscreen"` |

Multi-word ids always use hyphens (`-`), never underscores. The `*_SYSTEM_ID` constants in `src/ship/system_registry.rs` are the authoritative source; always use the helpers (`helm_system_id()`, `tactical_system_id()`, etc.) rather than inline string literals.

### `red_alert` vs `red-alert` quirk

The registry kind key uses `"red_alert"` (snake_case, `RED_ALERT_KIND`) for legacy reasons, while the wire `SystemId` is `"red-alert"` (kebab, `RED_ALERT_SYSTEM_ID`). All other systems have identical `*_KIND` and `*_SYSTEM_ID` values. New systems must use the same lowercase-kebab string for both.

## Coarse-system status (as of issue #529)

| Console | Kind registered | `ControlSystem` dispatch | `policy_for` gating | Channel-3 via `CoordinationEnqueue` | Issue |
|---------|----------------|--------------------------|---------------------|--------------------------------------|-------|
| Captain | ✅ `captain` | ✅ | ✅ | n/a | #499 |
| Helm | ✅ `helm` | ✅ | ✅ | ✅ | #497 |
| Tactical | ✅ `tactical` | ✅ | ✅ | ✅ | #491 |
| Power | ✅ `power` | ✅ | ✅ | n/a | #500 |
| Sensors | ✅ `sensors` | ✅ | ✅ | ✅ | #498 |
| Shields | ✅ `shields` | ✅ | ✅ | ✅ (#528) | #502/#528 |
| Comms | ✅ `comms` | ✅ | ✅ | ✅ | #503 |
| Viewscreen | ✅ `viewscreen` | ✅ | ✅ | n/a | #505 |
| Repair | ✅ `repair` | ✅ (#526) | ✅ (#526) | n/a | #525/#526 |
| Navigation | ✅ `navigation` | ✅ (#527) | ✅ (#527) | n/a | #527 |

## Fine-system ids (future, PRD C)

Fine-system decomposition (e.g. `"phaser-fore"`, `"torpedo-tube-fore-port"`) is tracked by issues #511–#515. These are out of scope for the coarse-system migration. Do not create fine-system registrations until those issues land.

## Key files

- `src/ship/system_registry.rs` — All `*_SYSTEM_ID`, `*_KIND`, `*_AI_CONTROLLER` constants and `*_system_id()` helpers.
- `src/ship/control_source.rs` — `ControlSourceResolver` and `policy_for`.
- `src/ship_plugin.rs` — `process_coordination_lag` delivers channel-3 messages.

## AI ship unification and per-kind AI plugins (PRD #520)

After `ControlSourceResolver` was established as the control-gating authority (PRD #517), PRD #520 extended it to NPC ships. Each coarse system now has (or will have) a dedicated `operate_<kind>_ai` Bevy system that runs after `AiTickLabel` and is gated on `policy_for(system_id).operate_ai`. This makes the AI/human split uniform: the same gate that prevents a human from driving a Backfill console also enables the per-kind plugin to operate it.

NPC ships carry `ShipSystemControlSources` seeded with `ControlSource::Ai` for all systems; player ships default to `ControlSource::Human` (modified by rating changes).

See [AI Ship Unification](./ai-ship-unification.md) for the full architecture.

## Cross-references

- [PRD #487 - Station / Console / System architecture redesign](../sources/prd-487-station-console-system-redesign.md)
- [PRD #517 - Consistency cleanup for the 9 coarse systems](../sources/prd-517-consistency-cleanup.md)
- [Issue #525 - SystemId naming convention](../sources/issue-525-systemid-naming.md)
- [PRD #520 - AI ship unification](../sources/prd-520-ai-ship-unification.md)
- [AI Ship Unification](./ai-ship-unification.md)
