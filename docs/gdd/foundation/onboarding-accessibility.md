# Project Phoenix — Onboarding, Tutorials, Manuals, Facilitation, and Accessibility

| Field | Value |
|---|---|
| Document | GDD-ONBOARDING-ACCESSIBILITY |
| Status | Working draft; current tutorial/manual behaviour and proposed accessibility requirements are distinguished below |
| Owner | Unassigned |
| Last updated | 2026-08-18 |
| Scope | First-session learning, contextual help, station handover, facilitation, input alternatives, sensory access, and test targets |
| Authority | Player-experience requirements. Current implementation is authoritative only where explicitly described as implemented. |

Phoenix should let a group enter play without studying a rulebook, while still rewarding crews who learn the ship deeply. Onboarding is therefore layered across the lobby, station choice, ship manual, contextual overlays, scenario opening, crew conversation, and post-session reflection. Accessibility is part of that learning path rather than a later visual polish pass.

Related documents: [Game and Session Lifecycle](./game-lifecycle.md), [Campaign Continuity and Persistence](./campaign-continuity.md), [Station Experiences](../systems/station-experiences.md), [Ships and Ship Systems](../systems/ships-and-systems.md), [AI and Backfill](../systems/ai-and-backfill.md), [Difficulty, Balance, and Playtesting](./difficulty-balance-playtesting.md), and [Future Modes](../future/future-modes.md).

Future crew-control and delivery details: [Command and Crew Control](../mechanics/command-and-crew-control.md), [Release Bands C7–C9](../future/release-bands-g-i.md), and [Planned but Not Scheduled](../future/planned-not-scheduled.md).

## Onboarding goals

- A first-time group can connect, choose a station, understand readiness, and begin without an account, installation, or external rules explanation.
- Every player knows their station’s core question, urgent controls, visible responsibilities, and at least one reason to speak to another officer.
- Help appears close to the state or action that makes it relevant and can be dismissed without blocking play.
- Returning players can skip or suppress familiar instruction while retaining access to reference material.
- A reconnect or station handover does not replay the entire first-session flow or leave the incoming operator without context.
- Reduced workload, alternative input, and sensory equivalents remain real participation, not spectator modes.

## Learning layers

| Layer | Purpose | Timing |
|---|---|---|
| Host and join guidance | Get devices connected and make failures recoverable. | Before lobby |
| Scenario and ship brief | Establish premise, expected duration, recommended crew, and hull workload. | Selection |
| Station card | State the station’s question, systems, rating choices, and current holder. | Lobby |
| Ship manual | Give ship-specific, read-only reference for every authored station and system. | Lobby and in play |
| First-visit overlay | Explain the station’s role and where primary controls live. | First mount |
| Contextual overlay | Explain a control or state when it first becomes relevant. | In play |
| Authored scenario opening | Provide the first shared problem and actionable objective. | Scenario start |
| Crew/facilitator teaching | Let experienced players assign, explain, and hand off work socially. | Throughout |
| Debrief | Connect actions to outcome and identify a next role or deeper setting. | Scenario end |

## Current implemented help model

Ship TOML may author `manual_overview` for a station and contextual `[[station.tutorial]]` entries. The host delivers tutorial definitions in the ship configuration, while the pure client evaluates them against local console state. Current trigger kinds are `first_visit`, `control_unused`, and a state comparison over the console’s own payload. Unknown kinds or comparisons fail closed.

Only one eligible overlay is active at a time, selected by authored priority and then authored order. Dismissal and control use are stored locally per station, so reconnecting does not replay completed tips and the same overlay id may exist independently on several stations. Tutorial dismissal is client-local presentation state and never becomes a ship command.

The ship manual is read-only and ship-specific. It combines the authored station overview with generated details from systems actually mounted on that hull: weapon ranges/arcs/cadence, torpedo capacity, radar ranges, reactor/battery, repair timings, comms range, helm capability, and rating automation. It should remain available during play without pausing the whole crew.

The Alliance Destroyer currently has the most complete contextual tutorial content. Other hulls require their own authored coverage as they move through the release-readiness sequence.

## Tutorial-writing rules

- Teach the player’s decision and observable consequence, not the implementation or message type.
- One overlay teaches one idea. A first-visit welcome may name the role and primary region; later overlays explain individual controls or states.
- State-triggered help appears before or at the first useful decision, not after the player has already failed it.
- A tutorial must never cover the only control needed to dismiss or recover from it.
- Text should fit the smallest supported phone without scrolling over an urgent control where practical.
- Use the same player-facing terms as the console, manual, objective, and crew callout.
- Do not instruct a fixed station layout when hulls bundle systems differently; author per hull/station where the placement differs.
- Important scenario knowledge belongs in objectives, comms, or persistent help—not a dismiss-once tutorial.

## First-session path

1. The host shows a join QR/link and plain-language connection state before requiring scenario commitment.
2. The scenario card states premise, likely duration, recommended crew, offered ships, and whether the experience is direct combat, operational crisis, or another form.
3. The ship card states maximum stations, current recommended use, and broad role. Station cards show systems and rating/automation impact.
4. Each player claims a station, chooses a comfortable rating, opens its short overview if needed, and readies independently.
5. A visible countdown confirms collective entry; changing readiness cancels it legibly.
6. On first mount, the console identifies the station and urgent controls. The scenario opening supplies one shared objective and the first opportunity to report or act.
7. Additional overlays appear only as relevant capabilities or states arise.
8. The debrief asks what the player knew, contributed, and would like to try next.

## Station handover and reconnect

A disconnect preserves identity and station assignment while Backfill operates its systems. Reconnecting should return the player to the same station, current rating, live authoritative state, and locally persisted tutorial progress. The interface should acknowledge that AI covered the station and summarise current control source without replaying stale transient alerts.

A deliberate handover should expose the station overview, current objective, rating, active target/waypoint, critical damage, outstanding messages or commitments, and any system currently under automation. The outgoing player can teach socially, but the interface must not require an oral memory dump for basic state.

## Facilitation

The baseline game should not require a facilitator. For events and mixed-experience groups, a facilitator may recommend stations, help choose ratings, explain the first objective, and pause or intervene through supported host/GM tools when available. Facilitation must not rely on hidden rules that ordinary players cannot later discover through manuals or feedback.

Facilitator guidance should include: suggested station assignment by player preference; common connection recovery; what Backfill is currently doing; how to reduce workload; scenario-specific content notes; when to let the crew struggle; and questions for a short debrief. Future GM tools are described in [Future Modes](../future/future-modes.md).

## Accessibility target

The proposed target is that joining, understanding a station, communicating essential state, and taking every time-critical action must not depend on one sensory channel, colour recognition, precise dragging, rapid repeated input, or hearing speech. Conventional web surfaces should target WCAG 2.2 AA where applicable, with game-specific evaluation informed by the Xbox Accessibility Guidelines. This is a target, not a claim of present conformance.

## Visual access

- Critical state combines text or symbols with colour. Directional shield arcs, alerts, weapon readiness, team state, and dialogue urgency need labelled distinctions.
- Text supports browser enlargement and zoom without hiding primary controls or forcing horizontal scrolling at supported widths.
- Contrast tokens are tested in real console states, including disabled, selected, warning, critical, and overlay combinations.
- Motion, camera shake, bloom, flashing, animated backgrounds, and high-speed streak effects can be reduced independently where technically possible.
- The host viewscreen uses readable silhouettes and overlays at room distance; essential detail also exists on a player console.
- Charts and radars provide list or target-detail alternatives for information that cannot be reliably interpreted from geometry alone.

## Hearing and communication access

- Every critical audio cue has a visual or textual equivalent with source and urgency where relevant.
- Any voiced dialogue retains a complete transcript, speaker name, and persistent response choices.
- Alerts should not require judging pitch alone; use labelled state and distinct visual patterns.
- The game should support a crew communicating through in-game pings, target/waypoint designation, objective priority, intent advisories, and visible requests when voice conversation is unavailable.
- Time-critical scenario information must remain reviewable after its sound ends.

## Motor and input access

- Primary touch targets are generous and separated from destructive or contradictory actions.
- Drag controls such as helm sticks and charge gestures provide discrete tap/keyboard alternatives with equivalent authoritative commands where the gesture itself is not the game decision.
- Hold actions offer a toggle or explicit set-state alternative and show whether the state is latched, held, charging, or inactive.
- Repeated tapping is not used as a measure of effort or repair speed.
- Time-critical controls remain on the summary surface; detail screens do not require precise navigation during emergencies.
- Physical peripherals may add alternatives but never become the only path.

## Cognitive access and workload

- Station ratings and Backfill allow the player to reduce direct workload without leaving the crew.
- Interfaces favour stable placement, consistent terms, progressive disclosure, and recognition over recall.
- Objectives, deadlines, messages, commitments, and errors persist long enough to review.
- Important actions use confirmation selectively: irreversible commitments and destructive host actions deserve it; routine combat inputs do not.
- Tutorials can be dismissed, revisited through help, and reset per station without clearing identity or other preferences.
- A host-level reduced-time-pressure option may alter scenario deadlines only as an explicit shared setting, never as an invisible local preference.

## Alternative interaction paths by station

| Station family | Interaction that needs an alternative | Minimum alternative path |
|---|---|---|
| Captain | Camera selection and objective priority | Labelled buttons/list with persistent current mode/focus |
| Helm | Analogue thrust/yaw/lateral dragging | Discrete directional controls, neutral action, numeric state |
| Tactical | Hold-to-charge and spatial target selection | Target list, tap fire/set-state controls, labelled arc/refusal reason |
| Sensors | Radar-only selection and colour categories | Contact list, text analysis, explicit observation quality |
| Navigation | Chart-only waypoint placement | Objective/contact destination list and current waypoint text |
| Comms | Audio dialogue or timed reading | Persistent transcript, speaker labels, no auto-expiring response unless scenario-essential and configurable |
| Shields | Colour-only arc bars | Facing names, numeric/semantic health, labelled focus |
| Power | Pip/colour-only allocation | Group names, numeric level/total, draining/charging/locked text |
| Repair | Drag ordering or animated team status | Tap-to-dispatch/pin, labelled travel/on-site/return state |

## Settings ownership

Local presentation/input preferences include text scale, contrast theme, reduced motion, volume categories, haptics, control alternatives, and tutorial visibility. Shared host/scenario settings include deadline multiplier, pause permissions, difficulty variant, and other changes to simulation timing or content. The UI must state when an accessibility choice is local versus session-wide.

Planned accessibility assistance is personal but may affect authoritative control ownership. A profile names functional effects and requested assistance, never diagnoses; any player may enable any setting for any reason. The host combines the private profile with hull capabilities to show station suitability and delegate named subfunctions to the same limited-AI machinery used by the complexity ladder. Other players can see the assisted function or unsuitable station, but not the setting or reason behind it.

A genuinely incompatible station is greyed out with an explanation, and human-seeking placement skips that player when the complete visiting station cannot be used at its scenario-required rating. Complete lockout should be rare. Every base playable hull at full supported player count must author at least one station/rating combination usable with the complete supported option set in a simple scenario. This guarantee does not cover understaffed play, scenario-added duties or solo completion of complex content.

## Roadmap integration

Accessibility is delivered incrementally from T1 rather than held for a late retrofit. T1 establishes the shared settings/profile, presentation and anonymous eligibility seams while its settings shell, Hero Bar, station resolver and native panes are changing. C2 composes assistance with station ratings and scenario floors. C3 covers alternative relative-motion, docking and multi-ship interaction paths. C4 adds assistance for mastery-heavy Engineering work. C5 addresses dense tactical information and sensory load. C6 adds continuing-mode maps, summaries and return support. C7 adds strategic alternatives, C8 makes accessibility a generation constraint, and C9 completes richer spectator participation and a whole-product audit. The detailed allocation is maintained in [Release Bands C7–C9](../future/release-bands-g-i.md).

## Validation and playtesting

- Test on the smallest supported viewport and with browser text enlargement, zoom, reduced-motion preference, keyboard-only navigation where applicable, and at least one mobile screen reader.
- Test critical states in monochrome and common colour-vision simulations, while verifying labels rather than accepting filters as proof.
- Run each station with the primary gesture unavailable and confirm the alternative performs the same command.
- Observe first-time players without pre-briefing: measure time to connection, station comprehension, first meaningful action, and facilitator interventions.
- Include disabled players and relevant specialist testers in design evaluation rather than relying only on automated audits.
- Record overload, idle time, missed alerts, accidental actions, unread text, and abandonment of a station rating.

## Acceptance criteria

- A first-time player can join and identify their station’s main responsibility without external documentation.
- Every time-critical action and critical state has the required sensory/input alternative.
- Tutorials never block the action they teach, survive reconnect correctly, and can be dismissed/reset locally.
- Manuals describe the selected hull’s actual capabilities and ratings rather than a generic ship.
- A station handover exposes sufficient current state for the incoming player to resume responsibly.
- Accessibility preferences clearly distinguish local presentation from shared rule changes.
- No optional native client, peripheral, GM, or venue integration weakens the zero-setup accessible browser route.

## Canonical sources

- `gui/tutorial-state.js`, `gui/components/ph-tutorial-overlay.js`, and tutorial tests.
- `src/ship/manual.rs` and `pasm/spec/design/ship-manuals.yaml`.
- `assets/entities/alliance_destroyer.toml` and `assets/strings/strings.csv` for current tutorial content.
- [Game Design Overview](./overview.md) for the initial accessibility target.
