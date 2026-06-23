---
title: Issue #524 - serde_json outside codec cleanup
type: source
tags: [prd-517, codec, serde, cleanup]
source_url: https://github.com/jkeywo/project-phoenix-v2/issues/524
status: shipped
updated: 2026-06-23
---

# Issue #524 - serde_json outside codec cleanup (PRD #517 A5)

## Status

Shipped. Parent: PRD #517 (slice A5).

## What was done

Removed all direct `serde_json::` calls outside `src/core/codec.rs`.

### Three violation sites found and resolved

| File | Action |
|---|---|
| `src/core/flag_kind.rs:32-33` | Deleted `serde_round_trip` test — identical coverage already exists in `codec.rs:1476` (`flag_kind_round_trips`). |
| `src/ship/coordination.rs:293-294` | Deleted `frequency_hint_payload_serde_round_trip` — covered by `codec.rs:2315` and `codec.rs:2323` (strict superset). |
| `src/regions/effects.rs:144-145` | Moved all 9 `RegionEffectKind` round-trip test functions + helper into `codec.rs` — this was the **only** coverage for `RegionEffectKind`; `codec.rs` had zero coverage for this type. |

### New codec.rs section

Added `// ── RegionEffectKind serde round-trips (moved from regions/effects.rs #524)` section at `src/core/codec.rs:2332` with:

- `region_effect_round_trip` helper
- 8 test functions covering all 7 `RegionEffectKind` variants plus boundary/negative/zero values

## Post-change state

`Get-ChildItem src -Recurse *.rs | Select-String "serde_json::" | Where { $_.Path -notlike "*core\codec.rs" }` → empty.
All 1945 `cargo test` tests pass.

## Cross-references

- [PRD #517 - Consistency cleanup](./prd-517-consistency-cleanup.md)
- [Codec Seam concept](../concepts/codec-seam.md)
