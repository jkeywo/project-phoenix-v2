# Keyboard operability — the conventions

Issue #1170 made **one** console operable from the keyboard: the Destroyer's
Tactical (Weapons) console. It is the tracer for PRD #1168 (T1 Interaction
Foundations), and its job was less to fix one screen than to **set the
conventions the family sweeps copy**. Issues #1176–#1178 apply these same
patterns to the remaining consoles; this guide is what they follow so the
bridge ends up with one keyboard contract rather than thirty-eight.

Everything here is deliberately thin. The keybinding **registry** and
player-facing remapping are T2's input-and-feedback layer — not this band. In
T1 a component's key handlers bind straight to the actions it already
dispatches, and T2 later lifts those bindings into a remappable registry
without redesigning the component. Do not build a registry here.

---

## 1. The keyboard contract

Four keys, and nothing a console author has to invent per screen:

| Key | Does |
| --- | --- |
| **Tab / Shift+Tab** | Move between the console's **components** — one stop per component, never one per control. |
| **Arrow keys** | Move **within** a composite widget (a toolbar of buttons, a scope's ring of contacts). |
| **Enter / Space** | Activate the focused control. |
| **Escape** | Close a layer (overlay, modal). *(Modal focus-trap + Escape is a sibling slice of #1168, not #1170; noted here so the contract reads whole.)* |

The one-stop-per-component rule is the whole reason dense consoles stay usable:
a tactical console has a radar plus three weapon panels with a dozen buttons
between them, and without roving tabindex that is a dozen-plus Tab presses to
cross one screen.

Helm is untouched. Its existing key relay (`gui/key-relay.js`) and gamepad
bindings are a separate, already-working path; do not fold Helm into this.

---

## 2. Focus styling — a token, adopted once

Focus is a **token-system** concern, not a per-component outline. There is a
documented **pair** in `gui/tokens.css`:

```css
--focus-ring:          var(--signal);   /* 11.16:1 on --surface-base — standard */
--focus-ring-contrast: #ffffff;         /* 18.79:1 on --surface-base — data-contrast="more" (issue #1171) */
--focus-ring-width:    2px;
--focus-ring-offset:   2px;
```

- `--focus-ring` is the standard ring. `--focus-ring-contrast` is the
  high-contrast ring that `data-contrast="more"` (the accessibility profile,
  `gui/accessibility-profile.js`) swaps in. **#1170 defines and adopts the
  standard half; #1171 wires the `data-contrast` swap to the contrast half** —
  which is why the contrast value already exists here, defined and usable.
- The **shared control family** (`gui/components/ph-console-styles.js`) is the
  ONE place that reads these. It is adopted into every `ph-*` shadow root *and*
  into the console document, so a single `:focus-visible` rule reaches every
  control on both sides of every shadow boundary.

**Do not** write a per-component focus outline. If a new control is not getting
a visible ring, the fix is that it is not in a scope that adopts the shared
family — adopt the family, don't hand-roll an outline.

Two shapes, because the family has two silhouettes:

- **Default:** a plain outset `outline` from the tokens. Serves mode toggles,
  overlay toggles, and any composite host that takes `tabindex` (the radar).
- **`.btn` / `.mini-btn`:** these clip to a chamfer, and a `clip-path` clips an
  outset outline away with it — so they draw the ring **inset**, as a
  `box-shadow` on the recessed `.btn-bg` / `.mini-bg`, where the clip cannot eat
  it. It reads over the armed-green and danger-red fills alike.

Use `:focus-visible`, never `:focus` — a pointer press must not paint a ring.

---

## 3. Roles and accessible names

`gui/` had **zero** `role` attributes before this. The conventions:

### Toolbars — a group of command buttons

A composite that is a set of buttons the arrow keys rove between is a
**toolbar**. On the host element:

```js
this.setAttribute('role', 'toolbar');
this.setAttribute('aria-orientation', 'vertical');   // arrows the toolbar owns
this.setAttribute('aria-label', t('component.phasers.title'));
```

The accessible **name is the string id the visible heading already uses**, so
the two can never drift. `ph-phasers-controls`, `ph-blasters-controls` and
`ph-torpedo-controls` are the worked examples.

### Canvas scopes — a focusable group

A `<canvas>` scope has no per-contact DOM to rove between (a structured contact
list for a screen reader is **out of scope** for this band). It is a single
focusable **group**:

```js
this.setAttribute('role', 'group');
this.setAttribute('aria-label', t('component.tactical_radar.label'));
this.setAttribute('tabindex', '0');
```

Arrow keys then cycle a target cursor and Enter/Space lock it (see §4).

### Every interactive control has a name

Native buttons take their name from their text (`FIRE`, `MANUAL`, `Intel`) — no
`aria-label` needed, and none wanted, since a redundant one just drifts.
**Glyph-only** controls are the exception: the `−` / `+` volley steppers get an
`aria-label`, because "minus" is not an identifiable name. That label is
player-visible English, so it rides the string catalogue like everything else:

```js
minusBtn.setAttribute('aria-label', t('component.torpedoes.volley_decrease'));
```

---

## 4. Reuse the roving-tabindex helper — don't re-derive it

`gui/roving-tabindex.js` is the shared rule, generalised from the Hero Bar's
`heroBarKeyTarget` (PRD #1092) so every composite reuses it instead of each
re-deriving "which control does ArrowDown land on".

```js
import { installRovingTabindex, syncRovingTabindex } from '../roving-tabindex.js';
```

- **`rovingKeyTarget(count, currentIndex, key, orientation)`** — the pure core.
  Wraps like the Hero Bar; orientation-aware (`'vertical'` owns Up/Down,
  `'horizontal'` Left/Right, `'both'` all four). Home/End jump to the ends.
- **`installRovingTabindex(host, { getItems, orientation })`** — binds
  arrow-key roving to the host's `keydown`, reading items fresh each press so a
  group whose controls come and go needs no re-binding. **Leaves Enter/Space
  alone** — the controls are native buttons that already activate on both, so
  binding them would be a behaviour fork.
- **`syncRovingTabindex(items, activeIndex)`** — leaves exactly one item in the
  tab order. **Call it at the end of every render** so a reconciled control set
  keeps its single Tab stop.

Two composite patterns, both in the tracer:

1. **Toolbar** (roving tabindex) — the weapon panels. `installRovingTabindex` +
   `syncRovingTabindex` after each render.
2. **Canvas scope** (single focusable + arrow-cycle) — the radar. The host is
   the one Tab stop; arrow keys move a cursor over `rovingKeyTarget`, Enter
   locks it. There is no roving *tabindex* here because there are no child
   elements to rove between — just a cursor over a data list.

---

## 5. Handlers dispatch the SAME named actions as touch

The keyboard must never grow its own behaviour. A key handler dispatches the
**same `action-map.js` named action** the pointer path already does — no second
code path, no binding registry (that is T2).

- A native button already fires its `click` handler on Enter/Space, so nothing
  is needed: `fire_phaser`, `fire_torpedo`, the steppers, the mode toggle.
- The exception is a control whose pointer behaviour is **not** a click. The
  blaster is hold-to-fire (mousedown charges, mouseup fires), so it gets a
  `keydown`/`keyup` pair that mirrors that onto the **same** `charge_blaster_start`
  / `fire_blaster` actions. Mirror the pointer; do not invent a keyboard-only
  action.
- The radar's Enter emits the **same `set_target`** a tap does — one
  designation path, keyed on the cursor instead of the tap point.

---

## 6. How to make the next console keyboard-operable (the #1176–#1178 recipe)

1. For each custom composite, add `role` + `aria-label` (§3), naming it from the
   string id its heading already uses.
2. If it is a button group, `installRovingTabindex` on the host and
   `syncRovingTabindex` at the end of its render (§4, pattern 1).
3. If it is a canvas scope, make the host the Tab stop and cycle a cursor with
   `rovingKeyTarget` (§4, pattern 2).
4. Give every glyph-only control an `aria-label` through the string catalogue
   (§3); leave text-labelled native buttons alone.
5. For any hold/drag pointer control, add a `keydown`/`keyup` pair onto the
   **existing** named actions (§5). Everything else already activates on
   Enter/Space for free.
6. Adopt the shared control family so the focus ring appears — never write an
   outline (§2).
7. Once the family is converted, **delete its block from the #1175 allow-list**
   (`DEBT` in `tests/client/interaction-floors.test.js`). That deletion *is* the
   sweep's done-ness — and it is not optional: a converted component left in the
   list fails the `allow-list is honest` test (§7).

---

## 7. The structural floors and the allow-list (#1175)

The recipe above is enforced the #1023 way — **structural tests over source**,
extending the control-floors mechanism, no external audit tooling. Issue #1175
added four floors that enumerate the whole console surface — every
`gui/components/ph-*.js` and every per-hull `gui/<hull>/*.html`, read live from
disk — and assert, per control:

| Floor | Asserts |
| --- | --- |
| **Focus-token adoption** | every component adopts the shared control family (directly, or by extending a sibling that does), so its controls get the §2 ring. |
| **Focusability** | every interactive control is a focusable thing — a native control, or a bare element given a `tabindex`. A `pointerdown` on a plain `<div>` with neither fails. |
| **Keyboard-reachability** | no control is stranded — `tabindex="-1"` with no roving to bring it back, or a surface with no focus target at all. |
| **Accessible name + role** | every control exposes a name (its text, a `t()` string id, or an `aria-label`); every custom composite exposes a role **and** a name. |

**What "a control" is** is decided structurally, from what the author wrote (as
control-floors decides "a control" from `cursor: pointer` and `<button>`): a
`document.createElement('button')`, a `<button>`/`<input>`/`<select>` in markup,
or a `pointerdown`/`click` wired onto a bare element. A name is present when the
control gets text — its own, a descendant's, or a runtime fill by `id` — or an
`aria-label`. The scanner is in `tests/client/interaction-scan.js`, shared the
way `css-scan.js` is; the coarse edge (a component that mixes a real `<button>`
with a delegated `<div>` click reads as focusable, and that div escapes) is why
the *sweep's* fine-grained conversion still carries its own jsdom + smoke tests.

**The allow-list is the mechanism the sweeps run against.** `DEBT`, grouped by
sweep (#1176/#1177/#1178), names every component that fails a floor today; the
floors run green against it. A sweep converts its family, then deletes its
block. Two tests keep that honest: a still-failing entry is acknowledged debt,
but an entry that **no longer fails** breaks `the allow-list is honest` — so a
fix cannot quietly leave a component listed. The tracer (the Tactical console)
is on neither list and passes every floor clean. Structural non-controls take an
`EXEMPT` entry with a reason instead (e.g. `ph-radar`, the base scope canvas
that is always wrapped by a labelled group and so is never its own tab stop).

## 8. What proves it

- **`tests/client/roving-tabindex.test.js`** — the pure helper.
- **`tests/client/interaction-scan.test.js`** — the shared floor scanner, with
  the **negative fixtures** that prove each floor can fail (an unnamed button, a
  glyph with no `aria-label`, an unreachable drag widget, a stranded control).
- **`tests/client/interaction-floors.test.js`** — the four floors over the live
  console surface, the debt allow-list, and the honesty check that forces a
  sweep to strike off what it fixes.
- **`tests/client/tactical-keyboard.test.js`** — roles, names, the single Tab
  stop, the glyph steppers' names, and the two key handlers (blaster
  hold-to-fire, radar cursor) in jsdom.
- **`tests/smoke/tactical-keyboard.spec.js`** — the end-to-end claim: the
  console's principal actions all fire from the keyboard, with a pointer-event
  guard asserting not one mouse/pointer/touch event was used. (Runs in CI's
  build+smoke jobs — it needs the built `dist/` and a browser.)
