# Acceptance kit — issue #1124, routing mouse, keyboard and touch input

**This is the human half of issue #1124.** The automated half is done and is
listed at the bottom; it proves the routing *logic* — the coordinate transforms,
the pane-boundary hit test, the keyboard-focus order, the touch contact-capture
map — with pure tests that run in CI on no hardware. None of that can prove the
only questions that matter here: **that a real mouse crosses real monitors and
operates a pane on each, that keyboard focus visibly and predictably moves
between panes, and that real fingers on a real multi-touch display land where
they are put and stay captured while they drag.**

So this is a script for one operator at the bridge machine. It has two parts:

- **Part A — mouse, keyboard, and the visible focus ring.** You can run this
  now. It is the closable half.
- **Part B — simultaneous touch.** Written and ready, but **verified only when
  multi-touch Windows hardware exists** — the dev machine has none. This is a
  sanctioned deviation: the automated synthetic tests already cover the touch
  *logic* (capture, per-screen independence, the coordinate transform), and the
  steps below stand ready for whoever first runs a real touchscreen. Do not tick
  acceptance criterion 6 until Part B has actually been run on touch hardware.

---

## 0. Preconditions — do these first, in this order

- [ ] **A build with the `ultralight` feature.** A pane is an embedded browser
      view, so the SDK must be linked:

      ```
      node scripts/build-client.mjs
      cargo build --release --features ultralight --bin phoenix-host
      ```

      The first run stages the Ultralight shared libraries beside the binary and
      prints `phoenix-host: staged N Ultralight libraries …`. If it instead
      prints that the SDK is not beside the binary, re-run the build from this
      checkout — see `docs/delivery-checklist.md`.

- [ ] **A client bundle exists** (`dist/client/index.html`). `--pane` loads the
      built bundle over this host's own HTTP, so the bundle must be built first.

- [ ] **Know your monitors.** Enumerate them and copy the stable identities:

      ```
      ./target/release/phoenix-host --setup
      ```

      It prints each monitor's `id` (e.g. `\\.\DISPLAY2@1920x1080`), its
      geometry and scale. You will paste those ids into a profile below.

Pick the setup that matches your hardware. **If you have one monitor, use the
single-monitor fallback (A0); it exercises within-window routing, keyboard focus
and the focus ring — everything but cross-monitor traversal.** If you have two or
more, use the multi-monitor setup (A1).

---

## Part A — mouse traversal, keyboard focus, and the visible focus ring

### A0. Single-monitor fallback (no `--profile`)

Two panes tiled side by side across the one viewscreen window. Nothing here needs
a second monitor.

```
./target/release/phoenix-host --client-dir dist \
    --world assets/worlds/combat_test.toml \
    --pane Ada --pane Grace --solo
```

- [ ] **Both consoles are visible**, left and right, each showing the join/lobby
      surface and then a console.
- [ ] **The mouse operates both.** Move the pointer to the LEFT pane and click a
      control (claim a station, press a button). Move to the RIGHT pane and do
      the same. One ordinary pointer, no mode switch, each pane responding only
      to clicks over its own half.
- [ ] **The boundary is clean.** Sweep the pointer slowly across the seam between
      the two panes. Control response hands over exactly at the seam — no dead
      strip, no pane responding to a click on the other's half.
- [ ] **Keyboard focus is visible and moves.** Press **Ctrl+Tab**. A bright
      **focus reticle** — a full frame plus four corner brackets — appears on one
      pane. Press Ctrl+Tab again: it moves to the other pane. **Ctrl+Shift+Tab**
      moves it back. The reticle is only ever on one pane at a time.
- [ ] **The focus indicator does not rely on colour.** Confirm the focused pane
      is distinguished by the *presence of the bracketed frame*, not by a colour
      change you would miss in greyscale. (Squint, or imagine the screen in
      black and white: you can still tell which pane is focused, because the
      other has no frame at all.)
- [ ] **Typing goes to the focused pane.** Focus a pane with Ctrl+Tab, click into
      a text field on it (a name field, a comms reply), and type. The characters
      land in that pane's field. Plain **Tab** moves between fields *within* the
      page — it does **not** jump panes (only Ctrl+Tab does).

> **This variant was retired by the bridge layout law (issue #1327).** It used to
> read: author a profile with your one monitor as a `station` of two panes and no
> viewscreen, and the host would open the Station window over the (unplaced)
> viewscreen. That is exactly the overlay the law now forbids, so such a profile
> is **refused at the prompt**:
>
> ```
> phoenix-host: --profile: the profile assigns one monitor but gives no monitor
> the "viewscreen" role; ... Give exactly one monitor the "viewscreen" role
> ```
>
> - [ ] **Optional, 30 seconds:** author that profile anyway and confirm
>       `--setup --profile` refuses it with the message above rather than opening
>       a console over the shared view.
>
> Compositing onto a Station window is exercised by the multi-monitor setup (A1)
> below; a one-monitor bridge has no lawful Station, and the `--pane`-only form
> at the top of A0 is its supported shape.

### A1. Multi-monitor (`--profile`, two or more monitors)

Author `bridge.toml` from the ids `--setup` printed — one `viewscreen`, one
`station` split into two panes:

```toml
version = 1

[[display]]
id = '\\.\DISPLAY1@1920x1080'   # paste your viewscreen monitor's id
role = "viewscreen"

[[display]]
id = '\\.\DISPLAY2@1920x1080'   # paste your Station monitor's id
role = "station"
split = "side_by_side"
[[display.pane]]
label = "Ada"
[[display.pane]]
label = "Grace"
```

Validate it, then run:

```
./target/release/phoenix-host --setup --profile bridge.toml   # expect "Profile matches"
./target/release/phoenix-host --client-dir dist \
    --world assets/worlds/combat_test.toml \
    --profile bridge.toml --pane Ada --pane Grace --solo
```

- [ ] **The viewscreen is on its monitor**, borderless fullscreen, drawing the
      3-D scene.
- [ ] **Both panes are on the Station monitor**, side by side, composited onto
      that window (not tiled on the viewscreen).
- [ ] **One mouse traverses every surface.** With the OS extended desktop moving
      the cursor across monitors, move the pointer from the viewscreen onto the
      Station monitor and operate the LEFT pane, then the RIGHT pane. Each pane
      receives the mouse for its own physical region — a click on the Station
      monitor's right half operates the right pane and nothing on the viewscreen.
- [ ] **Keyboard focus, the reticle, and typing** behave exactly as in A0:
      Ctrl+Tab / Ctrl+Shift+Tab cycle the bracketed focus frame between the two
      panes, and typing lands in the focused pane's field.
- [ ] **Display scaling is honoured.** If your Station monitor runs at a
      non-100% scale, controls still respond under the pointer with no offset —
      the click lands where the cursor is, not shifted by the scale factor.

**Part A passes** when every box above is ticked for your setup. That closes
acceptance criteria 1 and 2, and the mouse/scaling half of criterion 3.

---

## Part B — simultaneous touch (verified-when-touch-hardware-exists)

> **Run this only on a real multi-touch Windows display.** The dev machine has
> no touch hardware, so these steps are unrun. The touch *logic* is already
> covered by the automated synthetic tests (contact capture, per-screen
> independence, the physical→pane coordinate transform); what is left is the one
> thing only hardware can show — that Windows delivers the touch events this
> routing assumes, with a stable per-contact id, in the coordinate space the
> adapter expects.

Set up as in A1 with the Station monitor being (or being driven by) a multi-touch
display.

> **Assumption this routing makes, stated plainly:** touch routing assumes **one
> borderless-fullscreen Station window per touch display**. Under that assumption
> a contact is routed purely by the window the OS reports it against — the router
> resolves it among that window's panes and needs no device→monitor mapping. The
> profile's `[[touch]]` table is **recorded for issue #1123's persistence, not
> consulted by #1124's router**: it exists so a future revision can support a
> touch panel *decoupled* from its monitor (mapping device-global coordinates to
> a display, then to a pane). So for the setup below you do **not** need a
> `[[touch]]` entry, and adding one changes nothing about where a tap lands.
> Wiring `[[touch]]` into routing for genuinely decoupled panels is a future
> item, not part of #1124.

- [ ] **A single tap operates the pane under it.** Tap a control on the left pane;
      it responds. Tap one on the right pane; it responds. Each tap lands on the
      pane under the finger, accounting for the monitor's position and scale.
- [ ] **A drag stays with the pane it began on.** Press a finger down on the LEFT
      pane and drag it across the seam into the RIGHT pane's region and back. The
      gesture stays captured by the LEFT pane for its whole life — the right pane
      never sees it — and releases where the finger lifts. (This is the contact
      capture the automated tests assert; here it is confirmed on real hardware.)
- [ ] **Two fingers on two panes are independent.** With one finger on the left
      pane and another on the right pane at the same time, each pane responds to
      its own finger. Neither gesture disturbs the other.
- [ ] **Two fingers on two *monitors* are independent.** If you have two touch
      displays, a contact on each routes to its own screen's pane with no
      interference.
- [ ] **Known limit, confirm it is only this:** two fingers on the *same* pane
      share one pointer (Ultralight has no multi-touch surface), so pinch/rotate
      *within a single console* is not expected to work. Two fingers on
      *different* panes are fully independent. Note anything worse than this.

**Part B passes** — and only then may acceptance criterion 6 be ticked — when
every box above has been confirmed on real multi-touch hardware.

---

## Where to record results

Add a dated note to the issue #1124 thread (or the batch's acceptance log):

- which setup you ran (A0 single-monitor, A1 multi-monitor, and whether Part B
  was run on touch hardware or deferred),
- your monitor ids, scales, and the `bridge.toml` you used,
- each box: pass / fail / not-run, with a line on anything that surprised you,
- for any failure, the operator log around it (`--log info`) and a photo of the
  screen if it is a visual issue (a mis-placed reticle, a click landing offset).

If Part B is deferred for want of hardware, say so explicitly and leave criterion
6 unticked — that is the expected state until a touch display is available.

---

## What the automated half already covers (no hardware)

The pure routing model in `src/native_host/input_routing.rs` is tested by
`src/native_host/input_routing_tests.rs`, which runs in ordinary CI:

- **coordinate transforms** — a physical point maps to pane-local logical pixels
  at scale 1.0, 1.5 and 2.0, and a monitor at a non-zero desktop origin is
  handled together with its scale;
- **pane boundaries** — the shared seam between two tiled panes belongs to
  exactly one of them, points past the far edge hit nothing, and side-by-side and
  stacked splits route by the profile's own geometry;
- **mouse traversal** — a pointer path crossing a boundary reports the pane it is
  over, and an event is routed only among its own window's panes;
- **keyboard focus transitions** — Ctrl+Tab/Ctrl+Shift+Tab cycling, focusing by
  pointer, and a closed focused pane cleared rather than carried onto its
  successor;
- **touch contact capture** — a contact pinned to its starting pane through a
  drift off it, simultaneous contacts independent, a duplicate `Started` ignored,
  and a closing pane releasing the contacts it held.

The feature-gated winit/Ultralight adapter (`src/native_host/panes/ultralight.rs`,
behind `--features ultralight`) is the thin layer this kit exercises by hand: it
reads Bevy's per-window `CursorMoved`/`ButtonInput`/`KeyboardInput`/`TouchInput`,
asks the pure model where each event goes, injects it into the target view, draws
the focus reticle, and composites each pane onto its Station window. It is
provable only on a machine with the displays, the SDK and the input hardware —
which is what this kit is for.
