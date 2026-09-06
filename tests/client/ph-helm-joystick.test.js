// @vitest-environment jsdom
import { t } from '../../gui/strings.js';
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { PhHelmJoystick } from '../../gui/components/ph-helm-joystick.js';
import {
  HELM_STEERING_ACTION_ID,
  HELM_THRUST_ACTION_ID,
} from '../../gui/stations/helm-actions.js';

const JOYSTICK_SOURCE = String(PhHelmJoystick);

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
  if (opts && opts.activateSemanticAction) {
    window.activateSemanticAction = opts.activateSemanticAction;
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
    delete window.activateSemanticAction;
  });

  afterEach(() => {
    document.body.innerHTML = '';
    delete window.activateSemanticAction;
    restoreRAF();
  });

  it('is defined and registered as a custom element', () => {
    expect(customElements.get('ph-helm-joystick')).toBeDefined();
  });

  it('leaves gamepad selection, sampling, and tuning to the parent runtime', () => {
    expect(JOYSTICK_SOURCE).not.toContain('navigator.getGamepads');
    expect(JOYSTICK_SOURCE).not.toContain("'gamepadconnected'");
    expect(JOYSTICK_SOURCE).not.toContain('softenAxis');
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
    expect(badge.textContent.trim()).toBe(t('console.common.auto'));
    expect(el.shadowRoot.getElementById('well').classList.contains('auto')).toBe(true);
  });

  it('renders non-auto state with AUTO badge hidden', () => {
    const { el } = setup();
    el.state = { auto: false };
    const badge = el.shadowRoot.getElementById('auto-badge');
    expect(badge.style.display).toBe('none');
    expect(el.shadowRoot.getElementById('well').classList.contains('auto')).toBe(false);
  });

  it('does not activate an action when auto state is active', () => {
    mockRAF();
    const activateSemanticAction = vi.fn();
    const { el } = setup({ activateSemanticAction });
    el.state = { auto: true };
    stubWellRect(el, 240, 240);
    const well = el.shadowRoot.getElementById('well');
    well.dispatchEvent(new PointerEvent('pointerdown', { pointerId: 1, clientX: 200, clientY: 200 }));
    expect(activateSemanticAction).not.toHaveBeenCalled();
  });

  it('activates the two normalized Helm axes on pointer release', () => {
    mockRAF();
    const activateSemanticAction = vi.fn();
    const { el } = setup({ activateSemanticAction });
    el.state = { auto: false };
    stubWellRect(el, 240, 240);
    const well = el.shadowRoot.getElementById('well');

    // Drag far out — the clamp in #setFromPointer keeps dx,dy in [-1,1]
    well.dispatchEvent(new PointerEvent('pointerdown', { pointerId: 1, clientX: 500, clientY: -200 }));
    tickRaf(); // scheduleApply rAF (nub visual)

    // Release sends the final action synchronously
    well.dispatchEvent(new PointerEvent('pointerup', { pointerId: 1 }));

    expect(activateSemanticAction).toHaveBeenCalledTimes(2);
    expect(activateSemanticAction.mock.calls.map((call) => call[0])).toEqual([
      HELM_THRUST_ACTION_ID, HELM_STEERING_ACTION_ID,
    ]);
    for (const [, options] of activateSemanticAction.mock.calls) {
      expect(options.source).toBe('control');
      expect(Math.abs(options.value)).toBeLessThanOrEqual(1);
    }
  });

  it('snaps nub to center and activates zero thrust/steering on release', () => {
    vi.useFakeTimers();
    const activateSemanticAction = vi.fn();
    const { el } = setup({ activateSemanticAction });
    el.state = { auto: false };
    stubWellRect(el, 240, 240);
    const well = el.shadowRoot.getElementById('well');

    well.dispatchEvent(new PointerEvent('pointerdown', { pointerId: 1, clientX: 200, clientY: 50 }));
    vi.advanceTimersByTime(100);
    activateSemanticAction.mockClear();

    well.dispatchEvent(new PointerEvent('pointerup', { pointerId: 1 }));
    // allow the final sendAction (fired synchronously in onUp) plus visual rAF
    vi.advanceTimersByTime(0);

    const calls = activateSemanticAction.mock.calls;
    expect(calls).toHaveLength(2);
    expect(calls.map((call) => call[0])).toEqual([
      HELM_THRUST_ACTION_ID, HELM_STEERING_ACTION_ID,
    ]);
    expect(calls.every((call) => call[1].value === 0 || Object.is(call[1].value, -0))).toBe(true);

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

  it('derives radius from a smaller measured rect (issue #1376): same clamp, scaled nub', () => {
    mockRAF();
    const activateSemanticAction = vi.fn();
    const { el } = setup({ activateSemanticAction });
    el.state = { auto: false };
    // A rail shrunk well below the 240px dial — e.g. a narrow landscape
    // phone — still derives radius from the MEASURED rect (issue #1375/#1376):
    // radius = min(120,120)/2 - 28 = 32, a quarter of the 240px case's 92.
    stubWellRect(el, 120, 120);
    const well = el.shadowRoot.getElementById('well');
    const nub = el.shadowRoot.getElementById('nub');

    // Drag exactly one radius to the right (dx=1, dy=0): the nub should move
    // 32px — the smaller well's own radius — not the 240px well's 92px.
    well.dispatchEvent(new PointerEvent('pointerdown', { pointerId: 1, clientX: 92, clientY: 60 }));
    tickRaf();
    expect(parseFloat(nub.style.marginLeft)).toBeCloseTo(32, 5);
    expect(parseFloat(nub.style.marginTop)).toBeCloseTo(0, 5);

    // Dragging far outside the smaller well still clamps normalized axes to
    // [-1, 1] on release — the clamp is relative to the measured radius, not
    // a hardcoded distance.
    well.dispatchEvent(new PointerEvent('pointermove', { pointerId: 1, clientX: 500, clientY: -300 }));
    tickRaf();
    well.dispatchEvent(new PointerEvent('pointerup', { pointerId: 1 }));

    expect(activateSemanticAction).toHaveBeenCalledTimes(2);
    expect(activateSemanticAction.mock.calls.map((call) => call[0])).toEqual([
      HELM_THRUST_ACTION_ID, HELM_STEERING_ACTION_ID,
    ]);
    for (const [, options] of activateSemanticAction.mock.calls) {
      expect(Math.abs(options.value)).toBeLessThanOrEqual(1);
    }
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
