// @vitest-environment jsdom
import { t } from '../../gui/strings.js';
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
    save: vi.fn(),
    restore: vi.fn(),
    translate: vi.fn(),
    rotate: vi.fn(),
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

  it('creates a shadow root with inner ph-radar and on-screen button', () => {
    const { el } = setup();
    expect(el.shadowRoot).toBeDefined();
    expect(el.shadowRoot.getElementById('inner-radar')).toBeDefined();
    expect(el.shadowRoot.getElementById('on-screen-btn')).toBeDefined();
  });

  it('on-screen button click calls sendAction with SensorsRadar view request', () => {
    const sendAction = vi.fn();
    const { el } = setup({ sendAction });
    el.shadowRoot.getElementById('on-screen-btn').click();
    expect(sendAction).toHaveBeenCalledWith('set_view', { direction: 'SensorsRadar' });
  });

  it('passes base state through to inner ph-radar', () => {
    const { el, innerRadar } = setup();
    el.state = {
      blips: [{ uuid: 'a', bearing_deg: 0, range: 500 }],
      scan_range: 1000,
      ship_heading: 90,
      config: { max_range: 5000 },
    };
    expect(innerRadar.state).toEqual({
      blips: [{ uuid: 'a', bearing_deg: 0, range: 500 }],
      range: 1000,
      ship_heading: 90,
      config: { max_range: 5000 },
      selected_target_uuid: null,
    });
  });

  // ── One ring, not two (PRD #1023's defect list) ────────────────────
  //
  // ph-radar draws two independent rings: cyan around `selected_target_uuid`
  // (this console's own selection) and red around `target_uuid` (the ship's
  // weapons lock). Sensors handed its one scan target to BOTH keys, so every
  // contact the sensors officer picked came up double-ringed — and the red
  // ring asserted a weapons lock this console cannot know about, since
  // weapons may be holding fire or aimed somewhere else entirely.

  it('marks its scan target as a selection, not as a weapons lock', () => {
    const { el, innerRadar } = setup();
    el.state = {
      blips: [{ uuid: 'abc', bearing_deg: 45, range: 300 }],
      scan_range: 2000,
      target_uuid: 'abc',
    };
    expect(innerRadar.state.selected_target_uuid).toBe('abc');
    expect(innerRadar.state.target_uuid).toBeUndefined();
  });

  it('gives the scanned contact exactly one of the scope’s two rings', () => {
    const { el, innerRadar } = setup();
    el.state = {
      blips: [{ uuid: 'abc', radar_x: 0, radar_y: 0 }],
      scan_range: 2000,
      target_uuid: 'abc',
    };
    const s = innerRadar.state;
    const ringed = ['selected_target_uuid', 'target_uuid'].filter((key) => s[key] === 'abc');
    expect(ringed).toEqual(['selected_target_uuid']);
  });

  it('blip click on inner radar dispatches set_sensors_target directly', () => {
    const sendAction = vi.fn();
    const { el, innerRadar } = setup({ sendAction });
    el.state = {
      blips: [{ uuid: 'abc' }],
      science_target_uuid: 'def',
    };

    innerRadar.sendAction('set_target', { uuid: 'abc' });
    expect(sendAction).toHaveBeenCalledTimes(1);
    expect(sendAction).toHaveBeenCalledWith('set_sensors_target', { uuid: 'abc' });
  });

  it('does not forward set_target upstream', () => {
    const sendAction = vi.fn();
    const { el, innerRadar } = setup({ sendAction });

    innerRadar.sendAction('set_target', { uuid: 'abc' });
    expect(sendAction).not.toHaveBeenCalledWith('set_target', expect.anything());
  });
});
