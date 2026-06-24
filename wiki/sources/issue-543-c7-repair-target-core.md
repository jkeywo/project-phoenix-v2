---
title: Issue #543 C7 - RepairTarget::Core with Console::Core
type: source
tags: [prd-519, c7, repair, core, control-system]
source_url: https://github.com/jkeywo/project-phoenix-v2/issues/543
status: shipped
updated: 2026-06-24
---

# Issue #543 C7 - RepairTarget::Core With Console::Core

## Status

Shipped. Parent: [PRD #519](./prd-519-player-station-ai-backfill.md).

## What Changed

`Console::Core` represents ownerless ship-wide systems such as viewscreen/core targets. `RepairTarget::Core` now maps to `Console::Core` and dispatches a repair team normally instead of being a no-op. `Console::Core` is excluded from player-selectable stations.

Key code references:

- `src/core/messages.rs:300` - `Console::Core`.
- `src/core/messages.rs:914` - `RepairTarget::{Station, Core}`.
- `src/console/repair/server.rs:109` - station repair target mapping.
- `src/console/repair/server.rs:115` - `RepairTarget::Core => Console::Core`.
- `src/lobby/stations_config.rs:45` - Core skipped as a selectable station.

## Post-Change Contract

Repair dispatch through `ControlSystem` supports both station-owned consoles and core systems. Core is damageable/repairable but not claimable by a player.
