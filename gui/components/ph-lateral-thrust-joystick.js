// strings-boot first: its top-level await delays this module's evaluation —
// and therefore this element's registration and upgrade — until the string
// table is loaded, so the constructor's template t() calls never see an
// empty table. No-op in Node tests (setup-strings.js loads the table there).
import '../strings-boot.js';
import { t } from '../strings.js';
import {
  HELM_ACTION_CONTEXT,
  HELM_LATERAL_ACTION_ID,
} from '../stations/helm-actions.js';
import { PhElement, phDefine } from './ph-element.js';
import { createContinuousHelmInput } from './continuous-helm-input.js';

export class PhLateralThrustJoystick extends PhElement {
  #value = 0;
  #input = createContinuousHelmInput({
    keys: ['KeyQ', 'KeyE', 'ArrowLeft', 'ArrowRight'],
    auto: () => !!this.state?.auto,
    immediatePointer: true,
    pointer: (e) => this.#setFromPointer(e.clientX),
    keyboard: (keys) => this.#sampleKeys(keys),
    hasValue: () => this.#value !== 0,
    reset: () => {
      this.#value = 0;
    },
    paint: () => {
      this.#applyNubPosition();
      this.#updateReadout();
    },
    send: () => this.#sendAction(),
  });

  template() {
    return `
  <style>
    :host { display: flex; flex-direction: column; align-items: center; font-family: 'JetBrains Mono', monospace; color: var(--ink); }
    :host * { box-sizing: border-box; }
    .header { display: flex; justify-content: space-between; align-items: center; width: 100%; font-size: var(--text-sm); letter-spacing: 0.2em; color: var(--ink-dim); text-transform: uppercase; margin-bottom: 0.3rem; }
    .auto-badge { font-size: var(--text-xs); color: var(--reloading); border: 1px solid var(--reloading); padding: 0.1rem 0.4rem; letter-spacing: 0.2em; }
    /* 180px is the pad's SIZE, not its width — see the note on
       ph-helm-joystick's well (issue #1375). The drag maths reads the track's
       own rect, so a narrower rail gives a shorter throw rather than a pad
       hanging out over the scope beside it. */
    .track {
      position: relative; width: min(180px, 100%); height: 32px; border-radius: 16px;
      background: linear-gradient(to right, var(--surface-panel) 0%, var(--surface-panel) 50%, var(--surface-panel) 100%);
      border: 1px solid var(--line-faint); cursor: grab; touch-action: none; flex-shrink: 0;
    }
    .track:active { cursor: grabbing; }
    .track.auto { cursor: default; }
    .center-line { position: absolute; left: 50%; top: 4px; bottom: 4px; width: 1px; background: rgba(var(--rgb-cyan), 0.25); pointer-events: none; }
    .labels { position: absolute; inset: 0; display: flex; justify-content: space-between; align-items: center; padding: 0 8px; pointer-events: none; font-size: var(--text-xs); color: rgba(var(--rgb-cyan), 0.35); letter-spacing: 0.15em; }
    .nub {
      position: absolute; left: 50%; top: 50%; width: 28px; height: 28px; border-radius: 50%;
      background: radial-gradient(circle at 35% 30%, var(--cyan-dim) 0%, var(--surface-panel-up) 50%, var(--surface-panel) 100%);
      border: 1.5px solid rgba(var(--rgb-cyan), 0.8); box-shadow: 0 0 12px rgba(var(--rgb-cyan), 0.3);
      transform: translate(-50%, -50%); pointer-events: none; transition: none;
      will-change: margin-left;
    }
    .readout { font-size: var(--text-sm); color: rgba(var(--rgb-cyan), 0.8); letter-spacing: 0.1em; margin-top: 0.25rem; }
  </style>
  <div class="header">
    <span>${t('component.lateral.title')}</span>
    <span class="auto-badge" id="auto-badge" style="display:none">${t('console.common.auto')}</span>
  </div>
  <div class="track" id="track">
    <div class="center-line"></div>
    <div class="labels"><span>${t('console.common.port')}</span><span>${t('console.common.stbd')}</span></div>
    <div class="nub" id="nub"></div>
  </div>
  <div class="readout" id="readout">0.00</div>
`;
  }

  connectedCallback() {
    super.connectedCallback();
    this.#input.connect(this.shadowRoot.getElementById('track'));
    // Focusable, named group (issue #1176). The drag track was a bare <div>
    // the keyboard could not reach or name; the host becomes the one Tab stop.
    // `role="group"` is the honest role for a composite whose continuous axis a
    // screen reader cannot enumerate, and the name rides the string catalogue.
    // The focus ring comes from the document-adopted control family.
    this.setAttribute('role', 'group');
    this.setAttribute('aria-label', t('component.lateral.label'));
    if (!this.hasAttribute('tabindex')) this.setAttribute('tabindex', '0');
    // Q/E + arrows drive the same semantic lateral axis as pointer and the
    // parent-owned gamepad runtime. Deliberate
    // key-relay coexistence (issue #1176): one document-level handler — the
    // same path gui/key-relay.js relays — so a focused track adds a Tab stop
    // and a name but no second arrow handler, and the key state is a set keyed
    // by code so a native + relayed press cannot double-fire.
  }

  disconnectedCallback() {
    this.#input.disconnect();
  }

  render(state) {
    const auto = state ? !!state.auto : false;
    const root = this.shadowRoot;
    const badge = root.getElementById('auto-badge');
    const track = root.getElementById('track');
    badge.style.display = auto ? 'inline' : 'none';
    track.classList.toggle('auto', auto);
    if (auto && !this.#input.hasPointer()) {
      this.#value = 0;
      this.#applyNubPosition();
      this.#updateReadout();
    }
  }

  #setFromPointer(clientX) {
    const track = this.shadowRoot.getElementById('track');
    const r = track.getBoundingClientRect();
    const cx = r.left + r.width / 2;
    const half = r.width / 2 - 16;
    let dx = (clientX - cx) / half;
    if (dx > 1) dx = 1;
    if (dx < -1) dx = -1;
    this.#value = dx;
  }

  #applyNubPosition() {
    const track = this.shadowRoot.getElementById('track');
    const r = track.getBoundingClientRect();
    const half = r.width / 2 - 16;
    const nub = this.shadowRoot.getElementById('nub');
    nub.style.marginLeft = (this.#value * half) + 'px';
  }

  #updateReadout() {
    const root = this.shadowRoot;
    const v = this.#value || 0;
    root.getElementById('readout').textContent = (v >= 0 ? '+' : '') + v.toFixed(2);
  }

  #sampleKeys(keys) {
    let kv = 0;
    if (keys['KeyQ'] || keys['ArrowLeft']) kv -= 1;
    if (keys['KeyE'] || keys['ArrowRight']) kv += 1;
    let nv = kv;
    if (nv > 1) nv = 1;
    if (nv < -1) nv = -1;
    if (nv === 0) return false;
    this.#value = nv;
    return true;
  }

  #sendAction() {
    const activate = typeof window !== 'undefined' && window.activateSemanticAction;
    if (typeof activate === 'function') {
      activate(HELM_LATERAL_ACTION_ID, {
        context: HELM_ACTION_CONTEXT, source: 'control', value: this.#value || 0,
      });
    }
  }
}

phDefine('ph-lateral-thrust-joystick', PhLateralThrustJoystick);
