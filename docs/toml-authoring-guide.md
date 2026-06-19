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

**Purpose:** the single content file for a session — anchors, the static layout
of entity instances, named trigger-eligible spawns, world-event triggers, comms
dialogues, and objectives. One world per session; chaining (`load_scenario`)
is not supported.

**Location:** `assets/worlds/*.toml` (e.g. `assets/worlds/default.toml`,
`assets/worlds/patrol.toml`).

**Parser:** `src/world/config.rs` owns the entire world schema. `parse_world`
is a single-pass deserializer that produces a `WorldConfig` carrying the
normalised anchor table, the `[[entity]]` list, `[[trigger]]` blocks, and
`[[comms]]` templates. The JS-facing loader is `wasm_load_world` in
`src/server/bridge.rs`, which delegates to `entities::config_cache` to
populate the `WORLD_CONFIG` thread-local. The Bevy startup chain
(`insert_world_config_resource → spawn_world_entities → init_world_runtime
→ setup_fallback_world`) consumes it. Asteroid-field and named `[[entity]]`
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

### `[global]`

| Field | Type | Default | Notes |
|---|---|---|---|
| `seed` | u64 | `42` | Seeds deterministic generators (asteroid spawner, etc.). |

### `[anchors]`

A flat table of `name = [x, y, z]`. Used by `[[entity]] anchor = "..."`
references
and by AI `waypoints = [...]` lists.

```toml
[anchors]
starbase_alpha = [500.0, 0.0, 0.0]
patrol_alpha   = [300.0, 0.0, -300.0]
```

### `[[entity]]`

A single entity instance. Anonymous entries are static world layout;
entries carrying a `name` field become trigger- and comms-eligible (the
unified pipeline assigns them a stable UUID and registers `name → uuid`
for trigger/comms/`relative_to` lookups).

| Field | Type | Default | Notes |
|---|---|---|---|
| `template_path` | string | **required** | Path to entity template (e.g. `"assets/entities/star_sun.toml"`). |
| `id` | string | none | Stable instance ID for cross-reference. |
| `name` | string | none | When set, the unified pipeline assigns a UUID and registers `name → uuid` in `WorldConfig.name_to_uuid` and `WorldContentRuntime.name_to_uuid`. Triggers, comms, and `relative_to` lookups resolve names through this map. |
| `position` | `[f32; 3]` | `[0,0,0]` | World position. |
| `anchor` | string | none | Named entry from `[anchors]`; resolved to `[x,y,z]` at spawn time. |
| `relative_to` | string | none | Another named `[[entity]]` to position relative to. Used with `offset`. The referenced entity must use `anchor` or `position` (not another `relative_to`). |
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
template_path = "assets/entities/pirate_raider.toml"
name          = "raider_alpha"
anchor        = "patrol_alpha"

# Entity positioned relative to another named entity
[[entity]]
template_path = "assets/entities/pirate_raider.toml"
relative_to   = "starbase_alpha"
offset        = [10.0, 0.0, -5.0]
```

### 1.5 `[[trigger]]`

| Field | Type | Default | Notes |
|---|---|---|---|
| `condition` | string | **required** | One of `on_destroyed`, `on_attacked`, `on_timer`, `on_hailed`. |
| `entity` | string | depends | Required for entity-based conditions; references a named `[[entity]]` `name`. |
| `after_secs` | f32 | depends | Required for `on_timer`. |
| `[[trigger.action]]` | array | `[]` | Actions to fire (in order). |

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

**Removed actions** (no longer supported): `load_scenario`, `unload_scenario` —
each session loads exactly one world TOML and runs it to completion.

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

### Example — `assets/worlds/default.toml`

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
template_path = "assets/entities/player_ship.toml"
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
template_path = "assets/entities/pirate_raider.toml"
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
| `beam_range` | f32 | `0.0` | Phaser beam range. |
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

| Field | Type | Default | Notes |
|---|---|---|---|
| `power_multipliers` | `[f32; 4]` | none | |
| `[sensors_console.long_range_radar]` | RadarConfig | default | See **§6.1**. |
| `complexity_toml` | string | none | |

#### `[shields_console]`

Tunes the four-quadrant shield *focus* mechanic.

| Field | Type | Default | Notes |
|---|---|---|---|
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
| `emergency_threshold` | f32 | **required** | Battery level below which all consoles lock to level 1. |

### 2.2.1 Example — `assets/entities/player_ship.toml`

```toml
name = "Player Ship"
tags = ["player", "ship"]
faction = "aaaaaaaa-1111-4111-8111-aaaaaaaaaaaa"

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

### 2.2.2 Example — `assets/entities/pirate_raider.toml`

NPC ship: legacy single-slot hull, AI behaviour, no `[stations]`.

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

#### `[asteroid_field]`

| Field | Type | Default | Notes |
|---|---|---|---|
| `inner_radius` | f32 | **required** | Inner radius of donut. |
| `outer_radius` | f32 | **required** | Outer radius. Must be > `inner_radius`. |
| `density` | f32 | **required** | Asteroids per unit area (legacy field; grid takes precedence). |
| `spawn_distance` | f32 | `150.0` | Distance from ship at which asteroids start spawning. |
| `despawn_distance` | f32 | `250.0` | Must be ≥ `spawn_distance`. |
| `asteroid_type_paths` | array of strings | `[]` | Template TOMLs picked for gameplay asteroids. |
| `cosmetic_type_paths` | array of strings | `[]` | Template TOMLs picked for cosmetic-only asteroids. |
| `tags` | array of strings | `[]` | Tags inherited by spawned asteroids. |
| `[asteroid_field.grid]` | table | none | If present, overrides donut spawning with deterministic grid streaming. |

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

Planets are ordinary entities composed of a `[mesh]` (typically a sphere) and,
optionally, a `[collider]`. There is no dedicated `[planet]` section.

#### Example

```toml
name = "Earth"
tags = ["planet"]

[mesh]
shape = "sphere"
colour = [0.0, 0.5, 1.0]
radius = 20.0

[collider]
shape = "Ball"
radius = 20.0
length = 0.0
```

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

#### Example (excerpt from `pirate_raider.toml`)

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
