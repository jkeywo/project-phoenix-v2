# Acceptance kit — #1440, the M4 busy-session run

**Status: preparation only. No box below is checked, and no human session
has been run or accepted by this document.**

Issue #1440 is PRD #1419's own closing instruction (story 16): *"As a GM or
scenario author, I want to complete a real busy-session acceptance with
clear recorded outcomes, so that facilitation remains understandable and
controllable."* It is the human exit gate for GM console M4 — Facilitation
Assistance — carrying PRD #1418's usability contract into that acceptance
rather than treating it as a later polish pass. Seven T3 issues (#1433–#1439,
this branch) each shipped its own feature and its own exhaustive automated
integration coverage. **This document does not re-derive or duplicate any of
it.** It is the single script one Game Master runs, once, to sit at a busy
desk and judge whether the attention queue, the workload picture and the
widgets a scenario author composed are actually legible and trustworthy
under real, simultaneous demand — not whether each producer taken alone has
a passing test.

Parent PRD: #1419, story 16; inherited PRD #1418 stories 23–27, 32–33.
Blocked by #1439, #1430 — both landed on this integration branch (see §1).

---

## 1. What is already integrated on this branch

Every row below is a landed commit on `claude/t3-prd-sub-issues-65d146`.
This kit assumes their behaviour rather than re-testing it; each issue's own
automated suite remains the authority for *its* feature, and is cited below
so a failure found during this run can be filed against the right owner.

| Issue | Commit | What it added | Its own automated coverage |
| --- | --- | --- | --- |
| #1433 | `63b0abd8` | `src/gm_attention.rs` foundation: `GmAttentionBand` (Urgent/Attention/Background), the peer-local `gm_attention` Host Channel projection; `gui/gm-attention-panel.js` (band groups, held/live reading, Open/Snooze) and `gui/gm-attention-filters.js` (private band/category/ship filters, the 60-second snooze); pending-Comms producer with `[[gm_comms_route]].attention_band` authored override | `tests/gm_attention.rs`, `tests/client/gm-attention-panel.test.js` |
| #1434 | `17da8135` | Eligible-authored-beats producer sharing `manual_fire_would_land` with the real Fire evaluator (no condition read twice, no Rhai side effect); `.attention_band("…")` Rhai sibling of `.pauseable()`/`.skip()` | `tests/gm_attention_beats.rs` |
| #1435 | `28a131b1` | Idle-NPC producer: `[gm_attention]` table (`idle_npc_grace_secs`, `idle_npc_band`, `idle_npc_disabled`), 30 simulation-second default grace read from the ship's own `top_directive` over its live scored objectives — the same pool Helm/Weapons act on | `tests/gm_idle_npc.rs` |
| #1436 | `f8be4f9e` | Quiet-time producer: one non-escalating Background row after 120 simulation seconds with no `ActionFeedback::Applied`, no sustained `AdmittedCommands`, no `ObjectiveChanged` — shares `[gm_attention]` with #1435 | `tests/gm_attention_quiet.rs` |
| #1437 | `a3fc3e7b` | `src/gm_health.rs`: public `gm_health` projection (Live/Paused/Stale/Recovering/Disconnected), no `HostSlot`/token on the wire; `GmAttentionCategory::StationHealth` fixed at Urgent, reachable by no authored key; `gui/gm-health-banner.js` unfilterable technical-banner seam that no snooze, filter or hold can touch | `tests/gm_health.rs` |
| #1438 | `8f51e5a1` | `src/gm_workload.rs`: `GmWorkloadLevel` (Backfill/Offline/Underused/Engaged/Overloaded) from distinct outstanding human demands (pending Comms, unflown Navigation clearance, queued Repair dispatch, task activations) — never input frequency; `gui/gm-workload-panel.js` evidence expander | `tests/gm_workload.rs`, `tests/client/gm-workload-panel.test.js` |
| #1439 | `7e550501` | `[[gm_role_preset.widget]]` typed world authoring (attention/workload/actions/note, closed `GM_WIDGET_TYPES`); `gui/gm-widgets-panel.js` composing the *existing* attention/workload controllers and the *existing* action buttons — never a second queue, never a new permission; reconnect keeps live filters/snoozes, a fresh session and a live preset switch both reset to authored defaults | `tests/gm_widgets.rs`, `tests/client/gm-widgets-panel.test.js` (26), `tests/smoke/gm-layout.spec.js` (200%-scale) |
| #1430 | `b0f104ee` | GM directing/performing desk (T2, PRD #930 M1–M3) carried into 200%/forced-colours/dense-list coverage, plus the GM Station-puppet mirroring the GM's own resolved presentation onto the puppeted document | `docs/acceptance/1432-platform-comfort.md` Task 4, `tests/client/gm-directing-performing-dense-content.test.js` |

Two things worth a reviewer's attention before booking a human:

- **No producer re-bands a live occurrence.** Every authored band
  (`[[gm_comms_route]].attention_band`, `.attention_band()` on a beat,
  `idle_npc_band`) is fixed at authoring time; `GmAttentionCategory::StationHealth`
  is a system-fixed `Urgent` literal (`src/gm_attention.rs:1082`), and quiet
  time is a system-fixed `Background` literal (`:1209`). The "newly Urgent
  escalation breaks snooze, an already-Urgent snoozed item waits the minute"
  reconciliation rule (PRD #1419 Implementation Decisions) is real and
  covered — `tests/client/gm-attention-panel.test.js`'s
  `'breaks on escalation to Urgent but not for a row that was already Urgent'`
  drives the front-end engine through a synthetic same-occurrence band change
  — but no shipped producer today generates that transition on one *live*
  occurrence. §5.3 below designs the live exercise around what the shipped
  producers can actually reproduce (an already-Urgent snooze that must wait
  the minute, and a fresh Urgent occurrence that must never be hidden by an
  unrelated snooze) rather than claiming an unverified live re-banding path.
- **Widget action buttons carry no new authority.** `GM_WIDGET_ACTION_IDS`
  (`src/world/config.rs:935`) is exactly `["gm-session-pause",
  "gm-session-resume"]` today. A widget button ACTIVATES the shipped DOM
  control by id — same confirmation category, same admission check, same
  `ActionFeedbackLifecycle` — so §5.9's "truthful outcomes" exercise is about
  reading Applied/No-op/Refused honestly, not about a widget-specific action
  family.

---

## 2. Cheap automated prechecks — run before booking the GM

None of these need a generated fixture or a build; they are the existing
per-feature suites, and running them fresh on the exact revision under test
is cheap insurance against booking a real person's time against a regression
the ordinary gate would have caught. Record the revision and the pass counts
in §6 before scheduling the human session.

```sh
cargo test --features headless --test gm_attention
cargo test --features headless --test gm_attention_beats
cargo test --features headless --test gm_idle_npc
cargo test --features headless --test gm_attention_quiet
cargo test --features headless --test gm_health
cargo test --features headless --test gm_workload
cargo test --features headless --test gm_widgets
npx vitest run tests/client/gm-attention-panel.test.js tests/client/gm-health-panel.test.js \
  tests/client/gm-workload-panel.test.js tests/client/gm-widgets-panel.test.js \
  tests/client/gm-role-presets.test.js
```

If a built bundle exists in your worktree, the same revision's dense-list and
200%-scale coverage is already exercised on the real engine without a human:

```sh
npx playwright test tests/smoke/gm-layout.spec.js
```

If any command above fails, do not proceed to §4: file the failure against
the owning issue from §1 and re-run this section after a fix lands.

---

## 3. Prepare the event

Nominate **GM A** — the one human PRD #1419 story 16 and issue #1440's own
heading ("One GM runs...") require — and an evidence recorder (may be GM A).
§5.8's private-filter-isolation check needs a **second GM operator
identity**, not necessarily a second person: the mechanism it verifies
(`GM_ATTENTION_STORAGE_KEY = 'phoenix.gm.attention.v1'`, scoped by session id
*and* operator id, `gui/gm-attention-filters.js`) is already proven
same-browser-safe by
`tests/client/gm-attention-panel.test.js`'s `'keeps two Game Masters
isolated in one browser and resets for a new session'`. GM A may open a
second browser profile/tab and join as a second, distinctly-named GM
operator to exercise it live. If a second real person is available, use
one — record which was actually done; do not claim two-human isolation
evidence from a solo run.

Reuse this issue's scenario from the rest of the GM console acceptance
suite rather than a fresh one:

| Required input | Record before starting |
| --- | --- |
| Integrated build | Full commit, build/content stamp, date, owner |
| §2 precheck results | Exact commands, pass counts |
| Scenario | `assets/worlds/combat_test.toml`, seed, hull, real crewed Stations |
| Supplementary worlds (§5.4–§5.7) | `assets/worlds/probe_gm_attention.toml`, `probe_gm_beats.toml`, `probe_gm_idle_npc_authored.toml`, `probe_gm_quiet.toml`, `probe_gm_workload_authored.toml`, `probe_gm_widgets.toml` |
| GM operator identities | GM A's public operator id; the second identity used for §5.8 and whether it was a second person |
| Devices/viewports | Reuse `docs/acceptance/1421-device-matrix.md` §2's recorded landscape-tablet viewport if filled in; otherwise the `tablet-1280x720-interim-landscape` stand-in it names, plus the `desktop-1280x720-gm`/`desktop-1440x900-gm` fixtures |
| Text/effect settings | GM A's chosen text scale and contrast/effects mode for §5.10 |
| Evidence capture | Recorder, destination, screenshot/export method |

The supplementary worlds in §5.4–§5.7 are each purpose-built, already-tested
probes from their own issue (#1433–#1439); this kit adds none of its own —
see the decision recorded in the handover for why a combining script was not
built. Do not put private reconnect capabilities in the public issue report.

---

## 4. Read the queue before touching anything

1. Launch `combat_test.toml` and get the crew seated, but do not Ready or
   launch yet. Open `gui/gm-attention-panel.js` (`#gm-attention-panel`).
   Confirm it reads `server.gm.attention.empty` ("Nothing is waiting for
   you.") and the health panel (`#gm-health-panel`) shows every participant
   `server.gm.health.state.live` — this is the honest quiet baseline the
   rest of the session builds activity on top of.
2. Have GM A open **Settings → Controls** and note their default
   confirmation policy is unchanged from earlier GM kits (`event.fire:
   confirm`, `effect.lethal`/`world.despawn: confirm-preview`,
   `gui/gm-confirmation.js`) — §5.9 exercises what these actually produce.

---

## 5. Exercises

### 5.1 Start quiet, then get busy (`combat_test.toml`)

This is the spine of the "busy session": one continuous run on the same
scenario the rest of the GM acceptance suite uses, deliberately producing
several kinds of activity so they stack in the *same* queue at once — the
point PRD #1419 story 16 actually tests, distinct from any single producer's
own isolated suite.

1. **Quiet, deliberately.** With the crew seated and Ready but not yet
   touching controls, wait through 120 simulation seconds of genuinely no
   `ActionFeedback::Applied`, no sustained helm/weapons `AdmittedCommands`,
   no `ObjectiveChanged` (panel browsing and passive ticks do not count —
   PRD #1419 Implementation Decisions). Confirm one Background row appears
   reading `server.gm.attention.category.quiet_time` /
   `server.gm.attention.reason.quiet_time` with the authored 120-second
   interval, naming no operator, no Station and no keystroke count. Record
   the actual wall-clock wait.
2. **Get busy.** Have the crew launch the mission. As wave releases begin,
   confirm each becomes an **eligible-beat** row
   (`server.gm.attention.category.eligible_beat`,
   `server.gm.attention.reason.eligible_beat_manual` for the manual "yours
   to start" phrasing — combat_test's `release_wave_*` beats carry no
   authored band, so confirm they land in the default Attention group).
   GM A fires one wave through the row's **Open controls**
   (`server.gm.attention.open_beat`) rather than hunting for the mission
   panel unprompted; confirm it navigates to the same mission-panel Fire
   control #1320's kit already exercises, and that firing resolves the row
   (the queue's own predicate ceasing to hold, not a manual dismiss).
3. **Comms.** Have GM A open the **starbase-selected** or **starbase-fleet**
   route (`assets/worlds/combat_test.toml`) and send an in-character line, or
   let the crew hail Starbase Alpha. Confirm a pending-Comms row appears in
   the Attention band (`server.gm.attention.reason.pending_comms` for a
   selected-ship route or `_fleet` for the fleet route) and that opening it
   (`server.gm.attention.open`) navigates to the real conversation in
   `gui/gm-comms-panel.js` rather than a duplicate reading surface. Have the
   crew answer; confirm the row resolves and does not remain a phantom
   pending item.
4. **Workload.** With Comms pending and a wave in flight, open
   `#gm-workload-panel`. Confirm the Comms Station's row reads
   `server.gm.workload.state.engaged` or higher and expand it
   (`server.gm.workload.evidence`, "What is waiting") to confirm the listed
   demand names the actual pending conversation
   (`server.gm.workload.reason.pending_comms`) rather than a bare count.
   Note whether any other Station accrues a second demand from ordinary
   play (a Navigation clearance or Repair dispatch — `.reason.
   navigation_clearance` / `.reason.repair_dispatch`) and record it if so;
   do not force one if combat_test's ordinary flow does not produce it —
   §5.6 exercises Overloaded deliberately with an authored world instead of
   waiting on chance.
5. **Disconnection.** Mid-wave, with Comms still pending, have the human
   crewing one Station (not GM A) abruptly close their tab — the same
   abrupt-loss technique #1320 §7 uses for a GM, applied here to a *player*.
   Confirm within a few ticks: `#gm-health-panel` marks that Station/operator
   `server.gm.health.state.disconnected`; a **fresh** Urgent row appears in
   the attention queue (`server.gm.attention.category.station_health`,
   `server.gm.health.reason.station_disconnected`, naming the actual operator,
   Station and hull); and the *same* warning also appears as a persistent
   technical banner via `gui/gm-health-banner.js` — record that it renders
   in **both** places, since the banner seam is deliberately outside every
   filter/snooze/hold the queue offers.
6. Confirm, at the busiest moment of this run (wave in flight, Comms
   pending, disconnection banner up), that the attention queue shows
   **multiple bands with multiple rows at once** — this is the "busy" the
   issue names, not a sequence of isolated single-item checks. Have GM A
   describe, unprompted, whether they could tell what needed them first.

Evidence: the quiet-row wait, screenshots of the stacked multi-band queue at
its busiest point, the disconnection's paired queue-row-and-banner
appearance, and GM A's own account of triage clarity.

### 5.2 Held reading and Return to live

1. During 5.1's busy moment, GM A clicks into a row's detail (or focuses a
   row's Snooze button) and holds there while new activity keeps arriving —
   have the crew trigger another wave or Comms exchange while GM A is mid-
   read. Confirm the list freezes (`#gm-attention-panel`'s
   `dataset.freshness` becomes `held`), the status line reads
   `server.gm.attention.held` with an accurate new-item count, and GM A's
   focus/selection does not move under them.
2. Confirm a row that resolved *while held* (e.g. the crew answered a Comms
   conversation GM A is still looking at) shows
   `server.gm.attention.resolved` ("No longer waiting.") in place of its
   verbs, with its age frozen at the wait it ended on rather than resetting
   to 0:00 or silently vanishing.
3. GM A presses **Return to live** (`server.gm.attention.return_to_live`).
   Confirm the held updates apply, the status line returns to
   `server.gm.attention.live`, and holding the list never paused the
   simulation underneath (check the session clock kept advancing while held).

Evidence: before/after screenshots of the held state, the frozen-age
resolved row, and confirmation the world never paused.

### 5.3 Snooze mechanics

1. Snooze a Background or Attention row from 5.1 (`server.gm.attention.snooze`,
   "Snooze 1 min"). Confirm it disappears from the list and reappears
   unprompted after 60 real seconds (`GM_ATTENTION_SNOOZE_MS`,
   `gui/gm-attention-filters.js`) — do not refresh the page to make it
   reappear; watch it happen live.
2. **Already-Urgent waits the minute.** Provoke a Station disconnection (as
   in 5.1.5, on a different Station/human if the first has already
   reconnected) and immediately snooze that Urgent row. Confirm it stays
   snoozed for the *entire* 60 seconds rather than reappearing early because
   it is Urgent — this is the half of the escalation rule a live producer
   can actually exercise (§1's note on why the other half stays with the
   cited unit test).
3. **A fresh Urgent item is never hidden by an unrelated snooze.** With a
   Background/Attention row still snoozed from step 1, provoke a *second*,
   independent Station disconnection. Confirm the new Urgent row and its
   technical banner both appear immediately, visibly unaffected by the
   unrelated snooze still counting down elsewhere in the list.
4. Confirm the technical banner from either disconnection was never
   suppressable by any filter, snooze or hold exercised in this section —
   re-check it is still present after §5.2's hold/Return-to-live cycle.

Evidence: timestamps of each snooze/reappearance, the already-Urgent wait
confirmed against a clock, and the fresh-Urgent-while-snoozed screenshot.

### 5.4 Idle-NPC and quiet-time authored overrides

Switch to the purpose-built probes rather than waiting on chance or
combat_test's un-authored 30/120-second defaults.

1. Load `assets/worlds/probe_gm_idle_npc_authored.toml`
   (`idle_npc_grace_secs = 2.0`, `idle_npc_band = "urgent"`). Leave
   `world.probe_gm_idle_npc.drifter` without an order. After a little over 2
   simulation seconds, confirm one row appears in the **Urgent** band —
   not the shipped default Background — reading
   `server.gm.attention.reason.idle_npc` with the hull's name and elapsed
   idle time. Assign the authored `hold-station` doctrine
   (`SetNpcDoctrine`, the ordinary GM control) and confirm the row resolves.
2. Load `assets/worlds/probe_gm_quiet.toml` (`quiet_time_secs = 2.0`). Leave
   the crew idle past 2 simulation seconds; confirm the quiet row appears far
   sooner than combat_test's 120-second default, proving the override — not
   a coincidence — drove the timing (the world's own comment states this
   explicitly). Answer `quiet-route`'s hail to resolve it and confirm the row
   clears rather than lingering.
3. Record, for both, whether GM A could tell from the product screen alone
   that these were *authored* thresholds rather than the shipped defaults —
   the point of an override a scenario author chose is that a GM trusts it,
   not that a GM can detect it was configured. This is a comprehension
   record, not a new pass/fail axis.

Evidence: both rows' exact band/text, the resolution actions, and GM A's
comprehension note.

### 5.5 Comms and beat authored band overrides

1. Load `assets/worlds/probe_gm_attention.toml`. Open the `urgent-route`
   hail; confirm the resulting pending-Comms row lands in **Urgent**, not
   the pending-Comms default Attention. Open `background-route`'s hail and
   confirm its row lands in **Background**. Open `default-route`'s hail and
   confirm it lands in the un-authored default Attention band — the three
   rows side by side are the proof an author's per-route choice, not a
   coincidence of timing, decided each band.
2. Load `assets/worlds/probe_gm_beats.toml`. Confirm: `brief` (no authored
   band) lands in default Attention and, once fired, never reappears (a
   one-shot, permanently spent); `recall` lands in **Urgent** and, after
   firing, comes back as a **fresh occurrence** (repeatable); `storm` is
   absent until `open_gate` is fired (the `flag(storm_ready)` gate) and
   disappears again once `close_gate` is fired, without ever being spent;
   `relief`'s automatic on-timer row lands in **Background** and offers
   exactly its three authored levers (Fire/Pause/Skip) — confirm no extra or
   missing control on that row.

Evidence: the three Comms bands side by side, the one-shot/repeatable/gated
beat transitions, and the relief row's exact lever set.

### 5.6 Workload authored overrides and evidence

1. Load `assets/worlds/probe_gm_workload_authored.toml`
   (`workload_overload_count = 2`, `workload_overload_secs = 1.0`). Open one
   of the three routes' hails; confirm the Comms Station reads
   `server.gm.workload.state.engaged` at one demand. Open a second; confirm
   it crosses the authored threshold of 2 and begins
   `server.gm.workload.building` ("At {elapsed}s of {needed}s toward
   overloaded."). After roughly 1 simulation second held at or above the
   threshold, confirm it becomes `server.gm.workload.state.overloaded`.
2. Answer one hail, dropping the count back below 2. Confirm the Station
   drops out of Overloaded/Engaged-building and the elapsed timer resets to
   zero rather than resuming from where it left off — the drop-resets
   behaviour PRD #1419 Implementation Decisions names explicitly.
3. Expand the Station's evidence (`server.gm.workload.evidence`) at each
   state and confirm the listed demands name the actual open conversations,
   never a bare number.

Evidence: the Engaged→building→Overloaded transition with timestamps, the
threshold-drop reset, and the evidence list contents at each state.

### 5.7 Authored widgets desk

Load `assets/worlds/probe_gm_widgets.toml`.

1. Select the **tactical** role preset. Confirm `#gm-widgets` shows all four
   authored widgets: the `urgent-traffic` attention card pre-narrowed to
   Urgent/pending-Comms (`server.gm.widget.attention.narrowing`, "Starts on
   …"), the `seats` workload card, the `session-levers` action buttons
   (Pause/Resume), and the `brief` note rendered as plain text — attempt to
   select and copy the note's text to confirm there is no markup underneath
   it, matching the commit's "no path from authored content to parsed
   markup" claim.
2. Confirm the attention card is reading the **same** shared queue and
   filters §5.1–5.5 already used — trigger a fresh Urgent Comms occurrence
   (e.g. reload `probe_gm_attention.toml`'s `urgent-route`) and confirm it
   appears in both the main attention panel and the widget's card, never
   disagreeing about what is waiting.
3. Change the widget's narrowing by hand (select a different band/category
   in the main attention filters) and confirm the *authored default* is not
   silently restored — it only reapplies on a fresh preset selection or a
   new session, never on an ordinary filter change (§5.8 exercises the
   reconnect/new-session distinction directly).
4. Select **narrative**; confirm its differently-narrowed `quiet-watch`
   widget (ship-scoped to `world.probe_gm_widgets.escort`) and its own
   `brief` note replace the tactical desk's widgets entirely, and that
   switching back to **All** hides `#gm-widgets` outright (no widgets
   authored on the built-in preset).
5. Press a `session-levers` button whose control the desk is **not**
   currently offering (e.g. Resume while the session is already running).
   Confirm it renders disabled with `server.gm.widget.actions.unavailable`
   naming the count, rather than silently doing nothing when pressed.

Evidence: all four widget types on screen, the shared-queue agreement check,
the narrowing-survives-manual-change observation, the preset-switch reset,
and the disabled-button sentence.

### 5.8 Private per-GM filter isolation

1. Still on the busy `combat_test.toml` scenario from §5.1, GM A sets a
   narrow filter (e.g. category = Comms only) and snoozes one row. Open the
   second GM operator identity from §3 (second browser profile or second
   person) on the **same session**. Confirm that operator's attention queue
   shows its own default (unfiltered, unsnoozed) view — GM A's filter and
   snooze are not visible there.
2. Have the second identity set a *different* filter and snooze a
   *different* row. Confirm GM A's own view is unaffected by the second
   identity's choices, in either direction.
3. Reconnect GM A to the **same session** (close and reopen that profile).
   Confirm GM A's filter and remaining snooze time are restored exactly as
   left — this is PRD #1419 story 6, "retain private snoozes and filters on
   same-session reconnect." Then start a genuinely **new session** (a fresh
   join, not a reconnect) and confirm GM A's filters/snoozes are *not*
   carried over — the "start fresh in a new session" half of the same story.
4. Record whether either GM found the other's absence-of-interference
   confusing or reassuring — an operator expecting *some* shared state
   (e.g. "did my colleague already see this?") deserves to have that
   expectation checked against what actually happens, not assumed correct
   because the isolation is technically private.

Evidence: both operators' filter/snooze screenshots at each step, the
reconnect-restores / new-session-resets pair, and both operators' accounts.

### 5.9 Truthful existing action outcomes

1. From the busy `combat_test.toml` session, have GM A Fire an eligible wave
   event through the attention queue's **Open controls** link (5.1.2).
   Confirm the terminal result is `Applied` in the mission panel and in
   `gui/gm-journal-panel.js` (#1441, already landed) — cross-check the same
   action appears identically in both places.
2. Attempt to Fire that **same** wave a second time (it is a one-shot per
   `probe_gm_beats.toml`'s documented rules, and combat_test's waves are
   likewise non-repeatable). Confirm the result is a truthful `No-op` or a
   named refusal — never a silently repeated `Applied`.
3. Attempt to despawn a protected entity (e.g. the player's own fleet hull,
   or `starbase_alpha`) through the ordinary Despawn control. Confirm the
   result is `Refused` with `ProtectedEntity` (`gui/gm-action-reasons.js`'s
   `GM_ACTION_REFUSAL_REASON_LABELS` table) rendered as a human sentence in
   both the action feedback lifecycle and the journal — not a bare error
   code or a silent failure.
4. Press a `session-levers` widget button (§5.7.5) that the desk is not
   offering right now. Confirm no action was recorded in the journal at all
   — a mirrored-disabled control that does nothing is not the same event as
   a pressed control that was refused, and the journal must not show one.
5. Record, for each of the three outcomes above, whether GM A could tell
   what actually happened without reading this kit — that is #1418 story
   31's actual test ("Pending, Applied, No-op and Refused outcomes
   represented truthfully"), not merely that the correct enum value shipped.

Evidence: the journal rows for each attempted action, the exact refusal
sentence, and confirmation the disabled widget press left no journal entry.

### 5.10 A representative 200%/landscape pass

`tests/smoke/gm-layout.spec.js` already drives the attention, health,
workload and widgets panels through dense/realistic content at 100/150/200%
on the real engine (§1, #1439's commit). This is **not** a repeat of that —
it is the one live check that a real busy queue, not a synthetic fixture,
stays legible to a person.

1. At the busiest moment reachable (re-run 5.1's stacked-queue moment if it
   has since cleared), have GM A switch their own text scale to 200%
   (`server.html` → Settings → Display) on the recorded landscape-tablet
   viewport from §3. Confirm every band group, row reason sentence, the
   held/live status line and the workload evidence expander remain fully
   readable — no truncation, no row overlapping its neighbour, no control
   pushed off-screen.
2. Enable forced/high-contrast colours. Confirm the Urgent/Attention/
   Background distinction, the disconnected Station's row, and the disabled
   widget button from §5.9.4 all remain distinguishable by more than colour
   alone (words, not just a background tint).
3. Record whether reading position/focus survived the scale change mid-read
   (repeat §5.2's held-reading check at 200%) and whether GM A found
   anything clipped, overlapping or colour-dependent — this is the one live
   200%/forced-colours pass over a genuinely busy M4 desk; #1430's own
   coverage (§1) stops at the T2 directing/performing panels and does not
   reach the attention/health/workload/widgets panels this issue owns.

Evidence: a screenshot of the busy queue at 200% (and, where practical,
forced colours) with the specific text named above visible and intact, plus
GM A's account of legibility.

---

## 6. Preserve evidence and decide

Use the existing snapshot/export UI and the journal panel's own read-only
export (or the collector `docs/acceptance/1320-gm-live-event.md` describes)
to retain the attention/health/workload projections and journal contents at
each numbered step above. The attention list is not bounded the way the
Activity ring is, but a held list can still change under a late capture —
prefer capturing during the step, not after moving on.

The coordinator marks **Accept** only when all ten exercises (§5.1–§5.10)
are complete, every AC below has cited evidence, and any blocking failure
has been resolved with its owning issue from §1 — **a software fix or an
automated PASS does not change a human Reject without the relevant human
rerun.** Missing mandatory exercise or evidence keeps the record Pending;
#1440 stays open either way. This run is **browser-only**; native GM
availability is not implied and is not exercised here — native reuse of
these same components is T4 M6 (PRD #1418 Out of Scope), not a gap in this
acceptance.

```markdown
# #1440 M4 busy-session record

Status: Pending / Accept / Reject
Decision owner and date:
Session start/end and timezone:
Integrated commit/build stamp:
§2 precheck results (commands, pass counts):

## People and devices
| Role | Participant | Public operator id | Device/OS/browser | Viewport | Text scale / effects |
| --- | --- | --- | --- | --- | --- |
| GM A | | | | | |
| Second GM identity (§5.8) — second person? Y/N | | | | | |
| Evidence recorder | | | | | |

## Exercises
| Step | Actual action/observation | Result | Artifact/tick | Defect or follow-up |
| --- | --- | --- | --- | --- |
| 5.1 Quiet start, then stacked Comms/beat/workload/disconnection | | Pending | | |
| 5.2 Held reading, frozen-age resolution, Return to live | | Pending | | |
| 5.3 Snooze: personal minute, already-Urgent waits it, fresh-Urgent never hidden | | Pending | | |
| 5.4 Idle-NPC and quiet-time authored overrides | | Pending | | |
| 5.5 Comms and beat authored band overrides | | Pending | | |
| 5.6 Workload authored overrides, building/overloaded, drop-reset, evidence | | Pending | | |
| 5.7 Authored widgets desk (both presets, shared-queue agreement) | | Pending | | |
| 5.8 Private per-GM filter isolation (same-session reconnect vs. new session) | | Pending | | |
| 5.9 Truthful Applied/No-op/Refused outcomes, incl. disabled-widget non-event | | Pending | | |
| 5.10 200%/forced-colours pass on a genuinely busy desk | | Pending | | |

## Decision against #1440
AC1 — one documented busy-session run producing Comms, beat, idle, quiet, workload and disconnection activity:
AC2 — stable reading/activation, snooze/escalation, explained workload, authored overrides/widgets, private second-GM isolation, truthful outcomes:
AC3 — seed/revision/devices/viewports/text/effect settings, tasks, missed/incorrect actions, explicit verdict recorded; blocking usability failures resolved before acceptance:
AC4 — no invented native GM availability; this run is browser-only, native reuse deferred to M6:
Accept/Reject rationale, or missing work keeping Pending:
Defect dispositions and required human reruns:
```

---

## 7. Source pointers for the coordinator

- [Attention queue projection](../../src/gm_attention.rs) and [panel](../../gui/gm-attention-panel.js); [private filter/snooze controller](../../gui/gm-attention-filters.js)
- [Public health projection](../../src/gm_health.rs), [panel](../../gui/gm-health-panel.js) and [banner seam](../../gui/gm-health-banner.js)
- [Station workload](../../src/gm_workload.rs) and [panel](../../gui/gm-workload-panel.js)
- [Typed widget authoring](../../src/world/config.rs) (`GmRolePresetWidget`, `GM_WIDGET_TYPES`, `GM_WIDGET_ACTION_IDS`) and [desk panel](../../gui/gm-widgets-panel.js)
- [Confirmation policy registry](../../gui/gm-confirmation.js) and [refusal-reason labels](../../gui/gm-action-reasons.js)
- [Read-only action journal panel](../../gui/gm-journal-panel.js) (#1441) — used in §5.9 for the Applied/No-op/Refused cross-check
- Purpose-built probes reused unchanged by §5.4–§5.7: [`probe_gm_attention.toml`](../../assets/worlds/probe_gm_attention.toml), [`probe_gm_beats.toml`](../../assets/worlds/probe_gm_beats.toml), [`probe_gm_idle_npc_authored.toml`](../../assets/worlds/probe_gm_idle_npc_authored.toml), [`probe_gm_quiet.toml`](../../assets/worlds/probe_gm_quiet.toml), [`probe_gm_workload_authored.toml`](../../assets/worlds/probe_gm_workload_authored.toml), [`probe_gm_widgets.toml`](../../assets/worlds/probe_gm_widgets.toml)
- [`pasm/spec/design/gm-console-t3.yaml`](../../pasm/spec/design/gm-console-t3.yaml) — the M4 design entries this kit exercises: `gm-t3-attention-queue`, `gm-t3-eligible-beats`, `gm-t3-idle-npc-advisory`, `gm-t3-quiet-time-advisory`, `gm-t3-peer-health`, `gm-t3-station-workload`, `gm-t3-authored-gm-widgets`
- [`pasm/spec/roadmap/gm-console-milestones.yaml`](../../pasm/spec/roadmap/gm-console-milestones.yaml) — M4 `gm-milestone-facilitation-assistance` scope boundary
- [#1320's live-event kit](1320-gm-live-event.md) — the abrupt-disconnection technique §5.1.5 borrows, applied here to a player rather than a GM
- [#1448's M5 recovery kit](1448-m5-recovery.md) — the sibling kit this document's structure mirrors; §1's "no producer re-bands a live occurrence" note documents the one place this kit's exercise design deliberately narrows the PRD's stated rule to what is actually reproducible today
- [Device/viewport matrix](1421-device-matrix.md) — §2 is this kit's source for the actual landscape-tablet viewport recorded in §3

This kit does not itself supply integrated runtime evidence or human
acceptance; both remain the responsibility of the operator who runs it.
