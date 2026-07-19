import { t } from '../../gui/strings.js';
import { describe, it, expect } from 'vitest';
import { JSDOM } from 'jsdom';
import {
  RED_ALERT_KIND,
  RED_ALERT_SYSTEM_ID,
  SYSTEM_REGISTRY,
  renderRedAlertFragment,
} from '../../gui/system-registry.js';

describe('SYSTEM_REGISTRY', () => {
  it('registers Red Alert as the first coarse system fragment', () => {
    expect(SYSTEM_REGISTRY[RED_ALERT_KIND]).toMatchObject({
      kind: RED_ALERT_KIND,
      systemId: RED_ALERT_SYSTEM_ID,
      station: 'captain',
      fragmentId: 'red-alert-btn',
    });
    expect(typeof SYSTEM_REGISTRY[RED_ALERT_KIND].render).toBe('function');
  });

  it('is frozen', () => {
    expect(Object.isFrozen(SYSTEM_REGISTRY)).toBe(true);
    expect(Object.isFrozen(SYSTEM_REGISTRY[RED_ALERT_KIND])).toBe(true);
  });
});

describe('renderRedAlertFragment', () => {
  function doc() {
    const dom = new JSDOM(`
      <button id="red-alert-btn" type="button">
        <span id="red-alert-auto-badge" hidden></span>
      </button>
    `);
    return dom.window.document;
  }

  it('renders human-run Red Alert as enabled', () => {
    const root = doc();

    renderRedAlertFragment(root, {
      red_alert_system_id: 'red-alert',
      red_alert_auto: false,
    });

    const btn = root.querySelector('#red-alert-btn');
    const badge = root.querySelector('#red-alert-auto-badge');
    expect(btn.dataset.systemId).toBe('red-alert');
    expect(btn.dataset.auto).toBe('false');
    expect(btn.disabled).toBe(false);
    expect(btn.classList.contains('readonly')).toBe(false);
    expect(badge.hidden).toBe(true);
  });

  it('renders AI-run Red Alert as read-only with AUTO badge', () => {
    const root = doc();

    renderRedAlertFragment(root, {
      red_alert_system_id: 'red-alert',
      red_alert_auto: true,
    });

    const btn = root.querySelector('#red-alert-btn');
    const badge = root.querySelector('#red-alert-auto-badge');
    expect(btn.dataset.systemId).toBe('red-alert');
    expect(btn.dataset.auto).toBe('true');
    expect(btn.disabled).toBe(true);
    expect(btn.classList.contains('readonly')).toBe(true);
    expect(badge.hidden).toBe(false);
    expect(badge.textContent).toBe(t('console.common.auto'));
  });
});
