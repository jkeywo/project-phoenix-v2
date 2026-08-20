# Project Phoenix — Station Experience Suite

| Field | Value |
|---|---|
| Document | GDD-STATION-EXPERIENCES |
| Status | Working draft; combines implemented station behaviour with clearly marked accepted direction |
| Owner | Unassigned |
| Last updated | 2026-08-19 |
| Scope | Player experience for Captain, Helm, Tactical, Sensors, Navigation, Comms, Shields, Power, and Repair |
| Authority | Player-facing design overview. Ship TOML, console code, PASM, and runtime validation remain canonical. |

This document defines what each console family should feel like to operate, what information and authority it owns, and what conversation it should create. A ship may bundle several families into one station or expose each as a separate seat. A human-seeking station preserves its complete authored surface when it is hosted as a tab elsewhere rather than decomposing back into loose family panels.

Related documents: [Game Design Overview](../foundation/overview.md), [Ships and Ship Systems](./ships-and-systems.md), [Alliance Ships](../content/ships/alliance-ships.md), [AI and Backfill](./ai-and-backfill.md), [Onboarding, Tutorials, and Accessibility](../foundation/onboarding-accessibility.md), and [World and Environmental Systems](./world-environmental-systems.md).

Detailed mechanics: [Movement and Helm](../mechanics/movement.md), [Targeting and Weapons](../mechanics/targeting-weapons.md), [Damage, Diagnosis and Repair](../mechanics/damage-repair.md), [Power and Resource Network](../mechanics/power-resource-network.md), [Shields](../mechanics/shields.md), [Sensors and Epistemics](../mechanics/sensors-epistemics.md), [Navigation and Relative Motion](../mechanics/navigation-relative-motion.md), [Comms and Commitments](../mechanics/comms-commitments.md), and [External Operations](../mechanics/external-operations.md).

## Shared station principles

- Every station owns consequential decisions, not just status monitoring.
- No station should have all the information and authority needed to solve the mission alone.
- A console should show the facts its operator needs, the consequence of its last action, and the reason an action is unavailable or refused.
- A time-critical action belongs on the summary surface. Planning, diagnosis, configuration, and deeper control may live on detail surfaces.
- Automation operates the same fine systems through the same admitted command paths as a human. Taking control should not switch to a second simulation model.
- System damage, power, interference, range, and scenario state should alter what a console can truthfully show or do rather than merely recolour it.
- Shared information should come from one authoritative producer. Human-visible facts and AI facts must not be separately derived versions that happen to agree.

Every console uses one shared Hero Bar. It pins the player's directly held station first and lists visiting stations in hull-authored order. The selected station contributes its name, rating and authoritative capacity-weighted health; every tab keeps a separate health indicator plus an off-screen importance indicator. Alerts do not replace health or reorder tabs. One-off events clear when seen, while continuing critical conditions remain marked until resolved. Switching tabs preserves local UI context.

## Current and accepted control-depth model

The current runtime provides authored station ratings whose `automated_systems` list determines which fine systems are human- or AI-operated. A station holder can change rating during play and the host applies the resulting control-source changes immediately.

The accepted future direction extends this into an authored star ladder. Star 0 is complete Backfill; higher authored rungs combine AI, simplified, and detailed control per system, normally reaching three to five stars. A scenario may impose a detailed-control floor on particular system families. Simplified control is limited AI directed from a summary surface; detailed control exposes the full model. This ladder, scenario floor, and generalised detail-screen host are design direction, not a claim about the current shipped console set.

## Station-family overview

| Family | Core question | Primary outputs | Natural dependencies |
|---|---|---|---|
| Captain | What matters now, and what posture should the crew take? | Priorities, Red Alert, viewscreen requests, crew direction | Every station |
| Helm | Where should the ship be, facing which way, at what speed? | Thrust, steering, impulse, boost, lateral/vertical motion | Tactical, Navigation, Shields, Sensors |
| Tactical | Which target should be engaged, with which weapon and timing? | Target lock, energy fire, torpedo loading/launch, arc request | Sensors, Helm, Captain, Power |
| Sensors | What is present, what is it doing, and what can be established about it? | Designations, scans, threat bearing, frequency hints, evidence | Tactical, Shields, Captain, Comms |
| Navigation | Where is the operational destination and how should the ship approach it? | Waypoints, chart interpretation, route recommendation | Helm, Captain, Sensors, scenario objectives |
| Comms | Who can be reached, what are they asking, and what will the crew commit to? | Hails, responses, promises, acknowledgements | Captain, Sensors, Navigation, Operations |
| Shields | Which facing needs protection and when should focus change? | Focused arc and defensive posture | Sensors, Helm, Power, Repair |
| Power | Which systems receive scarce temporary capacity and how much reserve is acceptable? | Group allocations and recovery posture | Helm, Tactical, Shields, Repair |
| Repair | Where should limited teams go, and what should they fix first? | Dispatch and on-site repair priority | All damaged station owners, Power, Captain |
| Command | What posture should each AI-controlled station adopt? | Station stance and objective-specific direction | Captain, objectives, every AI-controlled station |
| Operations | Which external or away mission should launch, with which people and transport? | Mission plan, assignments, support and extraction | Duty Teams, relevant stations, Medical, Sensors, Comms |
| Medical | Who needs treatment and which biological risks affect the crew? | Triage, treatment priority, recovery and medical readiness | Science, Operations, Duty Officers, scenario hazards |

## Captain

### Experience

Captain owns attention rather than every control. The console should make the current mission, deadlines, contacts, ship posture, and crew state legible enough to choose a focus and ask useful questions. The player should feel responsible for coordinating expertise, deciding when risk is justified, and making or approving commitments.

### Information and actions

Captain sees objectives and priorities, visible deadlines, relevant sensor/contact summaries, Red Alert state, viewscreen state, and crew/station status appropriate to the hull. Captain may raise or stand down Red Alert, adjust mission-objective priority where supported, and request permitted viewscreen cameras or shared surfaces. Only the Captain chair may command Red Alert; readiness and scenario start remain collective lobby actions.

### Coordination

Captain asks Sensors what is known, Navigation and Helm what is possible, Tactical what force can achieve, Engineering what the ship can sustain, and Comms what has been promised. Good Captain play turns specialist reports into an explicit shared priority. The console should help track decisions without becoming an omniscient substitute for asking officers.

### Failure and automation

An overloaded Captain becomes a bottleneck; an under-informed Captain issues arbitrary orders. Backfill Captain may set posture from authored hostile-contact and combat facts, but it cannot supply human judgement about political commitments or competing mission values. Accessibility requires every camera and alert action to have a labelled tap target and persistent state, not depend on remembering a transient effect.

## Helm

### Experience

Helm directly feels the mass and capability of the selected hull. Its play is continuous but purposeful: hold a course, manage speed, bring weapons to bear, protect vulnerable shield arcs, avoid hazards, and place the ship within range for scans or external operations.

### Information and actions

Helm sees own heading, speed, thrust state, drive availability, waypoint, relevant contacts, target marker, obstacles, hostile weapon arcs when exposed by the current presentation rules, and temporary arc-bearing requests from Tactical. Actions include longitudinal thrust, steering, lateral and vertical thrust where fitted, boost, and impulse charge/cancel.

### Coordination

Tactical may request a firing orientation but does not take the helm. Navigation provides a destination, not direct actuator authority. Shields may report a failing facing; Operations may require stable range and position; Captain decides risk posture. Helm retains the movement decision while explaining when geometry makes another request unsafe or impossible.

### Failure and automation

Damaged actuators disappear or degrade according to authoritative availability. Collision and region hazards change the real movement problem. Backfill evaluates the same world readings and authored doctrine; manual control must preserve player agency rather than silently steering around the player’s command. Drag controls require discrete alternatives for thrust and yaw, with clear neutral/held state and no reliance on fine motor precision alone.

## Tactical

### Experience

Tactical turns contact knowledge and ship geometry into controlled force. The station should reward target discipline, timing, weapon-role understanding, and communication rather than maximum-rate button pressing.

### Information and actions

Tactical sees selectable contacts within the authored tactical radar horizon, current combat lock, weapon arcs and ranges, cooldown/readiness, phaser or blaster bank state, torpedo magazine and tube state, target shields/frequency when established, and Red Alert/fire-gate state. Actions include selecting a target, firing or setting weapon automation, charging/releasing volleys where authored, setting torpedo load intent, launching torpedoes, and requesting a bearing from Helm.

### Coordination

Sensors improves target choice and supplies designation or frequency information. Helm creates arcs and range. Power changes weapon intensity where the hull authors that relationship but not reach. Captain authorises alert posture. Tactical should call intended target, weapon opportunity, ammunition pressure, and required heading aloud.

### Failure and automation

An unavailable radar prevents new locks; destroyed lock-bearing radar clears its standing lock. A weapon outside arc, range, readiness, alert posture, or ammunition must say why it cannot act. Backfill ranks targets and operates each bank/tube through authored policies. Every hold, drag, or repeated-fire interaction needs a labelled tap/toggle alternative and status that is readable without colour.

## Sensors

### Experience

Sensors converts uncertain contacts into operational knowledge. The player should discover facts other stations can act on, not merely watch a better radar. In direct combat this means identity, motion, threat, shield, and frequency information; in operational scenarios it includes structural condition, capacity, environmental readings, dossiers, and evidence.

### Information and actions

Sensors sees a long-range authored radar picture, contact identity and motion, scan state, target analysis, and the quality or limits of current observations. It selects or designates contacts, initiates scans where supported, and communicates threat bearing and frequency hints through the shared coordination system. The accepted future field design adds detected/rough/fair/precise confidence tiers and a wide-versus-focused scan choice; those tiers are not yet a claim about the current interface.

### Coordination

Sensors reports what is known, how confidently, and what remains unknown. Tactical uses designations and target facts; Shields uses threat bearing; Captain uses evidence and threat interpretation; Comms uses identity and provenance. The most valuable Sensors output should often be a sentence another officer needs.

### Failure and automation

Range, radar damage, sensor blindness, fog, and interference limit observations. The console must distinguish “not detected,” “detected but uncertain,” and “known false/absent” where the model supports it. Backfill uses visible-world counterparts for its target and hint decisions. Critical categories need text/icon labels and patterns, not colour alone.

## Navigation

### Experience

Navigation translates scenario intent into spatial intent. It answers where the ship must go, which route or approach matters, and what traffic or operational geometry will complicate arrival.

### Information and actions

Navigation sees chart contacts, anchors or objective destinations, routes, waypoint state, and civilian movement relevant to the scenario. It selects or recommends a waypoint through the authoritative navigation system. Future orbital and relative-motion content may deepen this surface, but routine astronomical simulation is outside scope.

### Coordination

Captain supplies priority; Sensors supplies hazard/contact interpretation; Navigation supplies a destination; Helm retains course and actuator control. In traffic or rescue scenarios Navigation should help coordinate holds, diverts, corridors, shelter routes, and approach timing rather than duplicate the Helm radar.

### Failure and automation

Unreachable or invalid destinations must be refused visibly. Navigation belongs to a human-seeking station surface: the complete authored Navigation UI seeks only its finite compatible fallback list before reaching AI. On a fallback host it appears as a peer Hero Bar tab, keeps its own visiting rating and never changes the host station’s rating.

## Comms

### Experience

Comms turns other actors into a live part of the scenario. It is about attention, interpretation, and commitment: who can be reached, which messages are urgent, what a response means, and whether the crew can still fulfil what it says.

### Information and actions

Comms sees the hail roster, range/jamming availability, message threads, unread and urgent state, active dialogue choices, objective context, dossiers, and commitment-relevant text. It may hail an eligible contact, open or clear a thread, choose a response, and request the shared viewscreen where permitted. Important authored responses require an explicit confirmation step.

### Coordination

Captain should approve consequential promises and political posture; Navigation and Operations judge whether requested movement or work is feasible; Sensors supplies identity/evidence; Tactical reports coercive capability and risk. Comms should repeat important terms aloud and make promises visible enough that the bridge can plan around them.

### Failure and automation

Range, jamming, stale dialogue state, and invalid responses are authoritative refusals with visible feedback. Physical contacts may become unavailable while old messages remain readable. Comms is a complete human-seeking station, normally using its authored Simplified visiting rating unless a scenario requires more. Planned human ship calls preserve an explicit typed-text alternative and speaker identity without recording voice/video content.

## Shields

### Experience

Shields is directional damage management. The operator watches facings, anticipates attack bearing, and decides when concentrating defence on one side justifies exposing the others.

### Information and actions

Shields sees every authored arc, current/max HP, regeneration/offline state, current focus, target/combat-lock bearing where relevant, and authoritative threat bearing. The primary action is selecting or clearing arc focus. Focus bonuses and penalties are hull-authored, so the same decision can feel different across ships.

### Coordination

Sensors supplies threat bearing; Helm can rotate or evade; Power can improve regeneration; Repair responds when damage leaks through. Shields should report a failing or restored facing and request a turn before the problem becomes invisible hull damage.

### Failure and automation

An offline or destroyed shield capability cannot accept focus commands. Backfill tracks real incoming damage over an authored window and may use threat bearing, but its current retained arc-ranking kernel remains partly host-side transitional architecture. Bars require numeric/text state and directional labels in addition to colour.

## Power

### Experience

Power manages temporary advantage and recovery. The operator decides which group deserves capacity now, how deeply to draw the battery, and when to shed load before the exhaustion lock forces the whole ship low.

### Information and actions

Power sees every authored group and allowed level, current allocations, sustainable and commanded totals, battery reserve and direction, reactor/battery availability, draining/charging state, and exhaustion lock. It sets absolute group levels. Current Alliance groups are Helm, Weapons, and Shields; future supply/demand and heat design is roadmap direction, not current behaviour.

### Coordination

Helm requests manoeuvre capacity, Tactical requests weapon intensity, Shields requests regeneration, Repair reports damaged reactor/battery or critical systems, and Captain sets risk appetite. Power should announce both a boost and what was reduced to fund it.

### Failure and automation

Demand above sustainable output drains reserve. Empty reserve triggers a hard lock: all groups fall to minimum and controls freeze until recovery past the authored threshold. Backfill uses authored reserve hysteresis and budget-aware priority. The interface must expose exact lock/recovery state and provide one-tap safe presets in the accepted deeper-control direction.

## Repair

### Experience

Repair manages scarcity under incomplete information. The operator sends limited teams toward the most consequential damage, waits for travel and diagnosis, and chooses what an on-site team should restore first.

### Information and actions

Repair sees aggregate ship hull, exact Core damage, station-level dispatch targets and requests, team availability/travel/on-site/return state, and exact non-Core detail only where a team is on site. It dispatches a free team to Core or a station and, after arrival, pins the next visible damaged system for that team.

### Coordination

Station owners know exact local damage and request help socially or through advisory state; Engineering initially knows less. Captain sets mission priority, while Power may need to protect the reactor or support repair rate. The information gate is intentional: diagnosis arriving with the team creates conversation rather than an omniscient repair list.

### Failure and automation

A team in transit reveals no new detail and cannot be reprioritised as if already present. Destroyed or unavailable systems continue to impair the ship until repaired; all repairable system hull reaching zero ends the player ship. Backfill ranks candidates from the same visible aggregate/request facts. Lists and team states need explicit labels, progress, and confirmation that do not depend on animation or colour.

## Future Command

T1's auxiliary human-seeking Command station is normally hosted by Captain through hull data and need not create another lobby seat. It lists AI-controlled stations and sets authored standard, alert-neutral or objective-specific stances at station scale. It does not operate individual systems. Its complete lifecycle and shared Hero Bar presentation are defined in [Command and Crew Control](../mechanics/command-and-crew-control.md).

## Future Operations and Medical

Accepted unscheduled Operations systems plan and support external and Away Missions. A hull may carry several Operations systems with different mission tags, transport, capacity and personnel authority. Medical triages and treats Duty Officer consequences, supports missions and reads medical results while requesting specialist scans from Science.

Both use the common station-database framework and are detailed in [Duty Teams, Officers and Operations](../mechanics/duty-teams-and-operations.md).

## Shared viewscreen

The viewscreen is a communal surface, not Captain’s private monitor. Authorised stations may request their permitted system surface; Captain may select camera directions or cinematic mode. The most recent valid command wins, with no fixed source-priority hierarchy. The resolved owner and mode must remain visible so the bridge understands why the screen changed. Captain reclaims it by deliberately choosing Fore or Cinematic rather than through a hidden “default” action.

## Station acceptance criteria

- A new player can state their station’s core question, two primary actions, and at least one reason to speak to another officer.
- Every time-critical action exists on the summary surface and has clear authoritative feedback.
- Every AI world reading has a human-visible counterpart from the same producer; AI-private judgement and memory are not falsely presented as extra world knowledge.
- Damage, range, power, interference, control source, and scenario restrictions have distinguishable unavailable/refused states.
- A station remains usable when bundled with other families and when one of its systems visits another human-held station.
- Critical controls have tap/keyboard alternatives where applicable, generous targets, persistent state, and non-colour cues.
- Playtests record idle time, overload, unwanted automation, useful cross-station exchanges, and whether detail obscured urgent work.

## Canonical sources

- `pasm/spec/design/` station design slices, especially `console-complexity.yaml`, `station-ratings.yaml`, `viewscreen.yaml`, and `ship-manuals.yaml`.
- `gui/console-state.js`, `gui/console-families.js`, console HTML, and shared components.
- `assets/entities/alliance_*.toml` station/rating/system authoring.
- `wiki/concepts/information-parity-audit.md` for the current AI fact-to-console audit.
