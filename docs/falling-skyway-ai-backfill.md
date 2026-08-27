# Falling Skyway — what a 100 % AI-backfilled crew actually achieves

Findings from a survey of `assets/worlds/falling_skyway.toml`, the ship-level AI
fragment library, and the scenario's ops/comms/handlers, done to answer: *can a
ship whose every console is AI-backfilled play this scenario, and which
objectives are out of its reach?*

[ai] This document is AI-origin (produced by the survey agent). It cites the
authored state as it stands, and flags every unratified number it leans on.

> **Historical baseline note (pre-#1131 and pre-#1132).** The audit findings and measured
> timelines below were recorded against the original 40–470 second scenario
> clock and are preserved as that baseline. Issue #1131 subsequently authored
> the 160/360/400/580/740/768/900/1072/1528/1600 schedule, ratified the window
> close at 1800 seconds, and slowed tether strain to 0.5 points per ten-second
> beat (an unattended structural floor around t=1512). Issue #1132 then replaced
> the timer-granted Act-1 survey with three scans plus a Control report. Treat
> Findings 1–4 as measurements and design exploration of that earlier build,
> not current runtime behaviour.

> **Current traffic note (#1134/#1141).** The storm front no longer diverts civilians
> automatically. Navigation now sees a world-authored **ORDER TO STORM SHELTER**
> button on Meridian, Lark and Pell; one ordinary `OrderCivilian` diversion per
> craft before band one puts that craft on the shelter route for all three
> bands. An idle human-held Navigation console loses exposed traffic immediately
> and by name. A fully backfilled bridge consumes three payload-bearing `Order`
> objectives and emits those same admitted commands; #1134's human surface and
> authoritative traffic state remain the sole downstream path. The live fronts
> are narrow, corridor-long boxes (260 lateral by 1,100 longitudinal half-
> extents), so exposed traffic is physically taken while the westward shelter
> and eastern ladder routes remain clear; the historical sphere arithmetic
> below remains the explicitly preserved pre-#1131 baseline.

> **Current evidence note (#1136).** Havelock's maintenance-file copy remains
> useful ground in the strike negotiation, but it does not set
> `skyway_records_diff_found` and cannot stand in for scanning Ladder B. Only a
> negotiated strike plus that genuine scan diff lets the rigger corroborate it.
> Corroboration opens a sticky, range-gated Control filing thread and the later
> Havelock confrontation; it completes neither filing nor the optional evidence
> objective. The crew must submit Control's explicit filing response, which puts
> the diff and worker account on Control's dossier, sets `skyway_records_put`,
> and completes the objective. It files that two-part bundle unedited rather
> than moving the maintenance record's source rows. Force burns and visibly
> fails an unfiled route; filing first makes a retained force response return
> only the closed acknowledgement. Confrontation does not file it; mission
> finalisation closes both stale-response authorities and the explicit
> `skyway_evidence_filing_open` state before commitments, campaign facts and
> debrief copy freeze, then fails any active unfiled objective. A stale response
> after either closure receives only the closed acknowledgement. The thread is
> sticky across travel and range changes, while ordinary `ClearComms` remains a
> deliberate dismissal of the inbox and does not auto-reopen this optional route.

> **Current external-work note (#1162/#1164/#1135).** The audit below predates
> the retirement of `StartOperation`. Current Backfill consumes objective
> directives for `Tow`, `Stabilise`, `Transfer`, `FieldRepair`, and `Order`,
> driving the same tractor, dock, umbilical, repair-dispatch and traffic controls
> as humans. Falling Skyway's optional `Transfer` directive makes Helm
> dock to the receiving manifold and Engineering flow reserve fuel; completion
> is the target's capacity-backed threshold. The edge is remembered if work
> happens early and an unworked objective fails at the existing t=1600 opening.
> The later beneficiary allocation is a distinct protected decision and is not
> chosen by Backfill. Findings 1–4 and the old command-surface/gap tables remain
> historical evidence, not a description of the current runtime.

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

## Historical AI command surface (before the crew-owned system emitters)

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

**Historical decisive negative:** there was **no `operations_console`, no operations AI
policy, and no way for AI to emit `StartOperation`**. `StartOperation` has exactly
two producers in the repo — a *human* captain console (`gui/action-map.js:204`)
and *world script* (`src/world/script/effects.rs:410`). The destroyer owns all four
capability verbs (`alliance_destroyer.toml:1520-1640`) but nothing in any fragment
can order them. `tow`, `stabilise`, `field_repair` and `transfer` are therefore
strictly crew-only in that historical build.

## Finding 1 — Act 1 completed without the crew doing anything (historical)

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

**Closed by #1132.** The survey now posts on the t=160 tether slip and completes
only after the skyhook and both ladder depots have been scanned and the report
has been filed with Control. A silent run reaches t=360 with that objective
failed and an urgent Control transmission; corridor and triage retain their
existing world-event outcomes.

## Finding 2 — the strike did settle under baseline AI by first-pick accident (historical)

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

**Closed by #1133.** The strike is hailable from t=0 and posts independently at
t=160. Silence hardens the committee at t=120, before the shipped Backfill policy
receives that post; its first admitted response then preserves stage 1 and stops
the t=300 rung. The soft tree still accepts any two of three grounds, while a
hardened vote requires all three, so the old two-promise path no longer settles
the floor by itself. This observation does not claim a final Backfill outcome:
the policy can still encounter the file path, and whole-scenario success remains
the later AI-backfill evaluation's job. Both negotiation and force remain
available, and the lowered workforce disposition makes late force costlier.

## Finding 3 — historical: operations were unreachable

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

What a pure-AI run therefore **could** achieve in the historical baseline
(objectives that resolved on timers, auto-completions, or first-pick comms):

- Act 1: all three (Finding 1; the survey free pass was removed by #1132).
- `obj-a2-line` (strike settled by negotiation, Finding 2).
- `obj-a2-approach` (auto-complete on arrival at the Lyra).
- `obj-a2-lee` (Reach `storm_lee`, priority 75, posted at `lyra_clear_due`
  `:2511-2519`).
- `obj-a2-storm` auto-completes at `storm_passed`; `obj-a2-shelter` completes
  only if Navigation issued the three authored shelter orders and no traffic
  died. The front itself issues no diversion.
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

## Historical gap summary

| Gap | Evidence | Would require |
|---|---|---|
| AI cannot run operations (tow/stabilise/field_repair/transfer) | no ops policy, no `StartOperation` emission path | an operations AI policy surface + wiring `emit_ai_command` → `StartOperation` at the captain system (new feature; scope as its own issue) |
| AI always takes the first comms response | `fleet_baseline.toml:405` | a comms response-*selection* heuristic (e.g. prefer `important`, avoid `stall`), or per-scenario authored response order |
| Act 1 survey was a free pass for any behaviour | historical `on_survey_due` implementation | **Closed by #1132:** three scans plus Control pickup; the idle headless path now asserts a loud failure |
| Strike settlement was gated by the survey deadline and late first-pick promises produced a good ending | historical `on_survey_due` and two-of-three tree | **Closed by #1133:** t=0 hails, independent t=160 post, t=120/t=300 hardening, and a hardened three-of-three vote |
| `obj-a2-loss-report` has no human path either | `:2534-2538`; no Control node authored | #1039 (the report to Control) fills it in |

## Finding 4 — the survey was a timer, not a task, and the good settle needs no evidence (historical)

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

### The evidence chain (current runtime after #1136)

- BRIEFING provenance on world load: the maintenance record, filed before the
  crew act (`file_the_ladder_b_record`, `:2634-2645`). A crew who cannot tell a
  briefing from a reading cannot argue with the briefing (`:2631-2633`).
- The scan diff above produces the RECORDS finding (`ladder_b_maintenance_file`)
  and raises `skyway_records_diff_found`. Havelock can hand over the same text
  for negotiation, but that route deliberately does not raise the scan flag.
- The watcher: `skyway_records_diff_found` → `on_the_diff_lands` (the second
  gate, `:2713`).
- Worker corroboration: `on_rigger_ask` files `ladder_b_worker_account` under
  `dialogue` + `skyway_worker_corroboration_obtained` (`:2795-2804`).
- The unlock: `on_corroboration_obtained` requires the negotiated-strike flag,
  the genuine scan flag and both source findings. It raises
  `skyway_confront_unlocked`, posts `obj-a3-confront`, and opens one fixed
  `falling-skyway-evidence-filing` thread from Control. It leaves
  `obj-a2-corroborate` active.
- The filing: the range-gated `file_evidence` response calls
  `on_file_evidence`, which revalidates those gates at admission, appends the
  records diff and dialogue account to Control's dossier, raises
  `skyway_records_put`, and completes `obj-a2-corroborate`. The Havelock
  transfer-window shortcut is gone, and confronting Havelock preserves its own
  effects without setting the filed flag.
- The route state is explicit: corroboration opens
  `skyway_evidence_filing_open`; filing, force or mission finalisation closes it.
  The symmetric force admission gate rejects a retained response after filing,
  physical strike settlement, an earlier force order or mission finalisation.
  During the vote's physical stand-down the first irreversible response admitted
  wins, so neither order can produce both filed and forced state. Finalisation
  closes the filing state before resolving promises or writing campaign/debrief
  state, so an admitted stale response cannot rewrite the ending. `ClearComms`
  intentionally discards the live dialogue without recreating it; the optional
  objective remains unresolved and later fails visibly.
- Campaign side: `campaign.skyway.evidence.*` is exclusive
  corroborated > records > none (`:4969-4981`).

### Where the gates sat at audit time (and what was not gated)

- **`show_file` (the negotiation's evidence branch) is gated, and worth 1 ground
  — equal to a promise.** It only appears while the dossier holds
  `ladder_b_maintenance_file` (`:2055-2061`); showing it is "the only thing in
  this conversation the workers can check" (`:2100-2103`).
- **`call_the_vote` is gated on `ground >= 2`, NOT on evidence** (`:2062-2068`).
  Two promises reach the vote, so the strike settles by negotiation with no
  file shown — the pure-AI first-pick walk lands here (Finding 2).
- **At audit time, `obj-a1-survey` completed on the timer at t=90,
  unconditionally.** #1132 replaced that route with the three-scan plus Control
  pickup contract described below.
- **Keeping the records promise is evidence-gated.** `skyway_surface_records`
  promises that the genuine scan discrepancy and rigger's corroboration reach
  Control unedited; it does not promise to transfer Ladder B's maintenance rows.
  It is kept iff `skyway_records_put > 0`. That flag now has one route: Control's
  explicit filing response after genuine scan evidence and corroboration. An
  empty-handed crew can still settle the strike, but cannot silently manufacture
  a clean records ledger from Havelock's copy or the confrontation. The separate
  witness-protection promise remains open through filing and Havelock's
  confrontation, then resolves at mission close against whether Tacket was named
  to Havelock.

### What this means for "we should need to do the survey, and need evidence"

- **The survey task was authorable as scenario content and #1132 implemented
  it.** `scan.skyhook.taken`, `scan.depot_ladder_a.taken`, and
  `scan.depot_ladder_b.taken` gate a report response on Control. Filing raises
  `skyway_survey_reported`; an AI that cannot scan cannot pass it. A scan-capable
  AI remains a separately scoped policy surface.
- **The good settle could require evidence** by raising `call_the_vote` to need
  `showed_the_file` (or corrupt `ground`'s basis), forcing a no-evidence crew to
  the force path — the trade already authored as "costs people, a relationship,
  and the corroboration route" (`:2128-2131`).
- **The clean ledger is already the scaffold** for this: break nothing, and the
  records promise already cannot be kept without the scan. What is missing is
  the up-front requirement, not the consequence.

## The locked design — Falling Skyway as a mission the verbed AI can complete

This is the ratified follow-on to the survey above, reached by grilling the design
tree branch by branch. All numbered decisions are the human's; the prose that
rounded them out is AI-origin. The goal restated: make Falling Skyway require
real interaction, then give the AI the verbs (scan, tow/stabilise/transfer/repair,
traffic ordering)
so a fully-backfilled crew can complete the mandatory objectives without hidden
shortcuts. The later rough first-pass balance band is **25-95 % across seeds**;
this document does not claim a new sweep, and the historical audit measured
almost none of the mandatory set. An
idle crew fails loudly in every intensity era.

### The spine (mandatory set)

The load-bearing core a crew must complete across the three intensity eras is:
**survey, triage, strike
settled, rescue (Lyra towed), storm survival, stabilise the head**. The transfer
window (#1042) is **not** mandatory — it is the good-ending differentiator.
Idle crew fails these loudly across all eras.

### Suspense mechanics — passive availability + timer escalation, no act gates

- The `act` flag machinery (`flags.act` at `:1802,1878,2563`) is **demoted to a
  narrative counter**. No thread gates on shared act flags; each posts on its own
  trigger.
- Every task is **passively startable from t=0** — crew-initiated hail or scan
  opens it early (survey scans, committee/civilians, hailable Lyra at `:1491-1508`).
- Timers normally **escalate rather than remove interactions**: the deadline
  hardens the tree (frustrated committee, costlier options, Havelock
  counter-offer) but the crew-initiated path stays open. The survey deadline is
  the explicit scoring exception: it fails the unfiled objective loudly while
  leaving the report interaction available for later records and consequences.
  Idempotency comes from "did the crew already engage" flags, like the
  `skyway_strike_settled` guard.
- **Pre-emption is remembered**: early work (scan early, tow the Lyra, settle
  the strike, stabilise the head, or prime the transfer manifold) is recorded
  from authoritative target state and the objective
  **auto-resolves when it posts** — the full 25-minute schedule stays on stage;
early work relieves pressure rather than shortening the mission.

**The strike portion is implemented by #1133.** The committee and Havelock are
hailable from t=0. `post_strike_objectives` runs at the t=160 tether slip and
reuses `post_remembered_objective`, completing work performed before the post
and failing an already-closed corroboration route. The first admitted response
on either strike channel raises `skyway_strike_engaged`; an unanswered hail does
not. Deterministic timers at t=120 and t=300 stop on that memory. The first is
deliberately earlier than the t=160 incoming post: shipped Backfill encounters
stage 1, responds through the same admitted command path, and prevents stage 2.
Each timer
lowers the workers' base disposition by ten points (25→15→5). The soft tree
requires any two of safe passage, records, and file; a hardened tree requires
all three. At disposition 5, immediate force costs four casualties instead of
the soft tree's two. No hardening handler removes negotiation, stall, refusal,
warning, or immediate-force responses.

### Clock — pre-#1131 proposal and ratified close

The pre-#1131 proposal was to multiply **all** scenario clocks by 4:
`tether_slip` 40→**160**, `survey_due`
90→**360**, `storm_front` 100→**400**, bands 145/185/225→**580/740/900**,
`lyra_clear` 192→**768**, `storm_passed` 268→**1072**, `transfer_window`
400→**1600**, `window_closes` 470→**1880**, `skyhook_failure` 382→**1528**.
Issue #1131 ratified that schedule with the close brought inside the PASM envelope at
**1800**, not 1880. Fixed `schedule.after` beats (2-30 s) stayed unscaled — they
are real work beats, not clocks. The narrative counter advances at t=360,
giving the opening tour (spawn→B→A→head ≈ 2244 u ≈ 125 s at 18 u/s) its old
readable boundary without gating a thread on it.

### Threads — interleaved, each on its own trigger

Content split (through-line = 1/2/4/5/7, side-pressure = 3/8, back-half = 6):

| # | Thread | Role | Trigger |
|---|---|---|---|
| 1 | survey | through-line | crew scans + report |
| 2 | diplomacy / strike | through-line | opens mid-survey |
| 3 | traffic control | side-pressure | one authored `OrderCivilian` shelter diversion per endangered craft before band one; the route clears all three bands |
| 4 | rescue | through-line | opens on storm front / early tow |
| 5 | storm survival | through-line | storm front |
| 6 | stabilise / collapse | back-half | head comes down after rescue-clear or deadline |
| 7 | transfer window | through-line | optional manifold run-up may complete early; t=1600 fails it if unworked, then the protected allocation remains the good-ending differentiator |
| 8 | evidence / confrontation | side-pressure | worker corroboration + scan diff |

Survey + bad news → strike stews mid-survey → storm mid-negotiation → rescue and
window crowd the back half. (Q16) The **good ending is the clean-ledger
benchmark**: the mandatory set completed *plus* no civilian traffic lost,
commitments kept, skyhook held, evidence filed. The campaign flags (`:4960-5019`)
are the literal scoreboard. The ledger stays the engine of *which* two of three
claimants lift (ceiling 52 vs 66 — a clean run still has a real moral choice).

### The survey beat (the opening era becomes real crew work)

**Implemented by #1132.**

`obj-a1-survey` completes on: **scan all three structures** (`scan.skyhook.taken`
+ `scan.depot_ladder_a.taken` + `scan.depot_ladder_b.taken`) **and** a report
pickup on the **Control comms channel that resolves the objective itself**
(Control is transport and authority only; this does not file the separate
maintenance evidence or keep its promise). All three scans gate the
report pickup (`can_file`-style). Any scan band works (`alliance_destroyer.toml:527-552`
— detailed or coarse); a scan raising `scan.<id>.taken` fires `on_flag_set`.
The survey posts on the t=160 tether slip. `post_remembered_objective` consumes
the flag-backed `skyway_survey_reported` memory, so scans and filing completed
before that post resolve it immediately without moving the schedule. The live
narrative counter advances at t=360; an unfiled survey fails there with an
urgent Control message and exact missing-work flags.

### The AI surface (the verb batch, scoped separately)

- **Current `AiDirective` kinds carry the delivered verbs**: `Tow`, `Stabilise`,
  `Transfer`, `FieldRepair`, and `Order`, mirroring the existing `Hail`/`Dock`
  precedent. Station-specific emitters consume the shared scored-objective pool:
  Tractor, Dock, Umbilical, Repair and Navigation each issue their ordinary
  admitted control. A `Transfer` is a two-seat chain rather than a generic
  `Operate` command. `Scan` remains the dedicated Sensors slice (#1139).
- **(Q14) Objectives-driven emitters**: each actionable objective carries enough
  authored info (verb and target, plus route where required) for the AI to emit
  its verb. Not
  verb-gating or locks — give the AI the verbs, then let concurrency overwhelm
  it. Keep humans and AI symmetric; nothing branches on actor identity, only on
  timing of engagement.
- The ≈0 % result is the historical regression baseline, not current behavior.

### Measurement

Objectives-completion rate on the mandatory set can be driven
through `scripts/balance-runs.mjs` seed sweeps. Needs a per-objective status
rollup in `src/headless/report.rs` (`RunReport` at `:180-203` has
outcome/phase/ship/damage but no per-objective set).

## Files consulted

- `assets/worlds/falling_skyway.toml` — scenario, handlers, deadlines, anchors, comms.
- `assets/entities/fragments/ai/fleet_baseline.toml`
- `assets/entities/fragments/ai/captain_alliance.toml`
- `assets/entities/fragments/ai/movement_attack_pass.toml`
- `assets/entities/alliance_destroyer.toml` (current tractor, dock, umbilical and repair-dispatch tables; historical audit used the retired `[operations]` block)
- `assets/entities/region_radiation_band.toml`
- `src/tractor/`, `src/dock/`, `src/umbilical/`, and `src/console/repair/external_server.rs` (current crew-owned systems)
