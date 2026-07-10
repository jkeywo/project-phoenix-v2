// @vitest-environment jsdom
import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import '../../gui/components/ph-torpedo-controls.js';

function setup(opts) {
  const sendAction = opts && opts.sendAction;
  if (sendAction) {
    window.sendAction = sendAction;
  }
  document.body.innerHTML = '<ph-torpedo-controls id="test-el"></ph-torpedo-controls>';
  const el = document.getElementById('test-el');
  return { el };
}

function queryText(host, sel) {
  const el = host.shadowRoot.querySelector(sel);
  return el ? el.textContent.trim() : null;
}

describe('PhTorpedoControls', () => {
  beforeEach(() => {
    document.body.innerHTML = '';
    delete window.sendAction;
  });

  afterEach(() => {
    document.body.innerHTML = '';
    delete window.sendAction;
  });

  it('is defined and registered as a custom element', () => {
    expect(customElements.get('ph-torpedo-controls')).toBeDefined();
  });

  it('creates a shadow root', () => {
    const { el } = setup();
    expect(el.shadowRoot).toBeDefined();
  });

  it('renders NO TORPEDO TUBES placeholder with empty tubes', () => {
    const { el } = setup();
    el.state = { tubes: [], magazine: { current: 0, max: 0 } };
    expect(queryText(el, '#tubes')).toBe('NO TORPEDO TUBES');
  });

  it('renders NO TORPEDO TUBES placeholder with null state', () => {
    const { el } = setup();
    el.state = null;
    expect(queryText(el, '#tubes')).toBe('NO TORPEDO TUBES');
  });

  it('displays magazine count', () => {
    const { el } = setup();
    el.state = {
      tubes: [],
      magazine: { current: 6, max: 20 },
    };
    expect(queryText(el, '#magazine')).toBe('6 / 20');
  });

  it('renders a loaded tube with label and enabled fire button', () => {
    const { el } = setup();
    el.state = {
      tubes: [{ id: 'fore_port', label: 'Fore Port', loaded: true, load_progress: 0, auto: false }],
      magazine: { current: 6, max: 20 },
    };
    expect(queryText(el, '.lbl')).toBe('Fore Port');
    const fireBtn = el.shadowRoot.querySelector('.fire');
    expect(fireBtn.disabled).toBe(false);
    expect(fireBtn.textContent.trim()).toBe('FIRE');
  });

  it('disables fire button when tube is not loaded', () => {
    const { el } = setup();
    el.state = {
      tubes: [{ id: 'fore_port', label: 'Fore Port', loaded: false, load_progress: 0, auto: false }],
      magazine: { current: 6, max: 20 },
    };
    const fireBtn = el.shadowRoot.querySelector('.fire');
    expect(fireBtn.disabled).toBe(true);
  });

  it('disables load button when tube is already loaded', () => {
    const { el } = setup();
    el.state = {
      tubes: [{ id: 'fore_port', label: 'Fore Port', loaded: true, load_progress: 0, auto: false }],
      magazine: { current: 6, max: 20 },
    };
    const loadBtn = el.shadowRoot.querySelector('.load');
    expect(loadBtn.disabled).toBe(true);
  });

  it('disables load button when magazine is empty', () => {
    const { el } = setup();
    el.state = {
      tubes: [{ id: 'fore_port', label: 'Fore Port', loaded: false, load_progress: 0, auto: false }],
      magazine: { current: 0, max: 20 },
    };
    const loadBtn = el.shadowRoot.querySelector('.load');
    expect(loadBtn.disabled).toBe(true);
  });

  it('enables load button when tube is unloaded and magazine has ammo', () => {
    const { el } = setup();
    el.state = {
      tubes: [{ id: 'fore_port', label: 'Fore Port', loaded: false, load_progress: 0, auto: false }],
      magazine: { current: 6, max: 20 },
    };
    const loadBtn = el.shadowRoot.querySelector('.load');
    expect(loadBtn.disabled).toBe(false);
  });

  it('disables unload button when tube is not loaded', () => {
    const { el } = setup();
    el.state = {
      tubes: [{ id: 'fore_port', label: 'Fore Port', loaded: false, load_progress: 0, auto: false }],
      magazine: { current: 6, max: 20 },
    };
    const unloadBtn = el.shadowRoot.querySelector('.unload');
    expect(unloadBtn.disabled).toBe(true);
  });

  it('enables unload button when tube is loaded', () => {
    const { el } = setup();
    el.state = {
      tubes: [{ id: 'fore_port', label: 'Fore Port', loaded: true, load_progress: 0, auto: false }],
      magazine: { current: 6, max: 20 },
    };
    const unloadBtn = el.shadowRoot.querySelector('.unload');
    expect(unloadBtn.disabled).toBe(false);
  });

  it('shows load progress bar when tube state is loading', () => {
    const { el } = setup();
    el.state = {
      tubes: [{ id: 'fore_port', label: 'Fore Port', state: 'loading', loaded: false, load_progress: 0.4, auto: false }],
      magazine: { current: 6, max: 20 },
    };
    const wrap = el.shadowRoot.querySelector('.load-progress-wrap');
    const fill = el.shadowRoot.querySelector('.load-progress-fill');
    expect(wrap.style.display).not.toBe('none');
    expect(fill.style.width).toBe('40%');
  });

  it('hides load progress bar when tube is idle', () => {
    const { el } = setup();
    el.state = {
      tubes: [{ id: 'fore_port', label: 'Fore Port', loaded: true, load_progress: 0, auto: false }],
      magazine: { current: 6, max: 20 },
    };
    const wrap = el.shadowRoot.querySelector('.load-progress-wrap');
    expect(wrap.style.display).toBe('none');
  });

  it('shows AUTO badge and disables all buttons when auto is true', () => {
    const { el } = setup();
    el.state = {
      tubes: [{ id: 'fore_port', label: 'Fore Port', loaded: true, load_progress: 0, auto: true }],
      magazine: { current: 6, max: 20 },
    };
    const badge = el.shadowRoot.querySelector('.auto-badge');
    expect(badge.style.display).not.toBe('none');

    const buttons = el.shadowRoot.querySelectorAll('.tube-btn');
    buttons.forEach(btn => {
      expect(btn.disabled).toBe(true);
    });
  });

  it('dispatches load_tube on load button click', () => {
    const sendAction = vi.fn();
    const { el } = setup({ sendAction });
    el.state = {
      tubes: [{ id: 'fore_port', label: 'Fore Port', loaded: false, load_progress: 0, auto: false }],
      magazine: { current: 6, max: 20 },
    };
    const loadBtn = el.shadowRoot.querySelector('.load');
    loadBtn.click();
    expect(sendAction).toHaveBeenCalledWith('load_tube', { tube: 'fore_port' });
  });

  it('dispatches unload_tube on unload button click', () => {
    const sendAction = vi.fn();
    const { el } = setup({ sendAction });
    el.state = {
      tubes: [{ id: 'fore_port', label: 'Fore Port', loaded: true, load_progress: 0, auto: false }],
      magazine: { current: 6, max: 20 },
    };
    const unloadBtn = el.shadowRoot.querySelector('.unload');
    unloadBtn.click();
    expect(sendAction).toHaveBeenCalledWith('unload_tube', { tube: 'fore_port' });
  });

  it('dispatches fire_torpedo on fire button click', () => {
    const sendAction = vi.fn();
    const { el } = setup({ sendAction });
    el.state = {
      tubes: [{ id: 'fore_port', label: 'Fore Port', loaded: true, load_progress: 0, auto: false }],
      magazine: { current: 6, max: 20 },
    };
    const fireBtn = el.shadowRoot.querySelector('.fire');
    fireBtn.click();
    expect(sendAction).toHaveBeenCalledWith('fire_torpedo', { tube: 'fore_port' });
  });

  it('reconciles tube rows by id', () => {
    const { el } = setup();
    el.state = {
      tubes: [
        { id: 'fore_port', label: 'Fore Port', loaded: true, load_progress: 0, auto: false },
        { id: 'fore_starboard', label: 'Fore Starboard', loaded: false, load_progress: 0, auto: false },
      ],
      magazine: { current: 6, max: 20 },
    };
    expect(el.shadowRoot.querySelectorAll('.tube-row').length).toBe(2);

    el.state = {
      tubes: [{ id: 'fore_port', label: 'Fore Port', loaded: true, load_progress: 0, auto: false }],
      magazine: { current: 6, max: 20 },
    };
    expect(el.shadowRoot.querySelectorAll('.tube-row').length).toBe(1);
    expect(el.shadowRoot.querySelector('.tube-row').dataset.id).toBe('fore_port');
  });
});
