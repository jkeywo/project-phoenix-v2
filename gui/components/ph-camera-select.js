export class PhCameraSelect extends HTMLElement {
  #state = null;
  #btnCache = new Map();
  #emptyEl = null;

  constructor() {
    super();
    this.attachShadow({ mode: 'open' });
    const t = document.createElement('template');
    t.innerHTML = `
  <style>
    :host { display: flex; flex-direction: column; gap: 0.5rem; font-family: 'JetBrains Mono', monospace; color: #cce; }
    :host * { box-sizing: border-box; }
    .header { display: flex; justify-content: space-between; align-items: center; font-size: 0.75rem; letter-spacing: 0.2em; color: #6a7178; text-transform: uppercase; }
    .auto-badge { font-size: 0.6rem; color: #f0c040; border: 1px solid #f0c040; padding: 0.1rem 0.4rem; letter-spacing: 0.2em; }
    #container { display: grid; grid-template-columns: repeat(3, 1fr); grid-auto-rows: 1fr; gap: 0.35rem; }
    .cam-btn { background: #0e1117; border: 1px solid #282c38; color: #6a7178; font-family: 'Chakra Petch', sans-serif; font-size: 0.7rem; font-weight: 600; padding: 0.5rem 0; letter-spacing: 0.15em; text-transform: uppercase; cursor: pointer; transition: all 0.15s ease; }
    .cam-btn:hover:not(:disabled) { background: #161b24; color: #aab; }
    .cam-btn.active { background: #1a2a3a; border-color: #5fd8e8; color: #5fd8e8; }
    .cam-btn.active:hover { background: #1e2f42; }
    .cam-btn:disabled { opacity: 0.4; cursor: default; }
    .placeholder { font-size: 0.7rem; color: #6a7178; letter-spacing: 0.2em; padding: 0.5rem 0; text-align: center; }
  </style>
  <div class="header">
    <span>CAMERA</span>
    <span class="auto-badge" id="auto-badge" style="display:none">AUTO</span>
  </div>
  <div id="container"></div>
`;
    this.shadowRoot.appendChild(t.content.cloneNode(true));
  }

  connectedCallback() {
    this.sendAction ??= window.sendAction;
  }

  set state(val) {
    this.#state = val;
    this.#render();
  }

  get state() { return this.#state; }

  #render() {
    const s = this.#state || {};
    const root = this.shadowRoot;

    const views = s.views || [];
    const currentView = s.current_view || '';
    const auto = !!s.auto;

    root.getElementById('auto-badge').style.display = auto ? 'inline' : 'none';

    const container = root.getElementById('container');

    const live = new Set(views);
    for (const [key, btn] of this.#btnCache) {
      if (!live.has(key)) { btn.remove(); this.#btnCache.delete(key); }
    }

    if (views.length === 0) {
      if (!this.#emptyEl) { this.#emptyEl = document.createElement('div'); this.#emptyEl.className = 'placeholder'; this.#emptyEl.textContent = 'NO CAMERA'; container.appendChild(this.#emptyEl); }
      return;
    }
    if (this.#emptyEl) { this.#emptyEl.remove(); this.#emptyEl = null; }

    // Arrange the four principal views in a cross (FWD top, PORT left, STBD
    // right, AFT bottom); any additional named cameras flow into free cells.
    let flowCell = 0;
    const freeCells = ['1 / 1', '1 / 3', '3 / 1', '3 / 3', '2 / 2'];
    views.forEach(v => {
      let btn = this.#btnCache.get(v);
      if (!btn) {
        btn = document.createElement('button');
        btn.className = 'cam-btn';
        btn.dataset.view = v;
        btn.addEventListener('click', () => {
          if (!btn.disabled && this.sendAction) {
            this.sendAction('set_view', { direction: btn.dataset.view });
          }
        });
        this.#btnCache.set(v, btn);
        container.appendChild(btn);
      }
      btn.className = 'cam-btn' + (v === currentView ? ' active' : '');
      btn.disabled = auto;
      btn.textContent = v;

      const n = String(v).toLowerCase();
      let area = null;
      if (/(^|[^a-z])(fore|forward|fwd|front|bow)([^a-z]|$)/.test(n)) area = '1 / 2';
      else if (/(^|[^a-z])(aft|rear|reverse|back|stern)([^a-z]|$)/.test(n)) area = '3 / 2';
      else if (/(^|[^a-z])(port|left)([^a-z]|$)/.test(n)) area = '2 / 1';
      else if (/(^|[^a-z])(starboard|stbd|right)([^a-z]|$)/.test(n)) area = '2 / 3';
      else { area = freeCells[flowCell % freeCells.length]; flowCell++; }
      btn.style.gridArea = area;
    });
  }
}

if (typeof window !== 'undefined' && !customElements.get('ph-camera-select')) {
  customElements.define('ph-camera-select', PhCameraSelect);
}
