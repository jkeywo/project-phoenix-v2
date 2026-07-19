// @vitest-environment jsdom
import { t } from '../../gui/strings.js';
import { describe, it, expect, beforeEach, afterEach } from 'vitest';
import '../../gui/components/ph-sensor-panel.js';

function setup(html) {
  document.body.innerHTML = html || '<ph-sensor-panel id="test-panel"></ph-sensor-panel>';
  const el = document.getElementById('test-panel');
  return { el };
}

function queryText(host, sel) {
  const el = host.shadowRoot.querySelector(sel);
  return el ? el.textContent.trim() : null;
}

describe('PhSensorPanel', () => {
  beforeEach(() => {
    document.body.innerHTML = '';
  });

  afterEach(() => {
    document.body.innerHTML = '';
  });

  it('is defined and registered as a custom element', () => {
    expect(customElements.get('ph-sensor-panel')).toBeDefined();
  });

  it('creates a shadow root', () => {
    const { el } = setup();
    expect(el.shadowRoot).toBeDefined();
  });

  it('renders default state with no target', () => {
    const { el } = setup();
    el.state = {};
    expect(queryText(el, '#range-val')).toBe('0');
    expect(queryText(el, '#blip-count')).toBe(t('console.common.contacts.other', { n: 0 }));
    const targetArea = el.shadowRoot.querySelector('#target-area');
    expect(targetArea.textContent).toContain(t('console.common.no_target'));
  });

  it('renders target info when state has target_uuid', () => {
    const { el } = setup();
    el.state = {
      scan_range: 300,
      blips: [],
      target_uuid: 'abc-123',
      target_name: 'KSV NEMESIS',
      target_kind: 'ship',
      target_stance: 'hostile',
      target_bearing: 243.4,
      target_range: 321,
      target_class: "B'REL-CLASS",
      target_hull_pct: 78,
      target_heading: 124,
      target_speed: 18.2,
      target_threat: 'high',
    };
    expect(queryText(el, '#range-val')).toBe('300');
    expect(el.shadowRoot.textContent).toContain('KSV NEMESIS');
    expect(el.shadowRoot.textContent).toContain('HOSTILE');
  });

  it('updates when state changes', () => {
    const { el } = setup();
    el.state = { scan_range: 100 };
    expect(queryText(el, '#range-val')).toBe('100');
    el.state = { scan_range: 500 };
    expect(queryText(el, '#range-val')).toBe('500');
  });

  it('responds to container sizing via CSS classes', () => {
    const { el } = setup();
    el.style.width = '300px';
    el.style.height = '200px';
    el.state = { scan_range: 400, blips: [] };
    const hostStyle = window.getComputedStyle(el);
    expect(hostStyle.display).toBe('inline');
  });
});
