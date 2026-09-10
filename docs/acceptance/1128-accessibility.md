# Acceptance kit — issue #1128, accessible native setup and pane layouts

**This is the human half of issue #1128.** The automated half is done and is
listed at the bottom; it proves the *logic* — the reflow-headroom arithmetic, the
keyboard-focus order across monitors and split panes, the focus-reticle geometry
and its contrast/reduced-motion response, and that every setup action is
keyboard- and mouse-reachable — with pure tests that run in CI on no hardware,
plus one `#[ignore]`d test that runs the same reflow/focus checks against this
machine's **real** monitor geometries.

None of that can prove the only questions that finally matter here: **that real
text at the supported scaling extremes actually reflows on a real monitor without
a clipped or overlapping pixel, in both one-pane and two-pane layouts; that the
bracketed focus frame is visibly the focus cue without any colour telling you so;
and that the contrast and reduced-motion settings visibly reach the native setup
chrome, not just the console content.**

So this is a script for one operator at the bridge machine. Run it on the
multi-monitor Windows dev box.

---

## 0. Preconditions — do these first, in this order

- [ ] **A build with the `ultralight` feature**, so the panes actually render:

      ```
      node scripts/build-client.mjs
      cargo build --release --features ultralight --bin phoenix-host
      ```

- [ ] **A client bundle exists** (`dist/client/index.html`).

- [ ] **Know your monitors.** Enumerate them and read the new Accessibility
      section of the report:

      ```
      ./target/release/phoenix-host --setup
      ```

      Under `Accessibility:` it prints the OS defaults it read (text scale,
      contrast, reduced motion), the supported text-scaling range (`1x to 2x`
      since issue #1422; `1x to 1.5x` on any build predating it),
      and — once you pass a `--profile` — a per-pane line saying whether each pane
      preserves its console **to the supported maximum** or is **TOO SMALL**.

Pick the setup that matches your hardware, exactly as in the #1124 kit: **one
monitor → the single-monitor fallback (A0); two or more → the multi-monitor setup
(A1).** Author `bridge.toml` from the ids `--setup` printed, one `viewscreen` and
one `station` split into two panes (see `docs/acceptance/1124-input.md §A1` for
the file).

---

## Part A — text scaling preserves both layouts (acceptance criterion 1)

Launch the panes (single-monitor form shown; add `--profile bridge.toml` for the
multi-monitor form):

```
./target/release/phoenix-host --client-dir dist \
    --world assets/worlds/combat_test.toml \
    --pane Ada --pane Grace --solo
```

In each pane, open the console **Settings** and find the **text size** control
(the `--a11y-text-scale` slider). Do the following at the minimum (`1.0×`), the
former ceiling (`1.5×`) and the **actual maximum this build supports**
(`2.0×` since issue #1422 — a two-pane monitor now needs 640 logical px per
pane at the ceiling, not the 480px the checklist below originally named; a
build predating #1422 still tops out at 1.5×/480px, so record which ceiling
the tested build actually exposes).

### A-two-pane (the layout above)

- [ ] **Minimum scale (1.0×).** Every control on each console is visible and
      operable; no label is clipped, no two controls overlap.
- [ ] **Former ceiling (1.5×).** Text grows on every string at once. The console
      **reflows**: rows re-wrap and the pane scrolls vertically where needed — but
      **no control is unreachable** (scroll to it) and **no two controls overlap**
      horizontally. Confirm this on *both* panes, left and right.
- [ ] **Actual maximum (2.0×, #1422).** Repeat the 1.5× check at the real
      ceiling. This is the scale issue #1432's integrated acceptance pass
      names for the native rig — do not stop at 1.5× and call the 200%
      criterion met.
- [ ] **The seam is unaffected.** The two panes still meet cleanly at the split;
      enlarging text in one pane does not push its content over the other.

### A-one-pane

Author (or launch) a **one-pane** Station — one console filling a monitor — and
repeat the scale checks. A single full-monitor pane has the most room, so
this is the easy case; confirm it anyway, because acceptance criterion 1 names
*both* one- and two-pane layouts.

> The Station monitor must be a **different** monitor from the viewscreen's, and
> the profile must still name a `viewscreen` (issue #1327): a profile of stations
> with no viewscreen is refused at the prompt, because a console would open over
> the shared view. With only one display, drop `--profile` and use the
> `--pane`-only form above.

- [ ] One pane, **1.0×**: complete and clean.
- [ ] One pane, **1.5×**: reflows, everything reachable, nothing overlapping.
- [ ] One pane, **2.0×** (#1422 ceiling): reflows, everything reachable,
      nothing overlapping.

> If a pane ever reports **TOO SMALL** in the `--setup` Accessibility section,
> that monitor is genuinely too small for that split at the scale tested
> (480px demand at 1.5×, 640px at the actual 2.0× ceiling) — use one pane or a
> larger display. That is the tool telling you the truth, not a failure of this
> step.

---

## Part B — every setup action by keyboard and mouse (acceptance criterion 2)

The four setup actions are **display-role**, **pane**, **touch-device** and
**media** assignment. In this host all four are performed by editing the
`bridge.toml` profile and checking it with `--setup` — a keyboard/CLI route, with
mouse equivalents (a text editor, a file picker). None is touch-first.

- [ ] **Display role** — set a monitor's `role = "viewscreen"` / `"station"` in
      `bridge.toml` with the keyboard; re-run `--setup` and see the assignment
      change in the report. (Mouse: do the same edit in a GUI editor.) Exactly one
      monitor must keep the `viewscreen` role; giving every monitor `station` is
      refused with a message saying so, which is worth seeing once.
- [ ] **Pane** — add/remove a `[[display.pane]]` and change `split`; `--setup`
      reflects the new pane layout and its Accessibility per-pane lines.
- [ ] **Touch mapping** — add a `[[touch]]` entry mapping a device to a monitor;
      `--setup` accepts it. (No touchscreen needed to author it.)
- [ ] **Media** — add a `[[media]]` assignment (a `camera:`/`mic:`/`output:`);
      `--setup`'s media section reflects it.
- [ ] **Confirm none needs touch.** Every one of the above was done with keyboard
      and mouse only. There is no setup action you can *only* reach by touching a
      screen.

---

## Part C — focus is visible without colour, across monitors and split panes (AC 3)

With the panes running (multi-monitor form if you have two displays):

- [ ] **The bracketed focus frame.** Press **Ctrl+Tab**. A focus reticle — a full
      frame **plus four corner brackets** — appears on one pane. Ctrl+Tab again
      moves it to the next pane; **Ctrl+Shift+Tab** moves it back. It is only ever
      on one pane at a time.
- [ ] **Across monitors.** With a two-monitor profile, keep pressing Ctrl+Tab: the
      reticle walks every pane on the first Station monitor, then every pane on the
      next, and wraps back — no pane is skipped or unreachable.
- [ ] **Not by colour.** Squint, or imagine the screen in greyscale: you can still
      tell which pane is focused, because the other panes have **no frame at all**.
      The cue is the *presence of the bracketed shape*, not a hue you would miss in
      black and white.

---

## Part D — contrast and reduced motion reach the native setup chrome (AC 4)

The console content already honours these (issues #1127/#1171): the pane document
imports the OS preferences, and the CSS swaps contrast tokens and pauses
animations. This part is about the **native, host-drawn** chrome — the focus
reticle — honouring them too.

Set the machine's OS preferences (Windows Settings → Accessibility):

Quit and relaunch the host after each OS change. Windows defaults are read at
document creation; the adapter does not subscribe to live preference changes.
An explicit private profile override still wins until reset to follow system.

- [ ] **High contrast / "prefers-contrast: more".** With contrast on, focus a pane
      (Ctrl+Tab). The reticle frame is **bolder and fully opaque** than with
      contrast off. Compare the two: the high-contrast reticle is visibly thicker.
- [ ] **Reduced motion.** With reduced motion on, moving focus between panes is an
      **instant** jump — the reticle simply appears on the new pane with no slide,
      fade or pulse. (It is static by design, so this holds; confirm there is no
      motion introduced anywhere in the setup chrome.)
- [ ] **Console content too.** For completeness, confirm the console content
      responds as well — reduced motion stills the console's own animations and
      high contrast swaps its palette. That half is #1127/#1171's; this kit only
      needs the reticle half above to close criterion 4.

---

## Where to record results

Add a dated note to the issue #1128 thread (or the batch's acceptance log):

- which setup you ran (single- or multi-monitor), your monitor ids, scales, and
  the `bridge.toml` you used,
- each box: pass / fail / not-run, with a line on anything that surprised you,
- for a reflow failure, a photo of the clipped/overlapping console and the
  `--setup` Accessibility section for that profile,
- for a reticle failure, a photo at both contrast settings.

---

## What the automated half already covers (no hardware)

The pure model in `src/native_host/setup_accessibility.rs` is tested by
`src/native_host/setup_accessibility_tests.rs`, which runs in ordinary CI:

- **reflow headroom (AC 1)** — one-pane and two-pane (side-by-side and stacked)
  layouts on the supported monitor resolutions preserve every console at the
  minimum and maximum supported text scale; the two panes tile with no overlap;
  and an undersized monitor is correctly rejected at the maximum;
- **focus order (AC 3)** — the keyboard-focus order lists every Station pane once,
  monitor by monitor and split by split, and a `FocusRing` cycles all of them
  across monitors;
- **the non-colour reticle (AC 3/4)** — the reticle is a frame plus four corner
  brackets, high contrast bolds and opaques it without making colour the cue, and
  it never animates (the reduced-motion guarantee for host chrome);
- **setup-action reachability (AC 2)** — every display/pane/touch/media assignment
  is keyboard- and mouse-reachable and none is touch-only;
- **the `--setup` Accessibility report** — the OS defaults, the supported range,
  and the per-pane reflow verdict.

`tests/native_bridge_accessibility.rs` (`#[ignore]`d, `--features host`) runs the
reflow and focus-order checks against this machine's **real** monitor geometries:

```
cargo test --features host --test native_bridge_accessibility -- --ignored --nocapture
```

On a machine with only **one** display it reports `skipped — needs two displays`
and passes: a Station may not share the viewscreen's monitor (the bridge layout
law, issue #1327), so there is no lawful two-pane layout for it to check. Read the
`--nocapture` line before treating an OK as evidence.

The feature-gated winit/Ultralight adapter
(`src/native_host/panes/ultralight.rs`, behind `--features ultralight`) is the
thin layer this kit exercises by hand: it draws the reticle from the pure
`setup_accessibility::FocusReticle` descriptor and composites each pane onto its
Station window. It is provable only on a machine with the displays and the SDK —
which is what Parts A, C and D are for.
