// @vitest-environment jsdom
import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import '../../gui/components/ph-camera-select.js';

function setup(opts) {
  const sendAction = opts && opts.sendAction;
  if (sendAction) {
    window.sendAction = sendAction;
  }
  document.body.innerHTML = '<ph-camera-select id="test-el"></ph-camera-select>';
  const el = document.getElementById('test-el');
  return { el };
}

function queryText(host, sel) {
  const el = host.shadowRoot.querySelector(sel);
  return el ? el.textContent.trim() : null;
}

describe('PhCameraSelect', () => {
  beforeEach(() => {
    document.body.innerHTML = '';
    delete window.sendAction;
  });

  afterEach(() => {
    document.body.innerHTML = '';
    delete window.sendAction;
  });

  it('is defined and registered as a custom element', () => {
    expect(customElements.get('ph-camera-select')).toBeDefined();
  });

  it('creates a shadow root', () => {
    const { el } = setup();
    expect(el.shadowRoot).toBeDefined();
  });

  it('renders NO CAMERA placeholder with empty/null state', () => {
    const { el } = setup();
    el.state = null;
    expect(el.shadowRoot.textContent).toContain('NO CAMERA');

    el.state = {};
    expect(el.shadowRoot.textContent).toContain('NO CAMERA');
  });

  it('renders 4 buttons when views array provided', () => {
    const { el } = setup();
    el.state = {
      viewscreen_system_id: 'viewscreen',
      current_view: 'Fore',
      views: ['Fore', 'Aft', 'Port', 'Starboard'],
      auto: false,
    };
    const buttons = el.shadowRoot.querySelectorAll('.cam-btn');
    expect(buttons.length).toBe(4);
    expect(buttons[0].textContent.trim()).toBe('Fore');
    expect(buttons[1].textContent.trim()).toBe('Aft');
    expect(buttons[2].textContent.trim()).toBe('Port');
    expect(buttons[3].textContent.trim()).toBe('Starboard');
  });

  it('active view button has distinct styling (active class)', () => {
    const { el } = setup();
    el.state = {
      viewscreen_system_id: 'viewscreen',
      current_view: 'Port',
      views: ['Fore', 'Aft', 'Port', 'Starboard'],
      auto: false,
    };
    const buttons = el.shadowRoot.querySelectorAll('.cam-btn');
    expect(buttons[0].classList.contains('active')).toBe(false);
    expect(buttons[1].classList.contains('active')).toBe(false);
    expect(buttons[2].classList.contains('active')).toBe(true);
    expect(buttons[3].classList.contains('active')).toBe(false);
  });

  it('AUTO badge shown and buttons disabled when auto=true', () => {
    const { el } = setup();
    el.state = {
      viewscreen_system_id: 'viewscreen',
      current_view: 'Fore',
      views: ['Fore', 'Aft', 'Port', 'Starboard'],
      auto: true,
    };
    const badge = el.shadowRoot.getElementById('auto-badge');
    expect(badge.style.display).not.toBe('none');
    expect(badge.textContent.trim()).toBe('AUTO');

    const buttons = el.shadowRoot.querySelectorAll('.cam-btn');
    buttons.forEach(btn => {
      expect(btn.disabled).toBe(true);
    });
  });

  it('AUTO badge hidden when auto=false', () => {
    const { el } = setup();
    el.state = {
      viewscreen_system_id: 'viewscreen',
      current_view: 'Fore',
      views: ['Fore', 'Aft', 'Port', 'Starboard'],
      auto: false,
    };
    const badge = el.shadowRoot.getElementById('auto-badge');
    expect(badge.style.display).toBe('none');
  });

  it('clicking a view button calls sendAction with set_view and view name', () => {
    const sendAction = vi.fn();
    const { el } = setup({ sendAction });
    el.state = {
      viewscreen_system_id: 'viewscreen',
      current_view: 'Fore',
      views: ['Fore', 'Aft', 'Port', 'Starboard'],
      auto: false,
    };

    const btn = el.shadowRoot.querySelector('[data-view="Port"]');
    btn.click();
    expect(sendAction).toHaveBeenCalledTimes(1);
    expect(sendAction).toHaveBeenCalledWith('set_view', { view: 'Port' });
  });

  it('buttons disabled when auto=true', () => {
    const { el } = setup();
    el.state = {
      viewscreen_system_id: 'viewscreen',
      current_view: 'Fore',
      views: ['Fore', 'Aft', 'Port', 'Starboard'],
      auto: true,
    };
    const buttons = el.shadowRoot.querySelectorAll('.cam-btn');
    buttons.forEach(btn => {
      expect(btn.hasAttribute('disabled')).toBe(true);
    });
  });

  it('clicking a button does nothing when auto=true', () => {
    const sendAction = vi.fn();
    const { el } = setup({ sendAction });
    el.state = {
      viewscreen_system_id: 'viewscreen',
      current_view: 'Fore',
      views: ['Fore', 'Aft', 'Port', 'Starboard'],
      auto: true,
    };

    const btn = el.shadowRoot.querySelector('[data-view="Aft"]');
    btn.click();
    expect(sendAction).not.toHaveBeenCalled();
  });
});
