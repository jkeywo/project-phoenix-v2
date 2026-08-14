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
normalised anchor table and the `[[entity]]` list. Scenario logic is not part
of that struct: the `[script]` block is lifted and compiled separately
(`src/world/script/`). The JS-facing loader is `wasm_load_world` in
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
| `[[entity]]` | array of tables | `[]` | Entity instances spawned into the world. Single block type for all spawnables; named entries (with `name = "..."`) are the ones a script can reference. |
| `[script]` | table | none | Scenario logic — event registrations, handler fns and comms dialogue nodes (see §1.5, §1.7). |
| `[[deadline]]` | array of tables | `[]` | Named mission deadlines the crew can be shown and script can slip or cancel (see §1.6). |
| `[[route]]` | array of tables | `[]` | Named civilian traffic lanes: anchor chains a `[civilian]` craft flies (see §1.8). |
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

### 1.5 Scenario logic — the `[script]` block

A world's scenario logic — when something happens, and what happens — is
authored in Rhai, in the world's own `[script]` block (or a sibling `.rhai`
file). There is one front-end. The `[[trigger]]` and `[[comms]]` TOML arrays
were the other one; issue #985 deleted them, and a world that still authors
either is **refused at load** with a message naming the block, rather than
loading with its logic silently absent.

```toml
[script]
setup = """
# Registrations: one per world event you want to react to. Each names a handler
# fn defined in the same unit.
on_destroyed("raider_alpha", "on_raider_destroyed");
on_timer(45, "second_wave");
on_hailed("Starbase Alpha", "on_starbase_hailed");

fn on_raider_destroyed(ctx) {
    ctx.effects.add_objective(#{ id: "raider_killed", text: "Pirate raider destroyed." });
}
"""
```

#### Conditions

The registration fns mirror the `TriggerCondition` vocabulary one for one:

| Registration | Arguments | Notes |
|---|---|---|
| `on_destroyed(entity, handler)` | named `[[entity]]` reference id | Fires when that entity dies. |
| `on_all_destroyed(group, handler)` | entity group name | Fires once every member is gone. |
| `on_attacked(entity, handler)` | | |
| `on_hull_below(entity, threshold, handler)` | `threshold` in `(0, 1]` | Fires on a strictly DOWNWARD crossing. |
| `on_timer(after_secs, handler)` | world-relative seconds | |
| `on_hailed(entity, handler)` | | The Comms officer's hail. |
| `on_flag_set(name, handler)` / `on_flag_cleared(name, handler)` | flag name | |
| `on_world_loaded(handler)` | | |
| `on_entered_region(entity, handler)` / `on_exited_region(entity, handler)` | region entity | |
| `on_waypoint_reached(entity, handler)` | | Fires on arrival at any waypoint of that ship's route; "reached" means within that entity's `[behaviour] waypoint_arrival_radius`. |

A registration is single-shot: each fires at most once per session.

#### Effects

A handler fn takes `ctx` and calls `ctx.effects.*` / `ctx.flags.*` / `ctx.schedule.*`:

| Call | Notes |
|---|---|
| `ctx.effects.add_objective(#{ id, text, mandatory?, targets?, source?, base_priority?, directive_kind?, … })` | Add to the objectives list, with its AI directive and utility scoring. |
| `ctx.effects.complete_objective(id)` / `fail_objective(id)` | |
| `ctx.effects.spawn_entity(#{ template_path, name, position?/anchor?, rotation?, scale?, overrides? })` | `template_path` string literals are scanned statically for asset preload. |
| `ctx.effects.destroy_entity(name)` | The counterpart to `spawn_entity`: removes the named entity. It CHAINS — `on_destroyed` and `on_all_destroyed` fire off it in the same tick, exactly as they do off a combat kill, which is what lets a scripted collapse drive mission state. An unknown name warns and does nothing. Reusing a destroyed entity's name in a later `spawn_entity` is fine. Also available deferred: `ctx.schedule.in_seconds(n).destroy_entity(name)`. |
| `ctx.effects.load_world(path)` / `unload_world(path)` | Runtime composition of sub-world layers. |
| `ctx.effects.apply_modifier(…)` / `remove_modifier(…)` | `slot` ∈ {`MaxSpeed`, `MaxYawRate`, `RadarRange`, `PhaserDamage`, `HullDamageTaken`, `RepairRate`}. |
| `ctx.effects.apply_int_modifier(…)` / `remove_int_modifier(…)` | `slot` ∈ {`RepairTeams`}. |
| `ctx.effects.apply_flag(…)` / `remove_flag(…)` | `kind` ∈ {`CommsJammed`, `SensorBlind`}. |
| `ctx.effects.add_faction_enemy(…)` / `remove_faction_enemy(…)` | |
| `ctx.effects.game_over(message, "victory" \| "defeat")` | `message` may be empty. |
| `ctx.flags.name = n` / `ctx.flags.increment(name, by)` | World flags. Never `+=` on a `flags` member — the loader rejects it. |
| `ctx.schedule.in_seconds(n, effect)` | Defers ONE effect by `n` world-elapsed seconds (the old per-action `delay_secs`). |
| `ctx.schedule.after(n, \|ctx\| { … })` | Defers a whole closure. |
| `ctx.deadlines.remaining(id)` / `state(id)` | Read a named mission deadline — see §1.6. |
| `ctx.deadlines.slip(id, secs)` / `cancel(id)` | Move a deadline out (or in), or call it off. |
| `ctx.commitments.record(#{…})` / `keep(id)` / `break_promise(id)` / `state(id)` | Promises the captain makes — see §1.9. |
| `ctx.effects.order_hold(entity)` | Order a civilian to stop where it is — see §1.8. Refusable. |
| `ctx.effects.order_divert_route(entity, route)` / `order_divert_anchor(entity, anchor)` | Send it down another lane, or to a single anchor. Refusable. |
| `ctx.effects.order_dock(entity, structure)` | Send it to berth at a named structure. Refusable. |

Conditional effects are ordinary Rhai control flow — an `if` on a flag read —
rather than a per-action `when` predicate. A trigger-level gate is still a
`when` on the registration where one is offered.

**Removed actions** (no longer supported): `load_scenario`, `unload_scenario`.
Use `load_world` and `unload_world` for runtime composition.

### 1.6 Named mission deadlines

A deadline is a *named thing in the world* — `transfer_window_opens`,
`stabiliser_failure` — rather than an anonymous timer. Declare the data in
`[[deadline]]` blocks and name the fn each one runs from `[script]`:

```toml
[[deadline]]
id = "transfer_window_opens"      # unique in the world; script names it by this
label = "world.fs.deadline.transfer_window.label"   # a strings.csv id, never English
due_secs = 600                    # whole seconds from the mission's FIRST tick
visible = true                    # default false — the crew never sees it otherwise

[script]
setup = """
on_deadline("transfer_window_opens", "on_transfer_window");

fn on_transfer_window(ctx) { … }

fn on_strike_settled(ctx) {
    if ctx.deadlines.remaining("transfer_window_opens") < 60 {
        ctx.deadlines.slip("transfer_window_opens", 120);   // buy two minutes
    }
    if ctx.deadlines.state("stabiliser_failure") == "pending" {
        ctx.deadlines.cancel("stabiliser_failure");         // call it off entirely
    }
}
"""
```

* `due_secs` is measured from the first simulation tick of the **mission**, not
  from app start, so a long lobby costs a mission none of its deadlines.
* **Every `[[deadline]]` needs exactly one `on_deadline`, and vice versa.** Both
  mismatches are load-time errors that block the world, because a deadline with
  no handler cannot be armed at all — its failure would otherwise be a countdown
  reaching zero and nothing happening. A deadline you want purely as a countdown
  still gets a handler; write an empty one.
* A duplicate `id` is a load-time error naming both entries.
* `remaining(id)` is whole seconds, rounded up: `0` once it has fired, `-1` once
  cancelled or for an id no block declares. `state(id)` is `"pending"` /
  `"fired"` / `"cancelled"` / `"unknown"` and is the unambiguous read.
* `slip` is measured from the deadline's own due time, so slips accumulate; a
  negative `secs` pulls a deadline IN, never past the present tick. A slipped
  deadline does **not** also fire at its old time, and a cancelled one never
  fires. Slipping or cancelling one that has already fired or been cancelled is
  a no-op.
* Reads inside one handler see that handler's own writes.
* `visible = true` deadlines appear as a countdown on the destroyer captain
  console, counted down server-side. Everything else stays the mission's
  business.

### 1.7 Comms threads

A comms thread is opened by a handler and authored as one fn per dialogue node:

```toml
[script]
setup = """
on_hailed("Starbase Alpha", "on_starbase_hailed");

fn on_starbase_hailed(ctx) {
    ctx.effects.open_comms(#{ from: "Starbase Alpha", node_fn: "starbase_hail" });
}

# A node fn returns #{ message, responses }. An announcement — the one-way
# broadcast an announcement template used to be — is a node whose
# `responses` array is empty.
fn starbase_hail(ctx) {
    #{ message: "USS Phoenix, this is Starbase Alpha. Please state your business.",
       responses: [
         #{ text: "We require docking clearance.", on_pick: "on_dock" },
         #{ text: "No comment.", on_pick: "on_decline", important: true },
       ] }
}

# A response's `on_pick` names the fn that runs when the player picks it. That fn
# buffers the response's effects and RETURNS the follow-up node — or `()` for a
# terminal response that ends the thread.
fn on_dock(ctx) {
    ctx.effects.add_objective(#{ id: "obj-dock", text: "Dock at Starbase Alpha.", mandatory: true });
}
fn on_decline(ctx) { }
"""
```

`open_comms` keys, all optional except `from` and `node_fn`:

| Key | Notes |
|---|---|
| `from` | Sender reference id — a named `[[entity]]`, or a synthetic name such as `"_self"` for a ship-internal report. Resolved to a UUID for range and contact lookup. |
| `node_fn` | The fn returning this thread's root node. |
| `display_name` | Player-facing sender label. Falls back to `from`. |
| `thread_id` | Joins an existing thread. A fresh id is minted when absent. |
| `urgent` | Flags the message urgent; a follow-up inherits the thread's urgency. |

A response's `important` flag makes the client confirm before submitting it.

A DELAYED reply is `ctx.schedule.after(n, |ctx| ctx.effects.open_comms(#{ thread_id: "…", node_fn: "next" }))` — the placeholder-and-queue machinery the
declarative `follow_up.trigger` needed is gone with it.

#### Hailable contacts

A contact reaches the Comms officer's hail roster by opting in on the ENTITY,
not in the world:

```toml
# assets/entities/station_axiom.toml
[comms]
range        = 800
hailable     = true
display_name = "Starbase Alpha"   # optional; falls back to the entity's name
```

Per-world opt-in is an `overrides` on the `[[entity]]` block:
`overrides = { comms = { hailable = true } }`.

### 1.8 Civilian traffic: routes and orders

A **route** is a named lane in the world — an anchor chain with per-leg
behaviour and an authored ending. A **civilian** is a hull that flies one and
can be told otherwise. Both halves are authored data; neither is a mover.

```toml
[[route]]
id = "depot_run"                  # unique in the world; a craft or an order names it by this
on_complete = "loop"              # "loop" (default) or "terminate"

[[route.leg]]
anchor = "depot_north"            # a name in this world's [anchors] table
speed = 0.4                       # cruise fraction (0, 1] for THIS leg; default 0.5
hold_secs = 20                    # dwell here before pressing on; default 0

[[route.leg]]
anchor = "depot_south"
```

```toml
# On the entity template (or a world `[[entity]] overrides`):
[civilian]
route = "depot_run"               # omit for a craft with no standing lane
route_priority = 60.0             # utility priority of its travel objective

[civilian.compliance]             # optional; omit for a cooperative craft
ack_secs = 2                      # whole seconds before it answers at all
decide_secs = 3                   # …and before it acts on what it answered
hold = "comply"                   # per verb: "comply" (default) or "refuse"
divert = "refuse"
dock = "comply"
refusal_reason = "world.fs.hauler.refusal"   # a strings.csv id, never English
```

* An order is a **negotiation, not a remote control**. Every order — from a
  console or from a script — walks `received → acknowledged → complying`, or
  `received → refused`, on the craft's own authored clock. A craft that agrees
  and then finds it cannot comply (the berth is gone, the lane resolves
  nowhere) lands in `non_compliant`, which is a *different* state from
  `refused`: one declined and carried on down its own lane, the other agreed
  and got stuck.
* Compliance resolves **entity → faction → cooperative default**, so a scenario
  can make one hull difficult or a whole shipping line difficult. A faction
  authors the same `[compliance]` table in `assets/factions/*.toml`.
* A leg naming an anchor no world in the composition declares, and a `[civilian]
  route` naming a lane nobody declares, are **load-time errors** that block the
  world — neither table is ever written again, so a reference that misses at
  load misses forever.
* A duplicate route `id`, a lane with no legs, and a `speed` outside `(0, 1]`
  are load-time errors naming the route.
* Route following goes through the ordinary NPC helm: the craft is handed a
  `Patrol` directive over the lane's anchors and the existing patrol cursor is
  its leg pointer. A `hold` takes its directive away entirely, which is how any
  objective-less NPC comes to a stop.
* Navigation sees the whole picture — lane, leg and compliance — on its console.


### 1.9 Commitments: promises the captain makes

A **commitment** is a promise on the books — who it was made to, what its terms
are, what would count as keeping it, and whether it ends up kept or broken.
There is **no `[[commitment]]` block**: a promise is a runtime artifact, made in
the beat where the captain gives their word, and whether one exists at all
depends on what the player chose to say.

```toml
[script]
setup = """
# The negotiation beat. In a shipped mission this body is a dialogue `on_pick`.
fn on_promise_passage(ctx) {
    ctx.commitments.record(#{
        id:            "safe_passage",       # unique for the run; duplicates RAISE
        made_to:       "skyway_strike_committee",
        terms:         "world.fs.commitment.safe_passage.terms",    # strings.csv ids,
        resolves_when: "world.fs.commitment.safe_passage.resolves", # never English
    });
}

# THE PAYOFF: an option that exists only because the captain gave their word.
# Gating is ordinary control flow — there is no `when:` field on a response.
fn committee_calls_back(ctx) {
    let responses = [ #{ text: "world.fs.comms.stall", on_pick: "on_stall" } ];
    if ctx.commitments.state("safe_passage") == "open" {
        responses.push(#{ text: "world.fs.comms.honour", on_pick: "on_honour" });
    }
    #{ message: "world.fs.comms.committee", responses: responses }
}

fn on_honour(ctx) { ctx.commitments.keep("safe_passage"); }
"""
```

* `state(id)` is `"open"` / `"kept"` / `"broken"` / `"unknown"`. All four are
  load-bearing. **"Broken" is not "open"** — an unfinished errand is not a
  betrayal — and **"unknown" is not "broken"**: a promise nobody ever made is a
  different fact again, and it is the guard you use before recording one.
* **A duplicate `id` raises**, which drops the whole call's effects. If a beat
  can be reached twice, guard it:
  `if ctx.commitments.state("safe_passage") == "unknown" { … }`.
* `resolves_when` is a *statement*, not a predicate. Nothing evaluates it and
  nothing scans for promises that have come good — it is there so the bargain is
  data. **You** settle the promise, at the beat where the fiction tests it.
* Settling writes an ordinary world flag — `commitment.<id>.kept` or
  `commitment.<id>.broken` — so `on_flag_set("commitment.safe_passage.kept", "h")`
  is how a promise reaches past the scene it was made in. Two flags, not one,
  because a handler needs to know which way it went.
* Settling a promise twice is a no-op: the first resolution stands.
* `break_promise`, not `break` — the latter is a Rhai keyword.
* **A promise carries no clock.** For a promise-by-time, author a `[[deadline]]`
  (§1.6) and let its handler settle it:

  ```
  on_deadline("transfer_window_closes", "on_window_closed");

  fn on_window_closed(ctx) {
      if ctx.commitments.state("safe_passage") == "open" {
          ctx.commitments.break_promise("safe_passage");
      }
  }
  ```

* Reads inside one handler see that handler's own writes.
* `made_to` is stored exactly as you write it and is never resolved to an
  entity. A promise is made to a *party* — a faction, a committee, a person —
  and it outlives the hull you were talking to. If the party you name *is* an
  entity in this world, name it by its `[[entity]] id` and the promise appears
  on that subject's dossier (§1.10).


### 1.10 Dossiers: what the crew know about a subject

A **dossier** is the intelligence file on one subject, rendered as a list plus a
fact sheet on the destroyer's tactical console (the Intel overlay). There is no
`[dossier]` block and nothing to declare: the whole thing is projected, every
tick, from state you author elsewhere.

**Who gets a dossier.** A subject is any entity the crew *already* have an
authoritative surface on — one of exactly two doors:

* it is on the hail roster (`[comms] hailable = true`), or
* it publishes an infrastructure condition track (`[infrastructure] publish =
  true`, which is the default).

That is the whole rule, and it is deliberate: there is no way to declare that
the crew hold a file on something they have no other means of observing. A
subject with nothing known about it still gets a sheet — an *empty* file is a
different thing from a missing one, and the panel says so in as many words.

**What lands on the sheet.**

| Row | Comes from |
| --- | --- |
| the name, and the line under it | the entity's `name` and its `[target] description` |
| affiliation | the subject's faction, via that faction's `display_name` |
| in hailing range | the live comms roster |
| condition | `[infrastructure]`, **only** when `publish = true` |
| an operational flag | `[[infrastructure.threshold]]`, only when it has a `label` |
| a capacity | `[[infrastructure.capacity]]`, only when it has a `label` |
| a promise | a commitment whose `made_to` is this entity's `[[entity]] id` |

**Two things are kept off it by construction.**

1. `[infrastructure] publish = false` keeps a structure's condition off the wire
   and therefore off its dossier — the projection never holds the number at all.
   Use it for the record a mission is keeping back. The structure itself still
   appears if it is hailable; only the condition is absent.
2. A flag or capacity `id` is a machine name in *your* namespace and is never
   shown as prose. Author a `label` — a `strings.csv` id — beside it when the
   crew should be able to read it:

```toml
[[infrastructure.capacity]]
id     = "depot_berths"
amount = 4
label  = "world.fs.capacity.berths.label"   # shown; without this, script-only

[[infrastructure.threshold]]
flag        = "depot_transfer_capable"
fails_below = 0.4
label       = "world.fs.threshold.transfer.label"
```

A faction's crew-facing name works the same way: `assets/factions/*.toml` takes
an optional `display_name` beside its `name`. `name` stays the reference key
that `add_faction_enemy` and entity templates use; `display_name` is a
`strings.csv` id and the only string a player ever sees. A faction with no
`display_name` simply has no affiliation row.

See `assets/worlds/probe_dossier.toml` for all of the above in one world,
including the structure whose condition is withheld.

### Example — a world, end to end

```toml
title = "Default Patrol"
description = "Pirate raider patrol around Starbase Alpha."

[global]
seed = 42

[anchors]
starbase_alpha = [500.0, 0.0, 0.0]
patrol_alpha   = [300.0, 0.0, -300.0]

# Static map-half layout (anonymous — not script-referenceable)
[[entity]]
template_path = "assets/entities/star_sun.toml"
position = [0.0, 0.0, 0.0]

[[entity]]
template_path = "assets/entities/alliance_cruiser.toml"
id = "player-ship"
position = [150.0, 0.0, 0.0]
spawn_on = "game_start"

# Named [[entity]] — a script can reference this by name
[[entity]]
template_path = "assets/entities/station_outpost.toml"
name          = "Starbase Alpha"
position      = [500.0, 0.0, 0.0]

# Named NPC at an anchor
[[entity]]
template_path = "assets/entities/ship_harrow_patrol.toml"
name          = "raider_alpha"
anchor        = "patrol_alpha"

[script]
setup = """
on_destroyed("raider_alpha", "on_raider_destroyed");
on_hailed("Starbase Alpha", "on_starbase_hailed");

fn on_raider_destroyed(ctx) {
    ctx.effects.add_objective(#{ id: "raider_killed", text: "Pirate raider destroyed." });
}

fn on_starbase_hailed(ctx) {
    ctx.effects.open_comms(#{ from: "Starbase Alpha", node_fn: "starbase_hail" });
}

fn starbase_hail(ctx) {
    #{ message: "USS Phoenix, this is Starbase Alpha. Please state your business.",
       responses: [ #{ text: "We require docking clearance.", on_pick: "on_dock" } ] }
}

fn on_dock(ctx) {
    ctx.effects.add_objective(#{ id: "obj-dock", text: "Dock at Starbase Alpha.", mandatory: true });
}
"""
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

### 2.1.6 `[operations]` — external operations (issues #1026, #1027)

Which **external operations** this hull can perform: the verbs it applies to
things outside its own hull. The mirror image of `[infrastructure]` — that table
says what can be done *to* an entity, this one says what an entity can do.

Omitting the table changes nothing. A hull that authors none can start no
operation and is refused by name if asked to.

There are five verbs, and they share one implementation. What separates them is
**what you author**, not what the engine does: a tow and a field-repair run
through the same eligibility test and the same timed hold, and differ only in
the fields below that each one fills in.

| Verb | What it does | The fields that make it that verb |
|---|---|---|
| `stabilise` | Hold station on a failing structure and arrest its decline. | `condition_on_complete` |
| `tow` | The target's position becomes the operator's rig for the duration. | `tow_offset` |
| `escort` | Keep station on something that is *moving*. | `separation_limit` |
| `transfer` | Move a named capacity between the operator and the target. | `[…capability.transfer]` |
| `field_repair` | Work a structure's condition continuously, at a cost in repair teams. | `condition_per_second`, `repair_teams` |

```toml
[[operations.capability]]
verb                  = "field_repair"
range                 = 400.0         # world units, centre to centre
duration_secs         = 20            # whole seconds of ELIGIBLE hold
power_group           = "helm"        # which group the operation draws on
min_power_level       = 2             # …and the level it needs held
condition_per_second  = 2.0           # points paid for every second held
repair_teams          = 2             # teams unavailable to the ship meanwhile
stall_limit_secs      = 45            # optional cumulative stall budget

# What interrupts it, and what that does. Authored per capability, because how
# bad a storm is for a particular job is a judgement about the job.
[[operations.capability.interrupt]]
cause         = "attack"
response      = "fail"

[[operations.capability.interrupt]]
cause         = "region"
region_effect = "slow_zone"
response      = "slow"
rate_percent  = 50
```

| Field | Type | Default | Notes |
|---|---|---|---|
| `verb` | `"stabilise"` \| `"tow"` \| `"escort"` \| `"transfer"` \| `"field_repair"` | **required** | The operation this block authorises. |
| `range` | f32 | `400.0` | How far from the target the ship may be and still count the tick. |
| `duration_secs` | i64 | `20` | Whole seconds of *eligible* hold. Stalled ticks do not count towards it — that is the point of the hold. |
| `power_group` | string | `"helm"` | The power group the operation draws on. |
| `min_power_level` | u8 | `2` | The level that group must hold. `2` is where every group is seeded, so a ship that has stripped helm loses the operation. |
| `condition_on_complete` | f32 | `0.0` | Infrastructure condition points the target gains **on completion**, paid once. |
| `condition_per_second` | f32 | `0.0` | Condition points paid for every **second held**, scaled by the tick's rate. `field_repair`'s shape. |
| `repair_teams` | u8 | `0` | How many of the operator's own repair teams are unavailable for internal work while the hold runs. They never leave the hull. |
| `tow_offset` | `[f32; 3]` | `[0,0,0]` | Where a towed target rides, in the operator's **own frame**: `[starboard, up, forward]`, so `[0, 0, -150]` is 150 units astern. |
| `separation_limit` | f32 | *(none)* | Distance past which the hold **fails** rather than stalling. Must be at or beyond `range`. `escort`'s shape. |
| `target_requirement` | `"present"` \| `"condition_track"` \| `"capacity"` | *(the verb's own)* | What the target has to be. Override it to tow only damaged hulks, or to stabilise something unusual. |
| `stall_limit_secs` | i64 | *(none)* | Whole seconds of **cumulative** stalled time tolerated before the operation fails. Omit to let it stall indefinitely. |

`[operations.capability.transfer]` — required for a `transfer`, and what makes
the two ends two ends:

| Field | Type | Default | Notes |
|---|---|---|---|
| `capacity` | string | **required** | The `[[infrastructure.capacity]]` id being moved. Both the operator and the target must carry one under this id. |
| `amount` | i64 | **required** | How much moves, in the capacity's own units. Paid once, on completion. |
| `direction` | `"deliver"` \| `"collect"` | **required** | Named from the **operator's** point of view: a tender *delivers* to a depot and *collects* from it. |

`[[operations.capability.interrupt]]` — zero or more. A capability that authors
none behaves exactly as it did before interrupts existed: only eligibility can
stop the hold.

| Field | Type | Default | Notes |
|---|---|---|---|
| `cause` | `"attack"` \| `"region"` | **required** | Recent *landed hits* on the operator (firing your own guns is not being attacked), or membership of a region carrying `region_effect`. |
| `region_effect` | `"slow_zone"`, `"damage_zone"`, `"comms_jam"`, … | *(none)* | Required for `cause = "region"`, refused for anything else. The names mirror the `[effects]` sub-tables in **§2.5**. |
| `response` | `"slow"` \| `"pause"` \| `"fail"` | **required** | Keep going more slowly; freeze and spend the stall budget; or end it now. |
| `rate_percent` | u16 | `50` | For `response = "slow"`: what fraction of normal speed, `1..=100`. Author `pause` for a full stop. |

Two rules that both fire take the **stricter** response, and two `slow` rules
take the **lower** rate — so a capability carrying both cannot get a different
answer depending on which line you typed first.

**Power loss is not an interrupt cause**, deliberately. It is already an
eligibility condition, tested against the live grid every tick, and a second
spelling would let one capability say two different things about it.

A hold **stalls** rather than ending when eligibility lapses for something the
crew can fix — out of range, under-powered, a depot with no room, no free repair
team — and progress freezes where it stood rather than decaying, so recovering
resumes it. It **fails** when eligibility is lost for something they cannot fix
(the target is gone; the escortee is past the separation limit; the hull never
had the capability), when an authored interrupt says `fail`, or when the stall
budget runs out. A `slow` interrupt does *not* spend the stall budget, however
long it lasts: the operation is being held, just badly.

Refused at load, by field name: a zero range, a non-positive `duration_secs`, an
empty `power_group`, a negative payoff of either kind, a `separation_limit`
inside `range`, a `transfer` with no capacity id or a non-positive amount, a
`transfer` requirement with no transfer block, an interrupt with `cause =
"region"` and no `region_effect` (or the reverse), a `slow` rate outside
`1..=100`, a negative stall budget, or two blocks claiming the same verb.

Starting one: `ctx.effects.stabilise(ship, target)` — or `tow`, `escort`,
`transfer`, `field_repair` — from a script (**§1.5**), or a `StartOperation` /
`AbortOperation` console command at the `captain` system. Progress reaches the
crew on the operations blackboard, rendered by `<ph-operation-panel>`, which
offers a verb picker when the hull can do more than one.
`assets/worlds/probe_stabilise.toml` is a worked example of one verb end to end;
`assets/worlds/probe_operations.toml` runs the other four, plus a storm band
that slows two of them.

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
| `[compliance]` | table | none | How this faction's civilian traffic answers crew orders — the same shape as an entity's `[civilian.compliance]` (see §1.8). The fallback when a hull authors none of its own. |

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
