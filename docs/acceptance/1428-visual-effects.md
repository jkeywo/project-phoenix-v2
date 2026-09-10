# Surface-to-effect inventory and acceptance kit — issue #1428

**PRD #1418, stories 13–17.** Camera/page shake, flash/pulse and decorative
interface motion become three preferences instead of one, on every *full*
settings surface, wired to the consumers that actually render them.

This page is the record issue #1428 asks for: **which settings surface offers
which effect, what consumes it, and — where an effect is not offered — why
there is nothing there to adjust.** An effect a surface cannot render is written
down here and said in a sentence on the surface itself; it is never shown as a
control that does nothing.

The machine-readable copy of the table below lives in
`gui/visual-effects.js` (`EFFECT_INVENTORY`), which is what the surfaces build
their controls from, and `tests/client/visual-effects.test.js` checks that every
cell of it names either a consumer or a String-Table reason.

---

## 1. The inventory

| Settings surface | Camera / page shake | Flash / pulse | Decorative motion |
|---|---|---|---|
| **Private console profile** — the phone client's Accessibility tab and the native Station pane's, one panel over one profile (`gui/settings-panel.js`, `client.html`) | **Not applicable.** A console draws no camera, and the whole-page shake offset arrives on the `shake` host channel, which only `server.html` listens to. `client.html` contains no such path at all. | `#phone-bezel.alert-on` — the red-alert bezel pulse (`client.html`) | `gui/tokens.css` decorative bands, over `client.html`'s two spinners, the indeterminate loading bar and the two ready glows; `gui/console.css`'s `.tutorial-highlight`; `gui/components/ph-battery-bar.js` |
| **Shared Viewscreen** — the browser cog on `server.html` and the native host lobby's, one shared Display tab (`gui/viewscreen-presentation-panel.js`) | `viewscreen_border::apply_camera_shake` via `ViewscreenMotion::shake_intensity` — native camera jitter and the WASM whole-page translate | `viewscreen_border::drive_vignette_intensity` (the shield-hit flash uniform), and the `#hud-vignette` red-alert pulse on **both** runtimes' documents: `server.html` in a browser and `gui/viewscreen-hud.html` on native | `gui/tokens.css` decorative bands, over `server.html`'s spinner and the Coordination chatter entrance, the native lobby chrome, and the native HUD overlay document (which has no decorative loop of its own — see §1a) |
| **Game Master session** — the same cog on `server.html` while `html.phoenix-gm-page` is set | **Not applicable.** That class sets `#canvas { display: none }`, so the session draws no view of space and there is no shake. | **Not applicable.** The same rule hides `#hud-overlay`, so there is no red-alert vignette and no shield flash. | `gui/tokens.css` decorative bands over the GM workspace chrome |
| **Phone Viewscreen (limited subset)** | — | — | — *(issue #1429's; this issue owns the full surfaces)* |

### 1a. The native Viewscreen's third document

The shared Viewscreen row above names two documents for one surface, and the
second one is easy to miss. **In a browser** the Viewscreen is `server.html`: the
frame, the readout and the red-alert vignette are that page's own `#hud-overlay`,
and the page stamps its own root from the endpoint record. **On native** there is
no HTML canvas, so the same overlay is a *separate transparent Ultralight
surface* over the 3-D view, served statically at `/gui/viewscreen-hud.html` and
opened as its own surface (`native_host::app`, `host_lobby::document::
viewscreen_hud_url`). It is not the lobby document, so the presentation script
injected into that document's head never reaches it, and an Ultralight view
answers no `prefers-reduced-motion` query (issue #1127) — which left the
`@media` rule beside the pulse dead on the one runtime that needed it.

So the bands reach that document the only way anything reaches it: over the
**HUD channel it is already driven by**. `viewscreen_border::sync_viewscreen_motion`
resolves all three intensities into `ViewscreenMotion`;
`panes::ultralight::cache_hud_state` turns them into one
`window.__phoenixSetHudEffects({shake, flash, decorativeMotion})` statement and
retains it in the same command as `window.__updateHud`, effects first; the page's
classic prelude stamps `data-shake` / `data-flash` / `data-decorative-motion` and
the three `--a11y-*-scale` properties on its root, which is what
`applyEffectIntensitiesToRoot` does everywhere else. The stamping lives in the
prelude rather than the module island because a module whose import fails never
evaluates, and a red-alert glow still throbbing at an operator who chose **Off**
is the one failure that must not be survivable;
`tests/client/viewscreen-hud.test.js` pins the prelude's stamper against the
shared function so they cannot drift.

One thing this channel deliberately does **not** carry is that document's text
size and contrast. Those are issue #1427's record, and the same boundary applies
to them — the HUD overlay carries no `--a11y-text-scale` and no `data-contrast`
— but widening this push to them is a change to what #1427 shipped, not to what
#1428 owns. It is written down here rather than left silent: the overlay draws
four short numeric readouts and a designation, and the effects control is the
one whose promise ("Pulses the red-alert glow…") named this surface.

What that document does **not** have is a decorative loop of its own: its only
motion is the vignette pulse and the vignette's opacity fade. The decorative band
is stamped there anyway, and is recorded here as a band rather than as a loop —
because `gui/tokens.css`'s sweep and its `[data-effect="flash"]` re-assertion
both key off that attribute's presence, and because a loop added to that document
later is then already governed.

### What the three settings mean

Each effect's stored value is `default` — follow whatever the **Motion**
preference resolved to — or a number in `0..=1` where `0` is off and `1` is the
shipped strength. The controls offer three named stops: **Full** (`1`),
**Gentle** (`EFFECT_REDUCED`: shake `0.3`, flash `0.3`, interface animation
`0.4`) and **Off** (`0`), plus **System**.

Following the preference resolves to **`0` under reduce** and **`1` otherwise**,
which is exactly what shipped before this issue split the lever — so an operator
who never opens these controls sees no change at all.

### What "off" does not remove

| Effect off | What still tells you | Where |
|---|---|---|
| Flash | The red-alert frame holds full red at full glow — it simply stops moving | `client.html` `:root[data-flash="off"] #phone-bezel.alert-on`; `server.html` and `gui/viewscreen-hud.html` `:root[data-flash="off"] … #hud-vignette` (opacity `1`) |
| Shake | Hull damage is still on the HUD readout, and the Bevy/CSS transform is exactly zero rather than the damage being hidden | `viewscreen_border::shake_magnitude` |
| Interface animation | A stopped spinner still reads as a progress indicator, the indeterminate bar becomes a full dimmed track, the ready glows hold their bright frame, the tutorial highlight holds its lit outline | `client.html`, `gui/console.css`, `gui/host-lobby.css` |

### Where the three effects have to be kept apart in CSS

The one place independence is easy to lose is the cascade. `gui/tokens.css`
sweeps the whole interface for the decorative band — `:root[data-decorative-motion="off"] *`
and its `reduced` sibling, both `!important` — and *every* looping element is
caught by that `*`, including the two the **flash** preference owns:
`#phone-bezel.alert-on` (client.html) and `#hud-overlay.alert-on #hud-vignette`
(server.html). Neither page declares its loop `!important`, so on their own the
sweep wins and the three controls stop being three: **Flashes = Full** with
**Interface animation = Off** kills the red-alert pulse outright, and pressing
**Reduce effects** (which writes the `reduced` band) leaves it running one
iteration however the flash control is moved afterwards.

So each flash loop carries `data-effect="flash"`, and `gui/tokens.css`
re-asserts it after the sweep at a higher specificity:

    :root[data-decorative-motion]:not([data-flash="off"]) [data-effect="flash"] {
      animation-duration: var(--a11y-flash-duration) !important;
      animation-iteration-count: infinite !important;
    }

`--a11y-flash-duration` is the loop's authored period (`--a11y-flash-period`:
2.8s on the bezel, 1.3s on the vignette) divided by the resolved flash
intensity, computed beside the loop so the page's own declaration and this
override read one number. The `:not([data-flash="off"])` gate means the rule
only ever holds motion the flash preference still wants — an explicit **Off**
leaves it unmatched, and each page's `animation: none` held-frame rule stands.

`tests/client/visual-effects.test.js` §5b resolves this cascade over the real
stylesheets and asserts the winner; the browser-side half is the
`interface animation off does not take the red-alert flash with it` case in
`tests/smoke/viewscreen-effects.render.spec.js`.

---

## 2. What the automated tests already prove

Run these; they need no hardware.

- `npx vitest run tests/client/visual-effects.test.js` — the vocabulary and its
  resolution, the inventory's completeness, Reduce effects and its scope, the
  console tab's controls (including at 100/150/200% text), the Viewscreen and
  GM surfaces, persistence and both reset scopes, and the wasm seam's names.
- `npx vitest run tests/client/viewscreen-presentation.test.js` — the endpoint
  record now carries five effects and still cannot touch a private profile.
- `cargo test --features headless --lib server::viewscreen_border` — the shake
  and flash intensity maths, the follow-the-preference default, and the native
  latch.
- `cargo test --features headless --lib native_host::viewscreen_presentation` —
  the saved file, per-effect reset, the clamp and the seed script.
- `cargo test --features headless --lib native_host::host_lobby` — that a saved
  shake, flash or decorative band reaches the latch when the lobby is PUBLISHED,
  not only when somebody presses the control. On an Ultralight view the page's
  own `presentation.apply()` cannot do it (there are no wasm bindings there), so
  without the launch-time seed a room that had turned the hull shake off came
  back at full shake every launch.
- `cargo test --features headless --lib native_host::panes::hud` and
  `npx vitest run tests/client/viewscreen-hud.test.js` — the two ends of the
  native HUD-overlay channel described in §1a: the statement the host composes
  (effects ahead of the state, in one retained command) and the page answering
  it with exactly the attributes and properties `applyEffectIntensitiesToRoot`
  writes. `tests/client/visual-effects.test.js` §5b then resolves the real
  cascade over `gui/viewscreen-hud.html` itself, in both directions.
- `npx playwright test --project=render viewscreen-effects.render.spec.js` —
  the real WASM render path: a Display-tab press crosses the seam, the live
  shake channel emits only zeros, the vignette stops pulsing while staying fully
  lit, and — the case only a real engine can judge — turning the interface's
  animation off leaves the red-alert pulse looping at its own period while the
  decorative spinner really does stop. **Needs a Trunk bundle**; it attaches a
  default and a reduced capture for §3 below.

---

## 3. The human half

The automated checks cannot answer the two questions that finally matter, and
PRD #1418 says so in as many words: *"Evaluate flashing separately from motion,
including combined combat/UI effects in normal and reduced modes. A Reduce
effects toggle is not evidence that default flashing is acceptable."*

Record the build revision, the surface, the viewport/scale and the settings
used for every line below.

### 3.1 Flashing, judged on its own

- [ ] On a **shared display at room viewing distance** (see
      `docs/acceptance/1421-device-matrix.md` for the recorded distance), take a
      ship to red alert **and** into weapons fire, so the red-alert vignette
      pulse and the shield-hit white flash overlap. Watch for 60 seconds at
      **Full**.
      - Record: is any of it uncomfortable, and at what rate does the combined
        frame actually change? _(This is a judgement, not a pass mark. If it is
        uncomfortable at Full, that is a defect to file — not something the
        Gentle stop excuses.)_
- [ ] Repeat at **Gentle**. The pulse period should be visibly longer and the
      white jolt visibly dimmer, and a shield hit must still be noticeable.
- [ ] Repeat at **Off**. The alert must still be unmistakably a ship at red
      alert from the back of the room.

- [ ] On the **native host** (the one runtime where the Viewscreen's overlay is
      its own document — §1a — and the one no Playwright project can reach), take
      a ship to red alert and set flashes to **Off** on the host's own Display
      tab. The vignette must stop pulsing *on the projected screen* while staying
      fully lit, without restarting the host; set it back to **Full** and the
      pulse must return. Then quit and relaunch the host with flashes still Off
      and confirm the glow comes up held rather than pulsing for the first press.

### 3.2 Shake

- [ ] Take sustained hull damage with shake at **Full**, **Gentle** and **Off**
      on the shared display. At Off the picture must not move at all, and the
      HULL readout must still fall.
- [ ] Confirm the setting survives a host restart (native) and a page reload
      (browser).

### 3.3 One effect at a time

- [ ] On a **phone console**, set flashes **Off** and leave interface animation
      at **Full**. The red bezel must hold steady while the loading spinner and
      ready glow still animate. Then swap the two.
- [ ] Press **Reduce effects**, then raise interface animation back to **Full**
      on its own. The flash must stay where the preset put it.

### 3.4 The surfaces that do not offer an effect

- [ ] On a **phone console**, confirm there is **no** screen-shake control and
      that the sentence explaining why is present and readable.
- [ ] In a **Game Master session**, open the cog's Display tab and confirm there
      is no shake or flash control, and that both absences are explained.

### 3.5 Enlarged text and forced colours

- [ ] At **200%** text on the phone console and on the shared display, every
      effect control is reachable, its label is not clipped, and its status line
      is readable.
- [ ] In Windows High Contrast, the pressed state of each effect control is
      still distinguishable (it is a real `aria-pressed` button, so the focus
      ring and the border carry it).

---

## 4. Result

| Date | Build revision | Surface | Settings | Observation |
|---|---|---|---|---|
| | | | | |
