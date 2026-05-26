# PRD: Engine Additions for Branching Scenarios

## Problem Statement

Scenario authors writing branching narrative content for the bridge simulator are blocked by gaps in the world-engine vocabulary. The current trigger system supports only four conditions (`on_destroyed`, `on_attacked`, `on_timer`, `on_hailed`) and has no concept of cross-trigger state. As a result, authors cannot express common patterns such as: "fire this comm the moment the player enters a nebula", "arm this weapon when either of two unrelated events occurs but only once", "this NPC stays idle until a story beat triggers, then attacks", or "spawn this rescue pod after the enemy is destroyed". They also cannot model damage that bypasses shields — a mechanic needed for scripted "absorb the blast" sequences — and the asteroid-field spawner cannot describe a ring belt, so system layouts with an inner orbital belt require either no belt or a wrong-shaped one.

Without these additions, every non-trivial scenario fights the engine. The "Before the Fire" scenario in `docs/scenarios/before_the_fire.md` is the immediate forcing function: it requires all of these capabilities to be authored honestly rather than faked with hacks.

## Solution

Add a small, composable set of engine primitives that together cover the missing vocabulary:

- A flag-and-counter system with predicate-filtered triggers, so authors can express cross-event state, gates, OR-arming, and reactive story beats.
- Three new reactive trigger conditions (`on_world_loaded`, `on_entered_region`/`on_exited_region`, `on_flag_set`/`on_flag_cleared`) to cover the most common authoring patterns observed while drafting "Before the Fire".
- Two new trigger actions (`spawn_entity`, `destroy_entity`) for ad-hoc entity lifecycle outside of the existing `load_world`/`unload_world` pair.
- A universal `shield_pierce` property on damage sources, so scripted weapons and hazardous regions can bypass shields by a configurable fraction.
- A torus shape for the asteroid-field streaming spawner, so ring belts can be authored.
- A verified-idempotent `load_world` action, so authors can fire it from multiple converging triggers without worrying about double-loading.

Each addition is small enough to land independently and is designed to be reused by future scenarios, not bespoke to "Before the Fire". The flag system in particular is the load-bearing addition — once flags + the predicate filter exist, most other narrative gating becomes authorable in TOML without further engine work.

## User Stories

1. As a scenario author, I want to fire a trigger the moment a (sub-)world finishes loading, so that I can deliver opening comms and initial story beats without abusing zero-second timers.
2. As a scenario author, I want to fire a trigger when the player's ship enters a named region, so that I can deliver warning comms, start hazard timers, or arm story beats based on geography.
3. As a scenario author, I want to fire a trigger when the player's ship exits a named region, so that I can deliver "you're clear" comms and reset hazard state.
4. As a scenario author, I want to set, clear, and check named boolean flags from any trigger, so that I can express story state that outlives the trigger that produced it.
5. As a scenario author, I want to increment and assign integer counters from any trigger, so that I can track repeated events (such as evacuation rounds completed) and branch on the count.
6. As a scenario author, I want flags and counters to be scoped to the world file that owns them, so that two scenarios authored independently cannot collide on a generic name like `armed`.
7. As a scenario author, I want to reach into a parent (loader) world's flag namespace using an explicit `parent:` prefix that I can stack (`parent:parent:`) to walk up the loader chain, so that sub-scenarios can read and write the orchestrating scenario's state without implicit name resolution.
8. As a scenario author, I want to attach a `when = "..."` predicate filter to any trigger, so that the trigger fires its actions only when the predicate currently holds, regardless of its base condition.
9. As a scenario author, I want the `when` predicate to support `and`, `or`, `not`, `flag(name)`, and numeric counter comparisons (`counter(name) >= N`, plus `>`, `==`, `<=`, `<`), so that I can express realistic narrative gates.
10. As a scenario author, I want to fire a trigger the moment a specific flag transitions to set, so that I can react to story state changes without polling.
11. As a scenario author, I want to fire a trigger the moment a specific flag transitions to cleared, so that I can model recovery and reset states symmetrically.
12. As a scenario author, I want to spawn a single entity from a trigger action by referencing a template path and a position (anchor name or explicit coordinates), so that I can introduce rescue pods, debris, reinforcements, and other ad-hoc entities without authoring a whole sub-world.
13. As a scenario author, I want to destroy a named entity from a trigger action, so that I can script narrative deaths (the courier denounced and executed off-camera, the saboteur's ship vaporised) without faking it via huge negative modifiers.
14. As a scenario author, I want any damage source — region damage zones, phaser banks, torpedoes — to carry an optional shield-pierce fraction (0.0–1.0, default 0.0), so that I can author hazards or weapons that bypass shields by a configurable amount.
15. As a scenario author, I want pierced damage to apply directly to hull and console state, bypassing the normal quadrant shield mitigation, so that the absorbed-fraction-versus-pierced-fraction split is consistent regardless of damage source.
16. As a scenario author, I want to author asteroid belts as torus shapes with inner and outer radii around an anchor, so that I can describe orbital belts in my system layout TOML and have the streaming spawner populate them correctly.
17. As a scenario author, I want `load_world` to be a no-op when the target world is already loaded, so that I can wire two converging triggers to load the same sub-world without writing flag-based de-duplication on every fork.
18. As a scenario author, I want all new trigger conditions and actions to compose with existing ones (the `when` filter applies to any condition; flag actions can fire from any condition; the new conditions accept the same action list), so that the engine surface remains orthogonal rather than balkanised.
19. As a scenario author, I want a clear, documented error if my `when` predicate fails to parse or references an undefined flag at load time, so that I find typos at world-load rather than during play.
20. As a scenario author, I want predicate evaluation to treat an unset boolean flag as `false` and an unset counter as `0`, so that I do not have to initialise every flag in an opening trigger before using it in conditions.
21. As an engine maintainer, I want the flag-and-predicate subsystem to be a pure module testable without Bevy, so that the rules can be verified with fast unit tests and the same code paths run on native and WASM.
22. As an engine maintainer, I want the new trigger conditions (`on_world_loaded`, `on_entered_region`, `on_exited_region`, `on_flag_set`, `on_flag_cleared`) to be driven by events produced by existing systems rather than by per-tick polling, so that no new hot-path scan is added to the simulation loop.
23. As an engine maintainer, I want `shield_pierce` to default to `0.0` everywhere and be applied uniformly in the damage-application path, so that existing TOMLs are unchanged in behaviour and the new property has one well-tested implementation site.
24. As an engine maintainer, I want the torus asteroid shape to share the existing deterministic density seeding (`(field_idx, gx, gz) + Perlin`), so that determinism guarantees described in `AGENTS.md` remain intact.

## Implementation Decisions

### New / Modified Modules

- **Flag store and predicate evaluator** — a new pure module with no Bevy dependency. Owns the per-world flag namespace data structure (booleans and integer counters keyed by name), the `parent:` prefix walker that resolves a reference against the loader chain, the infix predicate parser (`and`/`or`/`not`/`flag(name)`/`counter(name)` with comparison operators), the predicate evaluator, and all error types. The module exposes a small surface: parse a predicate string, evaluate a parsed predicate against a snapshot of the relevant world's flag state, mutate flags via the action verbs. The Bevy integration plugin wraps this module with a resource and event types.

- **World trigger system** — extended to accept the new trigger conditions and to apply the optional `when` filter before dispatching actions. The condition-to-event wiring for `on_entered_region`/`on_exited_region` consumes new transition events emitted by the region containment system. The condition-to-event wiring for `on_flag_set`/`on_flag_cleared` consumes events emitted by the flag store on transition. `on_world_loaded` fires from the existing sub-world load pipeline at the same point that trigger states are registered.

- **Region containment system** — extended to emit per-ship-per-region enter/exit events when its existing internal containment state transitions. The containment data structure does not change; only the event emission is new.

- **Damage application** — every damage source path (region `damage_zone`, phaser banks, torpedoes, any future source) is updated to read a `shield_pierce: f32` field defaulting to `0.0`. The damage-application function splits incoming damage into a pierced portion (applied directly to hull/console state) and an absorbed portion (passed through the existing quadrant shield mitigation logic).

- **Asteroid field spawner** — extended with a new shape variant for torus belts (inner radius, outer radius, anchor). The deterministic density seeding is preserved; the shape variant determines which cells are eligible for population.

- **World layer loader** — `load_world` action is verified to no-op when the target path is already present in `WorldLayerMap`; any gap in this guarantee is closed. The unload counterpart already handles missing paths.

- **World config parser** — extended to accept the new TOML keys: trigger `condition = "on_world_loaded" | "on_entered_region" | "on_exited_region" | "on_flag_set" | "on_flag_cleared"`, the optional `when = "<predicate>"` field on any `[[trigger]]`, the new action variants (`set_flag`, `clear_flag`, `increment_flag`, `set_flag_value`, `spawn_entity`, `destroy_entity`), the `shield_pierce` field on damage source schemas, and the torus shape variant on asteroid field templates. Predicate strings are parsed eagerly at world-load and reported with a useful diagnostic on failure.

### Schema Decisions

- Flag namespace is per containing world file. A reference written as a bare `name` resolves against the world the trigger is authored in. A reference written as `parent:name` walks up one level in the loader chain; `parent:parent:name` walks up two levels; and so on. There is no absolute path syntax and no implicit walk-up.
- Flags are a single namespace covering both booleans and integer counters. Setting a counter and then querying it as a boolean is a defined operation: any non-zero counter reads as `true` in `flag(name)`.
- `set_flag` and `clear_flag` operate on the boolean view (clear = set to 0; set = set to 1). `increment_flag { name, by }` adds to the integer view. `set_flag_value { name, value }` assigns the integer view directly.
- The `when` predicate is parsed once at world-load and stored on the trigger. Evaluation happens at the moment the base condition's event arrives, before actions are dispatched. A predicate that evaluates to `false` suppresses all of that trigger's actions for that firing; the trigger remains live for future firings.
- `on_flag_set { name }` and `on_flag_cleared { name }` fire on transition only, not on every set of an already-set flag. The transition is observed by comparing before/after state inside the flag-mutation action.
- `on_world_loaded` fires once per (sub-)world load. If the same world is unloaded and re-loaded, it fires again on the re-load.
- `on_entered_region` and `on_exited_region` fire only for the player's ship in the current scope; NPC region entry is not surfaced as a trigger condition in this PRD. (The containment system continues to apply region effects to NPCs as today.)
- `spawn_entity` takes `template_path` (required), `name` (required, joins the world's name-to-UUID table for subsequent reference), and exactly one of `anchor` (named anchor reference) or `position` (explicit `[x, y, z]`). Optional `rotation` and `scale` mirror the static `[[entity]]` schema. The spawned entity belongs to the world that authored the trigger for unload-cascade purposes.
- `destroy_entity { entity }` resolves the name, sets hull to zero, runs the normal destruction cascade (events, despawn, AI notifications), so consumers of `AiEntityDestroyed` see a uniform event regardless of cause.
- `shield_pierce` is a `f32` clamped to `[0.0, 1.0]`. The split is computed once per damage application: `pierced = dmg * shield_pierce`, `absorbed = dmg * (1.0 - shield_pierce)`. The absorbed portion enters the existing shield-quadrant pipeline; the pierced portion calls the same hull/console damage path used today when shields are fully depleted in the facing quadrant.
- Torus asteroid shape is defined by `{ shape = "torus", anchor = "name", inner_radius = N, outer_radius = M }`. The streaming window's cell-eligibility test changes per shape; the per-cell density seed does not.

### Composition Rules

- The `when` filter is applicable to every trigger condition, including the new ones. In particular, `on_flag_set { name = "a" } when = "flag(b)"` is well-formed and fires only on transitions of `a` while `b` is currently set.
- Trigger actions are not restricted by condition type. Any action can fire from any condition.
- The flag store is mutated only by trigger actions. There is no direct simulation write into flags from non-trigger systems in this PRD. (Future work — for example, an "objective complete" event auto-setting a flag — is explicitly out of scope.)

## Testing Decisions

Good tests set up state, perform an action, and assert on observable output through the public interface. They do not assert on private fields, internal call counts, or implementation details.

### Flag store and predicate evaluator — unit tests (pure module)

- Predicate parsing: round-trip a representative set of expressions (single flag, negation, conjunction, disjunction, counter comparisons, mixed operators with the documented precedence) and assert the parsed form re-evaluates as expected against a hand-built flag snapshot.
- Predicate parse-error reporting: malformed expressions produce a diagnostic identifying the offending token rather than a panic.
- Default values: an unreferenced flag is `false`; an unreferenced counter is `0`. A predicate referencing an undefined flag evaluates without error.
- `parent:` resolution: a reference with `n` `parent:` prefixes resolves against the world `n` levels up in a synthetic loader chain. Resolving past the root produces a defined "not found" result (predicate component evaluates as if unset).
- Flag mutations: `set_flag`, `clear_flag`, `increment_flag`, `set_flag_value` produce the expected before/after state; the boolean view of a non-zero counter reads as `true`.

Prior art: `core/codec.rs` round-trip tests; pure-module tests in `lobby/handler.rs` and `ship/damage.rs`.

### Region transition events — Bevy integration tests

- A ship that enters a region produces exactly one enter event; staying inside produces no further enter events. Exiting produces exactly one exit event.
- Multiple regions overlapping at the same point produce one enter event per region on the same tick.

### Trigger composition — Bevy integration tests

- An `on_flag_set` trigger fires exactly once on the set transition and does not fire on subsequent no-op sets of the same flag.
- A trigger with `when = "flag(a)"` does not fire its actions when `a` is unset; the same trigger fires when `a` is set; the trigger does not consume its lifecycle on a suppressed firing.
- A trigger that calls `set_flag a` and another trigger reactive to `on_flag_set a` in the same world chain in a single tick.
- `on_world_loaded` fires once for a sub-world load, and again if the sub-world is unloaded and re-loaded.

### load_world idempotence — integration test

- Two triggers in the same world both calling `load_world` on the same path within one tick produces exactly one load.
- The second call is a true no-op: no duplicate entities, no duplicate trigger-state registration.

### Damage application with shield_pierce — unit tests

- A damage source with `shield_pierce = 0.0` behaves identically to today: all damage enters the shield quadrant.
- A damage source with `shield_pierce = 1.0` bypasses shields entirely; hull damage equals the full incoming amount regardless of shield state.
- A damage source with `shield_pierce = 0.3` produces hull damage equal to 30% of the incoming amount even with shields at maximum, and the shielded quadrant absorbs 70%.
- Clamping: values outside `[0.0, 1.0]` are clamped without panic.

Prior art: `ship/damage.rs` unit tests.

### spawn_entity and destroy_entity — Bevy integration tests

- `spawn_entity` with an anchor reference places the entity at the anchor's position; with explicit `position`, places it at that coordinate.
- The spawned entity is reachable by `name` from subsequent triggers (e.g. a follow-up `destroy_entity { entity = "<that name>" }` finds and destroys it).
- `destroy_entity` produces the same `AiEntityDestroyed` event as combat-induced destruction.
- Unloading the parent world despawns entities spawned via `spawn_entity` within that world's scope.

### Asteroid torus shape — unit tests

- The cell-eligibility test admits cells overlapping the annulus and rejects cells fully inside the inner radius or fully outside the outer radius.
- Density seeding for the same `(field_idx, gx, gz)` is unchanged from the existing implementation; the shape only filters which cells are considered.

Prior art: existing asteroid spawner and ring-buffer window tests.

## Out of Scope

- Saving and restoring flag state across game-session save/load (PRD #116 handles persistence; flag serialisation will be folded in when that PRD lands).
- Triggers reactive to objectives (`on_objective_completed`) — the same pattern can be expressed with flag(s) set by a `complete_objective`-adjacent action chain, and no scenario currently authored needs the dedicated condition.
- NPC region entry/exit triggers. Only the player ship surfaces enter/exit events in this PRD.
- Faction relation mutation (`set_faction_enemy` etc.). Not needed by current scenarios.
- A predicate DSL with objective/entity/phase predicates. Reserved for a follow-up if a scenario requires it.
- Multiple ships' flag namespaces, per-player flags, or any client-side flag visibility. Flags are server-side narrative state only.
- A separate `GamePhase::ScenarioComplete` distinct from `GameOver`. Long-form messages in `game_over { message }` remain the only ending payload.
- Console-complexity changes related to the universal `shield_pierce` field. Existing complexity presets are not retuned.
- UI changes to expose flag state or predicate evaluation to players or the host.

## Further Notes

- The forcing function for these additions is the "Before the Fire" scenario, drafted in `docs/scenarios/before_the_fire.md`. That scenario is the subject of a separate content PRD which depends on this one.
- The flag-and-predicate subsystem deliberately covers more than "Before the Fire" strictly needs (the scenario uses one counter and a handful of booleans). The generalisation is intentional: subsequent scenarios are expected to be flag-heavy, and a piecemeal flag system would require revisiting the parser surface every time.
- `on_world_loaded` and `on_timer { after_secs = 0.0 }` both fire on (sub-)world startup. The semantic distinction is documentation-only: `on_world_loaded` reads as "as part of this world becoming live"; the zero-timer reads as "in zero simulation time". Both are guaranteed to fire on the same tick as trigger-state registration; ordering between them within that tick is not specified.
- The Comms console's `comms_jammed` region effect already covers the "ships inside a nebula cannot reach us" case. The scenario's narrative reroute (warnings via Starcorp Command broadcasts when the source ship is jam-blocked) is content authoring, not engine work.
- The `parent:` walker only walks the static loader chain known at world-load. Worlds loaded into the same chain order are reachable; transient sibling sub-worlds loaded into a different branch are not. This is sufficient for the documented scenario shape (one root world, branch sub-worlds, a finale sub-world all loaded from the root).
