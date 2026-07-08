export class PhShieldFacings extends HTMLElement {
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
    .auto-badge { font-size: 0.55rem; color: #f0c040; border: 1px solid #f0c040; padding: 0.05rem 0.3rem; letter-spacing: 0.2em; }
    .arc-container { display: flex; justify-content: center; align-items: center; padding: 0.5rem 0; }
    svg { width: 100%; max-width: 200px; height: auto; overflow: visible; }
    .arc-path { cursor: pointer; transition: opacity 0.2s, filter 0.2s; }
    .arc-path:hover { filter: brightness(1.3); }
    .arc-path.focused { filter: brightness(1.5) drop-shadow(0 0 4px #4ec870); }
    .arc-path.down { opacity: 0.3; cursor: default; }
    .facing-label { font-size: 0.55rem; fill: #6a7178; text-anchor: middle; pointer-events: none; }
    .facing-label.focused-label { fill: #cce; font-weight: 600; }
    .empty { font-size: 0.65rem; color: #6a7178; text-align: center; padding: 0.75rem 0; letter-spacing: 0.2em; }
  </style>
  <div class="header">
    <span>SHIELD FACINGS</span>
    <span class="auto-badge" id="auto-badge" style="display:none">AUTO</span>
  </div>
  <div class="arc-container" id="arc-container">
    <div class="empty">NO FACING DATA</div>
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
    const container = this.shadowRoot.getElementById('arc-container');
    const badge = this.shadowRoot.getElementById('auto-badge');
    badge.style.display = auto ? 'inline' : 'none';

    if (facings.length === 0) {
      container.innerHTML = '<div class="empty">NO FACING DATA</div>';
      return;
    }

    const n = facings.length;
    const cx = 100, cy = 100, r = 70, ir = 35;
    const angleStep = (Math.PI * 2) / n;
    const startAngle = -Math.PI / 2 - angleStep / 2;

    let svg = `<svg viewBox="0 0 200 200" xmlns="http://www.w3.org/2000/svg">`;

    facings.forEach((f, i) => {
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

      // Full arc outline (background)
      const outer = `M ${x0} ${y0} A ${r} ${r} 0 ${largeArc} 1 ${x1} ${y1} L ${xi1} ${yi1} A ${ir} ${ir} 0 ${largeArc} 0 ${xi0} ${yi0} Z`;
      const fillColor = !online ? '#282c38' : isFocused ? '#4ec870' : '#1a3a28';
      const opacity = online ? (isFocused ? 0.9 : 0.5) : 0.2;

      svg += `<path class="arc-path${isFocused ? ' focused' : ''}${!online ? ' down' : ''}" d="${outer}" fill="${fillColor}" opacity="${opacity}" stroke="${isFocused ? '#4ec870' : '#282c38'}" stroke-width="${isFocused ? 2 : 1}" data-facing-id="${f.id}" />`;

      // HP fill arc
      if (online && pct > 0) {
        const fillPct = Math.min(1, Math.max(0, pct));
        const a0p = a0 + (1 - fillPct) * angleStep;
        const x0p = cx + ir * Math.cos(a0p), y0p = cy + ir * Math.sin(a0p);
        const x1p = cx + r * Math.cos(a0p), y1p = cy + r * Math.sin(a0p);

        const fillOuter = `M ${x0p} ${y0p} L ${x1p} ${y1p} A ${r} ${r} 0 ${largeArc} 1 ${x1} ${y1} L ${xi1} ${yi1} A ${ir} ${ir} 0 ${largeArc} 0 ${xi0} ${yi0} A ${ir} ${ir} 0 ${largeArc} 1 ${x0p} ${y0p} Z`;
        const hpColor = pct > 0.6 ? '#4ec870' : pct > 0.25 ? '#d8a040' : '#e0402c';
        svg += `<path d="${fillOuter}" fill="${hpColor}" opacity="${isFocused ? 0.85 : 0.55}" stroke="none" data-facing-id="${f.id}" />`;
      }

      // Label
      const midAngle = a0 + angleStep / 2;
      const lr = r + 16;
      const lx = cx + lr * Math.cos(midAngle);
      const ly = cy + lr * Math.sin(midAngle);
      const label = (f.label || f.id || '').substring(0, 5).toUpperCase();
      svg += `<text class="facing-label${isFocused ? ' focused-label' : ''}" x="${lx}" y="${ly}" dy="0.35em">${label}</text>`;

      // HP text inside arc
      const hpLabel = !online ? 'OFF' : Math.round(pct * 100) + '%';
      const ix = cx + (ir + (r - ir) / 2) * Math.cos(midAngle);
      const iy = cy + (ir + (r - ir) / 2) * Math.sin(midAngle);
      svg += `<text x="${ix}" y="${iy}" dy="0.35em" text-anchor="middle" font-size="0.5rem" fill="${online ? '#cce' : '#6a7178'}">${hpLabel}</text>`;
    });

    svg += '</svg>';
    container.innerHTML = svg;

    // Bind click handlers
    container.querySelectorAll('.arc-path').forEach(path => {
      path.addEventListener('click', () => {
        if (auto) return;
        const id = path.dataset.facingId;
        if (this.sendAction && id) {
          this.sendAction('focus_shield', { facing_id: id });
        }
      });
    });
  }
}

if (typeof window !== 'undefined' && !customElements.get('ph-shield-facings')) {
  customElements.define('ph-shield-facings', PhShieldFacings);
}
