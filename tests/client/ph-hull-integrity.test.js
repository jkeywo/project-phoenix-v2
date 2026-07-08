// @vitest-environment jsdom
import { describe, it, expect, beforeEach, afterEach } from 'vitest';
import '../../gui/components/ph-hull-integrity.js';

function setup(html) {
  document.body.innerHTML = html || '<ph-hull-integrity id="test-panel"></ph-hull-integrity>';
  const el = document.getElementById('test-panel');
  return { el };
}

describe('PhHullIntegrity', () => {
  beforeEach(() => {
    document.body.innerHTML = '';
  });

  afterEach(() => {
    document.body.innerHTML = '';
  });

  it('is defined and registered as a custom element', () => {
    expect(customElements.get('ph-hull-integrity')).toBeDefined();
  });

  it('creates a shadow root', () => {
    const { el } = setup();
    expect(el.shadowRoot).toBeDefined();
  });

  it('shows placeholder when state is null', () => {
    const { el } = setup();
    const placeholder = el.shadowRoot.getElementById('placeholder');
    expect(placeholder.style.display).not.toBe('none');
    expect(el.shadowRoot.textContent).toContain('NO HULL DATA');
  });

  it('null state set via property renders without error', () => {
    const { el } = setup();
    expect(() => { el.state = null; }).not.toThrow();
  });

  it('renders full hull with new format (total_pct, systems)', () => {
    const { el } = setup();
    el.state = {
      total_pct: 1.0,
      systems: [
        { system_id: 'helm', display_name: 'Helm', current: 25.0, max_hp: 25.0 },
        { system_id: 'tactical', display_name: 'Tactical', current: 10.0, max_hp: 10.0 },
      ],
    };
    const bar = el.shadowRoot.querySelector('ph-damage-bar');
    expect(bar).toBeDefined();
    expect(bar.state.pct).toBe(1.0);
    const detail = el.shadowRoot.querySelector('ph-damage-detail');
    expect(detail).toBeDefined();
    expect(detail.state.entries.length).toBe(2);
  });

  it('renders full hull with old format (pct, totalCurrent, totalMax, entries)', () => {
    const { el } = setup();
    el.state = {
      pct: 0.85,
      totalCurrent: 850,
      totalMax: 1000,
      entries: [
        { display_name: 'Helm', current: 25.0, max_hp: 25.0 },
        { display_name: 'Tactical', current: 10.0, max_hp: 10.0 },
      ],
    };
    const bar = el.shadowRoot.querySelector('ph-damage-bar');
    expect(bar.state.pct).toBe(0.85);
    expect(bar.state.totalCurrent).toBe(850);
    const detail = el.shadowRoot.querySelector('ph-damage-detail');
    expect(detail.state.entries.length).toBe(2);
  });

  it('shows damaged hull with correct pct', () => {
    const { el } = setup();
    el.state = { total_pct: 0.45 };
    const bar = el.shadowRoot.querySelector('ph-damage-bar');
    expect(bar.state.pct).toBe(0.45);
  });

  it('shows a destroyed system', () => {
    const { el } = setup();
    el.state = {
      total_pct: 0.5,
      systems: [
        { system_id: 'helm', display_name: 'Helm', current: 0, max_hp: 25.0 },
      ],
    };
    const detail = el.shadowRoot.querySelector('ph-damage-detail');
    expect(detail.state.entries[0].current).toBe(0);
    expect(detail.shadowRoot.textContent).toContain('DESTROYED');
  });

  it('updates when state changes', () => {
    const { el } = setup();
    el.state = { total_pct: 0.9 };
    const bar = el.shadowRoot.querySelector('ph-damage-bar');
    expect(bar.state.pct).toBe(0.9);
    el.state = { total_pct: 0.3 };
    expect(bar.state.pct).toBe(0.3);
  });
});
