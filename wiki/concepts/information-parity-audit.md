---
title: Information-Parity Audit — Fact to Console Checklist
type: concept
tags: [ai, backfill, parity, consoles, audit]
sources: [pasm/spec/DATA_DRIVEN_FINE_SYSTEM_AI.md, src/entities/ai_flag_hosts.rs, src/ship/helm_ai.rs, src/ai/server.rs, src/ai/core.rs, src/ship/power.rs, src/ship/sensors.rs, src/ship/shields.rs, src/console_ai/core.rs, src/console_ai/server.rs, src/console/captain/server.rs, src/console/comms/server.rs, src/console/repair/server.rs, src/console/navigation/mod.rs, src/console/weapons/mod.rs, src/console/weapons/beam.rs, src/console/weapons/blaster.rs, src/console/weapons/torpedo.rs, src/console/helm/server.rs, src/ship/coordination.rs, src/core/messages.rs, gui/console-state.js, gui/mount-plan.js, gui/console-resolver.js, gui/components/ph-helm-radar.js, gui/components/ph-sensor-panel.js]
updated: 2026-07-31
---

Summary

Issue #880 under PRD #870. Every typed fact a Backfill policy reads to make a
decision must have a human-visible counterpart on the console that owns the
system reading it, so a backfilled seat and a human seat play with the same
information. This page is that checklist, taken over the doctrine and fact
surface as shipped.

The result: **47 authored fact references (46 distinct fact names) across the
19 AI policy hosts. 41 have a rendered console counterpart on the hull that
authors them; 4 are derived policy state where no parity is owed; 2 render on
the battleship only. Three findings filed: #925, #926, #927.** That per-fact
tally is for the four player hulls — on the NPC hulls the seat mounts no
console at all, which is #925 and is structural rather than per-fact.

The three candidates #880 names — threat bearing, safe-range ring, closest
approach — are assessed individually below; one is a real gap (threat
bearing), two are not (safe range is parity-by-construction already; closest
approach is derived policy state, not a world reading).

The audit covers all doctrine that exists today. The courier arc-dance
doctrine (#877) is still open and deferred, so the three hostile-arc facts it
would read are authored by no hull yet; their row is recorded as **pending
#877** rather than blocking this audit — and their console counterpart already
shipped with #874, so that row is satisfied in advance.

## The rule this audit applies

Parity is owed on **world readings** — anything the policy learns about the
ship, the target, or the situation. It is not owed on **derived policy state**:
a fact the host computes by folding a world reading against that system's own
private memory, its authored parameters, or a bounded window. Those are the
AI's *judgement*, and the human analogue is the officer watching the same
readout and reaching the same conclusion. The line matters because two of
#880's three named candidates fall on the derived side.

Where a gap is closed, it is closed the way #874 closed the first one: **one
producer feeding both the AI fact and the console field**, never two
derivations that agree by coincidence. `entity_direct_fire_banks`
(`src/ai/server.rs:544`) is the model — its doc comment states the rule
outright, and both `entity_direct_fire_range` (`src/ai/server.rs:488`) and
`entity_weapon_arc_sectors` (`src/ai/server.rs:522`) are projections of that
one bank list.

## Method

- Hosts: the 19 entries in `AI_HOSTS` (`src/entities/ai_flag_hosts.rs:316`).
- Facts read: every `fact(...)`, `candidate_fact(...)` and `self_fact(...)`
  reference in `assets/entities/**.toml`, including the shared fragment library
  under `assets/entities/fragments/ai/`. A seeded fact no shipped hull authors
  a guard over is marked *unauthored* — it is not a parity risk today, but it
  is listed so the checklist stays complete against §5.2.
- Console counterpart: the field must survive all the way to a rendered
  element. A field present in a `gui/console-state.js` builder but dropped by
  the hull's console HTML is **not** a counterpart — see finding 2 and 3.
- Channel 3: every `CoordinationPayload` an AI receiver consumes is checked for
  a visible sender-side console origin.

## Fact families (§5.2) against consoles

| §5.2 family | Read by | Console counterpart | Verdict |
| --- | --- | --- | --- |
| Scenario flags/counters | Power, Comms ×2 (the three `FlagChain::Plumbed` hosts) | Objectives list on Captain; comms threads | OK — scenario state is surfaced as objectives/messages, never as raw counters, for either actor |
| Entity identity/tags/faction/hostility/threat | Sensors + Tactical selectors | Radar blips with stance/faction colouring; Sensors panel `THREAT` row | OK |
| `power_rating(self)` | all four selectors | Ship picker stat (`gui/components/ph-ship-picker.js:70`) | OK — authored, static, shown at ship choice |
| Objectives (score, directive affinity, anchors) | Navigation, Comms, Tactical selectors | Captain objective list; nav-chart objective markers | OK |
| Contacts and targets | Sensors, Tactical, Helm | Radar blips; tactical/science target markers on every radar; Sensors panel | OK |
| Navigation waypoint | Navigation selector, Helm | Waypoint blip on helm, tactical, sensors and nav radars | OK |
| Motion — range, bearing, closing rate | Helm ×6 | Sensors panel `BRG`/`RNG`; every radar's target marker | OK |
| Motion — closest approach, separation progress | Helm ×6 | none directly | **Derived policy state — no parity owed.** See candidate 3 |
| Ship capability (actuators, boost, availability) | Helm ×6 | Boost/impulse buttons, joystick, station damage bar | OK |
| Shields and damage (own) | Shields focus, Helm, Power, Repair | Shield facing bars, hull hero bar, core-damage panel, station damage bar | OK |
| Shields and damage (target) | Torpedo tube, Helm | `target_shields` / `target_shield_fraction` — **battleship only** | **Finding 3** |
| Weapons — arc, range, readiness, magazine, in-flight | Phaser/Blaster/Tube/Magazine, Helm | Bank + tube readiness widgets, magazine counter, phaser/torpedo arc overlays | OK |
| Threat bearing (Sensors → Shields, channel 3) | Shields focus | transient popup only; no persistent readout outside the battleship | **Finding 2** |
| Enemy weapon arcs | Helm ×6 | Helm-radar wedge overlay at red alert (#874) | OK — parity by construction; *pending #877* for the reader |
| Policy runtime (`state_time`, `memory(...)`, history windows) | Helm engines/steering/boost | n/a | Not world information — the AI's own bookkeeping |

## Authored facts, host by host

Every fact reference authored today, with the console field and the element
that renders it. `dest` = Alliance Destroyer (the demo hull); a field marked
*battleship only* renders on `gui/battleship/*` and is dropped by the composed
consoles the other three hulls use.

### Captain — `[captain_console.ai]`

| Fact | Producer | Console counterpart | Verdict |
| --- | --- | --- | --- |
| `fact(hostile_contact)` | `src/console/captain/server.rs:296` | Sensors radar blips (dest: captain station owns `sensors` + `sensor-radar`) | OK |
| `fact(hostile_range)` | `src/console/captain/server.rs:300` | Sensors panel `RNG`; blip distance | OK |
| `fact(secs_since_combat)` | `src/console/captain/server.rs:282` | none | Derived timer over visible events (fire, damage) — no parity owed |
| `fact(red_alert)` | shared | Red-alert control, `ph-red-alert` | OK |

### Helm ×6 axes — `[helm_console.{engines,steering,lateral,vertical,impulse,boost}_ai]`

| Fact | Producer | Console counterpart | Verdict |
| --- | --- | --- | --- |
| `fact(posture)` | `helm_ai.rs:1420` | Red-alert control state; `red_alert` on the helm payload | OK |
| `fact(target_valid)` | `helm_ai.rs:1444` | Tactical target marker on the helm radar | OK |
| `fact(range_to_target)` | `helm_ai.rs:1437` | Sensors panel `RNG`; target marker | OK |
| `fact(closing_rate)` | `helm_ai.rs:1440` | Target marker motion; Sensors `SPEED`/`HEADING` rows | OK — the range readout it differentiates is visible |
| `fact(speed_fraction)` | `helm_ai.rs:1446` | Helm `speed`, joystick, throttle | OK |
| `fact(shield_fraction)` | `helm_ai.rs:1549` | Shield facing bars (dest: engineering) | OK |
| `fact(boost_available)` | `helm_ai.rs:3040` | Boost button availability + battery | OK |
| `fact(safe_distance_held)` | `helm_ai.rs:1571` | none | Bounded-window verdict over `range_to_target` — derived, no parity owed |
| `fact(separation_progress)` | `helm_ai.rs:1603` | none | Bounded-window verdict over `range_to_target` — derived |
| `fact(range_above_min_seen)` | `helm_ai.rs:1456` | none | Folds `range_to_target` against private memory — **candidate 3**, derived |
| `fact(inside_threat_range)` | `helm_ai.rs:1616` | Enemy arc wedges drawn to each bank's own range | OK — **candidate 2**, parity by construction |
| `fact(tubes_full)` / `fact(tubes_fillable)` | `helm_ai.rs:2133` / `:2091` | Torpedo tube slot widget + magazine counter | OK |
| `fact(torpedoes_in_flight)` | `helm_ai.rs:2041` | Torpedo blips on the tactical radar | OK |
| `fact(target_facing_shield_down)` | `helm_ai.rs:1987` | `target_shields` — *battleship only* | **Finding 3** |
| `fact(hostile_arc_exposure)` / `_escape_deg` / `_inescapable` | `helm_ai.rs:1495`–`:1517` | Helm-radar wedge overlay at red alert (`gui/components/ph-helm-radar.js:207`) | **Pending #877** — counterpart already delivered by #874; no hull authors a reader yet |

### Weapons — phaser bank, blaster bank, torpedo tube, torpedo magazine

| Fact | Producer | Console counterpart | Verdict |
| --- | --- | --- | --- |
| `fact(in_arc)` | `beam.rs:342`, `blaster.rs:76`, `torpedo.rs:106` | Phaser/torpedo arc overlays on the tactical radar | OK |
| `fact(loaded)` | `torpedo.rs:103` | Tube slot state (`torpSlotStates`) | OK |
| `fact(target_facing_shields)` | `torpedo.rs:107` | `target_shields` — *battleship only* | **Finding 3** |
| `fact(red_alert)` | `beam.rs:345`, `blaster.rs:78`, `torpedo.rs:110` | Red-alert control | OK |
| `in_range`, `on_cooldown`, `cooldown_remaining`, `frequency`, `tubes_full`, `magazine`, `in_flight`, `loaded_count`, `target_count`, `ai_target_count`, `operates_ai` | seeded | Arc wedge radius + radar range rings; bank readiness + magazine widgets; AUTO badges | OK — *unauthored* by any shipped hull |

### Shields focus — `[shields_console.ai_policy]`

| Fact | Producer | Console counterpart | Verdict |
| --- | --- | --- | --- |
| `fact(recent_damage_pct_max)` | `src/console_ai/core.rs:553` | Shield facing HP bars — the level whose fall *is* the recent damage | OK |
| `recent_damage_<facing>`, `recent_damage_total`, `recent_damage_fraction_max`, `health_fraction_min_ratio`, `health_ratio_pct` | `console_ai/core.rs:540`–`:573` | same, plus hull hero bar | OK — *unauthored* |
| threat bearing (channel 3, overrides the damage decision) | `src/console_ai/server.rs:190`–`194` | transient popup only | **Finding 2** |

### Power — `[power.ai_policy]`

| Fact | Producer | Console counterpart | Verdict |
| --- | --- | --- | --- |
| `fact(battery_pct)` | `src/ship/power.rs:103` | Battery bar | OK |
| `fact(thrust)` | `power.rs:107` | Helm throttle/joystick; helm power-group level | OK |
| `fact(red_alert)` | `power.rs:108` | Red-alert control | OK |
| `power_<group>`, `total_allocation`, `offline_system_count`, `nearest_enemy_dist`, `has_destroy_objective`, `secs_since_combat` | `power.rs:113`–`:129` | Power group controls, total; core-damage panel; radar blips; objective list | OK — *unauthored* |

### Selectors — Sensors, Tactical, Navigation, Repair, Comms

| Fact | Console counterpart | Verdict |
| --- | --- | --- |
| `candidate_fact(detectable)`, `candidate_fact(hostile)` | Radar blips with stance colouring | OK |
| `candidate_fact(source_radar)`, `source_combat_lock`, `source_sensors_designation`, `source_last_attacker`, `source_objective`, `source_retained` | Each source has its own console surface: radar blip, Combat Lock marker, Sensors designation popup, incoming-fire beam/impact, objective list, current lock | OK |
| `candidate_fact(source_nav_objective)`, `source_chart_contact`, `reachable` | Nav-chart blips + objective markers | OK |
| `candidate_fact(objective_score)` | Captain objective list (priority order) | OK |
| `candidate_fact(tier_ordinal)`, `damage_fraction`, `assigned`, `source_repair_request` | Repair `dispatch_targets` damage %, `core_systems`, team status | OK |
| `candidate_fact(in_range)`, `has_open_hail_thread`, `source_hail_objective` | Comms contacts + thread list | OK |
| `fact(sender_in_range)`, `fact(comms_available)`, `self_fact(comms_available)` | Comms contact list; station damage bar | OK |
| `deficit`, `worst_system_damage_fraction`, `system_count`, `is_core`, `source_core_bucket`, `free_team_count`, `total_hull_health_fraction`, `is_urgent`, `is_read`, `is_orphaned`, `mandatory`, `response_count`, `contact_count`, `has_unread_from_sender`, `source_comms_contact` | Repair and Comms console widgets | OK — *unauthored* |

## Channel 3 — every payload an AI receiver consumes

The check is the one #870 asks for: the fact must fire from **authoritative
system state**, so a human-operated sender still feeds an AI receiver, and the
sender-side reading must be visible on the sender's own console.

| Payload | Emitted from | Sender-side console origin | AI receiver | Verdict |
| --- | --- | --- | --- | --- |
| `TargetDesignation` | `src/ship/sensors.rs:206` | Sensors radar selection | Tactical | OK |
| `ThreatBearing` | `src/ship/sensors.rs:491` — nearest hostile in sensor range, authoritative | Nearest hostile blip on the Sensors radar | Shields focus | OK on emission; **Finding 2** on the receiving console |
| `FrequencyHint` | `src/console_ai/server.rs:1327` | `target_shield_freq` — *battleship only* | Tactical | **Finding 3** |
| `ArcBearingRequest` | `src/console/weapons/mod.rs:684` | Phaser/torpedo arc overlay + target lock | Helm | OK |
| `NavigateTo` | `src/console/navigation/mod.rs:337` | Waypoint marker on the nav chart | Helm | OK |
| `RepairRequest` | `src/ship/damage_sync.rs:161` | Core-damage panel + dispatch targets (#737 projection applies to both actors) | Repair | OK |
| `ShieldFacingDown` / `ShieldFacingRestored` | `src/ship/shields.rs:419` / `:441` | Shield facing bars | Helm | OK |
| `PowerBrownout` | `src/ship/power.rs:638` | Power group levels + battery bar | crew (popup) | OK |
| `IntentAdvisory` | `src/ship/intent_narration.rs` | n/a — this one *is* the human-facing surface (#879) | broadcast to human seats | OK |

Routing itself is symmetric: `route_coordination` (`src/ship/coordination.rs:25`)
and `broadcast_to_ship` (`:100`) branch on the target's control source and the
sender's origin tag only, never on which actor produced the underlying state.

## The three named candidates

### 1. Threat bearing — **gap, filed**

The backfilled Shields focus consumes `PendingShieldsThreatBearing`, the
relative bearing of the nearest hostile in sensor range, and it **overrides**
the damage-based focus decision for that tick
(`src/console_ai/server.rs:190`–`194`). A human Shields officer gets only the
transient coordination popup.

The `ShieldsConsolePayload.target_bearing` field is not this quantity: it is
the bearing to this ship's *own* Combat Lock (`src/ship/shields.rs:552`), and
it renders on one console in the game — `gui/battleship/shields.html:40,80`,
labelled "Threat Bearing". The demo hull puts shields on
`gui/destroyer/engineering.html:87`, which renders facings and focus and no
bearing at all.

So the human seat sees a one-shot popup where the AI seat sees a standing
input, and on three of four player hulls not even the mislabelled proxy.
Filed.

### 2. Safe-range ring — **no gap; already parity by construction**

`safe_range` (`src/ship/helm_ai.rs:1563`) is `target_direct_fire_range` plus
this hull's authored `safe_range_margin`. The two halves separate cleanly:

- **The enemy's reach is already visible.** `entity_direct_fire_banks`
  (`src/ai/server.rs:544`) is one producer feeding both
  `entity_direct_fire_range` → `AiWorldEntity::direct_fire_range`
  (`src/ai/core.rs:220`) → the fact, *and* `entity_weapon_arc_sectors` → 
  `HelmBlackboard::hostile_weapon_arcs` (`src/core/messages.rs:2283`) → the
  helm-radar overlay. The overlay draws each wedge out to that bank's own
  range (`gui/components/ph-helm-radar.js:207`), and
  `target_direct_fire_range` is the maximum over exactly those banks. The
  human helm and the backfilled helm are reading the same geometry from the
  same list — the producer's own doc comment states this as its reason to
  exist.
- **The margin is not world information.** `safe_range_margin` is this hull's
  authored doctrine tuning, in its own TOML. Rendering it would show the
  human the AI's chosen standoff, which is a UX nicety, not parity.

Residual, recorded rather than filed: the overlay is gated on red alert and on
radar range while the fact is not. That is a deliberate #870 decision — "the
red-alert restriction applies to *display*, not to knowledge" — so it is
recorded here as a known, chosen asymmetry rather than reopened.

### 3. Closest approach — **no gap; derived policy state**

`range_above_min_seen` (`src/ship/helm_ai.rs:1456`) is
`range_to_target − memory(min_range_seen)`, where `min_range_seen`
(`src/ship/helm_ai.rs:1623`) is per-fine-system private memory, scoped to the
policy state *and* to the target identity. The doctrine compares it against
its own authored `closest_approach_hysteresis`.

Nothing in it is information about the world that a human lacks: the world
reading is `range_to_target`, which is on the Sensors panel `RNG` row
(`gui/components/ph-sensor-panel.js:128`) and on every radar's target marker.
"We are past closest approach" is the conclusion a human helm officer draws
from watching that number bottom out — the AI's running minimum is the
mechanisation of the judgement, not an extra input. No parity owed, and no
follow-up filed.

## Findings

1. **NPC-hull seats mount no console, so no fact on those hulls has a
   counterpart.** #871 gave the Harrow and Requiem hulls `[[station]]` blocks
   and rating structure, and a human can be admitted to those seats — but none
   of those stations authors a `console` path (`console: Option<String>`,
   `src/ship/config.rs:48`). `resolveConsoleUrl` returns `null`
   (`gui/console-resolver.js:4`) and `planMounts` skips the station
   (`gui/mount-plan.js:60`), so the seat mounts no iframe. Every fact the
   Harrow doctrine reads — including `target_facing_shields`,
   `target_facing_shield_down`, `inside_threat_range` and
   `separation_progress`, all of which only Harrow hulls author — therefore has
   no human counterpart on the hull that reads it. This is the largest parity
   gap in the game and it is structural, not per-fact. Filed as **#925**.
2. **Threat bearing has no persistent console counterpart outside the
   battleship, and the battleship's is a different quantity.** See candidate 1.
   Filed as **#926**.
3. **Target shield state renders on one console family only.**
   `target_shields`, `target_shield_fraction` and `target_shield_freq` are
   built into every Sensors payload (`gui/console-state.js:1264`–`:1266`) but
   rendered only by `gui/battleship/sensors.html:77`. `ph-sensor-panel`, the
   component the destroyer, cruiser and courier use, drops all three
   (`gui/components/ph-sensor-panel.js:133`–`:140`). The backfilled torpedo
   tube reads `fact(target_facing_shields)`, the backfilled helm reads
   `fact(target_facing_shield_down)`, and the `FrequencyHint` channel carries
   the target's shield frequency to Tactical. On three of four player hulls the
   human seat cannot see any of it. Filed as **#927**.

## Deferred

**Courier arc-dance doctrine (#877)** is open. `hostile_arc_exposure`,
`hostile_arc_escape_deg` and `hostile_arc_inescapable`
(`src/ship/helm_ai.rs:1495`–`:1517`) are seeded on all seven helm hosts but
authored by no hull, so no policy reads them yet. Their console counterpart —
the red-alert helm-radar wedge overlay — already shipped with #874, so when
#877 lands the parity row is satisfied on arrival. Re-check that row when the
courier doctrine is authored; no other row in this checklist depends on #877.

**Courier frozen-battery exception (issue #923, `alliance_courier.toml`).** Once a sustained red alert drains the courier's battery to the point sensors sheds to level 1, its `rates = [3, 2, 1, 0, -1, -3]` put a flat 0 at the resulting resting total, so the battery never recovers and no console on this hull shows the sensors power level to tell a crew reach is stuck at two thirds — accepted rather than filed; revisit with #877/#922.
