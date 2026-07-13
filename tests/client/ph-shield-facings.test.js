// @vitest-environment jsdom
import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import '../../gui/components/ph-shield-facings.js';

function setup(opts) {
  if (opts && opts.sendAction) {
    window.sendAction = opts.sendAction;
  }
  document.body.innerHTML = '<ph-shield-facings id="test-el"></ph-shield-facings>';
  const el = document.getElementById('test-el');
  return { el };
}

function queryText(host, sel) {
  const el = host.shadowRoot.querySelector(sel);
  return el ? el.textContent.trim() : null;
}

describe('PhShieldFacings', () => {
  beforeEach(() => {
    document.body.innerHTML = '';
    delete window.sendAction;
  });

  afterEach(() => {
    document.body.innerHTML = '';
    delete window.sendAction;
  });

  it('is defined and registered as a custom element', () => {
    expect(customElements.get('ph-shield-facings')).toBeDefined();
  });

  it('creates a shadow root', () => {
    const { el } = setup();
    expect(el.shadowRoot).toBeDefined();
  });

  it('renders empty state with no facing data', () => {
    const { el } = setup();
    el.state = {};
    expect(el.shadowRoot.textContent).toContain('NO FACING DATA');
  });

  it('renders empty state when facings is null', () => {
    const { el } = setup();
    el.state = { facings: null };
    expect(el.shadowRoot.textContent).toContain('NO FACING DATA');
  });

  it('renders facings with SVG arcs', () => {
    const { el } = setup();
    el.state = {
      facings: [
        { arc_id: 'fore', label: 'Fore', hp: 100, max_hp: 100, online: true },
        { arc_id: 'port', label: 'Port', hp: 50, max_hp: 100, online: true },
        { arc_id: 'aft', label: 'Aft', hp: 0, max_hp: 100, online: false },
      ],
      focused_facing: 'port',
      system_id: 'shields-system',
      auto: false,
    };
    const svg = el.shadowRoot.querySelector('svg');
    expect(svg).toBeDefined();
    expect(el.shadowRoot.textContent).toContain('FORE');
    expect(el.shadowRoot.textContent).toContain('PORT');
    expect(el.shadowRoot.textContent).toContain('AFT');
  });

  it('shows OFF label for offline facing', () => {
    const { el } = setup();
    el.state = {
      facings: [
        { arc_id: 'aft', label: 'Aft', hp: 0, max_hp: 100, online: false },
      ],
    };
    expect(el.shadowRoot.textContent).toContain('OFF');
  });

  it('shows HP percentage for online facing', () => {
    const { el } = setup();
    el.state = {
      facings: [
        { arc_id: 'fore', label: 'Fore', hp: 100, max_hp: 100, online: true },
      ],
    };
    expect(el.shadowRoot.textContent).toContain('100%');
  });

  it('shows AUTO badge and disables interaction when auto=true', () => {
    const { el } = setup();
    el.state = {
      facings: [{ arc_id: 'fore', label: 'Fore', hp: 100, max_hp: 100, online: true }],
      auto: true,
    };
    const badge = el.shadowRoot.getElementById('auto-badge');
    expect(badge.style.display).not.toBe('none');
  });

  it('hides AUTO badge when auto=false', () => {
    const { el } = setup();
    el.state = {
      facings: [{ arc_id: 'fore', label: 'Fore', hp: 100, max_hp: 100, online: true }],
      auto: false,
    };
    const badge = el.shadowRoot.getElementById('auto-badge');
    expect(badge.style.display).toBe('none');
  });

  it('clicking an unfocused facing arc dispatches set_shield_focus with focused: true', () => {
    const sendAction = vi.fn();
    const { el } = setup({ sendAction });
    el.state = {
      facings: [
        { arc_id: 'fore', label: 'Fore', hp: 100, max_hp: 100, online: true },
      ],
      focused_facing: null,
      auto: false,
    };
    const path = el.shadowRoot.querySelector('.arc-path');
    expect(path).toBeDefined();
    path.dispatchEvent(new MouseEvent('click'));
    expect(sendAction).toHaveBeenCalledWith('set_shield_focus', { arc_id: 'fore', focused: true });
  });

  it('clicking the already-focused facing arc dispatches set_shield_focus with focused: false', () => {
    const sendAction = vi.fn();
    const { el } = setup({ sendAction });
    el.state = {
      facings: [
        { arc_id: 'fore', label: 'Fore', hp: 100, max_hp: 100, online: true },
      ],
      focused_facing: 'fore',
      auto: false,
    };
    const path = el.shadowRoot.querySelector('.arc-path');
    path.dispatchEvent(new MouseEvent('click'));
    expect(sendAction).toHaveBeenCalledWith('set_shield_focus', { arc_id: 'fore', focused: false });
  });

  it('does not dispatch action when auto=true', () => {
    const sendAction = vi.fn();
    const { el } = setup({ sendAction });
    el.state = {
      facings: [
        { arc_id: 'fore', label: 'Fore', hp: 100, max_hp: 100, online: true },
      ],
      auto: true,
    };
    const path = el.shadowRoot.querySelector('.arc-path');
    path.dispatchEvent(new MouseEvent('click'));
    expect(sendAction).not.toHaveBeenCalled();
  });

  it('updates when state changes', () => {
    const { el } = setup();
    el.state = {
      facings: [{ arc_id: 'fore', label: 'Fore', hp: 100, max_hp: 100, online: true }],
      focused_facing: null,
    };
    expect(el.shadowRoot.textContent).toContain('FORE');
    el.state = {
      facings: [
        { arc_id: 'fore', label: 'Fore', hp: 100, max_hp: 100, online: true },
        { arc_id: 'port', label: 'Port', hp: 50, max_hp: 100, online: true },
      ],
      focused_facing: 'port',
    };
    expect(el.shadowRoot.textContent).toContain('PORT');
  });
});
