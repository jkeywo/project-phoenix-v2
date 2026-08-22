import './ph-radar.js';
// strings-boot first: its top-level await delays this module's evaluation —
// and therefore this element's registration and upgrade — until the string
// table is loaded, so the constructor's template t() calls never see an
// empty table. No-op in Node tests (setup-strings.js loads the table there).
import '../strings-boot.js';
import { t } from '../strings.js';
import { phColor } from './ph-console-styles.js';
import {
  SCOPE_CHROME_CSS, scopeChromeMarkup, updateScopeChrome,
  applyArcCompositeCap, cappedArcAlpha, phPx, TEXT_MIN_FALLBACK_PX,
} from './ph-scope-chrome.js';
import { rovingKeyTarget } from '../roving-tabindex.js';
import { PhElement, phDefine } from './ph-element.js';

/** The overlay's user-space box. Badge geometry is in these units, not pixels. */
const SCOPE_VIEWBOX = 100;

export class PhTacticalRadar extends PhElement {
  #state = null;
  #resizeObserver = null;
  // The keyboard target cursor (issue #1170): the contact the arrow keys have
  // moved to, which Enter/Space then locks. A scope is a canvas, so it takes
  // the single-focusable + arrow-to-cycle pattern rather than roving tabindex —
  // there are no per-contact DOM nodes to rove between (a structured contact
  // list for a screen reader is explicitly out of scope for this band).
  #cursorUuid = null;

  template() {
    return [
      '<style>',
      ':host { display: block; position: relative; }',
      '.container { position: relative; width: 100%; height: 100%; }',
      'ph-radar { display: block; width: 100%; height: 100%; }',
      '.overlay { position: absolute; inset: 0; pointer-events: none; overflow: visible; }',
      SCOPE_CHROME_CSS,
      '#torpedo-badges text {',
      '  font-family: var(--font-mono); font-size: var(--svg-badge-size);',
      '  letter-spacing: var(--tracking-tight); fill: var(--gold-bright);',
      '}',
      '</style>',
      '<div class="container">',
      '  <ph-radar id="inner-radar"></ph-radar>',
      '  <svg class="overlay" viewBox="0 0 ' + SCOPE_VIEWBOX + ' ' + SCOPE_VIEWBOX + '"'
        + ' preserveAspectRatio="xMidYMid meet">',
      '    <g id="phaser-arcs"></g>',
      '    <g id="torpedo-arcs"></g>',
      '    <g id="selected-highlight"></g>',
      '    <g id="keyboard-cursor"></g>',
      '    <g id="torpedo-badges"></g>',
      '  </svg>',
      scopeChromeMarkup(),
      '</div>',
    ].join('\n');
  }

  onTemplate() {
    // A PLAIN property, not a #field: onTemplate runs from the base constructor
    // before this subclass's field-init phase, so a declared #innerRadar would
    // be overwritten back to null the moment that phase runs (ph-element.js).
    this.innerRadar = this.shadowRoot.getElementById('inner-radar');
    // Both arc groups are capped once, here, rather than per render: the cap is
    // a property of the group, not of the arcs that happen to be in it today.
    applyArcCompositeCap(this.shadowRoot.getElementById('phaser-arcs'));
    applyArcCompositeCap(this.shadowRoot.getElementById('torpedo-arcs'));
  }

  connectedCallback() {
    super.connectedCallback();
    if (this.innerRadar) {
      this.innerRadar.sendAction = (action, payload) => {
        this.sendAction?.(action, payload);
      };
    }
    // Role + accessible name + keyboard reach (issue #1170). The scope is one
    // Tab stop (`tabindex="0"`); `role="group"` with a name is the honest role
    // for a composite whose contents a screen reader cannot yet enumerate —
    // name/role hygiene, not narration. Arrow keys then cycle the target
    // cursor and Enter/Space lock it, so the scope is fully keyboard-operable
    // without a pointer.
    this.setAttribute('role', 'group');
    this.setAttribute('aria-label', t('component.tactical_radar.label'));
    if (!this.hasAttribute('tabindex')) this.setAttribute('tabindex', '0');
    if (!this.#keydownBound) {
      this.addEventListener('keydown', this.#onKeyDown);
      this.#keydownBound = true;
    }
    this.#syncBadgeScale();
    if (typeof ResizeObserver === 'function' && !this.#resizeObserver) {
      this.#resizeObserver = new ResizeObserver(() => this.#syncBadgeScale());
      this.#resizeObserver.observe(this);
    }
  }

  #keydownBound = false;

  /**
   * The contacts the target cursor can rest on: anything with a uuid the
   * pointer path would also lock. Waypoints are markers, not targets, and a
   * contact the server flags `selectable: false` is excluded; everything else
   * is fair game, exactly as a tap is (see ph-radar's `#onPointerTap`, which
   * locks any blip it hits).
   */
  #selectableBlips() {
    const blips = (this.#state && this.#state.blips) || [];
    return blips.filter((b) => b && b.uuid && b.kind !== 'waypoint' && b.selectable !== false);
  }

  #onKeyDown = (event) => {
    const list = this.#selectableBlips();
    if (list.length === 0) return;
    const uuids = list.map((b) => b.uuid);
    const key = event.key;

    // Enter / Space lock the cursor's contact through the SAME named action the
    // tap emits (issue #1170) — no second designation path, no behaviour fork.
    if (key === 'Enter' || key === ' ' || key === 'Spacebar') {
      if (this.#cursorUuid && this.sendAction) {
        event.preventDefault();
        this.sendAction('set_target', { uuid: this.#cursorUuid });
      }
      return;
    }

    const current = uuids.indexOf(this.#cursorUuid);
    let next;
    if (current < 0) {
      // No cursor yet: the first navigation key lands on an end of the ring
      // rather than skipping the contact under it.
      if (key === 'ArrowUp' || key === 'ArrowLeft' || key === 'End') next = uuids.length - 1;
      else if (key === 'ArrowDown' || key === 'ArrowRight' || key === 'Home') next = 0;
      else return;
    } else {
      next = rovingKeyTarget(uuids.length, current, key, 'both');
    }
    if (next < 0) return;
    event.preventDefault();
    this.#cursorUuid = uuids[next];
    this.#renderCursor();
  };

  disconnectedCallback() {
    if (this.#resizeObserver) {
      this.#resizeObserver.disconnect();
      this.#resizeObserver = null;
    }
  }

  /**
   * Put the torpedo badge on the shared type floor.
   *
   * An SVG `font-size` inside `viewBox="0 0 100 100"` is in USER-SPACE units,
   * so it scales with the element: the authored `--svg-badge-size: 3.2px`
   * rendered at 9.6 CSS px on a 300px scope and 4.4 on a phone's 138px one —
   * comfortably under the 11px floor the type ramp exists to hold, and the size
   * the design audit noticed the badge had shipped at.
   *
   * A user-space length cannot carry a pixel floor on its own, so the floor is
   * converted INTO user space against the element's measured width every time
   * that width changes. `--text-min` stays the single definition of the floor;
   * this only expresses it in the units the overlay draws in.
   */
  #syncBadgeScale() {
    const rect = this.getBoundingClientRect ? this.getBoundingClientRect() : null;
    const width = rect && rect.width > 0 ? rect.width : 0;
    if (width <= 0) return;
    const floorPx = phPx(this, '--text-min', TEXT_MIN_FALLBACK_PX);
    this.style.setProperty('--svg-badge-size',
      (SCOPE_VIEWBOX * floorPx / width).toFixed(2) + 'px');
  }

  set state(val) {
    this.#state = val;
    this.#render();
  }

  get state() { return this.#state; }

  #render() {
    const s = this.#state || {};
    if (this.innerRadar) {
      this.innerRadar.state = {
        blips: s.blips || [],
        range: s.range,
        ship_heading: s.ship_heading,
        config: s.config || {},
        target_uuid: s.target_uuid || null,
      };
    }
    this.#renderOverlays(s);

    // The wire's field names are unpacked here and the readings handed on, so
    // the three corner readouts are rendered by one shared fragment rather than
    // by a third copy of the same twenty lines.
    updateScopeChrome(this.shadowRoot, {
      x: s.x, z: s.z, headingDeg: s.ship_heading, speed: s.speed,
    });
  }

  #renderOverlays(s) {
    const cx = 50, cy = 50, r = 46;
    this.#renderArcGroup(s.phaser_arcs || [], 'phaser-arcs', cx, cy, r, 'var(--loaded)', 0.3);
    this.#renderArcGroup(s.torpedo_arcs || [], 'torpedo-arcs', cx, cy, r, 'var(--gold-bright)', 0.25);
    this.#renderHighlight(s, cx, cy, r);
    // The keyboard cursor follows the same blip set; drop it if its contact
    // left the scope so a stale ring never hangs where nothing is (issue #1170).
    if (this.#cursorUuid && !this.#selectableBlips().some((b) => b.uuid === this.#cursorUuid)) {
      this.#cursorUuid = null;
    }
    this.#renderCursor(cx, cy, r);
    this.#renderTorpedoBadges(s, cx, cy, r);
  }

  /**
   * The keyboard target cursor (issue #1170): a dashed ring, in the focus-ring
   * token colour, around the contact the arrow keys have moved to. Deliberately
   * distinct from the solid server-side selection highlight and the fire-red
   * lock ring — this is "where the keyboard is", the scope's own focus
   * indicator, so it uses the same `--focus-ring` token the control family
   * draws its outline from.
   */
  #renderCursor(cx = 50, cy = 50, r = 46) {
    const g = this.shadowRoot.getElementById('keyboard-cursor');
    if (!g) return;
    const uuid = this.#cursorUuid;
    const blip = uuid ? ((this.#state && this.#state.blips) || []).find((b) => b.uuid === uuid) : null;
    if (!blip) {
      while (g.firstChild) g.removeChild(g.firstChild);
      return;
    }
    let ring = g.firstChild;
    if (!ring) {
      ring = document.createElementNS('http://www.w3.org/2000/svg', 'circle');
      ring.setAttribute('fill', phColor(this, 'none'));
      ring.setAttribute('stroke', phColor(this, 'var(--focus-ring)'));
      ring.setAttribute('stroke-width', '1.5');
      ring.setAttribute('stroke-dasharray', '3 2');
      g.appendChild(ring);
    }
    const bx = cx + (blip.radar_x != null ? blip.radar_x : 0) * r;
    const by = cy - (blip.radar_y != null ? blip.radar_y : 0) * r;
    ring.setAttribute('cx', bx.toFixed(1));
    ring.setAttribute('cy', by.toFixed(1));
    ring.setAttribute('r', '6.5');
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
      // Divided through by the group's cap, so one arc alone still paints at
      // the alpha it was authored with and only a STACK is pulled back. See
      // `applyArcCompositeCap`.
      path.setAttribute('fill-opacity',
        String(cappedArcAlpha(a.opacity ?? defaultOpacity)));
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

phDefine('ph-tactical-radar', PhTacticalRadar);
