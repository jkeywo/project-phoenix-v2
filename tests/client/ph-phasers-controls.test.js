// @vitest-environment jsdom
import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import '../../gui/components/ph-phasers-controls.js';

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
    expect(queryText(el, '#banks')).toBe('NO PHASER BANKS');
  });

  it('renders NO PHASER BANKS placeholder with null state', () => {
    const { el } = setup();
    el.state = null;
    expect(queryText(el, '#banks')).toBe('NO PHASER BANKS');
  });

  it('renders a phaser bank with label text', () => {
    const { el } = setup();
    el.state = {
      banks: [{ id: 'fore', label: 'Fore', state: 'ready', cooldown_pct: 0, auto: false }],
      target_valid: true,
    };
    expect(queryText(el, '.lbl')).toBe('Fore');
  });

  it('renders cooldown bar at correct width', () => {
    const { el } = setup();
    el.state = {
      banks: [{ id: 'aft', label: 'Aft', state: 'cooling', cooldown_pct: 0.5, auto: false }],
      target_valid: true,
    };
    const fill = el.shadowRoot.querySelector('.cooldown-fill');
    expect(fill.style.width).toBe('50%');
    expect(fill.classList.contains('cooling')).toBe(true);
  });

  it('fire button is disabled when cooldown > 0', () => {
    const { el } = setup();
    el.state = {
      banks: [{ id: 'fore', label: 'Fore', state: 'cooling', cooldown_pct: 0.3, auto: false }],
      target_valid: true,
    };
    const btn = el.shadowRoot.querySelector('.fire-btn');
    expect(btn.disabled).toBe(true);
  });

  it('fire button is disabled when target is invalid', () => {
    const { el } = setup();
    el.state = {
      banks: [{ id: 'fore', label: 'Fore', state: 'ready', cooldown_pct: 0, auto: false }],
      target_valid: false,
    };
    const btn = el.shadowRoot.querySelector('.fire-btn');
    expect(btn.disabled).toBe(true);
  });

  it('fire button is disabled when bank is auto', () => {
    const { el } = setup();
    el.state = {
      banks: [{ id: 'fore', label: 'Fore', state: 'ready', cooldown_pct: 0, auto: true }],
      target_valid: true,
    };
    const btn = el.shadowRoot.querySelector('.fire-btn');
    expect(btn.disabled).toBe(true);
  });

  it('fire button is enabled when ready, target valid, not auto', () => {
    const { el } = setup();
    el.state = {
      banks: [{ id: 'fore', label: 'Fore', state: 'ready', cooldown_pct: 0, auto: false }],
      target_valid: true,
    };
    const btn = el.shadowRoot.querySelector('.fire-btn');
    expect(btn.disabled).toBe(false);
  });

  it('shows AUTO badge when bank is auto', () => {
    const { el } = setup();
    el.state = {
      banks: [{ id: 'fore', label: 'Fore', state: 'ready', cooldown_pct: 0, auto: true }],
      target_valid: true,
    };
    const badge = el.shadowRoot.querySelector('.auto-badge');
    expect(badge.style.display).not.toBe('none');
    expect(badge.textContent.trim()).toBe('AUTO');
  });

  it('hides AUTO badge when bank is not auto', () => {
    const { el } = setup();
    el.state = {
      banks: [{ id: 'fore', label: 'Fore', state: 'ready', cooldown_pct: 0, auto: false }],
      target_valid: true,
    };
    const badge = el.shadowRoot.querySelector('.auto-badge');
    expect(badge.style.display).toBe('none');
  });

  it('clicking fire button dispatches fire_phaser action', () => {
    const sendAction = vi.fn();
    const { el } = setup({ sendAction });
    el.state = {
      banks: [{ id: 'fore', label: 'Fore', state: 'ready', cooldown_pct: 0, auto: false }],
      target_valid: true,
    };
    const btn = el.shadowRoot.querySelector('.fire-btn');
    btn.click();
    expect(sendAction).toHaveBeenCalledTimes(1);
    expect(sendAction).toHaveBeenCalledWith('fire_phaser', { bank: 'fore' });
  });

  it('reconciles banks by id, reusing existing DOM elements', () => {
    const { el } = setup();
    el.state = {
      banks: [
        { id: 'fore', label: 'Fore', state: 'ready', cooldown_pct: 0, auto: false },
        { id: 'aft', label: 'Aft', state: 'cooling', cooldown_pct: 0.5, auto: false },
      ],
      target_valid: true,
    };
    const rows1 = el.shadowRoot.querySelectorAll('.bank-row');
    expect(rows1.length).toBe(2);
    expect(rows1[0].dataset.id).toBe('fore');
    expect(rows1[1].dataset.id).toBe('aft');

    el.state = {
      banks: [
        { id: 'aft', label: 'Aft', state: 'cooling', cooldown_pct: 0.2, auto: false },
      ],
      target_valid: true,
    };
    const rows2 = el.shadowRoot.querySelectorAll('.bank-row');
    expect(rows2.length).toBe(1);
    expect(rows2[0].dataset.id).toBe('aft');
  });

  it('removes surplus rows when banks are removed', () => {
    const { el } = setup();
    el.state = {
      banks: [
        { id: 'fore', label: 'Fore', state: 'ready', cooldown_pct: 0, auto: false },
        { id: 'aft', label: 'Aft', state: 'cooling', cooldown_pct: 0.5, auto: false },
      ],
      target_valid: true,
    };
    expect(el.shadowRoot.querySelectorAll('.bank-row').length).toBe(2);

    el.state = {
      banks: [{ id: 'fore', label: 'Fore', state: 'ready', cooldown_pct: 0, auto: false }],
      target_valid: true,
    };
    expect(el.shadowRoot.querySelectorAll('.bank-row').length).toBe(1);
  });
});
