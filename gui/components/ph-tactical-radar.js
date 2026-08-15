import './ph-radar.js';
// strings-boot first: its top-level await delays this module's evaluation —
// and therefore this element's registration and upgrade — until the string
// table is loaded, so the constructor's template t() calls never see an
// empty table. No-op in Node tests (setup-strings.js loads the table there).
import '../strings-boot.js';
import { t } from '../strings.js';
import { phAdoptConsoleStyles, phColor } from './ph-console-styles.js';

export class PhTacticalRadar extends HTMLElement {
  #state = null;
  #innerRadar = null;

  constructor() {
    super();
    this.attachShadow({ mode: 'open' });
    // Every component adopts the shared control family (module 1 of PRD
    // #1023): custom properties cross a shadow boundary, class rules do not.
    phAdoptConsoleStyles(this.shadowRoot);
    const tpl = document.createElement('template');
    tpl.innerHTML = [
      '<style>',
      ':host { display: block; position: relative; }',
      '.container { position: relative; width: 100%; height: 100%; }',
      'ph-radar { display: block; width: 100%; height: 100%; }',
      '.overlay { position: absolute; inset: 0; pointer-events: none; overflow: visible; }',
      '.corner-label {',
      '  position: absolute; pointer-events: none; z-index: 10;',
      '  font-family: \'JetBrains Mono\', monospace; font-size: var(--text-xs);',
      '  letter-spacing: 0.1em; color: var(--edge-strong);',
      '}',
      '.corner-label.top-left { top: 4%; left: 6%; }',
      '.corner-label.top-right { top: 4%; right: 6%; text-align: right; }',
      '.corner-label.bottom-left { bottom: 6%; left: 6%; }',
      '#torpedo-badges text {',
      '  font-family: \'JetBrains Mono\', monospace; font-size: var(--svg-badge-size);',
      '  letter-spacing: 0.08em; fill: var(--gold-bright);',
      '}',
      '</style>',
      '<div class="container">',
      '  <ph-radar id="inner-radar"></ph-radar>',
      '  <svg class="overlay" viewBox="0 0 100 100" preserveAspectRatio="xMidYMid meet">',
      '    <g id="phaser-arcs"></g>',
      '    <g id="torpedo-arcs"></g>',
      '    <g id="selected-highlight"></g>',
      '    <g id="torpedo-badges"></g>',
      '  </svg>',
      '  <div class="corner-label top-left" id="label-pos">X: 0  Z: 0</div>',
      '  <div class="corner-label top-right" id="label-bearing">000°</div>',
      '  <div class="corner-label bottom-left" id="label-speed">0.0 km/s</div>',
      '</div>',
    ].join('\n');
    this.shadowRoot.appendChild(tpl.content.cloneNode(true));
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
        target_uuid: s.target_uuid || null,
      };
    }
    this.#renderOverlays(s);

    const posLabel = this.shadowRoot.getElementById('label-pos');
    if (posLabel) {
      const x = s.x != null ? s.x : 0;
      const z = s.z != null ? s.z : 0;
      posLabel.textContent = t('console.common.radar_pos', { x: x.toFixed(0), z: z.toFixed(0) });
    }

    const bearingLabel = this.shadowRoot.getElementById('label-bearing');
    if (bearingLabel) {
      const h = s.ship_heading != null ? ((s.ship_heading % 360) + 360) % 360 : 0;
      bearingLabel.textContent = String(h.toFixed(0)).padStart(3, '0') + '\u00B0';
    }

    const speedLabel = this.shadowRoot.getElementById('label-speed');
    if (speedLabel) {
      const spd = s.speed != null ? s.speed : 0;
      speedLabel.textContent = (spd * 3.6).toFixed(1) + ' km/s';
    }

  }

  #renderOverlays(s) {
    const cx = 50, cy = 50, r = 46;
    this.#renderArcGroup(s.phaser_arcs || [], 'phaser-arcs', cx, cy, r, 'var(--loaded)', 0.3);
    this.#renderArcGroup(s.torpedo_arcs || [], 'torpedo-arcs', cx, cy, r, 'var(--gold-bright)', 0.25);
    this.#renderHighlight(s, cx, cy, r);
    this.#renderTorpedoBadges(s, cx, cy, r);
  }

  /**
   * Torpedo-armed markers (issue #957): one short badge beside each hostile
   * contact whose hull carries tubes, drawn BEFORE it fires so the crew can
   * tell a torpedo boat from a phaser-only escort.
   *
   * The text is never composed here — it arrives on the blip as
   * `torpedo_badge`, already resolved from a strings.csv id by
   * `foldTorpedoBadges` in gui/console-state.js. A blip without the key draws
   * nothing, so a server that sent no capability data badges nobody.
   */
  #renderTorpedoBadges(s, cx, cy, r) {
    const g = this.shadowRoot.getElementById('torpedo-badges');
    if (!g) return;
    const badged = (s.blips || []).filter(b => b && b.torpedo_badge);
    while (g.children.length > badged.length) g.removeChild(g.lastChild);
    badged.forEach((b, i) => {
      let label = g.children[i];
      if (!label) {
        label = document.createElementNS('http://www.w3.org/2000/svg', 'text');
        g.appendChild(label);
      }
      const bx = cx + (b.radar_x != null ? b.radar_x : 0) * r;
      const by = cy - (b.radar_y != null ? b.radar_y : 0) * r;
      label.setAttribute('x', (bx + 3).toFixed(1));
      label.setAttribute('y', (by - 3).toFixed(1));
      label.setAttribute('data-uuid', b.uuid || '');
      label.textContent = b.torpedo_badge;
    });
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
      path.setAttribute('fill', phColor(this, a.color || defaultColor));
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
      circle.setAttribute('fill', phColor(this, 'none'));
      circle.setAttribute('stroke', phColor(this, 'var(--cyan)'));
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
