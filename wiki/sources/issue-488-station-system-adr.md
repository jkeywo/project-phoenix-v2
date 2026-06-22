---
title: Issue #488 - Station/System ADR
type: source
tags: [issue, adr, stations, systems, wire, ship-config]
source_url: https://github.com/jkeywo/project-phoenix-v2/issues/488
status: open
updated: 2026-06-22
---

# Issue #488 - Station/System ADR

## Status

Open implementation slice under PRD #487.

## Problem

The station/system migration needs a locked contract before runtime behavior
changes begin: the future ship-config schema, stable `StationId`/`SystemId`
conventions, power groups, per-station rating tables, and the control-message
addressing shape.

## Solution

Create ADR-0002 and add minimal additive Rust wire scaffolding:

- `StationId`, `SystemId`, and `PowerGroupId` string newtypes.
- `ClientMessage::ControlSystem { target: SystemId, payload }`.
- `SystemControlPayload` typed command variants.
- `RepairTarget::Station(StationId) | Core`.
- Codec round-trip tests proving the existing `MessageCodec` contract remains
  unchanged.

The slice does not migrate `assets/entities/player_ship.toml`, the current
station loader, lobby behavior, console AI, or client/server routing.

## Key decisions

- New ids are stable designer-authored strings, not UUIDs.
- The future `[[station]]` schema replaces old `[stations]` promotion tables,
  but implementation must remain additive until the loader migration.
- Station ratings are explicit per-station tables, not derived from legacy
  `assets/complexity/*.toml`.
- Ownerless systems omit `station` and must set `ai_only = true`; `Core` is a
  repair/control destination, not a station id.
- Power group membership lives on each `[[system]]`; group metadata is separate.
- Power commands address the power system and name a `PowerGroupId`; the power
  system resolves group membership to per-system effective allocation updates.

## Open user stories

None in this slice beyond review/acceptance and keeping CI green. Runtime
stories remain in PRD #487 follow-up work.

## Cross-references

- [PRD #487 - Station / Console / System architecture redesign](./prd-487-station-console-system-redesign.md)
- [ADR-0002](../../docs/adr/0002-station-system-ship-config-contract.md)
- [Codec Seam](../concepts/codec-seam.md)
- [player_ship.toml](./player_ship_toml.md)
