# Project Phoenix — Cruiser Elimination

| Field | Value |
|---|---|
| Last updated | 2026-10-02 |
| Status | Current authored competitive reference; broader T5 acceptance remains Plan |
| Authority | [Scenario design](../../../../pasm/spec/design/cruiser-elimination.yaml), [world asset](../../../../assets/worlds/cruiser_elimination.toml) and [runtime tests](../../../../tests/cruiser_elimination.rs) |

Two fixed Alliance Cruisers fight two fixed Dynasty player Cruisers. Unclaimed slots run on Backfill. The base catalogue offers this world; the curated public-demo catalogue does not. It provides a competitive team fight alongside Phoenix's cooperative scenarios.

## Current player work

Each bridge operates its own Stations, targeting, geometry, weapons, shield facings, power and repairs. Teammates can coordinate target pressure and survival while preserving each ship's control authority. The [Dynasty player Cruiser](../ships/dynasty-ships.md) has its own six-role bridge and strike-reserve decision; it is separate from the Harrow NPC Cruiser.

## Current ending

Named ship losses increment a team's counter once. Losing one ship moves its crew into cinematic spectating while surviving teammates continue. Elimination of one team awards the other team the win; elimination of both in the same resolved pass produces a draw. Shared localisable team report rows show the actual results instead of presenting a host-relative Victory banner to both sides.

## Plan — T5 proof

The roadmap's larger-session recovery, supported limits and human/network/language acceptance remain open. Reference starting geometry and existing seeded balance evidence do not by themselves prove fair or enjoyable human 2v2 play. This scenario does not implement the C7 strategic War run.

## Comparison questions

- Does coordination within and between bridges matter more than raw input rate?
- Are the two Cruiser control experiences distinct and legible?
- Do target/position/power decisions offer counterplay?
- How long do eliminated crews remain spectators, and is that enjoyable?

See [Current Baseline](../../foundation/current-baseline.md) and [Plan Baseline](../../future/roadmap-baseline.md).
