// @vitest-environment jsdom
import { t } from '../../gui/strings.js';
import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import '../../gui/components/ph-phasers-controls.js';

// The component consumes the server wire type `PhaserBankState`
// (core/messages.rs): { id, fire_ready, on_cooldown, cooldown_remaining }.
// Auto/Manual is a ship-level `phaser_mode` surfaced through the header
// toggle, not a per-bank field.

function setup(opts) {
  const sendAction = opts && opts.sendAction;
  if (sendAction) {
    window.sendAction = sendAction;
  }
  document.body.innerHTML = '<ph-phasers-controls id="test-el"></ph-phasers-controls>';
  const el = document.getElementById('test-el');
  return { el };
}

function queryText(host, sel) {
  const el = host.shadowRoot.querySelector(sel);
  return el ? el.textContent.trim() : null;
}

describe('PhPhasersControls', () => {
  beforeEach(() => {
    document.body.innerHTML = '';
    delete window.sendAction;
  });

  afterEach(() => {
    document.body.innerHTML = '';
    delete window.sendAction;
  });

  it('is defined and registered as a custom element', () => {
    expect(customElements.get('ph-phasers-controls')).toBeDefined();
  });

  it('creates a shadow root', () => {
    const { el } = setup();
    expect(el.shadowRoot).toBeDefined();
  });

  it('renders NO PHASER BANKS placeholder with empty banks', () => {
    const { el } = setup();
    el.state = { banks: [] };
    expect(queryText(el, '#banks')).toBe(t('component.phasers.empty'));
  });

  it('renders NO PHASER BANKS placeholder with null state', () => {
    const { el } = setup();
    el.state = null;
    expect(queryText(el, '#banks')).toBe(t('component.phasers.empty'));
  });

  it('renders a phaser bank with label text', () => {
    const { el } = setup();
    el.state = {
      banks: [{ id: 'fore', label: 'Fore', fire_ready: true, on_cooldown: false }],
      target_valid: true,
      mode: 'Manual',
    };
    expect(queryText(el, '.lbl')).toBe('Fore');
  });

  it('marks the cooldown bar as cooling while on cooldown', () => {
    const { el } = setup();
    el.state = {
      banks: [{ id: 'aft', label: 'Aft', fire_ready: false, on_cooldown: true }],
      target_valid: true,
      mode: 'Manual',
    };
    const fill = el.shadowRoot.querySelector('.cooldown-fill');
    expect(fill.classList.contains('cooling')).toBe(true);
  });

  it('fire button is disabled while on cooldown', () => {
    const { el } = setup();
    el.state = {
      banks: [{ id: 'fore', label: 'Fore', fire_ready: true, on_cooldown: true }],
      target_valid: true,
      mode: 'Manual',
    };
    const btn = el.shadowRoot.querySelector('.btn');
    expect(btn.disabled).toBe(true);
  });

  it('fire button is disabled when target is invalid', () => {
    const { el } = setup();
    el.state = {
      banks: [{ id: 'fore', label: 'Fore', fire_ready: true, on_cooldown: false }],
      target_valid: false,
      mode: 'Manual',
    };
    const btn = el.shadowRoot.querySelector('.btn');
    expect(btn.disabled).toBe(true);
  });

  it('fire button is disabled when the bank is not fire-ready', () => {
    const { el } = setup();
    el.state = {
      banks: [{ id: 'fore', label: 'Fore', fire_ready: false, on_cooldown: false }],
      target_valid: true,
      mode: 'Manual',
    };
    const btn = el.shadowRoot.querySelector('.btn');
    expect(btn.disabled).toBe(true);
  });

  it('fire button is disabled in Auto mode', () => {
    const { el } = setup();
    el.state = {
      banks: [{ id: 'fore', label: 'Fore', fire_ready: true, on_cooldown: false }],
      target_valid: true,
      mode: 'Auto',
    };
    const btn = el.shadowRoot.querySelector('.btn');
    expect(btn.disabled).toBe(true);
  });

  it('fire button is enabled when ready, target valid, cooldown clear, and Manual', () => {
    const { el } = setup();
    el.state = {
      banks: [{ id: 'fore', label: 'Fore', fire_ready: true, on_cooldown: false }],
      target_valid: true,
      mode: 'Manual',
    };
    const btn = el.shadowRoot.querySelector('.btn');
    expect(btn.disabled).toBe(false);
  });

  it('mode toggle shows AUTO in Auto mode and MANUAL in Manual mode', () => {
    const { el } = setup();
    el.state = { banks: [], mode: 'Auto' };
    expect(queryText(el, '#mode-toggle')).toBe(t('console.common.auto'));
    el.state = { banks: [], mode: 'Manual' };
    expect(queryText(el, '#mode-toggle')).toBe(t('component.phasers.manual'));
  });

  it('clicking the mode toggle dispatches set_phaser_mode with the flipped mode', () => {
    const sendAction = vi.fn();
    const { el } = setup({ sendAction });
    el.state = { banks: [], mode: 'Auto' };
    el.shadowRoot.querySelector('#mode-toggle').click();
    expect(sendAction).toHaveBeenCalledWith('set_phaser_mode', { mode: 'Manual' });

    el.state = { banks: [], mode: 'Manual' };
    el.shadowRoot.querySelector('#mode-toggle').click();
    expect(sendAction).toHaveBeenCalledWith('set_phaser_mode', { mode: 'Auto' });
  });

  it('clicking fire button dispatches fire_phaser action', () => {
    const sendAction = vi.fn();
    const { el } = setup({ sendAction });
    el.state = {
      banks: [{ id: 'fore', label: 'Fore', fire_ready: true, on_cooldown: false }],
      target_valid: true,
      mode: 'Manual',
    };
    const btn = el.shadowRoot.querySelector('.btn');
    btn.click();
    expect(sendAction).toHaveBeenCalledTimes(1);
    expect(sendAction).toHaveBeenCalledWith('fire_phaser', { bank: 'fore' });
  });

  it('reconciles banks by id, reusing existing DOM elements', () => {
    const { el } = setup();
    el.state = {
      banks: [
        { id: 'fore', label: 'Fore', fire_ready: true, on_cooldown: false },
        { id: 'aft', label: 'Aft', fire_ready: false, on_cooldown: true },
      ],
      target_valid: true,
      mode: 'Manual',
    };
    const rows1 = el.shadowRoot.querySelectorAll('.bank-row');
    expect(rows1.length).toBe(2);
    expect(rows1[0].dataset.id).toBe('fore');
    expect(rows1[1].dataset.id).toBe('aft');

    el.state = {
      banks: [
        { id: 'aft', label: 'Aft', fire_ready: false, on_cooldown: true },
      ],
      target_valid: true,
      mode: 'Manual',
    };
    const rows2 = el.shadowRoot.querySelectorAll('.bank-row');
    expect(rows2.length).toBe(1);
    expect(rows2[0].dataset.id).toBe('aft');
  });

  it('removes surplus rows when banks are removed', () => {
    const { el } = setup();
    el.state = {
      banks: [
        { id: 'fore', label: 'Fore', fire_ready: true, on_cooldown: false },
        { id: 'aft', label: 'Aft', fire_ready: false, on_cooldown: true },
      ],
      target_valid: true,
      mode: 'Manual',
    };
    expect(el.shadowRoot.querySelectorAll('.bank-row').length).toBe(2);

    el.state = {
      banks: [{ id: 'fore', label: 'Fore', fire_ready: true, on_cooldown: false }],
      target_valid: true,
      mode: 'Manual',
    };
    expect(el.shadowRoot.querySelectorAll('.bank-row').length).toBe(1);
  });
});
