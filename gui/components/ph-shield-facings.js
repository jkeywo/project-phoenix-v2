export class PhShieldFacings extends HTMLElement {
  #state = null;
  #facingGs = new Map();
  #emptyEl = null;
  #svgEl = null;
  #arcsGroup = null;

  constructor() {
    super();
    this.attachShadow({ mode: 'open' });
    const t = document.createElement('template');
    t.innerHTML = `
  <style>
    :host { display: flex; flex-direction: column; gap: 0.5rem; font-family: 'JetBrains Mono', monospace; color: var(--ink); }
    :host * { box-sizing: border-box; }
    .header { display: flex; justify-content: space-between; align-items: center; font-size: 0.75rem; letter-spacing: 0.2em; color: var(--ink-dim); text-transform: uppercase; }
    .auto-badge { font-size: 0.55rem; color: var(--reloading); border: 1px solid var(--reloading); padding: 0.05rem 0.3rem; letter-spacing: 0.2em; }
    .arc-container { display: flex; justify-content: center; align-items: center; padding: 0.5rem 0; }
    svg { width: 100%; max-width: 200px; height: auto; overflow: visible; }
    .arc-path { cursor: pointer; transition: opacity 0.2s, filter 0.2s; }
    .arc-path:hover { filter: brightness(1.3); }
    .arc-path.focused { filter: brightness(1.5) drop-shadow(0 0 4px var(--loaded)); }
    .arc-path.down { opacity: 0.3; cursor: default; }
    .facing-label { font-size: 0.55rem; fill: var(--ink-dim); text-anchor: middle; pointer-events: none; }
    .facing-label.focused-label { fill: var(--ink); font-weight: 600; }
    .empty { font-size: 0.65rem; color: var(--ink-dim); text-align: center; padding: 0.75rem 0; letter-spacing: 0.2em; }
  </style>
  <div class="header">
    <span>SHIELD FACINGS</span>
    <span class="auto-badge" id="auto-badge" style="display:none">AUTO</span>
  </div>
  <div class="arc-container" id="arc-container">
    <div class="empty" id="empty-placeholder">NO FACING DATA</div>
    <svg viewBox="0 0 200 200" xmlns="http://www.w3.org/2000/svg" id="facing-svg" style="display:none"><g id="facing-arcs"></g></svg>
  </div>
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
    const facings = Array.isArray(s.facings) ? s.facings : [];
    const focused = s.focused_facing || null;
    const auto = !!s.auto;
    const badge = this.shadowRoot.getElementById('auto-badge');
    badge.style.display = auto ? 'inline' : 'none';

    if (!this.#emptyEl) this.#emptyEl = this.shadowRoot.getElementById('empty-placeholder');
    if (!this.#svgEl) this.#svgEl = this.shadowRoot.getElementById('facing-svg');
    if (!this.#arcsGroup) this.#arcsGroup = this.shadowRoot.getElementById('facing-arcs');

    if (facings.length === 0) {
      this.#emptyEl.style.display = '';
      this.#svgEl.style.display = 'none';
      return;
    }
    this.#emptyEl.style.display = 'none';
    this.#svgEl.style.display = '';

    const n = facings.length;
    const cx = 100, cy = 100, r = 70, ir = 35;
    const angleStep = (Math.PI * 2) / n;
    const startAngle = -Math.PI / 2 - angleStep / 2;

    const NS = 'http://www.w3.org/2000/svg';
    const live = new Set(facings.map(f => f.arc_id));
    for (const [key, g] of this.#facingGs) {
      if (!live.has(key)) { g.remove(); this.#facingGs.delete(key); }
    }

    facings.forEach((f, i) => {
      const id = f.arc_id;
      const a0 = startAngle + i * angleStep;
      const a1 = a0 + angleStep;
      const pct = f.max_hp > 0 ? Math.min(1, Math.max(0, f.hp / f.max_hp)) : 0;
      const online = f.online !== false;
      const isFocused = focused === f.id || focused === f.label;

      const x0 = cx + r * Math.cos(a0), y0 = cy + r * Math.sin(a0);
      const x1 = cx + r * Math.cos(a1), y1 = cy + r * Math.sin(a1);
      const xi0 = cx + ir * Math.cos(a0), yi0 = cy + ir * Math.sin(a0);
      const xi1 = cx + ir * Math.cos(a1), yi1 = cy + ir * Math.sin(a1);

      const largeArc = angleStep > Math.PI ? 1 : 0;

      const midAngle = a0 + angleStep / 2;

      let g = this.#facingGs.get(id);
      if (!g) {
        g = document.createElementNS(NS, 'g');
        g.innerHTML = '<path class="arc-path"/><path class="hp-fill" stroke="none"/><text class="facing-label"/><text class="hp-text" text-anchor="middle" font-size="0.5rem"/>';
        g.querySelector('.arc-path').addEventListener('click', () => {
          const cur = this.#state || {};
          if (cur.auto) return;
          if (this.sendAction && id) {
            const isFocusedNow = cur.focused_facing === id || cur.focused_facing === f.label;
            this.sendAction('set_shield_focus', { arc_id: id, focused: !isFocusedNow });
          }
        });
        this.#facingGs.set(id, g);
        this.#arcsGroup.appendChild(g);
      }

      // Arc outline
      const outer = `M ${x0} ${y0} A ${r} ${r} 0 ${largeArc} 1 ${x1} ${y1} L ${xi1} ${yi1} A ${ir} ${ir} 0 ${largeArc} 0 ${xi0} ${yi0} Z`;
      const fillColor = !online ? '#282c38' : isFocused ? '#4ec870' : '#1a3a28';
      const opacity = online ? (isFocused ? 0.9 : 0.5) : 0.2;
      const outline = g.children[0];
      outline.setAttribute('d', outer);
      outline.setAttribute('fill', fillColor);
      outline.setAttribute('opacity', opacity);
      outline.setAttribute('stroke', isFocused ? '#4ec870' : '#282c38');
      outline.setAttribute('stroke-width', isFocused ? '2' : '1');
      outline.setAttribute('data-facing-id', id);
      outline.setAttribute('class', 'arc-path' + (isFocused ? ' focused' : '') + (!online ? ' down' : ''));

      // HP fill arc — fills/drains radially from the inner edge outward
      const hpFill = g.children[1];
      if (online && pct > 0) {
        const fillPct = Math.min(1, Math.max(0, pct));
        const ro = ir + fillPct * (r - ir);
        const xo0 = cx + ro * Math.cos(a0), yo0 = cy + ro * Math.sin(a0);
        const xo1 = cx + ro * Math.cos(a1), yo1 = cy + ro * Math.sin(a1);

        const fillOuter = `M ${xi0} ${yi0} L ${xo0} ${yo0} A ${ro} ${ro} 0 ${largeArc} 1 ${xo1} ${yo1} L ${xi1} ${yi1} A ${ir} ${ir} 0 ${largeArc} 0 ${xi0} ${yi0} Z`;
        const hpColor = pct > 0.6 ? '#4ec870' : pct > 0.25 ? '#d8a040' : '#e0402c';
        hpFill.setAttribute('d', fillOuter);
        hpFill.setAttribute('fill', hpColor);
        hpFill.setAttribute('opacity', isFocused ? '0.85' : '0.55');
        hpFill.style.display = '';
      } else {
        hpFill.style.display = 'none';
      }

      // Label
      const lr = r + 16;
      const lx = cx + lr * Math.cos(midAngle);
      const ly = cy + lr * Math.sin(midAngle);
      const label = (f.label || f.arc_id || '').substring(0, 5).toUpperCase();
      const labelEl = g.children[2];
      labelEl.setAttribute('x', lx);
      labelEl.setAttribute('y', ly);
      labelEl.setAttribute('dy', '0.35em');
      labelEl.textContent = label;
      labelEl.setAttribute('class', 'facing-label' + (isFocused ? ' focused-label' : ''));

      // HP text inside arc
      const hpLabel = !online ? 'OFF' : Math.round(pct * 100) + '%';
      const ix = cx + (ir + (r - ir) / 2) * Math.cos(midAngle);
      const iy = cy + (ir + (r - ir) / 2) * Math.sin(midAngle);
      const hpText = g.children[3];
      hpText.setAttribute('x', ix);
      hpText.setAttribute('y', iy);
      hpText.setAttribute('dy', '0.35em');
      hpText.setAttribute('fill', online ? '#cce' : '#6a7178');
      hpText.textContent = hpLabel;
    });
  }
}

if (typeof window !== 'undefined' && !customElements.get('ph-shield-facings')) {
  customElements.define('ph-shield-facings', PhShieldFacings);
}
