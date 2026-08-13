// @vitest-environment jsdom
import { t } from '../../gui/strings.js';
import { describe, it, expect, beforeEach, afterEach } from 'vitest';
import '../../gui/components/ph-damage-bar.js';

function setup(html) {
  document.body.innerHTML = html || '<ph-damage-bar id="test-panel"></ph-damage-bar>';
  const el = document.getElementById('test-panel');
  return { el };
}

function queryText(host, sel) {
  const el = host.shadowRoot.querySelector(sel);
  return el ? el.textContent.trim() : null;
}

describe('PhDamageBar', () => {
  beforeEach(() => {
    document.body.innerHTML = '';
  });

  afterEach(() => {
    document.body.innerHTML = '';
  });

  it('is defined and registered as a custom element', () => {
    expect(customElements.get('ph-damage-bar')).toBeDefined();
  });

  it('creates a shadow root', () => {
    const { el } = setup();
    expect(el.shadowRoot).toBeDefined();
  });

  it('renders with full data: bar is present and label shows totalCurrent / totalMax', () => {
    const { el } = setup();
    el.state = { pct: 0.85, damagePct: 0.15, totalCurrent: 850, totalMax: 1000 };
    const fill = el.shadowRoot.getElementById('bar-fill');
    expect(fill).toBeDefined();
    expect(fill.style.width).toBe('85%');
    expect(queryText(el, '#bar-label')).toBe('850 / 1000');
  });

  it('pct=1.0 renders green (no warn/crit class)', () => {
    const { el } = setup();
    el.state = { pct: 1.0, totalCurrent: 100, totalMax: 100 };
    const fill = el.shadowRoot.getElementById('bar-fill');
    expect(fill.className).toBe('fill');
  });

  it('pct=0.6 renders amber (warn class)', () => {
    const { el } = setup();
    el.state = { pct: 0.6, totalCurrent: 600, totalMax: 1000 };
    const fill = el.shadowRoot.getElementById('bar-fill');
    expect(fill.className).toBe('fill warn');
  });

  it('pct=0.3 renders red (crit class)', () => {
    const { el } = setup();
    el.state = { pct: 0.3, totalCurrent: 300, totalMax: 1000 };
    const fill = el.shadowRoot.getElementById('bar-fill');
    expect(fill.className).toBe('fill crit');
  });

  it('missing data renders without error, bar treated as full (pct=1)', () => {
    const { el } = setup();
    el.state = null;
    const fill = el.shadowRoot.getElementById('bar-fill');
    expect(fill).toBeDefined();
    expect(fill.style.width).toBe('100%');
    expect(fill.className).toBe('fill');
  });

  it('null data set via property renders without error', () => {
    const { el } = setup();
    expect(() => { el.state = null; }).not.toThrow();
  });

  it('updates correctly when data changes', () => {
    const { el } = setup();
    el.state = { pct: 0.9, totalCurrent: 900, totalMax: 1000 };
    expect(queryText(el, '#bar-label')).toBe('900 / 1000');
    el.state = { pct: 0.2, totalCurrent: 200, totalMax: 1000 };
    expect(queryText(el, '#bar-label')).toBe('200 / 1000');
    expect(el.shadowRoot.getElementById('bar-fill').className).toBe('fill crit');
  });

  it('pct exactly at 0.75 threshold is green', () => {
    const { el } = setup();
    el.state = { pct: 0.75, totalCurrent: 75, totalMax: 100 };
    const fill = el.shadowRoot.getElementById('bar-fill');
    expect(fill.className).toBe('fill');
  });

  it('pct exactly at 0.4 threshold is amber', () => {
    const { el } = setup();
    el.state = { pct: 0.4, totalCurrent: 40, totalMax: 100 };
    const fill = el.shadowRoot.getElementById('bar-fill');
    expect(fill.className).toBe('fill warn');
  });

  // ── Issue #1014: destroyed capability reads as a loss band ─────────────────
  //
  // `destroyed` is a host-supplied ship-wide share — the fraction of total hull
  // capacity held by systems that are gone, not merely damaged. It is painted
  // at the RIGHT end of the bar (the segment lost off the top) and is entirely
  // independent of the fill/warn/crit state, which still tracks remaining hull.

  it('renders no loss band when destroyed is 0', () => {
    const { el } = setup();
    el.state = { pct: 0.8, destroyed: 0, totalCurrent: 80, totalMax: 100 };
    const lost = el.shadowRoot.getElementById('bar-lost');
    expect(lost.style.display).toBe('none');
    expect(lost.style.width).toBe('0%');
  });

  it('renders no loss band when destroyed is absent (legacy host)', () => {
    const { el } = setup();
    el.state = { pct: 0.8, totalCurrent: 80, totalMax: 100 };
    expect(el.shadowRoot.getElementById('bar-lost').style.display).toBe('none');
  });

  it('renders the loss band at the destroyed width, anchored to the right end', () => {
    const { el } = setup();
    el.state = { pct: 0.7, destroyed: 0.25, totalCurrent: 70, totalMax: 100 };
    const lost = el.shadowRoot.getElementById('bar-lost');
    expect(lost.style.display).toBe('block');
    expect(lost.style.width).toBe('25%');
    // Anchored right, so it is the top of the bar that is shown as lost.
    const css = el.shadowRoot.querySelector('style').textContent;
    expect(css).toMatch(/\.lost\s*\{[^}]*right:\s*0/);
  });

  it('the loss band coexists with each fill class without changing it', () => {
    const { el } = setup();
    const fill = el.shadowRoot.getElementById('bar-fill');
    const lost = el.shadowRoot.getElementById('bar-lost');

    el.state = { pct: 0.9, destroyed: 0.1 };
    expect(fill.className).toBe('fill');
    expect(lost.style.width).toBe('10%');

    el.state = { pct: 0.6, destroyed: 0.2 };
    expect(fill.className).toBe('fill warn');
    expect(lost.style.width).toBe('20%');

    el.state = { pct: 0.3, destroyed: 0.5 };
    expect(fill.className).toBe('fill crit');
    expect(lost.style.width).toBe('50%');
  });

  it('exact fill thresholds are unaffected by a loss band', () => {
    const { el } = setup();
    const fill = el.shadowRoot.getElementById('bar-fill');
    el.state = { pct: 0.75, destroyed: 0.25, totalCurrent: 75, totalMax: 100 };
    expect(fill.className).toBe('fill');
    el.state = { pct: 0.4, destroyed: 0.25, totalCurrent: 40, totalMax: 100 };
    expect(fill.className).toBe('fill warn');
  });

  it('clears the loss band when destroyed drops back to 0', () => {
    const { el } = setup();
    const lost = el.shadowRoot.getElementById('bar-lost');
    el.state = { pct: 0.5, destroyed: 0.3 };
    expect(lost.style.display).toBe('block');
    el.state = { pct: 0.5, destroyed: 0 };
    expect(lost.style.display).toBe('none');
    expect(lost.style.width).toBe('0%');
  });

  it('clamps an out-of-range destroyed share to the bar', () => {
    const { el } = setup();
    const lost = el.shadowRoot.getElementById('bar-lost');
    el.state = { pct: 0, destroyed: 1.5 };
    expect(lost.style.width).toBe('100%');
    el.state = { pct: 1, destroyed: -0.5 };
    expect(lost.style.display).toBe('none');
  });

  it('null state hides the loss band', () => {
    const { el } = setup();
    el.state = { pct: 0.5, destroyed: 0.3 };
    el.state = null;
    expect(el.shadowRoot.getElementById('bar-lost').style.display).toBe('none');
  });
});
