// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import '../../gui/components/ph-helm-joystick.js';

/** Helper: mock rAF so tests can step through frame callbacks synchronously. */
let rafCb = null;
let rafIdCounter = 0;
let origRAF;
let origCARAF;

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

/** Tick one rAF frame synchronously. */
function tickRaf() {
  if (rafCb) {
    const cb = rafCb;
    rafCb = null;
    cb(performance.now());
  }
}

function setup(opts) {
  if (opts && opts.sendAction) {
    window.sendAction = opts.sendAction;
  }
  document.body.innerHTML = '<ph-helm-joystick id="test-el"></ph-helm-joystick>';
  const el = document.getElementById('test-el');
  return { el };
}

/** Mock getBoundingClientRect on the well so position calcs are deterministic. */
function stubWellRect(el, w, h) {
  const well = el.shadowRoot.getElementById('well');
  Object.defineProperty(well, 'getBoundingClientRect', {
    value: () => ({ left: 0, top: 0, width: w, height: h }),
    configurable: true,
  });
}

describe('PhHelmJoystick', () => {
  beforeEach(() => {
    document.body.innerHTML = '';
    delete window.sendAction;
  });

  afterEach(() => {
    document.body.innerHTML = '';
    delete window.sendAction;
    restoreRAF();
  });

  it('is defined and registered as a custom element', () => {
    expect(customElements.get('ph-helm-joystick')).toBeDefined();
  });

  it('creates a shadow root', () => {
    const { el } = setup();
    expect(el.shadowRoot).toBeDefined();
  });

  it('renders auto state with AUTO badge visible and well.auto class', () => {
    const { el } = setup();
    el.state = { auto: true };
    const badge = el.shadowRoot.getElementById('auto-badge');
    expect(badge.style.display).not.toBe('none');
    expect(badge.textContent.trim()).toBe('AUTO');
    expect(el.shadowRoot.getElementById('well').classList.contains('auto')).toBe(true);
  });

  it('renders non-auto state with AUTO badge hidden', () => {
    const { el } = setup();
    el.state = { auto: false };
    const badge = el.shadowRoot.getElementById('auto-badge');
    expect(badge.style.display).toBe('none');
    expect(el.shadowRoot.getElementById('well').classList.contains('auto')).toBe(false);
  });

  it('does not fire sendAction when auto state is active', () => {
    mockRAF();
    const sendAction = vi.fn();
    const { el } = setup({ sendAction });
    el.state = { auto: true };
    stubWellRect(el, 240, 240);
    const well = el.shadowRoot.getElementById('well');
    well.dispatchEvent(new PointerEvent('pointerdown', { pointerId: 1, clientX: 200, clientY: 200 }));
    expect(sendAction).not.toHaveBeenCalled();
  });

  it('sends normalized set_helm action on pointer release after drag', () => {
    mockRAF();
    const sendAction = vi.fn();
    const { el } = setup({ sendAction });
    el.state = { auto: false };
    stubWellRect(el, 240, 240);
    const well = el.shadowRoot.getElementById('well');

    // Drag far out — the clamp in #setFromPointer keeps dx,dy in [-1,1]
    well.dispatchEvent(new PointerEvent('pointerdown', { pointerId: 1, clientX: 500, clientY: -200 }));
    tickRaf(); // scheduleApply rAF (nub visual)

    // Release sends the final action synchronously
    well.dispatchEvent(new PointerEvent('pointerup', { pointerId: 1 }));

    expect(sendAction).toHaveBeenCalledTimes(1);
    const call = sendAction.mock.calls[0];
    expect(call[0]).toBe('set_helm');
    expect(Math.abs(call[1].thrust)).toBeLessThanOrEqual(1);
    expect(Math.abs(call[1].yaw)).toBeLessThanOrEqual(1);
  });

  it('snaps nub to center and sends zero thrust/yaw on release', () => {
    vi.useFakeTimers();
    const sendAction = vi.fn();
    const { el } = setup({ sendAction });
    el.state = { auto: false };
    stubWellRect(el, 240, 240);
    const well = el.shadowRoot.getElementById('well');

    well.dispatchEvent(new PointerEvent('pointerdown', { pointerId: 1, clientX: 200, clientY: 50 }));
    vi.advanceTimersByTime(100);
    sendAction.mockClear();

    well.dispatchEvent(new PointerEvent('pointerup', { pointerId: 1 }));
    // allow the final sendAction (fired synchronously in onUp) plus visual rAF
    vi.advanceTimersByTime(0);

    const calls = sendAction.mock.calls;
    expect(calls.length).toBeGreaterThanOrEqual(1);
    const last = calls[calls.length - 1];
    expect(last[0]).toBe('set_helm');
    expect(last[1].thrust === 0 || Object.is(last[1].thrust, -0)).toBe(true);
    expect(last[1].yaw === 0 || Object.is(last[1].yaw, -0)).toBe(true);

    // snap also applies via rAF — advance a frame
    vi.advanceTimersByTime(16);
    const nub = el.shadowRoot.getElementById('nub');
    expect(nub.style.marginLeft).toBe('0px');
    expect(nub.style.marginTop).toBe('0px');

    vi.useRealTimers();
  });

  it('sets nub position via marginLeft/marginTop on simulated drag', () => {
    mockRAF();
    const { el } = setup();
    el.state = { auto: false };
    stubWellRect(el, 240, 240);
    const well = el.shadowRoot.getElementById('well');
    const nub = el.shadowRoot.getElementById('nub');

    well.dispatchEvent(new PointerEvent('pointerdown', { pointerId: 1, clientX: 200, clientY: 50 }));
    tickRaf();

    const ml = parseFloat(nub.style.marginLeft);
    const mt = parseFloat(nub.style.marginTop);
    expect(Number.isFinite(ml)).toBe(true);
    expect(Number.isFinite(mt)).toBe(true);
    // nub moved away from center (non-zero)
    expect(ml).not.toBe(0);
    expect(mt).not.toBe(0);

    well.dispatchEvent(new PointerEvent('pointerup', { pointerId: 1 }));
  });

  it('snaps back nub to center on release', () => {
    mockRAF();
    const { el } = setup();
    el.state = { auto: false };
    stubWellRect(el, 240, 240);
    const well = el.shadowRoot.getElementById('well');
    const nub = el.shadowRoot.getElementById('nub');

    well.dispatchEvent(new PointerEvent('pointerdown', { pointerId: 1, clientX: 200, clientY: 50 }));
    tickRaf();

    well.dispatchEvent(new PointerEvent('pointerup', { pointerId: 1 }));
    tickRaf();

    expect(nub.style.marginLeft).toBe('0px');
    expect(nub.style.marginTop).toBe('0px');
  });
});
