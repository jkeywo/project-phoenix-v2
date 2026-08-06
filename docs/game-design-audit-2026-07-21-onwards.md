# Game Design Audit — 2026-07-21 onwards

Audit of **game-design decisions** (gameplay rules, balance, behaviour, feel) that
were made and recorded in code comments, authored TOML content, or PASM spec during
the period **2026-07-21 → 2026-08-04**. Purely architectural/code refactors are
excluded unless a commit carries an explicit gameplay/balance intent.

Records live in-repo in:
- `pasm/spec/architecture/*.yaml` and `pasm/spec/design/*.yaml` — spec prose
- `assets/entities/**` (TOML) — authored content + design commentary
- `scripts/balance-runs.demo.toml`, `assets/worlds/*.toml`, `assets/scenarios.toml`
- inline code comments that encode design intent

---

## 1. Combat & Fleet Balance

### Blaster damage is a step function, not a dial
| Field | Value |
|---|---|
| **Decision** | Blaster damage decides how many volley cycles a kill costs — a step function, not a tunable dial. Measured over 25 seeds vs the Harrow patrol: 15→56% win, 16→56% (top of the near-even band), 17→100% (first guaranteed sweep), 20→100%; nothing between. **16 is the shipped value.** Damage 20→15 was set as a *floor*, not a round number (at 14 the cruiser matchup collapses to 1/5). The two forward banks are the destroyer's whole offence. *Audit resolution (2026-08-05): the recorded fork "16 near-even vs 20 guaranteed patrol sweep" is settled — keep 16. The near-even patrol is the intended design; a 5/5-at-21s sweep is the "delete button" outcome to avoid. The 0.4 `min_win_rate` regression floor already survives the real 56%.* |
| **File** | `assets/entities/alliance_destroyer.toml` (per-bank comment blocks) |
| **Date** | 2026-08-02 |

### One durability ladder for the fleet
| Field | Value |
|---|---|
| **Decision** | The whole fleet re-pegged on a single durability ladder so shield/hull feel and time-to-kill are consistent across classes: Dynasty courier 100 / destroyer 200 / patrol+cruiser 300 / warhawk 400; Alliance courier 200 / destroyer 300 / cruiser 500 / battleship 800. Dynasty carries monolithic `hull_integrity`; Alliance's pool is the sum of per-system `max_hp`. *Audit resolution (2026-08-05): the ladder is really two faction-resolved ladders with a deliberate ~1.5–2× Alliance-heavy offset — intentional, load-bearing faction power asymmetry, not a scaffold. Same-named ships are never true mirrors cross-faction.* |
| **File** | `assets/entities/alliance_*.toml`, `assets/entities/ship_harrow_*.toml`, `scripts/balance-runs.demo.toml` |
| **Date** | 2026-08-02 |

### Radar systems carry no hull
| Field | Value |
|---|---|
| **Decision** | `helm-radar` / `tactical-radar` / `sensor-radar` carry no `[[hull.system_hull]]` on the Alliance hulls **as currently authored** — a snapshot, not a persistent design rule. Their points were redistributed to surviving systems; the commit notes radar had been doing "real work as damage soak" (patrol 4/5 → 3/5). The engine-level "blinded ship forgets its target" mechanic (#893) remains in place and test-exercised (`assets/worlds/probe_radar_kill.toml`); any hull or scenario that authors radar HP gets it. *Audit resolution (2026-08-05): "no ship has damageable radar" was a point-in-time statement, not a rule — the mechanic stays test-only, no load-time guard.* The Lancer sits deliberately **off** the ladder at 800. *Audit resolution (2026-08-05): the Lancer (ln class) is a coverage-instrumentation stat, not fleet design — intended to be **removed**; `rng_coverage.toml` should author its own local ln variant instead of pulling the shipped hull.* |
| **File** | `assets/entities/alliance_*.toml` (RADAR SYSTEMS CARRY NO HULL), `assets/entities/ship_harrow_lancer.toml` |
| **Date** | 2026-08-02 |

### Low-speed turn
| Field | Value |
|---|---|
| **Decision** | Hulls that fly slow turn harder: yaw scaled by `low_speed_turn_boost` (×1+X at standstill, lerp to ×1 at cap, but measured so full astern earns nothing). Identical for human helm and AI backfill. Capitals author `0.0` explicitly ("0 here is a balance decision, not an oversight"). Harrow destroyer's 0.3 < Alliance 0.5 so it doesn't hold both ends of the trade. *Audit resolution (2026-08-05): no retreat-friendliness skew — the pursuer usually needs no turn (already facing the chase) and otherwise burns down to use the same boost. Symmetric by access; the "helps runner as much as chaser" finding stands.* |
| **File** | `assets/entities/*.toml` (`[helm_console]`), `wiki/entities/helm-console.md` |
| **Date** | 2026-08-02 |

### Degradation thresholds are a feel change
| Field | Value |
|---|---|
| **Decision** | Destroyer system degradation 0.75/0.25 → 0.5/0.15 (operational-to-half / limping at 15%) — a FEEL change, not a balance change: buys ~3 s before the first knockout a human crew feels at the repair console, which AI backfills don't. *Audit resolution (2026-08-05): thresholds track repair **capacity**, not just feel — the destroyer has a small crew and few repair teams and the crew watches the ship-wide health bar, so breaking systems early would outrun their repair ability; heavier ships with multiple teams can afford earlier per-system decay. Intended: late-degradation cliff for low-repair ships, earlier per-system breakdown for high-repair ships.* |
| **File** | `assets/entities/alliance_destroyer.toml` (system-hull degradation comment) |
| **Date** | 2026-08-02 |

### `min_win_rate` is a regression floor, not a description
| Field | Value |
|---|---|
| **Decision** | `destroyer_vs_harrow_patrol` `min_win_rate` 0.8 → 0.4, reframed as a REGRESSION FLOOR (below the true 56% to survive 5-seed noise) rather than a description. Recorded that five fixed seeds cannot resolve a 56% duel; re-measure at 25 seeds. *Audit resolution (2026-08-05): a red `min_win_rate` **flags, never blocks** — the batch is advisory; a red threshold is a message to a human to resolve (cf. the Lancer removal), not a gate that refuses to record.* |
| **File** | `scripts/balance-runs.demo.toml` |
| **Date** | 2026-08-02 |

### Patrol duel is a deliberately near-even fight
| Field | Value |
|---|---|
| **Decision** | Hull 120→300 plus blaster 20→16 turned a 9.3 s fly-past into a near-even engagement (56% at a 25-seed rate). This is a deliberate design position, not a runaway accidentally left. |
| **File** | `scripts/balance-runs.demo.toml` (`destroyer_vs_harrow_patrol`) |
| **Date** | 2026-08-02 |

### Blasters lead where the target is going
| Field | Value |
|---|---|
| **Decision** | The blaster lead solver was mis-aiming (~26 units behind a straight course at hold range); does neither course change before launch always saves you. Now solves the exact intercept. |
| **File** | `src/weapons/blaster.rs` (intent), `pasm/spec/architecture/weapons.yaml` |
| **Date** | 2026-07-26 |

---

## 2. Weapons & Weapon Behaviour

### One target lock per ship (weapon targeting)
| Field | Value |
|---|---|
| **Decision** | A human holds exactly one radar selection → a player ship engages one target; AI weapon groups must not engage more targets than a player can. The AI gate was wider than admission's. *Audit resolution (2026-08-05): the whole ship's offense funneling through one lock (tactical officer as the trigger point) is intentional for now; deliberately single-target, no split-fire. Future extensions named: deliberate split-fire weapons, or multiple tactical radars with each weapon tied to one radar.* |
| **File** | `pasm/spec/architecture/weapons.yaml` |
| **Date** | 2026-07-30 |

### A blinded ship forgets its target
| Field | Value |
|---|---|
| **Decision** | The tick a tactical radar crosses into Destroyed, its standing target lock clears (human- and AI-held alike), keyed on system tier, never on who set it. A merely Disabled radar does not clear it. |
| **File** | `pasm/spec/architecture/weapons.yaml`, `src/ship/damage_sync.rs` |
| **Date** | 2026-08-02 |

### A phaser attack captures its target at start
| Field | Value |
|---|---|
| **Decision** | The beam freezes its target from the combat lock at attack start and does NOT jump when the lock changes mid-attack; it severs only when the captured target leaves arc/range or vanishes. Several banks may fire simultaneously (no bank disables a sibling). |
| **File** | `pasm/spec/architecture/weapons.yaml`, `src/console/weapons/beam.rs` |
| **Date** | 2026-07-24 |

### Ammo in a tube is already paid for; load and launch are separate verbs
| Field | Value |
|---|---|
| **Decision** | A round loaded is already paid; the magazine counts what is left to RELOAD, not what can fire. An empty magazine never blocks a fully loaded battery firing. LOAD reserves ammo, LAUNCH only consumes loaded rounds. *Audit resolution (2026-08-05): intended torpedo economy is "pre-stock rounds at leisure, then dump on command" — the magazine is a strategic pre-loading layer, not a burst cap.* |
| **File** | `pasm/spec/architecture/weapons.yaml` |
| **Date** | 2026-07-25 |

### Patterned barrel attacks
| Field | Value |
|---|---|
| **Decision** | A patterned multi-barrel attack is an origin/ordering map only — a step listing more barrels than are loaded never issues extra rounds; `loaded_count` drives the count. Patterns enable alternating-or-salvo multi-barrel attacks, backward-compatible with legacy single-shot cadence. *Audit resolution (2026-08-05): when a step finds fewer tubes loaded than listed, fire what's available now — the volley shape degrades silently rather than waiting for a full barrel set.* |
| **File** | `pasm/spec/architecture/weapons.yaml`, `assets/entities/alliance_battleship.toml` |
| **Date** | 2026-07-24 |

### Torpedoes fly in full 3D
| Field | Value |
|---|---|
| **Decision** | Vertical separation and rate-limited pitch threaded through guidance/collision/detonation; vertical offset genuinely changes homing. No new constant — pitch clamped by the same `turn_rate` as yaw. |
| **File** | `pasm/spec/architecture/weapons.yaml`, `src/weapons/torpedo.rs` |
| **Date** | 2026-07-24 |

### Weapon-family arc-bearing coordination
| Field | Value |
|---|---|
| **Decision** | Tactical asks Helm to turn when a weapon family has a target in range but out-of-arcs. *Audit resolution (2026-08-05): the fixed phasers > blasters > torpedoes priority is WRONG — family choice is state-dependent (e.g. turn onto the target for torpedoes when its shields are down) and belongs in the movement doctrines, not a global ordering.* — so only one request is active; issued only when a usable online emitter is OutOfArc in range and nothing is Ready. |
| **File** | `pasm/spec/architecture/weapons.yaml` |
| **Date** | 2026-07-24 |

### Hostile-weapon-arc threat overlay excludes torpedo tubes
| Field | Value |
|---|---|
| **Decision** | A homing round has no bounded threat radius, so a wedge would "lie about where it is safe"; offline banks drop out of the list so arcs and standoff can't disagree. Arc data is authored, never a scan sweep. *Audit resolution (2026-08-05): overlays stay energy-arcs-only, but add a "torpedo-armed" badge on hostiles (launched-missile markers plus a tube-capability badge) so players know a ship can run torpedoes before it fires.* |
| **File** | `src/weapons/arc_geometry.rs`, `pasm/spec/architecture/weapons.yaml` |
| **Date** | 2026-07-30 |

---

## 3. Helm / Navigation / Movement

### Ship-authored helm capability & impulse feel
| Field | Value |
|---|---|
| **Decision** | Helming is not universal — a ship's usable drives (engines, steering, lateral, vertical, impulse, boost) and mode are authored per hull. Impulse course-correction is deliberately harsh (steering multiplier 0.1); boost is unavailable during impulse. |
| **File** | `pasm/spec/architecture/helm-controls.yaml`, `pasm/spec/design/helm-controls.yaml` |
| **Date** | 2026-07-22 |

### Steering vs thrust split
| Field | Value |
|---|---|
| **Decision** | Steering and thrust are separate, independently-damageable axes so a ship can lose agility while keeping speed (or vice-versa) and a player/AI can mix them. |
| **File** | `pasm/spec/architecture/helm-controls.yaml` |
| **Date** | 2026-07-23 |

### Desired-motion planner
| Field | Value |
|---|---|
| **Decision** | Movement intent is separated from actuation; objectives + hazard avoidance form a shared per-ship "desired velocity + desired facing" 3D contract fine systems interpret by their own capability. Desired facing is kept split from desired travel so arc-requests/docking don't hijack translation. *Audit resolution (2026-08-05): the "ignore hazards smaller than own size" blind-spot rule is split by hazard kind — a big ship ignores smaller **ships** (deliberate clumsiness feel), but must NOT ignore smaller **static entities** (asteroids, stations, planets); static terrain is always avoided regardless of size.* |
| **File** | `pasm/spec/architecture/helm-controls.yaml` |
| **Date** | 2026-07-23 |

### Vertical flight is a per-ship mode (3 modes)
| Field | Value |
|---|---|
| **Decision** | Planar (none), **Bounded** (AI-only climb over *moving* hazards, capped and easing back), or full 3D six-DoF. Vertical thrust reacts only to moving hazards — static obstacles never push it. |
| **File** | `pasm/spec/architecture/helm-controls.yaml` |
| **Date** | 2026-07-23 |

### Data-authored engines & steering policies
| Field | Value |
|---|---|
| **Decision** | Whether engines/steering actuate each tick is an authored per-hull policy — a hull's movement aggression/safety is designer-tuned data, not a constant. |
| **File** | `pasm/spec/architecture/helm-controls.yaml` |
| **Date** | 2026-07-24 |

### Committed-heading escape feel
| Field | Value |
|---|---|
| **Decision** | On break-off a ship may commit a frozen escape heading; the "hold committed heading" verb flies that facing instead of re-solving against the moving threat — distinct from "hold", which would yaw forever. *Audit resolution (2026-08-05): "frozen escape heading" = frozen against re-acquisition, not frozen against collision — hazard avoidance bends the actual path around rocks while keeping the escape intent.* |
| **File** | `pasm/spec/architecture/helm-controls.yaml` |
| **Date** | 2026-08-01 |

### Waypoints & host teleport
| Field | Value |
|---|---|
| **Decision** | A ship follows one nav waypoint at a time (fixed or a live entity UUID refreshed until despawn; anchored waypoints auto-clear when the target dies). The host can teleport the player ship onto its cleared waypoint. *Audit resolution (2026-08-05): the host teleport is a pure debug/cheat tool — no GM/player-experience concerns apply.* |
| **File** | `pasm/spec/architecture/navigation.yaml` |
| **Date** | 2026-07-23 |

### Docking & arc-bearings
| Field | Value |
|---|---|
| **Decision** | Docking is the only request that drives the hull at low speed with controlled reverse/lateral (within `docking_engage_distance`). Tactical arc-bearings affect facing only — never lateral/reverse — and Helm may decline on a committed leg. |
| **File** | `pasm/spec/architecture/helm-controls.yaml`, `pasm/spec/design/helm-controls.yaml` |
| **Date** | 2026-07-23 |

---

## 4. Sensors, Power, Repair, Shields

### Sensors target selection tiers
| Field | Value |
|---|---|
| **Decision** | Backfilled Sensors picks (mirror combat lock → named Destroy objective → nearest faction-hostile in range), advisory to Tactical, never a firing lock; a selected hostile leaving the sensed horizon is dropped. Data-driven (reusable selector) rather than hardcoded. |
| **File** | `pasm/spec/architecture/radar-sensors.yaml` |
| **Date** | 2026-07-23 |

### Per-group power allocation, reserve-guarded
| Field | Value |
|---|---|
| **Decision** | Power is split into authored groups, each with a level by player or authored AI rules. Every rule's `when` guard must pass a minimum battery reserve before firing, so allocation never rises beyond what the battery sustains (no all-or-nothing emergency); reactor capped at 8 so elevating one group visibly costs another. *Audit resolution (2026-08-05): the AI should **see current available power** and prioritize which systems get points, never exceeding the max — budget-aware allocation, not silent cap-refusal-and-reemit.* *Audit note (2026-08-05): `fleet_baseline.toml`'s sensors-budget rationale ("SENSORS POWER IS WEAPON REACH") and the sensors rest-at-1 seeding are stale after the range-coupling revert above — re-author whichever sensors rules remain once sensors no longer scales phaser range.* |
| **File** | `pasm/spec/architecture/power-modifiers-regions.yaml`, `pasm/spec/design/power.yaml` |
| **Date** | 2026-07-24 (refined 07-25) |

### Power levels are gameplay values — sensors = weapon reach
| Field | Value |
|---|---|
| **Decision** | Power maps to gameplay: helm→MaxSpeed/Yaw, weapons→PhaserDamage, **sensors→RadarRange scaling every phaser's effective range**. Level 2 = ×1.0, so a hull fights at its authored `beam_range` only at nominal sensors (previously every AI-crewed Alliance hull fought at ⅔ of every authored range). Fleet authors a sensors red-alert elevation paid for by dropping the weapons spike (reactor capped). *Audit resolution (2026-08-05): this coupling is WRONG — power should modify **damage**, not range, for phasers/blasters; they attack independent of radar range. Sensors→RadarRange should not scale weapon reach. Reverts #923's range-coupling. Follow-on (2026-08-05): **drop the red-alert sensors elevation** — it was bug-compensation for the range bug, not design intent; re-grant those reactor points to weapons at red alert.* |
| **File** | `pasm/spec/architecture/power-modifiers-regions.yaml`, `assets/entities/fragments/ai/fleet_baseline.toml` |
| **Date** | 2026-08-01 |

### Shield collapse & slow recovery replace snap-back
| Field | Value |
|---|---|
| **Decision** | A collapsed facing stops `offline_duration` (6s destroyer / 10s battleship), then returns **online at zero** and climbs at `regen_per_sec` — not snap-to-full. Sustained fire collapses it again, making "break off until shields are a fraction" an explicit strategic question. *Audit resolution (2026-08-05): the collapse cliff is intentional drama — a shallow drop recovers fast, a collapse costs long (offline + climb), teaching "break off before the facing folds." Kept as-is.* |
| **File** | `pasm/spec/architecture/shields.yaml`, `pasm/spec/design/shields.yaml`, `assets/entities/ship_harrow_destroyer.toml` |
| **Date** | 2026-07-30+ |

### Shield focus on recent damage
| Field | Value |
|---|---|
| **Decision** | AI focuses the arc under greatest *recently measured* incoming damage, falling back to health-imbalance when not concentrated; corrected to read true incoming damage and ignore a focus decay side effect. |
| **File** | `pasm/spec/architecture/shields.yaml`, `pasm/spec/design/shields.yaml` |
| **Date** | 2026-07-23 (corrected through 07-25) |

### Repair visibility gated by team arrival
| Field | Value |
|---|---|
| **Decision** | Engineering sees ship-wide aggregate hull immediately; exact non-core damage detail only once a repair team is on site. On-site, Engineering gains subsystem priority authority; AI stations surface advisory repair requests. *Audit resolution (2026-08-05): a human Engineer can **redirect** any on-site repair to a different system at the same location at any time — repair to one system can be interrupted and swapped; on-site priority isn't locked to the initially-dispatched target.* |
| **File** | `pasm/spec/architecture/engineering-damage.yaml` |
| **Date** | 2026-07-22 |

### Authored repair dispatch priorities
| Field | Value |
|---|---|
| **Decision** | Which fine system in a station heals first is an authored per-system target selector ranked by damage fraction, identical for human and AI dispatch — no bespoke AI heuristic. The selector is the default/backfill behaviour; a human `SetRepairPriority`-style redirect overrides at will (per 2026-08-05 resolution). Tier-over-deficit triage kept: any higher DamageTier beats any lower even at full health; deficit bands 0.80/0.90/0.95 discriminate within a tier. *Audit resolution (2026-08-05): when a human isn't commanding something specific, the authored selector (old behaviour) applies; an AI worker may **reconsider its target mid-repair** if a higher-priority system gets damaged, rather than being locked to its initial dispatch.* |
| **File** | `pasm/spec/architecture/engineering-damage.yaml` |
| **Date** | 2026-07-25 |

### Damage makes systems forget their phase
| Field | Value |
|---|---|
| **Decision** | A shot-out helm system forgets its phase; on repair it resumes a fresh pass, not the interrupted move — disruption feels like a real interruption, not a pause (pinned by test). *Audit resolution (2026-08-05): forget-phase applies only to **momentary control actions** (helm escape, weapons attack). Repair **progress persists** through a system getting shot out — half-fixed damage stays half-fixed and resumes at the interrupted fraction; no double-punitive reset.* |
| **File** | `src/ship/helm_ai.rs` (test) |
| **Date** | 2026-07-27 |

---

## 5. NPC AI & Fleet Doctrine

### The three Harrow stances are distinct authored behaviours (→ one class library)
| Field | Value |
|---|---|
| **Decision** | Within one week the fleet shipped three distinct behaviours — destroyer *flies a pass*, cruiser *circles*, battleship *stops and snipes* — then generalised into a shared co-authored class-doctrine library (attack-pass, broadside-orbit, artillery fragments) so hulls become an `includes` line plus per-hull knobs (`commit_range`, `safe_range_margin`, `press_posture`). |
| **File** | `assets/entities/fragments/ai/movement_*.toml`, `assets/entities/ship_harrow_*.toml` |
| **Date** | 2026-07-25 → 2026-08-02 |

### Destroyer fly-through + shield-recovery doctrine
| Field | Value |
|---|---|
| **Decision** | Closes, merges, then flicks to a frozen escape heading and ignores the target (escape-dwell is inviolate — *frozen against re-acquisition, not collision* per 2026-08-05 resolution). Breaks off to a ring sized to the *enemy's* reach + margin when shields below `recover_shield_fraction` 0.15, re-engaging only when `reentry_shield_fraction` ≥0.75 AND distance held. If the escape can't open space (pressed), it abandons flight and fights at the stuck range as a jab. Consistent with shield-collapse: 0.75 is only reachable after the offline window ends, so re-engage never lands in an offline fold. |
| **File** | `assets/entities/fragments/ai/movement_attack_pass.toml` |
| **Date** | 2026-07-25/26 |

### Cruiser broadside orbit & torpedo run
| Field | Value |
|---|---|
| **Decision** | Holds a broadside combat ring so broadsides bear the whole ring, cutting a torpedo run only when a whole committed salvo (`tubes_full`) is loaded — a partial battery must not open a window; onto the struck-down shield arc (`target_facing_shields <= 0`, Harrow-only via `torpedo_run_shield_gap = 1.0`, as its rounds do 0 shield damage; the Alliance cruiser runs on readiness alone). The ring doesn't yield facing to arc requests. This is the concrete realisation of the 2026-08-05 "arc-bearing family choice belongs to doctrine, not a global phasers>blasters>torpedoes priority" resolution — the shield-down torpedo run is an authored doctrine transition, not a global tiebreak. |
| **File** | `assets/entities/fragments/ai/movement_broadside_orbit.toml`, `assets/entities/ship_harrow_cruiser.toml` |
| **Date** | 2026-07-26 |

### Warhawk predictive artillery & opportunistic close defence
| Field | Value |
|---|---|
| **Decision** | The battleship sits in a hysteresis band and pivots the bow onto *where the target will be* (lead from the blaster solver). Opposed fore/aft torpedoes gate fire per-tube (NOT ship-wide `tubes_full`) — a loaded fore tube must not refuse because the aft is reloading. Deliberately inverted reading vs the cruiser (opposed tubes can't salvo). Impulse autopilot switched off on the policy so doctrine can't re-enable it. |
| **File** | `assets/entities/fragments/ai/movement_artillery.toml`, `assets/entities/ship_harrow_warhawk.toml` |
| **Date** | 2026-07-26 |

### The fleet reads one book
| Field | Value |
|---|---|
| **Decision** | Player + Harrow hulls adopt the same shared class-movement doctrines via the fragment library (#875/876/878) rather than four hand-authored Harrow hulls. Harrow expresses its difference through a single `min_alert_to_fire` + the torpedo shield gap, not bespoke code. |
| **File** | `assets/entities/fragments/ai/`, `assets/entities/ship_harrow_*.toml` |
| **Date** | 2026-07-31 → 08-02 |

### Red-alert posture gate on the aggressive half of doctrine
| Field | Value |
|---|---|
| **Decision** | The pressed/aggressive half of every movement doctrine unlocks only under Red Alert (`posture` 0 DEFENSIVE / 1 PRESSED from own alert). Alliance crewed hulls fire until the alert (`min_alert_to_fire = 1`); Harrow always armed (`0`, no captain). First hostile contact raises the alert so a backfilled hull isn't shadow-forever. *Audit resolution (2026-08-05): the Red Alert system itself carries **no hull damage** (`red-alert` is a host-seeded typed fact, not a `[[hull.system_hull]]` target), so it can never be knocked out — a disabled-captain "never fires" trap cannot occur; the gate is pure authored content.* |
| **File** | `assets/entities/fragments/ai/captain_alliance.toml`, `assets/entities/fragments/ai/fleet_baseline.toml` |
| **Date** | 2026-07-30/31 |

### Missing AI declaration is a load error
| Field | Value |
|---|---|
| **Decision** | Every AI-capable system must declare its policy (or explicitly idle) — nothing silently inferred; an unconfigured NPC system takes no action instead of a quiet default. |
| **File** | `pasm/spec/architecture/data-driven-fine-system-ai.yaml` |
| **Date** | 2026-07-30 |

### AI decisions run on a fixed authored cadence, not frames
| Field | Value |
|---|---|
| **Decision** | Every AI decision runs once on the shared authored cadence (default 30 Hz; slower whole-multiple cycles) rather than once per frame, so the same seed reproduces the same firing/policy phase on every machine. |
| **File** | `pasm/spec/architecture/data-driven-fine-system-ai.yaml` |
| **Date** | 2026-07-27 |

### AI fire/target discipline = a player's
| Field | Value |
|---|---|
| **Decision** | AI target selection writes the same single combat lock as the human radar (one applier — an AI can't overwrite a human's lock); the AI holds its firing/lock only while it can see the target. |
| **File** | `pasm/spec/architecture/npc-ai-factions.yaml`, `pasm/spec/architecture/weapons.yaml` |
| **Date** | 2026-07-30 / 08-02 |

---

## 6. Game Flow, Red Alert, Objectives

### Red Alert is an explicit set-command
| Field | Value |
|---|---|
| **Decision** | Alert set by explicit set-state (active true/false), not a toggle blocker (retries can't invert). AI operators use the identical command shape. Captain console read-only while the system is AI-owned. |
| **File** | `pasm/spec/architecture/red-alert.yaml` |
| **Date** | 2026-07-23 |

### Advisories come from ship state, not who holds the station
| Field | Value |
|---|---|
| **Decision** | A Sensors/Tactical advisory fires identically whether a human or a Backinfill AI holds the station — a human sitting there no longer silences ship-wide advisories. Captain can call the alert on first hostile contact, not only after a hit. |
| **File** | `pasm/spec/architecture/coordination-blackboards.yaml`, `assets/entities/fragments/ai/captain_alliance.toml` |
| **Date** | 2026-07-30 / 07-31 |

### Pre-scenario lobby & first-valid selection
| Field | Value |
|---|---|
| **Decision** | Players join a pre-scenario lobby before any world loads; scenario/ship selection is first-valid-request-wins (no captain gate). Selectable roots: `default`, `combat_test`, `before_the_fire`. |
| **File** | `pasm/spec/architecture/game-flow.yaml`, `assets/scenarios.toml` |
| **Date** | 2026-07-23 |

### Collective-start readiness
| Field | Value |
|---|---|
| **Decision** | Round start is collective-readiness: a 5-second countdown once every player is ready, cancelling on any unready/disconnect/new join; game over returns to the lobby preserving last rating but clearing claims, ready state; the world is reused for round 2. |
| **File** | `pasm/spec/architecture/game-flow.yaml`, `src/lobby/handler.rs` |
| **Date** | 2026-07-23 |

### Combat scenario — death-gated waves
| Field | Value |
|---|---|
| **Decision** | Wave N+1 fires on `on_all_destroyed` of wave N (a 10s breather) so pacing self-balances across a power tier; only wave 1 on a clock. Picket patrols sit in a deliberate choice to a ship; victory/defeat are single `on_all_destroyed` gates, not wall-clock. *Audit resolution (2026-08-05): REVERSED — waves spawn on a **fixed timer**, not death-gated with a delay; only game-over is on a short delay from all ships destroyed. ALL enemy ships move to attack the **Starbase**, which should be of the **player's faction** (currently factionless) — no patrols/pickets at all.* |
| **File** | `assets/worlds/combat_test.toml` |
| **Date** | 2026-08-03 |

### Objectives: lifecycle, scoped priority, asymmetric visibility
| Field | Value |
|---|---|
| **Decision** | Objectives are first-class AI directives (Patrol, Destroy, Reach, Retreat, Hail) each with consumer systems; lifecycle Active→Completed/Failed from triggers/comms/destruction. Captain boost is **scoped to their own ship**; visibility is asymmetric by role. Owned by the world layer that authored them. *Audit resolution (2026-08-05): single-ship reality — objectives are black-box state gated on `Has<LocalShip>`; NPCs get empty objective lists and don't use this state (NPCs act via doctrine). "Scoped to the captain's ship" effectively equals the session today; the multi-ship wording is aspirational and harmless. Revisit only if multi-crew is ever added.* |
| **File** | `pasm/spec/architecture/objectives.yaml`, `assets/worlds/default.toml` |
| **Date** | 2026-07-23 |

---

## 7. Cross-cutting design rules

| Rule | File | Date |
|---|---|---|
| **Human/AI symmetry** — a backfilled seat behaves through the same admitted command surface as a human console; a legitimately designed character, not a degenerate autopilot. | AGENTS.md §6 | throughout |
| **Authored, not hardcoded** — every feel/number above (impulse multiplier, turn boost, docking engage range, hazard sensitivity, shield regen/offline, power reserves, damage windows, movement throttles, quirk) is TOML-authored; mandatory blocks omit at load error. | `assets/entities/fragments/ai/fleet_manner.toml` + others | throughout |
| **Determinism is load-bearing for balance** — the demo batch is 5 fixed seeds over 120 s, replayable; every stream of chance answers to one seeded RNG; outcome is a reported heuristic, not engine-adjudicated. | `scripts/balance-runs.demo.toml`, `pasm/spec/architecture/headless-balance-telemetry.yaml` | 07-31 / 08-02 |

---

## Method note

Assembled from `git log --since="2026-07-21"`, the PASM spec YAML under `pasm/spec/`,
authored TOML under `assets/`, and inline code comments carrying design intent.
Pure refactors (admission plumbing, RNG internals, per-entity migration mechanics,
CI/smoke-spec followers) were excluded unless a commit explicitly records a
gameplay/balance intent.

---

## Follow-up actions (from 2026-08-05 audit review)

Decisions from this session that **change** current authored behaviour and still
need implementation:

1. **Remove the Lancer (ln) fleet hull** — delete `assets/entities/ship_harrow_lancer.toml`
   (and its `assets/entities/ln.toml` template reference); have `assets/worlds/rng_coverage.toml`
   author its own local ln variant instead of pulling the shipped hull. Update
   `assets/strings/strings.csv` `entity.ln.*` entries accordingly.
2. **Revert phaser/blaster power range-coupling (#923)** — weapons power should scale
   **damage**, not range; phasers/blasters attack independent of radar range. Remove
   the sensors→RadarRange→phaser-range chain in `src/modifiers/coordination.rs`
   (`apply_power_modifiers_from_read_state`).
3. **Drop the red-alert sensors elevation** — was bug-compensation for the range issue;
   re-grant those reactor points to weapons at red alert. Re-author stale sensors
   budget/rules in `assets/entities/fragments/ai/fleet_baseline.toml` ("SENSORS POWER IS
   WEAPON REACH" rationale + sensors rest-at-1 seeding).
4. **Budget-aware AI power allocation** — AI should see current available power and
   prioritize point distribution without exceeding the max, replacing the silent
   cap-refusal-and-reemit mechanism.
5. **Arc-bearing family priority not global** — drop the fixed phasers>blasters>torpedoes
    default; family choice is doctrine-driven (e.g. torpedoes when target shields are down).
 6. **Torpedo-armed badge** — add a "torpedo-armed" badge on hostiles so players know a ship
    can launch torpedoes before it fires.
 7. **Static terrain always avoided** — "ignore hazards smaller than own size" must not apply to
    static entities (asteroids, stations, planets); big ships ignore small *ships* but never
    static terrain.
 8. **Combat scenario redesign (#892)** — revert death-gated waves to a fixed spawn timer
    (only game-over is delay-gated on all-destroyed); every enemy ship attacks the **Starbase**;
    make the Starbase **player-faction** (currently factionless); remove all patrols/pickups.
    Targeting: enemies engage the **player** only if the player is within their **sensor/radar
    range (~200 units)**; they **disengage** when the player leaves that range. Primary objective
    is always the Starbase (the player's faction). The ~200 engage / 150 alert-threshold ordering
    is intentional — an enemy shoots first within the 200-150 band and the player must close to 150
    (or take damage, which triggers red-alert) before their own guns unlock under the `min_alert_to_fire`
    gate (2026-08-05).

9. **Missing-AI-declaration rule kept (incl. test hulls)** — confirmed strict: test/coverage-only
    local hulls must also state AI policy or pull the shared fragment; no lenient omitted-→-Idle escape.

Resolved decisions recorded as `*Audit resolution (2026-08-05): ...*` on individual
entries (radar point-in-time, blaster keep-16, ladder intent, turn-boost symmetry,
degradation-by-repair-capacity, shield-collapse-kept, repair-redirect/mid-repair/persist,
repair-disable-false-alarm, min_win_rate flag-not-block, single-lock, load/launch
ammo economy, patterned-barrel fire-available, committed-heading vs collision, waypoint
debug-tool, static-vs-dynamic hazard, sensors selection, power range-revert + drop elevation +
budget-aware AI, arc-bearing family doctrine, torpedo-armed badge, red-alert-no-hull,
objectives single-ship, bridge-alert-on-damage ordering, missing-AI-rule-kept).