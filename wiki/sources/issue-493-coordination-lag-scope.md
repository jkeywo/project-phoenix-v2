---
title: Issue #493 - Coordination-lag scope
type: source
tags: [issue, stations, systems, coordination, ai, channel-3]
source_url: https://github.com/jkeywo/project-phoenix-v2/issues/493
status: open
updated: 2026-06-22
---

# Issue #493 - Coordination-lag scope

## Status

Open decision slice under PRD #487. Decision settled for the follow-up
channel-3 implementation slice (#494); no code changes are part of this issue.

## Problem

PRD #487 left one channel-3 question open: does the coordination-bus lag apply
to every channel-3 message, or only to AI-originated sends?

## Solution

The coordination-bus lag applies to **all channel-3 traffic**. The lag models
crew coordination and comprehension time, not just AI reaction latency.

The lag value remains the PRD's ship-wide TOML setting, defaulting to 2 seconds.
This issue settles only the scope and routing semantics; parsing and runtime
delivery belong to the channel-3 implementation slice.

## Key decisions

- Every channel-3 message is queued with
  `due_time = sent_at + ship_coordination_lag_secs`.
- Queued state captures the sender control origin at enqueue time, plus the
  target `SystemId` and typed payload.
- The target system's live control state is resolved at delivery time.
- At delivery time: AI targets consume the message; human targets receive a
  popup only when the captured sender origin is AI; human-originated messages
  delivered to human targets are suppressed.
- Channel 3 must not be used for authoritative immediate effects. Immediate
  state changes belong to channel 2 or to readable sim state. Red Alert is the
  canonical example: the sim-level Red Alert state changes immediately, AI
  systems can read it immediately, and any coordination chatter about it remains
  delayed if sent through channel 3.

## Open user stories

None. This issue exists to unblock the channel-3 coordination bus slice (#494).

## Cross-references

- [PRD #487 - Station / Console / System architecture redesign](./prd-487-station-console-system-redesign.md)
