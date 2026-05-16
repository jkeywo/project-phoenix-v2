---
phase: code-review
reviewed: 2026-05-16T16:30:00Z
depth: deep
files_reviewed: 11
files_reviewed_list:
  - src/server/bridge.rs
  - src/entities/config_cache.rs
  - server.html
  - src/world/server.rs
  - src/world/content.rs
  - src/entities/map_config.rs
  - src/core/messages.rs
  - src/console/comms/inbox.rs
  - src/console/comms/client.rs
  - src/ai/core.rs
  - src/ai/server.rs
  - src/entities/config.rs
findings:
  critical: 0
  warning: 0
  info: 2
  total: 2
status: issues_found
---

# Phase Code Review: Issue #267 — Normalize Scenario→World vocabulary

**Reviewed:** 2026-05-16T16:30:00Z
**Depth:** deep
**Files Reviewed:** 12 source files + 1 HTML
**Status:** issues_found

## Summary

Verified the Scenario→World vocabulary rename across all 11 specified files plus `src/world/content.rs` (the scenario file-format module). All 10 acceptance criteria were checked via direct file reading, grep for old names, and `cargo test`. The Rust source and `server.html` are fully migrated. 1600/1600 tests pass.

**Result: PASS with 2 minor info items** (wiki documentation stale, two internal function names still use "scenario" vocabulary they could drop for consistency).

---

## Acceptance Criteria Verification

### AC #1: `wasm_load_scenario` → `wasm_load_world_content` ✅

| Location | Status |
|---|---|
| `src/server/bridge.rs:215` | `pub fn wasm_load_world_content(...)` calls `crate::config_cache::wasm_load_world_content(...)` ✅ |
| `src/entities/config_cache.rs:320` | `pub fn wasm_load_world_content(...)` implemented ✅ |
| Old name anywhere in `src/` | **Not found** ✅ |

### AC #2: `wasm_get_default_scenario_path` → `wasm_get_world_content_path` ✅

| Location | Status |
|---|---|
| `src/server/bridge.rs:222` | `pub fn wasm_get_world_content_path() -> Option<String>` ✅ |
| `src/entities/config_cache.rs:344` | `pub fn wasm_get_world_content_path()` reads `map_config.default_scenario` ✅ |
| `src/entities/config_cache.rs:562` | Native stub exists ✅ |
| Old name anywhere in `src/` or `server.html` | **Not found** ✅ |

### AC #3: `SCENARIO_CONFIG` → `WORLD_CONTENT_CONFIG` ✅

| Location | Status |
|---|---|
| `src/entities/config_cache.rs:82` | `static WORLD_CONTENT_CONFIG: RefCell<Option<ScenarioConfig>>` ✅ |
| Old name `SCENARIO_CONFIG` | **Not found** ✅ |

### AC #4: `ScenarioResource` → `WorldContentResource` ✅

| Location | Status |
|---|---|
| `src/entities/config_cache.rs:456` | `pub struct WorldContentResource(pub ScenarioConfig)` ✅ |
| `src/entities/config_cache.rs:499-501` | ConfigCachePlugin inserts `WorldContentResource(scenario_config)` ✅ |
| Old name `ScenarioResource` | **Not found** ✅ |

### AC #5: `get_scenario_config()` → `get_world_content_config()` ✅

| Location | Status |
|---|---|
| `src/entities/config_cache.rs:378` | `pub fn get_world_content_config() -> Option<ScenarioConfig>` ✅ |
| `src/entities/config_cache.rs:557` | Native stub ✅ |
| Callers in `src/world/server.rs:94,194,256` | All call `crate::config_cache::get_world_content_config()` ✅ |
| Old name `get_scenario_config` | **Not found** ✅ |

### AC #6: JS callers in `server.html` updated ✅

| Location | Old → New |
|---|---|
| `server.html:237` | `wasmBindings.wasm_load_world_content` ✅ |
| `server.html:238` | `wasmBindings.wasm_get_world_content_path` ✅ |
| `server.html:244` | `wasm_get_world_content_path()` call ✅ |
| `server.html:250` | `wasm_load_world_content(path, toml)` call ✅ |
| Old names | **Not found** in `server.html` ✅ |

### AC #7: Doc-comments updated to "world content" vocabulary ✅

| Location | Assessment |
|---|---|
| `src/entities/config_cache.rs:1-15` | Uses "world content" / "map, entity, and complexity TOML" ✅ |
| `src/entities/config_cache.rs:315-318` | `wasm_load_world_content` doc: "Load a world content TOML string" ✅ |
| `src/world/server.rs:26-29` | `WorldContentRuntime` doc: "Server-side runtime state for the currently active world content" ✅ |
| `src/world/content.rs:1-9` | Uses "scenario" as file-format term (correct per AC) ✅ |
| `bridge.rs:219` | "Return the `default_scenario` path" — references config field name (file format) ✅ |

### AC #8: No "scenario" as plugin/runtime concept in Rust source ✅

Checked every Rust source file in the scope. All remaining "scenario" references fit the file-format exemption:

- `TriggerAction::LoadScenario` — file format action enum variant ✅
- `ScenarioConfig` struct — file format TOML deserialization type ✅
- `parse_scenario` function — file format parser ✅
- `default_scenario`/`extra_scenarios` fields — TOML config field names ✅
- `on_scenario_unloaded` — AI condition string from TOML config ✅
- `comms/inbox.rs` `scenario_id` — file-origin ownership scope ✅
- `comms/inbox.rs` `unload_scenario` — file-origin lifecycle operation ✅
- `WorldContentRuntime.scenario_id` — file-format scope string ✅
- Test names — explicitly permitted by AC ✅

**Former runtime types verified gone:**
- `ScenarioPlugin` → `WorldPlugin` (bridge.rs:98) ✅
- `ScenarioRuntime` → `WorldContentRuntime` (server.rs:32) ✅
- `ScenarioResource` → `WorldContentResource` (config_cache.rs:456) ✅

### AC #9: `cargo test` passes ✅

**Result: 1600 passed, 0 failed, 0 ignored.** ✅

### AC #10: `ScenarioRuntime` renamed ✅

`ScenarioRuntime` no longer exists anywhere. Replaced by `WorldContentRuntime` at `src/world/server.rs:32`. ✅

---

## Info Items

### IN-01: Function names `spawn_scenario_entities` and `init_scenario_runtime` still use "scenario" vocabulary

**Files:** `src/world/server.rs:73,193,252`
**Issue:** Two internal Bevy systems still use "scenario" in their function names:
- `spawn_scenario_entities` (line 73/193)
- `init_scenario_runtime` (line 73/252)

While these are file-format references (they read from the parsed `ScenarioConfig` file format), renaming them to `spawn_world_content_entities` and `init_world_content_runtime` would be fully consistent with the rest of the vocabulary migration. The AC's exemption for file-format terms applies, so this is **informational** — not a warning.

**Fix suggestion:**
```rust
// In src/world/server.rs line 73:
(spawn_world_content_entities, init_world_content_runtime),
```

### IN-02: Wiki documentation still references old `wasm_load_scenario` API name

**Files:**
- `wiki/concepts/world-plugin.md:31`
- `wiki/sources/prd-119-stations-scenarios-comms.md:25,46`

**Issue:** Two wiki pages reference the old `wasm_load_scenario` export name. While wiki files are outside the AC scope (which targets Rust source and `server.html`), stale wiki documentation will confuse future agents. The wiki should be updated to reflect the new `wasm_load_world_content` name.

---

_Reviewed: 2026-05-16T16:30:00Z_
_Reviewer: the agent (gsd-code-reviewer)_
_Depth: deep_
