# TOML Authoring Guide

This guide documents every TOML file format used under `assets/`. It is grouped
by file type. Each section lists the file's purpose, where it lives, every
field accepted by the parser (with type and default), and a minimal example.

Authoritative parser locations are referenced inline so this guide can be
regenerated when the Rust types change.

**File groups:**

1. [Worlds](#1-worlds) — `assets/worlds/*.toml`
2. [Entity templates](#2-entity-templates) — `assets/entities/*.toml`
   - [Ships (player + NPC)](#22-ships)
   - [Stations](#23-stations)
   - [Asteroid fields](#24-asteroid-fields)
   - [Individual asteroids](#25-individual-asteroids)
   - [Stars](#26-stars)
   - [Planets](#27-planets)
   - [Regions](#28-regions)
3. [Factions](#3-factions) — `assets/factions/*.toml`
4. [Complexity presets](#4-complexity-presets) — `assets/complexity/*.toml`
5. [Cross-cutting sub-schemas](#5-cross-cutting-sub-schemas) — radar, behaviour, stations block

Conventions:

- Coordinates are `[x, y, z]` in world units. The XZ plane is the playfield;
  Y is up. Ship forward is `-Z` at yaw 0.
- Colours are RGB or RGBA `[r, g, b]` / `[r, g, b, a]` in `0.0–1.0`.
- All TOML paths are relative to the repo root (begin with `assets/...`).
- Fields marked **required** have no default and must be present.
- Fields marked **optional** show their default in parentheses.

---

## 1. Worlds

**Purpose:** a root or supporting content file for a session: anchors, the
static layout of entity instances, named trigger-eligible spawns, world-event
triggers, comms dialogues, and objectives. A root world can load supporting
worlds through `extra_worlds`, and triggers can load worlds during play through
`load_world`.

**Location:** `assets/worlds/*.toml` (e.g. `assets/worlds/default.toml`,
`assets/worlds/patrol.toml`).

**Parser:** `src/world/config.rs` owns the entire world schema. `parse_world`
is a single-pass deserializer that produces a `WorldConfig` carrying the
normalised anchor table, the `[[entity]]` list, `[[trigger]]` blocks, and
`[[comms]]` templates. The JS-facing loader is `wasm_load_world` in
`src/server/bridge.rs`, which delegates to `entities::config_cache` to
populate the `WORLD_CONFIG` thread-local. The Bevy startup chain
(`insert_world_config_resource → spawn_world_entities → init_world_runtime
→ load_extra_worlds`) consumes it. Asteroid-field and named `[[entity]]`
instances spawn via `world::server::spawn_world_entities`; anonymous
non-asteroid entries spawn via `server_app::setup_world` (PRD #337).

### Top-level fields

| Field | Type | Default | Notes |
|---|---|---|---|
| `title` | string | `""` | Lobby display title. |
| `description` | string | `""` | Lobby display body. |
| `[global]` | table | `{ seed = 42 }` | Global generation params. |
| `[anchors]` | table | `{}` | Named `[x,y,z]` waypoints referenced by `[[entity]] anchor = "..."` and AI patrols. |
| `[[entity]]` | array of tables | `[]` | Entity instances spawned into the world. Single block type for all spawnables; named entries (with `name = "..."`) are trigger / comms eligible. |
| `[[trigger]]` | array of tables | `[]` | World-event handlers (see §1.5). |
| `[[comms]]` | array of tables | `[]` | Comms dialogue templates (see §1.6). |
| `[ambient_light]` | table | none | World ambient light override; omitted sub-fields fall back to renderer constants. |
| `[dust]` | table | none | Ambient dust / velocity-mote effect (see §1.3). |

### `[global]`

| Field | Type | Default | Notes |
|---|---|---|---|
| `seed` | u64 | `42` | Seeds deterministic generators (asteroid spawner, etc.). |
| `sim_tick_hz` | f32 | `60.0` | Fixed rate (Hz) of the LOGICAL SIMULATION tick (issue #895). The whole `SimSet` chain runs in Bevy's `FixedUpdate` at this rate, so the simulation advances identically whatever the host's frame rate. Three load-time constraints: it **must be at least 30 Hz** — below that the helm integrator's 1/30 s step cap would silently shorten every step and the simulation would under-integrate — it **must be at most 240 Hz** — above that a single lagged frame can unpack into tens of thousands of `FixedUpdate` steps (`max_delta / timestep`) and wedge the host — and `sim_tick_hz / ai_tick_hz` **must divide exactly**, since the AI decision tick is counted in whole sim ticks. A world that breaks any of the three fails to load. |
| `ai_tick_hz` | f32 | `30.0` | Fixed rate (Hz) of the ONE shared AI decision tick. Gates **every** AI policy host — the six per-axis helm systems, shield focus, power allocation, torpedo load/auto-fire, frequency hint, phaser and blaster auto-fire, AI target selection — decoupling AI decision cadence from the host frame rate (issues #803, #889). Accepts the pre-#889 name `ai_helm_tick_hz` as an alias. |
| `ai_snapshot_hz` | f32 | `10.0` | Rate (Hz) of the derived slower AI cadence: the `WorldSnapshot`/doctrine-blackboard rebuild and the two hosts that read them (Captain, Sensors). Realised as a whole number of `ai_tick_hz` ticks — `ai_tick_hz / ai_snapshot_hz` **must divide exactly**, or the world fails to load (issue #889). |

### `[anchors]`

A flat table of `name = [x, y, z]`. Used by `[[entity]] anchor = "..."`
references
and by AI `waypoints = [...]` lists.

```toml
[anchors]
starbase_alpha = [500.0, 0.0, 0.0]
patrol_alpha   = [300.0, 0.0, -300.0]
```

**A world must declare every anchor the hulls it spawns steer to** (issue #888).
An entity template's `[[behaviour.doctrine]]` may name anchors — `Patrol` via
`directive_anchors`, `Reach`/`Retreat` via `directive_anchor` — and those names
resolve against this table, not against the template's home scenario. A
reference nothing in the composition declares is rejected at load, naming the
entity, the anchor and the world: unlike an entity name, an anchor has no
runtime source, so a lookup that misses at load misses forever and the ship
silently never pursues its goal.

The effective doctrine is what counts, so a scenario that does not want a
hull's route has a second option — stand the entry down in the instance's
`overrides`, by the id the template gave it:

```toml
overrides = { behaviour = { doctrine = [
  { id = "patrol-warhawk", directive_kind = "None", directive_anchors = [], directive_loop = false },
] } }
```

Note that this is a per-id edit, not a replacement: an `overrides` doctrine list
merges **by id** and keeps every template entry it does not name, so adding one
directive does not remove the rest.

Sub-worlds resolve against their base world's table as well as their own, so a
layer whose ships fly a route the base world declares needs no copy of it.

### `[dust]`

Ambient dust motes on the viewscreen. Parsed by `world::config::DustPfxConfig`,
resolved against renderer defaults in `server::pfx::DustPfxSettings::from_world`.

This is a camera-relative *velocity field*, not world-space particles: speed
drives density, luminosity and streak length, while the ship's true velocity
vector (forward **and** lateral) drives direction. Every field is optional; the
renderer supplies a near/mid/far layer set when the world declares none, so a
bare `[dust]` block is a valid way to take the defaults.

| Field | Type | Default | Notes |
|---|---|---|---|
| `enabled` | bool | `true` | Master switch; `false` skips the effect entirely. |
| `speed_curve_exponent` | f32 | `2.0` | Applied to normalised speed before every ramp. `2.0` keeps the field restrained at low speed and ramps it hard under acceleration. `1.0` is linear. |
| `low_speed_tint` | `[f32; 3]` | `[0.55, 0.65, 0.75]` | Mote RGB at rest — a cool grey-blue. |
| `high_speed_tint` | `[f32; 3]` | `[0.95, 0.98, 1.0]` | Mote RGB at full speed — near-white. |
| `streak_response_secs` | f32 | `0.10` | Smoothing time constant for streak length. |
| `brightness_response_secs` | f32 | `0.22` | Smoothing time constant for brightness. |
| `spawn_response_secs` | f32 | `0.50` | Smoothing time constant for density. |
| `centre_fade_inner` | f32 | `0.15` | Normalised screen radius inside which motes are fully hidden. |
| `centre_fade_outer` | f32 | `0.55` | Radius beyond which motes are fully visible. |
| `edge_fade` | f32 | `0.12` | Fraction of the screen half-extent over which motes fade before leaving view. |
| `turbulence` | f32 | `0.05` | Lateral drift as a fraction of speed. Keep low or the direction stops reading. |
| `mote_speed_multiplier` | f32 | `2.0` | Apparent mote speed relative to true ship speed. |
| `[[dust.layer]]` | array of tables | built-in near/mid/far | Depth layers. |
| `[dust.warp]` | table | disabled | Impulse warp field. |

The three response constants are deliberately staggered — streak length leads,
brightness follows, density lags — so acceleration reads immediately without
motes visibly popping into existence. Keep that ordering when retuning.

`centre_fade_*` keeps streaks out of the middle of the viewscreen, where they
would compete with targeting and navigation, and pushes them into peripheral
vision where they read as motion.

#### `[[dust.layer]]`

Zero or more depth layers. When present they replace the built-in set and are
matched to it **by position** — the first block inherits the near layer's
defaults, the second the mid layer's, the third and beyond the far layer's — so
author them near-to-far. Declaring one layer therefore yields a single near
layer, not a near layer plus the built-in mid and far.

Ranged fields are `[at_rest, at_full_speed]` pairs interpolated by the speed
curve.

| Field | Type | Default | Notes |
|---|---|---|---|
| `name` | string | layer default | Authoring label only; the renderer identifies layers by position. |
| `texture` | string | per layer | Mote texture relative to `assets/`. White RGB with the shape in alpha, so it can be tinted. |
| `max_motes` | u32 | per layer | Hard cap on live motes. |
| `spawn_rate` | `[f32; 2]` | per layer | Motes per second. |
| `opacity` | `[f32; 2]` | per layer | |
| `brightness` | `[f32; 2]` | per layer | Emissive multiplier; above ~1.0 feeds bloom. |
| `width` | f32 | per layer | Mote width as a **fraction of screen height**, not world units — scaled by spawn depth so a layer's apparent size is independent of its `depth_band`. Constant with speed; apparent growth comes from `length`. |
| `length` | `[f32; 2]` | per layer | Streak length as a multiple of `width`. `1.0` renders as a point. |
| `max_lifetime_secs` | f32 | `0.8` near, `2.0` mid, `4.0` far | Upper bound only. Actual lifetime is the time to transit the volume and pass the camera, so it falls out of speed and `depth_band`; this cap only bites at low speed, where it stops motes hanging in space. |
| `depth_band` | `[f32; 2]` | per layer | `[min, max]` distance from camera. |
| `edge_bias` | f32 | per layer | `0.0` uniform, `>0` weights spawns toward screen edges, `<0` toward the centre. |
| `additive` | bool | per layer | `true` = additive blending, `false` = alpha. Far layers should use alpha; additive stacking at high mote counts fogs the scene. |
| `glint_texture` | string | near layer only | Optional rare-glint texture. |
| `glint_chance` | f32 | `0.02` near, else `0.0` | Fraction of motes drawn as glints. Keep at a few percent. |

Built-in layer defaults (`server::pfx::DUST_DEFAULT_LAYERS`):

| Layer | Texture | `max_motes` | `spawn_rate` | `depth_band` | `additive` |
|---|---|---|---|---|---|
| near | `pfx/space_mote_streak_head.png` | `24` | `[0, 12]` | `[4, 25]` | `true` |
| mid | `pfx/space_mote_streak_soft.png` | `160` | `[5, 160]` | `[10, 70]` | `true` |
| far | `pfx/space_mote_compact_core.png` | `220` | `[10, 250]` | `[40, 150]` | `false` |

#### `[dust.warp]`

The impulse warp field (a dedicated high-speed layer rather than the ordinary
motes stretched indefinitely). Ramps in off the impulse charge progress and
fades the ordinary layers out as it takes over.

| Field | Type | Default | Notes |
|---|---|---|---|
| `enabled` | bool | `false` | Opt-in: an absent `[dust.warp]` means impulse leaves the ordinary layers running. |
| `texture` | string | `pfx/space_mote_streak_soft.png` | |
| `motes` | u32 | `40` | |
| `width` | f32 | `0.07` | |
| `length_multiplier` | f32 | `60.0` | Streak length at full warp, relative to `width`. |
| `brightness` | f32 | `3.0` | |
| `enter_secs` | f32 | `0.4` | Ramp-in time. |
| `exit_secs` | f32 | `0.6` | Ramp-out time. Timed render-side — disengaging impulse is instantaneous in the simulation, so this is purely visual. |

Minimal example — take every default and switch the warp field on:

```toml
[dust]
enabled = true

[dust.warp]
enabled = true
```

A tuned example lives in `assets/worlds/combat_test.toml`; `assets/worlds/default.toml`
is the minimal case above.

### `[[entity]]`

A single entity instance. Anonymous entries are static world layout;
entries carrying a `name` field become trigger- and comms-eligible (the
unified pipeline assigns them a stable UUID and registers `name → uuid`
for trigger and comms lookups). `relative_to` does **not** go through that
map — see the field notes below.

| Field | Type | Default | Notes |
|---|---|---|---|
| `template_path` | string | **required** | Path to entity template (e.g. `"assets/entities/star_sun.toml"`). |
| `id` | string | none | Stable instance ID for cross-reference. Also accepted as a `relative_to` target. |
| `name` | string | none | When set, the unified pipeline assigns a UUID and registers `name → uuid` in `WorldConfig.name_to_uuid` and `WorldContentRuntime.name_to_uuid`. Triggers and comms resolve names through this map. |
| `position` | `[f32; 3]` | `[0,0,0]` | World position. |
| `anchor` | string | none | Named entry from `[anchors]`; resolved to `[x,y,z]` at spawn time. |
| `relative_to` | string | none | Another `[[entity]]` **in the same file** to position relative to, named by its `id` **or** its `name` (a `name` wins if the two collide). Used with `offset`. Order does not matter — the target may be declared before or after. The target must use `anchor` or `position`, not another `relative_to` (no chains). Resolved against the entity list directly, *not* through `name_to_uuid`. A `relative_to` that does not resolve fails world validation and blocks the whole world from spawning (issue #969) — before, it cost exactly the one entity, silently. |
| `offset` | `[f32; 3]` | `[0,0,0]` | Offset added to the `relative_to` entity's resolved position. |
| `spawn_on` | `"immediate"` \| `"game_start"` | `"immediate"` | `"immediate"` spawns at world load (lobby phase); `"game_start"` spawns when phase enters `InProgress`. |
| `overrides` | inline table | none | TOML overrides merged on top of the template (per-instance field tweaks). |

Position precedence (when more than one is supplied): `relative_to` >
`anchor` > `position` > origin.

```toml
[[entity]]
template_path = "assets/entities/station_outpost.toml"
name          = "starbase_alpha"        # trigger / comms-eligible
position      = [500.0, 0.0, 0.0]

# NPC positioned at a named anchor
[[entity]]
template_path = "assets/entities/ship_harrow_patrol.toml"
name          = "raider_alpha"
anchor        = "patrol_alpha"

# Entity positioned relative to another named entity
[[entity]]
template_path = "assets/entities/ship_harrow_patrol.toml"
relative_to   = "starbase_alpha"
offset        = [10.0, 0.0, -5.0]
```

### 1.5 `[[trigger]]`

| Field | Type | Default | Notes |
|---|---|---|---|
| `condition` | string | **required** | One of `on_destroyed`, `on_attacked`, `on_timer`, `on_hailed`, `on_waypoint_reached`. |
| `entity` | string | depends | Required for entity-based conditions; references a named `[[entity]]` `name`. |
| `after_secs` | f32 | depends | Required for `on_timer`. |
| `waypoint` | string | none | Only for `on_waypoint_reached`. Names an anchor on the ship's route. Omit to fire on arrival at *any* waypoint of that ship's route. |
| `[[trigger.action]]` | array | `[]` | Actions to fire (in order). |

`on_waypoint_reached` fires when the named ship reaches a waypoint on the
`Patrol` or `Reach` objective it is currently following. "Reached" means the
ship came within that entity's `[behaviour] waypoint_arrival_radius` (see
§ `[behaviour]`) of the waypoint's anchor — tune that radius per entity rather
than expecting ships to land on an anchor exactly.

Triggers are single-shot: each fires at most once per session.

#### `[[trigger.action]]` types

Every action has a `type` field. Additional fields vary:

| `type` | Required fields | Optional | Notes |
|---|---|---|---|
| `add_objective` | `id`, `text` | `mandatory` (default `false`) | Add to the objectives list. |
| `complete_objective` | `id` | — | Mark complete. |
| `fail_objective` | `id` | — | Mark failed. |
| `set_ai_state` | `entity`, `state` | `target` | Force-set an AI controller's state. `target` may name another spawn whose UUID becomes the AI's target. |
| `apply_modifier` | `entity`, `tag`, `slot`, `bonus` | — | Add a modifier. `slot` ∈ {`MaxSpeed`, `MaxYawRate`, `RadarRange`, `PhaserDamage`, `HullDamageTaken`, `RepairRate`}. |
| `remove_modifier` | `entity`, `tag`, `slot` | — | Remove by `(tag, slot)`. |
| `apply_int_modifier` | `entity`, `tag`, `slot`, `int_bonus` | — | `slot` ∈ {`RepairTeams`}. |
| `remove_int_modifier` | `entity`, `tag`, `slot` | — | |
| `apply_flag` | `entity`, `tag`, `kind` (→ `flag_kind`) | — | `kind` ∈ {`CommsJammed`, `SensorBlind`}. |
| `remove_flag` | `entity`, `tag`, `kind` | — | |
| `game_over` | — | `message` | End the game with an optional message. |

**Removed actions** (no longer supported): `load_scenario`, `unload_scenario`.
Use `load_world` and `unload_world` for runtime composition.

### 1.6 `[[comms]]`

A comms template — a top-level message and a tree of player response choices.

| Field | Type | Default | Notes |
|---|---|---|---|
| `from` | string | **required** | Channel/source identity used for hailing, range, contact lookup, and synthetic broadcasts. Usually a named `[[entity]]` `name`; synthetic names such as `"Starcorp Command"` are allowed for broadcasts with no physical contact. |
| `speaker` | string | none | Optional display speaker for this root message. Use when the voice on the channel is a specific character distinct from the hailed contact, e.g. `speaker = "Dr. Myst"` on a message sent via `from = "Research Outpost"`. |
| `trigger` | trigger condition | **required** | When to deliver this message. Any `TriggerCondition` works: `on_hailed`, `on_destroyed`, `on_attacked`, `on_all_destroyed`, `on_world_loaded`, `on_timer`, `on_flag_set`, `on_flag_cleared`, `on_entered_region`, `on_exited_region`. Use `on_timer` + `after_secs` for time-delayed broadcasts (the migration target for the old `delay_secs` shortcut, now removed). |
| `entity` | string | depends | The named `[[entity]]` whose event triggers delivery (typically the same as `from`). Required by entity-scoped triggers (`on_hailed`, `on_destroyed`, etc.). |
| `entities` | array of strings | none | Required by `on_all_destroyed`. |
| `after_secs` | number | none | Required by `on_timer`. World-relative seconds. |
| `name` | string | none | Flag name. Required by `on_flag_set` / `on_flag_cleared`. |
| `message` | string | **required** | The root message body. |
| `[[comms.response]]` | array | `[]` | Player response options. |

#### `[[comms.response]]`

| Field | Type | Default | Notes |
|---|---|---|---|
| `text` | string | **required** | Display text on the response button. |
| `[[comms.response.action]]` | array | `[]` | Same shape as `[[trigger.action]]`. |
| `[comms.response.follow_up]` | table | none | Recursive: another `{ message, speaker?, trigger?, response... }` block presented after this choice. Follow-ups may set `speaker`; legacy `from` is accepted as a display-speaker alias, but new content should use `speaker`. If `trigger` is set, the thread shows a `...` placeholder while waiting for the trigger to fire (or fires immediately on the next tick if the trigger condition is already true — see "Triggered follow-ups" below). |

#### Triggered follow-ups

A `[comms.response.follow_up]` (or a chained `[comms.follow_up]`) can optionally carry a `trigger` field that delays delivery until a world condition is met. Supported trigger conditions mirror the `[[trigger]]` block; the most common shapes are:

| Trigger | Fields | Notes |
|---|---|---|
| `on_timer` | `after_secs` | Queue-relative — counts from the moment the follow-up is queued (the response is picked, or the parent message is injected), NOT from world load. Replaces the legacy `delay_secs` shortcut. |
| `on_entered_region` | `entity` | The named region entity. Fires when the player ship enters the region, OR immediately on the next tick if the ship is already inside. |
| `on_exited_region` | `entity` | Fires when the ship leaves the region, OR immediately if it is already outside. |
| `on_flag_set` | `name` | Fires when the named world flag transitions to set, OR immediately if it is already set. |
| `on_flag_cleared` | `name` | Fires when the named world flag transitions to cleared, OR immediately if it is already cleared. |
| `on_destroyed` | `entity` | Fires when the named entity is destroyed, OR immediately if its UUID is no longer in the live ECS set. |
| `on_all_destroyed` | `entities` | Fires when every named entity is destroyed. |
| `on_attacked` | `entity` | Event-only: fires when a fresh `Attacked` event is observed for the entity. Does not have an "already attacked" state to short-circuit on. |
| `on_hailed` | `entity` | Event-only: fires when a fresh `Hailed` event is observed. |
| `on_world_loaded` | (none) | Fires immediately (the world is, by construction, loaded). |

Worked example — Axiom Station acknowledges arrival when the player ship enters its dock region (`assets/worlds/before_the_fire.toml`):

```toml
[[comms]]
from    = "Axiom Station"
trigger = "on_world_loaded"
entity  = "Axiom Station"
message = "This is Axiom Station — we have a situation. Please respond."

  [[comms.response]]
  text = "Understood, Axiom Station. We are proceeding to your location."

    [comms.response.follow_up]
    trigger = "on_entered_region"
    entity  = "Axiom Station Dock"
    message = "Ardent, we have you on the dock approach. Welcome to Axiom."
```

`from` is the radio endpoint; `speaker` is the voice currently talking on that endpoint. This lets one chat thread stay anchored to a station while multiple characters speak inside it:

```toml
[[comms]]
thread_id = "research-scholar"
from      = "Research Outpost"
trigger   = "on_hailed"
entity    = "Research Outpost"
message   = "A.E.V. Ardent, this is the Research Outpost. Stand by — patching you through to Dr. Myst now."

  [[comms.response]]
  text = "Patch them through."
    [comms.response.follow_up]
    speaker = "Dr. Myst"
    message = "Ardent, this is Dr. Myst. The resonance signature is getting stronger."
```

### Example — a declarative world, end to end

> **The shipped `assets/worlds/default.toml` no longer looks like this.** Issue
> #984 converted it — triggers *and* comms — to a single `[script]` block, so
> the real file is now the worked example of the Rhai form rather than the
> declarative one. The listing below is kept because `[[trigger]]` / `[[comms]]`
> are still parsed, `combat_test.toml` still authors them, and this is the shape
> a conversion starts from.

```toml
title = "Default Patrol"
description = "Pirate raider patrol around Starbase Alpha."

[global]
seed = 42

[anchors]
starbase_alpha = [500.0, 0.0, 0.0]
patrol_alpha   = [300.0, 0.0, -300.0]

# Static map-half layout (anonymous — not trigger-eligible)
[[entity]]
template_path = "assets/entities/star_sun.toml"
position = [0.0, 0.0, 0.0]

[[entity]]
template_path = "assets/entities/alliance_cruiser.toml"
id = "player-ship"
position = [150.0, 0.0, 0.0]
spawn_on = "game_start"

# Named [[entity]] (trigger / comms-eligible — PRD #339 slice 2)
[[entity]]
template_path = "assets/entities/station_outpost.toml"
name          = "Starbase Alpha"
position      = [500.0, 0.0, 0.0]

# Named NPC at an anchor (PRD #337 slice 3)
[[entity]]
template_path = "assets/entities/ship_harrow_patrol.toml"
name          = "raider_alpha"
anchor        = "patrol_alpha"

[[trigger]]
condition = "on_destroyed"
entity    = "raider_alpha"

  [[trigger.action]]
  type = "add_objective"
  id   = "raider_killed"
  text = "Pirate raider destroyed."

[[comms]]
from    = "Starbase Alpha"
trigger = "on_hailed"
entity  = "Starbase Alpha"
message = "USS Phoenix, this is Starbase Alpha. Please state your business."

  [[comms.response]]
  text = "We require docking clearance."
    [[comms.response.action]]
    type      = "add_objective"
    id        = "obj-dock"
    text      = "Dock at Starbase Alpha."
    mandatory = true
```

---

## 2. Entity templates

**Purpose:** describe what *one kind of thing* is — its hull, collider,
appearance, consoles, AI behaviour, region effects, etc. Templates are
referenced from world files by `[[entity]] template_path = ...`. Trigger-
and comms-eligible instances additionally carry a `name = "..."` on the
same `[[entity]]` block.

**Location:** `assets/entities/*.toml`.

**Parser:** `src/entities/config.rs` → `EntityConfig::from_toml`.

`EntityConfig` is a single universal struct. The *presence* of optional
sub-tables (`[mesh]`, `[ship]`, `[shape]+[effects]`, `[asteroid_field]`,
console blocks) and arrays (`[[light]]`) determines what kind of entity is
spawned. Stars and planets are no longer dedicated sections — they are simply
entities with `[mesh]` (and, for stars, `[[light]]`).

### 2.1 Common top-level fields (any entity)

| Field | Type | Default | Notes |
|---|---|---|---|
| `name` | string | `""` | Display name (used by trigger/comms targeting and shown in the editor). |
| `tags` | array of strings | `[]` | Used for filtering (radar `shows`, AI faction lookups, etc.). Known tags: `asteroid`, `asteroid_field`, `ship`, `star`, `planet`, `region`, `station`. Free-form tags are also allowed. |
| `faction` | UUID string | none | Faction UUID this entity belongs to (matched against `assets/factions/*.toml`). |
| `[mesh]` | table | none | Visual mesh + material. See **§2.1.4**. |
| `[[light]]` | array of tables | `[]` | Point or directional lights attached to the entity. See **§2.1.5**. |
| `[collider]` | table | none | Rapier collider. See below. |
| `[appearance]` | table | none | Visual hints (colour, size range). |
| `[hull]` | table | none | Hull integrity. See below. |
| `[behaviour]` | table | none | AI controller. See **§6.2**. |

### 2.1.1 `[collider]`

| Field | Type | Default | Notes |
|---|---|---|---|
| `shape` | `"Ball"` \| `"Capsule"` | **required** | Collider primitive. |
| `radius` | f32 | **required** | Radius in world units. |
| `length` | f32 | **required** | Capsule length (use `0.0` for `Ball`). |

### 2.1.2 `[appearance]`

| Field | Type | Default | Notes |
|---|---|---|---|
| `colour` | string | **required** | CSS-style hex (e.g. `"#ff0000"`). |
| `size_min` | f32 | **required** | Lower bound for visual size jitter. |
| `size_max` | f32 | **required** | Upper bound. |

### 2.1.3 `[hull]`

Three mutually compatible representations — pick the one that fits the entity
kind.

| Field | Type | Default | Notes |
|---|---|---|---|
| `hull_integrity` | f32 | `0.0` | **Legacy single-value HP.** Used for stations and asteroids. |
| `captain_chair` | f32 | none | HP for a single `CaptainChair` console slot. Used by NPC ships. Takes precedence over `hull_integrity` when set. |
| `[[hull.console_hull]]` | array | `[]` | **Per-console hull slots.** Required for player ships. Each entry has `console = "<Console name>"` and `max_hp = <f32>`. Console names match the `Console` enum: `CaptainChair`, `Helm`, `Tactical`, `Repair`, `Sensors`, `Shields`, `Navigation`, `Power`, `Comms`. |
| `repair_team_count` | u32 | `0` | Number of dispatchable repair teams (player ship typically 2). |

### 2.1.4 `[mesh]`

The viewscreen visual for an entity. Entities without `[mesh]` are not rendered
in 3-D (they still exist on radar and participate in collisions).

| Field | Type | Default | Notes |
|---|---|---|---|
| `shape` | `"sphere"` \| `"cuboid"` \| `"torus"` | **required** | Mesh primitive. |
| `colour` | `[f32; 3]` | **required** | Linear RGB (0–1). |
| `radius` | f32 | `0.0` | Sphere radius, or torus major radius. Ignored for `cuboid`. |
| `size` | `[f32; 3]` | none | Full XYZ dimensions for `cuboid`. |
| `minor_radius` | f32 | none | Torus minor radius. |
| `emissive` | f32 | `0.4` | Emissive multiplier. Set high (e.g. `2.0`) for self-lit objects like stars. |

### 2.1.5 `[[light]]`

Zero or more lights attached to the entity. A single light is inlined as a child
of the entity transform; multiple lights are spawned as children.

| Field | Type | Default | Notes |
|---|---|---|---|
| `kind` | `"point"` \| `"directional"` | **required** | Light type. |
| `colour` | `[f32; 3]` | **required** | Linear RGB. |
| `intensity` | f32 | **required** | Candela (point) or lux (directional). |
| `range` | f32 | `50.0` | Effective falloff range. Point lights only; ignored for `directional`. |

### 2.2 Ships

A "ship" is an entity with a `[collider]`, `[hull]`, and one or more
`*_console` blocks. The player ship additionally has a `[stations]` block
defining bridge crew layouts; see **§6.3**.

#### `[helm_console]`

| Field | Type | Default | Notes |
|---|---|---|---|
| `max_speed` | f32 | `0.0` | Forward speed cap (units/s). |
| `max_reverse_speed` | f32 | `0.0` | Reverse cap. |
| `acceleration` | f32 | `0.0` | Acceleration (units/s²). |
| `deceleration` | f32 | `0.0` | Deceleration when thrust = 0. |
| `max_yaw_rate` | f32 | `0.0` | Max yaw (rad/s). |
| `low_speed_turn_boost` | f32 | `0.0` | Extra turn authority for flying slow. Effective yaw rate is `max_yaw_rate * (1 + X * (1 - speed/cap))`: x`1+X` at a dead stop, lerping to x1 at the speed cap. Light hulls author the most, capital hulls `0`. |
| `radar_range` | f32 | `0.0` | Helm overlay radar range. |
| `radar_shows` | bool | `false` | Whether helm radar overlay is enabled. |
| `power_multipliers` | `[f32; 4]` | none | Bonus values per power level 0..3. See power model. |
| `complexity_toml` | string | none | Path to a `complexity/*.toml` for this console. |
| `impulse_charge_duration` | f32 | `3.0` | Seconds to charge impulse drive. |
| `impulse_speed_multiplier` | f32 | `10.0` | Speed multiplier while impulse is active. |

#### `[weapons_console]`

| Field | Type | Default | Notes |
|---|---|---|---|
| `radar_range` | f32 | `0.0` | Tactical radar range. |
| `target_range` | f32 | `0.0` | Max lock range. |
| `fire_arc` | f32 | `0.0` | Half-angle of forward fire arc (rad). |
| `beam_range` | f32 | `0.0` | Phaser beam range. **This is the reach, full stop** — no power level, region effect or damage state scales it (issue #955); see *Power levels are gameplay numbers* below. |
| `beam_damage_per_sec` | f32 | `0.0` | Beam DPS. |
| `beam_duration_secs` | f32 | `0.0` | Time beam stays on per fire. |
| `cooldown_secs` | f32 | `0.0` | Cooldown after firing. |
| `beam_color` | `[f32; 4]` | `[]` | RGBA beam colour. Empty → renderer default. |
| `power_multipliers` | `[f32; 4]` | none | See power model. |
| `complexity_toml` | string | none | Per-console complexity preset path. |

#### `[engineering_console]` — repair tuning

| Field | Type | Default | Notes |
|---|---|---|---|
| `repair_rate` | f32 | `0.0` | HP/sec during the Repairing phase. |
| `repair_hp_per_cycle` | i32 | `0` | (Legacy) per-cycle HP. |
| `repair_cooldown_secs` | f32 | `0.0` | Cooldown between cycles. |
| `cooldown_secs` | f32 | `0.0` | General cooldown. |
| `complexity_toml` | string | none | Preset path. |

#### `[captain_console]`

| Field | Type | Default | Notes |
|---|---|---|---|
| `complexity_toml` | string | none | Preset path. Block exists only to attach a preset. |

#### `[sensors_console]`

There is no `power_multipliers` here since issue #952 removed the `sensors`
power group. No radar horizon is power-driven any more: `RadarRange`,
`SensorRadarRange` and `HelmRadarRange` are all written only by system damage
and region effects.

| Field | Type | Default | Notes |
|---|---|---|---|
| `[sensors_console.long_range_radar]` | RadarConfig | default | See **§6.1**. |
| `complexity_toml` | string | none | |

#### `[shields_console]`

Tunes the four-quadrant shield *focus* mechanic, and carries the `shields`
power group's per-level curve.

| Field | Type | Default | Notes |
|---|---|---|---|
| `power_multipliers` | `[f32; 4]` | none | Bonus per shields power level 1..4. Drives `ModifierSlot::ShieldRegen`, which scales every `[[shield_arc]]`'s `regen_per_sec` (issue #952). It does **not** scale the offline delay or focus decay. Moved here from `[sensors_console]`. |
| `focus_bonus_max_hp` | i32 | `50` | Extra max HP on the focused facing. |
| `focus_bonus_regen` | f32 | `5.0` | Extra regen/s on the focused facing. |
| `focus_penalty_max_hp` | i32 | `25` | Max HP subtracted from each non-focused facing. |
| `focus_penalty_regen` | f32 | `2.5` | Regen/s subtracted from each non-focused facing. |
| `focus_decay_rate` | f32 | `10.0` | HP/s decay applied to non-focused facings when above reduced max. |
| `complexity_toml` | string | none | |

#### `[navigation_console]` and `[navigation_console.system_chart]`

The `system_chart` sub-table is a `RadarConfig` (**§6.1**) describing the
chart pushed to the viewscreen.

#### `[power]`

| Field | Type | Default | Notes |
|---|---|---|---|
| `capacity` | f32 | **required** | Total battery capacity. |
| `rates` | `[f32; 6]` | **required** | Per-level drain/regen rates (level 0..5). Negative = recharge. |
| `emergency_threshold` | f32 | **required** | Recovery threshold, in the same ABSOLUTE units as `capacity`. A reactor locked out by a flat battery stays locked until the charge climbs back to this level. Also published on `PowerBatteryBlackboard` so the gauge can paint the reserve band. |

##### The exhaustion lock

There are no per-group battery floors. When the battery is drained to **empty**,
the reactor browns out completely: every group is forced to level 1 and the
allocation controls freeze (both increase and decrease no-op) until the charge
recovers past `emergency_threshold`. A player who mismanages power loses the lot
— that is the intended consequence, not a bug.

(The per-group `[power.battery_floor]` ladder issue #952 briefly introduced —
which held each group at an authored floor rather than locking — was reverted.
`[power.battery_floor]` and `battery_floor_release_margin` are no longer valid
fields and will fail the entity load if authored.)

Avoidable brownouts are prevented on the **AI side**, not by the reactor: each
`[[power.ai_policy.rule]]` carries a `min_reserve_<group>` param referenced by
its `when` guard (`fact(battery_pct) >= param(min_reserve_<group>)`), so a
below-reserve group is never elevated. An AI-crewed hull stops spending before
it drains the battery; a human Power officer, who has no such guard, can still
drive it flat and trip the lock.

```toml
[power]
capacity = 70
rates = [ 5, 4, 3, 2, -2, -5 ]
emergency_threshold = 20
```

#### Power levels are gameplay numbers — and what each group actually buys

**Read this before authoring or retuning any range on a hull.** The power model
is not a side system: three of its groups multiply values you author elsewhere,
through `ModifierSlot`s that `apply_power_modifiers_from_read_state`
(`src/modifiers/coordination.rs`) writes every tick.

| Power group | Modifier slot | What it scales |
|---|---|---|
| `helm` | `MaxSpeed`, `MaxYawRate` | `[helm_console] max_speed` / `max_yaw_rate` |
| `weapons` | `PhaserDamage` | every bank's `beam_damage_per_sec` |
| `shields` | `ShieldRegen` | every `[[shield_arc]]`'s `regen_per_sec`. Authored on `[shields_console] power_multipliers`. |

`sensors` was the third group until issue #952 and drove `ModifierSlot::RadarRange`; it is gone. No power group writes `RadarRange` now — a hull's acquisition horizon is exactly its authored `[weapons_console.radar] range`, reduced only by radar hull damage and `RegionEffectKind::RadarDampening`.

The bonus per level is the console's own `power_multipliers` (**§2.2**), and
`ShipModifiers::rebuild_cache` folds the summed bonus as `1 + sum` when it is
positive and `1 / (1 + |sum|)` when it is negative. With the fleet-standard
`[-0.5, 0, 0.25, 0.5]` that gives:

| Level | Bonus | Multiplier |
|---:|---:|---:|
| 1 | −0.5 | **×0.667** |
| 2 | 0.0 | **×1.000** |
| 3 | +0.25 | ×1.25 |
| 4 | +0.5 | ×1.5 |

**Power does not buy REACH.** A phaser bank reaches its authored `beam_range`, a
blaster its authored `range` and a torpedo `speed × lifespan`, at every power
level, in every region, at any hull damage. If you write 40 in a file, the gun
reaches 40.

That is worth stating loudly because it was not always true. Until issue #955
every firing path multiplied `beam_range` by `RadarRange`, so a hull resting
`[power_groups.sensors]` at 1 — which all four Alliance hulls do, to keep their
four groups at a total their reactor sustains — fought at two thirds of every
range its own file authored, silently. Issue #923 compensated for that with an
authored red-alert `sensors` elevation, paid for by dropping the `weapons` one.
#955 removed the multiplication instead, so both halves are gone: there is no
`sensors` channel in `fragments/ai/fleet_baseline.toml` and the red-alert point
is back on `weapons`, where it buys ×1.25 damage.

What the `sensors` group still buys is **acquisition**: how far the ship can
paint and lock a contact, bounded by the radar ranges the hull authors. A hull
with its sensors down shoots just as far — it simply has less warning about what
to shoot at. When you retune this group, you are tuning what a ship can SEE.

Three consequences a doctrine author designs around:

* **A fighting ring, commit range or standoff is sized against the authored
  weapon range**, not against a power-dependent one. There is no low rung to
  hedge a REACH against any more.
* **Acquisition still bounds engagement, and it is still power-scaled.** A lock
  is a precondition for firing, so `[weapons_console.radar] range ×
  RadarRange` is a ceiling on what a hull can actually engage even though it is
  not a ceiling on how far the gun shoots. And because there is no `sensors`
  channel in any authored power policy, an AI-crewed hull never leaves level 1 —
  the live horizon is **×0.667 of the authored number, permanently**, not
  occasionally. So `[weapons_console.radar] range` must be authored **above the
  hull's longest gun with room to spare**: comfortably above its longest
  `beam_range` / blaster `range`, and above the outer edge of any engagement
  envelope it flies (`max_artillery_range`), because the doctrine leg that steps
  outside that envelope still needs the lock it is repositioning against. Pinned
  for the whole fleet by
  `every_hulls_acquisition_horizon_clears_its_longest_gun_at_rest`
  (`src/modifiers/coordination.rs`), which fails naming the hull and the value to
  author. A hull that authors no `[weapons_console.radar]` at all is unbounded
  and exempt.
* **A range derived from a TARGET's reach is the same number the target's file
  authors.** The `safe_range_margin` recovery ring adds the margin to
  `target_direct_fire_range`, which `entity_direct_fire_range` reads off that
  ship's live banks — offline banks drop out, nothing scales what is left.

Pinned by `direct_fire_reach_ignores_the_radar_range_slot` (`src/ai/server.rs`),
`phaser_reach_is_the_authored_beam_range_and_ignores_the_radar_range_slot`
(`src/console/weapons/server_tests.rs`) and
`a_cruisers_phaser_reach_never_leaves_its_authored_beam_range_in_a_live_duel`
(`tests/headless_runner.rs`).

### 2.2.1 Example — `assets/entities/alliance_cruiser.toml`

The player-selectable hulls are `alliance_courier`, `alliance_destroyer`,
`alliance_cruiser`, and `alliance_battleship`. There is **no**
`player_ship.toml`.

**Do not author player identity in a hull template.** The `player` tag and the
`playerShip` radar icon are injected by the engine at player spawn
(`spawn_game_start_entities` in `src/server_app.rs`), scoped to the single hull
the local player actually flies. A hull template carries only `tags = ["ship"]`
and an ordinary `icon = "ship"` — the same file is spawned as an NPC when a
world places a copy of it, and authoring `"player"` there would make every NPC
copy masquerade as the player (answering `player`-only radar filters, drawing
the player blip). The ordinary icon is the `icon` field of `[radar_appearance]`
(**§6**), left as `"ship"`.

```toml
name = "Alliance Cruiser"
tags = ["ship"]
faction = "aaaaaaaa-1111-4111-8111-aaaaaaaaaaaa"

[radar_appearance]
icon = "ship"   # ordinary ship blip; `playerShip` is injected at spawn

[collider]
shape = "Capsule"
radius = 3.0
length = 6.0

[hull]
repair_team_count = 2

[[hull.console_hull]]
console = "Helm"
max_hp = 25.0

[[hull.console_hull]]
console = "Tactical"
max_hp = 25.0

[helm_console]
max_speed = 50.0
acceleration = 16.7
deceleration = 50.0
max_yaw_rate = 1.5708
impulse_charge_duration = 3.0
impulse_speed_multiplier = 10.0

[power]
capacity = 100.0
rates = [6.0, 5.0, 4.0, 2.0, -2.0, -6.0]
emergency_threshold = 25.0

[stations]
min_players = 1
max_players = 6

[[stations.1]]
name = "Captain"
description = "Solo crew."
consoles = ["CaptainChair", "Helm", "Tactical", "Repair", "Power", "Sensors", "Shields", "Navigation", "Comms"]
rank = "Cpt."
next = "Helm"
# … see §6.3 for the full station shape …
```

### 2.2.2 Example — a minimal NPC ship

NPC ship: legacy single-slot hull, AI behaviour, no `[stations]`.

> **Historical.** This excerpt was taken from `pirate_raider.toml`, which issue
> #892 retired as a display-name duplicate of `ship_harrow_destroyer.toml`. It
> was already stale before that — the Pirate faction went in #472 and the
> `[behaviour]` FSM dissolved in #572 — so read it as the *shape* of a minimal
> NPC hull, not as shipped content. For a current one see
> `assets/entities/ship_harrow_patrol.toml`.

```toml
tags = ["ship", "npc", "enemy"]
faction = "bbbbbbbb-2222-4222-8222-bbbbbbbbbbbb"

[hull]
captain_chair = 60.0

[collider]
shape = "Capsule"
radius = 2.0
length = 4.0

[helm_console]
max_speed = 70.0
acceleration = 25.0
deceleration = 60.0
max_yaw_rate = 2.0

[weapons_console]
beam_range = 50.0
beam_damage_per_sec = 6.0
beam_duration_secs = 4.0
cooldown_secs = 5.0
beam_color = [1.0, 0.1, 0.1, 1.0]

[behaviour]
initial_state = "patrol"
# … see §6.2 for behaviour states and transitions …
```

### 2.3 Stations

A station entity is identified by the `"station"` tag. It has no bespoke
section — hull HP lives in the standard `[hull]` block, and the visual /
radar / collider come from the standard `[mesh]`, `[radar_appearance]`,
and `[collider]` blocks. Stations are static (no `[helm_console]`, no
physics integration), but a `[collider]` is recommended so weapons and
ship collisions register hits — and now that `[hull].hull_integrity` is
read by the spawner, those hits actually deplete station HP.

| Field | Type | Default | Notes |
|---|---|---|---|
| `tags` | array of strings | — | Must contain `"station"`. |
| `[hull].hull_integrity` | f32 | **required** | Station HP. Consumed by the live runtime (collision + weapons damage). |
| `[mesh]` | table | optional | Visual mesh — typically `shape = "torus"` or `"sphere"`. |
| `[radar_appearance]` | table | optional | Radar dot colour / radius. |
| `[collider]` | table | optional but recommended | Hit-testing. |

#### Example — `assets/entities/station_outpost.toml`

```toml
tags = ["station", "destructible"]

[hull]
hull_integrity = 200.0

[collider]
shape = "Ball"
radius = 12.0
length = 0.0

[radar_appearance]
colour = [0.3, 0.8, 0.6]
radius = 12.0

[mesh]
shape = "torus"
radius = 12.0
minor_radius = 3.0
colour = [0.3, 0.8, 0.6]
```

### 2.4 Asteroid fields

An asteroid-field entity has an `[asteroid_field]` block. Spawning is
delegated to the streaming grid system (`src/asteroids/`).

All of a world's asteroid-field entities feed **one composed density
evaluator** (#913): a single streaming window evaluates each world cell
exactly once, blending every covering field's density and fill threshold by
`weight`. Overlapping fields therefore combine instead of each spawning its
own rocks (no double-spawn); one covering field — picked by the same weights
— supplies the spawned rock's tuning (type lists, jitter, rotation, shield
pierce). When several fields exist, the shared lattice uses the **finest**
authored `grid.resolution` and the **largest** authored `spawn_cells` /
`despawn_cells`; a single-field world keeps exactly its own authored values.
Mixed-resolution composition uses the finest authored resolution for the
whole lattice, so a coarse field composed with a fine one gets
`(res_coarse/res_fine)²` more cells per area everywhere it covers; keep
authored resolutions equal unless that is intended.

#### `[asteroid_field]`

| Field | Type | Default | Notes |
|---|---|---|---|
| `inner_radius` | f32 | **required** | Inner radius of donut. |
| `outer_radius` | f32 | **required** | Outer radius. Must be > `inner_radius`. |
| `density` | f32 | **required** | Asteroids per unit area (legacy field; grid takes precedence). |
| `weight` | f32 | `1.0` | Relative contribution to the composed density evaluator where fields overlap. `0.0` mutes a field wherever a positively-weighted field also covers the cell. |
| `shape` | string | none | `"torus"` selects explicit annulus eligibility (cells whose bounding box overlaps the ring). Omitted = legacy cell-centre distance test. |
| `spawn_distance` | f32 | `150.0` | Distance from ship at which asteroids start spawning. |
| `despawn_distance` | f32 | `250.0` | Must be ≥ `spawn_distance`. |
| `asteroid_type_paths` | array of type entries | `[]` | Template TOMLs picked for gameplay asteroids. See *rarity weights* below. |
| `cosmetic_type_paths` | array of type entries | `[]` | Template TOMLs picked for cosmetic-only asteroids. Same shape. |
| `tags` | array of strings | `[]` | Tags inherited by spawned asteroids. |
| `random_rotation` | `[f32, f32, f32]` | none | Max random rotation per axis in degrees: `[±pitch, ±roll, ±yaw]`. E.g. `[30, 30, 180]` gives mild tilt with full spin. Omit for no rotation. |
| `[asteroid_field.grid]` | table | none | If present, overrides donut spawning with deterministic grid streaming. |

#### Rarity weights in a type list

A type entry is either a bare path string or an inline table carrying a
relative rarity `weight` (default `1.0`). Both spellings may sit in the same
array, and a bare string means exactly `weight = 1.0` — so a list written
before rarity existed keeps its old meaning.

```toml
asteroid_type_paths = [
    "assets/entities/asteroid_common_1_small.toml",                             # weight 1.0
    { path = "assets/entities/asteroid_uncommon_1_small.toml", weight = 0.1 },  # ~1:10
    { path = "assets/entities/asteroid_rare_1_small.toml", weight = 0.01 },     # ~1:100
]
```

Weights are relative *within one list*, never probabilities: an entry at `0.1`
is drawn a tenth as often as an entry at `1.0` beside it, and adding entries
re-normalises the rest. Nothing in the engine knows what "common" or "rare"
mean — a new rarity tier is a new number here, not a code change. Negative
weights clamp to zero; a list whose weights are *all* zero falls back to a
uniform draw rather than erasing the field.

Re-weighting a list changes *which* rock a cell gets, never where it sits: the
pick is resolved from a single draw in the slot the unweighted pick used, so a
field's layout is stable across rarity retunes.

#### `[asteroid_field.grid]`

| Field | Type | Default | Notes |
|---|---|---|---|
| `resolution` | f32 | **required** | Cell size in world units. |
| `fill_gameplay` | f32 | `0.4` | Per-cell probability for gameplay asteroids. |
| `fill_cosmetic` | f32 | `0.15` | Per-cell probability per cosmetic layer. |
| `uniformity` | f32 | `0.0` | 1.0 = pure random, 0.0 = pure noise-driven. |
| `noise_freq` | f32 | `0.02` | Spatial noise frequency for jitter. |
| `noise_octaves` | u32 | `3` | |
| `density_noise_freq` | f32 | `0.01` | Perlin frequency for density field. |
| `density_noise_octaves` | u32 | `2` | |
| `jitter` | f32 | `0.0` | Max offset from cell centre (m). |
| `cosmetic_y_offset` | f32 | `0.0` | Y offset for cosmetic layers. |
| `spawn_cells` | u32 | `10` | Cells radius at which to start spawning around the player. |
| `despawn_cells` | u32 | `12` | Cells radius at which to despawn. Must be ≥ `spawn_cells`. |

#### Example — `assets/entities/asteroid_field_main.toml`

```toml
tags = ["field", "main", "asteroid_field"]

[asteroid_field]
inner_radius = 100.0
outer_radius = 200.0
density = 0.005
spawn_distance = 150.0
despawn_distance = 250.0
asteroid_type_paths = [
    "assets/entities/asteroid_small.toml",
    "assets/entities/asteroid_large.toml",
]
cosmetic_type_paths = ["assets/entities/asteroid_cosmetic.toml"]

[asteroid_field.grid]
resolution = 15.0
fill_gameplay = 0.4
fill_cosmetic = 0.15
uniformity = 0.3
jitter = 10.0
cosmetic_y_offset = 15.0
```

### 2.5 Individual asteroids

An individual-asteroid template has `tags`, `[collider]`, and a legacy
`[hull]`. They are picked from the field's `asteroid_type_paths`.

```toml
name = "Large Asteroid"
tags = ["asteroid", "gameplay", "large"]

[collider]
shape = "Ball"
radius = 10.0
length = 0.0

[hull]
hull_integrity = 100
```

### 2.6 Stars

Stars are ordinary entities composed of a `[mesh]` with a high `emissive` and
one or more `[[light]]` blocks. There is no dedicated `[star]` section.

#### Example

```toml
name = "Sun"
tags = ["star", "center"]

[mesh]
shape = "sphere"
colour = [1.0, 0.8, 0.0]
radius = 50.0
emissive = 2.0

[[light]]
kind = "point"
colour = [1.0, 0.95, 0.85]
intensity = 150000.0
range = 5000.0
```

### 2.7 Planets

Planets carry a dedicated `[planet]` section: a UV sphere rendered with a
custom shader that samples equirectangular texture maps (1024×512, from
`assets/planets/<type>/`). Lighting is computed relative to the star's actual
position (soft day/night terminator), with optional nightside-gated emission
(city lights, nightglow), an alpha-blended cloud/smog/ash shell on a slightly
larger sphere, and a fresnel atmosphere rim glow. `[planet]` takes precedence
over `[mesh]` on the viewscreen; keep a procedural `[mesh]` sphere as the
fallback for headless/editor contexts.

| Field | Type | Default | Notes |
|---|---|---|---|
| `radius` | f32 | `20.0` | Sphere radius. |
| `longitude_segments` | u32 | `64` | Mesh resolution. |
| `latitude_segments` | u32 | `32` | Mesh resolution. |

#### `[planet.surface]` (required)

| Field | Type | Default | Notes |
|---|---|---|---|
| `albedo` | string | **required** | Base colour map (sRGB). |
| `normal` | string | none | Tangent-space normal map. |
| `roughness` | string | none | Grayscale; enables a subtle specular glint (oceans, ice). |
| `emissive_colour` | string | none | City lights / nightglow / lava colour (sRGB). |
| `emissive_mask` | string | none | Grayscale mask; omit when the colour map is black where unlit. |
| `emissive_night_only` | bool | `true` | `false` for emission visible in daylight (lava). |
| `emissive_strength` | f32 | `1.0` | Emission multiplier. |

#### `[planet.clouds]` (optional)

| Field | Type | Default | Notes |
|---|---|---|---|
| `albedo` | string | **required** | Cloud colour map (sRGB). |
| `opacity` | string | none | Grayscale opacity; albedo luminance is used when omitted. |
| `scale` | f32 | `1.03` | Shell radius as a multiple of the planet radius. |
| `drift_speed` | f32 | `0.0` | Longitudinal UV wraps per second. `0` = static. |

#### `[planet.atmosphere]` (optional)

| Field | Type | Default | Notes |
|---|---|---|---|
| `colour` | `[f32; 3]` | **required** | Linear RGB rim-glow tint. |
| `strength` | f32 | `1.0` | Rim-glow intensity. |

#### Example

```toml
name = "Earth"
tags = ["planet", "habitable"]

[mesh]                 # procedural fallback (headless/editor)
shape = "sphere"
colour = [0.0, 0.5, 1.0]
radius = 20.0

[collider]
shape = "Ball"
radius = 20.0
length = 0.0

[planet]
radius = 20.0

[planet.surface]
albedo = "assets/planets/earth/albedo.webp"
normal = "assets/planets/earth/normal.webp"
roughness = "assets/planets/earth/roughness.webp"
emissive_colour = "assets/planets/earth/emissive_colour.webp"
emissive_night_only = true
emissive_strength = 1.5

[planet.clouds]
albedo = "assets/planets/earth/cloud_albedo.webp"
opacity = "assets/planets/earth/cloud_opacity.webp"
scale = 1.03

[planet.atmosphere]
colour = [0.35, 0.55, 1.0]
strength = 1.0
```

Available texture sets: `earth`, `moon`, `gas_giant`, `ice_moon`,
`lava_planet`, `ecumenopolis` — see the entity templates `planet_earth.toml`,
`moon_luna.toml`, `planet_gas_giant.toml`, `moon_ice.toml`,
`planet_lava.toml`, `planet_ecumenopolis.toml`.

### 2.8 Regions

A region entity has a `[shape]` block plus an optional `[effects]` block.
`[effects]` without `[shape]` is a parse error.

#### `[shape]`

The shape is a tagged enum. Pick `type = "sphere" | "box" | "torus"`:

```toml
[shape]
type = "sphere"
radius = 150.0
```

```toml
[shape]
type = "box"
half_extents = [50.0, 30.0, 40.0]
yaw = 0.0                    # optional, default 0.0
```

```toml
[shape]
type = "torus"
inner_radius = 50.0
outer_radius = 80.0
```

Boxes and toruses live in the XZ plane; the Y dimension of `box` is full
height. Region containment is computed by `src/regions/shape.rs`.

#### `[effects]`

Each effect is an *optional* sub-table. Presence enables the effect. All
six are independent; any combination is valid.

| Sub-table | Field | Type | Default | Notes |
|---|---|---|---|---|
| `[effects.damage_zone]` | `damage_per_second` (alias `dps`) | f32 | **required** | DPS applied to entities inside. |
| `[effects.slow_zone]` | `thrust_modifier` | f32? | none | Additive bonus on `MaxSpeed`. |
| | `yaw_rate_modifier` | f32? | none | Additive bonus on `MaxYawRate`. |
| `[effects.blocks_impulse]` | — | — | — | Empty marker table; blocks impulse charge. |
| `[effects.radar_dampening]` | `range_modifier` (alias `multiplier`) | f32 | **required** | Multiplier on `RadarRange`. |
| `[effects.comms_jammed]` (alias `comms_jam`) | — | — | — | Empty marker; raises `CommsJammed` flag. |
| `[effects.sensor_blind]` | — | — | — | Empty marker; raises `SensorBlind` flag. |

#### Example — `assets/entities/region_nebula.toml`

```toml
tags = ["region", "nebula"]

[shape]
type = "sphere"
radius = 150.0

[effects]
[effects.comms_jammed]
[effects.sensor_blind]
```

---

## 3. Factions

**Purpose:** name + UUID + enemy list. Used by AI hostility checks.

**Location:** `assets/factions/*.toml`.

**Parser:** `src/ai/faction.rs` → `parse_faction_config`.

| Field | Type | Default | Notes |
|---|---|---|---|
| `uuid` | UUID string | **required** | Stable identity. Referenced from entity TOMLs (`faction = "..."`). |
| `name` | string | **required** | Display name. |
| `enemies` | array of UUID strings | `[]` | UUIDs this faction considers hostile. *Asymmetric* — listing B does not imply B is hostile to A. |

Factionless entities (no `faction` field on the entity) are neither
enemies nor targets.

### Example — `assets/factions/federation.toml`

```toml
uuid = "aaaaaaaa-1111-4111-8111-aaaaaaaaaaaa"
name = "Federation"
enemies = ["bbbbbbbb-2222-4222-8222-bbbbbbbbbbbb"]
```

---

## 4. Complexity presets

**Purpose:** define UI complexity levels per console — what's hidden, what's
delegated to an AI partner, and how the AI should be tuned.

**Location:** `assets/complexity/*.toml`. Referenced from console blocks
in entity TOMLs via `complexity_toml = "..."`.

**Parser:** `src/console_ai/complexity.rs` → `parse_complexity_config`.

### Top-level

| Field | Type | Default | Notes |
|---|---|---|---|
| `[[preset]]` | array | **required** | One entry per preset. Convention: `"Low"`, `"Std"`. |

### `[[preset]]`

| Field | Type | Default | Notes |
|---|---|---|---|
| `name` | string | **required** | Preset identifier (matches `SetComplexity { preset_name }`). |
| `hidden_elements` | array of strings | `[]` | UI element IDs hidden at this preset. |
| `[preset.delegated]` | table | `{}` | `<Console> = { controls = [...] }` — controls delegated to another console under this preset. |
| `[preset.ai]` | table | `{}` | `<behavior_name> = { ... params ... }` — tuning for server-side `console_ai`. |

### Example — `assets/complexity/tactical.toml`

```toml
[[preset]]
name = "Low"
hidden_elements = ["phaser_mode_selector", "torpedo_tube_selector", "target_lock_button"]

[preset.delegated]
Tactical = { controls = ["auto_fire_torpedoes", "auto_frequency_match"] }

[preset.ai]
torpedo_auto_fire = { lead_prediction = true, min_accuracy = 0.7 }
frequency_match   = { sweep_interval_secs = 2.0, auto_match_delay_secs = 3.0 }

[[preset]]
name = "Std"
hidden_elements = []
```

A minimal single-preset file (e.g. `navigation.toml`) is valid and signals
"no complexity choice — always full":

```toml
[[preset]]
name = "Std"
hidden_elements = []
```

---

## 5. Cross-cutting sub-schemas

These appear inside multiple file types.

### 6.1 RadarConfig

Used by `[sensors_console.long_range_radar]` and
`[navigation_console.system_chart]`.

**Parser:** `src/radar_config.rs`.

| Field | Type | Default | Notes |
|---|---|---|---|
| `range` | f32 | `50.0` | Detection range in world units. |
| `shows` | array of strings | `[]` | Tag filter (OR). Known tags: `asteroid`, `asteroid_field`, `ship`, `star`, `planet`, `region`. Unknown tags are dropped silently. |

```toml
[sensors_console.long_range_radar]
range = 200.0
shows = ["region", "asteroid_field", "asteroid", "ship"]
```

### 6.2 `[behaviour]` — AI controller

Used by NPC entity templates.

**Parser:** `src/entities/config.rs::BehaviourConfig` +
`src/ai/core.rs::TransitionConfig`.

#### `[behaviour]`

| Field | Type | Default | Notes |
|---|---|---|---|
| `initial_state` | string | **required** | Name of the starting state. |
| `[[behaviour.state]]` | array | `[]` | Per-state parameter blocks. |
| `[[behaviour.transition]]` | array | `[]` | Transition rules (evaluated in declaration order). |

#### `[[behaviour.state]]`

| Field | Type | Default | Notes |
|---|---|---|---|
| `name` | string | **required** | Stable identifier (referenced by `initial_state`, `from`, `to`). |
| `kind` | string | `""` | One of `idle`, `patrolling`, `pursuing`, `attacking`, `fleeing`, `warping_out`. |
| `waypoints` | array of strings | `[]` | Anchor names (for `patrolling`). |
| `loop_path` | bool | `false` | Loop waypoints (for `patrolling`). |
| `target_speed` | f32 | `0.0` | Desired forward speed [0,1]; clamped at load. |
| `maintain_range` | f32 | `0.0` | Stand-off distance (for `attacking`). |
| `duration_secs` | f32 | `0.0` | Lifetime before self-despawn (for `warping_out`). |

#### `[[behaviour.transition]]`

| Field | Type | Default | Notes |
|---|---|---|---|
| `from` | string \| array of strings | **required** | State name(s) this transition may fire from. |
| `to` | string | **required** | State to enter. |
| `condition` | string | **required** | `enemy_in_range`, `on_attacked`, `target_destroyed`, `in_weapons_range`, `hull_below`, `on_timer`, `on_scenario_unloaded`. |
| `radius` | f32 | none | For `enemy_in_range`. |
| `threshold` | f32 | none | For `hull_below` (0..1 fraction). |
| `seconds` | f32 | none | For `on_timer` (since state entry). |

#### Example (FSM-era excerpt, from the retired `pirate_raider.toml`)

> The `[behaviour]` FSM this shows was dissolved in #572 and the hull retired in
> #892; the section is kept only to document the legacy schema.

```toml
[behaviour]
initial_state = "patrol"

[[behaviour.state]]
name = "patrol"
kind = "patrolling"
waypoints = ["patrol_alpha", "patrol_beta", "patrol_gamma"]
loop_path = true
target_speed = 0.5

[[behaviour.state]]
name = "attack"
kind = "attacking"
maintain_range = 45.0
target_speed = 0.8

[[behaviour.transition]]
from = ["patrol", "idle"]
condition = "enemy_in_range"
radius = 200.0
to = "pursue"

[[behaviour.transition]]
from = ["attack", "pursue"]
condition = "hull_below"
threshold = 0.3
to = "flee"
```

### 6.3 `[stations]` — bridge crew layouts

Lives inside the player-ship entity TOML. Defines, for each supported
player count (1..N), which *stations* exist and which consoles each owns.

**Parser:** `src/lobby/stations_config.rs::ShipStations`.

#### `[stations]`

| Field | Type | Default | Notes |
|---|---|---|---|
| `min_players` | u32 | **required** | Lowest supported count. |
| `max_players` | u32 | **required** | Highest. |
| `[[stations.<N>]]` | array | **required** | One array per N in `[min, max]`. Each entry is a `StationDef`. |

#### `StationDef` (`[[stations.<N>]]` entry)

| Field | Type | Default | Notes |
|---|---|---|---|
| `name` | string | **required** | Display name (e.g. `"Helm"`). Must be unique within its count bucket. |
| `description` | string | **required** | Lobby description. |
| `consoles` | array of Console names | **required** | Consoles owned by this station. Names match the `Console` enum: `CaptainChair`, `Helm`, `Tactical`, `Repair`, `Sensors`, `Shields`, `Navigation`, `Power`, `Comms`. |
| `rank` | string | **required** | UI rank prefix (e.g. `"Cpt."`, `"Ltn."`, `"Ens."`). |
| `short_code` | string | `""` | Short label used in compact UI. |
| `next` | string | none | Station name (at count + 1) this station "promotes to" when a player joins. Must exist at N+1 if set. |
| `previous` | string | none | Station name (at count − 1) this station "demotes to" when a player leaves. Must exist at N−1 if set. |

The validator rejects dangling `next`/`previous` references and duplicate
names within a count bucket.

#### Example (excerpt — 3-player crew)

```toml
[stations]
min_players = 1
max_players = 6

[[stations.3]]
name = "Helm"
description = "Pilot the ship, command the bridge."
consoles = ["CaptainChair", "Helm"]
rank = "Cpt."
previous = "Helm"
next = "Helm"

[[stations.3]]
name = "Tactical"
description = "Manage weapons, sensors, shields, and navigation."
consoles = ["Tactical", "Sensors", "Navigation"]
rank = "Ltn."
previous = "Tactical"
next = "Tactical"

[[stations.3]]
name = "Engineering"
description = "Maintain and repair ship systems, manage power, shields, and manage comms."
consoles = ["Repair", "Power", "Shields", "Comms"]
rank = "Ltn."
next = "Engineering"
```

---

## Maintenance

When you change any of the parser modules referenced above, update the
matching section in this document. The authoritative sources are:

| Section | Source |
|---|---|
| Worlds (anchors, `[[entity]]`, `[ambient_light]`, scene shape) | `src/world/config.rs` |
| Entity templates (`[mesh]`, `[hull]`, `[[light]]`, etc.) | `src/entities/config.rs` |
| Region shape / effects | `src/regions/shape.rs`, `src/regions/effects.rs` |
| RadarConfig | `src/radar_config.rs` |
| Triggers / comms templates / objectives | `src/world/content.rs` |
| Factions | `src/ai/faction.rs` |
| Complexity presets | `src/console_ai/complexity.rs` |
| Behaviour states / transitions | `src/ai/core.rs` |
| Stations block | `src/lobby/stations_config.rs` |
