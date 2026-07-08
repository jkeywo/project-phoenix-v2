// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import '../../gui/components/ph-helm-radar.js';

let rafCb;
let rafIdCounter;
let origRAF;
let origCARAF;
let origRO;
let origGetContext;

function makeFakeCtx() {
  const ctx = {
    fillStyle: '',
    font: '',
    fillRect: vi.fn(),
    beginPath: vi.fn(),
    arc: vi.fn(),
    fill: vi.fn(),
    fillText: vi.fn(),
    drawImage: vi.fn(),
  };
  return ctx;
}

function mockRAF() {
  rafCb = null;
  rafIdCounter = 0;
  origRAF = window.requestAnimationFrame;
  origCARAF = window.cancelAnimationFrame;
  window.requestAnimationFrame = vi.fn((cb) => { rafCb = cb; return ++rafIdCounter; });
  window.cancelAnimationFrame = vi.fn();
}

function restoreRAF() {
  if (origRAF) window.requestAnimationFrame = origRAF;
  if (origCARAF) window.cancelAnimationFrame = origCARAF;
}

function setup(opts) {
  if (opts && opts.sendAction) {
    window.sendAction = opts.sendAction;
  }
  document.body.innerHTML = '<ph-helm-radar id="test-el"></ph-helm-radar>';
  const el = document.getElementById('test-el');
  return { el };
}

describe('PhHelmRadar', () => {
  beforeEach(() => {
    document.body.innerHTML = '';
    delete window.sendAction;
    mockRAF();
    origRO = window.ResizeObserver;
    window.ResizeObserver = function () {
      return { observe: vi.fn(), disconnect: vi.fn() };
    };
    origGetContext = HTMLCanvasElement.prototype.getContext;
    HTMLCanvasElement.prototype.getContext = function () { return makeFakeCtx(); };
  });

  afterEach(() => {
    document.body.innerHTML = '';
    delete window.sendAction;
    restoreRAF();
    if (origRO) window.ResizeObserver = origRO;
    if (origGetContext) HTMLCanvasElement.prototype.getContext = origGetContext;
  });

  it('is defined and registered as a custom element', () => {
    expect(customElements.get('ph-helm-radar')).toBeDefined();
  });

  it('creates a shadow root with inner ph-radar, SVG overlay, and ON SCREEN button', () => {
    const { el } = setup();
    expect(el.shadowRoot).toBeDefined();
    expect(el.shadowRoot.getElementById('inner-radar')).toBeDefined();
    expect(el.shadowRoot.getElementById('arc-port')).toBeDefined();
    expect(el.shadowRoot.getElementById('arc-stbd')).toBeDefined();
    expect(el.shadowRoot.getElementById('on-screen-btn')).toBeDefined();
  });

  it('passes base state through to inner ph-radar', () => {
    const { el } = setup();
    const inner = el.shadowRoot.getElementById('inner-radar');

    el.state = {
      blips: [{ uuid: 'a', bearing_deg: 0, range: 500 }],
      range: 1000,
      ship_heading: 90,
      on_screen_active: false,
    };

    expect(inner.state).toEqual({
      blips: [{ uuid: 'a', bearing_deg: 0, range: 500 }],
      range: 1000,
      ship_heading: 90,
      config: {},
    });
  });

  it('passes through config to inner ph-radar', () => {
    const { el } = setup();
    const inner = el.shadowRoot.getElementById('inner-radar');
    el.state = { config: { max_range: 5000 } };
    expect(inner.state.config).toEqual({ max_range: 5000 });
  });

  it('ON SCREEN button dispatches sendAction with set_view when clicked', () => {
    const sendAction = vi.fn();
    const { el } = setup({ sendAction });

    const btn = el.shadowRoot.getElementById('on-screen-btn');
    btn.click();

    expect(sendAction).toHaveBeenCalledWith('set_view', { view: 'radar' });
  });

  it('ON SCREEN button shows active class when on_screen_active is true', () => {
    const { el } = setup();
    const btn = el.shadowRoot.getElementById('on-screen-btn');

    expect(btn.classList.contains('active')).toBe(false);

    el.state = { on_screen_active: true };
    expect(btn.classList.contains('active')).toBe(true);

    el.state = { on_screen_active: false };
    expect(btn.classList.contains('active')).toBe(false);
  });

  it('thrust arcs update SVG path attributes from engine_port_thrust and engine_stbd_thrust', () => {
    const { el } = setup();
    const arcPort = el.shadowRoot.getElementById('arc-port');
    const arcStbd = el.shadowRoot.getElementById('arc-stbd');

    el.state = { engine_port_thrust: 0.5, engine_stbd_thrust: 1.0 };

    const portD = arcPort.getAttribute('d');
    const stbdD = arcStbd.getAttribute('d');
    expect(portD).toBeTruthy();
    expect(portD).toContain('A');
    expect(stbdD).toBeTruthy();
    expect(stbdD).toContain('A');
    expect(arcPort.style.opacity).toBe(String(0.2 + 0.8 * 0.5));
    expect(arcStbd.style.opacity).toBe(String(1.0));
  });

  it('zero thrust clears arc path', () => {
    const { el } = setup();
    const arcPort = el.shadowRoot.getElementById('arc-port');

    el.state = { engine_port_thrust: 0 };
    expect(arcPort.getAttribute('d')).toBe('');
  });

  it('clamps thrust arcs to [0, 1]', () => {
    const { el } = setup();
    const arcPort = el.shadowRoot.getElementById('arc-port');

    el.state = { engine_port_thrust: 2.5 };
    const d = arcPort.getAttribute('d');
    expect(d).toBeTruthy();

    el.state = { engine_port_thrust: -0.5 };
    expect(arcPort.getAttribute('d')).toBe('');
  });
});
