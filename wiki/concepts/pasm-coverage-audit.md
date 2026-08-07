---
title: PASM Coverage Audit
type: concept
tags: [pasm, audit, coverage, architecture, authority]
sources: [pasm/spec/architecture, pasm/spec/design, pasm/README.md, src/console_bridge.rs, src/core/codec.rs, src/server/bridge.rs, src/entities/tags.rs, src/entities/target.rs, src/console_ai/server.rs]
updated: 2026-07-14
---

# PASM Coverage Audit

## Summary

The feature backlog is fully marked captured and the authored architecture now
contains 329 entities. The model has strong feature breadth across gameplay,
world content, UI, and presentation. The remaining meaningful gaps are
cross-cutting authority, serialization, static-content evidence, and design
traceability rather than another missing console or mechanic.

The review used `pasm scan` against revision `9ae58ab5322f7f01e44a8fa40597e960b8fce489`.
It reported 4,519 repository files, 8,389 observed dependencies, and zero
validation errors. The scan is intentionally limited to Rust, JavaScript,
TypeScript, and HTML observations.

## Findings

### P1: Host-local console authority is not modelled

`src/console_bridge.rs:20` defines `LOCAL_CONSOLE_TOKEN`. The WASM bridge
decodes host-page actions and injects them as inbound messages under that token
in `src/server/bridge.rs:1017`. Tactical explicitly accepts this token as an
authorized local operator in `src/console/weapons/mod.rs` and routes it
to the local ship.

PASM models remote peer transport, sessions, station ownership, and command
admission, but has no entity for this host-local console bridge or its
authority exception. This is a material gap because it is a separate trusted
input path that does not start from a PeerJS session/station claim.

Resolved in the protocol/targeting/observation PASM slice.

### P1: The wire codec and HTML push protocol have no architecture contract

`src/core/codec.rs:1` owns the JSON codec, the sanctioned `serde_json`
boundary, short-form UI action rewriting, and host bridge decoding. The same
module serializes HUD, console, and lobby payloads. `src/console_bridge.rs:23`
defines the push messages, while `src/server/bridge.rs:1036` forwards them to
registered browser callbacks.

PASM maps individual messages incidentally, but does not model the codec,
wire-version/compatibility behavior, short-form normalization, or server-page
push callback boundary. A message-level feature slice therefore cannot state
or test which representations are accepted at which trust boundary.

Resolved in the protocol/targeting/observation PASM slice.

### P1: Authored TOML content is present but not observed as content

The five entities in `world-content-packs.yaml` report `Observed files: 0` in
`pasm scan`, despite mapping real files under `assets/worlds/`. PASM's scanner
recognizes code and HTML, not TOML or asset references. Path existence is
checked, but content changes, composition references, and root-versus-subworld
classification do not produce repository-observation evidence.

This is particularly important because worlds and entity templates carry a
large share of game behavior. The current `world-file-parser` model validates
runtime parsing, but it does not make the authored catalog auditable through
the Phase 5 observation layer.

Resolved for world and entity-template path references. Faction, complexity,
and model-rig semantic observation remain outside the deliberately narrow TOML
path scanner.

### P2: Entity classification and target presentation are not represented

`src/entities/tags.rs:13` defines typed entity tags and OR-based radar filter
matching. `src/entities/target.rs:8` defines targetability tags, cosmetic
threat level, and description. Tactical applies these filters and selection
rules in `src/console/weapons/mod.rs`.

Radar/Sensors and Entity Configuration mention filters and appearance, but
there is no explicit PASM contract for the entity tag vocabulary, targetability
metadata, target info projection, or the relationship between `shows` and
`selects`. This leaves an important authoring-to-player-information path
implicit.

Resolved in the protocol/targeting/observation PASM slice.

### P2: Retired low-complexity console AI is not distinguished from live AI

`src/server_app.rs:237` still registers `ConsoleAiPlugin`, but
`src/console_ai/server.rs:15` is an intentionally empty plugin after the old
complexity machinery was removed. `src/console_ai/core.rs` retains pure helper
rules for frequency, torpedo, power, and shield automation.

PASM records active per-system NPC and Backfill AI, including partial Sensors
and Comms operators, but does not record this dormant legacy surface. Without
that distinction, future work can mistake reusable pure helpers for active
runtime automation or overlook the registration seam.

Resolved as a deprecated PASM entity with a removal-or-migration condition. It
is intentionally removed from PASM when the corresponding code disappears.

### P2: Design traceability remains concentrated in three feature slices

Only Repair, Helm, and Red Alert have authored game-design files under
`pasm/spec/design/`. Architecture now covers all backlog features, but player
information, authority, failure, and recovery decisions for areas such as
Shields, Comms, Navigation, objectives, and game flow have no Phase 7/8 links.

This is not a code-model omission, but it limits PASM's ability to distinguish
an implemented mechanic from a gameplay-approved one outside the original
three slices.

Recommended capture: add concise design slices first for Comms, Navigation,
Shields, game flow, and player objective visibility; link only decisions that
need enforcement or player-information review.

### P3: Observation remains file-level and has broad ownership buckets

The scanner accurately reports direct Rust/JS/HTML edges, but it cannot
attribute individual callers within shared files or observe runtime dataflow.
For example, at the time of this audit `client-console-registry` owned 88
observed UI files (that module has since been deleted in #827; the equivalent
broad-ownership bucket is now `gui/console-state.js` / `gui/mount-plan.js`),
while multiple subsystems share `src/ship_plugin.rs` and `src/server_app.rs`.
Existing Helm migration warnings demonstrate the resulting ambiguity.

This is an accepted Phase 5/6 limit, not a validation failure. It means PASM
should continue to treat observations as evidence for focused audits, not proof
of system authority, control flow, or semantic reachability.

## Existing Findings

`pasm validate` reports 39 informational warnings and exits 0 with `Status: OK`
(see `pasm/README.md`); the category mix drifts as the model and code evolve.
They are useful visibility rather than new feature-coverage failures.

## Recommended Order

1. Add focused design traceability for Comms, Navigation, Shields, game flow,
   and objectives.
