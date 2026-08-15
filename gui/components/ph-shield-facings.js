// strings-boot first: its top-level await delays this module's evaluation —
// and therefore this element's registration and upgrade — until the string
// table is loaded, so the constructor's template t() calls never see an
// empty table. No-op in Node tests (setup-strings.js loads the table there).
import '../strings-boot.js';
import { t } from '../strings.js';
import { phAdoptConsoleStyles } from './ph-console-styles.js';

export class PhShieldFacings extends HTMLElement {
  #state = null;
  #facingGs = new Map();
  #emptyEl = null;
  #svgEl = null;
  #arcsGroup = null;
  #autoHintEl = null;
  #autoHintTimer = null;

  constructor() {
    super();
    this.attachShadow({ mode: 'open' });
    // Every component adopts the shared control family (module 1 of PRD
    // #1023): custom properties cross a shadow boundary, class rules do not.
    phAdoptConsoleStyles(this.shadowRoot);
    const tpl = document.createElement('template');
    tpl.innerHTML = `
  <style>
    :host { display: flex; flex-direction: column; gap: 0.5rem; font-family: 'JetBrains Mono', monospace; color: var(--ink); }
    :host * { box-sizing: border-box; }
    .header { display: flex; justify-content: space-between; align-items: center; font-size: 0.75rem; letter-spacing: 0.2em; color: var(--ink-dim); text-transform: uppercase; }
    .auto-badge { font-size: 0.55rem; color: var(--reloading); border: 1px solid var(--reloading); padding: 0.05rem 0.3rem; letter-spacing: 0.2em; }
    .arc-container { position: relative; display: flex; justify-content: center; align-items: center; padding: 0.5rem 0; }
    svg { width: 100%; max-width: 200px; height: auto; overflow: visible; }
    .arc-path { cursor: pointer; transition: opacity 0.2s, filter 0.2s; }
    .arc-path:hover, .arc-path.hover { filter: brightness(1.3); }
    .arc-path.focused { filter: brightness(1.5) drop-shadow(0 0 4px var(--loaded)); }
    .arc-path.down { opacity: 0.3; cursor: default; }
    /* Brief acknowledgment flash on any arc press (#1009) — retriggered via the
       classList remove/reflow/add trick in #flashArc, matching the rejection
       flash in ph-comms-current-message.js. */
    .arc-path.press-flash { animation: shield-arc-flash 0.6s ease; }
    @keyframes shield-arc-flash {
      0%   { filter: brightness(2.4); }
      100% { filter: brightness(1); }
    }
    /* Enlarged, invisible touch target layered above the visual wedge so a
       press doesn't need pixel-perfect accuracy (#1009). Sole click listener;
       hover is mirrored onto .arc-path below so the :hover rule above still
       fires even though the pointer never directly enters the visual path. */
    .hit-path { fill: transparent; cursor: pointer; pointer-events: all; }
    .hit-path.down { cursor: default; }
    .hp-fill, .hp-text { pointer-events: none; }
    .facing-label { font-size: 0.55rem; fill: var(--ink-dim); text-anchor: middle; pointer-events: none; }
    .facing-label.focused-label { fill: var(--ink); font-weight: 600; }
    .empty { font-size: 0.65rem; color: var(--ink-dim); text-align: center; padding: 0.75rem 0; letter-spacing: 0.2em; }
    /* AUTO-mode press feedback (#1009): a press on an arc while facing
       selection is unstaffed (auto) does nothing server-side, so this toast
       tells the player why instead of the press silently going nowhere. */
    .auto-hint {
      position: absolute; top: 50%; left: 50%; transform: translate(-50%, -50%);
      max-width: 85%; font-size: 0.5rem; line-height: 1.3; letter-spacing: 0.12em;
      text-transform: uppercase; text-align: center; color: var(--reloading);
      background: rgba(5,8,24,0.92); border: 1px solid var(--reloading);
      padding: 0.3rem 0.5rem; pointer-events: none; opacity: 0;
      transition: opacity 0.2s ease;
    }
    .auto-hint.show { opacity: 1; }
  </style>
  <div class="header">
    <span>${t('component.shield_facings.title')}</span>
    <span class="auto-badge" id="auto-badge" style="display:none">${t('console.common.auto')}</span>
  </div>
  <div class="arc-container" id="arc-container">
    <div class="empty" id="empty-placeholder">${t('component.shield_facings.empty')}</div>
    <svg viewBox="0 0 200 200" xmlns="http://www.w3.org/2000/svg" id="facing-svg" style="display:none"><g id="facing-arcs"></g></svg>
    <div class="auto-hint" id="auto-hint" role="status"></div>
  </div>
`;
    this.shadowRoot.appendChild(tpl.content.cloneNode(true));
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
    if (!this.#autoHintEl) this.#autoHintEl = this.shadowRoot.getElementById('auto-hint');

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
        // .hit-path is last (topmost in paint order) so it — not the visual
        // .arc-path underneath — receives every press, including presses that
        // land in its padding beyond the visual wedge (#1009).
        g.innerHTML = '<path class="arc-path"/><path class="hp-fill" stroke="none"/><text class="facing-label"/><text class="hp-text" text-anchor="middle" font-size="0.5rem"/><path class="hit-path"/>';
        const outline = g.children[0];
        const hitPath = g.children[4];
        hitPath.addEventListener('mouseenter', () => outline.classList.add('hover'));
        hitPath.addEventListener('mouseleave', () => outline.classList.remove('hover'));
        hitPath.addEventListener('click', () => {
          const cur = this.#state || {};
          // Every press gets a visible acknowledgment — previously an auto-mode
          // press was silently swallowed here with no feedback at all (#1009).
          this.#flashArc(outline);
          if (cur.auto) {
            this.#showAutoHint();
            return;
          }
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
      // A full render tick rewrites the class attribute wholesale below, which
      // would otherwise cut a still-playing press-flash short (or drop a live
      // hover) on the next state update — preserve both across the rewrite.
      const keepHover = outline.classList.contains('hover');
      const keepFlash = outline.classList.contains('press-flash');
      outline.setAttribute('d', outer);
      outline.setAttribute('fill', fillColor);
      outline.setAttribute('opacity', opacity);
      outline.setAttribute('stroke', isFocused ? '#4ec870' : '#282c38');
      outline.setAttribute('stroke-width', isFocused ? '2' : '1');
      outline.setAttribute('data-facing-id', id);
      outline.setAttribute('class', 'arc-path'
        + (isFocused ? ' focused' : '')
        + (!online ? ' down' : '')
        + (keepHover ? ' hover' : '')
        + (keepFlash ? ' press-flash' : ''));

      // Enlarged touch target: same wedge, padded outward/inward so a press
      // doesn't need pixel-perfect accuracy on the (fairly thin) visual
      // stroke (#1009). Radial padding only — facings tile the circle
      // contiguously with no gap between neighbours, so any angular pad
      // here would overlap the next facing's own wedge, and since later
      // <g> siblings paint on top, a press near a shared edge would fire
      // the neighbour's command instead of this one. Angular span stays
      // exactly [a0, a1] (already the full circle when n === 1).
      const hitOuterR = r + 8;
      const hitInnerR = Math.max(0, ir - 10);
      const hx0 = cx + hitOuterR * Math.cos(a0), hy0 = cy + hitOuterR * Math.sin(a0);
      const hx1 = cx + hitOuterR * Math.cos(a1), hy1 = cy + hitOuterR * Math.sin(a1);
      const hxi0 = cx + hitInnerR * Math.cos(a0), hyi0 = cy + hitInnerR * Math.sin(a0);
      const hxi1 = cx + hitInnerR * Math.cos(a1), hyi1 = cy + hitInnerR * Math.sin(a1);
      const hitPath = g.children[4];
      hitPath.setAttribute(
        'd',
        `M ${hx0} ${hy0} A ${hitOuterR} ${hitOuterR} 0 ${largeArc} 1 ${hx1} ${hy1} L ${hxi1} ${hyi1} A ${hitInnerR} ${hitInnerR} 0 ${largeArc} 0 ${hxi0} ${hyi0} Z`,
      );
      hitPath.setAttribute('data-facing-id', id);
      hitPath.setAttribute('class', 'hit-path' + (!online ? ' down' : ''));

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
      const hpLabel = !online ? t('component.shield_facings.off') : Math.round(pct * 100) + '%';
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

  /**
   * Briefly flash an arc's outline to acknowledge a press, regardless of
   * whether it went on to send a command (#1009). Uses the classList
   * remove/reflow/add trick (see ph-comms-current-message.js's rejection
   * flash) so a repeat press on an already-flashing arc restarts the
   * animation instead of being a no-op.
   * @param {SVGElement} outline
   */
  #flashArc(outline) {
    outline.classList.remove('press-flash');
    void outline.offsetWidth;
    outline.classList.add('press-flash');
  }

  /**
   * Show the "facing is on auto" hint after a press that could not send a
   * command because the station is unstaffed (#1009). Debounces like
   * ph-navigation-map's toast: a fresh press restarts the visible timer.
   */
  #showAutoHint() {
    if (!this.#autoHintEl) return;
    this.#autoHintEl.textContent = t('component.shield_facings.auto_hint');
    this.#autoHintEl.classList.add('show');
    clearTimeout(this.#autoHintTimer);
    this.#autoHintTimer = setTimeout(() => {
      this.#autoHintEl.classList.remove('show');
    }, 1800);
  }
}

if (typeof window !== 'undefined' && !customElements.get('ph-shield-facings')) {
  customElements.define('ph-shield-facings', PhShieldFacings);
}
