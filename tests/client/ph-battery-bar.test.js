// @vitest-environment jsdom
import { describe, it, expect, beforeEach, afterEach } from 'vitest';
import '../../gui/components/ph-battery-bar.js';

function setup(html) {
  document.body.innerHTML = html || '<ph-battery-bar id="test-panel"></ph-battery-bar>';
  const el = document.getElementById('test-panel');
  return { el };
}

function queryText(host, sel) {
  const el = host.shadowRoot.querySelector(sel);
  return el ? el.textContent.trim() : null;
}

describe('PhBatteryBar', () => {
  beforeEach(() => {
    document.body.innerHTML = '';
  });

  afterEach(() => {
    document.body.innerHTML = '';
  });

  it('is defined and registered as a custom element', () => {
    expect(customElements.get('ph-battery-bar')).toBeDefined();
  });

  it('creates a shadow root', () => {
    const { el } = setup();
    expect(el.shadowRoot).toBeDefined();
  });

  it('renders full battery with green fill', () => {
    const { el } = setup();
    el.state = { level_pct: 100, emergency_threshold_pct: 20 };
    const fill = el.shadowRoot.getElementById('bar-fill');
    expect(fill.style.width).toBe('100%');
    expect(fill.className).toBe('fill');
    expect(queryText(el, '#bar-label')).toBe('100%');
  });

  it('renders low battery (below threshold) with red fill', () => {
    const { el } = setup();
    el.state = { level_pct: 15, emergency_threshold_pct: 20 };
    const fill = el.shadowRoot.getElementById('bar-fill');
    expect(fill.style.width).toBe('15%');
    expect(fill.className).toBe('fill red');
  });

  it('renders amber fill when near threshold (within 10pp)', () => {
    const { el } = setup();
    el.state = { level_pct: 28, emergency_threshold_pct: 20 };
    const fill = el.shadowRoot.getElementById('bar-fill');
    expect(fill.className).toBe('fill amber');
  });

  it('shows charging indicator when charging is true', () => {
    const { el } = setup();
    el.state = { level_pct: 50, charging: true, emergency_threshold_pct: 20 };
    const indicator = el.shadowRoot.getElementById('charging-indicator');
    expect(indicator.style.display).toBe('flex');
    expect(indicator.textContent.trim()).toBe('CHARGING');
  });

  it('hides charging indicator when charging is false', () => {
    const { el } = setup();
    el.state = { level_pct: 50, charging: false, emergency_threshold_pct: 20 };
    const indicator = el.shadowRoot.getElementById('charging-indicator');
    expect(indicator.style.display).toBe('none');
  });

  it('renders empty state (level_pct = 0) correctly', () => {
    const { el } = setup();
    el.state = { level_pct: 0, emergency_threshold_pct: 20 };
    const fill = el.shadowRoot.getElementById('bar-fill');
    expect(fill.style.width).toBe('0%');
    expect(fill.className).toBe('fill red');
    expect(queryText(el, '#bar-label')).toBe('0%');
  });

  it('renders emergency threshold marker at correct position', () => {
    const { el } = setup();
    el.state = { level_pct: 75, emergency_threshold_pct: 25 };
    const marker = el.shadowRoot.getElementById('threshold-marker');
    expect(marker.style.left).toBe('25%');
  });

  it('computes level_pct from charge/capacity when level_pct absent', () => {
    const { el } = setup();
    el.state = { charge: 30, capacity: 120, charging: false, emergency_threshold_pct: 20 };
    const fill = el.shadowRoot.getElementById('bar-fill');
    expect(fill.style.width).toBe('25%');
    expect(queryText(el, '#bar-label')).toBe('25%');
  });

  it('updates correctly when state changes', () => {
    const { el } = setup();
    el.state = { level_pct: 90, emergency_threshold_pct: 20 };
    let fill = el.shadowRoot.getElementById('bar-fill');
    expect(fill.style.width).toBe('90%');
    el.state = { level_pct: 10, emergency_threshold_pct: 20 };
    fill = el.shadowRoot.getElementById('bar-fill');
    expect(fill.style.width).toBe('10%');
    expect(fill.className).toBe('fill red');
  });
});
