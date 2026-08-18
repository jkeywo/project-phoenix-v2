# Project Phoenix — Scenario Authoring

| Field | Value |
|---|---|
| Document | GDD-SCENARIO-AUTHORING |
| Status | Working draft |
| Owner | Unassigned |
| Last updated | 2026-08-18 |
| Scope | Generic scenario design, world TOML, Rhai choreography, outcomes, and validation |
| Authority | Design and authoring overview. Live world/script schemas, validators, assets, and PASM remain canonical. |

Phoenix scenarios range from direct tests with clear win conditions to operational crises with multiple actors, incomplete knowledge, and incompatible demands. Both are valid. A scenario must support the design pillars, but it does not have to exercise every pillar equally or avoid scripted answers entirely.

Related documents: [Thin Margin Setting](../foundation/thin-margin-setting.md), [Campaign Continuity and Persistence](../foundation/campaign-continuity.md), [World and Environmental Systems](./world-environmental-systems.md), [Difficulty and Balance](../foundation/difficulty-balance-playtesting.md), [Combat Test](../content/scenarios/combat-test.md), and [Falling Skyway](../content/scenarios/falling-skyway.md).

Detailed mechanics: [Sensors and Epistemics](../mechanics/sensors-epistemics.md), [Comms and Commitments](../mechanics/comms-commitments.md), and [External Operations](../mechanics/external-operations.md).

## Scenario contract

A scenario provides a bounded situation for one or more selectable player ships. It defines the initial world, available hulls, objectives and pressures, actors and relationships, relevant physical and informational systems, terminal outcomes, and enough presentation for the crew to understand what is happening.

The world TOML defines stable data: global timing, anchors, placed entities, selectable ships, player spawn, routes, workforces, deadlines, rendering, audio, and script sources. Rhai defines event choreography: trigger registration, dialogue, objectives, spawning, deadlines, state changes, consequences, and game-over calls. Persistent simulation systems continue to own movement, combat, damage, operations, condition, capacity, sensing, and faction behaviour.

## Scenario scales

| Form | Appropriate use | Example |
|---|---|---|
| Test or drill | Teach or validate a subsystem through a clear sequence and objective. | Combat Test |
| Mission | A goal-oriented authored situation with branching tactics and consequences. | Future patrol/rescue missions |
| Operational crisis | Several interacting systems, actors, deadlines, evidence, and trade-offs. | Falling Skyway |
| Campaign episode | A mission that reads prior facts and writes durable consequences. | Falling Skyway’s intended role |

## World TOML specification

```toml
[global]
seed = 1034
title = "world.example.global.title"
description = "world.example.global.description"
sim_tick_hz = 60
ai_tick_hz = 30
ai_snapshot_hz = 10

[anchors]
player_start = [0.0, 0.0, 0.0]
objective = [250.0, 0.0, -100.0]
patrol_north = [200.0, 0.0, 200.0]

[[available_ships]]
template_path = "assets/entities/alliance_destroyer.toml"
label = "world.example.ship.destroyer"

[player_spawn]
anchor = "player_start"

[[entity]]
template_path = "assets/entities/example_station.toml"
name = "world.example.entity.station.name"
display_name = "Example Station"

[entity.transform]
position = [250.0, 0.0, -100.0]

[[deadline]]
id = "relief_due"
label = "world.example.deadline.relief"
due_secs = 300
visible = true

[[route]]
id = "civilian_lane"
on_complete = "hold"

[[route.leg]]
anchor = "patrol_north"
speed = 0.8

[[route.leg]]
anchor = "objective"
speed = 0.5
hold_secs = 10

[[workforce]]
id = "dock_workers"
label = "world.example.workforce.dock_workers"
on_strike = false
disposition = 50

[script]
setup = '''
on_world_loaded("on_arrival");
on_deadline("relief_due", "on_relief_due");

fn on_arrival(ctx) {
    ctx.effects.add_objective(#{ id: "secure_station", text: "world.example.objective.secure", mandatory: true });
    ctx.effects.open_comms(#{ from: "world.example.entity.station.name", node_fn: "station_opening" });
}

fn station_opening(ctx) { #{ message: "world.example.comms.station_opening", responses: [] } }

fn on_relief_due(ctx) {
    if ctx.flags.station_secured != 0 {
        ctx.effects.game_over("world.example.outcome.secured", "victory");
    } else {
        ctx.effects.game_over("world.example.outcome.failed", "defeat");
    }
}
'''
```

Anchors are reusable positions for routes, objectives, AI directives, spawns, and script calls. A scenario should name spatial intentions rather than repeat coordinates. `extra_worlds` may compose additive world layers; unload policy determines whether pending delayed actions cancel or resolve when a layer leaves.

The simulation, AI, and snapshot cadences must divide into whole-number relationships. Player-visible values should use string identifiers where the interface expects them. A scenario catalogue controls what the host offers; being authored under `assets/worlds/` does not automatically mean public-demo availability.

## Rhai authoring model

All current event logic is Rhai. The former declarative `[[trigger]]` and `[[comms]]` front ends are retired and explicitly rejected so a scenario cannot load after silently losing its logic.

At load time the script registers event-to-handler relationships such as world loaded, timer, deadline, flag set/cleared, entity destroyed or attacked, hull threshold, waypoint reached, and group destroyed. At runtime the host calls named handlers with a bounded context. Handlers inspect supported flags, counters, evidence, deadlines, commitments, and dialogue state, then buffer supported effects. Scripts cannot mutate arbitrary Bevy state.

Common effect families include objectives; comms and dialogue; flags and counters; spawn/despawn and group membership; faction relations; infrastructure and civilian orders; external operations; deadlines and delayed calls; evidence, dossiers, and commitments; world-layer load/unload; and explicit victory or defeat. The exact callable signatures live in `src/world/script/authoring.rs` and must be checked rather than inferred from an old example.

## Scenario structure

### Opening

The first minute should establish the ship’s role, immediate situation, and first actionable decision. A concise drill may simply announce the objective and release opposition. A crisis may use comms, deadlines, contacts, and readings to reveal several pressures, but it should still provide a legible first move.

### Development

Pressure should come from changing shared state: actors moving, deadlines advancing, infrastructure failing, weapons causing damage, incomplete scans, promises constraining options, or resource capacity proving insufficient. Scripts should pace and acknowledge those changes, not simulate them twice.

### Resolution

Terminal outcomes must be authoritative and whole-session. Victory and defeat should identify why the scenario ended. More open scenarios may also write campaign facts, record casualties and promises, and distinguish a successful operation from a morally or politically clean one.

### Recovery and failure

Scenarios should identify which mistakes are recoverable, which close an option, and which end the mission. Avoid hidden fail states that continue indefinitely. If the player ship can be destroyed, that is a whole-session defeat unless the scenario explicitly supports another controlled ship.

## Recommended player counts

Each scenario records a possible range of `0–Max Players per selected ship` and a recommended range based on tested workload. The recommendation may vary by hull. Combat Test is currently recommended for 2–4. Falling Skyway remains to be established by playtest.

## Scenario design checklist

- State the player fantasy, operational premise, expected length, possible/recommended crew, and offered hulls.
- Identify the initial known facts, hidden but discoverable facts, and facts that change during play.
- List actors, goals, resources, relationships, movement, and failure modes.
- Map each important pressure to authoritative state and at least one player-facing way to observe it.
- Define objectives, deadlines, triggers, acknowledgement, recovery paths, and terminal outcomes.
- Verify that direct solutions, negotiated solutions, and unexpected physical solutions are allowed or refused by simulation facts rather than author fiat where the scenario is intended to be open.
- Define what campaign facts or debrief measures survive.
- Test with the minimum, recommended, and maximum practical crew and with Backfill operating vacant systems.

## Acceptance criteria

- The world and all entity templates parse, compose, preload, and validate without errors.
- Every anchor, route, named entity, group, function, deadline, faction, objective target, and string reference resolves.
- Every registered handler exists; every dialogue `on_pick` function exists; no retired declarative trigger/comms block remains.
- The scenario has an explicit opening, at least one achievable terminal outcome, and no common path that strands the game without progression.
- Scenario text agrees with authoritative position, condition, capacity, faction, damage, and operation state.
- Recommended player count and expected duration are based on observed play rather than document guesswork, or clearly marked TBD.
- A headless or smoke-level test covers loading and the most important outcome/progression contract in proportion to risk.

## Canonical sources

- `src/world/config.rs`, `src/world/validate.rs`, and `src/world/script/` — live world and Rhai contracts.
- `assets/worlds/combat_test.toml` and `assets/worlds/falling_skyway.toml` — shipped examples.
- `assets/scenarios.toml` and `assets/scenarios.demo.toml` — catalogues.
- `pasm/spec/architecture/world-files.yaml` and `pasm/spec/architecture/scenario-scripting.yaml` — intended architecture and decisions.
