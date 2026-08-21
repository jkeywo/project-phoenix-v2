import './ph-radar.js';
// strings-boot first: its top-level await delays this module's evaluation —
// and therefore this element's registration and upgrade — until the string
// table is loaded, so the constructor's template t() calls never see an
// empty table. No-op in Node tests (setup-strings.js loads the table there).
import '../strings-boot.js';
import { t } from '../strings.js';
import { phAdoptConsoleStyles, phColor } from './ph-console-styles.js';
import {
  SCOPE_CHROME_CSS, scopeChromeMarkup, updateScopeChrome,
  applyArcCompositeCap, cappedArcAlpha,
} from './ph-scope-chrome.js';

export class PhHelmRadar extends HTMLElement {
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
      '.svg-overlay { position: absolute; inset: 0; pointer-events: none; overflow: visible; }',
      '.thrust-arc { fill: none; stroke: var(--cyan); stroke-width: 4; stroke-linecap: round; }',
      '.hostile-arc { stroke: none; }',
      SCOPE_CHROME_CSS,
      '.on-screen-btn {',
      '  position: absolute; bottom: 6%; right: 6%;',
      '  pointer-events: auto; z-index: 10;',
      '  font-family: \'JetBrains Mono\', monospace; font-size: var(--text-xs);',
      '  letter-spacing: 0.15em; color: var(--ink-dim); background: rgba(var(--rgb-deep), 0.85);',
      '  border: 1px solid var(--line-faint); border-radius: 2px; padding: 2px 12px;',
      '  cursor: pointer; text-transform: uppercase;',
      '  transition: border-color 0.15s, color 0.15s, background 0.15s;',
      /* The touch floor (PRD #1023 module 3). inline-flex because min-height
         does nothing to an inline box, and the label has to stay centred in a
         control that is now taller than its own text. */
      '  display: inline-flex; align-items: center; justify-content: center;',
      '  min-height: var(--control-hit-min);',
      '}',
      '.on-screen-btn:hover { border-color: var(--cyan); }',
      '.on-screen-btn.active { border-color: var(--cyan); color: var(--cyan); background: rgba(var(--rgb-cyan), 0.18); }',
      '</style>',
      '<div class="container">',
      '  <ph-radar id="inner-radar"></ph-radar>',
      '  <svg class="svg-overlay" viewBox="0 0 100 100" preserveAspectRatio="xMidYMid meet">',
      // Hostile weapon arcs sit FIRST in this SVG so they paint under the
      // thrust arcs — the controls the helm is actually flying by. Note they do
      // NOT sit under the blips: `.svg-overlay` is a sibling AFTER `<ph-radar>`
      // with `position: absolute`, so the whole overlay paints over the radar's
      // contacts. The authored alpha (0.07) is what keeps the blips legible
      // through it, not the stacking order (issue #874).
      '    <g id="hostile-arcs"></g>',
      '    <path class="thrust-arc" id="arc-port" />',
      '    <path class="thrust-arc" id="arc-stbd" />',
      '  </svg>',
      scopeChromeMarkup(),
      '  <button class="on-screen-btn" id="on-screen-btn" type="button">' + t('console.common.on_screen') + '</button>',
      '</div>',
    ].join('\n');
    this.shadowRoot.appendChild(tpl.content.cloneNode(true));
    this.#innerRadar = this.shadowRoot.getElementById('inner-radar');
    // Every hostile on the scope contributes its own banks to this one group,
    // so it stacks harder than the tactical scope's does — a three-ship
    // engagement can put a dozen wedges over the same pixel.
    applyArcCompositeCap(this.shadowRoot.getElementById('hostile-arcs'));
  }

  connectedCallback() {
    if (!this.sendAction) {
      if (typeof window !== 'undefined' && typeof window.sendAction === 'function') {
        this.sendAction = window.sendAction;
      }
    }
    // Name + role for the scope (issue #1176). Unlike the tactical radar this
    // scope is a passive contact DISPLAY — it locks no target, so it takes no
    // tabindex and no arrow cursor (an operationless Tab stop would be a wrong
    // stop, the same reason ph-radar is EXEMPT). `role="group"` + a catalogue
    // name label the display for a screen reader without inventing a selection
    // behaviour; the ON SCREEN button inside stays the one keyboard-operable
    // control, focusable and Enter/Space-activated as a native button.
    this.setAttribute('role', 'group');
    this.setAttribute('aria-label', t('component.helm_radar.label'));
    this.shadowRoot.getElementById('on-screen-btn').addEventListener('click', () => {
      if (this.sendAction) {
        this.sendAction('set_radar_view', {});
      }
    });
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

    this.#updateThrustArcs(s);
    this.#renderHostileArcs(s);

    updateScopeChrome(this.shadowRoot, {
      x: s.x, z: s.z, headingDeg: s.ship_heading, speed: s.speed,
    });

    const btn = this.shadowRoot.getElementById('on-screen-btn');
    if (btn) {
      btn.classList.toggle('active', !!s.on_screen_active);
    }
  }

  #updateThrustArcs(state) {
    const port = Math.max(0, Math.min(1, state.engine_port_thrust || 0));
    const stbd = Math.max(0, Math.min(1, state.engine_stbd_thrust || 0));

    const cx = 50, cy = 50, r = 42;
    const maxSweep = 50;

    const arcPort = this.shadowRoot.getElementById('arc-port');
    const arcStbd = this.shadowRoot.getElementById('arc-stbd');

    if (arcPort) {
      const path = port > 0 ? this.#arcPath(cx, cy, r, 90 + port * maxSweep, 90) : '';
      arcPort.setAttribute('d', path);
      arcPort.style.opacity = String(0.2 + 0.8 * port);
    }
    if (arcStbd) {
      const path = stbd > 0 ? this.#arcPath(cx, cy, r, 90, 90 - stbd * maxSweep) : '';
      arcStbd.setAttribute('d', path);
      arcStbd.style.opacity = String(0.2 + 0.8 * stbd);
    }
  }

  /**
   * Draw the hostile weapon-arc overlay (issue #874).
   *
   * Every sector drawn here is a sector the SERVER produced: `bearing_deg` and
   * `half_angle_deg` arrive on the wire from
   * `weapons::arc_geometry::weapon_arc_sectors`, the same producer output the
   * backfilled helm AI's exposure fact is reduced from. This method does no arc
   * math — it only projects: world bearing → screen angle (subtract the ship's
   * heading), and world position/range → the radar's normalised scope space.
   *
   * The component deliberately does NOT recompute arcs from the hostile's yaw,
   * which it could: that would make the human's picture agree with the AI's by
   * coincidence rather than by construction.
   *
   * Red alert is not gated here — the server omits the field entirely when the
   * ship is not at red alert, and `buildHelmConsoleState` latches the same
   * condition. This method renders exactly what it is handed.
   */
  #renderHostileArcs(s) {
    const g = this.shadowRoot.getElementById('hostile-arcs');
    if (!g) return;
    // The colour is authored in `[helm_console] hostile_arc_color` and arrives
    // on the payload. This component deliberately carries NO placeholder of its
    // own: `ClientSimState` already initialises `hostileArcColor` with the
    // single client-side placeholder (AGENTS.md #11(b)), so a second literal
    // here would be both unreachable and a third value free to drift from the
    // other two. Handed no colour at all, paint no overlay — an invented colour
    // would be a hint the server never authorised.
    const rgba = s.hostile_arc_color;
    const contacts = Array.isArray(rgba) ? (s.hostile_arcs || []) : [];
    const fill = Array.isArray(rgba)
      ? 'rgb(' + [0, 1, 2].map(i => Math.round((rgba[i] ?? 0) * 255)).join(',') + ')'
      : 'none';
    const opacity = Array.isArray(rgba) ? (rgba[3] ?? 0) : 0;

    const range = s.range > 0 ? s.range : 1;
    const shipX = s.x || 0;
    const shipZ = s.z || 0;
    const heading = s.ship_heading || 0;
    const yaw = heading * Math.PI / 180;
    const cosY = Math.cos(yaw), sinY = Math.sin(yaw);

    const paths = [];
    for (const c of contacts) {
      // Project the anchor into the same ship-local scope space `buildBlips`
      // puts the blips in, so the wedges radiate from the contact's own blip.
      const dx = (c.x || 0) - shipX;
      const dz = (c.z || 0) - shipZ;
      const nx = (dx * cosY + dz * sinY) / range;
      const ny = (dx * sinY - dz * cosY) / range;
      const px = 50 + nx * 50;
      const py = 50 - ny * 50;
      for (const a of (c.arcs || [])) {
        // World bearing → ship-relative screen bearing. The only trigonometry
        // this overlay is allowed: a rotation, never an arc derivation.
        const facing = (a.bearing_deg || 0) - heading;
        const half = a.half_angle_deg || 0;
        const r = ((a.range || 0) / range) * 50;
        if (half <= 0 || r <= 0) continue;
        paths.push(this.#wedgePath(px, py, r, facing, half));
      }
    }

    while (g.children.length > paths.length) g.removeChild(g.lastChild);
    paths.forEach((d, i) => {
      let path = g.children[i];
      if (!path) {
        path = document.createElementNS('http://www.w3.org/2000/svg', 'path');
        path.setAttribute('class', 'hostile-arc');
        g.appendChild(path);
      }
      path.setAttribute('d', d);
      path.setAttribute('fill', phColor(this, fill));
      // The authored 0.07 keeps ONE wedge legible; the group's cap is what
      // keeps a dozen of them legible. See `applyArcCompositeCap`.
      path.setAttribute('fill-opacity', String(cappedArcAlpha(opacity)));
    });
  }

  /**
   * A pie wedge centred on `(cx, cy)`, spanning `facingDeg ± halfDeg` where
   * `facingDeg` is a ship-relative bearing (0 = up/forward, +90 = starboard).
   * Same construction as the Tactical radar's own arc wedges, but anchored at an
   * arbitrary point rather than at the scope centre.
   */
  #wedgePath(cx, cy, r, facingDeg, halfDeg) {
    const startDeg = facingDeg - halfDeg - 90;
    const endDeg = facingDeg + halfDeg - 90;
    const sr = startDeg * Math.PI / 180;
    const er = endDeg * Math.PI / 180;
    const x1 = (cx + r * Math.cos(sr)).toFixed(1);
    const y1 = (cy + r * Math.sin(sr)).toFixed(1);
    const x2 = (cx + r * Math.cos(er)).toFixed(1);
    const y2 = (cy + r * Math.sin(er)).toFixed(1);

    // The SVG spec (implementation notes F.6.2) OMITS an elliptical arc whose
    // endpoints are identical — such a wedge collapses to a zero-area line and,
    // with `.hostile-arc { stroke: none }`, paints nothing at all while
    // `arc_exposure` still reads the bank as covering. That is the human/AI
    // divergence this branch exists to prevent, so the test is the one the
    // renderer actually applies: the endpoints AS EMITTED, after `toFixed(1)`.
    // Testing `halfDeg * 2 >= 360` instead would miss every sweep just under a
    // full turn whose residual gap rounds away at small screen radii — a
    // short-ranged bank on a wide scope lands there routinely. A
    // `fire_arc_deg = 360.0` bank is authored content (the Alliance destroyer's
    // `omni` suppression phaser), so the collapsing case is a real hull's bank.
    // Emit the full disc as two half-circles, which have distinct endpoints and
    // so survive the spec's degenerate-arc rule.
    //
    // `halfDeg >= 180` is checked as well as the emitted endpoints, because a
    // bank wider than a full turn wraps PAST its own start: its endpoints stop
    // coinciding, and the arc renders as a disc with a notch cut out of it —
    // covering less than the whole circle while `arc_exposure` reads any
    // `half_angle_deg >= 180` as inescapable from every bearing. Nothing
    // authors more than 360 today, and `weapon_arc_sectors` does not clamp, so
    // this is the cheap half of the guard rather than a reachable bug.
    if (halfDeg >= 180 || (x1 === x2 && y1 === y2)) {
      const x = cx.toFixed(1);
      const rr = r.toFixed(1);
      const top = (cy - r).toFixed(1);
      const bottom = (cy + r).toFixed(1);
      return [
        'M', x, top,
        'A', rr, rr, 0, 1, 1, x, bottom,
        'A', rr, rr, 0, 1, 1, x, top,
        'Z',
      ].join(' ');
    }
    const large = halfDeg * 2 > 180 ? 1 : 0;
    return [
      'M', cx.toFixed(1), cy.toFixed(1),
      'L', x1, y1,
      'A', r.toFixed(1), r.toFixed(1), 0, large, 1, x2, y2,
      'Z',
    ].join(' ');
  }

  #arcPath(cx, cy, r, startAngle, endAngle) {
    const startRad = startAngle * Math.PI / 180;
    const endRad = endAngle * Math.PI / 180;
    const x1 = cx + r * Math.cos(startRad);
    const y1 = cy + r * Math.sin(startRad);
    const x2 = cx + r * Math.cos(endRad);
    const y2 = cy + r * Math.sin(endRad);
    const sweep = endAngle - startAngle;
    const largeArc = Math.abs(sweep) > 180 ? 1 : 0;
    const sweepFlag = sweep >= 0 ? 1 : 0;
    return [
      'M', x1.toFixed(1), y1.toFixed(1),
      'A', r, r, 0, largeArc, sweepFlag, x2.toFixed(1), y2.toFixed(1),
    ].join(' ');
  }
}

if (typeof window !== 'undefined' && !customElements.get('ph-helm-radar')) {
  customElements.define('ph-helm-radar', PhHelmRadar);
}
