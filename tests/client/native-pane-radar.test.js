// @vitest-environment jsdom
import { it, expect, vi } from 'vitest';
import { makeRadarCtx } from './radar-canvas-stub.js';

it('upgrades Tactical on the pane scheduler and redraws moving contacts with a stationary ship', async () => {
  const originalParent = Object.getOwnPropertyDescriptor(window, 'parent');
  const originalRequest = window.requestAnimationFrame;
  const originalCancel = window.cancelAnimationFrame;
  const originalObserver = window.ResizeObserver;
  const originalImage = window.Image;
  const pending = new Map();
  let next = 0;
  const engineRequest = vi.fn(() => -1); // quiet Ultralight rAF never fires
  const host = {
    PhoenixOperatorCapabilities: { surface: 'native-pane' },
    requestAnimationFrame: cb => { pending.set(++next, cb); return next; },
    cancelAnimationFrame: id => pending.delete(id),
  };
  const context = makeRadarCtx();
  const tick = () => {
    const due = [...pending.values()];
    pending.clear();
    for (const cb of due) cb(90000);
  };

  Object.defineProperty(window, 'parent', { configurable: true, value: host });
  window.requestAnimationFrame = engineRequest;
  window.cancelAnimationFrame = vi.fn();
  window.ResizeObserver = class { observe() {} disconnect() {} };
  window.Image = class { naturalWidth = 64; naturalHeight = 64; complete = true; };
  vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockReturnValue(context);
  vi.spyOn(HTMLElement.prototype, 'getBoundingClientRect').mockReturnValue({
    width: 300, height: 300, left: 0, top: 0, right: 300, bottom: 300,
  });
  try {
    // The actual Tactical page imports its radar before console-core. The
    // already-parsed tag must therefore use the pane scheduler at upgrade,
    // before initConsole or any later load event could repair a stuck rAF.
    document.body.innerHTML = '<ph-tactical-radar></ph-tactical-radar>';
    await import('../../gui/components/ph-tactical-radar.js');
    const tactical = document.querySelector('ph-tactical-radar');
    const radar = tactical.shadowRoot.querySelector('ph-radar');
    expect(engineRequest).not.toHaveBeenCalled();
    expect(pending.size).toBeGreaterThan(0);

    const ownShip = { ship_heading: 0, ship_x: 0, ship_y: 0 };
    tactical.state = { ...ownShip, blips: [{ uuid: 'contact', radar_x: 0, radar_y: 0.2 }] };
    tick();
    const firstContact = context._calls.arc.find(args => args[2] === 6);
    expect(firstContact).toBeDefined();
    expect(radar.needsRender).toBe(false);
    context._reset();
    tick();
    expect(context._calls.fillRect).toHaveLength(0); // quiet canvas, loop alive

    tactical.state = { ...ownShip, blips: [{ uuid: 'contact', radar_x: 0.4, radar_y: 0.2 }] };
    expect(radar.needsRender).toBe(true);
    tick();
    expect(radar.needsRender).toBe(false);
    expect(context._calls.fillRect.length).toBeGreaterThan(0);
    const movedContact = context._calls.arc.find(args => args[2] === 6);
    expect(movedContact[0]).toBeGreaterThan(firstContact[0]);
    expect(movedContact[1]).toBe(firstContact[1]);
    expect(engineRequest).not.toHaveBeenCalled();

    document.body.innerHTML = '';
    expect(pending.size).toBe(0); // disconnected radar cancels the parent's id
  } finally {
    document.body.innerHTML = '';
    Object.defineProperty(window, 'parent', originalParent);
    window.requestAnimationFrame = originalRequest;
    window.cancelAnimationFrame = originalCancel;
    window.ResizeObserver = originalObserver;
    window.Image = originalImage;
    vi.restoreAllMocks();
  }
});
