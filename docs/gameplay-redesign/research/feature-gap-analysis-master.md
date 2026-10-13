# Project Phoenix — Master Feature Gap Analysis

> **Synthesised from three analyses:** *vs. The Mandate* (Kickstarter campaign), *vs. 8 bridge-sim peers* (Artemis, Empty Epsilon, Thorium, Jump Space, PULSAR, SCOS, SpaceBourne 2, STO), and *vs. 13 broader space titles* (Bridge Crew, Horizons, SNIS, Void Crew, Bridge Commander, STSA, Starship Simulator, BC/UC trilogy, CC2, Nebulous, Objects in Space, Falling Frontier, Starsector).
>
> **Scope constraints in force:** No first-person / on-foot gameplay · No GM role · Browser-based WASM · No MMORPG persistence or sharding.

---

## 1. Project Phoenix — Authoritative Baseline

| Dimension | Current State |
|---|---|
| Platform | Browser (WASM) + phone clients via WebRTC |
| Multiplayer | Co-op, star topology, 1 viewscreen + N phone consoles |
| Player count | ~5–9 (one per station) |
| Stations | Captain, Helm, Tactical, Repair, Power, Sensors, Shields, Navigation, Comms |
| Combat | Phasers (port/starboard arc, auto/manual), torpedoes (3 homing tubes) |
| Damage | Hull HP, four-quadrant shields, breakdown queue → shape-match repair mini-game |
| Power | 6+2 points distributed across consoles; auxiliary battery system |
| Navigation | Thrust + yaw joystick; impulse burst; helm radar |
| Sensors | Three radar overlays (short / long / tactical); region flags (blind/jam) |
| World | Procedural asteroid grid; static entities (planets, stations, AI ships); region zones |
| Scenarios | TOML-driven: entities, objectives, triggers, comms exchanges |
| AI | Console AI (low-complexity auto-play); NPC ship AI in progress |
| Progression | Session-scoped; no persistent meta-progression |
| Economy | None |

---

## 2. Phoenix Competitive Advantages (What competitors lack)

These are strengths to preserve and build on, not gaps.

| Advantage | vs. |
|---|---|
| Zero-install browser play on any device | All PC-native titles |
| Phone-as-console with QR onboarding | All competitors |
| Shape-match repair mini-game (tactile, asymmetric) | All competitors |
| Breakdown queue with shape decoys (scalable difficulty) | All competitors |
| Spectator queue + auto-shuffle crew assignment | All competitors |
| Four-quadrant shield focus arc | All except Nebulous |
| Low-complexity AI delegation preset (console AI) | All competitors |
| WebRTC P2P — no server cost, no port forwarding | Starship Horizons, SNIS |
| Moddable via TOML scenario files (no code) | Most competitors |

---

## 3. Master Gap Register

Impact scale: ★★★★★ = game-changing · ★ = minor polish
Ease scale: ★★★★★ = hours · ★ = months
Source abbreviations: `[A]` Artemis · `[EE]` Empty Epsilon · `[T]` Thorium · `[JS]` Jump Space · `[PL]` PULSAR · `[SC]` SCOS · `[STO]` STO · `[M]` The Mandate · `[BC]` Bridge Crew · `[NS]` Nebulous · `[OiS]` Objects in Space · `[FF]` Falling Frontier · `[SS]` Starsector

---

### Tier 1 — Critical: game is significantly poorer without these

| # | Feature | Impact | Ease | Sources | Notes |
|---|---|---|---|---|---|
| 1 | **Granular subsystem damage** | ★★★★★ | ★★ | A/EE/PL/STO/NS/FF/M | The single biggest Engineering depth upgrade. Each subsystem (shields, weapons, engines, sensors, warp) has independent health 0–100%. At 0% capability is lost; Engineering must target specific repairs. Extends existing `HullIntegrity` + `damage.rs`. New `SubsystemHealth` struct; damage routing by hit context. Appears in every source. |
| 2 | **NPC enemy ships with basic AI** | ★★★★★ | ★★★ | ALL | Without enemies, Tactical shoots asteroids, Engineering has no urgency, Comms has no one to hail. NPC ship AI is noted as "in progress." Start with `roam` + `attack-nearest` state machine; builds on existing `simulation.rs` ship physics. |
| 3 | **Win / lose conditions** | ★★★★★ | ★★★★ | ALL | Nothing ends the game. Even "survive 10 waves" transforms the experience. New `GamePhase::GameOver(Reason)` variant broadcast when condition met. Trivial once enemy ships and station health exist. |
| 4 | **Wave-based enemy spawning** | ★★★★ | ★★★★ | A/EE/JS/PL/STO | Escalating waves with a countdown between them. One timer + spawn table in `simulation.rs`. Creates a tension arc with no scripting overhead. |
| 5 | **Multiple ship classes / loadouts** | ★★★★ | ★★★ | A/PL/SC/STO/M/STSA | Currently one fixed ship. Lobby selection of `ShipConfig { weapon_tubes, max_shield_facings, base_speed, hull_max }`. Aligns with existing TOML config system; extend to support loadout presets. Appears in 6+ sources. |
| 6 | **Space stations (functional, attackable) + docking** | ★★★★★ | ★★★ | A/EE/PL/SC/STO/M | Stations are the anchor of the game loop: enemies attack them, players dock for repair/rearm, station destruction is a loss condition. Stations already exist as static entities. New: `StationInfo { hp, faction }` + `ShipStatus::Docked(station_uuid)`; physics zeroed while docked; hull + torpedoes restored on dock. Comms pre-hail halves resupply time. |
| 7 | **Science console — sensor scanning** | ★★★★ | ★★★ | A/PL/STO/T/M | Level 1 scan: enemy class + shield strength. Level 2 deep scan: shield/beam frequency weakness + subsystem damage. New `ScanResult { depth, shield_freq_weakness, subsystem_damage }`. Enables the beam-frequency loop and unlocks Naval Intelligence reports. |
| 8 | **Faction reputation / alignment system** | ★★★★ | ★★ | A/EE/PL/SB/STO/SS/OiS | Track player actions against scenario factions. Affects NPC reactions, trade prices, mission gating. New `FactionStanding { faction_id, standing: f32 }` per session (or persistent). Appears in 7+ sources. |
| 9 | **Small-crew / solo scaling (AI crewmates)** | ★★★★ | ★★ | VoidCrew/OiS/BC | Critical for pick-up groups. AI auto-covers unmanned stations — extends existing console AI preset. Graceful degradation from 9 players down to 1–3. |

---

### Tier 2 — High impact, medium effort: natural next PRDs

| # | Feature | Impact | Ease | Sources | Notes |
|---|---|---|---|---|---|
| 10 | **Beam / shield frequency system** | ★★★★ | ★★★ | A/PL/STO/M | Science level-2 scan reveals enemy's weakest shield frequency. Tactical tunes phasers to match for a damage multiplier. Torpedoes unaffected. Creates a Science → Tactical cross-console loop. New `ShieldFrequency` in `ship_state.rs`; frequency-based modifier in `damage.rs`. |
| 11 | **Warp / FTL drive** | ★★★★ | ★★★ | ALL | Helm warp-factor slider (0–4). Physics velocity multiplied by factor × power allocation; energy drains proportionally. Expands play area to multiple sectors. New `warp_factor: u8` in ship state. |
| 12 | **Sector traversal + exploration** | ★★★★ | ★★ | PL/SB/STO/Horizons/FF | Currently a single flat play area. New `SectorMap { nodes, edges }`: each sector has hazards, enemies, stations, or points of interest. Different sector types: controlled, nebula, uncharted, enemy. Unlocks galaxy-scale structure. |
| 13 | **Mission / objective system (scripted)** | ★★★★★ | ★★ | ALL | TOML scenario system exists but needs a full trigger/event/outcome framework: `on_time`, `on_destroyed`, `on_docked` triggers; NPC spawn waves; narrative text; branching win/lose conditions. The difference between a sandbox and an experience. |
| 14 | **Torpedo warhead variants** | ★★★★ | ★★★ | ALL | Currently homing only. Add: **Nuke** (area blast), **ECM** (halves shields on detonation), **Mine** (proximity trigger), **EMP** (disables subsystems). Each is a warhead variant on the existing projectile entity. |
| 15 | **Cloaking system (player-controlled)** | ★★★★ | ★★★ | PL/STO/T/OiS | New `CloakState { active, detection_probability }`; power drain while cloaked; firing decloaks. Thorium variant: crew must go "silent" (reducing all console activity) to stay hidden — drama happens in the room. |
| 16 | **Electronic warfare basics (sensor jamming)** | ★★★ | ★★★ | NS/OiS/T | Sensor jamming and ECCM. Region flags for blind/jam already exist — expose as an active Tactical or Sensors station action against enemy entities. New `ActiveJam { freq, power, target_type }`. |
| 17 | **Ship component / upgrade system** | ★★★★ | ★★ | JS/PL/STO/VoidCrew/FF/SS | 5–7 component slots with tier upgrades (Mk 1–3). Components drop from destroyed enemies; hangar refit phase between missions. New `ShipComponents { engine, reactor, shields, weapons, sensors: ComponentTier }`. Creates the inter-mission loop. |
| 18 | **Enemy factions with distinct behaviour** | ★★★★ | ★★ | A/JS/PL/STO | Artemis: Kraliens (formations), Arvonians (carriers), Torgoth (shielded/huge/drones), Skaraan (cloak). PULSAR: Sylvassi, W.D. Corp, rogue AIs. Each faction has unique weapons and AI branches. Requires multiple NPC entity types. |
| 19 | **Captain status dashboard** | ★★★ | ★★★★ | T/SC | Aggregates speed, destination, current target, alert condition, crew count, damaged systems from existing `SimSnapshot` fields. Dramatically improves command ergonomics. Straightforward to build once subsystem data exists. |
| 20 | **5-level alert condition** | ★★★★ | ★★★★★ | T/SC | Extending existing `red_alert: bool` to `condition: u8` (Condition 5 = normal → 1 = battle stations) is a near-trivial change. Each level adjusts visual/audio state and power allocation. |
| 21 | **Jump drive (strategic FTL)** | ★★★ | ★★★ | JS/PL | Select destination, set charge; Engineering shortens countdown. Instant teleport with high crew coordination drama. New `JumpDriveState { charge_level, destination }`. |
| 22 | **Named NPC crew characters** | ★★★ | ★★★ | BC/UC/FF/BridgeCommander/M | Start cosmetic: name + portrait on viewscreen. Then functional: stat bonuses per console, personality flags that affect Comms options. |
| 23 | **Inter-mission loadout selection** | ★★★★ | ★★★ | VoidCrew/SS/FF/M | Pick torpedo type, phaser frequency, shield configuration before a run. Extends TOML config to loadout presets. Adds pre-mission strategy without runtime complexity. |
| 24 | **Phaser charging as a split task** | ★★★★ | ★★★ | T | Thorium's most innovative Tactical design: charging and firing are two separate sub-tasks on one console. One player charges the bank (watching heat); another fires at locked target. Firing heats the bank 50%; overheated cannot fire. New `phaser_charge` and `phaser_heat` per bank. Turns Tactical into a two-player station. |
| 25 | **Comms console suite: interception, jammer, cyphers** | ★★★ | ★★★ | T | Three Thorium-originated mini-games that make Comms a skill role: (a) frequency-dial interception of incoming enemy transmissions; (b) signal jammer (select freq + power level to jam torpedo guidance or enemy comms); (c) code cypher decryption (character-substitution puzzle). Drama happens in the room. |
| 26 | **Procedural mission generation** | ★★★★ | ★★ | BC/SS | Extend scenario system to generate encounter seeds. Reduces content-authoring burden and creates replayability without a full campaign. |

---

### Tier 3 — High payoff, substantial effort

| # | Feature | Impact | Ease | Sources | Notes |
|---|---|---|---|---|---|
| 27 | **Engineering: overcharge + heat + reactor control** | ★★★ | ★★★ | A/T | Two layered additions: (a) overcharge — power above 100% boosts a system but generates heat; coolant prevents overheat; (b) Reactor Control — a separate card from the distribution panel; Engineer can overload the reactor for more output or shut it down entirely. Gives Engineering a third layer of complexity. |
| 28 | **Damage reports (sequential steps + reactivation codes)** | ★★★ | ★★★ | T | Replace "press repair" with a theatrical workflow: Engineering receives a damage report listing sequential physical steps, executes each, enters a reactivation code. More dramatic than a cooldown timer; encourages cross-console communication. |
| 29 | **DamCon teams (visual ship-schematic repair)** | ★★★ | ★★ | A | 6-person teams dispatched through a 2D ship schematic to damaged nodes. Teams can suffer casualties. Requires a schematic layout per ship class + team pathfinding. |
| 30 | **Roguelite meta-progression between sessions** | ★★★★ | ★ | VoidCrew/PL/STO/AllianceSG | Persistent unlocks and upgrades per run. High effort — requires save/load + server state persistence. Highest replay value of any single feature. |
| 31 | **Economy / cargo / trading system** | ★★★ | ★★ | PL/SB/STO/BC/UC/SS | Cargo holds, commodity trading between stations, fluctuating prices. New `Cargo { items }` ship state; Comms manages trades. |
| 32 | **Branching diplomatic / narrative outcomes** | ★★★ | ★★ | Starship Simulator/STO/SC | Decision tree outcomes from Comms interactions — not just scripted responses but branching consequences affecting faction standing and mission gating. |
| 33 | **Proactive maintenance / time-based degradation** | ★★★ | ★★ | Starship Simulator | Systems degrade over time unless maintained — transforms Engineering from reactive (repair after damage) to proactive (prevent failure). |
| 34 | **Sector conquest / persistent territory meta** | ★★★★ | ★ | CC2/SS | Persistent conquest layer as a meta-game objective. Very high effort; consider as a long-term campaign mode. |
| 35 | **Multi-ship fleet management** | ★★★★★ | ★ | NS/CC2/A/EE | Two or more crews in one battle. Massive architectural change to `SimSnapshot` and message routing. High ambition, low near-term feasibility. |

---

### Tier 4 — Good additions, lower effort

| # | Feature | Impact | Ease | Sources | Notes |
|---|---|---|---|---|---|
| 36 | **Tutorial / training mission** | ★★★ | ★★★★ | STSA | TOML scenario can be authored now — just needs a dedicated onboarding scenario with step-by-step objectives. |
| 37 | **Self-destruct** | ★★★ | ★★★★ | T/SC | Captain sets a countdown; `GamePhase::GameOver(SelfDestruct)` fires. Dramatic resolution for hopeless situations. |
| 38 | **Railgun (point defence)** | ★★★★ | ★★★ | T | Click-to-fire defensive weapon that destroys incoming torpedoes and drones. Player clicks a sensor grid; shots that intersect an incoming projectile enough times destroy it. Creates active skill-based defence. |
| 39 | **Nebulae / hazard fields** | ★★★ | ★★★ | A/EE/PL/STO | Radial zones: radar range halves inside; FTL capped; possible DoT. New `NebulaInfo` entity. One of the few environmental features absent from Phoenix regions. |
| 40 | **Terrain-as-strategy (cover, ambush positions)** | ★★★ | ★★★ | FF/NS | Use nebulae and asteroid fields tactically for sensor cover and ambush positioning. Primarily a design/AI behaviour change on top of existing regions. |
| 41 | **Black holes / singularities** | ★★★ | ★★★★ | A/EE | If `dist < event_horizon_radius` → instant kill. Reuses the asteroid collision pipeline. Memorable navigational hazard. |
| 42 | **Wormholes** | ★★ | ★★★ | EE | Bidirectional portal pairs. Positional check → teleport to exit. Memorable navigation moment. |
| 43 | **Ship reverse throttle** | ★★ | ★★★★★ | A | Remove the `thrust.max(0.0)` clamp in `ship_physics.rs`. Disable warp while reversing. Near-trivial change. |
| 44 | **Observer / spectator console** | ★★ | ★★★★★ | A | Add `Observer` to `Console` enum; receives `SimSnapshot` but sends nothing back. Useful for waiting players and demos. |
| 45 | **Ship name customisation** | ★★ | ★★★★ | A/PL | New `ClientMessage::SetShipName(String)` in lobby → broadcast `ServerMessage::ShipRenamed`. Instant roleplay texture. |
| 46 | **Radar zoom levels / modes** | ★★★ | ★★★★★ | A/PL | 4 zoom levels + modes (Long-Range, Short-Range, Tactical). Pure client-side scale change in `client_helm.rs`. |
| 47 | **Nav beacons / waypoints** | ★★ | ★★★★★ | STO | Captain sets waypoints → visible on Helm radar. New `Waypoint { pos, label }`. |
| 48 | **Fleet / wingman commands** | ★★★ | ★★ | PL/STO | Captain or Tactical controls allied NPC escort behaviour: `FleetCommand { target, formation, stance }`. |
| 49 | **Engineering preset manager** | ★★ | ★★★★★ | T | Store and recall up to 10 power + coolant configurations. Pure UI feature. Trivial once power system exists. |
| 50 | **Carrier ships launching fighters** | ★★★ | ★★ | A/EE | Enemy carriers spawn fighter squadrons — fast, fragile, numerous. New `spawn_child_entity` mechanism + fighter entity with simple chase AI. |
| 51 | **Cloaking enemies** | ★★★ | ★★★ | A | Enemy disappears from radar for 60 seconds. Comms jammer at the right frequency forces uncloak. |
| 52 | **Mines (player-deployed)** | ★★★ | ★★★ | A | Tactical deploys proximity mines via torpedo tube. Static entity detonates when any ship enters radius. |
| 53 | **Auxiliary / secondary viewscreen outputs** | ★★★ | ★★★ | Horizons | Additional display outputs (alert status, shields, galactic map on secondary screens). |
| 54 | **Random encounter system** | ★★★ | ★★★ | SB | Events during warp traversal: distress calls, ambushes, hailing. New `EncounterTable` with weighted event types. |
| 55 | **Surrender mechanic** | ★★ | ★★★ | A | Comms requests enemy surrender; probability based on hull integrity + faction personality. Surrendered ships go yellow on radar. |
| 56 | **Officer / captain's log** | ★★ | ★★★★★ | T/STO | Captain and officers record log entries mid-mission. Server-pushed `LogEntry { text }`. Narrative flavour. |
| 57 | **Life support / fuel as managed subsystems** | ★★ | ★★★ | OiS/AllianceSG/M | Track life support atmosphere and fuel consumption (especially during impulse/warp use). Engineering manages via console. New fields in `ShipState`; consumption in `simulation.rs`. Nice-to-have for immersion. |
| 58 | **Probe network (deployed probe map)** | ★★★ | ★★★ | T/EE | Visual sector map of deployed probes. Science "links" a probe and sees through its sensors. Extends radar coverage. |
| 59 | **Anomalies (energy pickups)** | ★★★ | ★★★★ | A | Science detects energy-rift entities on long-range radar; flying through recharges energy pool. Simple sphere entity. Gives Science a job before full scanning. |
| 60 | **In-game scenario / mission editor** | ★★★ | ★★ | BridgeCommander | UI wrapper over the TOML scenario system for authoring missions in-game. Reduces barrier for community content. |

---

### Out of Scope (consistent across all analyses)

First-person / on-foot / EVA gameplay · MMORPG persistence or sharding · Orbital mechanics simulation · Ground combat / planetary landing · 3D vertical movement (pitch axis) — complicates all radar, AI targeting, and physics, not recommended · PvP / competitive multiplayer · Real astrophysics · Boarding parties (could be abstracted to a Comms mini-game in future) · Large Steam Workshop modding pipeline.

---

## 4. Console-by-Console Gap Mapping

### Captain Console
Status dashboard (T15) · 5-level Alert Condition (T20) · Self-destruct (T37) · Multiple viewscreen perspectives · Nav beacon / waypoint setting (T47) · Officer log (T56) · Fleet/wingman commands (T48)

### Helm Console
Warp / FTL drive with factor slider (T11) · Jump drive (T21) · Precision thrusters / fine maneuvering (T) · Navigation coordinate transcription mini-game (T — deliberate human-error mechanic) · Ship reverse throttle (T43) · Radar zoom levels (T46) · Nav beacons visible (T47) · Docking initiation (T6)

### Tactical Console
Granular subsystem damage routing (T1) · Subsystem targeting (active targeting of enemy subsystems) (T1) · Beam / shield frequency tuning (T10) · Torpedo warhead variants (T14) · Phaser charge + fire split (T24) · Railgun point defence (T38) · Player-deployed mines (T52) · Fleet / wingman commands (T48) · Multiple weapon system types (T) · Auto-turret priority management

### Engineering Console
Power distribution sliders, 4–6 ways (T6 existing, extend) · Per-subsystem repair targeting (T1) · Overcharge + heat management (T27) · Reactor Control (separate card) (T27) · Damage reports with sequential steps + reactivation codes (T28) · DamCon teams / schematic repair (T29) · Exocomps (pre-planned repair robots) · Emergency battery / backup power · Engineering preset manager (T49)

### Science Console (existing Sensors role, significantly expanded)
Two-level sensor scanning — class + shield frequency + subsystem damage (T7) · Anomaly detection (T59) · Environmental hazard analysis · Probe Construction and Network (T58) · Beam frequency weakness identification (T10) · Naval intelligence report generation · Library card / in-game database

### Comms Console
Short-range frequency-based hailing (T8) · Hail neutral ships for energy/trade · Interception frequency-dial mini-game (T25) · Signal Jammer (T25) · Code Cyphers decryption puzzle (T25) · Long-range email / satellite network · Surrender negotiation (T55) · Enemy faction-specific taunts · Faction standing management (T8)

---

## 5. Implementation Roadmap

### Phase 1 — Make the game complete (Tier 1 core)
*Estimated: 8–12 weeks*

1. **Enemy ships + basic AI** — state machine (`roam`, `attack-nearest`), builds on `simulation.rs`
2. **Win / lose conditions** — `GamePhase::GameOver(Reason)`; "survive N waves" is sufficient to ship
3. **Wave-based spawning** — one timer + spawn table
4. **Granular subsystem damage** — new `SubsystemHealth` struct; extend `damage.rs` and `breakdown.rs`
5. **Functional stations + docking** — extend existing station entities; `ShipStatus::Docked`; resupply trigger

### Phase 2 — Tactical depth and cross-console play (Tier 1–2)
*Estimated: 8–10 weeks*

6. **Multiple ship classes** — `ShipConfig` in lobby; TOML loadout presets
7. **Shield / beam frequency system** — `ShieldFrequency` in `ship_state.rs`; damage modifier in `damage.rs`; Science + Tactical console updates
8. **Science console scanning** — `ScanResult { depth, shield_freq_weakness, subsystem_damage }`; new Science role
9. **Torpedo warhead variants** — Nuke, ECM, Mine, EMP warhead types on existing projectile entity
10. **5-level alert condition** — extend `red_alert: bool` to `condition: u8`

### Phase 3 — World, factions, and the game loop (Tier 1–2)
*Estimated: 10–14 weeks*

11. **Faction reputation system** — `FactionStanding` per session; affects NPC reactions and mission gating
12. **Warp / FTL drive** — `warp_factor` in ship state; power drain
13. **Sector traversal** — `SectorMap { nodes, edges }`; different sector types
14. **Mission / objective system** — extended trigger/event framework on existing TOML scenarios
15. **Small-crew / solo scaling** — extend console AI to cover unmanned stations

### Phase 4 — Depth, polish, and replayability (Tier 2–3)
*Estimated: 12–16 weeks*

16. **Ship component / upgrade system + drydock phase** — `ShipComponents`; loot drops; hangar refit
17. **Inter-mission loadout selection** — loadout preset picker in lobby
18. **Enemy factions with distinct behaviour** — multiple NPC entity types and AI branches
19. **Cloaking system** — `CloakState`; power drain; fire-to-decloak
20. **Electronic warfare** — active jamming as station action; ECCM
21. **Engineering: overcharge + heat + reactor control**
22. **Comms mini-game suite** — interception, jammer, code cyphers
23. **Tutorial / training mission** — TOML scenario with step-by-step onboarding objectives

### Phase 5 — Long-term expansions (Tier 3)
- Roguelite meta-progression · Economy and trading · Branching narrative outcomes · Proactive maintenance / degradation · DamCon teams · Procedural mission generation

---

## 6. Recommended Quick Wins (high signal / low cost)

For near-term momentum with minimal engineering risk:

| Feature | Effort | Why now |
|---|---|---|
| Ship reverse throttle | Hours | One `max(0.0)` clamp removal in `ship_physics.rs` |
| Observer / spectator console | Hours | One enum variant; no simulation changes |
| Ship name customisation | Hours | One new `ClientMessage`; cosmetic |
| Radar zoom levels | Hours | Client-side scale only |
| Nav waypoints (Captain sets) | 1 day | New `Waypoint` entity; visible on Helm |
| 5-level alert condition | 1 day | Extend `bool` to `u8` |
| Engineering preset manager | 1–2 days | Pure UI once power system exists |
| Tutorial / training scenario | 2–3 days | Author in TOML; no new systems needed |
| Anomalies (energy pickups) | 2–3 days | Simple sphere entity; no damage geometry |
| Captain status dashboard | 3–4 days | Reads existing `SimSnapshot` fields |
| Black holes / singularities | 3–4 days | Reuses asteroid collision pipeline |

---

## 7. Key Design Principles from the Analysis

**Cross-console loops are the core value.** The highest-impact features (beam frequency, sensor scanning, signal jamming, Comms cyphers) all create work that crosses console boundaries. A Science player scans → tells Tactical the frequency → Comms jams the shields → Tactical fires. This is the mechanic that no single-player space game can replicate and should be the filter for feature prioritisation.

**Task-splitting (Thorium's principle) scales the crew.** Splitting Tactical into charge + fire, and Engineering into Reactor + Distribution, means a 7-player crew has genuinely different roles from a 4-player crew — not just the same roles with more people watching. Design new features with a "can this be split?" question.

**Deliberate human-error mechanics suit a party game.** Navigation coordinate transcription, damage-report reactivation codes, and code cyphers all derive their drama from human mistakes and verbal communication. These mechanics work better in a room than on a screen. Prioritise them over automated systems.

**The game loop is currently open-ended.** Win/lose conditions and wave spawning are not polish — they are the minimum viable game. Everything in Tier 1 should be treated as structural work, not features.
