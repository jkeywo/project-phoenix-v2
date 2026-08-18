# Project Phoenix — Campaign Continuity and Persistence

| Field | Value |
|---|---|
| Document | GDD-CAMPAIGN-CONTINUITY |
| Status | Directional design built on an implemented projection and save foundation; a complete campaign player flow is not yet shipped |
| Owner | Unassigned |
| Last updated | 2026-08-18 |
| Scope | Episode continuity, durable facts, save/resume, debrief, campaign selection, failure, refit, crew identity, and compatibility |
| Authority | Human-facing design synthesis. PASM and code govern the implemented projection and snapshot contracts; future implementation commitments belong in PASM and GitHub. |

Phoenix campaigns are episodic voyages: a crew completes a bounded scenario, sees what happened, and carries a deliberately small set of consequences into a later scenario. The game does not simulate a continuous galaxy between episodes. Continuity comes from remembered decisions, obligations, discoveries, relationships, surviving assets, damaged structures, and the identity the players give their ship and crew.

Related documents: [Game Design Overview](./overview.md), [Game and Session Lifecycle](./game-lifecycle.md), [Scenario Authoring](../systems/scenario-authoring.md), [Falling Skyway](../content/scenarios/falling-skyway.md), [Thin Margin Setting](./thin-margin-setting.md), [Ships and Ship Systems](../systems/ships-and-systems.md), [AI and Backfill](../systems/ai-and-backfill.md), [Onboarding and Accessibility](./onboarding-accessibility.md), and [Future Modes](../future/future-modes.md).

## Campaign promise

A campaign should let a group say, “this is our ship, this is what we did, and the world remembers,” without making attendance, bookkeeping, or a long-running save a prerequisite for play. Every campaign episode remains a complete scenario with its own immediate problem and ending. Prior history changes context, opportunities, relationships, and consequences; it should not make a later mission incomprehensible to a returning player or impossible for a new one.

The intended campaign is curated and consequence-led rather than an upgrade treadmill. It may eventually combine authored episodes with Patrol Mode assignments, but its core unit remains a scenario that can be loaded, played, debriefed, and left in a stable state.

## Three persistence scopes

| Scope | Purpose | Examples | Default boundary |
|---|---|---|---|
| Connection/session identity | Let a participant disconnect and return to the live host | Session token, display name, station claim while available, saved human rating | Host session |
| Mission snapshot | Resume the same authoritative run | Entities, damage, weapons, objectives, clocks, Rhai callbacks, flags, RNG streams, player/session state | Exact compatible scenario and content version |
| Campaign continuity | Seed a different episode with selected durable facts | Outcome, campaign tallies, commitments, evidence, standing, named surviving assets, structure condition | Campaign record across compatible episodes |

These scopes must not substitute for one another. Reconnecting does not rewind the mission. Loading a snapshot does not start a new episode. Projecting campaign facts does not copy the previous world wholesale.

## Current foundation

The runtime already supplies two important foundations:

- A `PhoenixSnapshot` records a whole authoritative mission payload in RON inside the `vellum-save` envelope. The host can save and resume a local slot and can export or import a portable `phoenix-save.ron` file. Restore checks format, build/content, scenario, and script compatibility before activating the world.
- `campaign::projection::project` folds a stored run into versioned `CampaignFacts`. Its declared fields are mission, outcome, `campaign.*` tallies, commitments, evidence, standing, named assets, and structures. `seed_flags` restores the campaign tallies and settled commitment flags for a later scenario script to read.

Falling Skyway already writes a complete campaign handoff at its close. What is not yet a finished product flow is the campaign shell around these pieces: campaign creation and naming, a campaign record containing multiple episodes, episode selection, debrief confirmation, later-world override binding, crew or ship roster management, campaign browsing, and an ordinary non-debug save interface.

## Campaign loop

The proposed loop is:

1. **Choose or continue a campaign.** The host creates a campaign or loads a compatible campaign record. A one-shot scenario remains the default path and requires no campaign.
2. **Review the situation.** The crew sees a concise “previously” summary, active obligations, relevant standing, ship identity, and the reason this episode is available. New players can understand the current problem without reading the complete archive.
3. **Choose an episode.** Early campaigns may offer one authored continuation. Later campaigns may offer a small set of assignments filtered by prerequisites, unresolved pressures, recent repetition, and campaign state.
4. **Prepare the ship and crew.** The scenario states the offered hulls, recommended crew, allowed loadout/refit choices, and any named crew assignments. Unclaimed stations use their authored defaults and Backfill.
5. **Play a bounded scenario.** The mission runs through the normal lobby and lifecycle. Campaign input is frozen at scenario start; it does not mutate underneath a live run.
6. **Resolve and debrief.** The authoritative terminal outcome closes the run. The campaign projection extracts only declared durable facts, and the debrief explains them in player-facing language.
7. **Commit the episode.** The host confirms the resulting campaign checkpoint. The prior checkpoint remains recoverable until the new one is safely stored.
8. **Continue, replay as a branch, return to one-shot play, or exit.** No continuation is forced immediately after a debrief.

## What the campaign remembers

### Mission and outcome

The record names the episode and its authoritative terminal outcome. An unfinished saved run is not silently converted into a completed episode. A campaign may retain a short episode history for debrief and branching, but later scenarios consume declared facts rather than parsing prose summaries or assuming that `victory` means the same thing in every mission.

### Campaign tallies and exclusive facts

Scenario-authored counters under the `campaign.<mission>.<family>.<fact>` prefix carry facts that do not fit a generic record. Mutually exclusive families must write exactly one member on every ordinary completion path. A later script reads the named counter directly, so absence is never used to guess which outcome occurred.

Tallies should describe consequential facts, not reproduce every event. “The convoy received passage,” “the strike was coerced,” or “three casualties occurred” can matter later. Every torpedo fired, repair tick, or traffic order does not.

### Commitments

Promises carry their authored identifier, the party to whom they were made, their text identifier, and whether they are open, kept, or broken. A later episode may acknowledge, complicate, or collect on a commitment. It must not rewrite a broken promise as kept merely because the player chose a different mission next.

### Evidence and knowledge

Evidence carries a named subject, player-facing text identifier, and provenance. It represents what the crew actually learned through play. Later episodes may unlock dialogue, warnings, objectives, or accusations from this knowledge, but should preserve uncertainty where the original evidence was circumstantial.

Knowledge remains scoped appropriately in future multi-ship campaigns. One bridge’s discovery is not automatically another bridge’s knowledge; transfer requires an authored communication or shared debrief rule with provenance.

### Standing and unresolved disputes

Standing records the disposition of authored parties and whether their workforce dispute remains active. This is relationship context, not a universal morality score. A party can respect the crew’s competence while opposing its decision, and two organisations in the same polity may react differently.

### Named assets and structures

Named entities that still exist may be carried as campaign assets by authored name and, where applicable, template. Structures carry condition as a fraction and their operational flags. The next scenario decides whether and how to instantiate those facts; identity never depends on a per-run UUID.

A surviving convoy, damaged skyhook, lost patrol craft, or intact depot may therefore reappear. Incidental unnamed ships, spawned debris, asteroid populations, and transient visual objects do not.

## What does not persist by default

Campaign continuity excludes transient combat and simulation state unless a future design explicitly adds a durable field with a player-facing reason. The default exclusions include current hull and shield fractions, weapon loads and cooldowns, repair progress, power allocation, heat, AI continuation state, positions and velocities, live targets, spawned hazards, mission clocks, pending callbacks, temporary flags, random-number positions, and console selections.

This boundary prevents one scenario’s tuning from accidentally controlling the next. If an episode wants damage to matter later, it should project a named consequence such as `ship_needs_refit`, a persistent injury, lost asset, or reduced strategic resource—not copy the final combat component graph into a new world.

## Ship identity and condition

The campaign may own a player-visible ship name, hull lineage, livery or presentation choices, accepted refits, and a log of notable service. The scenario still owns the physical ship instance it spawns. Authored names bridge the two layers; runtime UUIDs do not.

Persistent ship condition should remain legible and bounded. The default between-episode assumption is that routine fuel, ammunition, shields, and repairable combat wear are restored during unplayed transit or maintenance. A scenario may deny that reset only by writing a durable condition and providing a later episode or refit rule that consumes it. This keeps continuity meaningful without turning every mission into logistics cleanup.

## Crew identity and assignments

Player identity is not the same as character identity. Session tokens currently reconnect browsers to a live host; they are not accounts, campaign profiles, or officer characters. A campaign should allow different human players to occupy the same stations across episodes without treating absence as desertion or losing the ship’s history.

The accepted unscheduled personnel model carries named Duty Officers and fixed anonymous Duty Teams. Officers have traits, system or mission assignments and availability states; teams have a type, leader and current assignment. Campaign continuity may carry officer recruitment, promotion, fatigue, injury, disappearance and death together with mission outcomes. Watches, hunger, sleep, continuous location and individual ensign simulation remain outside scope. See [Duty Teams, Officers and Operations](../mechanics/duty-teams-and-operations.md).

## Episode input and output contract

Every campaign-capable scenario should declare:

- which campaign fact version it accepts;
- which prior facts it reads and the default for a one-shot or older campaign where they are absent;
- how standing, named assets, and structure condition bind to its authored entities;
- which prior facts affect briefing, initial state, available choices, dialogue, objectives, or ending;
- which exclusive campaign families it writes and the invariant for each;
- which commitments, evidence, standings, assets, and structures may leave the episode;
- how partial failure and every terminal route produce a coherent handoff;
- whether the episode is replayable, repeatable in the same continuity, or single-use.

A scenario must remain playable without prior facts unless it is explicitly marked as a locked continuation and the campaign shell prevents invalid selection. Missing history should normally select an authored neutral/default opening, not an error path.

## Falling Skyway handoff

Falling Skyway currently writes six fact families at mission close:

| Family | What it preserves |
|---|---|
| Passage | Which claimant groups were carried during the transfer opportunity and which remained behind |
| Strike | Whether the workforce dispute was negotiated, coerced, or left unresolved |
| Evidence | How much of the underlying conduct the crew established and what they did with the finding |
| Casualties | Casualties by consequential beat and in total |
| Structure | Whether the skyhook held or was lost |
| Commitments | How many promises were kept and broken |

These facts are implemented and projected, but no specific follow-on episode is yet the ratified consumer. A future continuation should use several families in independent ways rather than reduce them to one good/bad score: passage can determine who is present, strike resolution can alter workforce disposition, evidence can change leverage, casualties can change tone and trust, structure state can change the physical opening, and commitments can affect who believes the crew.

## Debrief and “previously” presentation

The debrief is the human-readable face of the projection. It should show the authoritative outcome, objective results, casualties where relevant, promises made and settled, important evidence, changes in standing, surviving or lost named assets, structure condition, and the specific facts being committed to the campaign. It must distinguish observed fact from interpretation and avoid collapsing trade-offs into a single score.

Before the next episode, a shorter “previously” view should present only facts relevant to that episode, with access to the complete campaign log on demand. Text identifiers rather than stored rendered English allow localisation and later editorial revision without changing the underlying fact.

## Save, resume, and checkpoint rules

A mission snapshot resumes the exact compatible run. It should be possible to save at an authoritative tick boundary, acknowledge completion or failure, and refuse incompatible data before mutating the world. Portable export allows a host to move the save deliberately; browser-local storage alone is not sufficient campaign custody.

A campaign checkpoint is written after an episode’s projection has been accepted. The write should be atomic from the player’s perspective: either the old checkpoint remains current or the complete new checkpoint does. Campaign records need their own format/version, campaign identifier, episode history, current durable facts, compatible content identity, and provenance for imported or modded episodes. The existing `CampaignFacts` version is a vocabulary version for one projection, not yet a full campaign-file format.

Autosave policy, number of manual slots, storage location, cloud/account support, encryption, and recovery retention are open product decisions. The zero-setup route must still offer an understandable local and portable path without requiring an account.

## Failure, replay, and branching

Failure should usually advance continuity with consequences rather than demand a perfect replay. An episode may destroy an objective, lose an asset, worsen standing, or close one route while opening another. A campaign-ending result must be explicitly authored and signalled before commitment.

Replaying a completed episode does not overwrite established history by default. The safe options are:

- replay as a non-canonical one-shot;
- branch from the pre-episode checkpoint into a new campaign lineage;
- deliberately replace the episode result through an explicit host action that warns which later history will be invalidated.

Restarting an unfinished mission from its last snapshot remains ordinary resume, not a branch. A campaign UI should display the distinction clearly.

## Refit, rewards, and progression

Progression should broaden choices and deepen identity rather than produce unbounded numerical growth. Campaign rewards may include access, allies, information, political leverage, named crew, service history, cosmetic identity, new sidegrade components, or an authored reactor improvement where the future customisation design permits it.

Routine between-episode restoration is free unless scarcity is the point of a campaign arc. Permanent improvements, injuries, losses, and strategic resources require explicit campaign fields, caps, recovery rules, and scenario compatibility. One-shot and fixed-balance scenarios ignore campaign advantages or substitute authored defaults so their challenge does not depend on an old save.

## Host, attendance, and zero-setup rules

- A campaign belongs to a portable host record, not to one phone or one player’s browser identity.
- Players can join or leave between episodes without corrupting continuity. Station assignment is renewed through the lobby each scenario.
- Backfill and AI remain available in every campaign episode; a campaign never requires the same human attendance to continue.
- A new player receives the relevant “previously” context and can take a station without reconstructing every prior decision.
- One-shot play remains the primary zero-setup route. Creating or loading a campaign is always optional.
- Future multi-ship campaigns must identify which ship or fleet owns each fact and preserve per-ship knowledge provenance.

## Compatibility and mod content

Snapshots are strict because they restore implementation state. Campaign projections should be more durable because they contain authored names and a small declared vocabulary, but they still require version checks and migrations when fields or meanings change. A consumer must never silently reinterpret an unknown version.

Modded scenarios may read and write campaign facts only through declared namespaces and must record their content identity in campaign history. Removing a mod should not erase old history, but it may make a dependent continuation unavailable. The campaign selector explains the missing dependency rather than attempting a partial load.

## Accessibility requirements

- Every campaign choice, save result, compatibility refusal, branch warning, debrief fact, and “previously” summary is available as persistent text, not only animation, colour, sound, or a transient toast.
- Dense episode history supports headings, filtering, and concise summaries; players are not required to read a long chronological log under time pressure.
- Save and commit actions confirm success and name the destination or checkpoint.
- Destructive replacement or branch invalidation requires clear confirmation and an accessible recovery explanation.
- Campaign continuity does not assume one person can remember prior sessions; the interface carries the relevant context.

## Playtest questions and success measures

1. After a break between sessions, can the crew explain the current situation and its relevant history from the “previously” view in under two minutes?
2. Can a new player join a continuing campaign, understand their station’s immediate responsibility, and contribute without a private briefing from another player?
3. Do players notice at least one prior decision changing a later episode in a specific, credible way?
4. Does the debrief match what players believe they did, including promises, casualties, evidence, and partial failure?
5. Do players understand the difference between resume, replay, branch, and continue before acting?
6. Can the host export, restore, and migrate a compatible campaign without losing the last committed checkpoint?
7. Do persistent rewards create interesting choices without making one-shot defaults or newer campaigns feel categorically inferior?
8. Does continuity generate anticipation and responsibility without making players afraid to experiment or accept failure?

## Open decisions

- The first authored episode that consumes Falling Skyway’s handoff and which of its facts materially alter the opening.
- Campaign file structure, storage backend, slot/branch presentation, autosave cadence, backup retention, and migration policy.
- Whether a campaign follows one named ship, a fleet, a command, or supports all three as explicit frames.
- The minimum ship identity fields and when hull changes preserve the same campaign ship.
- Duty Officer roster size, trait vocabulary, recruitment and promotion rules, and concrete recovery timings remain open within the accepted [Duty Teams and Operations](../mechanics/duty-teams-and-operations.md) model.
- Which customisation choices and strategic resources persist, and which scenarios opt out for fixed balance.
- Episode availability rules, repeatability, authored campaign endings, and how Patrol Mode assignments enter the sequence.
- Whether campaign history can be edited by a GM and what audit/recovery guarantees such edits require.

## Canonical sources

- `src/campaign/projection.rs` and `pasm/spec/architecture/world-files.yaml` for the implemented campaign fact vocabulary and projection boundary.
- `src/snapshot.rs`, the host save/import/export surfaces, and snapshot PASM for exact-run persistence and compatibility.
- `assets/worlds/falling_skyway.toml` and its end-to-end tests for the current six-family handoff.
- [Game and Session Lifecycle](./game-lifecycle.md) for live-session, round, lobby, and exit boundaries.
- [Future Modes](../future/future-modes.md) for customisation, crew assignments, Patrol Mode, multi-ship, and GM scope.
