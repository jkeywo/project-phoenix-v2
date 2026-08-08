---
title: PASM Coverage Audit
type: concept
tags: [pasm, audit, coverage, architecture, authority]
sources: [pasm/spec/architecture, pasm/spec/design, pasm/README.md, src/console_bridge.rs, src/core/codec.rs, src/server/bridge.rs, src/command_admission/policy.rs, src/entities/tags.rs, src/entities/target.rs, src/console_ai/server.rs, src/server_app.rs]
updated: 2026-08-09
---

# PASM Coverage Audit

> **Historical record — not current-code navigation.**
> This is a point-in-time PASM coverage audit. Its findings, counts, and scan
> figures were true **as of 2026-07-14** against revision `9ae58ab5`, and are
> preserved as written. It is deliberately *not* maintained as a live page: per
> `wiki/SCHEMA.md` the wiki is a current-state orientation layer, and a dated
> audit re-verified on every refactor is pure churn.
>
> The exception is a finding that later became **false** rather than merely
> superseded: those are corrected in place and flagged, because a reader cannot
> tell a stale gap from an open one. Line numbers have been dropped from
> already-resolved findings — the file path and symbol name survive a refactor,
> a line number does not.
>
> Full lint against `main` on 2026-08-09 (issue #972).

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

`src/console_bridge.rs` defines `LOCAL_CONSOLE_TOKEN`. Host-page console
actions reach the simulation under that token through
`server::bridge::wasm_receive_message`, and the bridge injects host-only
commands under it directly (`drain_god_mode_toggle`).

**Corrected 2026-08-09:** this finding originally said Tactical accepts the
token as an authorized local operator in `src/console/weapons/mod.rs`. That is
no longer where acceptance lives — `LOCAL_CONSOLE_TOKEN` does not appear in
that module at all (only in its `server_tests.rs`). Acceptance is centralised
in `command_admission::policy`, which is the correct reading of the boundary
regardless: the token is an admission-level authority, not a per-console one.
The original text also placed action *decoding* in the WASM bridge; issue #822
moved it to `gui/action-map.js`, which now submits full `ClientMessage` JSON.

PASM models remote peer transport, sessions, station ownership, and command
admission, but has no entity for this host-local console bridge or its
authority exception. This is a material gap because it is a separate trusted
input path that does not start from a PeerJS session/station claim.

Resolved in the protocol/targeting/observation PASM slice.

### P1: The wire codec and HTML push protocol have no architecture contract

`src/core/codec.rs` owns the JSON codec, the sanctioned `serde_json` boundary,
and host bridge decoding. The same module serializes HUD, console, and lobby
payloads. `src/console_bridge.rs` defines the push messages, while
`server::bridge::flush_host_channels` forwards them to the registered browser
callback.

**Corrected 2026-08-09:** this finding also credited `codec.rs` with
"short-form UI action rewriting". Issue #822 retired that shim once no console
emitted short form, and the bridge decoder now rejects bare short-form payloads
outright. The rest of the sentence stands.

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

`src/entities/tags.rs` defines the typed `EntityTag` vocabulary and OR-based
radar filter matching. `src/entities/target.rs` defines `TargetSection` —
targetability tags, cosmetic threat level, and description. Tactical applies
these filters and selection rules in `src/console/weapons/mod.rs`.

Radar/Sensors and Entity Configuration mention filters and appearance, but
there is no explicit PASM contract for the entity tag vocabulary, targetability
metadata, target info projection, or the relationship between `shows` and
`selects`. This leaves an important authoring-to-player-information path
implicit.

Resolved in the protocol/targeting/observation PASM slice.

### P2: Retired low-complexity console AI is not distinguished from live AI

`src/server_app.rs` registers `ConsoleAiPlugin`, defined in
`src/console_ai/server.rs`. `src/console_ai/core.rs` retains pure helper rules
for frequency, torpedo, power, and shield automation.

**Error, corrected 2026-08-09 — this was wrong when written, not merely
stale.** The finding claimed `ConsoleAiPlugin` was "an intentionally empty
plugin after the old complexity machinery was removed". What B4 (issue #534)
removed was the *complexity-preset machinery* — `ComplexityRules`,
`ConsoleComplexityState`, `build_complexity_rules`, `track_complexity_changes`
— not the plugin body. `ConsoleAiPlugin::build` is and was live: it calls
`ai::cadence::register_ai_cadence` and registers `ai_shield_focus`,
`ai_power_allocation`, `ai_torpedo_auto_fire`, `ai_torpedo_load`, and
`tick_frequency_hint_high_fidelity` under the shared AI tick cadence
(issues #692/#694/#698/#826/#831/#873/#889/#895). The registration seam is
therefore live runtime automation, and the "dormant legacy surface" framing
below applies only to the retired presets.

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

**Mostly resolved — verified 2026-08-09.** `pasm/spec/design/` now carries
fourteen slices, so the "only Repair, Helm, and Red Alert" reading is no longer
true of the repository. Four of the five recommended captures landed —
`comms.yaml`, `navigation.yaml`, `shields.yaml`, `game-flow.yaml` — alongside
`engineering-diagnosis.yaml`, `helm-controls.yaml`, `red-alert.yaml`,
`power.yaml`, `sensors.yaml`, `viewscreen.yaml`, `station-ratings.yaml`,
`ship-manuals.yaml`, `editor-authoring.yaml`, and `host-debug-controls.yaml`.
**Player objective visibility remains uncaptured**: objectives are modelled in
`pasm/spec/architecture/objectives.yaml` but have no Phase 7/8 design slice.

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

`pasm validate` exits 0 with `Status: OK` and reports a baseline of
informational warnings (see `pasm/README.md`) — 39 at the time of this audit,
40 on 2026-08-09. Both the count and the category mix drift as the model and
code evolve, so the number is recorded here as an observation, not a target: a
change that neither adds nor resolves a warning should leave it alone rather
than chase it. They are useful visibility rather than new feature-coverage
failures.

## Recommended Order

1. Add focused design traceability for Comms, Navigation, Shields, game flow,
   and objectives. *(Verified 2026-08-09: Comms, Navigation, Shields, and game
   flow have landed under `pasm/spec/design/`; objectives has not.)*
