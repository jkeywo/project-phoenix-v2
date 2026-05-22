---
title: Comms range
---

# Comms range

Range-gated hailing for the Comms console. Entities opt in via a `[comms].range = N` block on their EntityConfig TOML. A contact is reachable when `distance <= min(ship_range, entity_range)`. Entities without a `[comms]` block never appear in the contacts list.

## Data model

### TOML

```toml
# assets/entities/player_ship.toml, station_outpost.toml, pirate_raider.toml, ...
[comms]
range = 500.0
```

Parsed by `CommsConfig { range: f32 }` in `src/entities/config.rs:542`. Optional field on `EntityConfig`.

### ECS

- `CommsRange(pub f32)` Component — `src/comms/component.rs`. Inserted by `entities/spawner.rs` (`src/entities/spawner.rs:198`) when `config.comms.is_some()`.
- Pure helper: `comms::in_range(distance, a, b) -> bool` returns `distance <= a.min(b)` and is false for NaN inputs. `src/comms/range.rs`.

### Wire

- `CommsContact.in_range: bool` — `src/core/messages.rs:262`.
- `CommsMessage.sender_in_range: bool` — `src/core/messages.rs:255`.
- Both fields use `#[serde(default = "default_true")]` for backward-compat with older JSON payloads.

## Server flow

`update_comms_range_flags` (`src/world/server.rs:759`) runs every tick in `SimSet::Broadcast` immediately before `broadcast_comms_state`:

1. Reads the player `Ship` Transform + its `CommsRange`.
2. Walks `Query<(&EntityUuid, &Transform, &CommsRange)>`, computing per-entity `in_range`.
3. Updates `WorldContentRuntime.range_flags: HashMap<String, bool>`.
4. Prunes flags for despawned entities; prunes contacts whose entity lost (or never had) a `CommsRange` component (this is what excludes `[[comms]]`-template entries without a `[comms]` block).
5. Sets `needs_broadcast = true` on any flip so `CommsState` re-broadcasts even when the inbox is clean.

`broadcast_comms_state` (`src/world/server.rs:875`) then stamps `m.sender_in_range` per message from `range_flags`; missing UUIDs default to `false` when `range_active == true`.

New `CommsMessage` instances are stamped at injection time via `current_sender_in_range(&runtime, &sender_uuid)` (`src/world/server.rs:60`) — belt-and-braces so the field is correct from the moment the message lands, not only after the next broadcast.

### `range_active` semantics

`WorldContentRuntime.range_active` starts `false`. It flips `true` once `update_comms_range_flags` locates a player `Ship`. Once true it stays true even if the ship is later despawned mid-game; in that case every tracked flag is forced to `false` instead of the resource silently re-enabling all comms. This closes a back-door past the server-side Hail/Respond gates.

While `range_active == false` (lobby phase, pure-handler tests), range gating is fully bypassed and `sender_in_range`/`in_range` stay at their default `true`.

## Server enforcement

`handle_hail` and `handle_respond_to_message` (`src/world/server.rs:548`, `:648`) both reject the message when `range_active == true` and the target/sender's `range_flags` entry is missing or `false`. This means a stale or malicious client cannot bypass the gate.

## Client UI

`src/console/comms/client.rs` `refresh_all_comms_ui`:

- **Contacts strip** — out-of-range contacts are **hidden** (not greyed). Empty state shows "All contacts out of range".
- **Inbox rows** — sticky: every message stays. When `!sender_in_range`, the row appends an alert-red `[OUT OF RANGE]` tag (colour `Color::srgb(1.0, 0.2, 0.267)`, matching `COLOR_ALERT_RED` from `src/server/viewscreen_border.rs:119`).
- **Chat panel** — when viewing a message whose sender is out of range, response buttons are replaced with a disabled label. `detect_comms_clicks` also rejects clicks on those buttons.

## Values today

| Entity | Range | Notes |
|---|---|---|
| Player ship | 500 | `assets/entities/player_ship.toml` |
| Starbase Alpha (station_outpost) | 800 | `assets/entities/station_outpost.toml` |
| Pirate raider | 400 | `assets/entities/pirate_raider.toml` — calibrated so distress comms fire during typical phaser-range engagements |

Default world has ship at `(150, 0, 0)`, Starbase at `(500, 0, 0)` → distance 350, in range at game start. Raider at `(300, 0, -300)` → out of range until the player closes inside ~400u during combat.

## Tests

- Pure: `src/comms/range.rs` — equal/less/greater/zero/negative/NaN distance and range.
- Codec round-trip + missing-field defaults: `src/core/codec.rs` (`comms_state_payload_with_no_range_flags_defaults_both_to_true` and per-type tests).
- Entity spawning: `src/entities/spawner.rs` — `CommsRange` inserted iff `[comms]` block present.
- Server: `src/world/server.rs` integration tests cover broadcast stamping, contact pruning, server-side Hail/Respond rejection, entity-despawn-flips-sender-in-range, multi-entity independent flags, range-flip-triggers-broadcast, and ship-despawn-keeps-gates-closed.

## Sources

- `src/comms/{mod.rs,range.rs,component.rs}`
- `src/core/messages.rs`, `src/core/codec.rs`
- `src/entities/config.rs`, `src/entities/spawner.rs`
- `src/world/server.rs`
- `src/console/comms/client.rs`, `src/client_comms.rs`
- `assets/entities/player_ship.toml`, `station_outpost.toml`, `pirate_raider.toml`
