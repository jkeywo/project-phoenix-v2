import { describe, it, expect, vi } from 'vitest';
import { createClientOperatorProfile } from '../../gui/client-operator-profile.js';
import { createClientSemanticActionRegistry } from '../../gui/client-semantic-actions.js';
import { createGamepadInputRuntime } from '../../gui/gamepad-input.js';
import { createDefaultOperatorProfile, OPERATOR_PROFILE_KEY, serializeOperatorProfile } from '../../gui/operator-profile.js';

function storage() {
  const values = new Map();
  return { getItem: key => values.get(key) ?? null,
    setItem: vi.fn((key, value) => values.set(key, value)), values };
}
function fixture() {
  const registry = createClientSemanticActionRegistry();
  let store = storage();
  const order = [];
  let accessibility;
  let audio;
  const lifecycle = createClientOperatorProfile({ registry, getStorage: () => store,
    surfaceRoot: { navigator: { getGamepads: () => [], vibrate() {} } },
    getAudio: () => audio,
    getConsoles: () => ({ refreshBindings: () => order.push('bindings') }),
    accessibility: { read: () => accessibility,
      write: value => { accessibility = value; order.push('write'); },
      apply: () => order.push('apply'), assistanceChanged: () => order.push('assistance') },
    refreshGamepadPresentation: () => order.push('presentation'),
  });
  return { registry, lifecycle, order, get store() { return store; },
    setStorage: next => { store = next; }, attachAudio: () => { audio = { reload: () => order.push('audio') }; } };
}
function pad(axis = 0) {
  return { index: 0, id: 'remembered-pad', mapping: 'standard', axes: [axis, 0, 0, 0],
    buttons: Array.from({ length: 17 }, () => ({ pressed: false, value: 0 })) };
}

describe('Station client operator-profile lifecycle', () => {
  it('loads defaults without writes, then uses arriving native storage and late collaborators', () => {
    const f = fixture();
    f.setStorage(null);
    f.lifecycle.load();
    const incoming = storage();
    const profile = createDefaultOperatorProfile(f.registry);
    profile.feedback.semanticCues = false;
    profile.gamepad.preferredDevice = { id: 'remembered-pad', mapping: 'standard' };
    incoming.values.set(OPERATOR_PROFILE_KEY, serializeOperatorProfile(profile));
    f.setStorage(incoming);
    f.attachAudio();
    const runtime = createGamepadInputRuntime({ getGamepads: () => [pad()],
      getActions: () => f.registry.list('helm'), getContext: () => 'helm' });
    f.lifecycle.attachGamepad(runtime);
    f.order.length = 0;
    const originalRestore = runtime.restoreDevice;
    runtime.restoreDevice = value => { f.order.push('restore'); return originalRestore(value); };
    const originalNeutral = runtime.neutralize;
    runtime.neutralize = () => { f.order.push('neutral'); return originalNeutral(); };
    f.lifecycle.load();
    expect(incoming.setItem).not.toHaveBeenCalled();
    expect(runtime.state().preferredDevice).toEqual(profile.gamepad.preferredDevice);
    expect(f.lifecycle.state().feedback.semanticCues).toBe(false);
    expect(f.order).toEqual(['audio', 'write', 'restore', 'bindings', 'neutral', 'apply', 'assistance']);
  });

  it('validates and persists imports before any live changes, including storage refusal', () => {
    const f = fixture(); f.lifecycle.load(); f.order.length = 0;
    const before = f.lifecycle.exportJson();
    expect(f.lifecycle.importJson('{bad').status).toBe('rejected');
    expect(f.store.setItem).not.toHaveBeenCalled();
    const next = createDefaultOperatorProfile(f.registry); next.feedback.semanticCues = false;
    f.store.setItem.mockImplementation(() => { throw new Error('quota'); });
    expect(f.lifecycle.importJson(serializeOperatorProfile(next)).status).toBe('rejected');
    expect(f.lifecycle.exportJson()).toBe(before);
    expect(f.order).toEqual([]);
    f.store.setItem.mockImplementation((key, value) => { f.order.push('save'); f.store.values.set(key, value); });
    expect(f.lifecycle.importJson(serializeOperatorProfile(next)).status).toBe('imported');
    expect(f.order[0]).toBe('save');
    expect(f.lifecycle.state().feedback.semanticCues).toBe(false);
  });

  it('keeps inactive layouts and density through settings, export, and privacy filtering', () => {
    const f = fixture();
    const profile = createDefaultOperatorProfile(f.registry);
    profile.gmDensity = 'touch'; profile.previousLiveLayout = profile.liveLayout;
    profile.testLayout.selected = 'test-trace';
    profile.sessionToken = 'secret'; profile.stationAuthority = 'helm';
    f.lifecycle.importJson(JSON.stringify(profile));
    const imported = JSON.parse(f.lifecycle.exportJson());
    f.lifecycle.setHideTouchControls(false);
    f.lifecycle.saveAudio({ ...imported.audio, masterVolume: 0.7 });
    f.lifecycle.setBinding('captain.red-alert', 0, { code: 'KeyY', shiftKey: true });
    const exported = JSON.parse(f.lifecycle.exportJson());
    for (const field of ['testLayout', 'previousLiveLayout', 'gmDensity']) expect(exported[field]).toEqual(imported[field]);
    expect(exported.gamepad.hideTouchControls).toBe(false);
    expect(exported).not.toHaveProperty('sessionToken');
    expect(exported).not.toHaveProperty('stationAuthority');
    expect(JSON.parse(f.store.values.get(OPERATOR_PROFILE_KEY))).toEqual(exported);
  });

  it('neutralizes held input via the real runtime and keeps ordinary settings live on storage failure', () => {
    const f = fixture(); f.lifecycle.load();
    let pads = [pad()]; const activate = vi.fn();
    const runtime = createGamepadInputRuntime({ getGamepads: () => pads,
      getActions: () => f.registry.list('helm'), getContext: () => 'helm', activate });
    f.lifecycle.attachGamepad(runtime);
    expect(f.lifecycle.selectGamepad(0).status).toBe('selected');
    runtime.poll(pads, 0);
    pads = [pad(0.6)]; runtime.poll(pads, 1);
    expect(activate.mock.calls.some(([id, options]) => id === 'helm.steering' && options.value !== 0)).toBe(true);
    f.store.setItem.mockImplementation(() => { throw new Error('quota'); });
    expect(f.lifecycle.setTuning('helm.steering', { deadzone: 0.2, inverted: true }).status).toBe('applied');
    expect(activate.mock.calls.at(-1)).toEqual(['helm.steering', expect.objectContaining({ value: 0, neutral: true })]);
    expect(f.registry.tuningProfile()['helm.steering']).toEqual({ deadzone: 0.2, inverted: true });
    runtime.poll(pads, 2);
    expect(runtime.state().status).toBe('neutral');
    const count = activate.mock.calls.length;
    expect(f.lifecycle.setBinding('captain.red-alert', 0, { code: 'KeyR', ctrlKey: true }).status).toBe('reserved');
    expect(activate.mock.calls).toHaveLength(count);
    expect(f.lifecycle.setBinding('captain.red-alert', 0, { code: 'KeyY', shiftKey: true }).status).toBe('applied');
    expect(f.registry.bindingProfile()['captain.red-alert'][0].code).toBe('KeyY');
    expect(f.lifecycle.selectGamepad(null).status).toBe('none');
    expect(JSON.parse(f.lifecycle.exportJson()).gamepad.preferredDevice).toBeNull();
  });

  it('restores a late runtime and releases held input when remapping its action', () => {
    const f = fixture();
    const profile = createDefaultOperatorProfile(f.registry);
    profile.gamepad.preferredDevice = { id: 'remembered-pad', mapping: 'standard' };
    f.store.values.set(OPERATOR_PROFILE_KEY, serializeOperatorProfile(profile));
    f.lifecycle.load();
    let pads = [pad()]; const activate = vi.fn();
    const runtime = createGamepadInputRuntime({ getGamepads: () => pads,
      getActions: () => f.registry.list('helm'), getContext: () => 'helm', activate });
    f.lifecycle.attachGamepad(runtime);
    expect(runtime.state().preferredDevice).toEqual(profile.gamepad.preferredDevice);
    runtime.poll(pads, 0);
    pads = [pad(0.6)]; runtime.poll(pads, 1);
    f.order.length = 0;
    expect(f.lifecycle.setBinding('helm.steering', 0, null).status).toBe('applied');
    expect(activate.mock.calls.at(-1)).toEqual(['helm.steering', expect.objectContaining({ value: 0, neutral: true })]);
    expect(f.order).toEqual(['bindings']);
    const count = activate.mock.calls.length;
    runtime.poll(pads, 2);
    expect(activate.mock.calls).toHaveLength(count);
    expect(f.lifecycle.resetAction('helm.steering').status).toBe('applied');
    expect(f.lifecycle.resetAllBindings().status).toBe('applied');
    expect(JSON.parse(f.lifecycle.exportJson()).bindings).toEqual(f.registry.bindingProfile());
  });

  it('retains unavailable native gamepad and feedback preferences in portable output', () => {
    const registry = createClientSemanticActionRegistry(); const store = storage();
    const profile = createDefaultOperatorProfile(registry);
    profile.gamepad.preferredDevice = { id: 'remembered-pad', mapping: 'standard' };
    store.values.set(OPERATOR_PROFILE_KEY, serializeOperatorProfile(profile));
    const lifecycle = createClientOperatorProfile({ registry, getStorage: () => store,
      surfaceRoot: { PhoenixOperatorCapabilities: { surface: 'native-pane', gamepad: false, vibration: false } } });
    lifecycle.load(); lifecycle.setHideTouchControls(false);
    expect(lifecycle.state().feedback.vibration).toBe(false);
    const exported = JSON.parse(lifecycle.exportJson());
    expect(exported.feedback.vibration).toBe(true);
    expect(exported.gamepad.preferredDevice).toEqual(profile.gamepad.preferredDevice);
  });
});
