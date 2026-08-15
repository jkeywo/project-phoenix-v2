/**
 * gui/components/ph-tutorial-overlay.js — Contextual tutorial overlay card
 * (issue #916).
 *
 * Renders the `tutorial` block every console payload carries (merged by
 * `withTutorialOverlay` in gui/console-state.js): the single active
 * TOML-authored overlay for this station, or nothing. Mounted lazily into
 * every console iframe by gui/console-core.js — a console needs no per-file
 * HTML to gain tutorials; authoring `[[station.tutorial]]` in the ship TOML
 * is enough.
 *
 * State shape (see buildTutorialState in gui/tutorial-state.js):
 *   { active: { id, title, text, anchor? }, remaining: n } | null
 *
 * `title`/`text` are strings.csv ids resolved through t() here — never
 * pre-composed English. `anchor` names a light-DOM element id in the console
 * page; while the overlay is active that element carries the
 * `tutorial-highlight` class (styled in gui/console.css) so the player can
 * see which control the tip is about.
 *
 * Dismissing sends the `tutorial_dismiss` console action with the overlay id;
 * client.html folds it into the client-local tutorial progress and never
 * forwards it to the host.
 */

// strings-boot first: its top-level await delays this module's evaluation —
// and therefore this element's registration and upgrade — until the string
// table is loaded, so t() calls in render never see an empty table. No-op in
// Node tests (setup-strings.js loads the table there).
import '../strings-boot.js';
import { t } from '../strings.js';
import { phAdoptConsoleStyles } from './ph-console-styles.js';

// Node-safe base: gui/console-core.js imports this module and is itself
// imported by plain-Node vitest suites where HTMLElement does not exist.
const BaseElement = typeof HTMLElement !== 'undefined' ? HTMLElement : class {};

export class PhTutorialOverlay extends BaseElement {
  #state = null;
  #highlighted = null;

  constructor() {
    super();
    this.attachShadow({ mode: 'open' });
    // Every component adopts the shared control family (module 1 of PRD
    // #1023): custom properties cross a shadow boundary, class rules do not.
    phAdoptConsoleStyles(this.shadowRoot);
    const tpl = document.createElement('template');
    tpl.innerHTML = `
  <style>
    :host {
      position: fixed; left: 50%; bottom: 0.9rem; transform: translateX(-50%);
      z-index: 900; width: min(30rem, calc(100vw - 2rem));
      font-family: 'JetBrains Mono', monospace; color: var(--ink);
    }
    :host([hidden]) { display: none; }
    :host * { box-sizing: border-box; }
    .card {
      background: rgba(10, 20, 34, 0.92); border: 1px solid var(--cyan-dim);
      padding: 0.7rem 0.9rem; display: flex; flex-direction: column; gap: 0.35rem;
    }
    .eyebrow-row { display: flex; align-items: baseline; gap: 0.5rem; }
    .eyebrow {
      font-family: 'Chakra Petch', sans-serif; font-size: 0.6rem;
      letter-spacing: 0.25em; color: var(--cyan); text-transform: uppercase;
    }
    .more { margin-left: auto; font-size: 0.55rem; letter-spacing: 0.15em; color: var(--ink-faint); }
    .title {
      font-family: 'Chakra Petch', sans-serif; font-size: 0.85rem; font-weight: 600;
      letter-spacing: 0.12em; text-transform: uppercase; color: var(--ink);
    }
    .text { font-size: 0.7rem; line-height: 1.5; color: var(--ink-dim); }
    .dismiss {
      align-self: flex-end; font-family: 'Chakra Petch', sans-serif;
      font-size: 0.7rem; font-weight: 600; letter-spacing: 0.18em;
      text-transform: uppercase; color: var(--cyan); background: transparent;
      border: 1px solid var(--cyan-dim); padding: 0.3rem 0.8rem; cursor: pointer;
      touch-action: manipulation;
    }
    .dismiss:hover { background: rgba(26, 56, 72, 0.55); }
  </style>
  <div class="card">
    <div class="eyebrow-row">
      <span class="eyebrow">${t('component.tutorial.heading')}</span>
      <span class="more" id="more" hidden></span>
    </div>
    <div class="title" id="title"></div>
    <div class="text" id="text"></div>
    <button class="dismiss" id="dismiss">${t('component.tutorial.dismiss')}</button>
  </div>`;
    this.shadowRoot.appendChild(tpl.content.cloneNode(true));
    // Wire the dismiss button ONCE here — the button lives in this element's
    // own shadow root, so the listener's lifetime is the element's.
    // connectedCallback runs again on every re-parent and would stack
    // duplicate listeners (class fields like #dismiss are initialised before
    // the constructor body runs, so the handler exists at this point).
    this.shadowRoot.getElementById('dismiss').addEventListener('click', this.#dismiss);
    this.hidden = true;
  }

  connectedCallback() {
    this.sendAction ??= window.sendAction;
  }

  disconnectedCallback() {
    this.#clearHighlight();
  }

  #dismiss = () => {
    const active = this.#state && this.#state.active;
    if (!active || !this.sendAction) return;
    this.sendAction('tutorial_dismiss', { overlay_id: active.id });
  };

  set state(val) {
    this.#state = val || null;
    this.#render();
  }

  get state() { return this.#state; }

  #clearHighlight() {
    if (this.#highlighted) {
      this.#highlighted.classList.remove('tutorial-highlight');
      this.#highlighted = null;
    }
  }

  #render() {
    const active = this.#state && this.#state.active;
    this.#clearHighlight();
    if (!active) {
      this.hidden = true;
      return;
    }
    this.hidden = false;
    const root = this.shadowRoot;
    root.getElementById('title').textContent = t(active.title);
    root.getElementById('text').textContent = t(active.text);

    // "+N MORE" hint when further tips are queued behind this one.
    const more = root.getElementById('more');
    const queued = (this.#state.remaining || 0) - 1;
    if (queued > 0) {
      more.hidden = false;
      more.textContent = t('component.tutorial.more', { n: queued });
    } else {
      more.hidden = true;
      more.textContent = '';
    }

    // Highlight the anchored control in the console's light DOM.
    if (active.anchor) {
      const doc = this.ownerDocument || (typeof document !== 'undefined' ? document : null);
      const el = doc && doc.getElementById(active.anchor);
      if (el && el !== this) {
        el.classList.add('tutorial-highlight');
        this.#highlighted = el;
      }
    }
  }
}

if (typeof window !== 'undefined' && typeof customElements !== 'undefined'
    && !customElements.get('ph-tutorial-overlay')) {
  customElements.define('ph-tutorial-overlay', PhTutorialOverlay);
}
