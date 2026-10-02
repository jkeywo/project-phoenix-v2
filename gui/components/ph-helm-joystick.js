// strings-boot first: it populates the string table synchronously during its
// module evaluation, so this element's constructor-time template t() calls
// never see an empty table. No-op in Node tests (setup-strings.js loads the
// table there).
import '../strings-boot.js';
import { t } from '../strings.js';
import {
  HELM_ACTION_CONTEXT,
  HELM_STEERING_ACTION_ID,
  HELM_THRUST_ACTION_ID,
} from '../stations/helm-actions.js';
import { PhElement, phDefine } from './ph-element.js';
import { createContinuousHelmInput } from './continuous-helm-input.js';

export class PhHelmJoystick extends PhElement {
  #px = 0;
  #py = 0;
  #input = createContinuousHelmInput({
    keys: ['ArrowLeft', 'ArrowRight', 'ArrowUp', 'ArrowDown', 'KeyA', 'KeyD', 'KeyW', 'KeyS'],
    auto: () => !!this.state?.auto,
    immediatePointer: false,
    pointer: (e) => this.#setFromPointer(e.clientX, e.clientY),
    keyboard: (keys) => this.#sampleKeys(keys),
    hasValue: () => this.#px !== 0 || this.#py !== 0,
    reset: () => {
      this.#px = 0;
      this.#py = 0;
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
    .header { display: flex; justify-content: space-between; align-items: center; width: 100%; font-size: var(--text-sm); letter-spacing: 0.2em; color: var(--ink-dim); text-transform: uppercase; margin-bottom: 0.5rem; }
    .auto-badge { font-size: var(--text-xs); color: var(--reloading); border: 1px solid var(--reloading); padding: 0.1rem 0.4rem; letter-spacing: 0.2em; }
    /* 240px is the dial's SIZE, not its width: min(240px, 100%), so a rail
       narrower than the dial makes a smaller dial instead of a dial hanging
       out over whatever is beside the rail (issue #1375). Every pointer
       calculation below already measures the well's own rect, so a smaller
       dial is simply a smaller dial. The rail it sits in keeps a floor
       (--scope-rail-min in gui/console.css) so the dial cannot shrink past
       the point where the inset rings and the 56px nub stop making a dial. */
    .well {
      position: relative; width: min(240px, 100%); aspect-ratio: 1 / 1; height: auto; border-radius: 50%;
      background: radial-gradient(circle at center, var(--surface-panel) 0%, var(--surface-panel) 75%, var(--surface-abyss) 100%);
      border: 1px solid var(--line-faint); box-shadow: inset 0 0 0 4px var(--surface-panel), inset 0 0 0 5px rgba(var(--rgb-cyan), 0.35);
      cursor: grab; touch-action: none; flex-shrink: 0;
    }
    .well:active { cursor: grabbing; }
    .well.auto { cursor: default; }
    .ring { position: absolute; border-radius: 50%; border: 1px solid rgba(var(--rgb-cyan), 0.18); pointer-events: none; }
    .ring.outer { inset: 16px; }
    .ring.mid { inset: 52px; }
    .cross-h, .cross-v { position: absolute; background: rgba(var(--rgb-cyan), 0.14); pointer-events: none; }
    .cross-h { left: 18px; right: 18px; top: 50%; height: 1px; }
    .cross-v { top: 18px; bottom: 18px; left: 50%; width: 1px; }
    .arrow { position: absolute; width: 0; height: 0; pointer-events: none; opacity: 0.5; }
    .arrow.fwd { top: 8px; left: 50%; transform: translateX(-50%); border-left: 6px solid transparent; border-right: 6px solid transparent; border-bottom: 8px solid rgba(var(--rgb-cyan), 0.5); }
    .arrow.rev { bottom: 8px; left: 50%; transform: translateX(-50%); border-left: 6px solid transparent; border-right: 6px solid transparent; border-top: 8px solid rgba(var(--rgb-cyan), 0.5); }
    .arrow.port { left: 8px; top: 50%; transform: translateY(-50%); border-top: 6px solid transparent; border-bottom: 6px solid transparent; border-right: 8px solid rgba(var(--rgb-cyan), 0.5); }
    .arrow.stbd { right: 8px; top: 50%; transform: translateY(-50%); border-top: 6px solid transparent; border-bottom: 6px solid transparent; border-left: 8px solid rgba(var(--rgb-cyan), 0.5); }
    .ax-label { position: absolute; font-family: 'Chakra Petch', sans-serif; font-weight: 600; font-size: var(--text-xs); color: var(--ink-dim); letter-spacing: 0.22em; pointer-events: none; }
    .ax-label.fwd { top: 20px; left: 50%; transform: translateX(-50%); }
    .ax-label.rev { bottom: 20px; left: 50%; transform: translateX(-50%); }
    .ax-label.port { left: 18px; top: 50%; transform: translateY(-50%) rotate(-90deg); }
    .ax-label.stbd { right: 18px; top: 50%; transform: translateY(-50%) rotate(90deg); }
    .nub {
      position: absolute; left: 50%; top: 50%; width: 56px; height: 56px; border-radius: 50%;
      background: radial-gradient(circle at 35% 30%, var(--cyan-dim) 0%, var(--surface-panel-up) 50%, var(--surface-panel) 100%);
      border: 1.5px solid rgba(var(--rgb-cyan), 0.8); box-shadow: 0 0 20px rgba(var(--rgb-cyan), 0.35);
      transform: translate(-50%, -50%); pointer-events: none; transition: none;
      will-change: margin-left, margin-top;
    }
    .nub::after { content: ''; position: absolute; inset: 18px; border-radius: 50%; background: var(--surface-panel); border: 1px solid rgba(var(--rgb-cyan), 0.3); }
    .readout { display: flex; gap: 1rem; margin-top: 0.5rem; font-size: var(--text-md); color: rgba(var(--rgb-cyan), 0.8); letter-spacing: 0.1em; }
    .readout .sep { color: var(--ink-dim); }
  </style>
  <div class="header">
    <span>${t('component.helm_joystick.title')}</span>
    <span class="auto-badge" id="auto-badge" style="display:none">${t('console.common.auto')}</span>
  </div>
  <div class="well" id="well">
    <div class="ring outer"></div>
    <div class="ring mid"></div>
    <div class="cross-h"></div>
    <div class="cross-v"></div>
    <div class="arrow fwd"></div>
    <div class="arrow rev"></div>
    <div class="arrow port"></div>
    <div class="arrow stbd"></div>
    <div class="ax-label fwd">${t('component.helm_joystick.fwd')}</div>
    <div class="ax-label rev">${t('component.helm_joystick.rev')}</div>
    <div class="ax-label port">${t('console.common.port')}</div>
    <div class="ax-label stbd">${t('console.common.stbd')}</div>
    <div class="nub" id="nub"></div>
  </div>
  <div class="readout">
    <span id="thrust-readout">+0.00</span>
    <span class="sep">/</span>
    <span id="yaw-readout">+0.00</span>
  </div>
`;
  }

  connectedCallback() {
    super.connectedCallback();
    this.#input.connect(this.shadowRoot.getElementById('well'));
    // Focusable, named group (issue #1176). The drag well was a bare <div>: a
    // pointer control the keyboard could not land on, name, or reach. The host
    // becomes the one Tab stop — `role="group"` is the honest role for a
    // composite whose continuous 2-axis state a screen reader cannot enumerate
    // (name/role hygiene, not narration, exactly like the tactical radar) — and
    // its name rides the string catalogue. The focus ring comes from the
    // document-adopted control family (console-core adopts it into `document`),
    // so there is no per-component outline.
    this.setAttribute('role', 'group');
    this.setAttribute('aria-label', t('component.helm_joystick.label'));
    if (!this.hasAttribute('tabindex')) this.setAttribute('tabindex', '0');
    // Keyboard (WASD / arrows) drives the same semantic axis identities as the on-screen
    // thumbstick. Gamepad axes are owned by the parent semantic input runtime,
    // so this component never polls or chooses a device. This is the
    // DELIBERATE key-relay coexistence
    // (issue #1176): the arrow/WASD flight bindings stay a SINGLE document-level
    // handler — the same one gui/key-relay.js relays into the console — so a
    // focused well adds a Tab stop and a name but NOT a second arrow handler,
    // and one arrow press drives set_helm exactly once whether the event
    // arrives natively or relayed (the key state is a set keyed by code, so a
    // native + relayed pair cannot double-count). Ported from the legacy
    // helm-console.html so the new per-ship helm consoles keep desktop control.
  }

  disconnectedCallback() {
    this.#input.disconnect();
  }

  render(state) {
    const auto = state ? !!state.auto : false;
    const root = this.shadowRoot;
    const badge = root.getElementById('auto-badge');
    const well = root.getElementById('well');
    badge.style.display = auto ? 'inline' : 'none';
    well.classList.toggle('auto', auto);
    if (auto && !this.#input.hasPointer()) {
      this.#px = 0;
      this.#py = 0;
      this.#applyNubPosition();
      this.#updateReadout();
    }
  }

  #setFromPointer(clientX, clientY) {
    const well = this.shadowRoot.getElementById('well');
    const r = well.getBoundingClientRect();
    const cx = r.left + r.width / 2;
    const cy = r.top + r.height / 2;
    const radius = Math.min(r.width, r.height) / 2 - 28;
    let dx = (clientX - cx) / radius;
    let dy = (clientY - cy) / radius;
    const d = Math.hypot(dx, dy);
    if (d > 1) { dx /= d; dy /= d; }
    this.#px = dx;
    this.#py = dy;
  }

  #applyNubPosition() {
    const well = this.shadowRoot.getElementById('well');
    const r = well.getBoundingClientRect();
    const radius = Math.min(r.width, r.height) / 2 - 28;
    const nub = this.shadowRoot.getElementById('nub');
    nub.style.marginLeft = (this.#px * radius) + 'px';
    nub.style.marginTop = (this.#py * radius) + 'px';
  }

  #updateReadout() {
    const root = this.shadowRoot;
    const fmt = (v) => (v >= 0 ? '+' : '') + v.toFixed(2);
    root.getElementById('thrust-readout').textContent = fmt(-this.#py);
    root.getElementById('yaw-readout').textContent = fmt(this.#px);
  }

  // ── Keyboard + gamepad input ──────────────────────────────────────────
  #sampleKeys(keys) {
    let nx = 0, ny = 0;
    if (keys['ArrowLeft'] || keys['KeyA']) nx -= 1;
    if (keys['ArrowRight'] || keys['KeyD']) nx += 1;
    if (keys['ArrowUp'] || keys['KeyW']) ny -= 1;
    if (keys['ArrowDown'] || keys['KeyS']) ny += 1;
    const d = Math.hypot(nx, ny);
    if (d > 1) { nx /= d; ny /= d; }
    if (nx === 0 && ny === 0) return false;
    this.#px = nx;
    this.#py = ny;
    return true;
  }

  #sendAction() {
    const activate = typeof window !== 'undefined' && window.activateSemanticAction;
    if (typeof activate !== 'function') return;
    activate(HELM_THRUST_ACTION_ID, {
      context: HELM_ACTION_CONTEXT, source: 'control', value: -this.#py || 0,
    });
    activate(HELM_STEERING_ACTION_ID, {
      context: HELM_ACTION_CONTEXT, source: 'control', value: this.#px || 0,
    });
  }
}

phDefine('ph-helm-joystick', PhHelmJoystick);
