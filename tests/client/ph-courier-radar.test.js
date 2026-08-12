// @vitest-environment jsdom
import { t } from '../../gui/strings.js';
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import '../../gui/components/ph-courier-radar.js';

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
  document.body.innerHTML = '<ph-courier-radar id="test-el"></ph-courier-radar>';
  const el = document.getElementById('test-el');
  const innerRadar = el.shadowRoot.getElementById('inner-radar');
  return { el, innerRadar };
}

let origGetContext;
let origRAF;
let origCARAF;
let origRO;

describe('PhCourierRadar', () => {
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
    expect(customElements.get('ph-courier-radar')).toBeDefined();
  });

  it('inherits the tactical radar shadow DOM (inner radar + arc overlays)', () => {
    const { el } = setup();
    expect(el.shadowRoot.getElementById('inner-radar')).toBeTruthy();
    expect(el.shadowRoot.getElementById('phaser-arcs')).toBeTruthy();
    expect(el.shadowRoot.getElementById('selected-highlight')).toBeTruthy();
  });

  // The whole reason this component exists: the Courier has one station, so a
  // single tap has to drive both the blaster target and the sensor readout.
  it('fans one blip tap out to both set_target and set_sensors_target', () => {
    const sendAction = vi.fn();
    const { innerRadar } = setup({ sendAction });

    innerRadar.sendAction('set_target', { uuid: 'abc' });

    expect(sendAction).toHaveBeenCalledTimes(2);
    expect(sendAction).toHaveBeenCalledWith('set_target', { uuid: 'abc' });
    expect(sendAction).toHaveBeenCalledWith('set_sensors_target', { uuid: 'abc' });
  });

  it('sends the same uuid to both actions', () => {
    const sendAction = vi.fn();
    const { innerRadar } = setup({ sendAction });

    innerRadar.sendAction('set_target', { uuid: 'ship-42' });

    const uuids = sendAction.mock.calls.map(([, payload]) => payload.uuid);
    expect(uuids).toEqual(['ship-42', 'ship-42']);
  });

  it('still passes state through to the inner radar', () => {
    const { el, innerRadar } = setup();
    el.state = {
      blips: [{ uuid: 'a', radar_x: 0.5, radar_y: 0.5 }],
      ship_heading: 90,
      target_uuid: 'a',
    };
    expect(innerRadar.state.blips).toEqual([{ uuid: 'a', radar_x: 0.5, radar_y: 0.5 }]);
    expect(innerRadar.state.ship_heading).toBe(90);
    expect(innerRadar.state.target_uuid).toBe('a');
  });
});
