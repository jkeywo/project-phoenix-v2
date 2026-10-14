# Increment 7 — configurable weapons

Automated implementation is under verification. Physical phone/native monitor
and human-workload acceptance remains separate, including earlier increments.
The user authorised serial advancement through increment 18 on 14 October 2026.

## Implemented migration

All 36 weapons in ten authored entity files now use `[[weapon]]`, a common
mount and a typed beam/bolt/guided delivery. The distinct composition `id` and
existing `strategy_id` retain bank/tube and System addressing. Existing family
parameter blocks remain supported shorthand; conflicting identities are refused.
Compilation produces one set of existing strategy parameters, without another
live weapon, controller or mutable configuration store.

Shared mount geometry, impact delivery and barrel planning are consumed by the
actual weapon adapters. Existing beam cycles, jitter, fractional damage, bolt
lead prediction and loaded-round torpedo bursts retain their lifecycles. Guided
flight/payload tuning remains shared in `[torpedoes]`; per-tube range cannot
silently disagree with that tuning. New interception/traversing mounts/payload
families remain increment 15.

Workshop offers supported delivery, mount, marker and pattern editing, creation,
explicit inherited materialisation and the existing validated source/history,
export/reload, Preview and disposable Test journey. New weapons explicitly start
with an idle AI declaration, editable through ordinary AI authoring; missing
ship-level required declarations are still validation errors.

No new authoritative state or wire payload is introduced. Snapshot 42, simulation
rules 0.14 and protocol 6 continue to carry the existing lifecycle state.

## Operator checks

1. On a Cruiser, operate Tactical, select a contact, use manual/automatic beams,
   load and fire tubes, then delegate and reconnect. Check independent cooldowns,
   committed loading rounds and ordinary AI continuation.
2. On a Destroyer or Courier, operate its bolt bank; check charge/cancel or
   instant fire according to its authored delivery. Check the Battleship's
   supported eight-seat journey alongside all four shipped crew sizes.
3. In Workshop, edit an inherited weapon after explicit materialisation. Undo,
   redo, save/reload and export/reload; refuse invalid arcs and pattern indices
   without changing the source. Preview real geometry, run disposable Test,
   operate/delegate the equipped weapon, stop Test and check source preservation.
4. Save during a beam/cooldown, a partial bolt volley and tube loading/loaded
   burst; resume with the same equipment and verify continuation, authority and
   magazine conservation. Compare multiplayer deterministic checkpoints.

Record real-device outcomes here without treating browser wire tests or
headless balance runs as physical-device or human attention acceptance.

## Current verification

- Independent audits preserve the 13 beam, 8 bolt and 15 guided definitions,
  ordered policies, markers, patterns and all other authored entity data.
- Shared pure weapons owner: 248 tests passed; authoring: 8; Content merge: 36.
- Integrated Simulation weapons owner: 282 passed; integrated Workshop: 64.
- Shared mechanism and compiler/runtime/Workshop read-only reviews pass. The
  integrated validator preserves zero-delay simultaneous volleys; unknown
  pattern fields are refused rather than ignored.

Integrated gate, rendered-browser and baseline receipts follow when verified.
