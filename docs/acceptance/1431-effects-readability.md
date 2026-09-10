# Combined visual-effects and room-distance readability — acceptance kit (#1431)

**Status: preparation only. No human assessment has been run or accepted
against this document.**

Issue #1431 is PRD #1418's own instruction that flash assessment needs a
separate pass from the per-lever wiring: *"Use captures and the real rig to
assess overlapping default/reduced effects, textual backing and shared-display
readability at the recorded distance. Record thresholds/method and findings
without medical or overall conformance claims... Use the W3C Three Flashes or
Below Threshold method as engineering guidance."* This kit is that record. It
does not itself judge comfort, and it does not certify WCAG conformance —
only a dated human entry in §5 does that, against evidence this kit names.

## 1. What this kit adds over #1421/#1428/#1429

Three prerequisite issues are already integrated (`cd92ddf4`, `a31ad284`,
`50b849b3`) and this kit assumes their behaviour, without repeating their
own acceptance work:

- **#1421** (`docs/acceptance/1421-device-matrix.md`, `tests/fixtures/device-matrix.mjs`)
  supplies the device/viewport/build-recording fields this kit reuses
  directly — most importantly the **room viewing distance** field, still
  blank until an operator fills it in.
- **#1428** (`docs/acceptance/1428-visual-effects.md`) split one motion lever
  into three (shake / flash / decorative motion), named every real consumer,
  and already captures the DEFAULT and REDUCED combination once each via
  `tests/smoke/viewscreen-effects.render.spec.js` (§3 below reuses those
  captures rather than re-deriving them). Its §3.1–3.3 human checklist judges
  **flash alone**, **shake alone**, and **one effect changed at a time**.
- **#1429** limits the phone Viewscreen to text scale / contrast / Reduce
  effects and keeps level-3 AI-to-AI Coordination compact there.

What none of the three establish, and what #1431's acceptance criteria
actually ask for:

1. **All the sources of visual change at once**, not one lever at a time —
   hull-damage shake, the red-alert vignette pulse, decorative UI loops
   (spinners, ready glows) **and** the Coordination chatter bubbles entering
   and fading (`gui/coordination-popup.js`), overlapping on the same screen,
   assessed as a room would actually experience it — Default and Reduced
   each judged as a whole, not lever-by-lever.
2. **Flash judged by the W3C method's own criteria** (§2) rather than "does
   it look calmer" — repeat rate, the fraction of the screen affected at the
   *actual* recorded distance, and whether the transition is a saturated red
   one, each recorded rather than assumed.
3. **Essential feedback legible with effects fully off** — not per-lever (that
   is #1428's table), but the combined off state: HUD readouts read against
   the moving 3-D scene behind them, with no shake, no pulse and no
   decorative motion left to draw the eye.
4. **The distance/scale/build triple actually recorded**, not the interim
   `tablet-1280x720-interim-landscape` stand-in or an unrecorded "far enough".

## 2. Method: the W3C figure, as engineering guidance

The W3C **Three Flashes or Below Threshold** technique (linked in PRD #1418's
Further Notes) is written for web conformance auditing, not for a clinical
diagnosis, and this repository is explicit that no claim of medical safety or
full conformance is being made here (`pasm/spec/design/accessibility-programme.yaml`'s
"never labelled by medical need" guardrail applies equally to this kit). Used
as **engineering guidance**, the method asks three separate questions about
any flashing content, and this kit keeps them separate on purpose — the PRD's
own words are *"flash assessment distinct from motion"*, and within flash
assessment the three questions below are distinct from each other too:

1. **Repeat rate** — does the same area change (and change back) more than
   three times in any one-second window? This is the one question a source
   read can answer without a screen in the room: `tests/client/effects-flash-rate.test.js`
   (added by this issue, §4) computes it directly from the authored CSS
   period and the operator's resolved flash intensity for every looping
   flash consumer this repository has. All three are already well under the
   figure — see §4's recorded numbers — which is evidence about the *rate*
   only, not a finding that closes this kit.
2. **Flash area** — the method's area test is evaluated over roughly a 10°
   field of the viewer's vision at their seating distance, which is why it
   cannot be computed from CSS alone: the same vignette covers a different
   fraction of a viewer's visual field on a 6-metre briefing-room projector
   than on a 24-inch monitor at a desk. This is why §1421's room-distance
   field is a prerequisite input here, not an unrelated fixture — record the
   actual screen diagonal/resolution alongside it in §5.
3. **Red flash** — a saturated-red opposing transition is treated as a flash
   even when the general-flash area/luminance test alone would not flag it.
   Both looping flash consumers in this repository use the `--fire`/`--rgb-fire`
   danger-red token (`gui/tokens.css`: `#e64a34` / `rgb(230, 74, 52)` light
   theme, `#ff6a4a` / `rgb(255, 106, 74)` dark theme) for their glow, so both
   are assessed under the red-flash question as well as the general one —
   record which theme was active during capture.

None of the three questions, on their own or together, is a claim that
Phoenix has been tested against full WCAG conformance, and this kit does not
attempt the method's remaining machinery (combined-effect area summation
across simultaneous flashing regions, the transitional-red edge case). What
is recorded is the specific evidence in §4–§5, and the specific findings in
§6, with an explicit list of what was and was not assessed.

## 3. Capture procedure

### 3.1 What already exists to reuse

`tests/smoke/viewscreen-effects.render.spec.js` (issue #1428) boots
`combat_test` under the `render` Playwright project (real WebGL2 via
SwiftShader) and attaches two screenshots per run:

- `viewscreen-effects-default.png` — shake mid-frame and the red-alert
  vignette both live, everything at Full.
- `viewscreen-effects-reduced.png` — the same moment with shake and flash
  both set to Off through the real Display-tab controls.

Run it and collect the attachments (needs a Trunk-built bundle first, same
prerequisite #1428's own kit names — say so in §5 if skipped):

```sh
npx playwright test --project=render viewscreen-effects.render.spec.js
```

Attachments land under `test-results/` (see the run's own summary for the
exact path) or in `playwright-report/` if `--reporter=html` is used. These
are the DEFAULT/REDUCED base captures for §5's table — this kit does not ask
for a third render pass to reproduce them.

`tests/smoke/viewscreen-reduced-motion.render.spec.js` is the sibling that
captures the OS-`prefers-reduced-motion` path rather than this endpoint's own
Reduce-effects choice; attach its capture too if the acceptance run wants the
follow-system path compared against the explicit one (PRD #1418: *"explicit
operator values win... handle unavailable preference APIs honestly"*).

### 3.2 What still needs the real rig

A single screenshot cannot show a repeat rate, cannot show what the room
actually sees at the recorded distance, and — the specific gap this issue
closes — does not show the sources overlapping the way #1428's isolated
per-lever checklist did not exercise together:

- Run `combat_test` to red alert **and** into sustained weapons fire (as
  #1428 §3.1 already stages), and **additionally** trigger a Coordination
  exchange (any live level-3 AI-to-AI event the loaded scenario produces) so
  its chatter bubbles are entering/fading on the same screen as the vignette
  pulse and the shake. Watch the combination for 60 seconds at **Full**,
  then at **Gentle**, then at **Off**, each independently assessed (AC1) —
  do not let a comfortable Gentle pass stand in for judging Full.
- Do this **at the actual recorded room viewing distance** from
  `docs/acceptance/1421-device-matrix.md` §2, on the actual screen recorded
  there — not at a desk, if the acceptance target is the shared Viewscreen.
- Separately, repeat the same 60 seconds watching **only the flash** (cover
  or ignore the shake and chatter) and **only the motion** (mute the red
  alert if the scenario allows it, or focus attention on the shake/spinners
  only) — two separate judgements per AC2, not one impression averaged
  across both.
- With every effect at **Off**, read the HUD readouts (hull, heading,
  condition) and the Coordination chatter's tap-to-expand text (#1429) from
  the recorded distance. Record whether each stays legible against the
  moving starfield/3-D scene behind it with no motion left to draw the eye
  to it — this is the AC3 "essential feedback readable with effects off"
  criterion, and it is about the TEXT's own contrast/backing against scene
  content, not only about the effect controls existing.

## 4. Automated precheck (this issue)

`tests/client/effects-flash-rate.test.js` computes the **repeat-rate** third
of §2's method directly from the authored CSS, for every looping flash
consumer in the repository, and fails if the authored keyframe shape ever
stops being exactly one flash per period (so the rate arithmetic cannot go
silently stale). Run it with the rest of the vitest suite, or on its own:

```sh
npx vitest run tests/client/effects-flash-rate.test.js
```

Recorded result (this branch, build `137bd381`):

| Consumer | Authored period | Rate at Full | Rate at Gentle | Guidance ceiling |
| --- | --- | --- | --- | --- |
| `#hud-vignette` (`server.html`, browser Viewscreen) | 1.3s | 0.769/s | 0.231/s | 3/s |
| `#phone-bezel` (`client.html`, console) | 2.8s | 0.357/s | 0.107/s | 3/s |
| Native HUD overlay vignette (`gui/viewscreen-hud.html`) | 1.3s | 0.769/s | 0.231/s | 3/s |

All three sit well under the guidance ceiling by rate alone, at both Full and
Gentle — this is the one third of §2's method this repository can answer
without a screen in the room, and it rules out the worst failure (a loop
literally exceeding the repeat-rate figure) before anyone runs §3.2. It is
**not** evidence about area or red-saturation, and it is not a finding that
default flashing is comfortable — §3.2's human pass still judges that, per
PRD #1418's explicit warning that *"a Reduce effects toggle is not evidence
that default flashing is acceptable"* applies equally to a passing rate
check.

## 5. Assessment tables

Fill in per acceptance run. Distance/scale/build/capture-condition fields
first — an entry below without them is not usable evidence.

### 5.1 Recording fields

| Field | Value |
| --- | --- |
| Build revision (git SHA) | |
| Screen — diagonal, resolution, theme (light/dark token set) | |
| Room viewing distance (from `docs/acceptance/1421-device-matrix.md` §2, or measured fresh) | |
| Ambient lighting | |
| Capture method (direct observation / phone camera + fps / Playwright `render` attachment) | |
| Scenario and state used (e.g. `combat_test`, red alert + sustained fire + live Coordination event) | |

### 5.2 Default vs Reduced (AC1 — assessed independently)

| Mode | Sources combined | Observation | Comfortable at 60s? | Evidence |
| --- | --- | --- | --- | --- |
| Default (Full) | shake + flash + decorative + Coordination | | | |
| Reduced (Gentle) | shake + flash + decorative + Coordination | | | |
| Off | shake + flash + decorative | | | |

### 5.3 Flash vs motion (AC2 — judged separately, not as one impression)

| Question | Full | Gentle | Off | Evidence |
| --- | --- | --- | --- | --- |
| Flash alone: rate/area/red-saturation observation | | | | |
| Motion alone: shake/decorative observation | | | | |

### 5.4 Effects-off readability (AC3)

| Element | Legible against scene at recorded distance with effects Off? | Notes |
| --- | --- | --- |
| HUD hull/heading/condition readout | | |
| Red-alert state (held frame, no pulse) | | |
| Coordination chatter — compact bubble | | |
| Coordination chatter — tap-to-expand text (#1429) | | |

## 6. Findings

One row per observed issue (AC5 — concrete resolution/evidence, not a
restated checkbox). Leave empty until a run produces findings.

| Date | Finding | Severity | Resolution/evidence | Status |
| --- | --- | --- | --- | --- |
| | | | | |

## 7. Decision

```markdown
# #1431 effects-readability record

Status: Pending / Accept / Reject
Decision owner and date:
Build revision:
Distance/scale/capture fields (§5.1):

AC1 — default/reduced assessed independently:
AC2 — flash distinct from motion:
AC3 — effects-off readability:
AC4 — distance/scale/build/capture conditions recorded:
AC5 — findings with concrete resolution/evidence:

Explicit exclusions (not assessed by this kit):
- Flash area/red-saturation quantitative measurement (recorded as human
  judgement at the actual distance, not a computed percentage).
- Any claim of medical photosensitivity safety.
- Any claim of full WCAG 2.2 conformance.

Accept/Reject rationale, or missing work keeping Pending:
```

## 8. Source pointers

- [W3C Three Flashes or Below Threshold](https://www.w3.org/WAI/WCAG22/Understanding/three-flashes-or-below-threshold.html)
- `docs/acceptance/1421-device-matrix.md` — room-distance/build recording fields this kit reuses
- `docs/acceptance/1428-visual-effects.md` — per-lever inventory, consumer wiring, and the DEFAULT/REDUCED captures §3 reuses
- [`gui/visual-effects.js`](../../gui/visual-effects.js) — `EFFECT_REDUCED`, `EFFECT_FULL`, the vocabulary `tests/client/effects-flash-rate.test.js` imports
- [`gui/coordination-popup.js`](../../gui/coordination-popup.js) — the level-3 Coordination chatter this kit's combined scenario includes
- [`pasm/spec/design/accessibility-programme.yaml`](../../pasm/spec/design/accessibility-programme.yaml) — the "never labelled by medical need" guardrail this kit's framing follows
