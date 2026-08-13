// @vitest-environment jsdom
import { t } from '../../gui/strings.js';
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
    expect(el.shadowRoot.textContent).toContain(t('component.hull_integrity.no_data'));
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
    expect(detail.shadowRoot.textContent).toContain(t('console.common.destroyed'));
  });

  // ── Issue #1014: the destroyed-capability share is passed straight through ──
  //
  // The figure is ship-wide and host-supplied; `systems` is this recipient's
  // #737 projection and cannot produce it, so this component only forwards it.

  it('passes destroyed_pct through to the bar in the total_pct shape', () => {
    const { el } = setup();
    el.state = {
      total_pct: 0.6,
      destroyed_pct: 0.25,
      systems: [{ system_id: 'helm', display_name: 'Helm', current: 25.0, max_hp: 25.0 }],
    };
    const bar = el.shadowRoot.querySelector('ph-damage-bar');
    expect(bar.state.destroyed).toBe(0.25);
    expect(bar.state.pct).toBe(0.6);
  });

  it('passes destroyed_pct through to the bar in the pct shape', () => {
    const { el } = setup();
    el.state = { pct: 0.85, totalCurrent: 850, totalMax: 1000, destroyed_pct: 0.1 };
    const bar = el.shadowRoot.querySelector('ph-damage-bar');
    expect(bar.state.destroyed).toBe(0.1);
    expect(bar.state.totalMax).toBe(1000);
  });

  it('forwards a destroyed share even when no visible system is destroyed', () => {
    // The whole point: the lost system belongs to a station this client cannot
    // see, so the rows it was given look perfectly healthy.
    const { el } = setup();
    el.state = {
      total_pct: 0.5,
      destroyed_pct: 0.5,
      systems: [{ system_id: 'repair', display_name: 'Repair', current: 25.0, max_hp: 25.0 }],
    };
    const bar = el.shadowRoot.querySelector('ph-damage-bar');
    expect(bar.state.destroyed).toBe(0.5);
    expect(bar.shadowRoot.getElementById('bar-lost').style.width).toBe('50%');
  });

  it('omits destroyed when the host sent none (legacy payload)', () => {
    const { el } = setup();
    el.state = { total_pct: 0.6 };
    const bar = el.shadowRoot.querySelector('ph-damage-bar');
    expect(bar.state.destroyed).toBeUndefined();
    expect(bar.shadowRoot.getElementById('bar-lost').style.display).toBe('none');
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
