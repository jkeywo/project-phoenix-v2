// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import '../../gui/components/ph-sensor-radar.js';

function makeFakeCtx() {
  return {
    fillStyle: '',
    font: '',
    fillRect: vi.fn(),
    beginPath: vi.fn(),
    arc: vi.fn(),
    fill: vi.fn(),
    fillText: vi.fn(),
    drawImage: vi.fn(),
  };
}

function setup(opts) {
  opts = opts || {};
  if (opts.sendAction) {
    window.sendAction = opts.sendAction;
  }
  document.body.innerHTML = '<ph-sensor-radar id="test-el"></ph-sensor-radar>';
  const el = document.getElementById('test-el');
  const innerRadar = el.shadowRoot.getElementById('inner-radar');
  return { el, innerRadar };
}

let origGetContext;
let origRAF;
let origCARAF;
let origRO;

describe('PhSensorRadar', () => {
  beforeEach(() => {
    document.body.innerHTML = '';
    delete window.sendAction;
    origGetContext = HTMLCanvasElement.prototype.getContext;
    HTMLCanvasElement.prototype.getContext = function () { return makeFakeCtx(); };
    origRAF = window.requestAnimationFrame;
    window.requestAnimationFrame = vi.fn(() => 1);
    origCARAF = window.cancelAnimationFrame;
    window.cancelAnimationFrame = vi.fn();
    origRO = window.ResizeObserver;
    window.ResizeObserver = function () {
      return { observe: vi.fn(), disconnect: vi.fn() };
    };
  });

  afterEach(() => {
    document.body.innerHTML = '';
    delete window.sendAction;
    HTMLCanvasElement.prototype.getContext = origGetContext;
    window.requestAnimationFrame = origRAF;
    window.cancelAnimationFrame = origCARAF;
    if (origRO) window.ResizeObserver = origRO;
  });

  it('is defined and registered as a custom element', () => {
    expect(customElements.get('ph-sensor-radar')).toBeDefined();
  });

  it('creates a shadow root with inner ph-radar and DESIGNATE button', () => {
    const { el } = setup();
    expect(el.shadowRoot).toBeDefined();
    expect(el.shadowRoot.getElementById('inner-radar')).toBeDefined();
    expect(el.shadowRoot.getElementById('designate-btn')).toBeDefined();
  });

  it('passes base state through to inner ph-radar', () => {
    const { el, innerRadar } = setup();
    el.state = {
      blips: [{ uuid: 'a', bearing_deg: 0, range: 500 }],
      range: 1000,
      ship_heading: 90,
      config: { max_range: 5000 },
    };
    expect(innerRadar.state).toEqual({
      blips: [{ uuid: 'a', bearing_deg: 0, range: 500 }],
      range: 1000,
      ship_heading: 90,
      config: { max_range: 5000 },
    });
  });

  it('DESIGNATE button is disabled when no target is selected', () => {
    const { el } = setup();
    el.state = { blips: [{ uuid: 'abc' }], science_target_uuid: null };
    const btn = el.shadowRoot.getElementById('designate-btn');
    expect(btn.disabled).toBe(true);
  });

  it('DESIGNATE button is enabled when target selected and differs from science target', () => {
    const { el } = setup();
    el.state = {
      blips: [{ uuid: 'abc' }],
      selected_target_uuid: 'abc',
      science_target_uuid: 'def',
    };
    const btn = el.shadowRoot.getElementById('designate-btn');
    expect(btn.disabled).toBe(false);
  });

  it('DESIGNATE button is disabled when selected target is already the science target', () => {
    const { el } = setup();
    el.state = {
      blips: [{ uuid: 'abc' }],
      selected_target_uuid: 'abc',
      science_target_uuid: 'abc',
    };
    const btn = el.shadowRoot.getElementById('designate-btn');
    expect(btn.disabled).toBe(true);
  });

  it('clicking DESIGNATE dispatches sendAction with set_science_target', () => {
    const sendAction = vi.fn();
    const { el } = setup({ sendAction });
    el.state = {
      selected_target_uuid: 'abc-123',
      science_target_uuid: null,
    };
    const btn = el.shadowRoot.getElementById('designate-btn');
    btn.click();
    expect(sendAction).toHaveBeenCalledTimes(1);
    expect(sendAction).toHaveBeenCalledWith('set_science_target', { uuid: 'abc-123' });
  });

  it('blip click on inner radar updates selected_target_uuid and enables button', () => {
    const sendAction = vi.fn();
    const { el, innerRadar } = setup({ sendAction });
    el.state = {
      blips: [{ uuid: 'abc' }],
      science_target_uuid: 'def',
    };
    const btn = el.shadowRoot.getElementById('designate-btn');
    expect(btn.disabled).toBe(true);

    innerRadar.sendAction('select_target', { uuid: 'abc' });
    expect(btn.disabled).toBe(false);
    expect(sendAction).toHaveBeenCalledWith('select_target', { uuid: 'abc' });
  });
});
