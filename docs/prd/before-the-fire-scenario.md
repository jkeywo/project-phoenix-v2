# PRD: "Before the Fire" Scenario

## Problem Statement

The simulator currently ships two world files: a flavoured starbase patrol (`default.toml`) and a minimal raider scrap (`patrol.toml`). Neither exercises the full console roster, neither has narrative branching, neither has a finale beyond "kill the raider". Crews finish them in minutes and the experience does not justify the consoles' depth. The codebase has no example of a long-form, branching, lore-grounded scenario that other authors can read as a template.

There is also no reference implementation of how the existing systems compose under load: per-console hull damage from sustained combat, region effects layered onto a navigated approach, multi-stage objective flow, NPC AI with varied behaviour profiles, and an ending that is not "your ship blew up".

## Solution

Implement the "Before the Fire" scenario drafted in `docs/scenarios/before_the_fire.md` as a playable, default-quality world. The scenario is a 30–60 minute political-thriller-to-crisis-response narrative aboard the A.E.V. Ardent set in the Axiom System 35 years before the era the world bible otherwise depicts. The crew investigates collapsing peace talks, can follow three non-exclusive narrative branches (Diplomat, Scholar, Soldier), and faces a Kobayashi Maru finale with four simultaneously-available responses, none of which prevent the war but each with distinct consequences.

The content is shipped as a primary world TOML plus four sub-world TOMLs (one per narrative branch, one for the finale), the supporting entity TOMLs (already largely present in the repo), and any necessary faction tuning. The scenario depends on the engine additions specified in the companion engine PRD; nothing in this PRD requires Rust code beyond those additions, with two narrow exceptions called out under Implementation Decisions.

When this PRD ships, "Before the Fire" appears in the host's scenario selector alongside the existing two worlds, and a 2–6 player crew can complete it without operator intervention.

## User Stories

1. As a host, I want "Before the Fire" to appear in the scenario picker on `server.html`, so that I can launch it like any other world.
2. As a crew on the bridge, I want the opening of the scenario to deliver two simultaneous comms — Starcorp Command deployment orders and an Axiom Station distress hail — so that we are oriented to the situation within the first minute.
3. As a crew, I want a clear named star-system layout (Axiom Station, Research Outpost, Kaleth Prime, ship-start, Ironveil patrol route, Ashrender position, Kaleth Nebula) with rounded but recognisable coordinates, so that Helm and Science can plan navigation.
4. As a crew, I want an asteroid belt populated around the system origin, so that the inner system has navigational character and Helm has to manoeuvre rather than fly straight lines.
5. As a captain, I want hailing Axiom Station to load the Diplomat branch, so that I can pursue the political thread.
6. As a captain, I want hailing Research Outpost to load the Scholar branch, so that I can pursue the technical thread.
7. As a captain, I want engaging the Ironveil patrol to load the Soldier branch on attack or destruction, so that combat itself opens narrative content.
8. As a crew, I want all three branches to be independently triggerable in any order or combination within the same playthrough, so that the choice of who to investigate first does not lock out other content.
9. As a comms operator, I want each branch's dialogue to be authored as a coherent multi-step exchange with a named NPC (Administrator Chen on Axiom, Dr. Myst on the Outpost, intelligence summary from Ironveil's wreckage), so that the narrative beats land rather than being one-line dumps.
10. As a science officer, I want the Scholar branch to reveal the technical picture (stellar resonance class weapon, 20-minute charge, quantum interference zone outcome) and the first hint of the Singularity signal, so that the technical role has genuine intelligence to gather.
11. As a tactical officer, I want Ironveil to be a serious-but-defeatable opponent (patrols between two anchors, attacks the player on contact, flees below 30% hull, warps out below 15%), so that combat has stakes and characters.
12. As a crew, I want Ashrender to remain idle and ignore us until the Aphelion Protocol arms, then attack aggressively and never flee, so that the antagonist's mission-focused behaviour is felt mechanically.
13. As a crew, I want the Aphelion Protocol to arm when either Ironveil is destroyed or 600 seconds have elapsed (whichever comes first) and only arm once, so that aggressive and cautious crews both reach the finale with different time/intelligence trade-offs.
14. As a crew, I want the Aphelion finale to load a sub-scenario that delivers three simultaneous comms (Ashrender's warning, Starcorp's urgent order, Requiem House's encrypted offer), so that the four-response choice is presented within one minute of the timer firing.
15. As a crew, I want all four finale responses available simultaneously and attemptable in combination — Fight, Evacuate, Containment, Requiem Override — so that the Kobayashi Maru framing is honest rather than railroaded.
16. As a Helm pilot, I want the Fight option (navigate the nebula and destroy Ashrender) to require traversing the nebula's `damage_zone`, `sensor_blind`, `radar_dampening`, and `comms_jammed` effects, so that the approach genuinely costs hull and intelligence.
17. As an engineering operator, I want the Fight option to produce a steady stream of breakdown work from nebula hazard and Ashrender's gamma-frequency phaser fire, so that there is meaningful work for the repair teams under load.
18. As a comms operator, I want the Evacuate option to be a multi-round dialogue with Axiom Station coordinating evacuation rounds, where each completed round increments a casualty-mitigation counter that the ending message reflects, so that comms-only play has agency and the ending acknowledges effort.
19. As a science officer, I want the Evacuate option to require plotting escape vectors as a complementary objective, so that the science role has substantive work even when the ship is not in combat.
20. As a captain, I want the Containment option to start a 30-second window during which the radiation discharge applies a heavy damage zone to the ship with a configurable shield-pierce fraction, so that even a maxed-out fore shield does not trivially absorb the hit.
21. As a helm pilot, I want the Containment success criterion to be "Ardent survives the 30 seconds while in the discharge zone", so that the option is a survival sprint with a clear binary outcome.
22. As an engineering operator, I want the Containment option to generate a heavy repair load throughout the 30 seconds, so that the repair-team role is mechanically central to the outcome.
23. As a tactical officer, I want shield quadrant strength to matter during Containment (orienting the strongest shield toward Ashrender), so that the existing shield-quadrant mechanic carries narrative weight.
24. As a comms operator, I want the Requiem Override option to be a single accepted-channel exchange that disables the weapon, after which the Requiem courier is scripted-destroyed by Harrow, so that the moral cost of the "easy" option is paid on-screen.
25. As a crew, I want each of the four endings to fire a `game_over` with a distinct long-form epilogue describing the war's casus belli, the casualty figures (modulated by evacuation counter where applicable), the immediate political fallout, and the 35-year-later consequence (the Neutral Zone), so that the choice is acknowledged narratively.
26. As a crew, I want the scenario to be completable by a 3-player crew (Captain, Helm, Tactical) using the default station bundling for that player count, so that small-table sessions are not gated on having a Comms operator.
27. As a crew, I want NPC behaviour transitions (Ironveil flee/warp, Ashrender activation, Requiem courier flee on attack) to be transparent through standard comms and visual cues, so that the scenario does not depend on hidden state changes.
28. As a host, I want hailing or attempting the Requiem channel to be available from scenario start (no prior contact required), so that solo-comms-operator crews are not gated by exploration.
29. As a Helm pilot, I want Ashrender's nebula-edge position to require nebula approach, so that the Fight option has a non-trivial geographic cost relative to the Evacuate and Override options.
30. As a science officer, I want the Soldier branch to deliver "decrypted Harrow operational orders" as a narrative comm immediately on Ironveil's destruction (sourced as an internal report), so that the intelligence payoff is unambiguous without requiring a new scan mechanic.
31. As a scenario author reading this scenario, I want every authored TOML to use only patterns documented in the existing wiki and the engine PRD, so that this scenario doubles as a template for future content.
32. As a crew, I want a moment in the scenario flow where most exploration is done and the Aphelion Protocol has not yet armed, so that future save/load (PRD #116) has a natural checkpoint moment to capture.

## Implementation Decisions

### Authored Content

- **Worlds:**
  - `assets/worlds/before_the_fire.toml` — root scenario. Defines anchors for system layout, spawns Axiom Station, Research Outpost, Ironveil (with patrol behaviour), Ashrender (idle initial state), the Requiem courier (idle), the Kaleth Nebula region, and the asteroid belt (torus shape, inner ~90, outer ~260). Holds the opening twin-comms triggers, the branch-load triggers (hail Axiom → load path A, hail Outpost → load path B, attack/destroy Ironveil → load path C), the Aphelion arming logic (on_destroyed Ironveil OR on_timer 600s, both set `aphelion_armed`; an `on_flag_set aphelion_armed` trigger loads the finale sub-world), and the Ashrender AI swap (`on_flag_set aphelion_armed` sets Ashrender's AI state to attacking).
  - `assets/worlds/btf_path_a.toml` — Diplomat branch. Administrator Chen dialogue tree about the missing Dr. Sol Varen. A trigger conditional on `parent:ironveil_destroyed` (set by the root world on Ironveil destruction) spawns Dr. Varen's escape pod via `spawn_entity` and delivers the confirming intelligence comm.
  - `assets/worlds/btf_path_b.toml` — Scholar branch. Dr. Myst dialogue tree about anomalous resonance, weapon class, and the Singularity sub-harmonic hint.
  - `assets/worlds/btf_path_c.toml` — Soldier branch. Triggered by Ironveil attack/destruction. On destruction, delivers the decrypted Harrow operational orders comm (synthetic internal sender — see note below) and sets `parent:ironveil_destroyed` for path-A pickup.
  - `assets/worlds/btf_aphelion_protocol.toml` — Finale. On load, delivers the three simultaneous comms (Ashrender warning, Starcorp urgent, Requiem encrypted offer) via `on_world_loaded`. Spawns the radiation_zone region centred on Ashrender. Holds the four response dialogues and their resolution logic: Fight (track Ashrender hull → 0 → partial-detonation ending), Evacuate (multi-round dialogue + casualty counter → reduced-casualties ending), Containment (30s timer with shield-pierce damage zone + survival check → station-saved ending), Requiem Override (accept channel → weapon neutralised + `destroy_entity` on the courier → traitor-war ending). Each ending fires `game_over` with a long-form message.

- **Entities:** Audit and tune the existing TOMLs already present in the repo:
  - `assets/entities/station_axiom.toml` — verify hull/comms; ensure it is hailable with a `[comms]` block and faction-neutral (no `faction` field, as today).
  - `assets/entities/station_research_outpost.toml` — same audit.
  - `assets/entities/ship_harrow_patrol.toml` (Ironveil) — verify behaviour stanza covers patrol → attack on enemy-in-range → flee at hull_below 0.3 → warp_out at hull_below 0.15 (or via flee timer); confirm faction is `harrow`; tune phaser frequency to gamma to match doc.
  - `assets/entities/ship_harrow_warhawk.toml` (Ashrender) — initial state `idle`, no flee transitions, heavier hull and shielding than Ironveil. Confirm faction `harrow`.
  - `assets/entities/ship_requiem_courier.toml` — initial state `idle`, transition on_attacked → fleeing → warp_out. Faction `requiem` with empty enemies list (no relation mutations).
  - `assets/entities/region_kaleth_nebula.toml` — verify effects: `damage_zone` (3 DPS), `radar_dampening` (0.4× multiplier), `sensor_blind`, `comms_jammed`.
  - `assets/entities/region_radiation_zone.toml` — heavy `damage_zone` (~8 DPS) with `shield_pierce ≈ 0.3` — this is the load-bearing use of the engine PRD's new shield-pierce property.

- **Factions:** Reuse `federation` faction as the Alliance (no rename; the narrative names them "Alliance" via comms text). `harrow` faction has `federation` (Alliance) in its enemies list. `requiem` faction has empty enemies; the courier dies via scripted `destroy_entity`, not via faction hostility.

- **Coordinates:** Round the doc's coordinates to clean numbers, used as named anchors:
  - `ship_start = [0, 0, 0]`
  - `axiom_station = [350, 0, 50]` (or rounded variant)
  - `research_outpost = [-100, 0, 450]`
  - `requiem_courier = [-230, 0, -80]`
  - `ironveil_patrol_a = [180, 0, -160]`, `ironveil_patrol_b = [320, 0, -280]`
  - `ashrender_start = [620, 0, -380]`
  - `nebula_center = [680, 0, -440]` (radius ~220)
  - Asteroid belt anchored at origin, torus inner ~90, outer ~260.
- Final coordinates may shift during playtest tuning; the listed values are starting points.

### Host-side change

- **`server.html`** — add a button for `assets/worlds/before_the_fire.toml` to the existing scenario picker. No JS logic changes.

### Narrow exceptions to "no Rust beyond the engine PRD"

Two small implementation details are not covered by the engine PRD and need a decision during implementation. Neither is a blocker; both have a low-cost path:

- **Synthetic internal comm sender.** Path C delivers the decrypted Harrow orders as a comm "from Science" (i.e. from the player's own ship). The `[[comms]]` schema today requires `from = <entity_name>` resolved to a UUID via the spawn name table. Two options at implementation time:
  1. Reserve a sender name (e.g. `_self` or `_ship_internal`) that the comms system recognises and renders as an internal report. Trivial wiring change.
  2. Model the Ardent as a hailable entity with an internal `[comms]` block. Larger change; reusable for future scenarios that want ship-internal dialogue.
  This PRD recommends option 1 for v1, with option 2 deferred until a second scenario needs ship-internal subsystems addressable individually.

- **Ending text length.** The four finale endings each have a long-form `game_over { message }`. If the current `GameOver` broadcast or client rendering truncates long messages or mangles newlines, render handling will need a small adjustment. To be verified during implementation; flag as a follow-up if observed.

### Console feature coverage

The scenario authoring deliberately exercises:
- CaptainChair: Red Alert (Aphelion-armed beat), View Selector (survey checkpoints), StartGame (existing).
- Helm: thrust/steering throughout; nebula approach; Impulse Drive sprint for late intercept attempts; Containment positioning.
- Science: long-range radar (Path B detection of Ashrender from outside the nebula); impulse cancel (abort entry at wrong angle).
- Tactical: phasers (Ironveil); torpedoes (Ashrender's heavier hull); shield frequency tuning (Harrow gamma frequency).
- Engineering: repair load during Ironveil combat, nebula traversal, and Containment; power allocation (player choice during Containment).
- Comms: every NPC dialogue, the multi-round Evacuate path, the Requiem encrypted channel.

No new console mechanics are introduced by this PRD.

### Save/load and complexity

Out of scope (see Out of Scope). The "natural save point" called out in the scenario doc is informational: once PRD #116 ships, periodic saves will catch the pre-Aphelion exploration state automatically without scenario-side authoring.

## Testing Decisions

Good tests set up state, perform an action, and assert on observable output through the public interface. They do not assert on private fields, internal call counts, or implementation details.

### Smoke test (`tests/smoke/`)

Add a Playwright smoke test that boots `before_the_fire.toml`, drives a scripted crew, and asserts on the comm transcript and final `GameOver` message. Coverage targets:

- World loads cleanly: no parse errors, expected entities spawn at expected anchors, opening twin comms arrive within the first simulation second.
- Branch loading: hailing Axiom Station emits the path A introduction comm; hailing Research Outpost emits the path B introduction comm; attacking Ironveil emits the path C entry comm. All three can be triggered in the same session.
- Aphelion arming OR semantics: in one playthrough, destroy Ironveil quickly and observe the finale loads exactly once; in a second playthrough, wait 600 seconds and observe the finale loads exactly once; in a third, do both and observe still exactly one load.
- Finale endings: drive each of the four response paths to completion in independent runs and assert the `GameOver` message contains the expected ending tag (e.g. "Harrow casus belli", "Containment held", "Requiem denounced", "evacuation casualty count").
- Containment shield-pierce coupling: with the radiation_zone's `shield_pierce = 0.3`, drive a containment attempt with maxed fore shield and assert hull damage is non-zero (verifying the engine PRD's pierce mechanic is wired into this region).

Prior art: existing `tests/smoke/` Playwright tests for world bootstrap, AI patrol, engineering, comms.

### TOML linting

If a scenario-TOML test fixture exists (or is introduced lightly here), assert that every world file under `assets/worlds/before_the_fire*.toml` parses without error and resolves all `template_path`, anchor names, and entity name references that triggers and comms refer to. This is cheap insurance against author typos.

### Manual playtest checklist

- 3-player crew (Captain/Helm/Tactical) can complete each of the four endings within 30–60 minutes.
- Each branch sub-scenario contributes distinguishable narrative content visible in the comms log.
- Ashrender does not engage the player before Aphelion arms, and does engage after.
- The asteroid belt is navigable and visible from the helm radar.

## Out of Scope

- Engine work beyond what the companion engine PRD covers, with the two narrow exceptions documented above (synthetic internal sender; possible GameOver text rendering).
- Save/load support for this scenario (covered by PRD #116; this scenario does not author save points).
- Console-complexity tuning. The scenario uses default complexity presets. The doc's references to Low Tactical auto-firing torpedoes or Low Engineering lacking battery for containment are aspirational documentation; this PRD ships without re-tuning complexity presets.
- Localisation of comms text. English-only.
- Voiceover, sound design, or visual polish beyond what the existing renderer provides for stations, ships, and regions.
- A scenario-end UI distinct from `GamePhase::GameOver`. Endings are long-form `game_over { message }` strings.
- Cinematic camera, cutscenes, or scripted CaptainChair view-selector overrides.
- Faction relation mutation during play (Requiem stays neutral; the courier dies via `destroy_entity`).
- Difficulty knobs or replay variation. The scenario is the same every time apart from player choices.
- Additional ship templates beyond those listed; if the existing Harrow templates need tuning, that tuning is scope, but new ship classes are not.

## Further Notes

- This PRD depends on the engine additions PRD landing first. Splitting the work this way means scenario authoring is gated behind a small, well-tested engine surface that other future scenarios can also use.
- The scenario doc (`docs/scenarios/before_the_fire.md`) is the canonical source of narrative content. This PRD captures the mechanical and structural decisions; the doc captures the prose, lore, and tone. Authoring proceeds by translating the doc into TOML, not by re-inventing content here.
- "Before the Fire" is set 35 years before the era the rest of the world bible depicts. It is intended to be the first of multiple historical scenarios. Authoring it well — with full feature coverage and a reusable structure — is partly an investment in subsequent scenarios in this and the contemporary era.
- The scenario also functions as a torture-test for the engine: every console gets meaningful work, region effects layer, AI behaviours vary, sub-worlds load and unload, the flag system carries cross-scenario state, and the finale exercises the new shield-pierce property in a high-pressure scenario. Bugs in any of these systems will surface here first.
- Audit-then-tune is preferred over rewrite for the existing entity TOMLs (Axiom, Research Outpost, Ironveil, Ashrender, courier, nebula, radiation zone). Several already exist in the repo from earlier drafts; reuse what is correct, change only what conflicts with this PRD.
