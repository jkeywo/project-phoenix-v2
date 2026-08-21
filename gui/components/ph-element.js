/**
 * gui/components/ph-element.js — PhElement base class (issue #1231, T4.C0 of
 * the console-seam programme).
 *
 * Every `ph-*` component today hand-rolls the same six lines in its
 * constructor (attachShadow, phAdoptConsoleStyles, build a <template>,
 * append it, wire sendAction in connectedCallback, a `$(id)` shadow-lookup
 * helper) and the same synchronous `set state(v) { this.#state = v;
 * this.#render(); }` pair. This is that boilerplate, written once, as a base
 * class. Purely additive: nothing in the repo extends it yet (see the
 * programme's phase 4 — existing components migrate one PR at a time,
 * fat-suite leaves first, radars last). Adopting it is a later issue.
 *
 * ── Node-safety ──────────────────────────────────────────────────────────
 *
 * Several existing components (e.g. `ph-tutorial-overlay.js`) are imported
 * transitively by plain-Node vitest suites (`console-core.js` imports it,
 * and a plain-Node suite imports `console-core.js`) where `HTMLElement` does
 * not exist. The guard that makes that safe lives HERE, in the base class,
 * so no subclass needs its own copy of it — every `class PhFoo extends
 * PhElement` is Node-import-safe for free.
 *
 * ── The field-initialiser ordering hazard ───────────────────────────────
 *
 * A subclass MUST cache its shadow refs by assigning a PLAIN property inside
 * `onTemplate()` (`this.button = this.$('go')`) — and must NOT also declare
 * that name as a class field anywhere in its body, public or private
 * (`button = null;` / `#button = null;`). Declaring it breaks this in two
 * different ways, and both trace back to the same spec mechanic: a derived
 * class's OWN field initialisers run at the moment ITS OWN `super()` call
 * returns — which, for a `PhFoo extends PhElement` with no constructor of
 * its own, is only after `PhElement`'s *entire* constructor body (attachShadow
 * → adopt styles → build template → `onTemplate()`) has finished and
 * unwound all the way back out. `onTemplate()` therefore always runs BEFORE
 * the subclass's own fields exist yet, not after:
 *
 *   - A declared PRIVATE field (`#button`) has a hard brand check: writing
 *     `this.#button = …` before `PhFoo`'s own field-init phase has run
 *     throws `TypeError: Cannot write private member #button to an object
 *     whose class did not declare it` — this is not hypothetical, it is
 *     the exact crash a first draft of this file's own test suite hit.
 *   - A declared PUBLIC field (`button;` or `button = null;`) has no brand
 *     check, so the assignment in `onTemplate()` "succeeds" — and is then
 *     silently overwritten back to the field's declared initial value
 *     moments later, when `PhFoo`'s own field-init phase finally runs. The
 *     ref renders fine in this constructor call and is `null` forever after.
 *
 * A property with NO field declaration at all has neither problem — it is
 * an ordinary dynamic assignment, unaffected by any class's field-init
 * phase — which is why "plain property, no matching field declaration" is
 * the rule, not merely "use onTemplate instead of a field initialiser".
 */

import { phAdoptConsoleStyles } from './ph-console-styles.js';

// Node-safe base: this module — and therefore every PhElement subclass,
// transitively — is importable from plain-Node vitest suites where
// HTMLElement does not exist. Putting the guard here, once, is the whole
// point of a shared base class: no subclass writes this line again.
const Base = typeof HTMLElement !== 'undefined' ? HTMLElement : class {};

export class PhElement extends Base {
  /**
   * Opt-in for canvas-backed components (radars): batches `render()` calls
   * via requestAnimationFrame instead of calling it synchronously on every
   * `state` assignment. Default `false` — `set state` is synchronous by
   * default because the 34 existing component test suites set
   * `el.state = {...}` and assert the shadow DOM on the very next line;
   * changing that default would break every one of them.
   */
  static coalesce = false;

  #state = null;
  #rafId = null;

  constructor() {
    super();
    this.attachShadow({ mode: 'open' });
    // Every component adopts the shared control family (module 1 of PRD
    // #1023): custom properties cross a shadow boundary, class rules do not.
    phAdoptConsoleStyles(this.shadowRoot);
    const tpl = document.createElement('template');
    tpl.innerHTML = this.template();
    this.shadowRoot.appendChild(tpl.content.cloneNode(true));
    // See the field-initialiser ordering hazard note above: this is the
    // first point at which the template is guaranteed to exist, so it is
    // where a subclass caches its shadow refs — as plain properties, never
    // as a declared class field (public or private), because this call
    // happens strictly BEFORE the subclass's own field-init phase runs.
    this.onTemplate?.();
  }

  connectedCallback() {
    this.sendAction ??= window.sendAction;
  }

  /**
   * Override to supply this element's shadow-DOM markup. Called once, from
   * the base constructor, before the element has any shadow children.
   * @returns {string}
   */
  template() { return ''; }

  /**
   * Shadow-root element lookup — the `root.getElementById(id)` one-liner
   * every component already reaches for, spelled the same way everywhere.
   * @param {string} id
   */
  $(id) { return this.shadowRoot.getElementById(id); }

  /**
   * Assigning `.state` is how every console pushes a fresh payload into a
   * component. By default this renders synchronously, on the same tick —
   * `render()` runs before the setter returns. A subclass that sets
   * `static coalesce = true` instead batches: repeated assignments within
   * one frame collapse to a single `render()` call (the LAST value set
   * wins), scheduled on requestAnimationFrame. `state` itself always
   * reflects the latest assignment immediately, coalesced or not — only the
   * render is deferred.
   */
  set state(v) {
    this.#state = v || {};
    if (!this.constructor.coalesce) {
      this.render(this.#state);
      return;
    }
    if (this.#rafId != null) return;
    this.#rafId = requestAnimationFrame(() => {
      this.#rafId = null;
      this.render(this.#state);
    });
  }

  get state() { return this.#state; }

  /**
   * Override to paint `state` into the shadow DOM. No-op by default so a
   * subclass that has not defined one yet does not throw on first render.
   * @param {object} _state
   */
  render(_state) {}
}

/**
 * Define a custom element, guarded so it is both Node-safe (no-op where
 * `customElements` does not exist) and idempotent (a second call for a tag
 * already defined — repeated module evaluation, HMR — does not throw the
 * `NotSupportedError` `customElements.define` raises on redefinition).
 *
 * @param {string} tag
 * @param {typeof PhElement} ctor
 */
export function phDefine(tag, ctor) {
  if (typeof customElements === 'undefined') return;
  if (customElements.get(tag)) return;
  customElements.define(tag, ctor);
}
