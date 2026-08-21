# Falling Skyway — what a 100 % AI-backfilled crew actually achieves

Findings from a survey of `assets/worlds/falling_skyway.toml`, the ship-level AI
fragment library, and the scenario's ops/comms/handlers, done to answer: *can a
ship whose every console is AI-backfilled play this scenario, and which
objectives are out of its reach?*

[ai] This document is AI-origin (produced by the survey agent). It cites the
authored state as it stands, and flags every unratified number it leans on.

## Method

- Read the three AI fragments the Alliance Destroyer composes:
  `fragments/ai/fleet_baseline.toml` (10 policies + 5 selectors),
  `fragments/ai/captain_alliance.toml` (the captain policy), and
  `fragments/ai/movement_attack_pass.toml` (the three travel axes).
- Read the destroyer's `[operations]` capability table
  (`assets/entities/alliance_destroyer.toml:1489-1640`).
- Read the scenario's Act 1 / Act 2 handlers, deadlines, anchors and comms
  nodes in `assets/worlds/falling_skyway.toml`.
- Grepped the repo for every producer of `StartOperation` and for any
  `operations_console` AI surface.

## The AI command surface (what a console on auto can actually emit)

Every verb an AI backfill can order, and where it is declared:

| Console | Verb(s) | File:line |
|---|---|---|
| helm `engines_ai` | `actuate_desired_travel` (fly the planner's travel) | `fleet_baseline.toml:72-78` |
| helm `steering_ai` | `actuate_desired_facing` (yaw to desired facing) | `fleet_baseline.toml:90-96` |
| helm `lateral_ai` | `actuate_lateral_thrust` | `fleet_baseline.toml:101-107` |
| helm `vertical_ai` | `actuate_vertical_thrust` | `fleet_baseline.toml:113-119` |
| helm `impulse_ai` | `engage_impulse` | `fleet_baseline.toml:126-132` |
| helm `boost_ai` | `idle = true` (no AI boost) | `fleet_baseline.toml:141-142` |
| shields `ai_policy` | `focus_shield_arc` (arc focus, damage-window rules) | `fleet_baseline.toml:149-165` |
| power `ai_policy` | `set_power_group_allocation` (helm/weapons shed ladder, shields reserved 2) | `fleet_baseline.toml:286-396` |
| comms `ai` | `respond_to_message`, **`response_index = 0`** (answer any open hail with its first option) | `fleet_baseline.toml:398-405` |
| weapons `ai` | `bring_phasers_to_bear` / `bring_blasters_to_bear` / `bring_torpedoes_to_bear` (channel-3 arc-bearing) | `fleet_baseline.toml:455-473` |
| captain `ai` | `set_red_alert` (true/false) — the ONLY channel | `captain_alliance.toml:40-79` |
| movement (3 axes) | `actuate_desired_*`, `hold_committed_heading`, `hold_recovery_orbit`, `pivot_to_reengage`, `engage_boost` — a 9-state **combat** machine | `movement_attack_pass.toml` |

Plus the five target selectors (`fleet_baseline.toml:407-631`): sensors, weapons,
navigation (objective waypoints only), repair (own damage stations), comms
hail (hail-independent, band-scored).

**The decisive negative:** there is **no `operations_console`, no operations AI
policy, and no way for AI to emit `StartOperation`**. `StartOperation` has exactly
two producers in the repo — a *human* captain console (`gui/action-map.js:204`)
and *world script* (`src/world/script/effects.rs:410`). The destroyer owns all four
capability verbs (`alliance_destroyer.toml:1520-1640`) but nothing in any fragment
can order them. `tow`, `stabilise`, `field_repair` and `transfer` are therefore
strictly crew-only today.

## Finding 1 — Act 1 completes without the crew doing anything

All three Act-1 objectives resolve on world/timer events, not crew action
(`falling_skyway.toml`) :

- **`obj-a1-survey`** completes when `skyway_survey_due` fires at **t=90**
  (`on_survey_due`, line 1869). Pure timer.
- **`obj-a1-corridor`** completes when the *NPC* `hauler_lark` clears
  `corridor_gate` autonomously (`on_lane_open`, line 1861).
- **`obj-a1-triage`** completes when `skyhook_lift_capable` clears — the t=40
  `tether_slip` drops the head 48→42, under the authored 45 % lift line
  (`on_lift_tolerance_lost`, line 1840).

The header states the design at `:84-86`: *"Anything still pending when the
survey falls due is FAILED at the act boundary."* A fully silent crew gets all
three objectives green. The Reach directives (station_keeping, then
`ladder_transit`) exist so an AI-backfilled helm "flies the mission's shape"
(`:72-74, 1885-1891`) — they are decorative for completion. **Act 1 proves
nothing about whether backfill works**; it passes with zero commands issued.

## Finding 2 — the strike DOES settle under baseline AI (by negotiation)

The comms policy answers every open hail with **response index 0**
(`fleet_baseline.toml:398-405`). Walking the Act-2 dialogue tree with "always
pick the first option":

1. `committee_hails` → first response `listen` (`falling_skyway.toml:1984-1986`)
   → `on_committee_listen` → `committee_terms`.
2. `committee_terms` first call: `[promise_passage, promise_records, stall]` →
   picks `promise_passage` (line 2039).
3. second call: `[promise_records, stall]` → picks `promise_records` (line 2046).
4. third call: both promises on the ledger, `ground >= 2`, so
   `call_the_vote` is pushed ahead of `stall` (line 2062-2068) → picked → `on_settle`
   → 20 s later `skyway_strike_settled` (line 2118-2123) → `on_strike_settled`
   (line 1854) settles the strike and completes `obj-a2-line`.

So a baseline backfill **negotiates the strike closed in three comms picks**,
with no evidence file and no casualties — the good ending, reached by accident
of first-pick ordering. Two preconditions: the destroyer must be within the
committee's comms range when the hail opens at t=90 (yes from `station_keeping`,
per `:1887-1891`), and Comms must not be jammed/destroyed (guards at
`fleet_baseline.toml:403`).

## Finding 3 — operations are unreachable, so the storm and the collapse are unwinnable-by-AI

Because no policy can emit `StartOperation`:

- **`obj-a2-rescue`** (the tow) — cannot be started. The approach
  (`obj-a2-approach`, Reach `lyra_drift`, priority 70) IS flown by AI and
  auto-completes on arrival (explicitly noted at `falling_skyway.toml:2392-2410`),
  but the tow itself never happens. The Lyra is simply not under control by
  `lyra_clear_due` (t=192) → `obj-a2-rescue` fails, she is destroyed, and the
  mandatory `obj-a2-loss-report` posts (`:2496-2540`).
- **Act 3 (the head coming down)** — `stabilise` cannot be ordered, so the
  skyhook's strain decay is never arrested and it hits its structural floor at
  t=378 (the collapse branch). No AI-crewed path avoids the epilogue.
- **The transfer window (#1042)** — `transfer` cannot be ordered; even with the
  strike settled, no backfill stands a transfer up.

What a pure-AI run therefore **can** achieve (objectives that resolve on
timers, auto-completions, or first-pick comms):

- Act 1: all three (Finding 1).
- `obj-a2-line` (strike settled by negotiation, Finding 2).
- `obj-a2-approach` (auto-complete on arrival at the Lyra).
- `obj-a2-lee` (Reach `storm_lee`, priority 75, posted at `lyra_clear_due`
  `:2511-2519`).
- `obj-a2-storm` and `obj-a2-shelter` (auto-complete at `storm_passed`, `:2553-2558`,
  if no traffic died — traffic auto-diverts).
- `obj-a1-loss-report` (loss report, `:2534-2538`) — **pending authoring of
  Control's comms node; no completion path is authored yet** (the report to
  Control is #1039's business per `:1969-1970`).

What it **cannot** achieve: `obj-a2-rescue`, anything on Act 3 that needs
`stabilise`, the transfer window, the corroboration/blob evidence route, and the
Havelock confrontation.

## Act 2 — does the pure-AI ship survive the storm?

The authoritative crossing arithmetic lives in `region_radiation_band.toml:54-59`
([ai] unratified; effective damage **2.5 hull pts/s** after `shield_pierce = 0.5`,
destroyer 300 hull, 18 u/s clear / 10.8 u/s in-band):

| Path | Exposure | Damage | Verdict |
|---|---|---|---|
| cross one 520-unit band | ~48 s | 120 pts | survives |
| work the rescue, then fly for the lee | ~79 s | 198 pts | survives, hurt |
| loiter the whole three-band sweep | ~124 s | 310 pts | **DESTROYED** |

The pure-AI timeline (best case, no tow attempted):

- t=100 `storm_front_due`: `obj-a2-approach` posts (Reach `lyra_drift`,
  prio 70, `:2402-2410`). An AI helm flies it. `lyra_drift = [60,0,-560]`
  (`:821`); from `station_keeping = [180,0,170]` (`:722`) that is roughly 750 u
  ≈ 40 s of travel → arrives ~t=140, inside the first-arriving band envelope.
- t=145 band 1 (`storm_band_north = [0,0,-760]`, r 260, `:814` + template r 260):
  the Lyra's drift point is ~200 u from the band centre → the AI-held ship sits
  **inside** band 1 from ~t=145.
- t=192: band 2 is up (`storm_band_gate = [0,0,-420]`, `:815`); `lyra_clear_due`
  posts `obj-a2-lee` (Reach `storm_lee = [-560,0,-300]`, `:825`, prio 75).
  The helm leaves the Lyra → exposure until it clears the sweep toward the lee.
- t=225 band 3 (`storm_band_head = [0,0,-80]`, `:816`), retires t=265; lee is
  south-west of all three centres.

The outcome lands between the second and third table rows: the ship holds at the
Lyra from ~t=140 to t=192 (~50 s in or near band 1) then flies for the lee
(another ~30-40 s, potentially through band 2). That is on the order of
**~80-110 s exposure → ~200-270 pts → survives hurt or dies**, tightly dependent
on approach speed and the exact hold point. A ship that is slow to start the
approach (or that loiters at the Lyra waiting for a tow nobody can order) crosses
into the DESTROYED row. This is the one survivability result in the scenario
that is timing-sensitive for backfill, and the agents' numbers are unratified.

## Gap summary and what would close each gap

| Gap | Evidence | Would require |
|---|---|---|
| AI cannot run operations (tow/stabilise/field_repair/transfer) | no ops policy, no `StartOperation` emission path | an operations AI policy surface + wiring `emit_ai_command` → `StartOperation` at the captain system (new feature; scope as its own issue) |
| AI always takes the first comms response | `fleet_baseline.toml:405` | a comms response-*selection* heuristic (e.g. prefer `important`, avoid `stall`), or per-scenario authored response order |
| Act 1 is a free pass for any behaviour | `falling_skyway.toml:1861-1877` | intended skeleton behaviour (`:77-82`: the triage beat becomes a captain *decision* under #1035); a test asserting "Act 1 green with an idle crew" would document it deliberately |
| `obj-a2-loss-report` has no human path either | `:2534-2538`; no Control node authored | #1039 (the report to Control) fills it in |

## Finding 4 — the survey is a timer, not a task, and the good settle needs no evidence

Follow-up exploration of "Act 1 and Act 2 are passed accidentally" — what the
survey *should* be, what evidence the scenario already authored, and where the
gates actually sit ([ai] exploration; every ref verified against the file).

### The scan surface (what "doing the survey" could be built on)

- **The destroyer has a real scan suite.** `[scan]` at
  `assets/entities/alliance_destroyer.toml:527-552`, riding the `shields` power
  group at `min_power_level 2`. Two bands: `detailed` (max_range 120,
  condition_step 0.01) and `coarse` (max_range 260, condition_step 0.25,
  no capacities).
- **A scan is a first-class world event.** `scanned_flag(subject_id)` composes
  `scan.<id>.taken` (`src/science/scan.rs:465-467`); the latch in
  `src/science/server.rs:360-373` raises it in the base-world flag store and
  queues `WorldEvent::FlagSet`. Scenario `on_flag_set("scan.<id>.taken", …)`
  handlers therefore fire off a real sensor action. The scan reads only
  *published* infrastructure (`InfrastructureSnapshot::from_state` is #1025's
  gate, `server.rs:377-386`); `publish` defaults to true (`condition.rs:66-67`),
  so `skyhook`, `depot_ladder_a` and `depot_ladder_b` are all scannable unless
  authored off the wire.
- **The scenario already consumes exactly ONE scan**: `on_flag_set(
  "scan.depot_ladder_b.taken", "on_ladder_b_read")` (`falling_skyway.toml:2628`)
  plus the reverse-order `on_flag_cleared("depot_b_meets_certified_load", …)` at
  `:2629`. Both call `read_the_file_against_the_rung` (`:2652-2662`): it needs
  the scan *and* the certified-load flag clear, then files
  `…evidence.ladder_b_maintenance_file` under `records` and raises
  `skyway_records_diff_found`. Order-independent by design (`:2603-2607`).
- `depot_b_meets_certified_load` fails_below 0.6 against the authored 34/100
  (`:1080-1083`), so Depot B is below its standard from the first tick — the
  comparison beat is live from t=0, gated only on the crew scanning.

### The evidence chain (what a good outcome could demand)

- BRIEFING provenance on world load: the maintenance record, filed before the
  crew act (`file_the_ladder_b_record`, `:2634-2645`). A crew who cannot tell a
  briefing from a reading cannot argue with the briefing (`:2631-2633`).
- The scan diff above produces the RECORDS finding (`ladder_b_maintenance_file`)
  — deliberately the same entry the operator hands over in Act 2, so the
  second route is a silent no-op (`:2611-2617`).
- The watcher: `skyway_records_diff_found` → `on_the_diff_lands` (the second
  gate, `:2713`).
- Worker corroboration: `on_rigger_ask` files `ladder_b_worker_account` under
  `dialogue` + `skyway_worker_corroboration_obtained` (`:2795-2804`).
- The unlock: `on_corroboration_filed` completes `obj-a2-corroborate` and — if
  the crew hold the maintenance file (records) AND the worker account
  (dialogue) — raises `skyway_confront_unlocked` and posts `obj-a3-confront`
  (`:2857-2873`). Dual provenance is deliberate (`:2845-2851`).
- Campaign side: `campaign.skyway.evidence.*` is exclusive
  corroborated > records > none (`:4969-4981`).

### Where the gates actually sit today (and what is not gated)

- **`show_file` (the negotiation's evidence branch) is gated, and worth 1 ground
  — equal to a promise.** It only appears while the dossier holds
  `ladder_b_maintenance_file` (`:2055-2061`); showing it is "the only thing in
  this conversation the workers can check" (`:2100-2103`).
- **`call_the_vote` is gated on `ground >= 2`, NOT on evidence** (`:2062-2068`).
  Two promises reach the vote, so the strike settles by negotiation with no
  file shown — the pure-AI first-pick walk lands here (Finding 2).
- **`obj-a1-survey` completes on the timer at t=90, unconditionally**
  (`on_survey_due`, `:1869-1870`). No scan, no position, no report required.
- **Keeping the records promise is evidence-gated.** `skyway_surface_records`
  is kept iff `skyway_records_put > 0` (`:4778-4783`), and `put_the_file` is
  offered only while the dossier holds the maintenance file (`:4350-4351,
  :4375-4377`). So an empty-handed crew can settle the strike (good) but
  silently break the records ledger — the "accidentally good" run is only
  half-clean even before the collapse.

### What this means for "we should need to do the survey, and need evidence"

- **The survey task is authorable now.** Requiring `obj-a1-survey` to complete
  on scans (e.g. `scan.skyhook.taken` + `scan.depot_ladder_a.taken` +
  `scan.depot_ladder_b.taken`) or on a survey report to Control is pure
  scenario content — every primitive (scan flag, world event, dossier entry)
  exists. This is the deliberate design fork: a survey that matters flips
  Act-1-green-from-idle into the reverse, and an AI that cannot scan (the
  fragments emit no scan verb) becomes unable to pass it — the opposite failure
  mode of today's free pass. A scan-capable AI would be a new policy surface,
  scoped separately.
- **The good settle could require evidence** by raising `call_the_vote` to need
  `showed_the_file` (or corrupt `ground`'s basis), forcing a no-evidence crew to
  the force path — the trade already authored as "costs people, a relationship,
  and the corroboration route" (`:2128-2131`).
- **The clean ledger is already the scaffold** for this: break nothing, and the
  records promise already cannot be kept without the scan. What is missing is
  the up-front requirement, not the consequence.

## The locked design — Falling Skyway as a mission the verbed AI completes 80-95 %

This is the ratified follow-on to the survey above, reached by grilling the design
tree branch by branch. All numbered decisions are the human's; the prose that
rounded them out is AI-origin. The goal restated: make Falling Skyway require
real interaction, then give the AI the verbs (scan, operations, traffic ordering)
so a fully-backfilled crew completes the mandatory objectives semi-reliably —
**80-95 % across seeds** — where today's AI manages almost none of it, and an
idle crew fails loudly in every act.

### The spine (mandatory set)

The five-act load-bearing core a crew must complete: **survey, triage, strike
settled, rescue (Lyra towed), storm survival, stabilise the head**. The transfer
window (#1042) is **not** mandatory — it is the good-ending differentiator.
Idle crew fails these loudly across all acts.

### Suspense mechanics — passive availability + timer escalation, no act gates

- The `act` flag machinery (`flags.act` at `:1802,1878,2563`) is **demoted to a
  narrative counter**. No thread gates on shared act flags; each posts on its own
  trigger.
- Every task is **passively startable from t=0** — crew-initiated hail or scan
  opens it early (survey scans, committee/civilians, hailable Lyra at `:1491-1508`).
- Timers **escalate, never close**: the deadline hardens the tree (frustrated
  committee, costlier options, Havelock counter-offer) but the crew-initiated
  path stays open. Idempotency via "did the crew already engage" flags, like the
  `skyway_strike_settled` guard (`:1794-1797`).
- **Pre-emption is remembered**: early work (scan early, tow the Lyra at t=300,
  settle the strike before the deadline) is recorded and the objective
  **auto-resolves when it posts** — the full 25-minute schedule stays on stage;
  early work relieves pressure rather than shortening the mission.

### Clock — ×4 the times, 25 minutes of content

Multiply **all** scenario clocks by 4: `tether_slip` 40→**160**, `survey_due`
90→**360**, `storm_front` 100→**400**, bands 145/185/225→**580/740/900**,
`lyra_clear` 192→**768**, `storm_passed` 268→**1072**, `transfer_window`
400→**1600**, `window_closes` 470→**1880**, `skyhook_failure` 382→**1528**.
Fixed `schedule.after` beats (2-30 s) stay unscaled — they are real work beats,
not clocks. The act boundary stretches to ~t=180 for the tour (spawn→B→A→head ≈
2244 u ≈ 125 s at 18 u/s fits inside it).

### Threads — interleaved, each on its own trigger

Content split (through-line = 1/2/4/5/7, side-pressure = 3/8, back-half = 6):

| # | Thread | Role | Trigger |
|---|---|---|---|
| 1 | survey | through-line | crew scans + report |
| 2 | diplomacy / strike | through-line | opens mid-survey |
| 3 | traffic control | side-pressure | order-divert civilians (human-nav decision, `OrderCivilian`/`order_divert_route`, `:2439-2441`) |
| 4 | rescue | through-line | opens on storm front / early tow |
| 5 | storm survival | through-line | storm front |
| 6 | stabilise / collapse | back-half | head comes down after rescue-clear or deadline |
| 7 | transfer window | through-line | booked after rescue; the good-ending differentiator, compute over the #1042 ledger `:3477-3523` |
| 8 | evidence / confrontation | side-pressure | worker corroboration + scan diff |

Survey + bad news → strike stews mid-survey → storm mid-negotiation → rescue and
window crowd the back half. (Q16) The **good ending is the clean-ledger
benchmark**: the mandatory set completed *plus* no civilian traffic lost,
commitments kept, skyhook held, evidence filed. The campaign flags (`:4960-5019`)
are the literal scoreboard. The ledger stays the engine of *which* two of three
claimants lift (ceiling 52 vs 66 — a clean run still has a real moral choice).

### The survey beat (act 1 becomes a real objective)

`obj-a1-survey` completes on: **scan all three structures** (`scan.skyhook.taken`
+ `scan.depot_ladder_a.taken` + `scan.depot_ladder_b.taken`) **and** a report
pickup on the **Control comms channel that resolves the objective itself**
(the seam `put_the_file` already reuses, `:4345-4396`). All three scans gate the
report pickup (`can_file`-style). Any scan band works (`alliance_destroyer.toml:527-552`
— detailed or coarse); a scan raising `scan.<id>.taken` fires `on_flag_set`.
Stretch the act boundary to ~t=180 for the tour.

### The AI surface (the verb batch, scoped separately)

- **(Q17) New `AiDirective` kinds carry the verbs**: `Scan { target }`,
  `Operate { verb, target_uuid }`, `Order { target, route }` — mirroring the
  `Hail`/`Dock` precedent. New `SystemAffinity` entries route them into the per-
  system scored-objective pools (`score_doctrine_pool`); the new emitters —
  sensors AI scan (`src/ship/sensors.rs:633-650` has the applier, no policy),
  an operations emitter at the captain system, a navigation/captain order-
  civilian emitter — consume them exactly like Comms consumes `Hail`
  (`SystemAffinity::Comms`, `core/messages.rs:4295-4298`). This makes the AI
  "AI-addressable by construction": the objective *is* the directive the emitter
  serves.
- **(Q14) Objectives-driven emitters**: each mandatory objective carries enough
  authored info (verb + target UUID) for the AI to emit its verb. Not
  verb-gating or locks — give the AI the verbs, then let concurrency overwhelm
  it. Keep humans and AI symmetric; nothing branches on actor identity, only on
  timing of engagement.
- Today's AI at ≈0 % on the mandatory set is the regression baseline.

### Measurement

Objectives-completion rate on the **mandatory set, 80-95 % across seeds**, driven
through `scripts/balance-runs.mjs` seed sweeps. Needs a per-objective status
rollup in `src/headless/report.rs` (`RunReport` at `:180-203` has
outcome/phase/ship/damage but no per-objective set).

## Files consulted

- `assets/worlds/falling_skyway.toml` — scenario, handlers, deadlines, anchors, comms.
- `assets/entities/fragments/ai/fleet_baseline.toml`
- `assets/entities/fragments/ai/captain_alliance.toml`
- `assets/entities/fragments/ai/movement_attack_pass.toml`
- `assets/entities/alliance_destroyer.toml` (`[operations]` at 1489-1640)
- `assets/entities/region_radiation_band.toml`
- `gui/action-map.js` (`StartOperation` producer, line 204)
- `src/world/script/effects.rs` (`StartOperation` script producer, line 410)
- `src/operations/server.rs` (admission, line 242; start, line 309)