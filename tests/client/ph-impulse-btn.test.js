// @vitest-environment jsdom
import { t } from '../../gui/strings.js';
import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import '../../gui/components/ph-impulse-btn.js';
import {
  HELM_ACTION_CONTEXT,
  HELM_IMPULSE_ACTION_ID,
} from '../../gui/stations/helm-actions.js';

const IMPULSE_SOURCE = String(customElements.get('ph-impulse-btn'));

function setup(opts) {
  const activateSemanticAction = opts && opts.activateSemanticAction;
  if (activateSemanticAction) {
    window.activateSemanticAction = activateSemanticAction;
  }
  document.body.innerHTML = '<ph-impulse-btn id="test-el"></ph-impulse-btn>';
  const el = document.getElementById('test-el');
  return { el };
}

describe('PhImpulseBtn', () => {
  beforeEach(() => {
    document.body.innerHTML = '';
    delete window.activateSemanticAction;
  });

  afterEach(() => {
    document.body.innerHTML = '';
    delete window.activateSemanticAction;
  });

  it('is defined and registered as a custom element', () => {
    expect(customElements.get('ph-impulse-btn')).toBeDefined();
  });

  it('leaves Ctrl and gamepad matching to the parent semantic runtime', () => {
    expect(IMPULSE_SOURCE).not.toContain('observeGamepadButton');
    expect(IMPULSE_SOURCE).not.toContain("document.addEventListener('keydown'");
  });

  it('creates a shadow root', () => {
    const { el } = setup();
    expect(el.shadowRoot).toBeDefined();
  });

  it('renders ready state with IMPULSE text and enabled button', () => {
    const { el } = setup();
    el.state = { state: 'ready', charge_pct: 0, system_id: 'helm-impulse', auto: false };
    const btn = el.shadowRoot.getElementById('btn');
    expect(btn.textContent.trim()).toBe(t('component.impulse.ready'));
    expect(btn.className).toContain('ready');
    expect(btn.disabled).toBe(false);
  });

  it('renders charging state as a tappable CANCEL button showing percentage', () => {
    const { el } = setup();
    el.state = { state: 'charging', charge_pct: 67, system_id: 'helm-impulse', auto: false };
    const btn = el.shadowRoot.getElementById('btn');
    // Pressing IMPULSE again while charging cancels it, so the button stays
    // enabled and reads CANCEL rather than being an inert CHARGING label.
    expect(btn.textContent.trim()).toBe(t('component.impulse.cancel', { pct: 67 }));
    expect(btn.className).toContain('charging');
    expect(btn.disabled).toBe(false);
  });

  it('disables the charging button under AUTO so the operator cannot cancel', () => {
    const { el } = setup();
    el.state = { state: 'charging', charge_pct: 67, system_id: 'helm-impulse', auto: true };
    const btn = el.shadowRoot.getElementById('btn');
    expect(btn.disabled).toBe(true);
  });

  it('renders charging state and fills the button itself proportionally', () => {
    const { el } = setup();
    el.state = { state: 'charging', charge_pct: 42, system_id: 'helm-impulse', auto: false };
    const btn = el.shadowRoot.getElementById('btn');
    expect(btn.style.getPropertyValue('--charge')).toBe('0.42');
  });

  it('renders cooldown state with COOLDOWN text and disabled button', () => {
    const { el } = setup();
    el.state = { state: 'cooldown', charge_pct: 0, system_id: 'helm-impulse', auto: false };
    const btn = el.shadowRoot.getElementById('btn');
    expect(btn.textContent.trim()).toBe(t('console.common.cooldown'));
    expect(btn.className).toContain('cooldown');
    expect(btn.disabled).toBe(true);
  });

  it('shows AUTO badge and disables button when auto=true', () => {
    const { el } = setup();
    el.state = { state: 'ready', charge_pct: 0, system_id: 'helm-impulse', auto: true };
    const badge = el.shadowRoot.getElementById('auto-badge');
    expect(badge.style.display).not.toBe('none');
    expect(badge.textContent.trim()).toBe(t('console.common.auto'));
    const btn = el.shadowRoot.getElementById('btn');
    expect(btn.disabled).toBe(true);
  });

  it('clicking when ready activates the shared impulse identity', () => {
    const activateSemanticAction = vi.fn();
    const { el } = setup({ activateSemanticAction });
    el.state = { state: 'ready', charge_pct: 0, system_id: 'helm-impulse', auto: false };
    const btn = el.shadowRoot.getElementById('btn');
    btn.click();
    expect(activateSemanticAction).toHaveBeenCalledOnce();
    expect(activateSemanticAction).toHaveBeenCalledWith(
      HELM_IMPULSE_ACTION_ID, { context: HELM_ACTION_CONTEXT, source: 'control' },
    );
  });

  it('clicking when charging retains the same semantic identity', () => {
    const activateSemanticAction = vi.fn();
    const { el } = setup({ activateSemanticAction });
    el.state = { state: 'charging', charge_pct: 50, system_id: 'helm-impulse', auto: false };
    const btn = el.shadowRoot.getElementById('btn');
    btn.click();
    expect(activateSemanticAction).toHaveBeenCalledWith(
      HELM_IMPULSE_ACTION_ID, { context: HELM_ACTION_CONTEXT, source: 'control' },
    );
  });

  it('resets the charge fill to 0 when not charging', () => {
    const { el } = setup();
    el.state = { state: 'ready', charge_pct: 0, system_id: 'helm-impulse', auto: false };
    const btn = el.shadowRoot.getElementById('btn');
    expect(btn.style.getPropertyValue('--charge')).toBe('0');
  });
});
