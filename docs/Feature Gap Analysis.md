# Feature Gap Analysis — Artemis SBS, Empty Epsilon, Thorium

---

## All net-new features, ranked by Impact × Ease

**Sources:** `[A]` = Artemis SBS · `[EE]` = Empty Epsilon · `[T]` = Thorium  
**Impact:** ★★★★★ = game-changing · ★ = minor polish  
**Ease:** ★★★★★ = hours of work · ★ = months of work

---

### Tier 1 — Critical gaps; game is incomplete without these

| # | Feature | Src | Impact | Ease | Notes |
|---|---|---|---|---|---|
| 1 | **Enemy ships with basic AI** | A/EE | ★★★★★ | ★★★ | THE critical missing feature. Without enemies Tactical shoots asteroids, Engineering has no urgency, Comms has no one to taunt. Start with one type: `roam` + `attack-nearest` behaviours. Builds directly on existing ship physics in `simulation.rs`. New `NpcShip` entity; simple state machine; NPC beam-fire reusing `damage.rs`. |
| 2 | **Win/lose conditions** | A/EE | ★★★★★ | ★★★★ | Nothing ends the game. Even "survive 10 waves" transforms the experience. New `GamePhase::GameOver(Reason)` variant; broadcast once condition met. Trivial once enemies + stations exist. |
| 3 | **Wave-based enemy spawning** | A/EE | ★★★★ | ★★★★ | Escalating waves with a countdown between them. One timer + spawn table in `simulation.rs`. Creates a tension arc with no scripting system needed. |
| 4 | **Shared ship energy pool** | A/EE/T | ★★★★★ | ★★★ | Every action drains a shared `energy: f32`. Impulse, warp, beam fire, and shields all consume it. Engineering manages generation. Without this, Engineering is just a repair button; with it, every console competes for the same resource. New field in ship state; consumption functions per action. Draft 5's power sliders assume this exists. |
| 5 | **Alert Condition (5 levels)** | T | ★★★★ | ★★★★★ | Thorium's 5-level system (Condition 5 = normal → Condition 1 = battle stations) replaces the current binary Red Alert toggle. Each level triggers different visual/audio states. Extending the existing `red_alert: bool` to a `condition: u8` enum is trivial. The 5-level model gives the Captain meaningful escalation rather than an on/off toggle, and creates crew communication ("Captain, I recommend Condition 3"). |
| 6 | **Anomalies (energy pickups)** | A | ★★★ | ★★★★ | Science detects energy-rift entities on long-range radar; flying through one recharges the ship's energy pool. Simple sphere entity; no damage geometry needed. Gives Science a real job before full enemy scanning exists and creates "chase anomaly vs hold position" decisions. |
| 7 | **Black holes / singularities** | A/EE | ★★★ | ★★★★ | If `dist < event_horizon_radius` → instant kill. Reuses the asteroid collision pipeline; no new physics geometry. Memorable navigational hazard; forces crew to communicate danger. |
| 8 | **Nebulae** | A/EE | ★★★ | ★★★★ | Radial zone: ships inside are invisible to outside long-range radar; radar range halves inside; FTL capped at factor 1. One new `NebulaInfo` entity; a flag in the `radar_dots` iterator. Atmospheric and tactically significant. |
| 9 | **Ship reverse throttle** | A | ★★ | ★★★★★ | Remove the `thrust.max(0.0)` clamp in `ship_physics.rs`. Disable warp while reversing. One-line change. |
| 10 | **Observer / spectator console** | A | ★★ | ★★★★★ | Add `Observer` to the `Console` enum; it receives `SimSnapshot` but sends nothing back. Good for waiting players and demos. |
| 11 | **Ship name customisation (Helm, lobby)** | A | ★★ | ★★★★ | New `ClientMessage::SetShipName(String)` in lobby → broadcast `ServerMessage::ShipRenamed`. Adds roleplay texture instantly. |
| 12 | **Status dashboard card (Captain)** | T | ★★★ | ★★★★ | Thorium's Status card aggregates speed, destination, current target, crew count, radiation levels, coolant, damaged systems, alert condition, and ship image into one Captain-facing view. Currently the Captain only sees Red Alert + view selector. Pulling aggregated state from the existing `SimSnapshot` fields into a readable dashboard is straightforward. Dramatically improves command ergonomics. |
| 13 | **Self-destruct** | T | ★★★ | ★★★★ | Captain sets a countdown (hours:minutes:seconds). New `SelfDestruct { armed: bool, countdown_secs: u32 }` in server state; when zero, `GamePhase::GameOver(SelfDestruct)` fires and all client screens black out. Creates a dramatic resolution for hopeless situations. Easy to add; high emotional payoff. |

---

### Tier 2 — High impact, medium effort; natural next PRDs

| # | Feature | Src | Impact | Ease | Notes |
|---|---|---|---|---|---|
| 14 | **Space stations — functional, attackable** | A/EE | ★★★★★ | ★★★ | Draft 6 is a stub. Stations are the anchor of the whole game loop: enemies attack them, players dock for repair/rearm, their destruction is a loss condition. New `StationInfo { uuid, pos, hp, faction }` entity; dockable-range detection; resupply trigger on dock. |
| 15 | **Docking mechanic** | A/EE/T | ★★★★ | ★★★ | Helm initiates dock when within range. Docked state: hull restored, torpedoes refilled, DamCon teams topped up. Comms pre-hail ("stand by for docking") halves resupply time. New `ShipStatus::Docked(station_uuid)`; physics zeroed while docked. |
| 16 | **Warp / FTL drive** | A/EE/T | ★★★★ | ★★★ | Helm gets a warp factor slider (0–4). Physics velocity multiplied by warp factor × power allocation; energy drains proportionally. Expands the play area and gives Engineering a second major power sink. New `warp_factor: u8` in ship state; `ShipPhysicsConfig.warp_speed_base`. Thorium also has Warp 1–9.54 with separate heat tracks from impulse. |
| 17 | **Jump drive (alternative FTL)** | A/EE/T | ★★★ | ★★★ | Set direction + distance on Helm; countdown begins (Engineering power allocation shortens it); instant teleport. High drama, high crew coordination. New `JumpDriveState { direction, distance, countdown }`. |
| 18 | **Comms: short-range radio** | A/T | ★★★★ | ★★★ | Thorium's Short Range Comm card: hail ships and stations by frequency; support multiple simultaneous conversations (conference calls); switch between them. The Flight Director (or server logic) mediates which calls connect. Far richer than Artemis's fixed three-taunt system. Hailing a station on the correct frequency and relaying the reply verbally drives real table communication. Draft 8 fleshes out with this concrete mechanic. |
| 19 | **Comms: long-range email** | T | ★★★ | ★★★ | Thorium's Long Range Comm: compose messages addressed to distant entities, queue them, select a satellite (signal strength affects transmission speed), watch the dot travel to the destination. Creates deliberate delay and pacing in Comms' narrative work. Very different feel from the instant hail model. |
| 20 | **Friendly NPC vessels** | A/EE | ★★★★ | ★★★ | Transports and escorts patrol between stations. Enemies target them; Comms redirects them; their destruction adds loss-condition weight. Simple `orderFlyTowards / orderDefendLocation` state machine. Adds life to the world and moral pressure without complex AI. |
| 21 | **Multiple ship classes** | A | ★★★ | ★★★ | Scout (fast, weak), Light Cruiser (balanced), Missile Cruiser (4 tubes, no beams), Battleship (4 beams, slow), Dreadnought (aft beam emitter). Different stat configs per class chosen in lobby. Draft 1's entity config system supports this pattern directly. |
| 22 | **Multiple torpedo types** | A/EE | ★★★★ | ★★★ | Draft 4 designs homing torpedoes only. Add: **Nuke** (area blast ~1000u, 200 dmg), **ECM** (halves all shields in range on detonation), **Mine** (deploys at current position behind ship, proximity trigger). Each is a warhead variant on the same homing-projectile entity. Triples Tactical options. |
| 23 | **Shields: raise/lower + energy drain** | A/EE/T | ★★★★ | ★★★ | Draft 4 has the structural design. The key mechanic not yet specified: shields consume energy while raised, creating a tradeoff against speed and weapons. Both Weapons and Helm have the toggle (Artemis model). Thorium adds per-sector frequency adjustment (100–350 MHz) that interacts with the beam frequency mechanic (#27). |
| 24 | **Phaser charging as a split task** | T | ★★★★ | ★★★ | Thorium's most innovative Tactical design: **charging** and **firing** are two separate cards, intended for two separate people. One officer charges the phaser bank (building charge, watching heat); another fires at the locked target. Firing heats the bank at 50%; an overheated bank cannot fire. For Project Phoenix this could be a second tab on Tactical rather than a separate console. New `phaser_charge: f32` and `phaser_heat: f32` per bank; charging tick and firing cost functions. |
| 25 | **Railgun (point defence)** | T | ★★★★ | ★★★ | Thorium-unique: a click-to-fire defensive weapon that destroys incoming torpedoes and drones before impact. Player clicks on a sensor grid; a shot appears at the clicked position; if it intersects an incoming projectile enough times, the projectile is destroyed. Creates an active skill-based defence mini-game during combat rather than passive shield absorption. New `railgun_bolts: u32` in ship state; projectile-vs-point collision check. High payoff for a second Tactical player or a dedicated console. |
| 26 | **Science: two-level scan** | A/T | ★★★ | ★★★★ | Level 1 scan reveals enemy class + shield strength. Level 2 deep scan reveals shield/beam frequency weakness. Thorium also surfaces real-time subsystem damage in the scan readout. New `ScanResult { depth, shield_freq_weakness, subsystem_damage }` server message. |
| 27 | **Beam frequency system** | A | ★★★ | ★★★ | 5 selectable beam frequencies on Tactical. Science level-2 scan reveals the enemy's weakest frequency. Matching it multiplies beam damage. Simple `weak_frequency: u8` per enemy; compare to `selected_frequency` on each beam tick. Cross-console loop: Science calls out the frequency, Tactical adjusts. |
| 28 | **Per-subsystem damage (8 systems)** | A/EE/T | ★★★★ | ★★ | Currently only hull integrity exists. The 8 Artemis systems: Beams, Torpedo, Sensors, Maneuver, Impulse, Warp, Front Shield, Rear Shield. Each 0–100%; at 0% that system goes offline. Beams at 80% = 80% fire rate; Maneuver at 50% = 50% yaw rate. New `Subsystems` struct in ship state; damage routing by contact normal on hit. The single biggest Engineering depth upgrade. |
| 29 | **Engineering: overcharge + heat** | A/T | ★★★ | ★★★ | Power above 100% boosts a system (faster beams, more speed, stronger shields) but generates `heat: f32`. At max heat the node is damaged. Coolant (finite pool, redistributable) prevents overheat. Thorium splits this into two tasks: power distribution (dragging bars) and coolant allocation (Coolant Control card). Alongside Draft 5's sliders, makes Engineering genuinely complex. |
| 30 | **Reactor Control** | T | ★★★ | ★★★ | Thorium separates the *reactor* from the *distribution panel*. The reactor generates raw power; engineers can overload it for more output but accelerate heat, or shut it down entirely to stop all heat. The distribution panel then allocates whatever the reactor produces. This split gives Engineering a third layer: Reactor operator manages gross output; Power Distribution operator manages allocation. New `reactor_output: f32`, `reactor_heat: f32`; shutdown and overload modes. |
| 31 | **Damage reports (sequential steps + reactivation codes)** | T | ★★★ | ★★★ | Thorium replaces a simple "press repair" with a theatrical workflow: Engineering receives a damage report (auto-generated or FD-written) listing sequential physical steps; executes each in order; then enters a reactivation code to signal completion. This is more dramatic and harder to rush than the current 30-second cooldown model. New `DamageReport { steps: Vec<String>, reactivation_code: String }` messages from server; client shows a step checklist and a code entry field. Medium effort; high payoff for room atmosphere. |
| 32 | **Stealth Field (player-controlled cloaking)** | T | ★★★ | ★★★ | Thorium-unique player mechanic: activating the stealth field hides the ship from enemy sensors, but every ship system running at high power increases detection probability. A dashboard shows per-system activity levels. Crew must choose: cloak and go silent (cut engines, stop firing) or stay visible. Completely distinct from enemy cloaking (#47). New `stealth_active: bool`; `detection_probability: f32` computed from sum of active system power levels; checked against a threshold each server tick. |
| 33 | **Wormholes** | EE | ★★ | ★★★ | Bidirectional portal pairs. Positional check → teleport to exit. Tactical shortcut tool; memorable navigation moment. |
| 34 | **Enemy factions with personality** | A | ★★★★ | ★★ | Once basic enemy AI exists: Kraliens attack in formation (wingmen join any ship under fire), Arvonians use carrier-launched fast fighters, Torgoth are huge/slow/shielded and fire drones, Skaraan use elite abilities (cloak, jump bursts). Each faction changes how the crew must respond. Requires multiple entity types + faction-specific AI branches. |
| 35 | **Interception (Comms mini-game)** | T | ★★★ | ★★★ | Thorium-unique: Comms detects an incoming enemy transmission and must adjust a frequency dial to "lock on" before the window closes. On success, the intercepted message is delivered to the crew (possibly encoded — see Code Cyphers #37). Creates active attentive work for Comms instead of passive waiting. New `InterceptionOpportunity { freq: f32, window_secs: u32 }` server push; client shows a frequency-sweep dial. |
| 36 | **Signal Jammer** | T | ★★★ | ★★★ | Comms selects a frequency + power level to jam: enemy sensors, torpedo guidance systems, or their comms. No automatic feedback — Comms reports the jam and the result is communicated narratively. New `ActiveJam { freq, power, target_type }` message; server applies the effect to matching NPC entities. |
| 37 | **Code Cyphers (decryption puzzle)** | T | ★★★ | ★★★ | Comms receives an encoded message (a character-substitution cipher). They use a displayed cypher key to decode it and relay the content to the Captain verbally. Entirely new mini-game for Comms. Client-side: render a cypher key table and an encoded message string. New `ServerMessage::EncodedMessage { cypher_id, encoded_text }`. The puzzle is deliberate human effort rather than automation — the drama is in the room, not on screen. |

---

### Tier 3 — High payoff, substantial effort

| # | Feature | Src | Impact | Ease | Notes |
|---|---|---|---|---|---|
| 38 | **Scripted missions with objectives/narrative** | A/EE | ★★★★★ | ★★ | Draft 7 stub. Scenario file format: triggers (`on_time`, `on_destroyed`, `on_docked`), NPC spawn waves, narrative text messages, win/lose conditions. The difference between a sandbox and an experience. Requires an event system + trigger evaluator. |
| 39 | **Probe Construction (customisable equipment loadout)** | T | ★★★ | ★★ | Thorium: probes require assembling equipment modules (sensors, radio transceiver, etc.) before launch. No radio transceiver = probe can't report back. Creates an intentional pre-launch task for Science. Extends the basic EE probe concept with a build phase. New `ProbeEquipment` enum; server validates transceiver presence before allowing probe to appear on the network. |
| 40 | **Navigation with coordinate transcription** | T | ★★★ | ★★ | Thorium's Navigation card: Helm calculates XYZ coordinates for a destination, then *manually transcribes* them into the "current course" fields. Typos send the ship in the wrong direction. This deliberately adds human error as a mechanic. Requires a destination → coordinate lookup system and a target-course-vs-actual-input comparison. More effort than a "click destination" system but creates a moment of tension on every FTL jump. |
| 41 | **Thrusters (precision maneuvering)** | T | ★★★ | ★★★ | Thorium's Thrusters card: a fine-control panel for direction and rotation adjustments, with a live 3D ship-orientation display. Designed for docking, asteroid threading, and minor corrections. Adds a second sub-mode to Helm rather than a new console — requires a 3D orientation renderer or a 2D heading + elevation widget. |
| 42 | **Exocomps (repair robots)** | T | ★★★ | ★★ | Engineering attaches replacement parts to a small robot, dispatches it to a damaged system, waits for travel + repair time, and receives it back. An alternative repair mechanic that emphasises pre-planning (fitting the right parts) over real-time triage. New `Exocomp { parts: Vec<Part>, status: ExocompStatus }` entity; travel timer; repair execution at destination. |
| 43 | **DamCon teams (visual ship-schematic repair)** | A | ★★★ | ★★ | 6-person teams dispatched through a 2D ship schematic to damaged nodes. Teams can suffer casualties when a system is damaged while they are present. More tactile than current repair. Requires a schematic layout per ship class + team pathfinding (grid BFS). |
| 44 | **Tractor beam (player-controlled)** | T | ★★★ | ★★★ | Operations console retrieves objects from space. FD/server sets a "stress level" for the target; operator adjusts beam strength to match or exceed it. Different from the enemy tractor beam (#50): this is a tool the crew uses, not a threat they endure. Enables mission objectives like "recover the escape pod" or "retrieve the data beacon." New `TractorBeamState { stress, power }`. |
| 45 | **Security console** | T | ★★★ | ★★ | Entirely Thorium-originated. Security Officer manages internal ship threats: a deck map showing door status; lock/unlock doors remotely; evacuate a deck; deploy hazardous gas for extreme containment; dispatch named security teams to specific deck + room with orders and priority. Adds an internal-ship dimension to the game — the threat isn't only outside. New `SecurityConsole` plugin; `DeckMap { rooms, doors, teams }` server state. |
| 46 | **Medical: Sickbay** | T | ★★ | ★★ | Medical Officer scans crew for ailments, enters vitals, receives diagnoses. Adds an entirely new player role. Thorium's Medical suite includes Teams, Armory, Library, and Decontamination as additional tabs. High effort (new console + crew health state) but significantly expands the player roster capacity. |
| 47 | **Cloaking enemies (Skaraan)** | A | ★★★ | ★★★ | Enemy disappears from radar for 60 seconds. Comms taunt (or Signal Jammer at the right frequency) forces uncloak. `visible_on_radar: bool` flag + `cloaked_until: Instant` in NPC state. |
| 48 | **Drone weapons (Torgoth enemy)** | A | ★★★ | ★★★ | Slow-seeking projectiles fired by Torgoth enemies. Blocked by asteroids; can be shot down by Railgun. Rewards Helm using asteroids as shields. New `Drone` projectile entity type with weaker tracking than a homing torpedo. |
| 49 | **Multiple simultaneous ships (co-op)** | A/EE | ★★★★★ | ★ | Up to 6 bridges in one battle. Massive architectural change: per-ship `SimSnapshot`, per-bridge message routing, multi-ship viewscreen rendering. The current star topology maps poorly to this. |
| 50 | **Enemy tractor beam** | A | ★★ | ★★★ | Enemy holds player ship immobile for 30 seconds. New `ShipStatus::Tractored` override in physics — zero velocity, helm input blocked. Forces crew to react. |

---

### Tier 4 — Good additions, lower effort

| # | Feature | Src | Impact | Ease | Notes |
|---|---|---|---|---|---|
| 51 | **Mines (player-deployed)** | A | ★★★ | ★★★ | Weapons deploys proximity mines behind the ship using a torpedo tube. Mine entity: static position, detonates when any ship enters 1-unit radius. Same tube system as torpedoes; different trajectory (deploy at current position vs. fire forward). |
| 52 | **Particle Detector (sensor mini-game)** | T | ★★★ | ★★★ | Science clicks a grid to scan for specific particle types in adjacent sectors. FD/server places particles ahead of time. Creates active searching behaviour rather than passive radar watching. New `ParticleField` entity; client grid scanner. |
| 53 | **Probe Network (deployed probe map)** | T/EE | ★★★ | ★★★ | Visual sector map of deployed probes. Science can "link" a probe and see through its sensors (extends its radar coverage to the probe's position). Extends probe deployment with a dedicated view card. |
| 54 | **Engineering preset manager** | T | ★★ | ★★★★★ | Store and recall up to 10 power + coolant configurations. Pure Engineering UI feature. Trivial once Draft 5's power system exists. |
| 55 | **Surrender mechanic** | A | ★★ | ★★★ | Comms requests enemy surrender. Probability based on enemy hull integrity + personality flags. Surrendered ships go yellow on radar and cannot be targeted by Weapons. |
| 56 | **Enemy-specific taunts (per faction)** | A | ★★★ | ★★★ | Each faction has race-specific taunt options; each enemy captain has a personality affecting which taunts succeed. Science Intel report (level-2 scan) informs Comms which taunts to avoid. |
| 57 | **Naval intelligence reports** | A | ★★ | ★★★ | Per-ship data revealed by Science level-2 scan: known capabilities, personality flags, historical notes. Pure content addition once Science scanning exists. |
| 58 | **Library card (in-game database)** | T | ★★ | ★★★ | Crew look up information about ships, hazards, factions, and anomalies during a mission. FD pre-populates per scenario. Primarily a content system with a simple key→value lookup UI. |
| 59 | **Officer Log** | T | ★★ | ★★★★ | Captain and officers can record log entries during the mission. Flavour/narrative feature; trivial to implement. `ClientMessage::AddLogEntry(String)` → persisted on server; viewable in debrief. |
| 60 | **Anti-torp (enemy ability)** | A | ★★ | ★★★ | Specific enemy ships shoot down incoming torpedoes. Projectile-vs-projectile collision check. Rewards Weapons choosing the right ordnance type. |
| 61 | **Carrier ships launching fighters** | A/EE | ★★★ | ★★ | Enemy carriers spawn fighter squadrons. Fighters are fast and fragile; numerous. New `spawn_child_entity` mechanism in AI + fighter entity type with simple chase AI. |
| 62 | **Warp jammers** | EE | ★★ | ★★ | Zone entity disabling warp/jump for hostile ships within its radius. Tactical area-denial tool. |
| 63 | **Supply drops / escape pods** | EE | ★★ | ★★★ | Collectible entities. Supply drops restore torpedo supply; escape pods are "rescue" mission objectives. |
| 64 | **Faction reputation system** | EE | ★★ | ★★ | Track reputation with factions; affects NPC reactions and available missions. High complexity for moderate gain — best added once scripted missions exist. |
| 65 | **Space whales (passive NPC)** | A | ★★ | ★★★ | Peaceful NPC entities. Arvonian AI turns hostile if you destroy one; Torgoth AI breaks off pursuit to attack one. Low gameplay impact alone; high faction-flavour; adds memorable incidents. |
| 66 | **Hail neutral ships for energy** | A | ★★ | ★★★★ | Comms hails a Colonial Transport → one-time energy boost. `ClientMessage::HailNeutral(uuid)` → server checks entity type → fires `EnergyBoost` event. |
| 67 | **Transporters** | T | ★★ | ★★ | Beam objects/crew between locations: scan, charge, execute. Primarily a story-mechanic enabler for scripted missions. |
| 68 | **Shuttle management** | T | ★★ | ★★ | Launch and recover shuttles with multi-step procedures (clamps, airlock, doors). Procedural drama; adds a dedicated Operations player role. |
| 69 | **Junior stations** | T | ★★ | ★★★ | Simplified versions of each console for younger or less-experienced players. Easy to add once all main consoles exist — just a subset view with fewer controls. Broadens accessibility. |
| 70 | **Radar zoom levels (Helm)** | A | ★★ | ★★★★★ | 4 zoom levels; pure client-side scale change in `client_helm.rs`. |
| 71 | **Biomechs (4-stage evolving neutral enemy)** | A | ★★★ | ★ | Stage 1 eats asteroids → evolves to Stage 2 → Stage 3 (aggressive) → Stage 4 (partially communicable via Comms). Very memorable; very high effort. Complex multi-stage state machine. |
| 72 | **PvP mode** | A/EE | ★★★★ | ★ | Two (or more) crews fight each other. Requires multi-ship architecture + faction system + player ships as valid Tactical targets. |
| 73 | **3D vertical movement** | A | ★ | ★ | ±500-unit elevation axis (Artemis 2.0). Complicates all radar rendering, AI targeting, and physics significantly. High cost; minimal party-game benefit. **Not recommended.** |

---

## What Thorium adds that Artemis / Empty Epsilon don't

Thorium's design philosophy differs from Artemis and EE in three important ways that yield features the others don't have.

### 1. Task splitting

Thorium deliberately splits jobs that other simulators keep together:

- Phaser **Charging** and **Targeting** are two separate cards, intended for two separate people.
- **Power Distribution** and **Reactor Control** are two separate cards.
- **Navigation** (course calculation) and **Thrusters** (fine maneuvering) are two separate cards.

For Project Phoenix this means a single "Tactical" or "Engineering" console could become a multi-tab station where different tabs are designed to be worked simultaneously by two players. The Phaser Charging split (#24) is the highest-impact example: one person charges, another fires, neither can solo the system under pressure.

### 2. Deliberate human-error mechanics

The Navigation coordinate transcription (#40) forces a human to copy numbers by hand, creating intentional failure modes. Damage Reports with sequential steps and reactivation codes (#31) and Code Cyphers (#37) share the same principle: the challenge is not reflexes but attention and communication. These mechanics work particularly well for a party game because the drama happens in the room, not on the screen.

### 3. Internal ship threats

Thorium's Security console (#45) and Medical Sickbay (#46) model threats *inside* the ship — boarders, security incidents, biological hazards, injured crew — rather than only external combat. This opens a dimension of play that Artemis and EE don't have, and makes Security, Medical, and related players meaningful even when the space outside is quiet.

---

## Recommended sequencing

Reading across all three sources, the highest-leverage path for Project Phoenix:

1. **Enemy ships + wave spawner + win/lose conditions** (#1–3) — unblocks everything else
2. **Space stations — functional** (#14) — enemies need a target; players need a home base
3. **Alert Condition 5-level + Status dashboard + Self-destruct** (#5, #12, #13) — near-zero effort, major command feel
4. **Shared energy pool** (#4) — transforms Engineering from repair-bot to resource manager
5. **Warp drive + Navigation coordinate input** (#16, #40) — expands the play area; adds Helm deliberateness
6. **Comms console: short-range + long-range** (#18, #19) — Draft 8 fleshed out with Thorium's two distinct mechanics
7. **Phaser Charging split** (#24) — turns Tactical into a two-player station with no new console
8. **Railgun** (#25) — makes Tactical feel genuinely reactive and dangerous
9. **Interception + Signal Jammer + Code Cyphers** (#35–37) — makes Comms never boring between contacts
10. **Stealth Field** (#32) — novel mechanic that rewards crew coordination in a completely different way
