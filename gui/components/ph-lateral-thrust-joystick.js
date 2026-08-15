// strings-boot first: its top-level await delays this module's evaluation —
// and therefore this element's registration and upgrade — until the string
// table is loaded, so the constructor's template t() calls never see an
// empty table. No-op in Node tests (setup-strings.js loads the table there).
import '../strings-boot.js';
import { t } from '../strings.js';
import { phAdoptConsoleStyles } from './ph-console-styles.js';

export class PhLateralThrustJoystick extends HTMLElement {
  #state = null;
  #value = 0;
  #pointerId = null;
  #rafId = null;
  #hbRaf = null;
  #lastHbSend = 0;
  #keys = {};
  #inputRaf = null;
  #lastKbSend = 0;

  constructor() {
    super();
    this.attachShadow({ mode: 'open' });
    // Every component adopts the shared control family (module 1 of PRD
    // #1023): custom properties cross a shadow boundary, class rules do not.
    phAdoptConsoleStyles(this.shadowRoot);
    const tpl = document.createElement('template');
    tpl.innerHTML = `
  <style>
    :host { display: flex; flex-direction: column; align-items: center; font-family: 'JetBrains Mono', monospace; color: var(--ink); }
    :host * { box-sizing: border-box; }
    .header { display: flex; justify-content: space-between; align-items: center; width: 100%; font-size: var(--text-sm); letter-spacing: 0.2em; color: var(--ink-dim); text-transform: uppercase; margin-bottom: 0.3rem; }
    .auto-badge { font-size: var(--text-xs); color: var(--reloading); border: 1px solid var(--reloading); padding: 0.1rem 0.4rem; letter-spacing: 0.2em; }
    .track {
      position: relative; width: 180px; height: 32px; border-radius: 16px;
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
    this.shadowRoot.appendChild(tpl.content.cloneNode(true));
  }

  connectedCallback() {
    if (!this.sendAction) {
      if (typeof window !== 'undefined' && typeof window.sendAction === 'function') {
        this.sendAction = window.sendAction;
      }
    }
    this.#bindEvents();
    if (typeof document !== 'undefined') {
      document.addEventListener('keydown', this.#onKeyDown);
      document.addEventListener('keyup', this.#onKeyUp);
    }
    if (typeof window !== 'undefined') {
      window.addEventListener('blur', this.#onBlur);
      window.addEventListener('gamepadconnected', this.#onGamepadConnected);
    }
  }

  disconnectedCallback() {
    if (typeof document !== 'undefined') {
      document.removeEventListener('keydown', this.#onKeyDown);
      document.removeEventListener('keyup', this.#onKeyUp);
    }
    if (typeof window !== 'undefined') {
      window.removeEventListener('blur', this.#onBlur);
      window.removeEventListener('gamepadconnected', this.#onGamepadConnected);
    }
    if (this.#inputRaf) { cancelAnimationFrame(this.#inputRaf); this.#inputRaf = null; }
    if (this.#hbRaf) { cancelAnimationFrame(this.#hbRaf); this.#hbRaf = null; }
    if (this.#rafId) { cancelAnimationFrame(this.#rafId); this.#rafId = null; }
  }

  set state(val) {
    this.#state = val;
    this.#render();
  }

  get state() { return this.#state; }

  #render() {
    const auto = this.#state ? !!this.#state.auto : false;
    const root = this.shadowRoot;
    const badge = root.getElementById('auto-badge');
    const track = root.getElementById('track');
    badge.style.display = auto ? 'inline' : 'none';
    track.classList.toggle('auto', auto);
    if (auto && this.#pointerId === null) {
      this.#value = 0;
      this.#applyNubPosition();
      this.#updateReadout();
    }
  }

  #bindEvents() {
    const track = this.shadowRoot.getElementById('track');
    track.addEventListener('pointerdown', this.#onDown);
    track.addEventListener('pointermove', this.#onMove);
    track.addEventListener('pointerup', this.#onUp);
    track.addEventListener('pointercancel', this.#onUp);
    track.addEventListener('lostpointercapture', this.#onUp);
  }

  #onDown = (e) => {
    const auto = this.#state ? !!this.#state.auto : false;
    if (auto) return;
    if (this.#pointerId !== null) return;
    this.#pointerId = e.pointerId;
    const track = this.shadowRoot.getElementById('track');
    if (track.setPointerCapture) track.setPointerCapture(e.pointerId);
    if (this.#rafId) { cancelAnimationFrame(this.#rafId); this.#rafId = null; }
    if (!this.#hbRaf) {
      this.#hbRaf = requestAnimationFrame(this.#heartbeatLoop);
    }
    this.#setFromPointer(e.clientX);
    this.#sendAction();
    e.preventDefault();
  };

  #heartbeatLoop = () => {
    this.#hbRaf = null;
    if (this.#pointerId === null) return;
    const now = performance.now();
    if (now - this.#lastHbSend >= 100) {
      this.#sendAction();
      this.#lastHbSend = now;
    }
    this.#hbRaf = requestAnimationFrame(this.#heartbeatLoop);
  };

  #onMove = (e) => {
    if (e.pointerId !== this.#pointerId) return;
    this.#setFromPointer(e.clientX);
    this.#sendAction();
  };

  #onUp = (e) => {
    if (e.pointerId !== this.#pointerId) return;
    this.#pointerId = null;
    const track = this.shadowRoot.getElementById('track');
    try { if (track.releasePointerCapture) track.releasePointerCapture(e.pointerId); } catch (_) { }
    if (this.#hbRaf) { cancelAnimationFrame(this.#hbRaf); this.#hbRaf = null; }
    if (this.#rafId) { cancelAnimationFrame(this.#rafId); this.#rafId = null; }
    this.#value = 0;
    this.#applyNubPosition();
    this.#updateReadout();
    this.#sendAction();
  };

  #setFromPointer(clientX) {
    const track = this.shadowRoot.getElementById('track');
    const r = track.getBoundingClientRect();
    const cx = r.left + r.width / 2;
    const half = r.width / 2 - 16;
    let dx = (clientX - cx) / half;
    if (dx > 1) dx = 1;
    if (dx < -1) dx = -1;
    this.#value = dx;
    this.#scheduleApply();
  }

  #scheduleApply() {
    if (this.#rafId) return;
    this.#rafId = requestAnimationFrame(() => {
      this.#rafId = null;
      this.#applyNubPosition();
      this.#updateReadout();
    });
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

  #onKeyDown = (e) => {
    const tag = e.target && e.target.tagName;
    if (tag === 'INPUT' || tag === 'TEXTAREA') return;
    const relevant = ['KeyQ', 'KeyE', 'ArrowLeft', 'ArrowRight'];
    if (relevant.indexOf(e.code) === -1) return;
    e.preventDefault();
    this.#keys[e.code] = true;
    this.#startInputLoop();
  };

  #onKeyUp = (e) => {
    delete this.#keys[e.code];
    this.#startInputLoop();
  };

  #onBlur = () => {
    this.#keys = {};
    this.#startInputLoop();
  };

  #onGamepadConnected = () => {
    this.#startInputLoop();
  };

  #getGamepadInput() {
    if (typeof navigator === 'undefined' || !navigator.getGamepads) return 0;
    const pads = navigator.getGamepads();
    for (let i = 0; i < pads.length; i++) {
      const gp = pads[i];
      if (!gp || !gp.buttons) continue;
      // LB = buttons[4], RB = buttons[5]
      let v = 0;
      if (gp.buttons[4] && gp.buttons[4].pressed) v -= 1;
      if (gp.buttons[5] && gp.buttons[5].pressed) v += 1;
      return v;
    }
    return 0;
  }

  #hasGamepad() {
    if (typeof navigator === 'undefined' || !navigator.getGamepads) return false;
    const pads = navigator.getGamepads();
    for (let i = 0; i < pads.length; i++) if (pads[i]) return true;
    return false;
  }

  #startInputLoop() {
    if (this.#inputRaf) return;
    this.#inputRaf = requestAnimationFrame(this.#inputLoop);
  }

  #inputLoop = () => {
    this.#inputRaf = null;
    const auto = this.#state ? !!this.#state.auto : false;
    const keepPolling = Object.keys(this.#keys).length > 0 || this.#hasGamepad();

    if (auto || this.#pointerId !== null) {
      if (!auto && keepPolling) this.#inputRaf = requestAnimationFrame(this.#inputLoop);
      return;
    }

    let kv = 0;
    if (this.#keys['KeyQ'] || this.#keys['ArrowLeft']) kv -= 1;
    if (this.#keys['KeyE'] || this.#keys['ArrowRight']) kv += 1;

    const gp = this.#getGamepadInput();
    let nv = kv !== 0 ? kv : gp;
    if (nv > 1) nv = 1;
    if (nv < -1) nv = -1;

    const hasInput = nv !== 0;
    if (hasInput) {
      this.#value = nv;
      this.#applyNubPosition();
      this.#updateReadout();
      const now = Date.now();
      if (now - this.#lastKbSend >= 100) { this.#sendAction(); this.#lastKbSend = now; }
      this.#inputRaf = requestAnimationFrame(this.#inputLoop);
    } else {
      if (this.#value !== 0) {
        this.#value = 0;
        this.#applyNubPosition();
        this.#updateReadout();
        this.#sendAction();
      }
      if (keepPolling) this.#inputRaf = requestAnimationFrame(this.#inputLoop);
    }
  };

  #sendAction() {
    if (this.sendAction) {
      this.sendAction('set_lateral_thrust', { lateral: this.#value || 0 });
    }
  }
}

if (typeof window !== 'undefined' && !customElements.get('ph-lateral-thrust-joystick')) {
  customElements.define('ph-lateral-thrust-joystick', PhLateralThrustJoystick);
}
