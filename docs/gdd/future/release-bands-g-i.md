# Project Phoenix — Release Bands C7–C9 (formerly G–I)

| Field | Value |
|---|---|
| Status | Accepted roadmap direction |
| Scope | War MVP, Patrol/War procedural generation, Sandbox Mode, supporting features, technical investment, polish and accessibility increments |
| Audience | Product, design, engineering, content, accessibility and playtest |

Content bands C7–C9 extend the bridge-first generated-play ladder established by Band C6. Every band is a playable public release combining headline content, additional gameplay systems used by that content, technical investment, a focused polish pass and an accessibility increment. Scheduling a cluster here does not pull every accepted future feature into the roadmap.

Related documents: [Future Modes](./future-modes.md), [Planned but Not Scheduled](./planned-not-scheduled.md), [Scenario Authoring](../systems/scenario-authoring.md), [Duty Teams, Officers and Operations](../mechanics/duty-teams-and-operations.md), [Campaign Continuity](../foundation/campaign-continuity.md), and [Onboarding and Accessibility](../foundation/onboarding-accessibility.md).

## Band C6 boundary — authored Patrol MVP

Band C6 ships the first complete Patrol run as authored content. It establishes the persistent multi-system run shell, discrete warp, normally abundant warp fuel, station databases, mission logging, docking refit/rewards and the mode-profile seam later consumed by procedural generation. It does not ship generated assignments or the shared procedural Director.

The authored run is both a useful release and the quality reference for Band C8. It proves that exploration, evidence, assignments, continuity and return-to-run flow work before a generator begins composing them.

## Band C7 — War MVP

### Content

Ship one authored multi-system War run leading toward a decisive final mission. Intervening systems let the crew recruit allies, acquire supplies and intelligence, answer or ignore crises and weaken the opposition. Those choices project into the final mission through ordinary scenario state rather than a separate battle resolver. The conflict belongs to the pre-Dynasty-war period of the Thin Margin setting; its exact factions and region remain content decisions.

### Gameplay systems

- Pursuit tracks an exact authoritative lead while Intelligence reports an evidence-derived countdown, uncertainty interval and graded interception risk.
- Coarse fronts, fleet dispositions and allied availability create strategic consequences without continuously simulating a sector.
- Allied orders travel through Comms to autonomous actors who may accept, refuse, negotiate or fail.
- Duty Teams, Duty Officers, Operations, authored Away Missions, boarding, surrender and a Medical MVP form one personnel vertical used by the run.
- Assigned personnel alone face discrete fatigue, injury, disappearance or death; unassigned personnel are not simulated.

### Technical investment

Add a persistent run orchestrator above ordinary scenarios, coarse strategic state, scenario-to-run and run-to-scenario projection, reliable system-boundary checkpoints and long-run snapshot recovery. Extend headless and balance tooling to execute multi-mission runs and report strategic inputs, final-mission conditions and terminal outcomes.

### Polish

Add a strategic map with equivalent list presentation, a searchable run journal, clear allied-order outcomes, final-mission preparation summaries and campaign-scale debrief/resume flow.

### Accessibility increment

Strategic state must have map/list parity. Pursuit is reported numerically and verbally. Routes, allied orders and projected consequences have persistent summaries. Optional strategic summarisation and recommendation assistance may explain choices but never commits one without the player.

## Band C8 — Patrol and War procedural generation

### Content

Procedurally generate Patrol assignments and War runs from authored, versioned content libraries. Ship curated benchmark seeds alongside generated play, covering distress, diplomacy, investigation, infrastructure, Away Mission, pursuit and combat structures. Generated content uses the authored Band C6 and C7 runs as quality references rather than replacing them.

### Gameplay systems

- Generated Away Missions reuse the Band C7 Operations and personnel contracts.
- Expanded anomalies and space-terrain templates provide reusable exploration and hazard material.
- Orbital Mode adds a coarse wrapping planetary surface, points of interest, normal planar controls, disabled impulse and an authored maximum-thrust escape hold.
- Generated planetary, pursuit and system events remain bounded scenarios with ordinary entities, evidence and consequences.

### Technical investment

Build one deterministic seeded generator and Director with separate Patrol and War profiles. Authors define grammars, constraints and content libraries; generation binds compatible pieces and validates objectives, evidence paths, personnel slots, terminal states and workload. Add authoring preview, seed records, repetition control, automated generated-run testing and telemetry. Sandbox will later consume the same generator and Director rather than fork them.

### Polish

Add seed/run browsing, objective and evidence summaries, procedural map presentation, repetition feedback, generation diagnostics for authors and legible Director pacing feedback.

### Accessibility increment

Accessibility becomes a generation constraint. A generated assignment is rejected when its required interaction paths, station/rating coverage or workload cannot be satisfied under the supported accessibility contract. Generated content carries warnings and equivalent representations, and automated seed sweeps include accessibility regression cases.

## Band C9 — Sandbox Mode

### Content

Ship a curated starting sector that develops into persistent player-chosen work, faction crises, exploration, trade and conflict. Players choose their assignments; the Director may offer central-command missions, crises and opportunities but does not impose a fixed Patrol chain or War destination. The mode supports ordinary play and GM-led sessions.

### Gameplay systems

- A coarse event-driven living world advances actors, production, consumption, shortages and needs without continuous galaxy simulation.
- Trade uses discrete cargo lots over Band C4's bulk-resource model.
- Standing remains layered by actor, faction and institution.
- Sector control, fleet management and autonomous allied assignments create persistent strategic relationships.
- Docking, repair, replenishment, refit, planetary points of interest and generated assignments form the recurring activity loop.

### Technical investment

Add persistent coarse actor/economy state, scalable sector storage, save migration for long-running worlds, deterministic background advancement, corruption recovery and content-pack extension points. Long-horizon performance and simulation tests cover large histories and repeated world advancement.

### Polish

Add searchable history and ledgers, faction/front visualisation, job discovery, world-change notifications and expanded Director/GM controls for long-running sessions.

### Accessibility increment

Complete richer authorised spectator monitors and accessible transitions between playing and observing. Long-running worlds gain configurable information volume, summaries and recap tools. Finish a whole-product keyboard, touch and assistive-technology audit and maintain an accessibility regression suite across authored, generated and Sandbox play.

## Accessibility integration from T1 onward

Accessibility is one continuing programme delivered in increments rather than a content-themed feature in each band. Technical band T1 starts the programme; each content band then advances it.

| Band | Increment |
|---|---|
| T1 | Shared Accessibility settings tab; private effect-named profile; text scale, contrast and reduced-motion controls; OS preferences as defaults; keyboard/focus semantics and non-colour Hero Bar state; anonymous station/rating eligibility seam; per-function assistance schema. |
| C2 | Compose assistance with station ratings and scenario floors; explain local station suitability; filter human-seeking hosts without revealing settings; validate the simple-scenario/base-hull accessibility guarantee. |
| C3 | Alternative representations for relative motion and docking; keyboard/touch alternatives to gesture-only control; persistent text equivalents for consequential multi-ship coordination. |
| C4 | Timed repair-procedure assistance; simplified network representations; Power presets/recommendations; timing support and table/label equivalents for diagrams. |
| C5 | Accessible uncertainty, signatures and EWar; sensory-intensity and alert-priority controls; alternatives to rapidly changing radar overlays; detailed-station accessibility audit. |
| C6 | Accessible system maps and database search; persistent mission summaries; route comparison, session return and recap support. |
| C7 | Strategic map/list parity, evidence countdown alternatives, route/consequence summaries and strategic explanation assistance. |
| C8 | Accessibility-aware generation, warnings, station/workload validation and seed-sweep regression. |
| C9 | Advanced spectator participation, long-session information controls and whole-product completion audit. |

Private settings never describe or disclose medical needs. Other participants and the authoritative resolver receive only the functional result required for eligibility or assistance. The guarantee remains scoped to at least one usable station/rating on a base hull at full supported player count in a simple scenario; it does not promise solo completion of every complex scenario.

## Scope remaining unscheduled

Human ship-to-ship media and its Discord adapter, persistent mines, cloaking, expanded post-C5 EWar, self-destruct, custom peripherals and external show systems remain accepted but unscheduled. They may enter a later band only with a proving content use, bounded state and explicit release capacity.

## Canonical sources

- [PRD #1094 — Band G: War Mode MVP and Strategic Campaign Foundation](https://github.com/jkeywo/project-phoenix-v2/issues/1094)
- [PRD #1095 — Band H: Patrol and War Procedural Generation](https://github.com/jkeywo/project-phoenix-v2/issues/1095)
- [PRD #1096 — Band I: Sandbox Mode and Persistent Sector](https://github.com/jkeywo/project-phoenix-v2/issues/1096)
- [Phoenix delivery roadmap](../../../pasm/spec/roadmap/phoenix-delivery-roadmap.yaml)
- [Future Modes](./future-modes.md)
- [Planned but Not Scheduled](./planned-not-scheduled.md)
- [Onboarding and Accessibility](../foundation/onboarding-accessibility.md)
