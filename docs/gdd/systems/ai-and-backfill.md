# Project Phoenix — AI, Backfill, and Command Doctrine

| Field | Value |
|---|---|
| Document | GDD-AI-BACKFILL |
| Status | Working draft; current behaviour and transitional architecture are identified explicitly |
| Owner | Unassigned |
| Last updated | 2026-08-18 |
| Scope | Vacant-station operation, human/AI handoff, NPC doctrine, information parity, transparency, and difficulty boundaries |
| Authority | Player-facing AI design. PASM, ship TOML, command admission, and runtime systems are authoritative. |

Phoenix uses AI to keep every authored ship functional at any crew size and to operate NPC ships through the same fine-system model. Backfill is not a separate simplified ship and NPC AI is not a scenario puppet: both decide actions against authoritative state and issue the same system-control commands admitted for humans.

Related documents: [Station Experiences](./station-experiences.md), [Ships and Ship Systems](./ships-and-systems.md), [Difficulty, Balance, and Playtesting](../foundation/difficulty-balance-playtesting.md), [Alliance Ships](../content/ships/alliance-ships.md), [Harrow Ships](../content/ships/harrow-ships.md), and [Onboarding, Tutorials, and Accessibility](../foundation/onboarding-accessibility.md).

Band A2 crew direction is specified in [Command and Crew Control](../mechanics/command-and-crew-control.md).

## Design goals

- A session remains playable from zero human crew through the ship’s station maximum.
- Adding a human transfers meaningful authority rather than spawning a duplicate controller.
- Removing or disconnecting a human produces a readable, recoverable handoff without disabling the ship.
- AI acts from facts a human operator could also receive, except for clearly identified private judgement or policy memory.
- AI competence is authored per fine system and hull, making doctrine inspectable and testable.
- NPCs and player Backfill obey the same physics, damage, power, sensors, target, faction, and command-admission rules.
- AI is useful enough to sustain the ship but leaves room for a skilled human crew to improve decisions and coordination.

## Terms

| Term | Meaning |
|---|---|
| Fine system | The smallest authoritative controllable capability, such as one phaser bank, Helm steering, Shields focus, Power reactor, or Comms. |
| Control source | The current authority allowed to operate a fine system: a human-held station or AI. |
| Backfill | AI operation of systems on a player-capable ship because a station is unclaimed, disconnected, or selected for automation by rating. |
| NPC AI | AI operation of an independently acting non-player ship. It uses the same fine systems but may have different stations, doctrine, and policies. |
| Policy | Authored ordered rules over host-provided facts/parameters that choose a registered channel/verb action. |
| Selector | Authored ranking over a candidate set, used for targets such as contacts, waypoints, repair destinations, or hail recipients. |
| Doctrine objective | Current top-level authored goal and movement intent such as Destroy, Reach, Retreat, or Patrol. It remains live but is partly transitional architecture. |

## Authority and symmetry

Human and AI commands converge at admission. Downstream gameplay systems do not branch on whether the accepted command came from a person or AI. The same action therefore has the same power, damage, range, cooldown, target, and state consequence regardless of origin.

Control source is resolved per fine system rather than per ship. A player may manually fly while Tactical weapons are automated; one Tactical bank may be manual while another is automated if the authored control-depth model permits it. There must never be two valid writers fighting over one fine system. When ownership changes, admission changes immediately and stale commands from the previous controller are refused or cease to apply according to that system’s command semantics.

## Station ratings and Backfill

Current station ratings author which owned systems are automated. The station holder may change rating during play; only that holder can do so. An unclaimed station is Backfill. On disconnect the station remains associated with its holder for reconnection but operates at Backfill until the session restores human control or another valid claim is made.

The accepted future star ladder adds simplified and detailed control rungs per system. Simplified control means a limited AI drives the full model from human-selected summary settings; it does not replace the system with a lower-fidelity simulation. Star 0 remains full Backfill. This ladder is accepted direction, not the current complete interface.

Human-seeking systems such as Navigation and Comms walk a complete authored station order, owner first, and attach to the first human-held station. They fall back to AI only when no human is available. Visiting systems carry their own control depth and do not inflate the host station’s rating.

## AI decision model

AI decisions run on the fixed logical tick and the shared authored AI cadence, not rendered frames or wall time. A host seeds a typed fact snapshot from authoritative state, the policy or selector resolves an action, and the AI emits an admitted command targeting one fine system. Readiness, range, damage, capability, and other final gates remain in the ordinary command handlers.

Every AI-capable fine system in production content must explicitly author a policy/selector or explicit idle declaration. There are no quiet default synthesisers. Invalid channels, verbs, parameter use, unreachable host facts, duplicate priority collisions, or missing declarations are content errors where validators can establish them.

Facts fall into two categories:

- **World readings** describe ship, target, objective, faction, range, bearing, damage, power, contact, waypoint, message, or scenario state. A human-visible counterpart is required.
- **Derived policy state** is the AI’s own running minimum, elapsed policy state, bounded-history verdict, or similar judgement over visible readings. It need not be exposed as an extra fact; the human analogue is reasoning from the same display.

## Cross-system coordination

Systems exchange authoritative coordination payloads rather than AI-only hints. Sensors can designate a target, report threat bearing, or send a frequency hint; Tactical can request a firing bearing from Helm; Navigation can send a waypoint; damaged stations can request Repair; Shields can report facing down/restored; Power can issue a brownout advisory.

The payload is derived from system state, so a human-operated sender can inform an AI receiver and an AI sender can inform a human receiver. Routing may choose presentation based on the receiver’s control source, but the underlying fact does not acquire a different meaning.

## Backfill behaviour by family

| Family | Current Backfill responsibility | Human advantage |
|---|---|---|
| Captain | Raises/stands down alert from authored hostile/contact and combat rules. | Understands mission values, promises, bluff, and crew intent. |
| Helm | Converts objectives and hazard state into authored actuator decisions. | Anticipates crew plans, accepts nuanced risk, and improvises geometry. |
| Tactical | Ranks targets and operates declared banks, tubes, and magazine policies. | Coordinates focus fire, ammunition, restraint, and future opportunities. |
| Sensors | Selects contacts and provides threat/frequency coordination. | Chooses investigative focus and interprets uncertain evidence. |
| Navigation | Selects eligible objective/chart destinations and issues waypoints. | Understands route intent, traffic, timing, and non-scored plans. |
| Comms | Selects eligible hails and authored responses. | Makes social, political, and commitment-bearing judgements. |
| Shields | Focuses from threat bearing and recent damage/health patterns. | Anticipates manoeuvre and can protect a planned exposure. |
| Power | Allocates group levels from authored triggers, reserve hysteresis, and budget priority. | Trades reserve against a spoken plan and can deliberately accept risk. |
| Repair | Ranks dispatch candidates from visible damage/request facts. | Understands which capability matters to the crew’s next action. |

Backfill should be competent at maintaining a baseline and responding to obvious state. It should not secretly solve evidence interpretation, negotiate the “best” political outcome, or know the crew’s unexpressed plan.

## NPC doctrine

NPC ships use faction relationships, contacts, objectives, and authored doctrine to decide what they are trying to achieve. Fine-system policies decide how their systems serve that intent. Scenario scripts may add objectives, alter faction relations, spawn groups, or override doctrine through supported data; they should not directly puppeteer every actuator or weapon shot.

NPC roles should be legible through behaviour. A Harrow Destroyer creates mobile ranged pressure; a Warhawk holds an artillery line; a civilian follows its route and responds to orders; an enforcer may remain a coercive presence without opening fire. Destruction, disablement, retreat, escort, surrender, or continued civilian operation can all be valid results when authored.

The current top-level doctrine/Helm path is transitional. Most fine systems already decide through authored policies, but Helm travel, impulse geometry, and shield-focus arc selection retain limited host-side decision kernels, and `[[behaviour.doctrine]]` remains live. Documentation should not describe doctrine retirement as complete until PASM’s removal conditions are met.

## Transparency and intent

Players need to know which systems AI controls, what the AI is doing, and how to take responsibility back. Station rating and control-source presentation should be persistent. AUTO badges, selected targets, waypoints, focus arcs, allocations, repair assignments, and visible system actions show results; intent advisories should explain important upcoming or changed behaviour without narrating every tick.

An AI explanation should use player terms: “Backfill is taking us to the priority waypoint,” “Weapons are held until Red Alert,” or “Repair Team 1 is travelling to Tactical.” It should not expose internal rule priority, raw facts, or policy-state identifiers in ordinary play. Debug/GM tools may expose those separately.

## Difficulty boundaries

AI difficulty should not be created by hidden stat bonuses, extra sensor truth, immunity to crew constraints, faster tick rates, or bypassing command admission. Appropriate authored difficulty levers include doctrine quality, reaction cadence within validated shared timing, target priorities, risk/reserve thresholds, coordination lag, willingness to retreat, and scenario force composition.

Backfill is primarily workload support, not a difficulty selector. Making a station more automated may lower cognitive and motor load but can reduce crew adaptability or efficiency. Scenario difficulty and accessibility settings should not silently rewrite Backfill competence for one player without telling the group.

## Failure and recovery

- If a system is damaged or disabled, AI loses the same availability a human does.
- If Sensors cannot observe a target, AI cannot use a private unrestricted scan to compensate.
- A reconnect or rating change must not leave stale AI intent applying after human control resumes.
- An AI with no eligible target or matching policy holds/idle rather than inventing an action.
- Rejected AI commands follow the same diagnosis and logging expectations as human commands.
- Scenario progress must not depend on a human-only UI action when zero-crew play is claimed; either AI/script can perform it through supported authority or the possible-player statement must be narrowed.

## Playtest questions

1. Could players identify which systems were automated without opening debug tools?
2. Did Backfill keep the ship viable without making occupied stations feel irrelevant?
3. When a player took control, was the handoff immediate and understandable?
4. Did the AI act on information the human could also see?
5. Could players explain an important AI choice from visible state, even if they would have chosen differently?
6. Did automation reduce unwanted workload, or did it take away the interesting decision?
7. Did an NPC’s movement and system use express a recognisable role?

## Acceptance criteria

- Every production AI-capable system has explicit authored policy/selector or idle state and passes strict validation.
- Exactly one control source can command each fine system at a time.
- Human and AI commands reach the same authoritative handler and produce identical rules-level effects.
- Every authored AI world reading on a player-capable hull has a rendered human counterpart from the same producer, with documented exceptions only for derived policy state.
- Disconnect, reconnect, station claim, rating change, and human-seeking movement preserve a coherent control source.
- AI cannot bypass damage, range, power, faction, alert, ammunition, or scenario authority.
- Seeded tests cover doctrine and outcome stability; human playtests cover helpfulness, legibility, workload, and perceived agency.

## Canonical sources

- `pasm/spec/architecture/data-driven-fine-system-ai.yaml`, `npc-ai-factions.yaml`, and `station-system-authority.yaml`.
- `pasm/spec/design/console-complexity.yaml` and `station-ratings.yaml`.
- `src/entities/ai_flag_hosts.rs`, `src/command_admission/`, and fine-system AI hosts.
- `assets/entities/fragments/ai/` and shipped hull TOML.
- `wiki/concepts/information-parity-audit.md`.
