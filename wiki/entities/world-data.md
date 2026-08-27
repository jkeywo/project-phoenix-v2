---
title: World Data
type: entity
tags: [world, scenario, transform, ambient_light, snapshot, includes]
sources: [src/world/config.rs, src/world/server.rs, src/world/server_tests.rs, src/world/layers.rs, src/world/validate.rs, src/world/deadlines.rs, src/world/script/load.rs, src/world/script/schedule.rs, src/comms/scripted.rs, src/entities/config_cache.rs, src/snapshot.rs, src/server/bridge.rs, server.html, src/server/renderer.rs, src/entities/config.rs, src/entities/include_resolve.rs, tests/snapshot_resume.rs, assets/worlds/default.toml]
updated: 2026-08-27
---

# World Data

The TOML-defined layout for a root world or a supporting world layer: anchors,
entity instances, objectives, global ambient light, and the `[script]` block
carrying its scenario logic. The root loads at startup; supporting worlds can be composed at startup
or loaded during play.

## Source schema (`src/world/config.rs`)

```toml
# Hull-agnostic selectors: Station ids (console families) or System kinds.
scenario_detail_floor = ["navigation"]

[global]
seed = 42                                # optional; drawn from the OS when omitted

[ambient_light]                          # AmbientLightConfig in src/world/config.rs
color = [0.6, 0.55, 0.5]                 # sRGB; default Color::srgb(0.6, 0.55, 0.5)
brightness = 300.0                       # default 300.0

[anchors]
starbase = [0.0, 0.0, 0.0]               # normalised to [f32; 3]

[[entity]]                               # WorldEntity in src/world/config.rs
template_path = "assets/entities/station_axiom.toml"
name = "axiom"                           # optional; overrides EntityConfig.name
transform = { anchor = "starbase", offset = [10.0, 0.0, 0.0] }

[[entity]]
template_path = "assets/entities/star_sun.toml"
transform = { position = [0.0, 50.0, 0.0], rotation = [0.0, 0.0, 0.0], scale = [1.0, 1.0, 1.0] }
```

Scenario logic — event registrations, effects, and Comms dialogue — is authored
in `[script]` and documented under [World Plugin](../concepts/world-plugin.md).
Declarative `[[trigger]]` and `[[comms]]` blocks are rejected at load.

`scenario_detail_floor` is root-world-only: additive/supporting world loads reject
it rather than ambiguously overriding or discarding the selected scenario's
crew-detail contract. It is resolved only after the lobby-selected hull exists.
`write_scenario_detail_floor` matches each selector against that hull's Station
ids and System kinds, writes the concrete System ids to the LocalShip's
`ScenarioDetailFloor`, and clears stale values when the active world changes.

### Supporting-layer script and deadline lifecycle

A supporting world loaded through `extra_worlds` or `load_world` uses the same
`[script]`, `[[deadline]]` and `on_deadline` contract as the root. An inline
script compiles immediately. A top-level sibling `script = "child.rhai"` is
resolved beside the layer; in a browser the host retains the fetched TOML and
requests that sibling through the existing overlay-aware fetch bridge before
attempting activation. Pending, failed and legitimately empty script sources
are distinct states. A missing sibling refuses the whole layer once—entities,
handlers and deadlines never activate partially.

After compilation, the layer owns a deterministic list of literal
`ctx.effects.spawn_entity(...)` references from the exact resolved source set,
ordered by source path and then lexical line. Composition validation consumes
that list instead of rescanning the inline script, so a sibling reference is
not missed and an inline reference is not reported twice. Each error points at
the script file and line that authored it. Computed template-path expressions
remain legal and are resolved only when the script executes.

The candidate is composition-gated before entity UUID allocation, AST/handler
registration, deadline arming or `WorldLoaded`. Its doctrine anchors are the
union of the root world's anchors, anchors from currently active supporting
layers, and its own declarations. The union is recomputed before every queued
change: a successfully activated earlier layer can satisfy a later layer in
the same batch, while a later or rejected provider cannot. Browser validation
uses the overlay-aware template and fragment sources and preserves their
non-final pending-cache semantics.

Layer script state is owned by the layer path. The same sibling AST may be
shared by several layers, but triggers, callbacks, delayed effects, Comms opens
and deadline mutations carry `origin_layer` and read that layer's flag chain.
Local deadline ids are scoped by `(origin_layer, id)`, so root and child worlds
may all author `window`; Captain presentation qualifies child ids while keeping
the authored label. Child `due_secs` starts at the tick activation actually
lands and uses the root simulation cadence. Unload runs before callback drain,
removes owned rows and queued work, and releases a shared AST only after its
last owner; reload starts a fresh activation-relative window. Snapshot format
13 persists the ordered active composition, loader ownership, flags and
declared-layer entity identities. Resume removes bootstrap-only layers, loads
missing dynamic layers in captured activation order, treats a desired failed
sentinel as terminal rather than retrying, and only then restores callbacks,
deadlines and Comms state. That preserves index-aligned handlers and prevents
bootstrap duplicate arming.

## TransformConfig (`src/world/config.rs`)

Single struct replaces the old flat `position` / `anchor` / `relative_to` / `offset` fields on `WorldEntity`. Resolution precedence is defined by `TransformConfig::resolve` and `resolve_entity_position_with` in `src/world/config.rs`:

1. `relative_to = "<id-or-name>" + offset` — resolved against another `[[entity]]` in the **same world file**, named by its `id` or its `name`. Declaration order is irrelevant: `build_named_entity_positions` builds the whole lookup table before anything is positioned, so a target below the reference resolves as readily as one above it. A `name` beats another entity's `id` of the same spelling. The target must not itself use `relative_to` — chains are unsupported — and a reference that resolves to nothing is an `unresolved-relative-to` **error** that blocks activation of the whole world (`validate_relative_to`, issue #969), rather than dropping the one entity
2. `anchor = "<anchor-name>" + offset` — resolved against an entry in `[anchors]`
3. `position = [x, y, z]` — absolute
4. otherwise origin

Additional fields:
- `rotation: [f32; 3]` — XYZ Euler radians, applied via `Quat::from_euler(EulerRot::XYZ, x, y, z)`. Default `[0, 0, 0]`.
- `scale: [f32; 3]` — uniform-per-axis scale; default `[1, 1, 1]`. **Scale lives only on `TransformConfig`**; there is no `EntityConfig.scale` field.

## AmbientLightConfig (`src/world/config.rs`)

Optional top-level `[ambient_light]` block on the world TOML. `apply_world_ambient_light` in `src/server/renderer.rs` applies it in `PostStartup`, after `insert_world_config_resource` has placed `WorldConfig`. Missing fields use `render_setup::default_ambient_light`.

## EntityConfig name + lights

- `EntityConfig.name: Option<String>` in `src/entities/config.rs` is a template-level default. A `WorldEntity.name` override beats it. Both are stored as the `EntityName` component in `src/entities/spawner.rs`.
- `[[light]]` array-of-tables on `EntityConfig` spawns Bevy lights as children of the entity. Each `LightConfig` has `kind = "point" | "directional"`, `colour: [f32; 3]`, `intensity: f32`, optional `range: f32`. The `Lights` component in `src/entities/spawner.rs` carries them to `render_spawned_entities` in `src/server_app_render.rs`.
- `[mesh].emissive: Option<f32>` on `EntityConfig` controls the StandardMaterial emissive multiplier (renderer default `0.4`; star templates use `2.0`).

## Entity template composition (`includes`)

An entity template may declare an ordered top-level `includes = ["...", ...]`. The
paths resolve **relative to the declaring template**, are lexically canonicalised
(`\` → `/`, `.`/`..` collapsed), and are merged depth-first in declared order, with
the declaring template merged **last** so the includer wins.

### Compose and instance merge policies

Both layers call `entity_override::merge_entity_config_toml_with`, but they pass
different `MergePolicy` values because composing reusable fragments and
overriding one world instance have different array semantics.

| | `ComposeFragments` (`includes`) | `InstanceOverride` (`[[entity]].overrides`) |
|---|---|---|
| `behaviour.doctrine` | by `id` | by `id` |
| `[[system]]`, `[[station]]`, `[[shield_arc]]`, `[[weapons_console.phaser_banks]]`, `[[weapons_console.blaster_banks]]`, `[[torpedoes.tubes]]` | by `id` | replaces wholesale |
| `[[station.rating]]` | by `name` | replaces wholesale |
| `tags` | **unions** | **replaces** |
| `*.ai.rule`, `*_ai.state`, `*.selector.score`, `hull.system_hull` | replaces wholesale | replaces wholesale |
| `{ id = "…", _remove = true }` | removes that entry | not honoured — **the merge rejects the whole override** |
| authored `[]` | clears the list | clears the list |
| omitted key | leaves it alone | leaves it alone |

`tags` is the asymmetry to understand, because it is the one array with no key:
bare strings can only union or replace. A fragment library wants union. A world
override needs replace, because replace is the only way to take a tag *away* —
`default.toml`, `patrol.toml` and `reinforcements.toml` all override
`ship_harrow_patrol`'s tags precisely to drop `comms_contact`, and tags are
behaviourally live.

The tombstone is a **compose-layer marker only**, and writing one in a world
`[[entity]].overrides` is an error rather than a no-op:
`merge_entity_config_toml_with` returns `Result` and rejects any `_remove` key,
at any depth, under `InstanceOverride`. It has to: relying on the parser to
catch a stray marker does not work. `behaviour.doctrine` is the one array that
reconciles at that layer, so a tombstone written there deep-merges *into* the
matching template entry — and `DoctrineObjective` is **not**
`deny_unknown_fields`, so serde could ignore it and leave the doctrine present.
The merge therefore rejects every instance-override tombstone before parsing.

To take an entry away in a world override, restate the array without it, or
clear the whole array with `[]`.

`BehaviourConfig` is `deny_unknown_fields` and has no `behaviour.state` array;
a resolved document carrying one does not parse.

### What a fragment author writes

```toml
includes = ["fragments/escort_systems.toml"]

# EXTEND — a new id is appended to the fragment's suite.
[[system]]
id = "phaser-dorsal"
kind = "phaser_bank"

# REPLACE ONE ENTRY — a matching id deep-merges in place, keeping the
# fragment's other fields AND the entry's position.
[[system]]
id = "helm-thrust"
ai_only = false

# REMOVE ONE ENTRY — the tombstone. Never reaches the resolved document.
[[system]]
id = "legacy-probe"
_remove = true

# CLEAR THE LIST — still the whole-array lever, and it still wins.
shield_arc = []
```

Position is a **guarantee**, not a side effect: `[[shield_arc]]` order is
load-bearing (`ShieldSystem::from_arcs` maps arcs positionally, `focused_facing`
is a positional index, and the first arc's `frequency` seeds the ship-wide
shield frequency), so a specialised entry stays where the fragment put it and
only genuinely new entries append.

Arrays with no stable identity keep replacing wholesale, deliberately: the only
candidate key for `*.ai.rule` is the composite `(channel, priority)`, so an
author bumping a priority would silently "rename" a rule and get an append
instead of an edit. **A fragment contributing an AI policy contributes it
whole** — that is the intended granularity.

### Browser override preview

`editor/override-editor.js` has an independent `deepMerge` used for browser
preview and does not implement Rust's keyed array reconciliation. The
authoritative result is the Rust resolver and its validation; do not use the
browser preview to infer exact composed-array behavior.

Resolution is pure and lives in `src/entities/include_resolve.rs`. It returns one
resolved TOML document plus **provenance** — which template authored each dotted
field path and the include chain that reached it. Cycles, missing fragments,
unparseable fragments, malformed `includes` lists, and an invalid *resolved*
template are all load errors carrying the chain; only the fully resolved document
is ever validated or spawned. `includes` never reaches `EntityConfig`
(`deny_unknown_fields`), so nothing about composition exists at runtime.

Both hosts walk the same closure: natively via `FsFragmentSource` (used by
`FsTemplateLoader`, the headless template preload, and `build_layer_config_cache`),
and in the browser via `config_cache::drain_resolved_templates`, which reports the
fragments JS must still fetch through the existing `PENDING_QUEUE`/`IN_FLIGHT`
pair. Fragments are held as raw text only and never enter the config cache.

Shipped Alliance, Harrow, tender, and civilian hulls compose reusable AI policy
fragments from `assets/entities/fragments/ai/`. The parent
`assets/entities/fragments/` directory also holds the nested `npc_escort_core` /
`composed_escort` fixtures that exercise include ordering and provenance without
presenting fragments as root hull templates.

## Lifecycle

1. **Startup:** Trunk fires `wasm_load_world`; `parse_world` populates the `WORLD_CONFIG` thread-local.
2. **`WorldPlugin` startup chain:** `insert_world_config_resource` → `spawn_world_entities` → `init_world_runtime` → `load_extra_worlds`. Production always loads a world TOML via the WASM bridge; native unit tests without a `WorldConfig` see an empty world.
3. **Renderer backdrop:** `RendererPlugin` attaches the shared `assets/skybox/phoenix_space_cubemap.png` cubemap to `GameCamera`; it is independent of world TOML content.
4. **`apply_world_ambient_light` (`PostStartup`)** reads `WorldConfig.ambient_light` and inserts the `GlobalAmbientLight` resource.
5. **`WorldSetup` broadcast** carries the per-instance entity snapshots to clients on `GameStart` and re-broadcasts via `Welcome` to late joiners.
6. **Supporting layer changes:** `apply_world_layer_changes` composition-validates retained TOML and its exact resolved script sources before UUID minting, then atomically activates entities/triggers/deadlines and publishes `WorldLoaded` only afterwards. Validation sees root + currently active + candidate doctrine anchors and is recomputed per queued change. It runs before scripted callback draining, so an unload on a callback's due tick retracts the callback deterministically.
7. **For the rest of the session:** anchors and ambient light are immutable. Entities can be destroyed (asteroids, hull-zero stations); triggers, callbacks, deadlines, Comms and objectives mutate through authoritative runtime state.

## Related

- [Asteroid](./asteroid.md) · [Asteroid Field](../concepts/asteroid-field.md)
- [World Plugin](../concepts/world-plugin.md)
