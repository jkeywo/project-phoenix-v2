# Increment 7 — configurable weapons

Implementation and automated verification pass. Physical phone/native monitor
and human-workload acceptance remains pending, including earlier increments.
The user authorised advancement through increment 18, then explicitly requested
validation and fixes on isolated worktrees while main continues the next increment.

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
- Shared pure weapons owner: 248 tests passed; initial authoring: 8; Content merge: 36.
  The final compiler's nine tests, including the zero-cadence regression, pass
  within the integrated native suite.
- Integrated Simulation weapons owner: 282 passed; integrated Workshop: 64.
- Shared mechanism and compiler/runtime/Workshop read-only reviews pass. The
  integrated validator preserves zero-delay simultaneous volleys; unknown
  pattern fields are refused rather than ignored.

- Required integrated gates pass: 9,149 native tests with 102 existing ignored;
  4,263 simulation tests without default features; 8,762 JavaScript tests across
  442 files; format, strict Clippy, layout/string/LOD/layer and all three PASM gates.
- Chromium acceptance passes ten Operations/weapon wire and UI cases. Rendered
  Workshop acceptance passes inherited editing, materialisation, invalid-input
  refusal, history, export/reload, actual Cruiser Preview geometry and disposable
  Test. The inspected screenshot is hashed in the receipt. These checks do not
  establish physical-device usability or station workload.
- Combat Test and Falling Skyway reports match Increment 6 byte for byte;
  repeated seeded runs, replay checkpoints and performance continuations agree.
  The unchanged Cruiser matrix passes 20/20 victories and all 16 thresholds,
  with identical merged results. Tick measurements and their limits are recorded
  in the comparison; no balance thresholds or performance budgets changed.

Runtime and gate revision is `5e7264d5`; capture revision `e848e2f3` adds only
approved future planning documents. The matrix uses an isolated worktree pinned
to that capture, allowing main to proceed without changing its inputs. No new
authoritative state, save schema, simulation rules or transport was introduced.

See [validation receipts](../gameplay-redesign/implementation/weapons-validation.json)
and [baseline comparison](../gameplay-redesign/implementation/weapons-baseline.md).
The ledger remains `in_progress` for physical/device and human acceptance; the
user's later advancement instruction allows subsequent implementation to continue.
