# Project Phoenix — Alliance Convoy Escort

| Field | Value |
|---|---|
| Last updated | 2026-10-02 |
| Status | Current authored T5 reference; scale/pacing and human acceptance remain Plan |
| Authority | [Scenario design](../../../../pasm/spec/design/alliance-convoy-escort.yaml) and [world asset](../../../../assets/worlds/alliance_convoy_escort.toml) |

Four authored Alliance escort slots protect three civilian transports on their final crossing. Each slot offers an Alliance Destroyer or Cruiser; unclaimed ships use Backfill. The world is selectable in the base catalogue and excluded from the curated demo.

## Current player work

The shared task is keeping the transports alive until they reach safety. Addressed optional recovery and pursuit tasks distribute responsibilities among available ship slots instead of showing every bridge the same private instruction. Escorts use ordinary movement, targeting, weapons and ship support machinery.

The convoy opts out of civilian impulse travel and follows its authored route fraction. It has no twenty-minute fail timer. The PASM design records a no-combat crossing probe at 1,065.4 seconds; this is a route baseline, not a measured human session length or a pacing promise.

## Current ending

Every transport independently arrives or is destroyed. The mission ends only when all three fates are resolved, even if all escorts have already been destroyed. Narrative endings report the actual number that arrived rather than impose a binary success threshold. Wrecked crews remain in the spectator lifecycle while the convoy and surviving ships continue.

## Plan — T5 proof

Ratify scaling and pacing for the supported ship/crew combinations, close recovery/performance and integrated journeys, and complete real-network, long-session and human accessibility/language acceptance under #1090/#1153 and #1515–#1517. The existing world is a working reference, not completed generated Patrol or War content.

## Comparison questions

- Do the optional ship-specific tasks create useful divided work without leaving the escort problem unattended?
- Do bridges need to exchange information or coordinate, or can each play independently?
- How much of the crossing is judgement versus waiting or routine fire?
- Does losing an escort create useful remaining participation for that crew?

See [Current Baseline](../../foundation/current-baseline.md) and [Plan Baseline](../../future/roadmap-baseline.md).
