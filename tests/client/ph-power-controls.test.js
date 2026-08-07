// @vitest-environment jsdom
import { t } from '../../gui/strings.js';
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
    expect(el.shadowRoot.textContent).toContain(t('component.power.empty'));
  });

  it('renders empty state when groups is null', () => {
    const { el } = setup();
    el.state = { groups: null };
    expect(el.shadowRoot.textContent).toContain(t('component.power.empty'));
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
    const incr = el.shadowRoot.querySelector('.mini-btn[data-action="incr"]');
    expect(incr.disabled).toBe(true);
  });

  it('disables decrement button at min_level', () => {
    const { el } = setup();
    el.state = {
      groups: [
        { id: 'helm', label: 'HELM', level: 1, min_level: 1, max_level: 4 },
      ],
    };
    const decr = el.shadowRoot.querySelector('.mini-btn[data-action="decr"]');
    expect(decr.disabled).toBe(true);
  });

  it('enables increment button when below max_level', () => {
    const { el } = setup();
    el.state = {
      groups: [
        { id: 'helm', label: 'HELM', level: 2, min_level: 1, max_level: 4 },
      ],
    };
    const incr = el.shadowRoot.querySelector('.mini-btn[data-action="incr"]');
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
    expect(sendAction).toHaveBeenCalledWith('set_power', { target: 'helm', level: 3 });
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

  // ── Floored groups (issue #952) ──────────────────────────────────────────
  //
  // `level` is the EFFECTIVE level and `commanded_level` the standing order;
  // they differ while the reactor's battery floor holds a group down. Every
  // control steps from the commanded one, because `set_power` carries an
  // ABSOLUTE level that the server measures against the standing order.

  // Commanded 3, held down to 2 by the reactor.
  const heldHelm = {
    id: 'helm', label: 'HELM', level: 2, commanded_level: 3, min_level: 1, max_level: 4,
  };
  // Commanded at the cap, held down to 2.
  const heldHelmAtCap = {
    id: 'helm', label: 'HELM', level: 2, commanded_level: 4, min_level: 1, max_level: 4,
  };

  it('steps + from the commanded level, not the floored one', () => {
    const sendAction = vi.fn();
    const { el } = setup({ sendAction });
    el.state = { groups: [heldHelm] };
    el.shadowRoot.querySelector('.mini-btn[data-action="incr"]').click();
    // Stepping from the effective 2 would send 3, which the server measures
    // against a standing order that is already 3 — the press would do nothing
    // at all, and on a deeper floor it would be an outright DECREASE.
    expect(sendAction).toHaveBeenCalledWith('set_power', { target: 'helm', level: 4 });
  });

  it('steps − from the commanded level, not the floored one', () => {
    const sendAction = vi.fn();
    const { el } = setup({ sendAction });
    el.state = { groups: [heldHelmAtCap] };
    el.shadowRoot.querySelector('.mini-btn[data-action="decr"]').click();
    // From the effective 2 this would have sent 1 and collapsed an order of 4
    // by three whole steps.
    expect(sendAction).toHaveBeenCalledWith('set_power', { target: 'helm', level: 3 });
  });

  it('disables the buttons against the commanded level', () => {
    const { el } = setup();
    // Commanded at the cap while floored: `+` must be refused even though the
    // effective level has room.
    el.state = { groups: [heldHelmAtCap] };
    expect(el.shadowRoot.querySelector('.mini-btn[data-action="incr"]').disabled).toBe(true);
    expect(el.shadowRoot.querySelector('.mini-btn[data-action="decr"]').disabled).toBe(false);
    // Commanded at the minimum: `−` must be refused.
    el.state = {
      groups: [{ id: 'helm', label: 'HELM', level: 1, commanded_level: 1, min_level: 1, max_level: 4 }],
    };
    expect(el.shadowRoot.querySelector('.mini-btn[data-action="decr"]').disabled).toBe(true);
  });

  it('shows the held-down gap between effective and commanded', () => {
    const { el } = setup();
    el.state = { groups: [heldHelmAtCap] };
    expect(el.shadowRoot.textContent).toContain(t('component.power.held', { n: 2, c: 4 }));
    const pips = el.shadowRoot.querySelectorAll('.pip'); // levels 1..4
    expect(pips[0].classList.contains('active')).toBe(true);
    expect(pips[1].classList.contains('active')).toBe(true);
    expect(pips[2].classList.contains('held')).toBe(true);
    expect(pips[3].classList.contains('held')).toBe(true);
  });

  it('falls back to level when commanded_level is absent (pre-#952 payload)', () => {
    const sendAction = vi.fn();
    const { el } = setup({ sendAction });
    el.state = {
      groups: [{ id: 'helm', label: 'HELM', level: 2, min_level: 1, max_level: 4 }],
    };
    expect(el.shadowRoot.textContent).toContain(t('component.power.level', { n: 2 }));
    el.shadowRoot.querySelector('.mini-btn[data-action="incr"]').click();
    expect(sendAction).toHaveBeenCalledWith('set_power', { target: 'helm', level: 3 });
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
