// @vitest-environment jsdom
import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import '../../gui/components/ph-power-controls.js';

function setup(opts) {
  if (opts && opts.sendAction) {
    window.sendAction = opts.sendAction;
  }
  document.body.innerHTML = '<ph-power-controls id="test-el"></ph-power-controls>';
  const el = document.getElementById('test-el');
  return { el };
}

function queryText(host, sel) {
  const el = host.shadowRoot.querySelector(sel);
  return el ? el.textContent.trim() : null;
}

describe('PhPowerControls', () => {
  beforeEach(() => {
    document.body.innerHTML = '';
    delete window.sendAction;
  });

  afterEach(() => {
    document.body.innerHTML = '';
    delete window.sendAction;
  });

  it('is defined and registered as a custom element', () => {
    expect(customElements.get('ph-power-controls')).toBeDefined();
  });

  it('creates a shadow root', () => {
    const { el } = setup();
    expect(el.shadowRoot).toBeDefined();
  });

  it('renders empty state with no power groups', () => {
    const { el } = setup();
    el.state = {};
    expect(el.shadowRoot.textContent).toContain('NO POWER GROUPS');
  });

  it('renders empty state when groups is null', () => {
    const { el } = setup();
    el.state = { groups: null };
    expect(el.shadowRoot.textContent).toContain('NO POWER GROUPS');
  });

  it('renders groups with label and level text', () => {
    const { el } = setup();
    el.state = {
      groups: [
        { id: 'helm', label: 'HELM', level: 3, min_level: 1, max_level: 4 },
      ],
    };
    expect(el.shadowRoot.textContent).toContain('HELM');
    expect(el.shadowRoot.textContent).toContain('LVL 3');
  });

  it('renders pip track with correct number of pips', () => {
    const { el } = setup();
    el.state = {
      groups: [
        { id: 'helm', label: 'HELM', level: 3, min_level: 1, max_level: 4 },
      ],
    };
    const pips = el.shadowRoot.querySelectorAll('.pip');
    expect(pips.length).toBe(4); // min 1, max 4 = 4 pips
  });

  it('marks pips up to current level as active', () => {
    const { el } = setup();
    el.state = {
      groups: [
        { id: 'helm', label: 'HELM', level: 2, min_level: 0, max_level: 4 },
      ],
    };
    const pips = el.shadowRoot.querySelectorAll('.pip');
    expect(pips.length).toBe(5);
    expect(pips[0].classList.contains('active')).toBe(true);
    expect(pips[1].classList.contains('active')).toBe(true);
    expect(pips[2].classList.contains('active')).toBe(true);
    expect(pips[3].classList.contains('active')).toBe(false);
    expect(pips[4].classList.contains('active')).toBe(false);
  });

  it('disables increment button at max_level', () => {
    const { el } = setup();
    el.state = {
      groups: [
        { id: 'helm', label: 'HELM', level: 4, min_level: 1, max_level: 4 },
      ],
    };
    const incr = el.shadowRoot.querySelector('.step-btn[data-action="incr"]');
    expect(incr.disabled).toBe(true);
  });

  it('disables decrement button at min_level', () => {
    const { el } = setup();
    el.state = {
      groups: [
        { id: 'helm', label: 'HELM', level: 1, min_level: 1, max_level: 4 },
      ],
    };
    const decr = el.shadowRoot.querySelector('.step-btn[data-action="decr"]');
    expect(decr.disabled).toBe(true);
  });

  it('enables increment button when below max_level', () => {
    const { el } = setup();
    el.state = {
      groups: [
        { id: 'helm', label: 'HELM', level: 2, min_level: 1, max_level: 4 },
      ],
    };
    const incr = el.shadowRoot.querySelector('.step-btn[data-action="incr"]');
    expect(incr.disabled).toBe(false);
  });

  it('shows AUTO badge when auto=true', () => {
    const { el } = setup();
    el.state = {
      groups: [{ id: 'helm', label: 'HELM', level: 3, min_level: 1, max_level: 4 }],
      auto: true,
    };
    const badge = el.shadowRoot.getElementById('auto-badge');
    expect(badge.style.display).not.toBe('none');
  });

  it('hides AUTO badge when auto=false', () => {
    const { el } = setup();
    el.state = {
      groups: [{ id: 'helm', label: 'HELM', level: 3, min_level: 1, max_level: 4 }],
      auto: false,
    };
    const badge = el.shadowRoot.getElementById('auto-badge');
    expect(badge.style.display).toBe('none');
  });

  it('clicking pip dispatches set_power action', () => {
    const sendAction = vi.fn();
    const { el } = setup({ sendAction });
    el.state = {
      groups: [{ id: 'helm', label: 'HELM', level: 2, min_level: 0, max_level: 4 }],
    };
    const pips = el.shadowRoot.querySelectorAll('.pip');
    pips[3].click(); // click level 3
    expect(sendAction).toHaveBeenCalledWith('set_power', { group_id: 'helm', level: 3 });
  });

  it('does not dispatch set_power when auto=true', () => {
    const sendAction = vi.fn();
    const { el } = setup({ sendAction });
    el.state = {
      groups: [{ id: 'helm', label: 'HELM', level: 2, min_level: 0, max_level: 4 }],
      auto: true,
    };
    const pips = el.shadowRoot.querySelectorAll('.pip');
    pips[3].click();
    expect(sendAction).not.toHaveBeenCalled();
  });

  it('updates when state changes', () => {
    const { el } = setup();
    el.state = {
      groups: [{ id: 'helm', label: 'HELM', level: 1, min_level: 0, max_level: 4 }],
    };
    expect(el.shadowRoot.textContent).toContain('LVL 1');
    el.state = {
      groups: [{ id: 'helm', label: 'HELM', level: 4, min_level: 0, max_level: 4 }],
    };
    expect(el.shadowRoot.textContent).toContain('LVL 4');
  });
});
