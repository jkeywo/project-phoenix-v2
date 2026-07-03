---
title: CONTEXT.md (project root)
type: source
tags: [context, vocabulary, glossary, domain]
source_path: CONTEXT.md
status: shipped
updated: 2026-07-03
---

# CONTEXT.md

The canonical glossary. **Use these exact terms in code, comments, PRs, and architecture discussions.** Rewritten in the 2026-07-03 docs audit: entries describing deleted mechanics (shape-matching Breakdown/Repair, `IncreasePower` power wire, Bevy client resources) were removed or rewritten, and the Station/Console/System architecture vocabulary was added.

## Game-domain terms defined

- **Console** — one of nine operator surfaces (CaptainChair, Helm, Tactical, Repair, Sensors, Shields, Navigation, Power, Comms) plus `Core`; access derives from `Player.station` + `ShipConfig`.
- **Station / Station Rating** — the authoritative seat (`StationId`, `[[station]]` in `player_ship.toml`); ratings declare which owned systems run on AI; implicit `Backfill` rating automates everything on disconnect.
- **System** — fine-grained operable unit under a console, stable kebab `SystemId` (coarse / fine / ownerless-capability patterns, pinned by issue #525); every kind registers an AI controller.
- **Control Source** — per-system `Human | Ai | Offline` with `control_tick_policy` (accept human input / operate AI / coordinate).
- **Session / Session Token** — server-side player record keyed by UUIDv4 token in `localStorage`.
- **Lobby / In-Progress Phase**, **Captain**, **Helm Input** (10 Hz `ControlSystem`), **Red Alert** (`red-alert` ownerless system).
- **Hull Integrity** — **per-console** (`ConsoleHull`): shields first, pierce split, random distribution across consoles, ship dies when all consoles hit 0.
- **Repair** — `DispatchRepairTeam { team_idx, target }` to one of three repair teams (travel → repair at HP/sec → return), timings from `[repair]` TOML. Shape-matching breakdown puzzle deleted.
- **Modifier / Flag** — slot-keyed multiplier cache; OR-aggregated typed booleans (`CommsJammed`, `SensorBlind`).
- **Power Group / Power Allocation** — `PowerGroupId` buckets, `SetPowerGroupAllocation { group, level }`, data-driven from `[power]` TOML (capacity, rates, emergency threshold); `PowerBlackboard` publishes raw truth.
- **World File / World / Region / Entity Snapshot / Asteroid Window / World Data / Entity Config** — the unified TOML world pipeline (unchanged from PRD #153/#191/#342 definitions; `AsteroidWindow` now lives in `src/asteroids/lifecycle.rs`).
- **Console Complexity** — marked *legacy*: superseded by per-system Control Sources; vestigial `complexity` wire field remains, don't build on it.
- **Sensors / Shields / Navigation / Comms consoles** — the Science split trio + comms inbox (client state now `gui/comms-state.js`).

## Architecture terms defined

- **ControlSystem (wire)** — `ClientMessage::ControlSystem { target: SystemId, payload: SystemControlPayload }`; humans and AI issue identical commands.
- **AdmittedCommand** — post-admission command with source identity stripped (`response_token` only for reply routing).
- **Blackboard** — per-system `SystemBlackboard` published into `ShipSystemBlackboards` during `SimSet::Publish`; aggregators read in `PublishAggregate`; `FrozenBlackboards` = last tick's snapshot for deterministic cross-system reads (Channel 1, dirty-tracked sync per #557).
- **Coordination Lag** — `CoordinationEnqueue` → `CoordinationLagQueue` → `process_coordination_lag`; `DeliverAction` resolves at delivery time (issue #493).
- **LocalShip** — marker for the crew's ship, used only for viewscreen/local scoping; NPC and player ships otherwise share code paths (PRD #597).
- **LobbyHandlerResult**, **radar_dots**, **Broadcaster / LobbyBroadcaster / SimBroadcaster / Audience / Cadence**, **States\<GamePhase\>**, **WorldPlugin** — unchanged seams.
- **Console Plugin (server)** — Bevy plugin at `src/console/<name>/server.rs`; client side is an HTML iframe (`gui/<name>-console.html`), no client Rust.
- **View-Model** — server `GameState` (in `GameStateCache`); client-side pure JS (`gui/lobby-state.js`, `gui/sim-state.js`, `build*()` in `gui/console-state.js`).
- **Client Sim State (JS)** / **Active Console** — the JS replacements for the deleted Rust `ClientSimState` / `ActiveConsole` resources.

## Why this is load-bearing

Naming drift is the silent killer of long-lived projects. If "session" and "player" become interchangeable, every code review and PRD review loses time disambiguating. The wiki adopts these names as primary headings — every entity/concept page maps to a `CONTEXT.md` term where one exists.

## Cross-references

- [Station](../entities/station.md), [System](../entities/system.md), [Console](../entities/console.md), [Client Architecture](../concepts/client-architecture.md), [Broadcaster Seam](../concepts/broadcaster-seam.md), [Coarse-system migration](../concepts/coarse-system-migration.md).
