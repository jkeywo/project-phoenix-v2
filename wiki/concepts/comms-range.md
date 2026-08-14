---
title: Comms range
---

# Comms range

Range-gated hailing for the Comms console. Entities opt in via a `[comms].range = N` block on their EntityConfig TOML. A contact is reachable when `distance <= min(ship_range, entity_range)`. Entities without a `[comms]` block never appear in the contacts list.

`range` and hailability are **separate authored facts** (#985). `range` marks a
range-gated comms *endpoint* — every shipped warship and station declares one —
and by itself puts nothing on the roster. `hailable = true` is what adds the
entity to the roster. Nothing in shipped content sets it yet; see
[the roster sources](#roster-sources) below.

## Data model

### TOML

```toml
# assets/entities/station_outpost.toml, ship_harrow_patrol.toml, alliance_cruiser.toml, ...
[comms]
range = 500.0

# Optional (#985) — opt in to the hail roster, with an optional contact label.
# Authored on NOTHING in shipped content today; the Rhai M7 world conversions
# turn it on per world as they retire that world's `[[comms]]` block.
hailable = true
display_name = "Relay Outpost"
```

Parsed by `CommsConfig { range, hailable, display_name }` in
`src/entities/config.rs`. Optional field on `EntityConfig`.

### ECS

- `CommsRange(pub f32)` Component — `src/comms/component.rs`. Inserted by `entities/spawner.rs` when `config.comms.is_some()`.
- `CommsHailable { display_name }` Component — `src/comms/component.rs`. Inserted alongside it only when `hailable = true`.
- Pure helper: `comms::in_range(distance, a, b) -> bool` returns `distance <= a.min(b)` and is false for NaN inputs. `src/comms/range.rs`.

## Roster sources

`CommsRuntime.contacts` — who the Comms officer (human or Backfill AI) can hail
— has two sources during the Rhai transition, unioned on entity UUID:

| Source | Built by | Contact label |
|---|---|---|
| Declarative `[[comms]]` templates in the world TOML | `init_comms_runtime` / `merge_world_comms`, resolving each template's `from` through `name_to_uuid` | template `display_name`, else `from` |
| Live entities carrying `CommsHailable` | `update_comms_range_flags`, via the pure `merge_entity_contacts` in `src/comms/roster.rs` | `[comms] display_name`, else the entity's `EntityName`, else the UUID |

On a UUID collision the **declarative entry wins** — it keeps its authored name
and its live `in_range`/`is_urgent` stamps, so every shipped world's roster is
unchanged while both sources coexist. Entity-derived contacts are appended
after the declarative ones, sorted `(name, uuid)` so the player-visible order
never depends on ECS query order. At Rhai M7 the declarative half is deleted
and the entity-derived half becomes the only source.

### Wire

- `CommsContact.in_range: bool` — `src/core/messages.rs:493`.
- `CommsMessage.sender_in_range: bool` — `src/core/messages.rs:467`.
- Both fields use `#[serde(default = "default_true")]` for backward-compat with older JSON payloads.

## Server flow

`update_comms_range_flags` (`src/comms/server.rs`, relocated from `src/world/server.rs` in #816) runs every tick in `SimSet::Broadcast` immediately before `broadcast_comms_state`:

1. Reads the player `Ship` Transform + its `CommsRange`.
2. Walks `Query<(&EntityUuid, &Transform, &CommsRange, Option<&CommsHailable>, Option<&EntityName>)>`, computing per-entity `in_range` and collecting the entity-derived hail candidates in the same pass.
3. Updates `WorldContentRuntime.range_flags: HashMap<String, bool>`.
4. Prunes flags for despawned entities; prunes contacts whose entity lost (or never had) a `CommsRange` component (this is what excludes `[[comms]]`-template entries without a `[comms]` block).
5. Unions the entity-derived candidates into `contacts` (#985) — after the prune, before the range stamp, so a new contact carries its real reachability on the tick it appears. This is also the roster's lifecycle: a hailable entity joins the tick after it spawns and drops the tick after it is destroyed, off the same live set the prune uses.
6. Sets `needs_broadcast = true` on any flip so `CommsState` re-broadcasts even when the inbox is clean.

`broadcast_comms_state` (`src/comms/server.rs`) then stamps `m.sender_in_range` per message from `range_flags`; missing UUIDs default to `false` when `range_active == true`.

New `CommsMessage` instances are stamped at injection time via `current_sender_in_range(&runtime, &sender_uuid)` (`src/console/comms/server.rs:97`) — belt-and-braces so the field is correct from the moment the message lands, not only after the next broadcast.

### `range_active` semantics

`WorldContentRuntime.range_active` starts `false`. It flips `true` once `update_comms_range_flags` locates a player `Ship`. Once true it stays true even if the ship is later despawned mid-game; in that case every tracked flag is forced to `false` instead of the resource silently re-enabling all comms. This closes a back-door past the server-side Hail/Respond gates.

While `range_active == false` (lobby phase, pure-handler tests), range gating is fully bypassed and `sender_in_range`/`in_range` stay at their default `true`.

## Server enforcement

`handle_hail` and `handle_respond_to_message` (`src/console/comms/server.rs:113`, `:235` — relocated from `src/world/server.rs` in #608) both reject the message when `range_active == true` and the target/sender's `range_flags` entry is missing or `false`. This means a stale or malicious client cannot bypass the gate.

## Client UI

`src/console/comms/client.rs` `refresh_all_comms_ui`:

- **Contacts strip** — out-of-range contacts are **hidden** (not greyed). Empty state shows "All contacts out of range".
- **Inbox rows** — sticky: every message stays. When `!sender_in_range`, the row appends an alert-red `[OUT OF RANGE]` tag (colour `Color::srgb(1.0, 0.2, 0.267)`).
- **Chat panel** — when viewing a message whose sender is out of range, response buttons are replaced with a disabled label. `detect_comms_clicks` also rejects clicks on those buttons.

## Values today

Thirteen shipped entity templates declare a `[comms]` block. None sets
`hailable`, so none of them is on the roster by way of this block — the census
is exactly why hailability is a separate opt-in (#985).

| Entity template | Range |
|---|---|
| `alliance_battleship.toml` | 1000 |
| `alliance_courier.toml` | 1000 |
| `alliance_cruiser.toml` | 1200 |
| `alliance_destroyer.toml` | 1000 |
| `ship_harrow_cruiser.toml` | 700 |
| `ship_harrow_destroyer.toml` | 800 |
| `ship_harrow_patrol.toml` | 600 — the ambient `raider_alpha` in `default.toml`/`patrol.toml` since #892 retired `pirate_raider.toml` (which carried 400) |
| `ship_harrow_warhawk.toml` | 800 |
| `ship_requiem_courier.toml` | 800 |
| `station_axiom.toml` | 800 — Starbase Alpha in `default.toml` and `combat_test.toml` |
| `station_outpost.toml` | 1600 |
| `station_research_outpost.toml` | 600 |
| `test/rng_coverage_lancer.toml` | 800 |

Default world puts the player hull at `(280, 0, 0)` and Starbase Alpha at the `starbase_alpha` anchor `(1000, 0, 0)` → distance 720 against an effective 800 (the station's range is the binding constraint against a cruiser's 1200), so it is in range at game start. Raider at `patrol_alpha` `(600, 0, -600)` → distance 680 against an effective 600 (the raider's range binds), so it is out of range until the player closes during combat.

## Tests

- Pure: `src/comms/range.rs` — equal/less/greater/zero/negative/NaN distance and range.
- Pure: `src/comms/roster.rs` — label precedence, UUID de-duplication, declarative-wins collision, query-order independence, idempotence across ticks.
- Shipped content: `src/comms/roster.rs` (`shipped_world_rosters`) — asserts no shipped entity template opts in, and snapshots each shipped world's declarative roster (`combat_test`: 12 templates → 1 contact; `default`: 3 → 2; everything else empty).
- Codec round-trip + missing-field defaults: `src/core/codec.rs` (`comms_state_payload_with_no_range_flags_defaults_both_to_true` and per-type tests).
- Entity spawning: `src/entities/spawner.rs` — `CommsRange` inserted iff `[comms]` block present; `CommsHailable` iff `hailable = true`.
- Server: `src/comms/server.rs` integration tests cover broadcast stamping, contact pruning, server-side Hail/Respond rejection, entity-despawn-flips-sender-in-range, multi-entity independent flags, range-flip-triggers-broadcast, ship-despawn-keeps-gates-closed, and the entity-derived roster (opt-in gating, label precedence, spawn/despawn lifecycle, collision, append order).

## Sources

- `src/comms/{mod.rs,range.rs,component.rs,roster.rs}`
- `src/core/messages.rs`, `src/core/codec.rs`
- `src/entities/config.rs`, `src/entities/spawner.rs`
- `src/comms/server.rs` (CommsRuntime, range systems — relocated in #816)
- `src/console/comms/server.rs` (handle_hail / handle_respond_to_message / current_sender_in_range)
- `src/console/comms/client.rs`, `src/client_comms.rs`
- `assets/entities/player_ship.toml`, `station_outpost.toml`, `ship_harrow_patrol.toml`
