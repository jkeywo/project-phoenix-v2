---
title: Issue #489 - Ship config loader + verifier
type: source
tags: [issue, ship-config, stations, systems, ratings, verifier]
source_url: https://github.com/jkeywo/project-phoenix-v2/issues/489
status: open
updated: 2026-06-22
---

# Issue #489 - Ship config loader + verifier

## Status

Open implementation slice under PRD #487. Builds on ADR-0002 and the #488 wire
contract.

## Problem

The station/system architecture needs a pure Rust loader and verifier for the
future ship-config schema before runtime systems can consume it. The current
`lobby::stations_config` module is tied to legacy per-player-count station
tables and Bevy `Resource` use, so it is not the right home for the new model.

## Solution

Add `src/ship/config.rs` as a pure serde/TOML model:

- `ShipConfig`
- `StationConfig`
- `StationRatingConfig`
- `SystemInstanceConfig`
- `PowerGroupConfig`

The verifier accepts a list of registered system kind strings so #490 can wire
it to the real `SystemKind` registry later. Runtime lobby behavior and
`assets/entities/player_ship.toml` remain unchanged in this slice.

## Key decisions

- Place the new loader under `ship`, not `lobby`, because it models ship
  authoring rather than current lobby assignment policy.
- Use `StationId`, `SystemId`, and `PowerGroupId` from `core::messages`.
- Keep `SystemInstanceConfig.config` opaque as `toml::Value`; kind-specific
  parsing belongs with the future system registry.
- Reject ownerless systems unless `ai_only = true`.
- Reject duplicate system IDs, unknown system kinds, dangling rating references,
  rating references to systems owned by another station, unknown station owners,
  unknown power groups, duplicate rating names, empty IDs, and reserved `core`
  station IDs.

## Open user stories

Runtime consumption is deferred. Future slices wire the validated config into
the system registry, control-source resolver, lobby fixed roster, and rating
selection.

## Cross-references

- [PRD #487 - Station / Console / System architecture redesign](./prd-487-station-console-system-redesign.md)
- [Issue #488 - Station/System ADR](./issue-488-station-system-adr.md)
- [ADR-0002](../../docs/adr/0002-station-system-ship-config-contract.md)
- [player_ship.toml](./player_ship_toml.md)
