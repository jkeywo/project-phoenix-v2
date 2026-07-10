import './ph-radar.js';

export class PhTacticalRadar extends HTMLElement {
  #state = null;
  #innerRadar = null;

  constructor() {
    super();
    this.attachShadow({ mode: 'open' });
    const t = document.createElement('template');
    t.innerHTML = [
      '<style>',
      ':host { display: block; position: relative; }',
      '.container { position: relative; width: 100%; height: 100%; }',
      'ph-radar { display: block; width: 100%; height: 100%; }',
      '.overlay { position: absolute; inset: 0; pointer-events: none; overflow: visible; }',
      '</style>',
      '<div class="container">',
      '  <ph-radar id="inner-radar"></ph-radar>',
      '  <svg class="overlay" viewBox="0 0 100 100" preserveAspectRatio="xMidYMid meet">',
      '    <g id="phaser-arcs"></g>',
      '    <g id="torpedo-arcs"></g>',
      '    <g id="selected-highlight"></g>',
      '  </svg>',
      '</div>',
    ].join('\n');
    this.shadowRoot.appendChild(t.content.cloneNode(true));
    this.#innerRadar = this.shadowRoot.getElementById('inner-radar');
  }

  connectedCallback() {
    this.sendAction ??= window.sendAction;
    if (this.#innerRadar) {
      this.#innerRadar.sendAction = (action, payload) => {
        this.sendAction?.(action, payload);
      };
    }
  }

  set state(val) {
    this.#state = val;
    this.#render();
  }

  get state() { return this.#state; }

  #render() {
    const s = this.#state || {};
    if (this.#innerRadar) {
      this.#innerRadar.state = {
        blips: s.blips || [],
        range: s.range,
        ship_heading: s.ship_heading,
        config: s.config || {},
      };
    }
    this.#renderOverlays(s);
  }

  #renderOverlays(s) {
    const cx = 50, cy = 50, r = 46;
    this.#renderArcGroup(s.phaser_arcs || [], 'phaser-arcs', cx, cy, r, '#4ec870', 0.3);
    this.#renderArcGroup(s.torpedo_arcs || [], 'torpedo-arcs', cx, cy, r, '#e8c84a', 0.25);
    this.#renderHighlight(s, cx, cy, r);
  }

  #wedgePath(cx, cy, r, facingDeg, arcDeg) {
    const halfArc = arcDeg / 2;
    const startDeg = facingDeg - halfArc - 90;
    const endDeg = facingDeg + halfArc - 90;
    const sr = startDeg * Math.PI / 180;
    const er = endDeg * Math.PI / 180;
    const x1 = cx + r * Math.cos(sr);
    const y1 = cy + r * Math.sin(sr);
    const x2 = cx + r * Math.cos(er);
    const y2 = cy + r * Math.sin(er);
    const large = arcDeg > 180 ? 1 : 0;
    return [
      'M', cx.toFixed(1), cy.toFixed(1),
      'L', x1.toFixed(1), y1.toFixed(1),
      'A', r, r, 0, large, 1, x2.toFixed(1), y2.toFixed(1),
      'Z',
    ].join(' ');
  }

  #renderArcGroup(arcs, containerId, cx, cy, r, defaultColor, defaultOpacity) {
    const g = this.shadowRoot.getElementById(containerId);
    if (!g) return;
    while (g.children.length > arcs.length) g.removeChild(g.lastChild);
    arcs.forEach((a, i) => {
      const d = this.#wedgePath(cx, cy, r, a.facing_deg || 0, a.arc_deg || 0);
      let path;
      if (i < g.children.length) { path = g.children[i]; }
      else { path = document.createElementNS('http://www.w3.org/2000/svg', 'path'); g.appendChild(path); }
      path.setAttribute('d', d);
      path.setAttribute('fill', a.color || defaultColor);
      path.setAttribute('fill-opacity', String(a.opacity ?? defaultOpacity));
    });
  }

  #renderHighlight(s, cx, cy, r) {
    const g = this.shadowRoot.getElementById('selected-highlight');
    if (!g) return;
    const uuid = s.selected_target_uuid;
    const blip = uuid ? (s.blips || []).find(b => b.uuid === uuid) : null;
    if (!blip) {
      while (g.firstChild) g.removeChild(g.firstChild);
      return;
    }
    let circle = g.firstChild;
    if (!circle) {
      circle = document.createElementNS('http://www.w3.org/2000/svg', 'circle');
      circle.setAttribute('fill', 'none');
      circle.setAttribute('stroke', '#6cb6d0');
      circle.setAttribute('stroke-width', '1.5');
      g.appendChild(circle);
    }
    const bx = cx + (blip.radar_x != null ? blip.radar_x : 0) * r;
    const by = cy - (blip.radar_y != null ? blip.radar_y : 0) * r;
    circle.setAttribute('cx', bx.toFixed(1));
    circle.setAttribute('cy', by.toFixed(1));
    circle.setAttribute('r', '5');
  }
}

if (typeof window !== 'undefined' && !customElements.get('ph-tactical-radar')) {
  customElements.define('ph-tactical-radar', PhTacticalRadar);
}
