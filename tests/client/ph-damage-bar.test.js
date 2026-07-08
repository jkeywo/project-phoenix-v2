// @vitest-environment jsdom
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
});
