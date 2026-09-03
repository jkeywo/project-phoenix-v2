// @vitest-environment jsdom
import { t } from '../../gui/strings.js';
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import '../../gui/components/ph-courier-radar.js';
import { makeRadarCtx } from './radar-canvas-stub.js';

function makeFakeCtx() {
  return makeRadarCtx();
}

function setup(opts) {
  opts = opts || {};
  if (opts.sendAction) {
    window.sendAction = opts.sendAction;
  }
  if (opts.activateSemanticAction) {
    window.activateSemanticAction = opts.activateSemanticAction;
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
    delete window.activateSemanticAction;
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
  it('fans one blip tap to the exact Tactical lock and shared Sensors action', () => {
    const sendAction = vi.fn();
    const activateSemanticAction = vi.fn();
    const { innerRadar } = setup({ sendAction, activateSemanticAction });

    innerRadar.sendAction('set_target', { uuid: 'abc' });

    expect(sendAction).toHaveBeenCalledTimes(1);
    expect(sendAction).toHaveBeenCalledWith('set_target', { uuid: 'abc' });
    expect(activateSemanticAction).toHaveBeenCalledWith('sensors.target-selection', {
      source: 'control', detail: { uuid: 'abc' },
    });
  });

  it('sends the same uuid to both actions', () => {
    const sendAction = vi.fn();
    const activateSemanticAction = vi.fn();
    const { innerRadar } = setup({ sendAction, activateSemanticAction });

    innerRadar.sendAction('set_target', { uuid: 'ship-42' });

    expect(sendAction.mock.calls[0][1].uuid).toBe('ship-42');
    expect(activateSemanticAction.mock.calls[0][1].detail.uuid).toBe('ship-42');
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
