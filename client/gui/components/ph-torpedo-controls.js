export class PhTorpedoControls extends HTMLElement {
  #state = null;

  constructor() {
    super();
    this.attachShadow({ mode: 'open' });
    const t = document.createElement('template');
    t.innerHTML = `
  <style>
    :host { display: flex; flex-direction: column; gap: 0.5rem; font-family: 'JetBrains Mono', monospace; color: #cce; }
    :host * { box-sizing: border-box; }
    .header { display: flex; justify-content: space-between; align-items: center; font-size: 0.75rem; letter-spacing: 0.2em; color: #6a7178; text-transform: uppercase; }
    .magazine { font-size: 0.9rem; color: #f08438; font-weight: 600; font-family: 'Chakra Petch', sans-serif; }
    .tube-row { display: flex; align-items: center; gap: 0.4rem; font-size: 0.65rem; padding: 0.3rem 0; }
    .tube-row .lbl { min-width: 4rem; color: #6a7178; }
    .load-progress-wrap { flex: 1; height: 0.5rem; background: #05080e; border: 1px solid #282c38; overflow: hidden; }
    .load-progress-fill { height: 100%; background: linear-gradient(90deg, #805818, #f0c040); transition: width 0.3s ease; }
    .tube-btn { font-family: 'Chakra Petch', sans-serif; font-size: 0.55rem; font-weight: 700; padding: 0.2rem 0.5rem; letter-spacing: 0.1em; text-transform: uppercase; cursor: pointer; border: 2px solid #282c38; color: #cce; background: #0e1117; transition: all 0.15s ease; }
    .tube-btn:hover:not(:disabled) { background: #161b24; border-color: #4a5060; }
    .tube-btn:disabled { opacity: 0.35; cursor: default; }
    .tube-btn.fire { border-color: #4ec870; color: #4ec870; }
    .tube-btn.fire:hover:not(:disabled) { background: #16281d; }
    .tube-btn.load { border-color: #f0c040; color: #f0c040; }
    .tube-btn.load:hover:not(:disabled) { background: #1a1a0a; }
    .tube-btn.unload { border-color: #e05555; color: #e05555; }
    .tube-btn.unload:hover:not(:disabled) { background: #1a0a0a; }
    .auto-badge { font-size: 0.55rem; color: #f0c040; border: 1px solid #f0c040; padding: 0.05rem 0.3rem; letter-spacing: 0.2em; margin-left: 0.3rem; }
    .empty { font-size: 0.65rem; color: #6a7178; text-align: center; padding: 0.75rem 0; letter-spacing: 0.2em; }
  </style>
  <div class="header">
    <span>TORPEDOES</span>
    <span class="magazine" id="magazine">0 / 0</span>
  </div>
  <div id="tubes"></div>
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
    const tubes = Array.isArray(s.tubes) ? s.tubes : [];
    const mag = s.magazine || {};
    const magCurrent = mag.current != null ? mag.current : 0;
    const magMax = mag.max != null ? mag.max : 0;

    this.shadowRoot.getElementById('magazine').textContent = magCurrent + ' / ' + magMax;

    const container = this.shadowRoot.getElementById('tubes');

    if (tubes.length === 0) {
      container.innerHTML = '<div class="empty">NO TORPEDO TUBES</div>';
      return;
    }

    const newIds = new Set(tubes.map(t => t.id));
    Array.from(container.children).forEach(child => {
      if (!newIds.has(child.dataset.id)) {
        child.remove();
      }
    });

    tubes.forEach((tube, idx) => {
      let row = container.querySelector(`[data-id="${tube.id}"]`);
      if (!row) {
        row = document.createElement('div');
        row.className = 'tube-row';
        row.dataset.id = tube.id;

        const lbl = document.createElement('span');
        lbl.className = 'lbl';
        row.appendChild(lbl);

        const progressWrap = document.createElement('div');
        progressWrap.className = 'load-progress-wrap';
        const progressFill = document.createElement('div');
        progressFill.className = 'load-progress-fill';
        progressWrap.appendChild(progressFill);
        row.appendChild(progressWrap);

        const badge = document.createElement('span');
        badge.className = 'auto-badge';
        badge.textContent = 'AUTO';
        row.appendChild(badge);

        const loadBtn = document.createElement('button');
        loadBtn.className = 'tube-btn load';
        loadBtn.textContent = 'LOAD';
        loadBtn.addEventListener('click', () => {
          if (this.sendAction && !loadBtn.disabled) {
            this.sendAction('load_tube', { tube: tube.id });
          }
        });
        row.appendChild(loadBtn);

        const unloadBtn = document.createElement('button');
        unloadBtn.className = 'tube-btn unload';
        unloadBtn.textContent = 'UNLOAD';
        unloadBtn.addEventListener('click', () => {
          if (this.sendAction && !unloadBtn.disabled) {
            this.sendAction('unload_tube', { tube: tube.id });
          }
        });
        row.appendChild(unloadBtn);

        const fireBtn = document.createElement('button');
        fireBtn.className = 'tube-btn fire';
        fireBtn.textContent = 'FIRE';
        fireBtn.addEventListener('click', () => {
          if (this.sendAction && !fireBtn.disabled) {
            this.sendAction('fire_torpedo', { tube: tube.id });
          }
        });
        row.appendChild(fireBtn);

        if (idx < container.children.length) {
          container.insertBefore(row, container.children[idx]);
        } else {
          container.appendChild(row);
        }
      }

      row.querySelector('.lbl').textContent = tube.label || tube.id;

      const isLoaded = !!tube.is_loaded;
      const isBusy = tube.state === 'loading' || tube.state === 'unloading';
      const auto = !!tube.auto;
      const progressPct = Math.max(0, Math.min(100, (tube.load_progress_pct || 0) * 100));

      const progressWrap = row.querySelector('.load-progress-wrap');
      const progressFill = row.querySelector('.load-progress-fill');
      if (isBusy) {
        progressWrap.style.display = 'block';
        progressFill.style.width = progressPct + '%';
      } else {
        progressWrap.style.display = 'none';
      }

      const badge = row.querySelector('.auto-badge');
      badge.style.display = auto ? 'inline' : 'none';

      const loadBtn = row.querySelector('.load');
      loadBtn.disabled = auto || isLoaded || magCurrent <= 0;

      const unloadBtn = row.querySelector('.unload');
      unloadBtn.disabled = auto || !isLoaded;

      const fireBtn = row.querySelector('.fire');
      fireBtn.disabled = auto || !isLoaded;
    });
  }
}

if (typeof window !== 'undefined' && !customElements.get('ph-torpedo-controls')) {
  customElements.define('ph-torpedo-controls', PhTorpedoControls);
}
