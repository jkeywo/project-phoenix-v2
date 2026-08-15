// gui/components/ph-console-styles.js — the console control family.
//
// ── Why this is a JS module and not a .css file ────────────────────────────
//
// Custom properties inherit THROUGH a shadow boundary, so every ph-* component
// reads gui/tokens.css off its document's :root for free. Class rules do not.
// A component that wants the fleet's button has to be handed the rules, which
// is what this module does: it carries them as a constructable stylesheet that
// `phAdoptConsoleStyles` pushes into a shadow root.
//
// gui/console-core.js adopts the same sheet into the console DOCUMENT, so the
// handful of consoles that write `class="btn"` in light DOM get the identical
// control rather than a second copy of them in console.css. One definition,
// both sides of the boundary — which is the whole point, because there used to
// be two.
//
// ── The two scales this collapses ──────────────────────────────────────────
//
// `.btn` meant two different controls depending on which side of a shadow root
// you were on. console.css's was 5.5rem tall with a 1.75rem label and a
// 0.875rem chamfer; this file's was 1.9rem tall with a 0.62rem label and a
// 0.34rem chamfer — the same design at 2.9× the scale, under the same name,
// with the gradients and LED geometry written out twice and already drifting.
// console.css also carried a `.chip`, a fourth scale, which no markup in the
// repo used.
//
// Now: ONE family, three size variants, every dimension from a token.
//
//   .btn            the LED pill        (default size: md)
//   .btn--sm        square stepper      (was .mini-btn)
//   .btn--md        compact pill        (was this file's .btn)
//   .btn--lg        full-screen pill    (was console.css's .btn)
//   .btn.plain      no LED, centred label
//   .mini-btn       alias of .btn--sm, kept for existing markup
//
// Colour variants (.armed / .danger / .tactical / :disabled) are written once
// and apply at every size.
//
// ── What this deliberately does NOT do ─────────────────────────────────────
//
// `--control-hit-min` (44px) is defined in tokens.css and not applied here.
// Raising every packed console control to the touch floor re-flows columns
// that need looking at on a real phone; that is module 3's sweep. The three
// heights below are today's rendered sizes, named rather than changed, so the
// sweep has exactly one place to move them from.

const CSS = `
/* ── Size variants ───────────────────────────────────────────────
   A variant sets four properties and nothing else; every rule below reads
   them, so a size is a data change rather than a second copy of the design. */
:host,
:root {
  --btn-h:    var(--control-h-md);
  --btn-cham: var(--control-cham-md);
  --btn-font: var(--control-font-md);
  --btn-pad:  var(--control-pad-md);
}
.btn--sm {
  --btn-h:    var(--control-h-sm);
  --btn-cham: var(--control-cham-sm);
  --btn-font: var(--control-font-sm);
  --btn-pad:  var(--control-pad-sm);
}
.btn--md {
  --btn-h:    var(--control-h-md);
  --btn-cham: var(--control-cham-md);
  --btn-font: var(--control-font-md);
  --btn-pad:  var(--control-pad-md);
}
.btn--lg {
  --btn-h:    var(--control-h-lg);
  --btn-cham: var(--control-cham-lg);
  --btn-font: var(--control-font-lg);
  --btn-pad:  var(--control-pad-lg);
}

/* ── The chamfered silhouette, cut once ──────────────────────────
   Eight points off one radius. Every control in the family clips to it, and
   the recessed body inside clips to a slightly smaller one. */
.btn,
.mini-btn {
  position: relative;
  height: var(--btn-h);
  padding: var(--btn-pad);
  display: inline-flex; align-items: center;
  font-family: var(--font-display);
  font-weight: 600;
  font-size: var(--btn-font);
  letter-spacing: var(--tracking-wide);
  text-transform: uppercase;
  color: var(--ink-dim);
  background: linear-gradient(180deg,
    var(--edge-strong) 0%, var(--edge) 50%, var(--surface-lift) 100%);
  clip-path: polygon(
    var(--btn-cham) 0, calc(100% - var(--btn-cham)) 0, 100% var(--btn-cham),
    100% calc(100% - var(--btn-cham)), calc(100% - var(--btn-cham)) 100%,
    var(--btn-cham) 100%, 0 calc(100% - var(--btn-cham)), 0 var(--btn-cham)
  );
  border: none;
  cursor: pointer;
  user-select: none;
  touch-action: manipulation;
}

.btn > .btn-bg,
.mini-btn > .mini-bg {
  position: absolute; inset: var(--control-inset);
  --btn-cham: calc(var(--btn-cham) - 0.04rem);
  background: linear-gradient(180deg,
    var(--surface-lift) 0%, var(--surface-base) 50%, var(--surface-deep) 100%);
  clip-path: polygon(
    var(--btn-cham) 0, calc(100% - var(--btn-cham)) 0, 100% var(--btn-cham),
    100% calc(100% - var(--btn-cham)), calc(100% - var(--btn-cham)) 100%,
    var(--btn-cham) 100%, 0 calc(100% - var(--btn-cham)), 0 var(--btn-cham)
  );
  z-index: 0;
}

.btn > .led,
.btn > .label,
.mini-btn > .lbl { position: relative; z-index: 1; }
.mini-btn > .lbl { line-height: 1; }

/* ── LED ─────────────────────────────────────────────────────────
   One lamp, recoloured per state. Each lit state is the accent's -bright
   rung at the hot spot falling to its base and -dim rungs. */
.btn .led {
  position: absolute;
  left: 0.5rem; top: 50%;
  transform: translateY(-50%);
  width: 0.6rem; height: 0.6rem;
  border-radius: 50%;
  background: radial-gradient(circle at 35% 30%,
    var(--edge-control) 0%, var(--surface-base) 70%);
  box-shadow: inset 0 0 3px rgba(var(--rgb-void), 0.6);
}
.btn .led.on    { background: radial-gradient(circle at 35% 30%, var(--loaded-bright) 0%, var(--loaded) 35%, var(--loaded-dim) 80%); box-shadow: 0 0 8px var(--loaded), inset 0 0 2px rgba(var(--rgb-white), 0.5); }
.btn .led.fire  { background: radial-gradient(circle at 35% 30%, var(--tactical-bright) 0%, var(--fire) 35%, var(--fire-dim) 80%); box-shadow: 0 0 8px var(--fire), inset 0 0 2px rgba(var(--rgb-white), 0.5); }
.btn .led.amber { background: radial-gradient(circle at 35% 30%, var(--reloading-bright) 0%, var(--reloading) 35%, var(--reloading-dim) 80%); box-shadow: 0 0 7px var(--reloading), inset 0 0 2px rgba(var(--rgb-white), 0.5); }
.btn .led.cyan  { background: radial-gradient(circle at 35% 30%, var(--cyan-bright) 0%, var(--cyan) 35%, var(--cyan-dim) 80%); box-shadow: 0 0 7px var(--cyan), inset 0 0 2px rgba(var(--rgb-white), 0.5); }

/* ── Colour variants ─────────────────────────────────────────────
   Each is the same two gradients on a different accent ramp. */
.btn.armed            { color: var(--loaded); background: linear-gradient(180deg, var(--loaded) 0%, var(--loaded-dim) 60%, var(--loaded-deep) 100%); }
.btn.armed > .btn-bg  { background: linear-gradient(180deg, var(--loaded-deep) 0%, var(--surface-deep) 100%); }
.btn.danger           { color: var(--fire-bright); background: linear-gradient(180deg, var(--fire-bright) 0%, var(--fire-dim) 60%, var(--fire-deep) 100%); }
.btn.danger > .btn-bg { background: linear-gradient(180deg, var(--fire-deep) 0%, var(--surface-void) 100%); }
.btn.tactical             { color: var(--tactical); background: linear-gradient(180deg, var(--tactical) 0%, var(--tactical-dim) 60%, var(--tactical-deep) 100%); }
.btn.tactical > .btn-bg   { background: linear-gradient(180deg, var(--tactical-deep) 0%, var(--surface-void) 100%); }

.btn.disabled, .btn:disabled {
  color: var(--ink-faint);
  background: linear-gradient(180deg, var(--surface-ridge) 0%, var(--surface-base) 100%);
  cursor: default;
}
.btn.disabled > .btn-bg, .btn:disabled > .btn-bg {
  background: linear-gradient(180deg, var(--surface-deep) 0%, var(--surface-void) 100%);
}
.btn:disabled .led:not(.keep) {
  background: radial-gradient(circle at 35% 30%, var(--edge-control) 0%, var(--surface-base) 70%);
  box-shadow: inset 0 0 3px rgba(var(--rgb-void), 0.6);
}
/* Selected: the cyan state the retired .chip tube selector carried. */
.btn.selected           { color: var(--cyan-bright); background: linear-gradient(180deg, var(--cyan) 0%, var(--cyan-dim) 60%, var(--cyan-deep) 100%); }
.btn.selected > .btn-bg { background: linear-gradient(180deg, var(--cyan-dim) 0%, var(--cyan-deep) 100%); }

.btn:not(:disabled):hover,
.mini-btn:not(:disabled):hover { filter: brightness(1.15); }
.mini-btn:disabled { opacity: 0.35; cursor: default; }

/* No-LED pill: label centred, no room reserved on the left for a lamp. */
.btn.plain { padding: 0 0.7rem; justify-content: center; }

/* ── Square stepper ──────────────────────────────────────────────
   The one member of the family that is a fixed square rather than a pill,
   because it holds a single glyph. Same silhouette, same gradients. */
.btn--sm,
.mini-btn {
  --btn-h:    var(--control-h-sm);
  --btn-cham: var(--control-cham-sm);
  --btn-font: var(--control-font-sm);
  width: var(--control-h-sm); height: var(--control-h-sm);
  flex-shrink: 0; padding: 0;
  justify-content: center;
  font-weight: 700;
}
`;

let sheet;
try {
  sheet = new CSSStyleSheet();
  sheet.replaceSync(CSS);
} catch (e) {
  // Older engines without constructable stylesheets: callers fall back to a
  // <style> element (see phAdoptConsoleStyles).
  sheet = null;
}

export const phConsoleStyles = sheet;
export const phConsoleStylesText = CSS;

/**
 * Adopt the shared control family into a shadow root — or into a document.
 *
 * Every ph-* component calls this from its constructor. It is not optional and
 * not a per-component choice: five of thirty-six components adopted it once,
 * and the other thirty-one hand-rolled their own chrome, which is how the
 * fleet ended up with buttons that agreed on nothing.
 *
 * @param {ShadowRoot|Document} root
 */
export function phAdoptConsoleStyles(root) {
  if (!root) return;
  if (phConsoleStyles && 'adoptedStyleSheets' in root) {
    // Idempotent: console-core.js adopts into a document that a component may
    // already have adopted into, and a doubled sheet is a doubled cascade.
    if (root.adoptedStyleSheets.includes(phConsoleStyles)) return;
    root.adoptedStyleSheets = [...root.adoptedStyleSheets, phConsoleStyles];
  } else if (typeof document !== 'undefined' && root.appendChild) {
    const style = document.createElement('style');
    // Marked so a component's own stylesheet stays findable: this one is
    // appended FIRST (the shared family must lose to a component's overrides,
    // so it has to come earlier in the cascade), which would otherwise make it
    // whatever `shadowRoot.querySelector('style')` returns.
    style.setAttribute('data-ph-shared', '');
    style.textContent = phConsoleStylesText;
    (root.head || root).appendChild(style);
  }
}
