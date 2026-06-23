---
title: Issue #523 - Console ID lookup
type: source
tags: [issue, stations, systems, console, coordination, channel-3]
source_url: https://github.com/jkeywo/project-phoenix-v2/issues/523
status: open
updated: 2026-06-23
---

# Issue #523 - Console ID lookup

## Status

Open implementation slice under PRD #517 (A4), which is itself under PRD #487.

## Problem

`process_coordination_lag` in `src/ship_plugin.rs` resolves a station's
`console` id field back to a `Console` variant using a hardcoded nine-element
array of all variants followed by a linear search. This means adding or
renaming a `Console` variant silently rots the delivery path for channel-3
coordination popups.

## Solution

Add `Console::from_console_id(id: &str) -> Option<Console>` as an associated
function on `Console` in `src/core/messages.rs`, directly after
`station_console_id`. The helper covers all nine current consoles and returns
`None` for unknown ids.

Replace the hardcoded array + `.find` in `process_coordination_lag` with a
single `Console::from_console_id(console_id)` call.

## Key decisions

- `from_console_id` is an associated function on `Console`, symmetric with
  `station_console_id`, and lives in the same `impl` block.
- Return type is `Option<Console>`; `None` covers the unknown-id rejection
  required by the acceptance criteria and fits the existing `Option`-chaining
  call site.
- Tests live inline in `src/core/messages.rs` in a new `console_id_tests`
  module, co-located with the code under test.

## Open user stories

None. This slice closes PRD #517 acceptance criterion A4.

## Cross-references

- [PRD #517 - Consistency cleanup for the 9 coarse systems](./prd-517-consistency-cleanup.md)
- [PRD #487 - Station / Console / System architecture redesign](./prd-487-station-console-system-redesign.md)
- [Issue #493 - Coordination-lag scope](./issue-493-coordination-lag-scope.md)
