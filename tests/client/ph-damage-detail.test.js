// @vitest-environment jsdom
import { describe, it, expect, beforeEach, afterEach } from 'vitest';
import '../../gui/ph-damage-detail.js';

function setup(html) {
  document.body.innerHTML = html || '<ph-damage-detail id="test-panel"></ph-damage-detail>';
  const el = document.getElementById('test-panel');
  return { el };
}

describe('PhDamageDetail', () => {
  beforeEach(() => {
    document.body.innerHTML = '';
  });

  afterEach(() => {
    document.body.innerHTML = '';
  });

  it('is defined and registered as a custom element', () => {
    expect(customElements.get('ph-damage-detail')).toBeDefined();
  });

  it('creates a shadow root', () => {
    const { el } = setup();
    expect(el.shadowRoot).toBeDefined();
  });

  it('renders correct number of rows for entries', () => {
    const { el } = setup();
    el.data = {
      entries: [
        { display_name: 'Phaser Bank A', current: 200, max_hp: 200, tier: 3, debuff_magnitude: 0.0 },
        { display_name: 'Torpedo Launcher', current: 75, max_hp: 150, tier: 2, debuff_magnitude: 0.0 },
        { display_name: 'Deflector Array', current: 100, max_hp: 100, tier: 1, debuff_magnitude: 0.0 },
      ],
    };
    const rows = el.shadowRoot.querySelectorAll('.row');
    expect(rows.length).toBe(3);
  });

  it('renders display_name and tier badge for each entry', () => {
    const { el } = setup();
    el.data = {
      entries: [
        { display_name: 'Phaser Bank A', current: 200, max_hp: 200, tier: 3, debuff_magnitude: 0.0 },
      ],
    };
    expect(el.shadowRoot.textContent).toContain('Phaser Bank A');
    expect(el.shadowRoot.textContent).toContain('T3');
  });

  it('a destroyed system (current === 0) renders DESTROYED text', () => {
    const { el } = setup();
    el.data = {
      entries: [
        { display_name: 'Torpedo Launcher', current: 0, max_hp: 150, tier: 2, debuff_magnitude: 0.5 },
      ],
    };
    expect(el.shadowRoot.textContent).toContain('DESTROYED');
    const row = el.shadowRoot.querySelector('.row');
    expect(row.className).toContain('destroyed');
  });

  it('a non-destroyed system does not render DESTROYED text', () => {
    const { el } = setup();
    el.data = {
      entries: [
        { display_name: 'Phaser Bank A', current: 200, max_hp: 200, tier: 3, debuff_magnitude: 0.0 },
      ],
    };
    expect(el.shadowRoot.textContent).not.toContain('DESTROYED');
    const row = el.shadowRoot.querySelector('.row');
    expect(row.className).not.toContain('destroyed');
  });

  it('missing data renders empty list without error', () => {
    const { el } = setup();
    el.data = null;
    const list = el.shadowRoot.getElementById('list');
    expect(list.innerHTML).toBe('');
  });

  it('null data set via property renders without error', () => {
    const { el } = setup();
    expect(() => { el.data = null; }).not.toThrow();
    const list = el.shadowRoot.getElementById('list');
    expect(list.innerHTML).toBe('');
  });

  it('empty entries array renders empty list', () => {
    const { el } = setup();
    el.data = { entries: [] };
    const rows = el.shadowRoot.querySelectorAll('.row');
    expect(rows.length).toBe(0);
  });

  it('applies correct colour class for healthy system (pct >= 0.75)', () => {
    const { el } = setup();
    el.data = {
      entries: [
        { display_name: 'Healthy System', current: 200, max_hp: 200, tier: 1, debuff_magnitude: 0.0 },
      ],
    };
    const fill = el.shadowRoot.querySelector('.fill');
    expect(fill.className).toBe('fill');
  });

  it('applies warn class for amber system (0.4 <= pct < 0.75)', () => {
    const { el } = setup();
    el.data = {
      entries: [
        { display_name: 'Damaged System', current: 60, max_hp: 100, tier: 2, debuff_magnitude: 0.0 },
      ],
    };
    const fill = el.shadowRoot.querySelector('.fill');
    expect(fill.className).toBe('fill warn');
  });

  it('applies crit class for critically damaged system (pct < 0.4)', () => {
    const { el } = setup();
    el.data = {
      entries: [
        { display_name: 'Critical System', current: 30, max_hp: 100, tier: 2, debuff_magnitude: 0.0 },
      ],
    };
    const fill = el.shadowRoot.querySelector('.fill');
    expect(fill.className).toBe('fill crit');
  });

  it('updates correctly when data changes', () => {
    const { el } = setup();
    el.data = {
      entries: [
        { display_name: 'System A', current: 100, max_hp: 100, tier: 1, debuff_magnitude: 0.0 },
      ],
    };
    expect(el.shadowRoot.querySelectorAll('.row').length).toBe(1);
    el.data = {
      entries: [
        { display_name: 'System A', current: 100, max_hp: 100, tier: 1, debuff_magnitude: 0.0 },
        { display_name: 'System B', current: 0, max_hp: 80, tier: 2, debuff_magnitude: 0.3 },
      ],
    };
    expect(el.shadowRoot.querySelectorAll('.row').length).toBe(2);
    expect(el.shadowRoot.textContent).toContain('DESTROYED');
  });
});
