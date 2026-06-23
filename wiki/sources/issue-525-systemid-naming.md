---
title: Issue #525 - Normalize SystemId naming where coarse systems are exposed
type: source
tags: [prd-517, system-registry, naming-convention]
source_url: https://github.com/jkeywo/project-phoenix-v2/issues/525
status: shipped
updated: 2026-06-23
---

# Issue #525 - Normalize SystemId naming (PRD #517 A6)

## Status

Shipped. Parent: PRD #517 (slice A6).

## What was done

### `src/ship/system_registry.rs` changes

- Added module-level `//!` doc-block documenting the three-tier naming convention (coarse / fine / ownerless), the lowercase-kebab rule, and the `red_alert` vs `red-alert` legacy quirk.
- Grouped constants into "Ownerless capability systems" and "Station-owned coarse systems" sections with per-constant `///` doc-comments.
- Added `REPAIR_SYSTEM_ID = "repair"`, `REPAIR_KIND = "repair"`, `REPAIR_AI_CONTROLLER = "repair_ai"` constants and `repair_system_id()` helper. Registration in `with_core_systems` follows in issue #526.
- Added helper section header `// ── SystemId helpers ──` with guidance to prefer helpers over inline literals.
- Added 3 new tests pinning convention:
  - `coarse_system_ids_are_lowercase_kebab` — asserts no uppercase, no underscores, non-empty for all 11 ids.
  - `coarse_system_id_values_are_stable` — pins the exact string value of each `*_SYSTEM_ID` constant.
  - `system_id_helpers_return_expected_values` — asserts each `*_system_id()` helper returns `SystemId(THE_CONST.into())`.
- Added 2 missing per-system tests: `core_registry_has_sensors_ai_controller`, `core_registry_has_shields_ai_controller`.

### `wiki/concepts/coarse-system-migration.md` created

New concept page with:
- Naming convention table
- Coarse-system status table (all 9+1 consoles with issue links)
- Fine-system forward reference
- Key files and cross-references

## Post-change state

`cargo test` passes (all tests including 14 new/changed tests in `system_registry.rs`).

## Cross-references

- [PRD #517 - Consistency cleanup](./prd-517-consistency-cleanup.md)
- [Coarse-system migration concept](../concepts/coarse-system-migration.md)
