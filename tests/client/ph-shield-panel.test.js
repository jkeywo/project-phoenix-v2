// @vitest-environment jsdom
import { describe, it, expect, beforeEach, afterEach } from 'vitest';
import '../../gui/components/ph-shield-panel.js';

function setup(html) {
  document.body.innerHTML = html || '<ph-shield-panel id="test-panel"></ph-shield-panel>';
  const el = document.getElementById('test-panel');
  return { el };
}

function queryText(host, sel) {
  const el = host.shadowRoot.querySelector(sel);
  return el ? el.textContent.trim() : null;
}

describe('PhShieldPanel', () => {
  beforeEach(() => {
    document.body.innerHTML = '';
  });

  afterEach(() => {
    document.body.innerHTML = '';
  });

  it('is defined and registered as a custom element', () => {
    expect(customElements.get('ph-shield-panel')).toBeDefined();
  });

  it('creates a shadow root', () => {
    const { el } = setup();
    expect(el.shadowRoot).toBeDefined();
  });

  it('renders default state with full hull', () => {
    const { el } = setup();
    el.state = {};
    expect(queryText(el, '#panel-hull-val')).toBe('100%');
    expect(queryText(el, '#grid-status')).toBe('GRID NOMINAL');
    expect(queryText(el, '#panel-focus-display')).toBe('FOCUS: OMNI');
  });

  it('renders shield facings from state', () => {
    const { el } = setup();
    el.state = {
      facings: [
        { arc_id: 'fore', label: 'Fore', hp: 100, max_hp: 100, online: true },
        { arc_id: 'port', label: 'Port', hp: 50, max_hp: 100, online: true },
        { arc_id: 'aft', label: 'Aft', hp: 0, max_hp: 100, online: false },
      ],
      hull_integrity_pct: 78,
      focused_facing: 'Fore',
      grid_status: 'GRID NOMINAL',
    };
    expect(el.shadowRoot.textContent).toContain('FORE');
    expect(el.shadowRoot.textContent).toContain('PORT');
    expect(el.shadowRoot.textContent).toContain('FOCUS: Fore');
    expect(queryText(el, '#panel-hull-val')).toBe('78%');
  });

  it('renders grid OFFLINE when all shields are down', () => {
    const { el } = setup();
    el.state = {
      facings: [],
      hull_integrity_pct: 100,
      grid_status: 'GRID OFFLINE',
    };
    expect(queryText(el, '#grid-status')).toBe('GRID OFFLINE');
  });

  it('updates when state changes', () => {
    const { el } = setup();
    el.state = { hull_integrity_pct: 90 };
    expect(queryText(el, '#panel-hull-val')).toBe('90%');
    el.state = { hull_integrity_pct: 45 };
    expect(queryText(el, '#panel-hull-val')).toBe('45%');
  });

  it('responds to container sizing via CSS', () => {
    const { el } = setup();
    el.state = { hull_integrity_pct: 100 };
    const hostStyle = window.getComputedStyle(el);
    expect(hostStyle.display).toBe('inline');
  });
});
