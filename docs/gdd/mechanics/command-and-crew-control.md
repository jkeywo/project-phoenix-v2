# Project Phoenix — Command and Crew Control

| Field | Value |
|---|---|
| Status | Accepted for Band A2, with later accessibility assistance |
| Scope | Command stances, AI station direction, spectators, AFK delegation, human-seeking stations and the shared Hero Bar |
| Audience | Design, UI, simulation, content and playtest |

Band A2 gives a small or partially automated crew a way to direct the ship without operating every system. Command works at station scale: it tells an AI-controlled station what posture to adopt, then that station's existing authored policies decide how its individual systems act.

Related documents: [AI and Backfill](../systems/ai-and-backfill.md), [Station Experiences](../systems/station-experiences.md), [Game and Session Lifecycle](../foundation/game-lifecycle.md), [Onboarding and Accessibility](../foundation/onboarding-accessibility.md), [Native and Network Foundation](../systems/native-network-foundation.md), and [Planned but Not Scheduled](../future/planned-not-scheduled.md).

## Command system

Command is an auxiliary human-seeking station normally hosted by Captain through hull data, never by a hard-coded station rule. It need not create a dedicated lobby seat. It lists AI-controlled stations as aggregate units and exposes the stances currently available to each. It does not operate individual systems, bypass station admission or grant the operator a second set of controls outside its own station surface.

Each station authors standard stances and two fallback stances: normal-alert neutral and high-alert neutral. Scenarios and active objectives may contribute additional station-specific stances. A stance supplies facts and policy choices to the station's ordinary AI hosts, in the same broad manner that Red Alert currently informs behavior; it never applies a hidden statistical bonus.

Existing AI behaviors that branch directly on Red Alert should migrate to the two neutral stances where they express station posture. Red Alert remains available as an AI fact for cases that genuinely need it.

## Stance lifecycle

Every stance authors whether it persists behind human control or resets to the appropriate neutral stance. This prevents an old aggressive order from unexpectedly resuming after a human handoff while still allowing durable orders such as maintaining an escort posture.

An objective-specific stance exists only while its objective remains active. If the objective completes, fails or becomes invalid, the stance is removed immediately and any station using it falls back to the neutral stance for the current alert level. Objective stances may otherwise persist behind human control according to their normal authored rule.

Changing alert level switches between the two neutral stances only when the station is already in one of them. It does not overwrite a deliberately selected standard or objective stance.

## Human and AI Command

When a human controls Command, their choices are admitted and applied through the authoritative stance path. Human-held stations can see command intent as advice but are not constrained by it.

When no human controls Command, its AI operator—normally Captain through authored placement—selects stances from current ship knowledge and authored policy. AI Command uses the same catalogue and lifecycle as a human. It may choose objective stances when policy supports them; it does not invent orders outside the authored vocabulary.

## Station identity, hosting and delegation

A primary station may be a claimable player seat; an auxiliary station such as Command may exist only as a hosted surface. A human-seeking station always retains its own identity, complete UI, rating, systems and state. Its direct owner, current presentation host and fine-system control sources are separate facts.

A directly held station is preferred while its owner is active. Otherwise the station walks its finite authored fallback list, considering only directly player-held compatible stations, and appears there as a complete peer tab. It cannot land on another visiting station. An exhausted list falls back to AI even when an unrelated human remains available. Placement is automatic and has no manual decline control.

A visiting station uses its own authored visiting rating, normally Simplified, raised where necessary by a scenario requirement. Its host may change that visiting rating independently of the primary station. A direct claim returns a primary station to its owner without resetting state.

An AFK setting temporarily delegates every system on the player's directly held station while retaining the seat and its previous control configuration. The player and their station stop qualifying as active destinations. Human-seeking stations relocate through their authored fallback lists or fall back to AI. A human-seeking station whose own holder disconnects or goes AFK likewise relocates while keeping that seat reserved, then returns intact when its holder resumes.

## Shared Hero Bar

Every web and native console uses one shared Hero Bar shell. The directly held station is pinned first, followed by visiting stations in hull-authored order. Tabs never reorder in response to alerts. Selecting a tab presents that station's complete UI and preserves session-local interface context when switching away.

The bar shows the selected station's name, rating and authoritative health. Station health is the capacity-weighted aggregate of its damageable systems; a station with no damage model shows a neutral state. Every tab retains its own health indicator plus a separate importance indicator. One-off important events remain unread until the station is visited, while continuing critical conditions remain indicated until resolved. Alert semantics are authored or host-derived, never guessed from arbitrary client value changes.

## Spectator MVP

Band A2 adds an explicit Spectator role. Spectators do not count toward readiness and cannot issue simulation commands. The MVP provides one crew-public summary screen and allows a spectator to claim an eligible open station manually.

Richer spectator views are planned but not scheduled. They may select or swipe among authorised system-monitor projections, but must obey scenario knowledge boundaries rather than receiving an omniscient debug feed.

## Accessibility direction

Later accessibility assistance builds on the same delegation and human-seeking rules. Player settings describe functional effects, not diagnoses, and remain private. Eligibility is evaluated for the complete visiting station at its required rating. Other players may see that a station destination is unavailable or unsuitable, but never the reason.

Band A2 establishes the low-cost integration seam rather than deferring it: a shared Accessibility settings tab and private effect-named profile, an anonymous station/rating eligibility result consumed by the hosting resolver, keyboard/focus semantics and non-colour health/importance states in the shared Hero Bar, plus a per-function assistance schema. Later bands implement deeper assistance against this contract.

Every base playable hull at its full supported player count must provide at least one station/rating combination compatible with the complete supported accessibility-option set in a simple scenario. This guarantee does not extend to understaffed play or every complex scenario duty. Human-seeking placement should avoid overloading that accessibility anchor where another compatible player exists.

## Band A2 acceptance criteria

- Command lists AI-controlled stations and applies only authored stances available to them.
- Standard, neutral and objective stances follow their authored persistence and invalidation rules.
- Alert changes switch neutral stances without overwriting explicit orders.
- Human and AI Command use the same catalogue and authoritative application path.
- Human-seeking stations retain their complete UI, identity, rating and state while moving only to eligible directly held hosts.
- The shared Hero Bar presents stable station tabs, selected-station identity/rating/health, persistent per-tab health and separate off-screen importance alerts.
- AFK delegates systems without relinquishing the station and relocates human-seeking stations safely.
- Spectators do not block readiness, cannot command the simulation and can manually claim eligible open stations.
- Current Red Alert-dependent station posture is represented through the neutral-stance path where appropriate.

## Playtest questions

- Can a Captain direct a mostly AI crew without micromanaging individual systems?
- Can players explain why a station changed stance and when it will reset?
- Do objective stances disappear cleanly without leaving stale aggressive behavior?
- Does AFK preserve a player's seat while keeping the ship and visiting stations usable?
- Can a spectator understand the situation and enter an open role without delaying the lobby?

## Canonical sources

- [PRD #1092 — Band A2 Bridge Foundations: Crew Control Foundation](https://github.com/jkeywo/project-phoenix-v2/issues/1092)
- [Phoenix delivery roadmap](../../../pasm/spec/roadmap/phoenix-delivery-roadmap.yaml)
- [AI and Backfill](../systems/ai-and-backfill.md)
- [Console complexity design](../../../pasm/spec/design/console-complexity.yaml)
