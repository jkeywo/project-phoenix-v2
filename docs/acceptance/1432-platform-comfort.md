# Integrated platform-comfort acceptance — issue #1432

**Status: preparation only. No box below is checked by this issue, and no
human run has yet been performed against it.** Issue #1432 is PRD #1418's own
closing instruction: *"Required HITL: phones, landscape tablet and native
multi-monitor setup; enlarged text, reduced effects, dense lists and recovery
understanding on the surfaces where those features are available. Record
viewport/scale, selected settings, build revision, task and result."* Ten T3
issues (#1421–#1431, this branch) each shipped its own feature and its own
per-feature kit or automated coverage. **This document does not re-derive or
duplicate any of them.** It is the single integrated script one operator
runs, once per physical surface, to exercise all ten together — because the
acceptance question the PRD actually asks is whether a real phone, a real
landscape tablet and the real native rig are comfortable *as a whole*, not
whether each lever taken alone has a passing spec.

Parent PRD: #1418, user stories 1, 7, 8, 9, 10, 18, 19, 20, 21, 22, 33.

---

## 1. Acceptance criteria this kit exists to satisfy

| # | Issue #1432 criterion | Satisfied by | Status |
| --- | --- | --- | --- |
| 1 | Recorded hardware/viewport/settings/build/results | §4 tables (hardware/viewport delegate to #1421 §2; settings/build/results are new here) | **NOT RECORDED** — blank template |
| 2 | Complete console tasks at 200% | §5 Task 2 (tablet) and Task 3 (native rig) | **NOT RUN** |
| 3 | Phone exception verified | §5 Task 1 | **NOT RUN** |
| 4 | Actual Windows setting adoption confirmed | §3 + §5 Task 3, deferring the record itself to `docs/acceptance/1127-windows-preferences.md` | **NOT RUN — see §3, this kit cannot itself satisfy this criterion** |
| 5 | Landscape web GM pass | §5 Task 4 | **NOT RUN** |
| 6 | Unavailable native GM explicitly remains M6 | §5 Task 4, final note | **Recorded as a design fact below** (no test can pass or fail an unbuilt surface) |
| 7 | Failures resolved or honestly block acceptance | §6 Findings + §7 Decision | **Pending a run** |

---

## 2. What is already integrated on this branch

Every row below is a landed commit on `claude/t3-prd-sub-issues-65d146`
(or its own worktree, merged in). This kit assumes their behaviour rather
than re-testing it; each issue's own kit or automated suite remains the
authority for *its* feature.

| Issue | Commit | What it added | Its own kit / tests |
| --- | --- | --- | --- |
| #1421 | `cd92ddf4` | Device/viewport/browser/scale matrix; shared fixtures | `docs/acceptance/1421-device-matrix.md`, `tests/fixtures/device-matrix.mjs`, `tests/client/device-matrix.test.js` |
| #1422 | `8c6dd804` | Text-scale ceiling 150%→200%, forced-colours baseline, shell scaling, scoped resets, live-preview status, Power workflow carried end to end | `tests/client/accessibility-200-percent.test.js`, `tests/smoke/text-scale-power-workflow.spec.js` |
| #1423 | `c01910ed` | Canvas radar/map labels scale with `--a11y-text-scale`; pending-waypoint state gains a non-colour cue | `tests/client/ph-navigation-map.test.js`, `tests/client/ph-radar.test.js`, `tests/smoke/helm-navigation-text-scale.spec.js` |
| #1424 | `79f7ba95` | Tactical/Sensors readable and operable at 100/150/200% + forced colours; hostile-contact shape cue | `tests/client/ph-tactical-radar.test.js`, `tests/client/tactical-sensors-contact-cues.test.js`, `tests/smoke/text-scale-tactical-sensors-workflow.spec.js` |
| #1425 | `27668a08` | Engineering/Operations (allocation, Repair, Tractor, Umbilical, Dock, Transport) at up to 200%, tablet + native split-pane floor | `tests/smoke/engineering-operations-text-scale.spec.js` |
| #1426 | `4392a48e` | Comms/Captain at 100/150/200% + zoom + forced colours (test-only issue; no production change needed) | `tests/smoke/text-scale-comms-captain-workflow.spec.js` |
| #1427 | `4b99afbf` | Shared Viewscreen's own endpoint-owned text size/contrast, saved on the host machine, isolated from scenario saves | `tests/client/viewscreen-presentation.test.js`, `tests/smoke/viewscreen-settings-presentation.spec.js` |
| #1428 | `a31ad284` | Camera shake / flash-pulse / decorative motion split into three settable levers on every full settings surface | `docs/acceptance/1428-visual-effects.md`, `tests/smoke/viewscreen-effects.render.spec.js` |
| #1429 | `50b849b3` | Phone Viewscreen's limited Display tab (text/contrast/Reduce effects only) and the level-3 Coordination compact/tap-to-expand exception | `tests/client/phone-viewscreen.test.js`, `tests/client/phone-chatter-reader.test.js`, `tests/smoke/phone-viewscreen.spec.js` |
| #1430 | `b0f104ee` | GM directing/performing desk (T2, PRD #930 M1–M3) carried into 200%/forced-colours/dense-list coverage, plus the GM Station-puppet mirroring the GM's own resolved presentation | `tests/client/gm-directing-performing-dense-content.test.js`, `tests/client/gm-station-puppet.test.js`, `tests/smoke/gm-layout.spec.js` |
| #1431 | `ad0f1d24` | Combined visual-effects (all sources at once) + room-distance readability kit; flash repeat-rate precheck | `docs/acceptance/1431-effects-readability.md`, `tests/client/effects-flash-rate.test.js` |

`docs/acceptance/1128-accessibility.md` (issue #1128, T1) is the existing
native multi-monitor reflow/focus/contrast kit; this branch's #1422 raised the
supported ceiling past what that kit's own checklist named, so this issue
corrects that checklist's stale `1.5×`/480px references to the actual `2.0×`/
640px ceiling (see the diff to that file alongside this one) rather than
leaving an operator following this kit into a document that caps out below
the criterion it is meant to prove.

---

## 3. #1127 Windows preference adoption is a separate, still-open kit

`docs/acceptance/1127-windows-preferences.md`'s own words: *"Actual Windows
adoption, override and multi-monitor inspection are **NOT RUN** until an
operator records them,"* and its one native two-pane attempt reported
**SKIPPED: needs two displays** on a single-monitor machine. **That status is
unchanged by this issue.** #1432's criterion 4 ("actual Windows setting
adoption confirmed") is not something this document can satisfy on its own —
it is satisfied only by a dated entry being added to #1127's own kit. What
this issue does is put that pass on the same physical-rig visit as this
kit's own Task 3 (§5), because both need the same native multi-monitor
machine with Windows settings changed and the host relaunched between
changes, and it is wasteful to book that rig twice. **Run #1127's rig-pass
script in full and record its result in #1127's own document first; this
kit's Task 3 only cites that result (pass/fail/date) and does not duplicate
its steps.**

---

## 4. Operator-to-record fields — fill in before running §5

### 4.1 Hardware and viewport

Use `docs/acceptance/1421-device-matrix.md` §2 as the single source of truth
for physical phone(s), the landscape tablet (and its actual viewport),
the native multi-monitor rig, room viewing distance and browser
versions. **Fill that table first, or confirm it is already filled, before
running any task below** — do not re-type those values here where they can
drift out of sync with the copy #1421 and #1431 already reference.

### 4.2 Settings actually exercised (new — #1421 §2 has no settings column)

| Surface | Text scale(s) tested | Contrast | Reduced motion | Shake | Flash | Decorative motion | Follow-system or explicit? |
| --- | --- | --- | --- | --- | --- | --- | --- |
| Phone console | | | | | | | |
| Phone Viewscreen | | | | | | | |
| Landscape tablet console | | | | | | | |
| Native Station pane(s) | | | | | | | |
| Landscape web GM | | | | | | | |
| Shared Viewscreen (#1427 Display tab) | | | | | | | |

### 4.3 Build

| Field | Value |
| --- | --- |
| Build revision (git SHA) under test | |
| Client bundle built fresh this session? (`node scripts/build-client.mjs`) | |
| Host binary built fresh this session? (`cargo build --release --features ultralight --bin phoenix-host`) | |

---

## 5. Integrated task scripts

Each task below deliberately interleaves steps from several of the ten
issues in §2 so an operator exercises them **together**, in the order a real
session would meet them, rather than as ten separate isolated passes. Record
pass/fail per numbered step in §6.

### Task 1 — Phone console + phone Viewscreen (AC "phone exception verified")

On the physical phone(s) recorded in §4.1:

1. Join as a player console (`client.html`). Run `docs/acceptance/1421-device-matrix.md`
   Task B in full — dense Comms content, every `TEXT_SCALES` stop the build
   exposes (100/150/200%), browser zoom separately.
2. Open the settings cog, **Accessibility** tab. Change text scale, contrast
   and reduced motion; confirm live preview (no reload) and that each has its
   own **Reset** plus a combined **Reset all** scoped to presentation only
   (#1422).
3. Reconnect (or force-close and reopen the browser tab) and confirm the
   chosen values survived (#1422's persistence contract).
4. Now open the phone as a **Viewscreen** (`server.html` on the phone,
   landscape-forced per `data-phx-force-landscape`). Open its Display tab and
   confirm **only** text scale, contrast and Reduce effects are offered — no
   individual shake/flash/decorative-motion rows (#1429).
5. Trigger (or load a scenario that produces) a level-3 AI-to-AI System
   Coordination event. Confirm the chatter bubble stays compact regardless of
   the chosen text scale, **and** that an ordinary Comms message on the same
   screen enlarges normally — the exemption must not leak past Coordination.
6. Tap the compact Coordination bubble. Confirm it opens the full message at
   the chosen accessible size in a dismissible reading surface, and that a
   new incoming message while it is open does not change what is displayed
   underneath (#1429's "pinned to the message tapped" contract).
7. Set Reduce effects on the phone Viewscreen and confirm it is honoured
   (bezel pulse, decorative loops) without needing the full three-lever
   surface.

### Task 2 — Landscape tablet: full console sweep at 200% (AC "complete console tasks at 200%")

On the landscape tablet at its **actual recorded viewport** (§4.1, not the
`tablet-1280x720-interim-landscape` stand-in unless that is genuinely what
was recorded):

1. Claim each station family in turn on a hull that carries it — Helm/
   Navigation (#1423: canvas labels and the waypoint-pending cue), Tactical/
   Sensors (#1424: hostile-contact shape cue, weapon banks, scan readouts),
   Engineering/Operations (#1425: Repair, Tractor, Umbilical, Dock, Transport
   dispatch on both dedicated and composite hull surfaces), Comms/Captain
   (#1426: the 322-character Rigger Tacket thread, the two-click important-
   response confirm), Power (#1422's own tracer). At **each** of 100%, 150%
   and 200%: no label clips, no control overlaps another, current target/
   status stays visible, every action stays reachable by scroll rather than
   shrinking.
2. Separately from the in-app slider, set the tablet browser's own zoom
   through 100/125/150/200% and repeat the reachability check on at least one
   station (Task B's "verify browser zoom separately" requirement — this is
   a different code path from the text-scale slider and both must be proven).
3. Toggle forced colours (OS-level, or the browser's forced-colours emulation
   if the tablet browser exposes it) and confirm the focus ring and control
   edges remain visible via system colours (#1422's `forced-colors: active`
   baseline), not only via the `data-contrast="more"` palette.
4. Change one setting and use its own Reset; change several and use Reset
   all; confirm scoping (presentation only — bindings, identity, saves
   untouched).
5. Reconnect and confirm persistence.

### Task 3 — Native multi-monitor rig: split panes at 200% + Windows adoption

On the native rig recorded in §4.1:

1. Run `docs/acceptance/1128-accessibility.md` Parts A–D in full, **at the
   corrected ceiling this issue's diff to that file adds** — 1.0×, the former
   1.5× ceiling, and the actual 2.0× ceiling (#1422) — in both one-pane and
   two-pane layouts, confirming the 640-logical-px two-pane floor rather than
   the stale 480px figure.
2. Immediately after (same rig session), run `docs/acceptance/1127-windows-preferences.md`'s
   reproducible rig pass end to end: Windows text size/contrast/reduced-motion
   at each of its five steps, quitting and relaunching the host between OS
   changes as that script requires. **Record the result in #1127's own
   document**, not here; this task only confirms the pass happened and cites
   its date/outcome in §6 below.
3. With a two-Station-pane profile, confirm the Windows-adopted or explicit-
   override presentation is independent per pane (#1127 step 3) and that the
   split seam is unaffected by either pane's text scale (#1128 Part A).
4. Quit the host entirely and relaunch. Confirm every pane's chosen values
   (explicit overrides, or renewed follow-system reads) come back correctly
   before any settings panel is opened — the restart-persistence half of AC1.

### Task 4 — Landscape web GM pass (AC "landscape web GM pass") and the native-GM non-gap (AC "unavailable native GM explicitly remains M6")

1. Open `server.html` as GM at both `desktop-1280x720-gm` and
   `desktop-1440x900-gm` (§1.1 of #1421's matrix), and at the landscape
   tablet's actual viewport if it can run a full browser.
2. Load `DENSE_CONTENT.gmLists` at `repeatHint` volume into the mission log /
   Objective list / Knowledge Compare panel (#1430). At 100/150/200% and
   forced colours: reading position, focus and the selected target do not
   jump as new entries arrive; a new-item count and **Return to live** appear
   instead of silent reordering; critical connection/recovery banners stay
   visible separately from the held list (`#gm-attention-banners` is never
   hideable by a role preset, per `pasm/spec/design/gm-console-t3.yaml`).
3. Confirm the GM Station-puppet (compatible player/NPC seat) mirrors the
   GM's own resolved presentation — text scale, contrast, effect intensities
   — rather than the puppeted seat's own operator profile (#1430).
4. **Native GM is not run in this task by design, not by oversight.**
   `pasm/spec/roadmap/gm-console-milestones.yaml` places native GM
   integration at T4 M6, *after* Operations and Safety (M5) — PRD #1418's own
   Implementation Decisions confirm: *"Native GM integration remains T4 M6...
   this PRD does not bring native GM integration forward."* There is no
   native GM binary or surface to run this task against yet. Record this
   fact in §6/§7 as satisfying criterion 6 outright — it is a true statement
   about the roadmap, not a blocked or skipped test.

---

## 6. Findings

One row per observed issue (blank until a run produces findings — do not
pre-fill with assumptions):

| Date | Task/step | Finding | Severity | Resolution/evidence | Status |
| --- | --- | --- | --- | --- | --- |
| | | | | | |

## 7. Recording table

| Date | Device (from §4.1) | Task | Build SHA | Result | Notes |
| --- | --- | --- | --- | --- | --- |
| | | Task 1 (phone) | | | |
| | | Task 2 (tablet) | | | |
| | | Task 3 (native rig) | | | |
| | | Task 3 → #1127 rig pass (recorded in that doc, cited here) | | | |
| | | Task 4 (web GM) | | | |

---

## 8. Decision

```markdown
# #1432 platform-comfort record

Status: Pending / Accept / Reject
Decision owner and date:
Build revision:
Hardware/viewport/settings fields (§4):

AC1 — hardware/viewport/settings/build/results recorded:
AC2 — console tasks complete at 200% (tablet + native rig):
AC3 — phone exception verified:
AC4 — Windows setting adoption confirmed (cite #1127 doc's dated entry):
AC5 — landscape web GM pass:
AC6 — native GM explicitly remains M6 (no test possible; recorded as fact):
AC7 — failures resolved, or honestly blocking acceptance:

Accept/Reject rationale, or missing work keeping Pending:
```

---

## 9. Source pointers

- `docs/acceptance/1421-device-matrix.md` — device/viewport/browser matrix and shared fixtures this kit's Tasks 1–2 build on.
- `docs/acceptance/1127-windows-preferences.md` — the separate, still-open Windows-adoption kit Task 3 schedules alongside but does not satisfy.
- `docs/acceptance/1128-accessibility.md` — native multi-monitor reflow/focus/contrast kit (ceiling corrected by this issue's diff).
- `docs/acceptance/1428-visual-effects.md`, `docs/acceptance/1431-effects-readability.md` — the effects/flash kits this document does not duplicate.
- `pasm/spec/roadmap/gm-console-milestones.yaml` — M6 native GM integration placement (T4, after M5).
- `pasm/spec/design/gm-console-t3.yaml` — GM attention-banner non-hideability and the presentation-projection contracts Task 4 exercises.
- `pasm/spec/design/accessibility-programme.yaml` — the `accessibility-incremental-delivery` decision's per-T3-issue rationale list.
