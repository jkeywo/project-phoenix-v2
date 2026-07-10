export class PhHelmJoystick extends HTMLElement {
  #state = null;
  #px = 0;
  #py = 0;
  #pointerId = null;
  #rafId = null;
  #hbId = null;
  #keys = {};
  #inputRaf = null;
  #lastKbSend = 0;

  constructor() {
    super();
    this.attachShadow({ mode: 'open' });
    const t = document.createElement('template');
    t.innerHTML = `
  <style>
    :host { display: flex; flex-direction: column; align-items: center; font-family: 'JetBrains Mono', monospace; color: #cce; }
    :host * { box-sizing: border-box; }
    .header { display: flex; justify-content: space-between; align-items: center; width: 100%; font-size: 0.75rem; letter-spacing: 0.2em; color: #6a7178; text-transform: uppercase; margin-bottom: 0.5rem; }
    .auto-badge { font-size: 0.6rem; color: #f0c040; border: 1px solid #f0c040; padding: 0.1rem 0.4rem; letter-spacing: 0.2em; }
    .well {
      position: relative; width: 240px; height: 240px; border-radius: 50%;
      background: radial-gradient(circle at center, #0a0d11 0%, #14171c 75%, #050608 100%);
      border: 1px solid #282c38; box-shadow: inset 0 0 0 4px #0a0d11, inset 0 0 0 5px rgba(108,182,208,0.35);
      cursor: grab; touch-action: none; flex-shrink: 0;
    }
    .well:active { cursor: grabbing; }
    .well.auto { cursor: default; }
    .ring { position: absolute; border-radius: 50%; border: 1px solid rgba(108,182,208,0.18); pointer-events: none; }
    .ring.outer { inset: 16px; }
    .ring.mid { inset: 52px; }
    .cross-h, .cross-v { position: absolute; background: rgba(108,182,208,0.14); pointer-events: none; }
    .cross-h { left: 18px; right: 18px; top: 50%; height: 1px; }
    .cross-v { top: 18px; bottom: 18px; left: 50%; width: 1px; }
    .arrow { position: absolute; width: 0; height: 0; pointer-events: none; opacity: 0.5; }
    .arrow.fwd { top: 8px; left: 50%; transform: translateX(-50%); border-left: 6px solid transparent; border-right: 6px solid transparent; border-bottom: 8px solid rgba(108,182,208,0.5); }
    .arrow.rev { bottom: 8px; left: 50%; transform: translateX(-50%); border-left: 6px solid transparent; border-right: 6px solid transparent; border-top: 8px solid rgba(108,182,208,0.5); }
    .arrow.port { left: 8px; top: 50%; transform: translateY(-50%); border-top: 6px solid transparent; border-bottom: 6px solid transparent; border-right: 8px solid rgba(108,182,208,0.5); }
    .arrow.stbd { right: 8px; top: 50%; transform: translateY(-50%); border-top: 6px solid transparent; border-bottom: 6px solid transparent; border-left: 8px solid rgba(108,182,208,0.5); }
    .ax-label { position: absolute; font-family: 'Chakra Petch', sans-serif; font-weight: 600; font-size: 10px; color: #6a7178; letter-spacing: 0.22em; pointer-events: none; }
    .ax-label.fwd { top: 20px; left: 50%; transform: translateX(-50%); }
    .ax-label.rev { bottom: 20px; left: 50%; transform: translateX(-50%); }
    .ax-label.port { left: 18px; top: 50%; transform: translateY(-50%) rotate(-90deg); }
    .ax-label.stbd { right: 18px; top: 50%; transform: translateY(-50%) rotate(90deg); }
    .nub {
      position: absolute; left: 50%; top: 50%; width: 56px; height: 56px; border-radius: 50%;
      background: radial-gradient(circle at 35% 30%, #3a4049 0%, #22262d 50%, #14171c 100%);
      border: 1.5px solid rgba(108,182,208,0.8); box-shadow: 0 0 20px rgba(108,182,208,0.35);
      transform: translate(-50%, -50%); pointer-events: none; transition: none;
      will-change: margin-left, margin-top;
    }
    .nub::after { content: ''; position: absolute; inset: 18px; border-radius: 50%; background: #14171c; border: 1px solid rgba(108,182,208,0.3); }
    .readout { display: flex; gap: 1rem; margin-top: 0.5rem; font-size: 0.8rem; color: rgba(108,182,208,0.8); letter-spacing: 0.1em; }
    .readout .sep { color: #6a7178; }
  </style>
  <div class="header">
    <span>HELM</span>
    <span class="auto-badge" id="auto-badge" style="display:none">AUTO</span>
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
    <div class="ax-label fwd">FWD</div>
    <div class="ax-label rev">REV</div>
    <div class="ax-label port">PORT</div>
    <div class="ax-label stbd">STBD</div>
    <div class="nub" id="nub"></div>
  </div>
  <div class="readout">
    <span id="thrust-readout">+0.00</span>
    <span class="sep">/</span>
    <span id="yaw-readout">+0.00</span>
  </div>
`;
    this.shadowRoot.appendChild(t.content.cloneNode(true));
  }

  connectedCallback() {
    if (!this.sendAction) {
      if (typeof window !== 'undefined' && typeof window.sendAction === 'function') {
        this.sendAction = window.sendAction;
      }
    }
    this.#bindEvents();
    // Keyboard (WASD / arrows) + gamepad drive the same set_helm output as the
    // on-screen thumbstick. Ported from the legacy helm-console.html so the new
    // per-ship helm consoles keep desktop/controller control (issue: new GUI).
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
    if (this.#hbId) { clearInterval(this.#hbId); this.#hbId = null; }
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
    const well = root.getElementById('well');
    badge.style.display = auto ? 'inline' : 'none';
    well.classList.toggle('auto', auto);
    if (auto && this.#pointerId === null) {
      this.#px = 0;
      this.#py = 0;
      this.#applyNubPosition();
      this.#updateReadout();
    }
  }

  #bindEvents() {
    const well = this.shadowRoot.getElementById('well');
    well.addEventListener('pointerdown', this.#onDown);
    well.addEventListener('pointermove', this.#onMove);
    well.addEventListener('pointerup', this.#onUp);
    well.addEventListener('pointercancel', this.#onUp);
  }

  #onDown = (e) => {
    const auto = this.#state ? !!this.#state.auto : false;
    if (auto) return;
    if (this.#pointerId !== null) return;
    this.#pointerId = e.pointerId;
    const well = this.shadowRoot.getElementById('well');
    if (well.setPointerCapture) well.setPointerCapture(e.pointerId);
    if (this.#rafId) { cancelAnimationFrame(this.#rafId); this.#rafId = null; }
    if (!this.#hbId) {
      this.#hbId = setInterval(() => this.#sendAction(), 100);
    }
    this.#setFromPointer(e.clientX, e.clientY);
    e.preventDefault();
  };

  #onMove = (e) => {
    if (e.pointerId !== this.#pointerId) return;
    this.#setFromPointer(e.clientX, e.clientY);
  };

  #onUp = (e) => {
    if (e.pointerId !== this.#pointerId) return;
    this.#pointerId = null;
    const well = this.shadowRoot.getElementById('well');
    try { if (well.releasePointerCapture) well.releasePointerCapture(e.pointerId); } catch (_) { /* noop */ }
    if (this.#hbId) { clearInterval(this.#hbId); this.#hbId = null; }
    this.#px = 0;
    this.#py = 0;
    this.#scheduleApply();
    this.#sendAction();
  };

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
  #onKeyDown = (e) => {
    const tag = e.target && e.target.tagName;
    if (tag === 'INPUT' || tag === 'TEXTAREA') return;
    const relevant = ['ArrowLeft', 'ArrowRight', 'ArrowUp', 'ArrowDown', 'KeyA', 'KeyD', 'KeyW', 'KeyS'];
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
    if (typeof navigator === 'undefined' || !navigator.getGamepads) return { x: 0, y: 0 };
    const pads = navigator.getGamepads();
    for (let i = 0; i < pads.length; i++) {
      const gp = pads[i];
      if (!gp || gp.axes.length < 2) continue;
      const dead = 0.1;
      return {
        x: Math.abs(gp.axes[0]) > dead ? gp.axes[0] : 0,
        y: Math.abs(gp.axes[1]) > dead ? gp.axes[1] : 0,
      };
    }
    return { x: 0, y: 0 };
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

    // The on-screen thumbstick (pointer drag) and AUTO both take priority.
    if (auto || this.#pointerId !== null) {
      if (!auto && keepPolling) this.#inputRaf = requestAnimationFrame(this.#inputLoop);
      return;
    }

    let kx = 0, ky = 0;
    if (this.#keys['ArrowLeft'] || this.#keys['KeyA']) kx -= 1;
    if (this.#keys['ArrowRight'] || this.#keys['KeyD']) kx += 1;
    if (this.#keys['ArrowUp'] || this.#keys['KeyW']) ky -= 1; // forward thrust
    if (this.#keys['ArrowDown'] || this.#keys['KeyS']) ky += 1;

    const gp = this.#getGamepadInput();
    let nx = kx !== 0 ? kx : gp.x;
    let ny = ky !== 0 ? ky : gp.y;
    const d = Math.hypot(nx, ny);
    if (d > 1) { nx /= d; ny /= d; }

    const hasInput = nx !== 0 || ny !== 0;
    if (hasInput) {
      this.#px = nx;
      this.#py = ny;
      this.#applyNubPosition();
      this.#updateReadout();
      const now = Date.now();
      if (now - this.#lastKbSend >= 100) { this.#sendAction(); this.#lastKbSend = now; }
      this.#inputRaf = requestAnimationFrame(this.#inputLoop);
    } else {
      // Input released this frame — snap back to centre and send a stop once.
      if (this.#px !== 0 || this.#py !== 0) {
        this.#px = 0;
        this.#py = 0;
        this.#applyNubPosition();
        this.#updateReadout();
        this.#sendAction();
      }
      // Keep polling while a gamepad is present (its stick can re-engage
      // without a keydown to restart the loop).
      if (keepPolling) this.#inputRaf = requestAnimationFrame(this.#inputLoop);
    }
  };

  #sendAction() {
    if (this.sendAction) {
      this.sendAction('set_helm', { thrust: -this.#py || 0, yaw: this.#px || 0 });
    }
  }
}

if (typeof window !== 'undefined' && !customElements.get('ph-helm-joystick')) {
  customElements.define('ph-helm-joystick', PhHelmJoystick);
}
